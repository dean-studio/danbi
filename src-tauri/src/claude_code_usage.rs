//! Claude Code 사용량 — `~/.claude/projects/**/*.jsonl` 파서 + 집계.
//!
//! Claude Code 가 매 어시스턴트 응답마다 jsonl 한 줄을 자기 디스크에
//! 기록한다. 그 안의 `message.usage` 필드에는 input/output/cache 토큰이
//! 정확히 박혀 있어, 우리는 OAuth endpoint 호출 없이도 사용량을
//! 100% 복원할 수 있다.
//!
//! ## 파일 구조
//!
//! ```text
//! ~/.claude/projects/
//!   └── -Users-hazbola-works-agent-danbi/   ← cwd 인코딩 (/ → -)
//!         ├── <session-uuid>.jsonl
//!         └── ...
//! ```
//!
//! 각 jsonl 라인 (assistant 메시지) 형식:
//!
//! ```json
//! {
//!   "type": "assistant",
//!   "timestamp": "2026-06-08T13:24:10.753Z",
//!   "cwd": "/Users/hazbola/works/agent/danbi",
//!   "gitBranch": "main",
//!   "sessionId": "...",
//!   "message": {
//!     "model": "claude-opus-4-7",
//!     "id": "msg_bdrk_...",   ← "msg_bdrk_" prefix = Bedrock 응답
//!     "usage": {
//!       "input_tokens": 6,
//!       "output_tokens": 204,
//!       "cache_creation_input_tokens": 22745,
//!       "cache_read_input_tokens": 20251
//!     }
//!   }
//! }
//! ```
//!
//! ## 모드 감지
//!
//! - `message.id` 가 `msg_bdrk_` 로 시작 → Bedrock 모드 (종량형)
//! - 그 외 → Anthropic API 모드 (구독형 또는 API key — id 만으로는 구분 불가)
//!
//! ## 캐싱
//!
//! mcp_inbound 와 동일 패턴. (path, mtime, size) 키로 in-memory cache,
//! 변경된 파일만 재파싱. 단비 첫 실행 시 풀스캔, 이후엔 mtime 비교만.

use crate::pricing::{self, PriceUsdPerMTok};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::SystemTime;

const DAY_MS: i64 = 86_400_000;

/// 정규화된 한 건의 어시스턴트 응답 사용량.
#[derive(Debug, Clone, Serialize)]
pub struct CcEvent {
    pub ts_ms: i64,
    pub session_id: String,
    /// cwd 디렉토리 절대 경로 (e.g. `/Users/hazbola/works/agent/danbi`).
    pub cwd: String,
    pub git_branch: Option<String>,
    pub model: String,
    /// `bedrock` | `anthropic_api`. id prefix 로만 구분.
    pub backend: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// `cache_creation_input_tokens` 합계. 5m + 1h 가 합쳐진 값.
    pub cache_creation_tokens: u64,
    /// `cache_creation.ephemeral_5m_input_tokens` (있으면).
    pub cache_5m_tokens: u64,
    /// `cache_creation.ephemeral_1h_input_tokens` (있으면).
    /// 1h 캐시는 5m 보다 단가가 ~1.6× 비싸서 분리.
    pub cache_1h_tokens: u64,
    pub cache_read_tokens: u64,
}

impl CcEvent {
    /// 절대 단가 (Bedrock cross-region) 기반 USD 추정. cache_5m / cache_1h /
    /// cache_read 모두 가격표 절대값을 사용 — input × 비율 가정 폐기.
    /// transcript 가 1h 분리를 안 주면 (구버전 Claude Code) 모두 5m 으로 처리.
    fn usd(&self, p: PriceUsdPerMTok) -> f64 {
        let m = 1_000_000.0;
        let cc_5m = if self.cache_5m_tokens > 0 || self.cache_1h_tokens > 0 {
            self.cache_5m_tokens
        } else {
            // 분리 정보 없으면 전부 5m 로 가정 (보수적 — 5m 이 더 쌈).
            self.cache_creation_tokens
        };
        let cc_1h = self.cache_1h_tokens;
        let price_1h = p.cache_1h.unwrap_or(p.cache_5m);
        (self.input_tokens as f64 / m) * p.input
            + (self.output_tokens as f64 / m) * p.output
            + (cc_5m as f64 / m) * p.cache_5m
            + (cc_1h as f64 / m) * price_1h
            + (self.cache_read_tokens as f64 / m) * p.cache_read
    }
}

