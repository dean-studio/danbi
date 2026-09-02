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

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
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
    /// stat 으로 본 마지막 파일 크기 (변경 감지용).
    size: u64,
    /// 마지막 개행까지 실제로 소비(파싱)한 바이트 오프셋. transcript 는
    /// append-only 라 다음 refresh 때 이 지점부터 tail 만 읽으면 된다.
    parsed_bytes: u64,
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

/// `start` 바이트 오프셋부터 EOF 까지 읽어 **완결된 줄(개행으로 끝나는
/// 줄)만** 파싱한다. 반환값의 두 번째 요소는 마지막 개행 다음 절대
/// 오프셋 — 다음 tail 파싱의 시작점이 된다. 아직 개행으로 끝나지 않은
/// 꼬리(쓰는 중인 부분 줄)는 소비하지 않고 남겨둔다.
fn parse_from(path: &Path, start: u64) -> std::io::Result<(Vec<CcEvent>, u64)> {
    let mut f = fs::File::open(path)?;
    if start > 0 {
        f.seek(SeekFrom::Start(start))?;
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;

    // 마지막 개행 위치까지만 "완결" 로 간주.
    let complete_end = match buf.iter().rposition(|&b| b == b'\n') {
        Some(i) => i + 1,
        None => 0,
    };

    let mut events = Vec::new();
    let mut line_start = 0usize;
    for i in 0..complete_end {
        if buf[i] == b'\n' {
            if let Ok(s) = std::str::from_utf8(&buf[line_start..i]) {
                let t = s.trim();
                if !t.is_empty() {
                    if let Some(ev) = parse_line(t) {
                        events.push(ev);
                    }
                }
            }
            line_start = i + 1;
        }
    }
    Ok((events, start + complete_end as u64))
}

/// 캐시 반영 액션. 파싱은 락 밖에서 끝내고, 짧게 write-lock 을 잡아
/// 결과만 반영한다.
enum CacheUpdate {
    /// `from` 오프셋부터 tail 만 파싱한 델타. append-only 파일용.
    Append {
        from: u64,
        events: Vec<CcEvent>,
        parsed_bytes: u64,
        mtime_ms: i64,
        size: u64,
    },
    /// 신규 / 잘림·회전 파일 — 전체 재파싱.
    Replace {
        events: Vec<CcEvent>,
        parsed_bytes: u64,
        mtime_ms: i64,
        size: u64,
    },
}

/// 모든 transcript 캐시를 최신화(증분).
///
/// v0.8.3: 예전 구현은 mtime/size 가 바뀐 파일을 **통째로** 다시 파싱했다.
/// Claude Code 가 활성 세션 transcript 에 계속 append 하므로, 팝오버를
/// 열거나 창에 포커스가 갈 때마다 수백 MB(하루 300M 토큰이면 활성 파일
/// 합계 400MB+)를 매번 처음부터 JSON 파싱 → 메뉴바 팝오버 버벅임의 주범.
/// transcript 는 append-only 라, 마지막으로 소비한 오프셋(`parsed_bytes`)
/// 이후의 새 바이트만 파싱한다. 파일이 줄거나(size < parsed_bytes) 사라진
/// 뒤 새로 생기면 전체 재파싱으로 안전하게 폴백.
fn refresh_cache() {
    let Some(root) = projects_dir() else {
        return;
    };
    let files = list_jsonl_files(&root);

    // 1) read-lock 으로 이전 상태 스냅샷만 확보 (파싱은 락 밖에서).
    let mut prev: HashMap<PathBuf, (i64, u64, u64)> = HashMap::new();
    if let Ok(cache) = cache().read() {
        for path in &files {
            if let Some(e) = cache.files.get(path) {
                prev.insert(path.clone(), (e.mtime_ms, e.size, e.parsed_bytes));
            }
        }
    }

    // 2) 락 없이 변경분만 파싱.
    let mut work: Vec<(PathBuf, CacheUpdate)> = Vec::new();
    for path in &files {
        let Ok(meta) = fs::metadata(path) else { continue };
        let m = mtime_ms(&meta);
        let s = meta.len();
        match prev.get(path) {
            // 변화 없음 → skip.
            Some(&(pm, ps, _)) if pm == m && ps == s => {}
            // append-only 성장(또는 mtime 만 갱신) → tail 만.
            Some(&(_, _, parsed)) if s >= parsed => {
                if let Ok((events, new_parsed)) = parse_from(path, parsed) {
                    work.push((
                        path.clone(),
                        CacheUpdate::Append {
                            from: parsed,
                            events,
                            parsed_bytes: new_parsed,
                            mtime_ms: m,
                            size: s,
                        },
                    ));
                }
            }
            // 신규 or 잘림/회전 → 전체.
            _ => {
                if let Ok((events, new_parsed)) = parse_from(path, 0) {
                    work.push((
                        path.clone(),
                        CacheUpdate::Replace {
                            events,
                            parsed_bytes: new_parsed,
                            mtime_ms: m,
                            size: s,
                        },
                    ));
                }
            }
        }
    }

    // 3) 짧게 write-lock 을 잡고 반영 + 죽은 파일 정리.
    if let Ok(mut cache) = cache().write() {
        for (path, update) in work {
            match update {
                CacheUpdate::Append {
                    from,
                    mut events,
                    parsed_bytes,
                    mtime_ms,
                    size,
                } => {
                    // 스냅샷 이후 다른 스레드가 먼저 진행시켰으면(오프셋 불일치)
                    // 이 델타는 버린다 — 이중 집계 방지. 그 파일은 이미 최신.
                    if let Some(entry) = cache.files.get_mut(&path) {
                        if entry.parsed_bytes == from {
                            entry.events.append(&mut events);
                            entry.parsed_bytes = parsed_bytes;
                            entry.mtime_ms = mtime_ms;
                            entry.size = size;
                        }
                    }
                    // entry 가 사라졌으면 skip — 다음 refresh 가 전체 재파싱.
                }
                CacheUpdate::Replace {
                    events,
                    parsed_bytes,
                    mtime_ms,
                    size,
                } => {
                    cache.files.insert(
                        path,
                        FileEntry {
                            mtime_ms,
                            size,
                            parsed_bytes,
                            events,
                        },
                    );
                }
            }
        }
        // 사라진 파일은 캐시에서 제거.
        let alive: std::collections::HashSet<&Path> =
            files.iter().map(|p| p.as_path()).collect();
        cache.files.retain(|p, _| alive.contains(p.as_path()));
    }
}

/// 캐시를 최신화한 뒤 read-lock 을 잡은 채, 시간순 정렬된 **참조** 슬라이스로
/// 클로저를 실행한다.
///
/// v0.8.0: 예전 `load_all()` 은 84k+ 이벤트(파일별 Vec)를 매 호출마다 새 Vec 로
/// 통째 clone + sort 했다. Claude Code 대시보드 한 번 열면 summarize/daily/monthly
/// 세 경로가 각각 clone → 수십 MB 임시 할당이 반복되며 allocator RSS 고수위를
/// 밀어올렸다. 이제 struct 는 복사하지 않고 포인터(`&CcEvent`)만 모아 정렬한다.
fn with_events<R>(f: impl FnOnce(&[&CcEvent]) -> R) -> R {
    refresh_cache();
    let guard = cache().read();
    let mut refs: Vec<&CcEvent> = Vec::new();
    if let Ok(cache) = guard.as_ref() {
        for entry in cache.files.values() {
            refs.extend(entry.events.iter());
        }
    }
    refs.sort_by_key(|e| e.ts_ms);
    f(&refs)
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
        self.calls += other.calls;
        self.sessions += other.sessions;
    }

    fn add(&mut self, ev: &CcEvent) {
        self.input_tokens += ev.input_tokens;
        self.output_tokens += ev.output_tokens;
        self.cache_creation_tokens += ev.cache_creation_tokens;
        self.cache_read_tokens += ev.cache_read_tokens;
        self.total_tokens += ev.input_tokens
            + ev.output_tokens
            + ev.cache_creation_tokens
            + ev.cache_read_tokens;
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
    /// 추적 ON/OFF — OFF 면 빈 요약이고 UI 가 안내 배너를 보여준다.
    pub enabled: bool,
    pub totals: Totals,
    pub by_model: Vec<ModelBreakdown>,
    pub by_project: Vec<ProjectBreakdown>,
    pub by_backend: HashMap<String, Totals>,
    pub daily: Vec<DailyPoint>,
    pub hourly: Vec<HourlyHeatPoint>,
    /// 오늘 vs 1년 전 오늘 비교 (히스토리 카드용).
    pub year_ago_today: Option<Totals>,
    /// 토큰이 가장 많았던 날 Top 5 (range 내).
    pub top_days: Vec<DailyPoint>,
    /// 안내 문구.
    pub disclaimer: String,
}

const DISCLAIMER_TOKENS: &str =
    "이 숫자는 ~/.claude/projects 의 transcript 를 직접 읽어 집계한 토큰량입니다. OAuth endpoint 호출 없이 자기 디스크의 자기 파일만 사용합니다.";

pub fn summarize(range: Range) -> CcSummary {
    let (from_ms, to_ms) = range.window();
    with_events(|events| summarize_events(events, from_ms, to_ms, range.label()))
}

fn summarize_events(
    events: &[&CcEvent],
    from_ms: i64,
    to_ms: i64,
    range_label: &str,
) -> CcSummary {
    let mut totals = Totals::default();
    let mut by_model: HashMap<(String, String), Totals> = HashMap::new();
    let mut by_project: HashMap<String, Totals> = HashMap::new();
    let mut by_backend: HashMap<String, Totals> = HashMap::new();
    let mut by_day: BTreeMap<String, Totals> = BTreeMap::new();
    let mut by_hour: HashMap<(u8, u8), u64> = HashMap::new();
    let mut sessions: std::collections::HashSet<String> = std::collections::HashSet::new();

    for &ev in events {
        if ev.ts_ms < from_ms || ev.ts_ms >= to_ms {
            continue;
        }
        totals.add(ev);
        sessions.insert(ev.session_id.clone());

        let key = (ev.model.clone(), ev.backend.clone());
        by_model.entry(key).or_default().add(ev);

        by_project.entry(ev.cwd.clone()).or_default().add(ev);
        by_backend.entry(ev.backend.clone()).or_default().add(ev);

        let day = day_label(ev.ts_ms);
        by_day.entry(day).or_default().add(ev);

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
    by_model_vec.sort_by(|a, b| b.totals.total_tokens.cmp(&a.totals.total_tokens));

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
    by_project_vec.sort_by(|a, b| b.totals.total_tokens.cmp(&a.totals.total_tokens));
    by_project_vec.truncate(10);

    let daily: Vec<DailyPoint> = by_day
        .into_iter()
        .map(|(date, totals)| DailyPoint { date, totals })
        .collect();

    let mut top_days = daily.clone();
    top_days.sort_by(|a, b| b.totals.total_tokens.cmp(&a.totals.total_tokens));
    top_days.truncate(5);

    let hourly: Vec<HourlyHeatPoint> = by_hour
        .into_iter()
        .map(|((dow, hour), tokens)| HourlyHeatPoint { dow, hour, tokens })
        .collect();

    let year_ago_today = compute_year_ago_today(events);

    CcSummary {
        from_ms,
        to_ms,
        range: range_label.to_string(),
        enabled: true,
        totals,
        by_model: by_model_vec,
        by_project: by_project_vec,
        by_backend,
        daily,
        hourly,
        year_ago_today,
        top_days,
        disclaimer: DISCLAIMER_TOKENS.to_string(),
    }
}

fn compute_year_ago_today(events: &[&CcEvent]) -> Option<Totals> {
    let today = Local::now().date_naive();
    let target = NaiveDate::from_ymd_opt(today.year() - 1, today.month(), today.day())?;
    let target_str = target.format("%Y-%m-%d").to_string();
    let mut totals = Totals::default();
    let mut hit = false;
    for &ev in events {
        if day_label(ev.ts_ms) == target_str {
            totals.add(ev);
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryRow {
    date: String,
    totals: Totals,
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
fn finalized_daily(events: &[&CcEvent]) -> BTreeMap<String, Totals> {
    let today = day_label(Utc::now().timestamp_millis());
    let mut history = load_history();

    // transcript 에서 어제까지의 날짜만 집계 (오늘 제외)
    let mut by_day: BTreeMap<String, Totals> = BTreeMap::new();
    for &ev in events {
        let day = day_label(ev.ts_ms);
        if day == today {
            continue;
        }
        if history.contains_key(&day) {
            // 이미 finalize 된 날짜 — transcript 재집계 스킵.
            continue;
        }
        by_day.entry(day).or_default().add(ev);
    }

    // 새로 계산된 날짜는 history 에 finalize (한 번만).
    let now_ms = Utc::now().timestamp_millis();
    for (date, totals) in &by_day {
        let row = HistoryRow {
            date: date.clone(),
            totals: totals.clone(),
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
pub fn daily_series(days: u32) -> Vec<DailyPoint> {
    let now = Utc::now().timestamp_millis();
    let from = now - (days as i64) * DAY_MS;
    let today = day_label(now);

    with_events(|events| {
        let mut by_day = finalized_daily(events);

        // 오늘은 transcript 에서 실시간 집계.
        let mut today_totals = Totals::default();
        for &ev in events {
            if day_label(ev.ts_ms) == today {
                today_totals.add(ev);
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
    })
}

/// 월별 히스토리 (전체 — 청구 추이).
///
/// v0.7.1+: 어제까지 = history.jsonl 에서 월 단위 fold, 오늘 = transcript.
pub fn monthly_series() -> Vec<DailyPoint> {
    let today = day_label(Utc::now().timestamp_millis());

    with_events(|events| {
        let by_day = finalized_daily(events);

        // history 의 모든 어제까지 + 오늘 transcript.
        let mut by_month: BTreeMap<String, Totals> = BTreeMap::new();
        for (date, totals) in &by_day {
            let key = date[..7].to_string(); // "YYYY-MM"
            by_month.entry(key).or_default().merge(totals);
        }
        // 오늘
        let mut today_totals = Totals::default();
        for &ev in events {
            if day_label(ev.ts_ms) == today {
                today_totals.add(ev);
            }
        }
        let key_today = today[..7].to_string();
        by_month.entry(key_today).or_default().merge(&today_totals);

        by_month
            .into_iter()
            .map(|(date, totals)| DailyPoint { date, totals })
            .collect()
    })
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
    fn totals_sum_all_token_kinds() {
        let mut t = Totals::default();
        let e = ev(0, "claude-opus-4-7", "bedrock", 1000, 500, 200, 100);
        t.add(&e);
        assert_eq!(t.input_tokens, 1000);
        assert_eq!(t.output_tokens, 500);
        assert_eq!(t.cache_creation_tokens, 200);
        assert_eq!(t.cache_read_tokens, 100);
        assert_eq!(t.total_tokens, 1800);
        assert_eq!(t.calls, 1);
    }

    #[test]
    fn summarize_aggregates_totals_and_sorts_by_tokens() {
        let now = Utc::now().timestamp_millis();
        let events = vec![
            ev(now - 1000, "claude-opus-4-7", "bedrock", 1000, 500, 0, 0),
            ev(now - 2000, "claude-sonnet-4-6", "bedrock", 1000, 500, 0, 0),
            ev(now - 3000, "claude-opus-4-7", "anthropic_api", 100, 50, 0, 0),
        ];
        let refs: Vec<&CcEvent> = events.iter().collect();
        let s = summarize_events(&refs, 0, now + 1, "all");
        assert_eq!(s.totals.calls, 3);
        assert!(s.by_model.len() >= 2);
        assert!(s.by_backend.contains_key("bedrock"));
        assert!(s.by_backend.contains_key("anthropic_api"));
        // 토큰 많은 순 정렬 확인.
        assert!(
            s.by_model[0].totals.total_tokens >= s.by_model[1].totals.total_tokens
        );
    }
}
