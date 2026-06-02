//! Aggregations for the "MCP 저장 토큰" dashboard panel.
//!
//! Reads `usage.jsonl` (filtered to `provider == "mcp"`) and folds the
//! events into UI-ready summaries at three resolutions:
//!
//! 1. Vault-wide  → `summarize_vault(range)`     — total + per-project + daily series
//! 2. Per-project → `summarize_project(p, range)` — per-domain + daily + per-client
//! 3. Per-domain  → `summarize_domain(p, d, range)` — daily + per-client + per-tool
//!
//! The numbers here are *estimates* of the content tokens that crossed the
//! MCP boundary into the vault. They are NOT the same as the LLM-billed
//! tokens for the calling agent's session — see the disclaimer carried in
//! every payload.
//!
//! ## Caching
//!
//! Loading 50k JSONL lines on every dashboard render is wasteful, so we
//! keep an in-memory cache keyed by file size + mtime. A change to either
//! triggers a full reload (still cheap — ~50 MB ceiling). Concurrent
//! readers share a `RwLock`. We don't need a SQLite mirror at this scale.

use crate::pricing;
use crate::usage::{self, UsageEvent, MCP_PROVIDER};
use chrono::{Local, NaiveDate, TimeZone, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// One day in milliseconds — used for date math throughout.
const DAY_MS: i64 = 86_400_000;

/// Range shorthand the frontend passes. We resolve `from_ms` from "now"
/// in UTC; "today" snaps to local midnight so the user's "today" in
/// Seoul lines up with the daily note path.
#[derive(Debug, Clone, Copy)]
pub enum Range {
    Today,
    Days7,
    Days30,
    Days90,
    All,
}

impl Range {
    pub fn parse(s: &str) -> Range {
        match s {
            "today" => Range::Today,
            "7d" => Range::Days7,
            "30d" => Range::Days30,
            "90d" => Range::Days90,
            "all" => Range::All,
            _ => Range::Days7,
        }
    }

    /// Returns `(from_ms, to_ms)` window. `to_ms` is "now + 1ms" so the
    /// range is inclusive of in-flight events without race conditions.
    pub fn window(self) -> (i64, i64) {
        let now_ms = Utc::now().timestamp_millis();
        let to_ms = now_ms + 1;
        let from_ms = match self {
            Range::Today => local_midnight_ms(),
            Range::Days7 => now_ms - 7 * DAY_MS,
            Range::Days30 => now_ms - 30 * DAY_MS,
            Range::Days90 => now_ms - 90 * DAY_MS,
            Range::All => 0,
        };
        (from_ms, to_ms)
    }

    pub fn label(self) -> &'static str {
        match self {
            Range::Today => "today",
            Range::Days7 => "7d",
            Range::Days30 => "30d",
            Range::Days90 => "90d",
            Range::All => "all",
        }
    }
}

/// Local-midnight of "today" expressed as unix milliseconds. Falls back
/// to `Utc::now()` if the local timezone refuses the conversion (DST
/// transition, etc.) — better to over-include than to crash.
fn local_midnight_ms() -> i64 {
    let now = Local::now();
    let date = now.date_naive();
    match Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap_or_default())
        .single()
    {
        Some(dt) => dt.timestamp_millis(),
        None => Utc::now().timestamp_millis() - DAY_MS,
    }
}

// ---------- Output payloads -------------------------------------------

/// Disclaimer that every payload carries so the UI can render it
/// without hard-coding the wording in three places. Returned in Korean
/// because every other dashboard string in the app is.
const DISCLAIMER: &str = "이 숫자는 단비에 저장된 콘텐츠를 cl100k_base 토크나이저로 추정한 값입니다. Claude Code / Codex 세션이 실제 LLM에 청구한 토큰(시스템 프롬프트·대화 히스토리·tool 스키마 포함)과는 다릅니다. 정확한 청구액은 Anthropic / OpenAI 콘솔에서 확인하세요.";

