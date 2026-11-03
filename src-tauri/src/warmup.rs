//! 첫 로딩 진행률 — 프로세스가 새로 뜬 뒤 처음 한 번만 오래 걸리는 작업
//! (Claude Code transcript 파싱, tantivy 색인 빌드)의 "지금 몇 번째 파일인지"
//! 를 프런트가 가볍게 조회할 수 있게 전역에 기록한다.
//!
//! 이벤트(`app.emit`)가 아니라 조회(poll) 방식인 이유: 느린 루프들이 AppHandle
//! 없는 자유 함수(`refresh_cache`, `build_corpus`)라 핸들을 꿰어 넣는 것보다
//! 전역 상태 + `warmup_progress` command 한 개가 변경 범위가 작다. 프런트는
//! 로딩 중일 때만 300ms 간격으로 묻는다.
//!
//! 진행률 갱신은 파일 하나당 mutex 한 번 — 수천 파일이라도 체감 비용 없음.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Claude Code transcript(jsonl) 캐시 채우기 — `claude_code_usage::refresh_cache`.
pub const TASK_USAGE: &str = "usage";
/// vault 전문 검색 색인(tantivy) 빌드 — `search::ensure_corpus_fresh`.
pub const TASK_SEARCH: &str = "search";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WarmupProgress {
    pub task: String,
    /// 작업별 단계 이름. 프런트가 한국어 문구로 바꾼다.
    pub phase: String,
    pub done: usize,
    pub total: usize,
    /// 아직 돌고 있으면 true. `finish` 뒤에는 마지막 값이 남되 false.
    pub active: bool,
    /// 이 작업이 시작된 epoch ms — 프런트가 경과 시간을 맞추는 데 쓴다.
    pub started_ms: i64,
    pub updated_ms: i64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn table() -> &'static Mutex<HashMap<&'static str, WarmupProgress>> {
    static T: OnceLock<Mutex<HashMap<&'static str, WarmupProgress>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_entry(task: &'static str, f: impl FnOnce(&mut WarmupProgress)) {
    if let Ok(mut t) = table().lock() {
        let now = now_ms();
        let e = t.entry(task).or_insert_with(|| WarmupProgress {
            task: task.to_string(),
            phase: String::new(),
            done: 0,
            total: 0,
            active: false,
            started_ms: now,
            updated_ms: now,
        });
        f(e);
        e.updated_ms = now;
    }
}

/// 새 단계 시작. 작업이 쉬고 있었다면 `started_ms` 도 지금으로 리셋.
pub fn phase(task: &'static str, phase: &str, total: usize) {
    with_entry(task, |e| {
        if !e.active {
            e.started_ms = now_ms();
        }
        e.active = true;
        e.phase = phase.to_string();
        e.done = 0;
        e.total = total;
    });
}

/// 현재 단계에서 `done` 개 처리됨.
pub fn step(task: &'static str, done: usize) {
    with_entry(task, |e| e.done = done);
}

/// 작업 끝 — 마지막 단계·개수는 남기고 active 만 내린다.
pub fn finish(task: &'static str) {
    with_entry(task, |e| {
        e.active = false;
        e.done = e.total.max(e.done);
    });
}

/// 프런트 조회용. 한 번도 돈 적 없는 작업은 None.
pub fn snapshot(task: &str) -> Option<WarmupProgress> {
    table().lock().ok().and_then(|t| t.get(task).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_step_finish_round_trip() {
        const T: &str = "warmup-test";
        assert!(snapshot(T).is_none());
        phase(T, "parsing", 3);
        let p = snapshot(T).unwrap();
        assert!(p.active);
        assert_eq!((p.phase.as_str(), p.done, p.total), ("parsing", 0, 3));
        step(T, 2);
        assert_eq!(snapshot(T).unwrap().done, 2);
        finish(T);
        let p = snapshot(T).unwrap();
        assert!(!p.active);
        // finish 는 done 을 total 까지 올려 프런트 막대가 100% 로 닫히게 한다.
        assert_eq!(p.done, 3);
        // 다음 phase 는 시작 시각을 새로 잡는다.
        phase(T, "listing", 0);
        assert!(snapshot(T).unwrap().active);
    }

    /// 실제 transcript 캐시 채우기(read-only, ~/.claude*)가 진행률을 남기고
    /// 끝에는 active 를 내리는지 — 프런트가 "n / m" 에 멈춰 있지 않게.
    /// 디버그 빌드에서 수 분 걸려 기본 실행에서는 뺀다:
    /// `cargo test --lib warmup -- --ignored`
    #[test]
    #[ignore]
    fn usage_refresh_leaves_finished_progress() {
        crate::claude_code_usage::bench_warm_cache();
        let p = snapshot(TASK_USAGE).expect("refresh_cache 가 진행률을 남긴다");
        assert!(!p.active);
        assert_eq!(p.phase, "merging");
    }
}