// ---------- jsonl 파싱 ----------

#[derive(Deserialize)]
struct RawLine {
    #[serde(rename = "type")]
    ty: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    /// v0.7.x Claude Code: 5m / 1h 캐시 분리 breakdown.
    /// 없거나 한쪽만 0 인 케이스 정상.
    #[serde(default)]
    cache_creation: Option<RawCacheBreakdown>,
}

#[derive(Deserialize, Default)]
struct RawCacheBreakdown {
    #[serde(default)]
    ephemeral_5m_input_tokens: i64,
    #[serde(default)]
    ephemeral_1h_input_tokens: i64,
}

fn parse_line(line: &str) -> Option<CcEvent> {
    let raw: RawLine = serde_json::from_str(line).ok()?;
    if raw.ty.as_deref() != Some("assistant") {
        return None;
    }
    let msg = raw.message?;
    let usage = msg.usage?;
    let model = msg.model?;
    let id = msg.id.unwrap_or_default();
    let backend = if id.starts_with("msg_bdrk_") {
        "bedrock"
    } else {
        "anthropic_api"
    };
    let ts = raw.timestamp.as_deref()?;
    let ts_ms = DateTime::parse_from_rfc3339(ts)
        .ok()?
        .with_timezone(&Utc)
        .timestamp_millis();
    let bd = usage.cache_creation.unwrap_or_default();
    Some(CcEvent {
        ts_ms,
        session_id: raw.session_id.unwrap_or_default(),
        cwd: raw.cwd.unwrap_or_default(),
        git_branch: raw.git_branch,
        model,
        backend: backend.to_string(),
        input_tokens: usage.input_tokens.max(0) as u64,
        output_tokens: usage.output_tokens.max(0) as u64,
        cache_creation_tokens: usage.cache_creation_input_tokens.max(0) as u64,
        cache_5m_tokens: bd.ephemeral_5m_input_tokens.max(0) as u64,
        cache_1h_tokens: bd.ephemeral_1h_input_tokens.max(0) as u64,
        cache_read_tokens: usage.cache_read_input_tokens.max(0) as u64,
    })
}

// ---------- 디스크 walk + 캐시 ----------

fn projects_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join("projects"))
}

#[derive(Default)]
struct FileEntry {
    mtime_ms: i64,
    size: u64,
    events: Vec<CcEvent>,
}

#[derive(Default)]
struct Cache {
    files: HashMap<PathBuf, FileEntry>,
}

fn cache() -> &'static RwLock<Cache> {
    static C: OnceLock<RwLock<Cache>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(Cache::default()))
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn list_jsonl_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                walk(&path, out);
            } else if ft.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    if root.exists() {
        walk(root, &mut out);
    }
    out
}

fn parse_file(path: &Path) -> std::io::Result<Vec<CcEvent>> {
    let f = fs::File::open(path)?;
    let mut events = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(ev) = parse_line(&line) {
            events.push(ev);
        }
    }
    Ok(events)
}

/// 모든 transcript 를 메모리에 적재 (증분). 변경 없는 파일은 캐시 적중.
pub fn load_all() -> Vec<CcEvent> {
    let Some(root) = projects_dir() else {
        return Vec::new();
    };
    let files = list_jsonl_files(&root);

    // 1) 캐시 무효 항목만 다시 파싱.
    let mut to_reparse: Vec<(PathBuf, i64, u64)> = Vec::new();
    {
        let cache = cache().read().ok();
        for path in &files {
            let Ok(meta) = fs::metadata(path) else { continue };
            let m = mtime_ms(&meta);
            let s = meta.len();
            let needs = match cache.as_ref().and_then(|c| c.files.get(path)) {
                Some(entry) => entry.mtime_ms != m || entry.size != s,
                None => true,
            };
            if needs {
                to_reparse.push((path.clone(), m, s));
            }
        }
    }

    if !to_reparse.is_empty() {
        if let Ok(mut cache) = cache().write() {
            for (path, m, s) in to_reparse {
                let events = parse_file(&path).unwrap_or_default();
                cache.files.insert(
                    path,
                    FileEntry {
                        mtime_ms: m,
                        size: s,
                        events,
                    },
                );
            }
            // 사라진 파일은 캐시에서 제거.
            let alive: std::collections::HashSet<&Path> =
                files.iter().map(|p| p.as_path()).collect();
            cache.files.retain(|p, _| alive.contains(p.as_path()));
        }
    }

    // 2) 캐시에서 모은다.
    let mut all: Vec<CcEvent> = Vec::new();
    if let Ok(cache) = cache().read() {
        for entry in cache.files.values() {
            all.extend(entry.events.iter().cloned());
        }
    }
    all.sort_by_key(|e| e.ts_ms);
    all
}

