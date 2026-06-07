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
    let root = resolve_root(root)?;
    let scan = scanner::scan(&root)?;
    let mut events = Vec::new();
    for p in scan.transcript_files.iter().chain(&scan.subagent_files) {
        events.extend(parser::events_jsonl(p, 0).unwrap_or_default());
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(blocks::detect_blocks(events, now_ms))
}
