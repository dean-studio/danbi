//! Pricing table + KRW estimator.
//!
//! Each provider publishes token prices in USD per 1M tokens. We keep a
//! hand-maintained lookup table (reviewed quarterly) and a configurable
//! USD→KRW conversion rate. Token counts come from the usage log written
//! by `crate::usage`; this module only owns the math layer.
//!
//! Matching strategy (best → fallback):
//!   1. Exact model id
//!   2. Longest-prefix match against known id stems (so "us.anthropic.claude-sonnet-4-6-20250201-v1:0"
//!      still matches "claude-sonnet-4-6")
//!   3. Zero price (the model is unpriced — we show the token count only)

use crate::usage::{self, UsageEvent};
use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;

/// USD per 1M tokens, by token kind. Embedding models set everything but
/// `input` to 0. Cache fields are absolute prices — NOT a multiplier of
/// input — because Anthropic / Bedrock publish them that way and the
/// ratio doesn't always match (e.g. Bedrock's cross-region inference
/// has the same input price as Anthropic API for some models but
/// different cache prices).
///
/// `cache_5m` / `cache_1h` apply to `cache_creation_input_tokens` (the
/// transcript field). `cache_read` applies to `cache_read_input_tokens`.
/// When `cache_1h` is `None` the model doesn't support 1h caching —
/// fall back to `cache_5m`.
#[derive(Debug, Clone, Copy)]
pub struct PriceUsdPerMTok {
    pub input: f64,
    pub output: f64,
    pub cache_5m: f64,
    pub cache_1h: Option<f64>,
    pub cache_read: f64,
}

impl PriceUsdPerMTok {
    /// Convenience: a non-cache-aware entry (embedding / OpenAI / etc.)
    /// where we don't track cache breakdown. Cache fields default to a
    /// reasonable multiplier of input so legacy paths still produce
    /// non-zero estimates if cache tokens leak in.
    const fn flat(input: f64, output: f64) -> Self {
        Self {
            input,
            output,
            cache_5m: input * 1.25,
            cache_1h: None,
            cache_read: input * 0.10,
        }
    }
    /// Anthropic/Bedrock entry with explicit cache prices.
    const fn cached(
        input: f64,
        output: f64,
        cache_5m: f64,
        cache_1h: f64,
        cache_read: f64,
    ) -> Self {
        Self {
            input,
            output,
            cache_5m,
            cache_1h: Some(cache_1h),
            cache_read,
        }
    }
}

