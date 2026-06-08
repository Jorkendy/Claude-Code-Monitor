// Per-million-token USD pricing.
//
// Defaults sourced from platform.claude.com/docs/en/about-claude/pricing
// (snapshot 2026-06). Users can override / extend via
// `~/.config/tokenscope/pricing.toml`:
//
//   [models.claude-opus-4-8]
//   input = 5.0
//   output = 25.0
//   cache_write = 6.25
//   cache_read = 0.50
//
// `cache_write` here is the 5-minute rate (1.25x base input). The 1-hour
// rate is 2x base input. Our 4-field usage summary cannot distinguish 5m
// vs 1h portions, so cost is a slight underestimate when sessions used
// 1h caching. This is part of the "ước tính" disclaimer in F5.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

use crate::model::Tokens;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

pub fn default_table() -> HashMap<String, ModelPricing> {
    let mut m = HashMap::new();
    let opus = ModelPricing {
        input: 5.0,
        output: 25.0,
        cache_write: 6.25,
        cache_read: 0.50,
    };
    let opus_legacy = ModelPricing {
        input: 15.0,
        output: 75.0,
        cache_write: 18.75,
        cache_read: 1.50,
    };
    let sonnet = ModelPricing {
        input: 3.0,
        output: 15.0,
        cache_write: 3.75,
        cache_read: 0.30,
    };
    let haiku = ModelPricing {
        input: 1.0,
        output: 5.0,
        cache_write: 1.25,
        cache_read: 0.10,
    };
    let haiku_35 = ModelPricing {
        input: 0.80,
        output: 4.0,
        cache_write: 1.0,
        cache_read: 0.08,
    };
    for k in [
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-opus-4-5",
    ] {
        m.insert(k.into(), opus);
    }
    for k in ["claude-opus-4-1", "claude-opus-4"] {
        m.insert(k.into(), opus_legacy);
    }
    for k in ["claude-sonnet-4-6", "claude-sonnet-4-5", "claude-sonnet-4"] {
        m.insert(k.into(), sonnet);
    }
    m.insert("claude-haiku-4-5".into(), haiku);
    m.insert("claude-haiku-3-5".into(), haiku_35);
    m
}

pub fn load() -> HashMap<String, ModelPricing> {
    let mut table = default_table();
    if let Some(overrides) = load_overrides() {
        for (k, v) in overrides {
            table.insert(k, v);
        }
    }
    table
}

#[derive(Deserialize)]
struct OverrideFile {
    #[serde(default)]
    models: HashMap<String, ModelPricing>,
}

fn load_overrides() -> Option<HashMap<String, ModelPricing>> {
    let path = dirs::config_dir()?.join("tokenscope/pricing.toml");
    let text = fs::read_to_string(&path).ok()?;
    let parsed: OverrideFile = toml::from_str(&text).ok()?;
    Some(parsed.models)
}

pub fn lookup<'a>(
    table: &'a HashMap<String, ModelPricing>,
    model: &str,
) -> Option<&'a ModelPricing> {
    if model.is_empty() || model == "<synthetic>" {
        return None;
    }
    table.get(model)
}

pub fn cost_usd(p: &ModelPricing, t: &Tokens) -> f64 {
    (t.input as f64 * p.input
        + t.output as f64 * p.output
        + t.cache_creation as f64 * p.cache_write
        + t.cache_read as f64 * p.cache_read)
        / 1_000_000.0
}

/// Context window limit by model. All current Claude models are 200K. The
/// 1M-token Opus variant is opt-in via API header and currently has no
/// distinct model id in `~/.claude` data, so we treat 200K as the safe
/// default. If we ever surface 1M sessions, add a per-model override here.
pub const DEFAULT_CONTEXT_LIMIT: u64 = 200_000;

pub fn context_limit_for(_model: &str) -> u64 {
    DEFAULT_CONTEXT_LIMIT
}
