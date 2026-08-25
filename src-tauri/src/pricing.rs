//! Token aggregation for the "단비 자체 LLM 사용량" dashboard.
//!
//! Token counts come from the usage log written by `crate::usage`. This
//! module folds those events by role into a summary the frontend renders.
//! (Cost/KRW estimation was removed in v0.8.0 — we display token amounts
//! only.)

use crate::usage::{self, UsageEvent};
use serde::Serialize;

/// One usage bucket in the dashboard summary. `role` is the same tag the
/// call site passed to `usage::with_role`.
#[derive(Debug, Clone, Serialize)]
pub struct RoleSummary {
    pub role: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Most-used model in this bucket — lets the UI show "Opus 4.7" etc.
    /// next to the role name.
    pub top_model: Option<String>,
}

/// Period-aggregated usage summary returned to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    /// Start of the aggregation window (inclusive), unix-ms.
    pub from_ms: i64,
    /// End of the window (exclusive), unix-ms.
    pub to_ms: i64,
    /// Total input tokens in the window.
    pub total_input_tokens: u64,
    /// Total output tokens in the window.
    pub total_output_tokens: u64,
    /// Aggregated by role, sorted by total tokens descending.
    pub by_role: Vec<RoleSummary>,
    /// Call count in the window.
    pub calls: u64,
}

/// Aggregate usage events inside `[from_ms, to_ms)`, folding by role.
pub fn summarize(events: &[UsageEvent], from_ms: i64, to_ms: i64) -> UsageSummary {
    use std::collections::HashMap;
    struct Acc {
        input: u64,
        output: u64,
        model_counts: HashMap<String, u64>,
    }
    let mut by_role: HashMap<String, Acc> = HashMap::new();
    let mut total_input = 0u64;
    let mut total_output = 0u64;
    let mut calls = 0u64;

    for ev in events {
        if ev.ts_ms < from_ms || ev.ts_ms >= to_ms {
            continue;
        }
        calls += 1;
        total_input += ev.input_tokens as u64;
        total_output += ev.output_tokens as u64;
        let acc = by_role.entry(ev.role.clone()).or_insert_with(|| Acc {
            input: 0,
            output: 0,
            model_counts: HashMap::new(),
        });
        acc.input += ev.input_tokens as u64;
        acc.output += ev.output_tokens as u64;
        *acc.model_counts.entry(ev.model.clone()).or_insert(0) += 1;
    }

    let mut rows: Vec<RoleSummary> = by_role
        .into_iter()
        .map(|(role, acc)| RoleSummary {
            role,
            input_tokens: acc.input,
            output_tokens: acc.output,
            top_model: acc
                .model_counts
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .map(|(m, _)| m),
        })
        .collect();
    rows.sort_by(|a, b| {
        (b.input_tokens + b.output_tokens).cmp(&(a.input_tokens + a.output_tokens))
    });

    UsageSummary {
        from_ms,
        to_ms,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        by_role: rows,
        calls,
    }
}
