// High-level API consumed by both the CLI and the Tauri app.
// Single source of truth: "list sessions" / "list blocks" live here so the
// two frontends never drift.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::{blocks, cache, joiner, model::SessionRow, parser, pricing, scanner};

pub fn resolve_root(root: Option<PathBuf>) -> Result<PathBuf> {
    match root {
        Some(p) => Ok(p),
        None => dirs::home_dir()
            .map(|h| h.join(".claude"))
            .context("cannot resolve $HOME"),
    }
}

pub fn list_sessions(root: Option<PathBuf>) -> Result<Vec<SessionRow>> {
    let root = resolve_root(root)?;
    let scan = scanner::scan(&root)?;
    let mut cache_file = cache::load(&root)?;
    let mut rows = joiner::join(&scan, &mut cache_file)?;
    cache::save(&root, &cache_file)?;
    let pricing_table = pricing::load();
    for row in &mut rows {
        row.cost_usd = row.model.as_deref().and_then(|m| {
            pricing::lookup(&pricing_table, m).map(|p| {
                let mut combined = row.tokens;
                combined.add(&row.subagent_tokens);
                pricing::cost_usd(p, &combined)
            })
        });
    }
    Ok(rows)
}

pub fn list_blocks(root: Option<PathBuf>) -> Result<Vec<blocks::SessionBlock>> {
    Ok(list_blocks_enriched(root)?.blocks)
}

/// Burn-rate window: trailing wall-clock used to derive USD/hr. 10 min is
/// short enough to react to a started task but long enough to smooth out
/// the bursty per-message cost pattern.
const BURN_WINDOW_MS: i64 = 10 * 60 * 1000;
/// Floor on the span we divide by so a single event doesn't synthesise a
/// gigantic instantaneous rate.
const BURN_SPAN_FLOOR_MS: i64 = 60_000;

#[derive(Debug, Clone)]
pub struct ActiveBurn {
    /// Current cost of the active block (matches block_cost in renderer).
    pub current_usd: f64,
    /// Rolling USD/hr over the last `BURN_WINDOW_MS`.
    pub burn_usd_per_hr: f64,
    /// `current_usd + burn × remaining_hours_to_block_end`.
    pub projected_block_usd: f64,
    /// `start_ms` of the active block — used by callers to deduplicate
    /// per-block proactive alerts.
    pub block_start_ms: i64,
}

#[derive(Debug, Clone)]
pub struct EnrichedBlocks {
    pub blocks: Vec<blocks::SessionBlock>,
    pub active: Option<ActiveBurn>,
}

pub fn list_blocks_enriched(root: Option<PathBuf>) -> Result<EnrichedBlocks> {
    let root = resolve_root(root)?;
    let scan = scanner::scan(&root)?;
    let mut events = Vec::new();
    for p in scan.transcript_files.iter().chain(&scan.subagent_files) {
        events.extend(parser::events_jsonl(p, 0).unwrap_or_default());
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let blocks_list = blocks::detect_blocks(events.clone(), now_ms);

    let table = pricing::load();
    let active = blocks_list.iter().find(|b| b.is_active).map(|ab| {
        // Active-block cost: same heuristic as renderer (first non-empty
        // model in the block).
        let current_usd = ab
            .models
            .iter()
            .find(|m| !m.is_empty())
            .and_then(|m| pricing::lookup(&table, m))
            .map(|p| pricing::cost_usd(p, &ab.tokens))
            .unwrap_or(0.0);

        let window_start = (now_ms - BURN_WINDOW_MS).max(ab.start_ms);
        let mut window_cost = 0.0;
        let mut earliest_ts = i64::MAX;
        for e in events
            .iter()
            .filter(|e| e.timestamp_ms >= window_start && e.timestamp_ms <= now_ms)
        {
            if e.timestamp_ms < earliest_ts {
                earliest_ts = e.timestamp_ms;
            }
            window_cost += e
                .model
                .as_deref()
                .and_then(|m| pricing::lookup(&table, m))
                .map(|p| pricing::cost_usd(p, &e.tokens))
                .unwrap_or(0.0);
        }
        let burn_usd_per_hr = if earliest_ts == i64::MAX {
            0.0
        } else {
            let span_ms = (now_ms - earliest_ts).max(BURN_SPAN_FLOOR_MS);
            window_cost * 3_600_000.0 / span_ms as f64
        };

        let remaining_hours = ((ab.end_ms - now_ms).max(0)) as f64 / 3_600_000.0;
        let projected_block_usd = current_usd + burn_usd_per_hr * remaining_hours;

        ActiveBurn {
            current_usd,
            burn_usd_per_hr,
            projected_block_usd,
            block_start_ms: ab.start_ms,
        }
    });

    Ok(EnrichedBlocks {
        blocks: blocks_list,
        active,
    })
}

