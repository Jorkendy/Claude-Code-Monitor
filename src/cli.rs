use anyhow::{Result, anyhow};
use clap::Parser;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::{api, blocks, cache, model::SessionRow, parser, pricing, renderer, scanner};

#[derive(Parser, Debug)]
#[command(name = "cc-monitor", about = "Claude Code session monitor (read-only)")]
pub struct Args {
    /// Override ~/.claude root path
    #[arg(long, env = "CLAUDE_HOME")]
    pub root: Option<PathBuf>,

    /// Output JSON instead of table
    #[arg(long)]
    pub json: bool,

    /// Sort key
    #[arg(long, default_value = "total", value_parser = ["total", "updated", "name", "cost"])]
    pub sort: String,

    /// Include cache_read in Total column
    #[arg(long)]
    pub include_cache_read: bool,

    /// Filter by repo (basename of cwd)
    #[arg(long)]
    pub repo: Option<String>,

    /// Show full numbers (default formats compactly, e.g. 55.7K)
    #[arg(long)]
    pub full: bool,

    /// Drill down per-subagent breakdown for a session (not implemented yet)
    #[arg(long, value_name = "SESSION_ID")]
    pub subagents: Option<String>,

    /// Show 5-hour billing blocks instead of the session list
    #[arg(long)]
    pub blocks: bool,
}

impl Args {
    pub fn run(self) -> Result<()> {
        let root = api::resolve_root(self.root.clone())?;
        let opts = renderer::RenderOpts {
            include_cache_read: self.include_cache_read,
            full: self.full,
        };
        if let Some(prefix) = &self.subagents {
            let scan = scanner::scan(&root)?;
            let mut cache = cache::load(&root)?;
            let sid = resolve_session_prefix(&scan, prefix)?;
            renderer::render_subagent_drilldown(&scan, &mut cache, &sid, &opts)?;
            cache::save(&root, &cache)?;
            return Ok(());
        }
        if self.blocks {
            let scan = scanner::scan(&root)?;
            let pricing_table = pricing::load();
            let mut events = Vec::new();
            for p in scan.transcript_files.iter().chain(&scan.subagent_files) {
                events.extend(parser::events_jsonl(p, 0).unwrap_or_default());
            }
            let now_ms = chrono::Utc::now().timestamp_millis();
            let detected = blocks::detect_blocks(events, now_ms);
            renderer::render_blocks(&detected, &pricing_table, now_ms, self.json)?;
            return Ok(());
        }
        let mut rows = api::list_sessions(Some(root.clone()))?;
        if let Some(repo) = &self.repo {
            rows.retain(|r| {
                r.cwd
                    .as_deref()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .map(|s| s == repo)
                    .unwrap_or(false)
            });
        }
        sort_rows(&mut rows, &self.sort, self.include_cache_read);
        if self.json {
            renderer::render_json(&rows)?;
        } else {
            renderer::render_table(&rows, &opts)?;
        }
        Ok(())
    }
}

fn sort_rows(rows: &mut [SessionRow], key: &str, include_cache_read: bool) {
    // Always include session_id as tiebreaker so output is stable across runs
    // (HashMap iteration in the joiner is non-deterministic).
    match key {
        "total" => rows.sort_by(|a, b| {
            total_with_subs(b, include_cache_read)
                .cmp(&total_with_subs(a, include_cache_read))
                .then_with(|| a.session_id.cmp(&b.session_id))
        }),
        "cost" => rows.sort_by(|a, b| {
            let ac = a.cost_usd.unwrap_or(f64::NEG_INFINITY);
            let bc = b.cost_usd.unwrap_or(f64::NEG_INFINITY);
            bc.partial_cmp(&ac)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.session_id.cmp(&b.session_id))
        }),
        "updated" => rows.sort_by(|a, b| {
            b.updated_at_ms
                .unwrap_or(0)
                .cmp(&a.updated_at_ms.unwrap_or(0))
                .then_with(|| a.session_id.cmp(&b.session_id))
        }),
        "name" => rows.sort_by(|a, b| {
            let an = a.name.as_deref().unwrap_or(&a.session_id);
            let bn = b.name.as_deref().unwrap_or(&b.session_id);
            an.cmp(bn).then_with(|| a.session_id.cmp(&b.session_id))
        }),
        _ => {}
    }
}

fn total_with_subs(r: &SessionRow, include_cache_read: bool) -> u64 {
    let mut t = r.tokens;
    t.add(&r.subagent_tokens);
    t.total(include_cache_read)
}

/// Resolve a session id prefix (e.g. "f9cd9074") to a full UUID. Exact match
/// wins; otherwise unique prefix match. Errors out on no-match or ambiguous.
fn resolve_session_prefix(scan: &scanner::ScanResult, input: &str) -> Result<String> {
    let mut all: HashSet<String> = HashSet::new();
    for p in &scan.transcript_files {
        if let Some(s) = p.file_stem().and_then(|s| s.to_str()) {
            all.insert(s.to_string());
        }
    }
    for p in &scan.subagent_files {
        if let Some(s) = p
            .parent()
            .and_then(|p| p.parent())
            .and_then(|d| d.file_name())
            .and_then(|s| s.to_str())
        {
            all.insert(s.to_string());
        }
    }
    if all.contains(input) {
        return Ok(input.to_string());
    }
    let candidates: Vec<String> = all
        .into_iter()
        .filter(|s| s.starts_with(input))
        .collect();
    match candidates.len() {
        0 => Err(anyhow!("No session matches prefix '{input}'")),
        1 => Ok(candidates.into_iter().next().unwrap()),
        n => {
            let preview = candidates
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            Err(anyhow!(
                "Ambiguous prefix '{input}' (matches {n}): {preview}"
            ))
        }
    }
}