#[derive(Debug, Serialize, Clone)]
pub struct ClientBreakdown {
    pub client: String,
    pub tokens: u64,
    pub calls: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct ToolBreakdown {
    pub tool: String,
    pub tokens: u64,
    pub calls: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DailyPoint {
    /// "YYYY-MM-DD" in local time.
    pub date: String,
    pub tokens: u64,
    pub calls: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProjectStats {
    pub project: String,
    pub tokens: u64,
    pub calls: u64,
    pub by_client: Vec<ClientBreakdown>,
    pub top_domains: Vec<DomainStub>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DomainStub {
    pub domain: String,
    pub tokens: u64,
    pub calls: u64,
}

/// Spike on a single domain — token volume that day was sufficiently
/// far above the rolling baseline that the user might want to know.
/// We surface these on the dashboard as a yellow callout next to the
/// top-contributors list.
#[derive(Debug, Serialize, Clone)]
pub struct Anomaly {
    pub project: String,
    pub domain: String,
    /// Date the spike happened (local "YYYY-MM-DD").
    pub date: String,
    /// Tokens written that day for this domain.
    pub tokens: u64,
    /// Rolling 7-day mean (tokens/day) for the same domain. The
    /// frontend computes the multiple from these two for display.
    pub baseline: f64,
    /// `tokens / baseline`, capped at 99.9 for ergonomics.
    pub multiple: f64,
}

/// Reference-only KRW estimate. The math uses the writer-tier price
/// (Sonnet 4.6 input by default) since users typically saved content
/// from a writer-class call. Marked `reference` because it does NOT
/// represent a billed amount — it's "if you re-fed this content as
/// input to Sonnet, it would cost roughly this much."
#[derive(Debug, Serialize, Clone)]
pub struct CostEstimate {
    pub model_stem: String,
    pub usd_per_mtok_input: f64,
    pub krw_per_usd: f64,
    pub krw: f64,
    pub usd: f64,
    /// Always `true` — surfaces in the UI as a "참고용 추정" tag.
    pub reference_only: bool,
}

/// One row of the "오늘의 Top Contributors" list — the 5 domains
/// (across the entire vault) that received the most tokens during
/// the selected window. Distinct from `by_project.top_domains` which
/// is per-project.
#[derive(Debug, Serialize, Clone)]
pub struct TopContributor {
    pub project: String,
    pub domain: String,
    pub tokens: u64,
    pub calls: u64,
}

/// 7×24 token heatmap — `cells[dow][hour]` is total tokens written in
/// that local-time bucket inside the selected window. `dow` is
/// 0=Sunday … 6=Saturday so the frontend matches the convention used
/// by every weekly calendar in the app. Empty windows return all
/// zeros, never `null`, so the renderer can iterate without a guard.
#[derive(Debug, Serialize, Clone)]
pub struct Heatmap {
    pub cells: Vec<Vec<u64>>,
    pub max_cell: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct VaultSummary {
    pub range: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub total_tokens: u64,
    pub total_calls: u64,
    pub by_client: Vec<ClientBreakdown>,
    pub by_tool: Vec<ToolBreakdown>,
    pub by_project: Vec<ProjectStats>,
    pub daily: Vec<DailyPoint>,
    /// 5 highest-token (project, domain) pairs in the window.
    pub top_contributors: Vec<TopContributor>,
    /// Spikes vs the rolling 7-day baseline. May be empty.
    pub anomalies: Vec<Anomaly>,
    /// Reference-only KRW estimate at the writer-tier price.
    pub cost_estimate: CostEstimate,
    /// 7×24 token heatmap by local day-of-week × hour-of-day.
    pub heatmap: Heatmap,
    /// Fixed Korean-language disclaimer. Always present — the UI must
    /// render it adjacent to any totals it displays.
    pub disclaimer: &'static str,
    /// Always `true` for now. Reserved for a future "validated" flag
    /// once we add Phase G's tokenizer-vs-billed comparison.
    pub estimated: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProjectDetail {
    pub project: String,
    pub range: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub total_tokens: u64,
    pub total_calls: u64,
    pub by_client: Vec<ClientBreakdown>,
    pub by_tool: Vec<ToolBreakdown>,
    pub by_domain: Vec<DomainStub>,
    pub daily: Vec<DailyPoint>,
    pub cost_estimate: CostEstimate,
    pub disclaimer: &'static str,
    pub estimated: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct DomainDetail {
    pub project: String,
    pub domain: String,
    pub range: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub total_tokens: u64,
    pub total_calls: u64,
    pub by_client: Vec<ClientBreakdown>,
    pub by_tool: Vec<ToolBreakdown>,
    pub daily: Vec<DailyPoint>,
    pub cost_estimate: CostEstimate,
    pub disclaimer: &'static str,
    pub estimated: bool,
}

// ---------- Cache ------------------------------------------------------

#[derive(Default)]
struct Cache {
    /// File size at the time of the last load.
    size: u64,
    /// File mtime (ms since epoch) at the time of the last load.
    mtime_ms: i64,
    /// Filtered set of MCP-inbound events. Already pre-filtered so the
    /// per-render fold doesn't have to test `provider` on every line.
    events: Vec<UsageEvent>,
}

static CACHE: OnceLock<RwLock<Cache>> = OnceLock::new();

fn cache_lock() -> &'static RwLock<Cache> {
    CACHE.get_or_init(|| RwLock::new(Cache::default()))
}

/// Read the raw JSONL log if it changed since last call, otherwise
/// reuse the cached vector. Filters down to `provider == "mcp"` while
/// loading so subsequent folds run on the smaller slice.
fn load_events() -> Vec<UsageEvent> {
    let path = match usage::usage_log_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let (size, mtime_ms) = match std::fs::metadata(&path) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            (m.len(), mtime)
        }
        Err(_) => return Vec::new(),
    };

    {
        let cache = cache_lock().read().unwrap();
        if cache.size == size && cache.mtime_ms == mtime_ms {
            return cache.events.clone();
        }
    }

    // Cache miss — reload. We fully reload rather than tail-incrementing
    // because the file is bounded (retention truncates it) and a full
    // reload is simpler to keep correct. ~50k lines parses in <100ms.
    let all = usage::load_all().unwrap_or_default();
    let filtered: Vec<UsageEvent> = all.into_iter()
        .filter(|e| e.provider == MCP_PROVIDER)
        .collect();
    let mut cache = cache_lock().write().unwrap();
    cache.size = size;
    cache.mtime_ms = mtime_ms;
    cache.events = filtered.clone();
    filtered
}

/// Drop the cache. Called from tests and from the retention sweep so a
/// shrunken file isn't served from a stale snapshot.
pub fn invalidate_cache() {
    let mut cache = cache_lock().write().unwrap();
    cache.size = 0;
    cache.mtime_ms = 0;
    cache.events.clear();
}

// ---------- Aggregation primitives ------------------------------------

fn within(ev: &UsageEvent, from_ms: i64, to_ms: i64) -> bool {
    ev.ts_ms >= from_ms && ev.ts_ms < to_ms
}

fn fold_by_client(events: &[&UsageEvent]) -> Vec<ClientBreakdown> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    for ev in events {
        let key = ev.client.clone().unwrap_or_else(|| "unknown".to_string());
        let entry = map.entry(key).or_insert((0, 0));
        entry.0 += ev.input_tokens as u64;
        entry.1 += 1;
    }
    let mut rows: Vec<ClientBreakdown> = map
        .into_iter()
        .map(|(client, (tokens, calls))| ClientBreakdown { client, tokens, calls })
        .collect();
    rows.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    rows
}

fn fold_by_tool(events: &[&UsageEvent]) -> Vec<ToolBreakdown> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    for ev in events {
        let key = ev.tool.clone().unwrap_or_else(|| "unknown".to_string());
        let entry = map.entry(key).or_insert((0, 0));
        entry.0 += ev.input_tokens as u64;
        entry.1 += 1;
    }
    let mut rows: Vec<ToolBreakdown> = map
        .into_iter()
        .map(|(tool, (tokens, calls))| ToolBreakdown { tool, tokens, calls })
        .collect();
    rows.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    rows
}

fn fold_by_domain(events: &[&UsageEvent]) -> Vec<DomainStub> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    for ev in events {
        let Some(d) = ev.domain.as_ref() else {
            continue;
        };
        let entry = map.entry(d.clone()).or_insert((0, 0));
        entry.0 += ev.input_tokens as u64;
        entry.1 += 1;
    }
    let mut rows: Vec<DomainStub> = map
        .into_iter()
        .map(|(domain, (tokens, calls))| DomainStub { domain, tokens, calls })
        .collect();
    rows.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    rows
}

/// Local-date string for an event timestamp, e.g. "2026-06-02".
fn local_date(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let nanos = ((ts_ms % 1000) * 1_000_000) as u32;
    match Utc.timestamp_opt(secs, nanos).single() {
        Some(dt) => dt.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        None => "1970-01-01".to_string(),
    }
}

/// Build a complete daily series from `from_ms` to `to_ms`, zero-filling
/// gaps so the chart doesn't break on quiet days.
fn fold_daily(events: &[&UsageEvent], from_ms: i64, to_ms: i64) -> Vec<DailyPoint> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    for ev in events {
        let key = local_date(ev.ts_ms);
        let entry = map.entry(key).or_insert((0, 0));
        entry.0 += ev.input_tokens as u64;
        entry.1 += 1;
    }

