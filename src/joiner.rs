// Joiner: merge scanner outputs + parser totals + liveness checks into SessionRow[].
// Key = sessionId.
//
// A session present only in projects/ (no sessions/ file) MUST still appear,
// with status = Inactive and Name = None (UID column carries the identity).

use crate::cache::{self, CacheFile};
use crate::liveness;
use crate::model::{SessionRow, Tokens};
use crate::scanner::ScanResult;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct SessionMeta {
    #[serde(rename = "sessionId")]
    session_id: String,
    pid: Option<i32>,
    cwd: Option<PathBuf>,
    name: Option<String>,
    status: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<i64>,
}

#[derive(Default, Debug)]
struct Partial {
    name: Option<String>,
    cwd: Option<PathBuf>,
    pid: Option<i32>,
    status_field: Option<String>,
    tokens: Tokens,
    subagent_tokens: Tokens,
    subagent_count: usize,
    model: Option<String>,
    updated_at_ms: Option<i64>,
}

pub fn join(scan: &ScanResult, cache: &mut CacheFile) -> Result<Vec<SessionRow>> {
    let mut map: HashMap<String, Partial> = HashMap::new();

    for p in &scan.session_files {
        let Ok(bytes) = fs::read(p) else { continue };
        let Ok(meta): serde_json::Result<SessionMeta> = serde_json::from_slice(&bytes) else {
            continue;
        };
        let entry = map.entry(meta.session_id).or_default();
        entry.name = meta.name;
        entry.cwd = meta.cwd;
        entry.pid = meta.pid;
        entry.status_field = meta.status;
        entry.updated_at_ms = meta.updated_at;
    }

    for p in &scan.transcript_files {
        let Some(sid) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        let Ok(s) = cache::cached_sum(p, cache) else { continue };
        let entry = map.entry(sid).or_default();
        entry.tokens.add(&s.tokens);
        if let Some(m) = s.model {
            entry.model = Some(m);
        }
        if entry.cwd.is_none() {
            if let Some(slug) = p
                .parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
            {
                entry.cwd = Some(slug_repo_guess(slug));
            }
        }
    }

    for p in &scan.subagent_files {
        let Some(sid) = p
            .parent()
            .and_then(|s| s.parent())
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(s) = cache::cached_sum(p, cache) else { continue };
        let entry = map.entry(sid).or_default();
        entry.subagent_tokens.add(&s.tokens);
        entry.subagent_count += 1;
    }

    let rows = map
        .into_iter()
        .map(|(session_id, p)| {
            let status = liveness::classify(p.pid, p.status_field.as_deref());
            SessionRow {
                session_id,
                name: p.name,
                cwd: p.cwd,
                pid: p.pid,
                status,
                tokens: p.tokens,
                subagent_tokens: p.subagent_tokens,
                subagent_count: p.subagent_count,
                model: p.model,
                updated_at_ms: p.updated_at_ms,
                cost_usd: None,
            }
        })
        .collect();
    Ok(rows)
}

/// Slug -> PathBuf whose basename approximates the repo name.
/// The slug encoding loses information (every '/' becomes '-', so a real
/// path segment containing '-' is indistinguishable from a '/' separator).
/// For the Repo column we only need basename, so use the last '-' segment.
fn slug_repo_guess(slug: &str) -> PathBuf {
    PathBuf::from(slug.rsplit('-').next().unwrap_or(slug))
}
