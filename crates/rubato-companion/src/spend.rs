//! Spend per session, summed from the Claude Code transcript named by the hook payload (A11).
//! Prices come from `workspace.yml`; nothing is guessed. The dashboard cross-checks against
//! `claude_code.cost.usage`.

use crate::config::ModelPrice;
use anyhow::Result;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct Line {
    /// One assistant message can span several lines while streaming; the last line per id wins.
    #[serde(rename = "requestId", default)]
    request_id: Option<String>,
    #[serde(default)]
    message: Option<Message>,
    /// Older transcripts carried a precomputed cost.
    #[serde(rename = "costUSD", default)]
    cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize, Default, Clone, Copy)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Spend {
    pub usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Models seen with no price configured; their tokens are counted but cost nothing here.
    pub unpriced_models: Vec<String>,
}

pub fn price_for<'a>(
    pricing: &'a BTreeMap<String, ModelPrice>,
    model: &str,
) -> Option<&'a ModelPrice> {
    pricing
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, p)| p)
}

pub fn sum_text(text: &str, pricing: &BTreeMap<String, ModelPrice>) -> Spend {
    let mut s = Spend::default();
    // Dedupe by message id (or request id): keep the last line seen for each.
    let mut by_key: Vec<(String, Line)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Ok(l) = serde_json::from_str::<Line>(line) else {
            continue;
        };
        let key = l
            .message
            .as_ref()
            .and_then(|m| m.id.clone())
            .or_else(|| l.request_id.clone())
            .unwrap_or_else(|| format!("line-{i}"));
        if let Some(slot) = by_key.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = l;
        } else {
            by_key.push((key, l));
        }
    }
    for (_, l) in by_key {
        if let Some(c) = l.cost_usd {
            s.usd += c;
        }
        let Some(m) = l.message else { continue };
        let Some(u) = m.usage else { continue };
        s.input_tokens += u.input_tokens;
        s.output_tokens += u.output_tokens;
        s.cache_read_tokens += u.cache_read_input_tokens;
        s.cache_write_tokens += u.cache_creation_input_tokens;
        if l.cost_usd.is_some() {
            continue;
        }
        let model = m.model.unwrap_or_default();
        match price_for(pricing, &model) {
            Some(p) => {
                s.usd += u.input_tokens as f64 * p.input / 1e6
                    + u.output_tokens as f64 * p.output / 1e6
                    + u.cache_read_input_tokens as f64 * p.cache_read / 1e6
                    + u.cache_creation_input_tokens as f64 * p.cache_write / 1e6;
            }
            None => {
                if !model.is_empty() && !s.unpriced_models.contains(&model) {
                    s.unpriced_models.push(model);
                }
            }
        }
    }
    s
}

pub fn sum_file(path: &Path, pricing: &BTreeMap<String, ModelPrice>) -> Result<Spend> {
    Ok(sum_text(&std::fs::read_to_string(path)?, pricing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_usage_with_longest_prefix_price() {
        let mut pricing = BTreeMap::new();
        pricing.insert(
            "claude-".into(),
            ModelPrice {
                input: 1.0,
                output: 2.0,
                cache_read: 0.1,
                cache_write: 1.25,
            },
        );
        pricing.insert(
            "claude-fable".into(),
            ModelPrice {
                input: 10.0,
                output: 20.0,
                cache_read: 1.0,
                cache_write: 12.5,
            },
        );
        let text = r#"{"type":"assistant","message":{"model":"claude-fable-5-1","usage":{"input_tokens":1000000,"output_tokens":0}}}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","usage":{"input_tokens":0,"output_tokens":1000000,"cache_read_input_tokens":1000000}}}
{"type":"user","message":{"role":"user"}}
not json
{"type":"assistant","costUSD":0.5,"message":{"model":"old","usage":{"input_tokens":7}}}
{"type":"assistant","message":{"model":"mystery-9","usage":{"input_tokens":5}}}"#;
        let s = sum_text(text, &pricing);
        assert!((s.usd - (10.0 + 2.0 + 0.1 + 0.5)).abs() < 1e-9, "{s:?}");
        assert_eq!(s.input_tokens, 1_000_012);
        assert_eq!(s.unpriced_models, vec!["mystery-9".to_string()]);
    }

    #[test]
    fn streamed_lines_for_one_message_count_once() {
        let mut pricing = BTreeMap::new();
        pricing.insert(
            "claude-".into(),
            ModelPrice {
                input: 1.0,
                output: 1.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        );
        let text = r#"{"requestId":"r1","message":{"id":"msg_1","model":"claude-x","usage":{"input_tokens":10,"output_tokens":1}}}
{"requestId":"r1","message":{"id":"msg_1","model":"claude-x","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let s = sum_text(text, &pricing);
        assert_eq!((s.input_tokens, s.output_tokens), (10, 5));
    }

    #[test]
    fn empty_pricing_counts_tokens_but_no_cost() {
        let s = sum_text(
            r#"{"message":{"model":"x","usage":{"input_tokens":3}}}"#,
            &BTreeMap::new(),
        );
        assert_eq!(s.usd, 0.0);
        assert_eq!(s.input_tokens, 3);
        assert_eq!(s.unpriced_models, vec!["x".to_string()]);
    }
}