    // Walk the date range day-by-day in local time and emit a point for
    // every day, even ones with no events. The chart needs continuous
    // x-axis values to render gridlines correctly.
    let mut out: Vec<DailyPoint> = Vec::new();
    let start = local_date(from_ms);
    let end = local_date(to_ms.saturating_sub(1));
    let start_d = NaiveDate::parse_from_str(&start, "%Y-%m-%d").ok();
    let end_d = NaiveDate::parse_from_str(&end, "%Y-%m-%d").ok();
    if let (Some(mut d), Some(end_d)) = (start_d, end_d) {
        // Cap the daily series at 365 buckets so `range=all` on a year+
        // log doesn't return a huge payload. The chart UI needs no more
        // than a year of resolution anyway.
        let mut emitted = 0;
        while d <= end_d && emitted < 365 {
            let key = d.format("%Y-%m-%d").to_string();
            let (tokens, calls) = map.remove(&key).unwrap_or((0, 0));
            out.push(DailyPoint { date: key, tokens, calls });
            d = d.succ_opt().unwrap_or(end_d);
            emitted += 1;
            if d == end_d && emitted >= 365 {
                break;
            }
        }
    } else {
        // Fallback: emit just the buckets we saw, sorted.
        let mut keys: Vec<String> = map.keys().cloned().collect();
        keys.sort();
        for k in keys {
            let (tokens, calls) = map[&k];
            out.push(DailyPoint { date: k, tokens, calls });
        }
    }
    out
}

