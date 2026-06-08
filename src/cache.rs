// Incremental cache (P1): per-file {size, mtime_ms, byte_offset, tokens, model}.
//
// Reuse rules:
//   size + mtime both match -> reuse cached tokens, skip read
//   size grew               -> seek to byte_offset, parse appended bytes, add
//   anything else           -> full re-read (file shrank/rewritten/new)
//
// Cache file: <root>/.tokenscope-cache.json — the only path the tool ever
// writes inside CLAUDE_HOME (read-only guarantee from spec excludes our
// own cache).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::model::Tokens;
use crate::parser::{self, JsonlSummary};

// Schema-versioned filename: bumping invalidates old caches without an
// explicit migration step. v1 lacked `latest_name`, which left inactive
// sessions nameless because size+mtime always match for closed transcripts.
const CACHE_FILE: &str = ".tokenscope-cache-v2.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub size: u64,
    pub mtime_ms: i64,
    pub byte_offset: u64,
    pub tokens: Tokens,
    pub model: Option<String>,
    /// Latest non-synthetic event's prompt size (context window usage).
    /// `#[serde(default)]` lets old caches still load — they'll be re-parsed
    /// on next file change since size/mtime match-only path also needs these.
    #[serde(default)]
    pub latest_context_tokens: u64,
    #[serde(default)]
    pub latest_ts_ms: i64,
    /// Last-seen `customTitle` for this transcript. Survives session exit
    /// (Claude Code removes `sessions/{pid}.json` on exit but keeps the
    /// JSONL), giving us a stable name for inactive sessions.
    #[serde(default)]
    pub latest_name: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CacheFile {
    pub files: HashMap<PathBuf, FileEntry>,
}

pub fn load(root: &Path) -> Result<CacheFile> {
    let path = root.join(CACHE_FILE);
    let Ok(bytes) = fs::read(&path) else {
        return Ok(CacheFile::default());
    };
    Ok(serde_json::from_slice(&bytes).unwrap_or_default())
}

pub fn save(root: &Path, cache: &CacheFile) -> Result<()> {
    let path = root.join(CACHE_FILE);
    let tmp = root.join(format!("{CACHE_FILE}.tmp"));
    let bytes = serde_json::to_vec(cache)?;
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Parser-with-cache wrapper. Same return shape as `parser::sum_jsonl`,
/// transparently reusing cached totals when possible.
pub fn cached_sum(path: &Path, cache: &mut CacheFile) -> Result<JsonlSummary> {
    let meta = fs::metadata(path)?;
    let size = meta.len();
    let mtime_ms = mtime_millis(&meta);

    if let Some(prev) = cache.files.get(path).cloned() {
        if prev.size == size && prev.mtime_ms == mtime_ms {
            return Ok(JsonlSummary {
                tokens: prev.tokens,
                model: prev.model,
                byte_offset: prev.byte_offset,
                latest_context_tokens: prev.latest_context_tokens,
                latest_ts_ms: prev.latest_ts_ms,
                latest_name: prev.latest_name,
            });
        }
        if size > prev.size {
            let mut new = parser::sum_jsonl(path, prev.byte_offset)?;
            new.tokens.add(&prev.tokens);
            if new.model.is_none() {
                new.model = prev.model.clone();
            }
            // Latest-event wins: the appended portion only updates context if
            // its newest event is at least as recent as the prior one.
            if prev.latest_ts_ms > new.latest_ts_ms {
                new.latest_ts_ms = prev.latest_ts_ms;
                new.latest_context_tokens = prev.latest_context_tokens;
            }
            // Name only appears once per /rename. Keep the prior cached name
            // when the delta scan didn't see another custom-title event.
            if new.latest_name.is_none() {
                new.latest_name = prev.latest_name;
            }
            cache.files.insert(
                path.to_path_buf(),
                FileEntry {
                    size,
                    mtime_ms,
                    byte_offset: new.byte_offset,
                    tokens: new.tokens,
                    model: new.model.clone(),
                    latest_context_tokens: new.latest_context_tokens,
                    latest_ts_ms: new.latest_ts_ms,
                    latest_name: new.latest_name.clone(),
                },
            );
            return Ok(new);
        }
    }

    let summary = parser::sum_jsonl(path, 0)?;
    cache.files.insert(
        path.to_path_buf(),
        FileEntry {
            size,
            mtime_ms,
            byte_offset: summary.byte_offset,
            tokens: summary.tokens,
            model: summary.model.clone(),
            latest_context_tokens: summary.latest_context_tokens,
            latest_ts_ms: summary.latest_ts_ms,
            latest_name: summary.latest_name.clone(),
        },
    );
    Ok(summary)
}

fn mtime_millis(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
