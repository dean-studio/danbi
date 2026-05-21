//! Auto-Journal viewer — parses a project's `daily/*.md` files into
//! discrete trigger entries (decision / cause / todo / knowhow /
//! pitfall / other) so the ProjectHome can render "Claude Code 가
//! 받아 적은 것" as a structured timeline instead of raw markdown.
//!
//! We do NOT call any LLM here. The CLAUDE.md template instructs
//! Claude Code to write entries with a `### ...` heading + a category
//! word in the body ("결정", "원인", "TODO" 등). We pattern-match
//! those headings into a typed enum.

use crate::error::DanbiResult;
use crate::vault::PROJECTS_DIRNAME;
use chrono::{Local, NaiveDate};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    /// "결정" — a tech/architecture decision was finalized.
    Decision,
    /// "원인" — a debugging session located the root cause.
    Cause,
    /// "TODO" — work to pick up in a future session.
    Todo,
    /// "노하우" — reusable trick worth remembering.
    Knowhow,
    /// "재발 방지" — pitfall that bit us; do not repeat.
    Pitfall,
    /// Heading didn't match any known trigger word.
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct JournalEntry {
    /// "YYYY-MM-DD" — the daily note this entry was extracted from.
    pub date: String,
    /// Heading text without the leading "### ".
    pub title: String,
    pub kind: TriggerKind,
    /// First few lines of body text after the heading. UI renders this
    /// as a 1–2 line preview; we cap at ~280 chars to keep the
    /// snapshot small over IPC.
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DayCounts {
    pub date: String,
    pub decision: u32,
    pub cause: u32,
    pub todo: u32,
    pub knowhow: u32,
    pub pitfall: u32,
    pub other: u32,
}

impl DayCounts {
    pub fn total(&self) -> u32 {
        self.decision
            + self.cause
            + self.todo
            + self.knowhow
            + self.pitfall
            + self.other
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectJournalView {
    pub project: String,
    pub today: String,
    /// Today's per-trigger counts.
    pub today_counts: DayCounts,
    /// Up to 8 most recent entries across the last 7 days, newest
    /// first. Each carries its own date so the UI can group them.
    pub recent_entries: Vec<JournalEntry>,
    /// Per-day counts for the last 7 days (today + 6).
    pub last_7_days: Vec<DayCounts>,
    /// daily/*.md 파일별 trigger kind 집합. 사이드바에서 파일명 옆에
    /// chip 으로 표시하는 데 쓴다 — key 는 도메인 path
    /// ("daily/2026-05-20.md"), value 는 그 파일에 등장한 unique kind 들.
    /// 추가 IPC 없이 ProjectJournalView 캐시 하나로 처리.
    pub daily_file_kinds: std::collections::HashMap<String, Vec<TriggerKind>>,
}

/// Public entry point — read every daily note in `<vault>/Projects/
/// <project>/daily/`, parse it, and produce the view-model.
pub fn view(vault: &Path, project: &str) -> DanbiResult<ProjectJournalView> {
    let daily_dir = vault
        .join(PROJECTS_DIRNAME)
        .join(project)
        .join(crate::daily::DAILY_DIRNAME);

    let today = Local::now().date_naive();
    let today_str = today.format("%Y-%m-%d").to_string();

    // Build the 7-day window once so every code path (counts + entries)
    // shares the same horizon.
    let mut last_7: Vec<DayCounts> = (0..7)
        .map(|i| {
            let d = today - chrono::Duration::days(i);
            DayCounts {
                date: d.format("%Y-%m-%d").to_string(),
                ..Default::default()
            }
        })
        .collect();

    let mut all_entries: Vec<JournalEntry> = Vec::new();
    let mut daily_file_kinds: std::collections::HashMap<String, Vec<TriggerKind>> =
        std::collections::HashMap::new();

    if daily_dir.exists() {
        for de in std::fs::read_dir(&daily_dir)? {
            let de = de?;
            let path = de.path();
            if !path.is_file() {
                continue;
            }
            let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // daily file convention: YYYY-MM-DD.md
            let Some(date_str) = fname.strip_suffix(".md") else {
                continue;
            };
            let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let entries = parse_entries(date_str, &content);
            // Per-day count update if within the 7-day window.
            let days_ago = (today - date).num_days();
            if (0..7).contains(&days_ago) {
                let bucket =
                    last_7.iter_mut().find(|c| c.date == date_str).unwrap();
                for e in &entries {
                    match e.kind {
                        TriggerKind::Decision => bucket.decision += 1,
                        TriggerKind::Cause => bucket.cause += 1,
                        TriggerKind::Todo => bucket.todo += 1,
                        TriggerKind::Knowhow => bucket.knowhow += 1,
                        TriggerKind::Pitfall => bucket.pitfall += 1,
                        TriggerKind::Other => bucket.other += 1,
                    }
                }
            }
            // 파일별 kind 집합 (사이드바 chip 용). dedup + 안정적인 정렬:
            // decision → cause → todo → knowhow → pitfall → other 순서로
            // 들어가게 enum 의 자연 순서를 따른다.
            let mut kinds: Vec<TriggerKind> =
                entries.iter().map(|e| e.kind).collect();
            kinds.sort_by_key(|k| match k {
                TriggerKind::Decision => 0,
                TriggerKind::Cause => 1,
                TriggerKind::Todo => 2,
                TriggerKind::Knowhow => 3,
                TriggerKind::Pitfall => 4,
                TriggerKind::Other => 5,
            });
            kinds.dedup();
            if !kinds.is_empty() {
                daily_file_kinds.insert(format!("daily/{}.md", date_str), kinds);
            }
            // Collect entries for the recent feed.
            all_entries.extend(entries);
        }
    }

    // Sort entries newest-first by date string (ISO so lexical works).
    // 프론트에서 chip 으로 필터링할 수 있도록 7일치 모두 보낸다 — 8 로 잘라
    // 보내면 필터가 빈 결과를 자주 만든다. 7일치라도 entry 는 보통 수십개라
    // payload 부담 없음 (preview 가 280자로 캡됨).
    all_entries.sort_by(|a, b| b.date.cmp(&a.date));
    let recent_entries: Vec<JournalEntry> = all_entries;

    let today_counts = last_7
        .iter()
        .find(|c| c.date == today_str)
        .cloned()
        .unwrap_or_else(|| DayCounts {
            date: today_str.clone(),
            ..Default::default()
        });

    Ok(ProjectJournalView {
        project: project.to_string(),
        today: today_str,
        today_counts,
        recent_entries,
        last_7_days: last_7,
        daily_file_kinds,
    })
}

/// Walk a daily-note's markdown and return one JournalEntry per
/// `### heading` block. Trigger kind is inferred from keywords in the
/// heading text first, then the first body line as a fallback.
fn parse_entries(date: &str, body: &str) -> Vec<JournalEntry> {
    let mut out: Vec<JournalEntry> = Vec::new();
    let mut cur_title: Option<String> = None;
    let mut cur_body: Vec<String> = Vec::new();

    let flush = |title: &str,
                 body_lines: &[String],
                 out: &mut Vec<JournalEntry>| {
        let body_text = body_lines.join("\n").trim().to_string();
        let kind = classify(title, &body_text);
        let preview: String = body_text.chars().take(280).collect();
        out.push(JournalEntry {
            date: date.to_string(),
            title: title.to_string(),
            kind,
            preview,
        });
    };

    for line in body.lines() {
        // Treat both H3 and H4 as entry boundaries — Claude Code
        // sometimes uses one or the other depending on the parent
        // section structure.
        if line.starts_with("### ") || line.starts_with("#### ") {
            // Flush previous block.
            if let Some(prev) = cur_title.take() {
                flush(&prev, &cur_body, &mut out);
                cur_body.clear();
            }
            let stripped = line
                .trim_start_matches('#')
                .trim_start()
                .to_string();
            cur_title = Some(stripped);
        } else if cur_title.is_some() {
            cur_body.push(line.to_string());
        }
    }
    if let Some(prev) = cur_title.take() {
        flush(&prev, &cur_body, &mut out);
    }
    out
}

/// Heuristic classifier — runs against a heading + first paragraph and
/// returns the matching TriggerKind. Keywords match the canonical
/// Korean triggers from the CLAUDE.md template plus a few common
/// English variants.
fn classify(title: &str, body: &str) -> TriggerKind {
    let haystack = format!("{title}\n{body}").to_lowercase();
    // Order matters — pitfall ("재발 방지") and knowhow ("노하우") are
    // checked before generic "결정" / "원인" so multi-word headings
    // get the more specific tag.
    if haystack.contains("재발 방지") || haystack.contains("pitfall") {
        return TriggerKind::Pitfall;
    }
    if haystack.contains("노하우") || haystack.contains("knowhow") {
        return TriggerKind::Knowhow;
    }
    if haystack.contains("결정") || haystack.contains("decision") {
        return TriggerKind::Decision;
    }
    if haystack.contains("원인")
        || haystack.contains("버그")
        || haystack.contains("bug")
        || haystack.contains("cause")
    {
        return TriggerKind::Cause;
    }
    if haystack.contains("todo")
        || haystack.contains("할 일")
        || haystack.contains("이어서")
    {
        return TriggerKind::Todo;
    }
    TriggerKind::Other
}