// ---------- Cost / contributors / anomalies ---------------------------

/// Default model stem used for the reference cost estimate. Sonnet 4.6
/// is the writer-tier default in Danbi's onboarding, so it's the most
/// representative price for "what would re-feeding this content cost?"
const COST_MODEL_STEM: &str = "claude-sonnet-4-6";

/// Build the reference-only cost estimate for a token total. Pulls the
/// USD→KRW rate out of `config.json` (falls back to the default
/// 1380 if the file is unreadable). Output tokens are zeroed because
/// MCP inbound never has output.
fn cost_estimate_for(total_tokens: u64) -> CostEstimate {
    let krw_per_usd = match crate::config::default_vault_path() {
        Ok(vault) => match crate::config::load_config(&vault) {
            Ok(Some(cfg)) => cfg.usage.krw_per_usd,
            _ => 1_380.0,
        },
        Err(_) => 1_380.0,
    };
    let price = pricing::lookup(COST_MODEL_STEM);
    let usd = (total_tokens as f64 / 1_000_000.0) * price.input;
    let krw = usd * krw_per_usd;
    CostEstimate {
        model_stem: COST_MODEL_STEM.to_string(),
        usd_per_mtok_input: price.input,
        krw_per_usd,
        krw,
        usd,
        reference_only: true,
    }
}