/// Hand-maintained price map. USD / 1M tokens. Sources checked 2026-05-21:
///   Anthropic/Bedrock: https://www.anthropic.com/pricing#api
///   OpenAI: https://openai.com/api/pricing
///   Bedrock Titan: https://aws.amazon.com/bedrock/pricing
///   Google Gemini: https://ai.google.dev/pricing
///   Voyage AI: https://docs.voyageai.com/docs/pricing
/// Update quarterly — CLAUDE.md note.
///
/// We list model stems (the distinctive middle) so both Anthropic-native
/// ("claude-sonnet-4-6") and Bedrock-prefixed ("us.anthropic.claude-sonnet-4-6-…")
/// ids collapse to the same entry.
// 가격 출처: AWS Bedrock Global Cross-region Inference (us-east-2 기준).
// 단비 사용자 (hckim) 가 Bedrock 으로 호출하므로 Bedrock 단가가 정답.
// Anthropic 직접 API 단가는 일부 모델에서 다름 — Anthropic-native
// 호출이 들어오면 따로 분기. 분기 시그널 = transcript 의 message.id
// (`msg_bdrk_` prefix 있으면 Bedrock).
//
// 분기마다 https://aws.amazon.com/bedrock/pricing 와 대조 갱신.
// CLAUDE.md "분기 가격 갱신" 노트 참고.
const TABLE: &[(&str, PriceUsdPerMTok)] = &[
    // ----- Claude 4.x (AWS Bedrock Global Cross-region, 2026-06-24 확인) -----
    ("claude-fable-5", PriceUsdPerMTok::cached(10.00, 50.00, 12.50, 20.00, 1.00)),
    ("claude-opus-4-8", PriceUsdPerMTok::cached(5.00, 25.00, 6.25, 10.00, 0.50)),
    ("claude-opus-4-7", PriceUsdPerMTok::cached(5.00, 25.00, 6.25, 10.00, 0.50)),
    ("claude-opus-4-6", PriceUsdPerMTok::cached(5.00, 25.00, 6.25, 10.00, 0.50)),
    ("claude-opus-4-5", PriceUsdPerMTok::cached(5.00, 25.00, 6.25, 10.00, 0.50)),
    ("claude-sonnet-4-6", PriceUsdPerMTok::cached(3.00, 15.00, 3.75, 6.00, 0.30)),
    ("claude-sonnet-4-5", PriceUsdPerMTok::cached(3.00, 15.00, 3.75, 6.00, 0.30)),
    ("claude-haiku-4-5", PriceUsdPerMTok::cached(1.00, 5.00, 1.25, 2.00, 0.10)),
    ("claude-haiku-3-5", PriceUsdPerMTok::flat(0.80, 4.00)),
    // OpenAI GPT-4.x / o-series (rough reference; users may override later)
    ("gpt-4o-mini", PriceUsdPerMTok::flat(0.15, 0.60)),
    ("gpt-4o", PriceUsdPerMTok::flat(2.50, 10.00)),
    ("gpt-4.1-mini", PriceUsdPerMTok::flat(0.40, 1.60)),
    ("gpt-4.1", PriceUsdPerMTok::flat(2.00, 8.00)),
    ("o4-mini", PriceUsdPerMTok::flat(1.10, 4.40)),
    // Embedding models
    ("titan-embed-text-v2", PriceUsdPerMTok::flat(0.02, 0.0)),
    ("titan-embed-text-v1", PriceUsdPerMTok::flat(0.10, 0.0)),
    ("text-embedding-3-small", PriceUsdPerMTok::flat(0.02, 0.0)),
    ("text-embedding-3-large", PriceUsdPerMTok::flat(0.13, 0.0)),
    // Gemini — 2.5 family (current as of 2026-05). flash 가 무료 티어에서
    // 충분해 단비 기본 권장. embedding-001 은 무료 티어 한도 내 최대 1500/min.
    ("gemini-2.5-flash", PriceUsdPerMTok::flat(0.30, 2.50)),
    ("gemini-2.5-pro", PriceUsdPerMTok::flat(1.25, 10.00)),
    ("gemini-2.0-flash", PriceUsdPerMTok::flat(0.10, 0.40)),
    ("gemini-1.5-pro", PriceUsdPerMTok::flat(1.25, 5.00)),
    ("gemini-embedding-001", PriceUsdPerMTok::flat(0.0, 0.0)),
    // Voyage — 200M tokens/month free, 그 이후 유료. 단비는 거의 무료
    // 티어 안에 들어가지만 표에 0 으로 두면 사용량 트래킹 자체를 못 해서
    // 명목 가격만 등록.
    ("voyage-multilingual-2", PriceUsdPerMTok::flat(0.12, 0.0)),
    ("voyage-3", PriceUsdPerMTok::flat(0.06, 0.0)),
    ("voyage-3-lite", PriceUsdPerMTok::flat(0.02, 0.0)),
    ("voyage-code-3", PriceUsdPerMTok::flat(0.18, 0.0)),
];

/// Look up the price for a given model id. Returns zero-price when no stem
/// matches, so unpriced-but-logged calls still contribute a token count.
pub fn lookup(model_id: &str) -> PriceUsdPerMTok {
    let id = model_id.to_lowercase();
    let mut best: Option<(usize, PriceUsdPerMTok)> = None;
    for (stem, price) in TABLE {
        if id.contains(stem) {
            let len = stem.len();
            if best.map(|(l, _)| len > l).unwrap_or(true) {
                best = Some((len, *price));
            }
        }
    }
    best.map(|(_, p)| p).unwrap_or(PriceUsdPerMTok::flat(0.0, 0.0))
}

