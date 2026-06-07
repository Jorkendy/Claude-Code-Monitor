// Renderer: simple terminal table (default) or JSON (--json).
// Compact number formatting (55.7K) unless --full.

use crate::cache::{self, CacheFile};
use crate::model::{LiveStatus, SessionRow, Tokens};
use crate::scanner::ScanResult;
use anyhow::Result;
use serde::Deserialize;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

pub struct RenderOpts {
    pub include_cache_read: bool,
    pub full: bool,
}

pub fn render_table(rows: &[SessionRow], opts: &RenderOpts) -> Result<()> {
    let header = [
        "UID", "NAME", "REPO", "STATUS", "IN", "OUT", "C-WRITE", "C-READ", "TOTAL", "SUBS",
        "COST", "MODEL",
    ];
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len() + 1);
    grid.push(header.iter().map(|s| s.to_string()).collect());

    for row in rows {
        let mut combined = row.tokens;
        combined.add(&row.subagent_tokens);
        let total_value = combined.total(opts.include_cache_read);
        let fmt = |n: u64| -> String {
            if opts.full {
                n.to_string()
            } else {
                compact(n)
            }
        };
        grid.push(vec![
            short_uuid(&row.session_id),
            row
                .name
                .as_deref()
                .map(|n| truncate(n, 40))
                .unwrap_or_else(|| "-".into()),
            row.cwd
                .as_deref()
                .and_then(repo_basename)
                .unwrap_or_else(|| "-".into()),
            status_label(row.status).into(),
            fmt(combined.input),
            fmt(combined.output),
            fmt(combined.cache_creation),
            fmt(combined.cache_read),
            fmt(total_value),
            row.subagent_count.to_string(),
            format_cost(row.cost_usd),
            row.model.clone().unwrap_or_else(|| "-".into()),
        ]);
    }

    let cols = header.len();
    let widths: Vec<usize> = (0..cols)
        .map(|i| grid.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    let numeric_cols = 4..=10;
    for row in &grid {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            let w = widths[i];
            if numeric_cols.contains(&i) {
                let _ = write!(line, "{cell:>w$}");
            } else {
                let _ = write!(line, "{cell:<w$}");
            }
            if i < cols - 1 {
                line.push_str("  ");
            }
        }
        println!("{line}");
    }
    println!();
    println!(
        "Cost is an estimate (per-token rates, snapshot 2026-06). \
         Pro/Max subscription billing may differ. \
         Override pricing at ~/.config/cc-monitor/pricing.toml."
    );
    Ok(())
}

pub fn render_json(rows: &[SessionRow]) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(rows)?);
    Ok(())
}

pub fn render_subagent_drilldown(
    scan: &ScanResult,
    cache_file: &mut CacheFile,
    session_id: &str,
    opts: &RenderOpts,
) -> Result<()> {
    #[derive(Default)]
    struct AgentRow {
        id: String,
        agent_type: String,
        description: String,
        tokens: Tokens,
    }

    let mut rows: Vec<AgentRow> = Vec::new();
    for path in &scan.subagent_files {
        let parent_sid = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str());
        if parent_sid != Some(session_id) {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let meta_path = path.with_file_name(format!("{id}.meta.json"));
        let (agent_type, description) = read_meta(&meta_path).unwrap_or_default();
        let summary = cache::cached_sum(path, cache_file).unwrap_or_default();
        rows.push(AgentRow {
            id,
            agent_type,
            description,
            tokens: summary.tokens,
        });
    }
    if rows.is_empty() {
        println!("No subagents found for session {session_id}");
        return Ok(());
    }
    rows.sort_by(|a, b| {
        b.tokens
            .total(opts.include_cache_read)
            .cmp(&a.tokens.total(opts.include_cache_read))
    });

    let header = [
        "AGENT ID", "TYPE", "DESCRIPTION", "IN", "OUT", "C-WRITE", "C-READ", "TOTAL",
    ];
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len() + 1);
    grid.push(header.iter().map(|s| s.to_string()).collect());
    for r in &rows {
        let fmt = |n: u64| -> String {
            if opts.full {
                n.to_string()
            } else {
                compact(n)
            }
        };
        grid.push(vec![
            r.id.clone(),
            r.agent_type.clone(),
            truncate(&r.description, 40),
            fmt(r.tokens.input),
            fmt(r.tokens.output),
            fmt(r.tokens.cache_creation),
            fmt(r.tokens.cache_read),
            fmt(r.tokens.total(opts.include_cache_read)),
        ]);
    }
    let cols = header.len();
    let widths: Vec<usize> = (0..cols)
        .map(|i| grid.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    let numeric_cols = 3..=7;
    for row in &grid {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            let w = widths[i];
            if numeric_cols.contains(&i) {
                let _ = write!(line, "{cell:>w$}");
            } else {
                let _ = write!(line, "{cell:<w$}");
            }
            if i < cols - 1 {
                line.push_str("  ");
            }
        }
        println!("{line}");
    }
    Ok(())
}

fn read_meta(path: &Path) -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct M {
        #[serde(rename = "agentType", default)]
        agent_type: String,
        #[serde(default)]
        description: String,
    }
    let bytes = fs::read(path).ok()?;
    let m: M = serde_json::from_slice(&bytes).ok()?;
    Some((m.agent_type, m.description))
}

fn status_label(s: LiveStatus) -> &'static str {
    match s {
        LiveStatus::Active => "active",
        LiveStatus::Idle => "idle",
        LiveStatus::Inactive => "inactive",
    }
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}

fn repo_basename(p: &Path) -> Option<String> {
    p.file_name()?.to_str().map(str::to_string)
}

fn format_cost(c: Option<f64>) -> String {
    match c {
        None => "N/A".into(),
        Some(v) if v < 0.01 => "<$0.01".into(),
        Some(v) => format!("${v:.2}"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

fn compact(n: u64) -> String {
    let n_f = n as f64;
    if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}K", n_f / 1_000.0)
    } else if n < 1_000_000_000 {
        format!("{:.1}M", n_f / 1_000_000.0)
    } else {
        format!("{:.1}B", n_f / 1_000_000_000.0)
    }
}