/// Pick the top 5 (project, domain) pairs by token count.
fn top_contributors(events: &[&UsageEvent]) -> Vec<TopContributor> {
    let mut map: HashMap<(String, String), (u64, u64)> = HashMap::new();
    for ev in events {
        let Some(domain) = ev.domain.as_ref() else {
            continue;
        };
        let project = ev.project.clone().unwrap_or_else(|| "(unscoped)".to_string());
        let entry = map
            .entry((project, domain.clone()))
            .or_insert((0, 0));
        entry.0 += ev.input_tokens as u64;
        entry.1 += 1;
    }
    let mut rows: Vec<TopContributor> = map
        .into_iter()
        .map(|((project, domain), (tokens, calls))| TopContributor {
            project,
            domain,
            tokens,
            calls,
        })
        .collect();
    rows.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    rows.truncate(5);
    rows
}

/// Detect spikes vs the trailing 7-day baseline for each (project,
/// domain) pair. We bucket events by local day, walk each domain's
/// time series, and emit an `Anomaly` whenever a single day's tokens
/// exceed `max(3 * baseline, baseline + 1000)`. The 1000-token floor
/// keeps tiny baselines from generating false positives ("yesterday
/// was 5 tokens, today is 30 tokens, 6× spike!").
///
/// Only spikes inside the requested window are returned. The baseline
/// itself is computed from the 7 days leading up to the window so a
/// `range=today` view still has 7 days of context.
fn anomalies_in_window(events: &[UsageEvent], from_ms: i64, to_ms: i64) -> Vec<Anomaly> {
    if events.is_empty() {
        return Vec::new();
    }
    // Bucket: (project, domain, date) -> tokens.
    let mut by_day: HashMap<(String, String, String), u64> = HashMap::new();
    for ev in events {
        let Some(domain) = ev.domain.as_ref() else {
            continue;
        };
        let project = ev.project.clone().unwrap_or_else(|| "(unscoped)".to_string());
        let date = local_date(ev.ts_ms);
        let entry = by_day
            .entry((project, domain.clone(), date))
            .or_insert(0);
        *entry += ev.input_tokens as u64;
    }

    // Reorganize into (project, domain) -> sorted Vec<(date, tokens)>.
    let mut by_pair: HashMap<(String, String), Vec<(String, u64)>> = HashMap::new();
    for ((p, d, day), v) in by_day {
        by_pair.entry((p, d)).or_default().push((day, v));
    }

    let from_date = local_date(from_ms);
    let to_date = local_date(to_ms.saturating_sub(1));

    let mut out: Vec<Anomaly> = Vec::new();
    for ((project, domain), mut series) in by_pair {
        series.sort_by(|a, b| a.0.cmp(&b.0));
        for i in 0..series.len() {
            let (ref day, tokens) = series[i];
            // Only consider days inside the user-selected window.
            if day.as_str() < from_date.as_str() || day.as_str() > to_date.as_str() {
                continue;
            }
            // Trailing 7-day baseline (excluding the current day).
            let lookback_start = i.saturating_sub(7);
            let trailing: Vec<u64> = series[lookback_start..i]
                .iter()
                .map(|(_, t)| *t)
                .collect();
            if trailing.is_empty() {
                continue;
            }
            let baseline = trailing.iter().copied().sum::<u64>() as f64
                / trailing.len() as f64;
            let threshold = (3.0 * baseline).max(baseline + 1000.0);
            if (tokens as f64) > threshold && tokens > 500 {
                let multiple = if baseline > 0.0 {
                    (tokens as f64 / baseline).min(99.9)
                } else {
                    99.9
                };
                out.push(Anomaly {
                    project: project.clone(),
                    domain: domain.clone(),
                    date: day.clone(),
                    tokens,
                    baseline,
                    multiple,
                });
            }
        }
    }
    // Surface the most dramatic spikes first.
    out.sort_by(|a, b| {
        b.multiple
            .partial_cmp(&a.multiple)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(5);
    out
}

/// Build the 7×24 heatmap. Treats every event in the slice as
/// already filtered to the user-selected range — caller decides the
/// window. We use `chrono::Datelike::weekday().num_days_from_sunday()`
/// so day 0 = Sunday, matching the rest of the app.
fn build_heatmap(events: &[&UsageEvent]) -> Heatmap {
    use chrono::{Datelike, Timelike};
    let mut cells: Vec<Vec<u64>> = vec![vec![0u64; 24]; 7];
    let mut total: u64 = 0;
    let mut max_cell: u64 = 0;
    for ev in events {
        let secs = ev.ts_ms / 1000;
        let nanos = ((ev.ts_ms % 1000) * 1_000_000) as u32;
        let Some(dt) = Utc.timestamp_opt(secs, nanos).single() else {
            continue;
        };
        let local = dt.with_timezone(&Local);
        let dow = local.weekday().num_days_from_sunday() as usize;
        let hour = local.hour() as usize;
        if dow < 7 && hour < 24 {
            let v = ev.input_tokens as u64;
            cells[dow][hour] = cells[dow][hour].saturating_add(v);
            total = total.saturating_add(v);
            if cells[dow][hour] > max_cell {
                max_cell = cells[dow][hour];
            }
        }
    }
    Heatmap { cells, max_cell, total_tokens: total }
}

// ---------- Public aggregations ---------------------------------------

/// Vault-wide summary across every project. Top-level dashboard card.
pub fn summarize_vault(range: Range) -> VaultSummary {
    let (from_ms, to_ms) = range.window();
    let events = load_events();
    let in_range: Vec<&UsageEvent> =
        events.iter().filter(|e| within(e, from_ms, to_ms)).collect();

    let total_tokens: u64 = in_range.iter().map(|e| e.input_tokens as u64).sum();
    let total_calls = in_range.len() as u64;

    // Per-project fold.
    let mut by_proj: HashMap<String, Vec<&UsageEvent>> = HashMap::new();
    for ev in &in_range {
        let key = ev.project.clone().unwrap_or_else(|| "(unscoped)".to_string());
        by_proj.entry(key).or_default().push(ev);
    }
    let mut by_project: Vec<ProjectStats> = by_proj
        .into_iter()
        .map(|(project, evs)| {
            let tokens: u64 = evs.iter().map(|e| e.input_tokens as u64).sum();
            let calls = evs.len() as u64;
            let by_client = fold_by_client(&evs);
            let mut top_domains = fold_by_domain(&evs);
            top_domains.truncate(5);
            ProjectStats { project, tokens, calls, by_client, top_domains }
        })
        .collect();
    by_project.sort_by(|a, b| b.tokens.cmp(&a.tokens));

    let top_contributors = top_contributors(&in_range);
    let anomalies = anomalies_in_window(&events, from_ms, to_ms);
    let cost_estimate = cost_estimate_for(total_tokens);
    let heatmap = build_heatmap(&in_range);

    VaultSummary {
        range: range.label().to_string(),
        from_ms,
        to_ms,
        total_tokens,
        total_calls,
        by_client: fold_by_client(&in_range),
        by_tool: fold_by_tool(&in_range),
        by_project,
        daily: fold_daily(&in_range, from_ms, to_ms),
        top_contributors,
        anomalies,
        cost_estimate,
        heatmap,
        disclaimer: DISCLAIMER,
        estimated: true,
    }
}

/// Project drill-down: per-domain, per-client, daily series.
pub fn summarize_project(project: &str, range: Range) -> ProjectDetail {
    let (from_ms, to_ms) = range.window();
    let events = load_events();
    let in_range: Vec<&UsageEvent> = events
        .iter()
        .filter(|e| within(e, from_ms, to_ms))
        .filter(|e| {
            e.project.as_deref().map(|p| p == project).unwrap_or(false)
        })
        .collect();

    let total_tokens: u64 = in_range.iter().map(|e| e.input_tokens as u64).sum();
    let total_calls = in_range.len() as u64;

    ProjectDetail {
        project: project.to_string(),
        range: range.label().to_string(),
        from_ms,
        to_ms,
        total_tokens,
        total_calls,
        by_client: fold_by_client(&in_range),
        by_tool: fold_by_tool(&in_range),
        by_domain: fold_by_domain(&in_range),
        daily: fold_daily(&in_range, from_ms, to_ms),
        cost_estimate: cost_estimate_for(total_tokens),
        disclaimer: DISCLAIMER,
        estimated: true,
    }
}

/// Domain drill-down: per-client, per-tool, daily series for the file.
pub fn summarize_domain(project: &str, domain: &str, range: Range) -> DomainDetail {
    let (from_ms, to_ms) = range.window();
    let events = load_events();
    let in_range: Vec<&UsageEvent> = events
        .iter()
        .filter(|e| within(e, from_ms, to_ms))
        .filter(|e| {
            e.project.as_deref() == Some(project)
                && e.domain.as_deref() == Some(domain)
        })
        .collect();

    let total_tokens: u64 = in_range.iter().map(|e| e.input_tokens as u64).sum();
    let total_calls = in_range.len() as u64;

    DomainDetail {
        project: project.to_string(),
        domain: domain.to_string(),
        range: range.label().to_string(),
        from_ms,
        to_ms,
        total_tokens,
        total_calls,
        by_client: fold_by_client(&in_range),
        by_tool: fold_by_tool(&in_range),
        daily: fold_daily(&in_range, from_ms, to_ms),
        cost_estimate: cost_estimate_for(total_tokens),
        disclaimer: DISCLAIMER,
        estimated: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts_ms: i64, project: &str, domain: &str, client: &str, tool: &str, tokens: u32) -> UsageEvent {
        UsageEvent {
            ts_ms,
            provider: MCP_PROVIDER.to_string(),
            role: usage::MCP_ROLE.to_string(),
            model: usage::MCP_TOKENIZER.to_string(),
            input_tokens: tokens,
            output_tokens: 0,
            client: Some(client.to_string()),
            project: Some(project.to_string()),
            domain: Some(domain.to_string()),
            tool: Some(tool.to_string()),
            raw_user_agent: None,
        }
    }

    #[test]
    fn fold_by_client_sums_and_sorts() {
        let events = vec![
            ev(1, "a", "x.md", "claude_code", "danbi_log", 100),
            ev(2, "a", "x.md", "codex", "danbi_log", 30),
            ev(3, "a", "x.md", "claude_code", "danbi_log", 50),
        ];
        let refs: Vec<&UsageEvent> = events.iter().collect();
        let rows = fold_by_client(&refs);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].client, "claude_code");
        assert_eq!(rows[0].tokens, 150);
        assert_eq!(rows[0].calls, 2);
        assert_eq!(rows[1].client, "codex");
        assert_eq!(rows[1].tokens, 30);
    }

    #[test]
    fn fold_by_tool_partitions() {
        let events = vec![
            ev(1, "a", "x.md", "claude_code", "danbi_log", 100),
            ev(2, "a", "y.md", "claude_code", "danbi_append", 200),
            ev(3, "a", "z.md", "claude_code", "danbi_log", 50),
        ];
        let refs: Vec<&UsageEvent> = events.iter().collect();
        let rows = fold_by_tool(&refs);
        // danbi_append (200) sorts before danbi_log (150).
        assert_eq!(rows[0].tool, "danbi_append");
        assert_eq!(rows[0].tokens, 200);
        assert_eq!(rows[1].tool, "danbi_log");
        assert_eq!(rows[1].tokens, 150);
    }

    #[test]
    fn fold_by_domain_skips_unset() {
        let e1 = ev(1, "a", "x.md", "claude_code", "danbi_log", 100);
        let e2 = ev(2, "a", "y.md", "claude_code", "danbi_log", 50);
        let e3 = ev(3, "a", "y.md", "claude_code", "danbi_log", 25);
        // Simulate a folder-create call where domain is None — should
        // not contribute to any per-domain bucket.
        let mut e4 = ev(4, "a", "ignored", "claude_code", "danbi_create_folder", 999);
        e4.domain = None;

        let events = vec![e1, e2, e3, e4];
        let refs: Vec<&UsageEvent> = events.iter().collect();
        let rows = fold_by_domain(&refs);
        assert_eq!(rows.len(), 2, "folder-create event should be skipped");
        // Sorted by tokens desc: x.md (100) first, y.md (75) second.
        assert_eq!(rows[0].domain, "x.md");
        assert_eq!(rows[0].tokens, 100);
        assert_eq!(rows[0].calls, 1);
        assert_eq!(rows[1].domain, "y.md");
        assert_eq!(rows[1].tokens, 75);
        assert_eq!(rows[1].calls, 2);
    }

    #[test]
    fn within_window_inclusive_left_exclusive_right() {
        let e = ev(100, "a", "x.md", "claude_code", "danbi_log", 1);
        assert!(within(&e, 100, 101));
        assert!(!within(&e, 101, 200));
        assert!(!within(&e, 0, 100));
    }

    #[test]
    fn range_today_starts_at_local_midnight() {
        let (from, _to) = Range::Today.window();
        // Sanity: from_ms should be within the last 25 hours of "now".
        let now = Utc::now().timestamp_millis();
        assert!(now - from <= 25 * DAY_MS / 24);
        assert!(from <= now);
    }

    #[test]
    fn disclaimer_is_present_in_summary() {
        // No events on disk yet but we still want the disclaimer to
        // appear so the empty state UI can render it.
        let s = summarize_vault(Range::Today);
        assert!(!s.disclaimer.is_empty());
        assert!(s.estimated);
    }

    #[test]
    fn top_contributors_truncates_to_five() {
        let mut events: Vec<UsageEvent> = Vec::new();
        // 7 distinct (project, domain) pairs with descending tokens.
        for i in 0..7u32 {
            events.push(ev(
                1000 + i as i64,
                "p",
                &format!("d{i}.md"),
                "claude_code",
                "danbi_log",
                100 + (10 - i) as u32 * 10,
            ));
        }
        let refs: Vec<&UsageEvent> = events.iter().collect();
        let top = top_contributors(&refs);
        assert_eq!(top.len(), 5);
        // Strict descending order.
        for w in top.windows(2) {
            assert!(w[0].tokens >= w[1].tokens);
        }
    }

    #[test]
    fn anomaly_threshold_requires_meaningful_volume() {
        // Steady tiny baseline (5 tokens/day for 7 days), then a 30
        // token day. Multiple is huge but absolute volume is below
        // 500 → must NOT alert (the floor protects us from noise).
        let mut events = Vec::new();
        for i in 0..7 {
            events.push(ev(
                (i as i64) * 86_400_000 + 100,
                "p",
                "x.md",
                "claude_code",
                "danbi_log",
                5,
            ));
        }
        events.push(ev(
            8 * 86_400_000 + 100,
            "p",
            "x.md",
            "claude_code",
            "danbi_log",
            30,
        ));
        let from = 7 * 86_400_000;
        let to = 9 * 86_400_000;
        let anomalies = anomalies_in_window(&events, from, to);
        assert!(
            anomalies.iter().all(|a| a.domain != "x.md"),
            "tiny-volume spikes should not trigger anomalies"
        );
    }

    #[test]
    fn anomaly_detects_real_spike() {
        // Steady 1000 tokens/day for 7 days, then a single 8000 day.
        // 8× the baseline AND well over 500 → should appear.
        let mut events = Vec::new();
        for i in 0..7 {
            events.push(ev(
                (i as i64) * 86_400_000 + 100,
                "p",
                "x.md",
                "claude_code",
                "danbi_log",
                1000,
            ));
        }
        events.push(ev(
            8 * 86_400_000 + 100,
            "p",
            "x.md",
            "claude_code",
            "danbi_log",
            8000,
        ));
        let from = 7 * 86_400_000;
        let to = 9 * 86_400_000;
        let anomalies = anomalies_in_window(&events, from, to);
        assert!(
            anomalies.iter().any(|a| a.domain == "x.md"),
            "8x spike should produce an anomaly: got {:?}",
            anomalies
        );
        let a = anomalies.iter().find(|a| a.domain == "x.md").unwrap();
        assert!(a.multiple > 3.0);
        assert_eq!(a.tokens, 8000);
    }

    #[test]
    fn cost_estimate_uses_sonnet_input_price() {
        // 1M tokens at Sonnet 4.6 input ($3/MTok) ≈ $3.
        let est = cost_estimate_for(1_000_000);
        assert!((est.usd - 3.0).abs() < 0.01, "got {}", est.usd);
        assert_eq!(est.model_stem, COST_MODEL_STEM);
        assert!(est.reference_only);
    }
}