/// One usage bucket in the dashboard summary. `role` is the same tag the
/// call site passed to `usage::with_role`.
#[derive(Debug, Clone, Serialize)]
pub struct RoleSummary {
    pub role: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub krw: f64,
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
    /// Total spend in KRW.
    pub total_krw: f64,
    /// Exchange rate that was applied.
    pub krw_per_usd: f64,
    /// Aggregated by role, sorted by KRW descending.
    pub by_role: Vec<RoleSummary>,
    /// Call count in the window.
    pub calls: u64,
}

/// Convert input+output tokens at a given model price into KRW.
fn krw_for(
    input_tokens: u64,
    output_tokens: u64,
    price: PriceUsdPerMTok,
    krw_per_usd: f64,
) -> f64 {
    let usd = (input_tokens as f64 / 1_000_000.0) * price.input
        + (output_tokens as f64 / 1_000_000.0) * price.output;
    usd * krw_per_usd
}

/// Convenience: KRW estimate for a single call using the current rate.
pub fn estimate_call_krw(
    model_id: &str,
    input_tokens: u64,
    output_tokens: u64,
    krw_per_usd: f64,
) -> f64 {
    krw_for(input_tokens, output_tokens, lookup(model_id), krw_per_usd)
}

/// Aggregate usage events inside `[from_ms, to_ms)`. Feed every recorded
/// event through the lookup table and fold by role.
pub fn summarize(
    events: &[UsageEvent],
    from_ms: i64,
    to_ms: i64,
    krw_per_usd: f64,
) -> UsageSummary {
    use std::collections::HashMap;
    struct Acc {
        input: u64,
        output: u64,
        krw: f64,
        model_counts: HashMap<String, u64>,
    }
    let mut by_role: HashMap<String, Acc> = HashMap::new();
    let mut total_krw = 0.0;
    let mut calls = 0u64;

    for ev in events {
        if ev.ts_ms < from_ms || ev.ts_ms >= to_ms {
            continue;
        }
        calls += 1;
        let price = lookup(&ev.model);
        let krw = krw_for(
            ev.input_tokens as u64,
            ev.output_tokens as u64,
            price,
            krw_per_usd,
        );
        total_krw += krw;
        let acc = by_role.entry(ev.role.clone()).or_insert_with(|| Acc {
            input: 0,
            output: 0,
            krw: 0.0,
            model_counts: HashMap::new(),
        });
        acc.input += ev.input_tokens as u64;
        acc.output += ev.output_tokens as u64;
        acc.krw += krw;
        *acc.model_counts.entry(ev.model.clone()).or_insert(0) += 1;
    }

    let mut rows: Vec<RoleSummary> = by_role
        .into_iter()
        .map(|(role, acc)| RoleSummary {
            role,
            input_tokens: acc.input,
            output_tokens: acc.output,
            krw: acc.krw,
            top_model: acc
                .model_counts
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .map(|(m, _)| m),
        })
        .collect();
    rows.sort_by(|a, b| b.krw.partial_cmp(&a.krw).unwrap_or(std::cmp::Ordering::Equal));

    UsageSummary {
        from_ms,
        to_ms,
        total_krw,
        krw_per_usd,
        by_role: rows,
        calls,
    }
}

/// Start of the current calendar month in UTC, as unix milliseconds.
pub fn current_month_start_ms() -> i64 {
    let now: DateTime<Utc> = Utc::now();
    let start = Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now);
    start.timestamp_millis()
}

use chrono::TimeZone;

/// Load the usage log and build the MTD summary at the supplied FX rate.
pub fn month_to_date(krw_per_usd: f64) -> std::io::Result<UsageSummary> {
    let events = usage::load_all()?;
    let from = current_month_start_ms();
    let to = Utc::now().timestamp_millis() + 1;
    Ok(summarize(&events, from, to, krw_per_usd))
}
