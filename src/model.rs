use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_creation += other.cache_creation;
        self.cache_read += other.cache_read;
    }

    /// F3: input + output + cache_creation by default; cache_read only when opted in.
    pub fn total(&self, include_cache_read: bool) -> u64 {
        self.input
            + self.output
            + self.cache_creation
            + if include_cache_read { self.cache_read } else { 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LiveStatus {
    Active,
    Idle,
    Inactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub session_id: String,
    pub name: Option<String>,
    pub cwd: Option<PathBuf>,
    pub pid: Option<i32>,
    pub status: LiveStatus,
    pub tokens: Tokens,
    pub subagent_tokens: Tokens,
    pub subagent_count: usize,
    pub model: Option<String>,
    pub updated_at_ms: Option<i64>,
    /// Filled by the CLI after join, using the pricing table. None when
    /// the model is unknown / synthetic / has no transcript data.
    pub cost_usd: Option<f64>,
}

/// A single assistant-message-with-usage event. Used for time-based
/// analysis (5-hour blocks, daily rollups, burn rate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub timestamp_ms: i64,
    pub tokens: Tokens,
    pub model: Option<String>,
}
