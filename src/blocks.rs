// 5-hour billing block detection. Port of ccusage's algorithm
// (rust/crates/ccusage/src/blocks.rs) — adapted to our types and extended
// with `message_count` for Pro/Max users whose limit is message-count-per-window.
//
// Algorithm:
//   sort events by timestamp
//   for each event:
//     if no current block OR
//        (event - block.start > 5h) OR (event - last_event > 5h):
//       flush current block
//       if (event - last_event > 5h): insert gap block
//       start new block at floor_to_hour(event.timestamp)
//     add event to current block
//   flush final block
//
// "Gap-based reset" mirrors Anthropic's rolling window: enough idle time
// → next message starts a new block. Snap block start to top-of-hour for
// readability.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::model::{Tokens, UsageEvent};

const MS_PER_HOUR: i64 = 60 * 60 * 1000;
const SESSION_DURATION_MS: i64 = 5 * MS_PER_HOUR;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionBlock {
    pub start_ms: i64,
    pub end_ms: i64,
    pub actual_end_ms: Option<i64>,
    pub is_active: bool,
    pub is_gap: bool,
    pub tokens: Tokens,
    pub message_count: usize,
    pub models: Vec<String>,
}

pub fn detect_blocks(mut events: Vec<UsageEvent>, now_ms: i64) -> Vec<SessionBlock> {
    if events.is_empty() {
        return Vec::new();
    }
    events.sort_by_key(|e| e.timestamp_ms);
    let mut blocks = Vec::new();
    let mut current_start: Option<i64> = None;
    let mut current: Vec<UsageEvent> = Vec::new();

    for event in events {
        let mut start_new = false;
        if let Some(start) = current_start {
            let last_time = current
                .last()
                .map(|e| e.timestamp_ms)
                .unwrap_or(start);
            let since_start = event.timestamp_ms - start;
            let since_last = event.timestamp_ms - last_time;
            if since_start > SESSION_DURATION_MS || since_last > SESSION_DURATION_MS {
                blocks.push(create_block(
                    start,
                    std::mem::take(&mut current),
                    now_ms,
                ));
                if since_last > SESSION_DURATION_MS {
                    blocks.push(create_gap_block(last_time, event.timestamp_ms));
                }
                start_new = true;
            }
        } else {
            start_new = true;
        }
        if start_new {
            current_start = Some(floor_to_hour(event.timestamp_ms));
        }
        current.push(event);
    }

    if let Some(start) = current_start {
        if !current.is_empty() {
            blocks.push(create_block(start, current, now_ms));
        }
    }
    blocks
}

fn create_block(start: i64, events: Vec<UsageEvent>, now_ms: i64) -> SessionBlock {
    let end = start + SESSION_DURATION_MS;
    let actual_end = events.last().map(|e| e.timestamp_ms);
    let is_active = actual_end
        .map(|last| now_ms - last < SESSION_DURATION_MS && now_ms < end)
        .unwrap_or(false);
    let mut tokens = Tokens::default();
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for e in &events {
        tokens.add(&e.tokens);
        if let Some(m) = &e.model {
            if seen.insert(m.clone()) {
                models.push(m.clone());
            }
        }
    }
    SessionBlock {
        start_ms: start,
        end_ms: end,
        actual_end_ms: actual_end,
        is_active,
        is_gap: false,
        tokens,
        message_count: events.len(),
        models,
    }
}

fn create_gap_block(last_event_ms: i64, next_event_ms: i64) -> SessionBlock {
    let start = last_event_ms + SESSION_DURATION_MS;
    SessionBlock {
        start_ms: start,
        end_ms: next_event_ms,
        actual_end_ms: None,
        is_active: false,
        is_gap: true,
        tokens: Tokens::default(),
        message_count: 0,
        models: Vec::new(),
    }
}

fn floor_to_hour(ts_ms: i64) -> i64 {
    ts_ms - ts_ms.rem_euclid(MS_PER_HOUR)
}