/// 캐시 강제 무효화 — Settings 의 "다시 인덱싱" 버튼 등에서 호출.
pub fn invalidate_cache() {
    if let Ok(mut c) = cache().write() {
        c.files.clear();
    }
}

// ---------- 집계 출력 ----------

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
    pub fn window(self) -> (i64, i64) {
        let now = Utc::now().timestamp_millis();
        let to = now + 1;
        let from = match self {
            Range::Today => local_midnight_ms(),
            Range::Days7 => now - 7 * DAY_MS,
            Range::Days30 => now - 30 * DAY_MS,
            Range::Days90 => now - 90 * DAY_MS,
            Range::All => 0,
        };
        (from, to)
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

fn local_midnight_ms() -> i64 {
    let now = Local::now();
    let date = now.date_naive();
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap_or_default())
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(|| Utc::now().timestamp_millis() - DAY_MS)
}

fn day_label(ts_ms: i64) -> String {
    let dt = Local
        .timestamp_millis_opt(ts_ms)
        .single()
        .unwrap_or_else(|| Local::now());
    dt.format("%Y-%m-%d").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Totals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_tokens: u64,
    pub usd: f64,
    pub krw: f64,
    pub calls: u64,
    pub sessions: u64,
}

impl Totals {
    /// 다른 Totals 합치기 (월별 fold 등).
    fn merge(&mut self, other: &Totals) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.total_tokens += other.total_tokens;
        self.usd += other.usd;
        self.krw += other.krw;
        self.calls += other.calls;
        self.sessions += other.sessions;
    }

    fn add(&mut self, ev: &CcEvent, krw_per_usd: f64) {
        let p = pricing::lookup(&ev.model);
        let usd = ev.usd(p);
        self.input_tokens += ev.input_tokens;
        self.output_tokens += ev.output_tokens;
        self.cache_creation_tokens += ev.cache_creation_tokens;
        self.cache_read_tokens += ev.cache_read_tokens;
        self.total_tokens += ev.input_tokens
            + ev.output_tokens
            + ev.cache_creation_tokens
            + ev.cache_read_tokens;
        self.usd += usd;
        self.krw += usd * krw_per_usd;
        self.calls += 1;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelBreakdown {
    pub model: String,
    pub backend: String,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectBreakdown {
    /// cwd 절대 경로 (예: `/Users/hazbola/works/agent/danbi`).
    pub cwd: String,
    /// cwd 의 마지막 segment (보기 좋게 — `danbi`).
    pub label: String,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyPoint {
    pub date: String,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct HourlyHeatPoint {
    pub dow: u8,
    pub hour: u8,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CcSummary {
    pub from_ms: i64,
    pub to_ms: i64,
    pub range: String,
    pub krw_per_usd: f64,
    pub totals: Totals,
    pub by_model: Vec<ModelBreakdown>,
    pub by_project: Vec<ProjectBreakdown>,
    pub by_backend: HashMap<String, Totals>,
    pub daily: Vec<DailyPoint>,
    pub hourly: Vec<HourlyHeatPoint>,
    /// 오늘 vs 1년 전 오늘 비교 (히스토리 카드용).
    pub year_ago_today: Option<Totals>,
    /// 가장 비쌌던 날 Top 5 (range 내).
    pub top_days: Vec<DailyPoint>,
    /// 모드별 안내 문구.
    pub disclaimer: String,
}

const DISCLAIMER_BEDROCK_ONLY: &str =
    "이 숫자는 ~/.claude/projects 의 transcript 를 직접 읽어 계산한 값입니다. AWS Bedrock Global Cross-region 단가표 (5m / 1h / cache_read 절대 단가) 적용 — 실제 청구액과 거의 일치합니다.";
const DISCLAIMER_API: &str =
    "이 숫자는 ~/.claude/projects 의 transcript 를 직접 읽어 계산한 값입니다. Anthropic API key 모드는 실제 청구액에 가깝고, Pro/Max 구독 모드는 토큰만 의미가 있습니다 (구독료는 정액).";

fn pick_disclaimer(by_backend: &HashMap<String, Totals>) -> String {
    let only_bedrock = by_backend.contains_key("bedrock") && !by_backend.contains_key("anthropic_api");
    if only_bedrock {
        DISCLAIMER_BEDROCK_ONLY.to_string()
    } else {
        DISCLAIMER_API.to_string()
    }
}

pub fn summarize(range: Range, krw_per_usd: f64) -> CcSummary {
    let (from_ms, to_ms) = range.window();
    let events = load_all();
    summarize_events(&events, from_ms, to_ms, range.label(), krw_per_usd)
}

fn summarize_events(
    events: &[CcEvent],
    from_ms: i64,
    to_ms: i64,
    range_label: &str,
    krw_per_usd: f64,
) -> CcSummary {
    let mut totals = Totals::default();
    let mut by_model: HashMap<(String, String), Totals> = HashMap::new();
    let mut by_project: HashMap<String, Totals> = HashMap::new();
    let mut by_backend: HashMap<String, Totals> = HashMap::new();
    let mut by_day: BTreeMap<String, Totals> = BTreeMap::new();
    let mut by_hour: HashMap<(u8, u8), u64> = HashMap::new();
    let mut sessions: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ev in events {
        if ev.ts_ms < from_ms || ev.ts_ms >= to_ms {
            continue;
        }
        totals.add(ev, krw_per_usd);
        sessions.insert(ev.session_id.clone());

        let key = (ev.model.clone(), ev.backend.clone());
        by_model.entry(key).or_default().add(ev, krw_per_usd);

        by_project.entry(ev.cwd.clone()).or_default().add(ev, krw_per_usd);
        by_backend.entry(ev.backend.clone()).or_default().add(ev, krw_per_usd);

        let day = day_label(ev.ts_ms);
        by_day.entry(day).or_default().add(ev, krw_per_usd);

        let dt = Local
            .timestamp_millis_opt(ev.ts_ms)
            .single()
            .unwrap_or_else(|| Local::now());
        let dow = dt.weekday().num_days_from_sunday() as u8;
        let hour = dt.hour() as u8;
        let bucket = by_hour.entry((dow, hour)).or_insert(0);
        *bucket += ev.input_tokens
            + ev.output_tokens
            + ev.cache_creation_tokens
            + ev.cache_read_tokens;
    }
    totals.sessions = sessions.len() as u64;

    let mut by_model_vec: Vec<ModelBreakdown> = by_model
        .into_iter()
        .map(|((model, backend), totals)| ModelBreakdown { model, backend, totals })
        .collect();
    by_model_vec.sort_by(|a, b| b.totals.usd.partial_cmp(&a.totals.usd).unwrap_or(std::cmp::Ordering::Equal));

    let mut by_project_vec: Vec<ProjectBreakdown> = by_project
        .into_iter()
        .map(|(cwd, totals)| {
            let label = Path::new(&cwd)
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| cwd.clone());
            ProjectBreakdown { cwd, label, totals }
        })
        .collect();
    by_project_vec.sort_by(|a, b| b.totals.usd.partial_cmp(&a.totals.usd).unwrap_or(std::cmp::Ordering::Equal));
    by_project_vec.truncate(10);

    let daily: Vec<DailyPoint> = by_day
        .into_iter()
        .map(|(date, totals)| DailyPoint { date, totals })
        .collect();

    let mut top_days = daily.clone();
    top_days.sort_by(|a, b| b.totals.usd.partial_cmp(&a.totals.usd).unwrap_or(std::cmp::Ordering::Equal));
    top_days.truncate(5);

    let hourly: Vec<HourlyHeatPoint> = by_hour
        .into_iter()
        .map(|((dow, hour), tokens)| HourlyHeatPoint { dow, hour, tokens })
        .collect();

    let year_ago_today = compute_year_ago_today(events, krw_per_usd);
    let disclaimer = pick_disclaimer(&by_backend);

    CcSummary {
        from_ms,
        to_ms,
        range: range_label.to_string(),
        krw_per_usd,
        totals,
        by_model: by_model_vec,
        by_project: by_project_vec,
        by_backend,
        daily,
        hourly,
        year_ago_today,
        top_days,
        disclaimer,
    }
}

fn compute_year_ago_today(events: &[CcEvent], krw_per_usd: f64) -> Option<Totals> {
    let today = Local::now().date_naive();
    let target = NaiveDate::from_ymd_opt(today.year() - 1, today.month(), today.day())?;
    let target_str = target.format("%Y-%m-%d").to_string();
    let mut totals = Totals::default();
    let mut hit = false;
    for ev in events {
        if day_label(ev.ts_ms) == target_str {
            totals.add(ev, krw_per_usd);
            hit = true;
        }
    }
    hit.then_some(totals)
}

// ---------- 영속 히스토리 (v0.7.1) ----------
//
// 매번 transcript 풀스캔하면 1년치면 50MB+ 이 되어 카드 갱신 100ms+
// 들어감. 이를 피하려고 "어제까지" 의 일별 합계를 디스크에 영속.
//
// 전략:
//   1. 오늘만 transcript 실시간 집계
//   2. 어제 이전은 history.jsonl 에서 읽어옴 (이미 finalize 됨)
//   3. 캐시 miss 인 과거 날짜 = transcript 에서 한 번 계산 후 history.jsonl 에 append
//   4. 단가가 바뀌어도 finalize 된 과거는 그대로 (그게 사용자가 그 시점에 본 청구액)
//
// 파일 위치: ~/.danbi/cc_billing_history.jsonl (vault 아님 — git 노이즈 회피)

const PRICING_VERSION: &str = "v0.7.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryRow {
    date: String,
    totals: Totals,
    /// 단가표 버전. 향후 단가 정정으로 재계산하고 싶을 때 필터 가능.
    pricing_version: String,
    /// finalize 된 시각 (디버그용).
    finalized_at_ms: i64,
}

fn history_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("danbi").join("cc_billing_history.jsonl"))
}

fn load_history() -> HashMap<String, HistoryRow> {
    let mut out = HashMap::new();
    let Some(path) = history_path() else {
        return out;
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return out;
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(row) = serde_json::from_str::<HistoryRow>(line) {
            // 같은 날짜가 여러 번 append 됐으면 마지막 것 (최신 finalize) 유지.
            out.insert(row.date.clone(), row);
        }
    }
    out
}

fn append_history(row: &HistoryRow) {
    let Some(path) = history_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let line = serde_json::to_string(row).unwrap_or_default();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
}

/// 어제까지의 (날짜, Totals) 를 history 우선, 없으면 transcript 에서 한 번
/// 계산 후 history 에 finalize. 오늘은 호출자가 따로 transcript 실시간 집계.
fn finalized_daily(events: &[CcEvent], krw_per_usd: f64) -> BTreeMap<String, Totals> {
    let today = day_label(Utc::now().timestamp_millis());
    let mut history = load_history();

    // transcript 에서 어제까지의 날짜만 집계 (오늘 제외)
    let mut by_day: BTreeMap<String, Totals> = BTreeMap::new();
    for ev in events {
        let day = day_label(ev.ts_ms);
        if day == today {
            continue;
        }
        if history.contains_key(&day) {
            // 이미 finalize 된 날짜 — transcript 재집계 스킵.
            continue;
        }
        by_day.entry(day).or_default().add(ev, krw_per_usd);
    }

    // 새로 계산된 날짜는 history 에 finalize (한 번만).
    let now_ms = Utc::now().timestamp_millis();
    for (date, totals) in &by_day {
        let row = HistoryRow {
            date: date.clone(),
            totals: totals.clone(),
            pricing_version: PRICING_VERSION.to_string(),
            finalized_at_ms: now_ms,
        };
        append_history(&row);
        history.insert(date.clone(), row);
    }

    // history 의 모든 날짜 (오늘 제외) 를 결과에 합침.
    let mut out = BTreeMap::new();
    for (date, row) in history {
        if date == today {
            continue;
        }
        out.insert(date, row.totals);
    }
    out
}

/// 일별 히스토리 (90일 sparkline / 달력 잔디용 — 빈 날짜 0 으로 채움).
///
/// v0.7.1+: 어제까지 = history.jsonl 에서, 오늘 = transcript 실시간.
pub fn daily_series(days: u32, krw_per_usd: f64) -> Vec<DailyPoint> {
    let now = Utc::now().timestamp_millis();
    let from = now - (days as i64) * DAY_MS;
    let today = day_label(now);
    let events = load_all();

    let mut by_day = finalized_daily(&events, krw_per_usd);

    // 오늘은 transcript 에서 실시간 집계.
    let mut today_totals = Totals::default();
    for ev in &events {
        if day_label(ev.ts_ms) == today {
            today_totals.add(ev, krw_per_usd);
        }
    }
    by_day.insert(today.clone(), today_totals);

    // 빈 날짜 0 으로 채움 (sparkline / 잔디 길이 보장).
    let mut filled: BTreeMap<String, Totals> = BTreeMap::new();
    for d in 0..=days as i64 {
        let ts = now - (days as i64 - d) * DAY_MS;
        let day = day_label(ts);
        if ts < from {
            continue;
        }
        filled.insert(day.clone(), by_day.remove(&day).unwrap_or_default());
    }

    filled
        .into_iter()
        .map(|(date, totals)| DailyPoint { date, totals })
        .collect()
}

/// 월별 히스토리 (전체 — 청구 추이).
///
/// v0.7.1+: 어제까지 = history.jsonl 에서 월 단위 fold, 오늘 = transcript.
pub fn monthly_series(krw_per_usd: f64) -> Vec<DailyPoint> {
    let events = load_all();
    let today = day_label(Utc::now().timestamp_millis());

    let by_day = finalized_daily(&events, krw_per_usd);

    // history 의 모든 어제까지 + 오늘 transcript.
    let mut by_month: BTreeMap<String, Totals> = BTreeMap::new();
    for (date, totals) in &by_day {
        let key = date[..7].to_string(); // "YYYY-MM"
        by_month.entry(key).or_default().merge(totals);
    }
    // 오늘
    let mut today_totals = Totals::default();
    for ev in &events {
        if day_label(ev.ts_ms) == today {
            today_totals.add(ev, krw_per_usd);
        }
    }
    let key_today = today[..7].to_string();
    by_month.entry(key_today).or_default().merge(&today_totals);

    by_month
        .into_iter()
        .map(|(date, totals)| DailyPoint { date, totals })
        .collect()
}

/// 영속 히스토리 강제 재빌드 — 단가 보정 후 사용자가 "처음부터 다시" 원할
/// 때 호출. cc_billing_history.jsonl 을 비우고 다음 호출 때 finalize 다시.
pub fn reset_history() -> std::io::Result<()> {
    let Some(path) = history_path() else {
        return Ok(());
    };
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(
        ts_ms: i64,
        model: &str,
        backend: &str,
        in_t: u64,
        out_t: u64,
        cc: u64,
        cr: u64,
    ) -> CcEvent {
        CcEvent {
            ts_ms,
            session_id: "s".to_string(),
            cwd: "/x/danbi".to_string(),
            git_branch: None,
            model: model.to_string(),
            backend: backend.to_string(),
            input_tokens: in_t,
            output_tokens: out_t,
            cache_creation_tokens: cc,
            cache_5m_tokens: cc,
            cache_1h_tokens: 0,
            cache_read_tokens: cr,
        }
    }

    #[test]
    fn parse_assistant_with_bedrock_id() {
        let line = r#"{"type":"assistant","timestamp":"2026-06-08T13:24:10.753Z","sessionId":"s1","cwd":"/x","gitBranch":"main","message":{"id":"msg_bdrk_abc","model":"claude-opus-4-7","usage":{"input_tokens":6,"output_tokens":204,"cache_creation_input_tokens":22745,"cache_read_input_tokens":20251}}}"#;
        let e = parse_line(line).unwrap();
        assert_eq!(e.backend, "bedrock");
        assert_eq!(e.input_tokens, 6);
        assert_eq!(e.cache_creation_tokens, 22745);
    }

    #[test]
    fn parse_assistant_with_anthropic_api_id() {
        let line = r#"{"type":"assistant","timestamp":"2026-06-08T13:24:10.753Z","message":{"id":"msg_01abc","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let e = parse_line(line).unwrap();
        assert_eq!(e.backend, "anthropic_api");
    }

    #[test]
    fn parse_skips_user_and_mode_lines() {
        assert!(parse_line(r#"{"type":"user","message":{}}"#).is_none());
        assert!(parse_line(r#"{"type":"mode","mode":"normal"}"#).is_none());
    }

    #[test]
    fn cache_5m_priced_at_absolute_bedrock_rate() {
        let e = ev(0, "claude-opus-4-7", "bedrock", 0, 0, 1_000_000, 0);
        let p = pricing::lookup("claude-opus-4-7");
        // Bedrock Opus 4.7 5m cache write = $6.25 / 1M
        let usd = e.usd(p);
        assert!((usd - 6.25).abs() < 0.01, "got {usd}");
    }

    #[test]
    fn cache_read_priced_at_absolute_bedrock_rate() {
        let e = ev(0, "claude-opus-4-7", "bedrock", 0, 0, 0, 1_000_000);
        let p = pricing::lookup("claude-opus-4-7");
        // Bedrock Opus 4.7 cache read = $0.50 / 1M
        let usd = e.usd(p);
        assert!((usd - 0.5).abs() < 0.01, "got {usd}");
    }

    #[test]
    fn cache_1h_priced_higher_than_5m() {
        // 1h breakdown 이 들어오면 더 비싼 단가 적용.
        let mut e = ev(0, "claude-opus-4-7", "bedrock", 0, 0, 1_000_000, 0);
        e.cache_5m_tokens = 0;
        e.cache_1h_tokens = 1_000_000;
        let p = pricing::lookup("claude-opus-4-7");
        // Bedrock Opus 4.7 1h cache write = $10.00 / 1M
        let usd = e.usd(p);
        assert!((usd - 10.0).abs() < 0.01, "got {usd}");
    }

    #[test]
    fn input_output_priced_at_bedrock_rate_not_anthropic() {
        // Bedrock cross-region: input $5 / output $25 (not Anthropic API $15/$75).
        let e = ev(0, "claude-opus-4-7", "bedrock", 1_000_000, 1_000_000, 0, 0);
        let p = pricing::lookup("claude-opus-4-7");
        let usd = e.usd(p);
        assert!((usd - 30.0).abs() < 0.01, "got {usd}");
    }

    #[test]
    fn summarize_aggregates_totals_and_top_days() {
        let now = Utc::now().timestamp_millis();
        let events = vec![
            ev(now - 1000, "claude-opus-4-7", "bedrock", 1000, 500, 0, 0),
            ev(now - 2000, "claude-sonnet-4-6", "bedrock", 1000, 500, 0, 0),
            ev(now - 3000, "claude-opus-4-7", "anthropic_api", 100, 50, 0, 0),
        ];
        let s = summarize_events(&events, 0, now + 1, "all", 1400.0);
        assert_eq!(s.totals.calls, 3);
        assert!(s.by_model.len() >= 2);
        assert!(s.by_backend.contains_key("bedrock"));
        assert!(s.by_backend.contains_key("anthropic_api"));
    }

    #[test]
    fn pick_disclaimer_bedrock_only() {
        let mut bb: HashMap<String, Totals> = HashMap::new();
        bb.insert("bedrock".into(), Totals::default());
        let d = pick_disclaimer(&bb);
        assert!(d.contains("Bedrock"));
    }

    #[test]
    fn pick_disclaimer_mixed_uses_api_message() {
        let mut bb: HashMap<String, Totals> = HashMap::new();
        bb.insert("bedrock".into(), Totals::default());
        bb.insert("anthropic_api".into(), Totals::default());
        let d = pick_disclaimer(&bb);
        assert!(d.contains("구독"));
    }
}
