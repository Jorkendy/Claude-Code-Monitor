// JSONL parser: stream line-by-line, filter lines containing the literal "usage"
// substring before parsing, then sum the four token counters.
//
// VERIFIED schema notes (don't deviate without re-checking real files):
// - usage lives at `.message.usage`, NOT `.usage` at top level.
// - model name lives at `.message.model` (e.g. "claude-opus-4-8").
// - `usage.iterations[]` repeats the same counters for retries/compactions;
//   summing them DOUBLE-counts. Read top-level usage fields only.
// - Missing fields treated as 0.

use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use crate::model::{Tokens, UsageEvent};

#[derive(Debug, Default, Clone)]
pub struct JsonlSummary {
    pub tokens: Tokens,
    /// Most recently observed model id, if any.
    pub model: Option<String>,
    /// Bytes consumed so far — used by the cache for incremental resume.
    pub byte_offset: u64,
    /// Context window usage = `input + cache_read + cache_creation` of the
    /// latest non-synthetic assistant turn. Each turn re-sends the full
    /// history (or post-compaction summary), so this is the prompt size at
    /// the most recent turn — i.e., how full the context is right now.
    pub latest_context_tokens: u64,
    /// Timestamp (ms) of the event that contributed `latest_context_tokens`.
    /// Used to pick the winner when merging incremental parses.
    pub latest_ts_ms: i64,
    /// Most recently observed `customTitle` from a `{type:"custom-title"}`
    /// event. Claude Code writes this when the user runs `/rename`. Survives
    /// session exit (which removes `sessions/{pid}.json`), so this is the
    /// only source of name for inactive sessions.
    pub latest_name: Option<String>,
}

const NEEDLE_USAGE: &[u8] = b"\"usage\"";
const NEEDLE_TITLE: &[u8] = b"\"customTitle\"";

pub fn sum_jsonl(path: &Path, resume_from_offset: u64) -> Result<JsonlSummary> {
    let mut file = File::open(path)?;
    let total_len = file.metadata()?.len();
    let start = if resume_from_offset <= total_len {
        resume_from_offset
    } else {
        0
    };
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let mut reader = BufReader::new(file);

    let mut summary = JsonlSummary {
        tokens: Tokens::default(),
        model: None,
        byte_offset: start,
        latest_context_tokens: 0,
        latest_ts_ms: 0,
        latest_name: None,
    };
    let mut buf: Vec<u8> = Vec::with_capacity(16 * 1024);
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        // Partial trailing line (no \n yet). Skip and DON'T advance offset, so
        // the next incremental run re-reads it once it's complete. Otherwise
        // we'd either lose data or, worse, double-count if we counted it now.
        if buf.last() != Some(&b'\n') {
            break;
        }
        summary.byte_offset += n as u64;
        let has_usage = contains_subslice(&buf, NEEDLE_USAGE);
        let has_title = !has_usage && contains_subslice(&buf, NEEDLE_TITLE);
        if !has_usage && !has_title {
            continue;
        }
        let Ok(value): serde_json::Result<serde_json::Value> = serde_json::from_slice(&buf) else {
            continue;
        };
        if has_title {
            if let Some(t) = value.get("customTitle").and_then(|v| v.as_str()) {
                summary.latest_name = Some(t.to_string());
            }
            continue;
        }
        let Some(usage) = value.pointer("/message/usage") else {
            continue;
        };
        let event_input = as_u64(usage.get("input_tokens"));
        let event_output = as_u64(usage.get("output_tokens"));
        let event_cache_creation = as_u64(usage.get("cache_creation_input_tokens"));
        let event_cache_read = as_u64(usage.get("cache_read_input_tokens"));
        let t = &mut summary.tokens;
        t.input += event_input;
        t.output += event_output;
        t.cache_creation += event_cache_creation;
        t.cache_read += event_cache_read;
        // Skip "<synthetic>" — Claude Code's marker for compaction summaries
        // and similar messages it generates internally (not a real model call).
        // Keep the most recent REAL model so cost lookup works.
        let real_model = value
            .pointer("/message/model")
            .and_then(|v| v.as_str())
            .filter(|m| *m != "<synthetic>");
        if let Some(m) = real_model {
            summary.model = Some(m.to_string());
        }
        // Track context window usage: only real model events (not synthetic
        // compaction markers) define the live context size.
        if real_model.is_some() {
            if let Some(ts_str) = value.get("timestamp").and_then(|t| t.as_str()) {
                if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                    let ts_ms = ts.timestamp_millis();
                    if ts_ms >= summary.latest_ts_ms {
                        summary.latest_ts_ms = ts_ms;
                        summary.latest_context_tokens =
                            event_input + event_cache_read + event_cache_creation;
                    }
                }
            }
        }
    }
    Ok(summary)
}

/// Stream events with timestamps. Used for block / daily aggregation —
/// the aggregate `sum_jsonl` path remains for cached totals.
///
/// `since_ms` filters events older than this (set to `0` for all).
/// Unlike `sum_jsonl` this does not advance any byte_offset — callers
/// must accept that we re-read whichever range they want each call.
pub fn events_jsonl(path: &Path, since_ms: i64) -> anyhow::Result<Vec<UsageEvent>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut buf: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut events = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        if buf.last() != Some(&b'\n') {
            break;
        }
        if !contains_subslice(&buf, NEEDLE_USAGE) {
            continue;
        }
        let Ok(v): serde_json::Result<serde_json::Value> = serde_json::from_slice(&buf) else {
            continue;
        };
        let Some(ts_str) = v.get("timestamp").and_then(|t| t.as_str()) else {
            continue;
        };
        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(ts_str) else {
            continue;
        };
        let ts_ms = ts.timestamp_millis();
        if ts_ms < since_ms {
            continue;
        }
        let Some(usage) = v.pointer("/message/usage") else {
            continue;
        };
        let tokens = Tokens {
            input: as_u64(usage.get("input_tokens")),
            output: as_u64(usage.get("output_tokens")),
            cache_creation: as_u64(usage.get("cache_creation_input_tokens")),
            cache_read: as_u64(usage.get("cache_read_input_tokens")),
        };
        let model = v
            .pointer("/message/model")
            .and_then(|m| m.as_str())
            .filter(|m| *m != "<synthetic>")
            .map(String::from);
        events.push(UsageEvent {
            timestamp_ms: ts_ms,
            tokens,
            model,
        });
    }
    Ok(events)
}

fn as_u64(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|v| v.as_u64()).unwrap_or(0)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}
