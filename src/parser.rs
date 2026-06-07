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

use crate::model::Tokens;

#[derive(Debug, Default, Clone)]
pub struct JsonlSummary {
    pub tokens: Tokens,
    /// Most recently observed model id, if any.
    pub model: Option<String>,
    /// Bytes consumed so far — used by the cache for incremental resume.
    pub byte_offset: u64,
}

const NEEDLE: &[u8] = b"\"usage\"";

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
        if !contains_subslice(&buf, NEEDLE) {
            continue;
        }
        let Ok(value): serde_json::Result<serde_json::Value> = serde_json::from_slice(&buf) else {
            continue;
        };
        let Some(usage) = value.pointer("/message/usage") else {
            continue;
        };
        let t = &mut summary.tokens;
        t.input += as_u64(usage.get("input_tokens"));
        t.output += as_u64(usage.get("output_tokens"));
        t.cache_creation += as_u64(usage.get("cache_creation_input_tokens"));
        t.cache_read += as_u64(usage.get("cache_read_input_tokens"));
        // Skip "<synthetic>" — Claude Code's marker for compaction summaries
        // and similar messages it generates internally (not a real model call).
        // Keep the most recent REAL model so cost lookup works.
        if let Some(m) = value
            .pointer("/message/model")
            .and_then(|v| v.as_str())
            .filter(|m| *m != "<synthetic>")
        {
            summary.model = Some(m.to_string());
        }
    }
    Ok(summary)
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
