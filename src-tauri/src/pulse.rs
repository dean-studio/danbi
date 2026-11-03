//! 사이드 프로젝트가 많을 때 "다시 시작하기 쉽게" (0.10).
//!
//! - **다음 한 걸음**: 프로젝트별 한 줄 (`Projects/<P>/.danbi/next.json`).
//!   사용자가 적거나, 에이전트가 `danbi_log(next_step)` 로 남긴다. 없으면
//!   최근 daily 노트의 `- 다음 할 일:` 줄(세션 자동 기록이 쓰는 형식)에서 고른다.
//! - **재진입 카드**: 마지막 작업 · 다음 한 걸음 · 열린 할 일 · 유효한 결정.
//! - **할 일함**: vault 전체의 열린 할 일 — `- [ ]` 체크박스, `TODO`/`할 일`
//!   제목 아래 bullet, `다음 할 일:`/`TODO:` 줄. 열린 체크박스가 많은 계획
//!   문서는 항목을 펼치지 않고 '체크리스트' 한 줄(남은 수/전체)로 묶는다.
//! - **쉬는 중**: 오래 손대지 않은 프로젝트(기본 30일)와 보관한 프로젝트.
//! - **시간 분배**: Claude Code 작업 시간(5분 칸)·토큰을 연결된 코드 폴더로
//!   프로젝트에 묶고, vault 기록 수를 더한다.
//! - **다른 프로젝트의 비슷한 기록**: 키워드 검색으로 같은 문제를 겪은 기록 찾기.
//!
//! 전부 LLM 없이 파일 mtime · markdown 줄 · git · 검색 색인만 쓴다.

use crate::config::DanbiConfig;
use crate::daily::DAILY_DIRNAME;
use crate::error::{DanbiError, DanbiResult};
use crate::vault::{self, DomainNode, ProjectNode, VaultTree, PROJECTS_DIRNAME};
use chrono::{Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const DAY_MS: i64 = 86_400_000;
/// 앱이 스스로 다시 쓰는 뼈대 문서 — 활동으로 치지 않는다.
pub const SCAFFOLD_DOCS: [&str; 3] = ["SKILL.md", "purpose.md", "schema.md"];
const NEXT_FILE: &str = ".danbi/next.json";
pub const NEXT_MAX_CHARS: usize = 200;
const TODO_TEXT_CHARS: usize = 240;
/// 열린 체크박스가 이만큼 이상인 (daily 아닌) 문서는 계획·체크리스트로 본다.
pub const CHECKLIST_MIN_OPEN: usize = 6;
const INBOX_LIMIT: usize = 600;
const CARD_TODOS: usize = 5;
const CARD_CHECKLISTS: usize = 3;

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

fn now_secs() -> i64 {
    Local::now().timestamp()
}

pub fn is_scaffold(domain: &str) -> bool {
    SCAFFOLD_DOCS.contains(&domain)
}

fn is_daily(domain: &str) -> bool {
    domain.starts_with(&format!("{DAILY_DIRNAME}/"))
}

/// `daily/2026-11-02.md` (또는 `daily/2026-11/2026-11-02.md`) 의 날짜.
fn daily_date(domain: &str) -> Option<NaiveDate> {
    let rest = domain.strip_prefix(&format!("{DAILY_DIRNAME}/"))?;
    let stem = rest.strip_suffix(".md")?;
    let stem = stem.rsplit('/').next()?;
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

/// 그 날짜 23:59:59 (로컬) 의 ms — daily 항목을 "쓴 날" 기준으로 정렬할 때.
fn day_end_ms(d: NaiveDate) -> i64 {
    d.and_hms_opt(23, 59, 59)
        .and_then(|dt| Local.from_local_datetime(&dt).single())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn is_safe_project(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.chars().any(|c| matches!(c, '/' | '\\' | '\0' | '\n' | '\r'))
}

/// 프로젝트의 모든 markdown (최상위 + 하위 폴더).
pub fn flatten(p: &ProjectNode) -> Vec<&DomainNode> {
    fn walk<'a>(subs: &'a [vault::SubfolderNode], out: &mut Vec<&'a DomainNode>) {
        for s in subs {
            out.extend(s.domains.iter());
            walk(&s.subfolders, out);
        }
    }
    let mut out: Vec<&DomainNode> = p.domains.iter().collect();
    walk(&p.subfolders, &mut out);
    out
}

// ---------- 다음 한 걸음 ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NextStep {
    /// 비어 있으면 "사용자가 완료/지움" 표시 (아래 `supersedes` 만 의미 있음).
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub set_at: i64,
    /// 누가 적었나 — `human` · 에이전트 이름(`claude-code` 등).
    #[serde(default)]
    pub by: String,
    /// 적을 때 daily 노트에서 고른 후보(`domain\nraw`). 같은 후보가 다시
    /// 이 값을 덮지 않게 한다 — 더 새 후보만 덮는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
}

fn next_path(vault: &Path, project: &str) -> DanbiResult<PathBuf> {
    if !is_safe_project(project) {
        return Err(DanbiError::Config(format!("unsafe project name: {project}")));
    }
    Ok(vault.join(PROJECTS_DIRNAME).join(project).join(NEXT_FILE))
}

pub fn load_next(vault: &Path, project: &str) -> Option<NextStep> {
    let raw = std::fs::read_to_string(next_path(vault, project).ok()?).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_next(vault: &Path, project: &str, step: &NextStep) -> DanbiResult<()> {
    let path = next_path(vault, project)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(step)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 다음 한 걸음을 적는다. 빈 문자열이면 지운다(완료). 지금 daily 노트에서
/// 고를 후보를 함께 기억해 두어, 그 후보가 이 값을 다시 덮지 않게 한다.
pub fn set_next(vault: &Path, project: &str, text: &str, by: &str) -> DanbiResult<Option<NextStep>> {
    if !vault::project_exists(vault, project) {
        return Err(DanbiError::Config(format!("unknown project: {project}")));
    }
    let text = text.trim().lines().next().unwrap_or("").trim();
    let supersedes = note_candidate(vault, project).map(|t| todo_key(&t));
    let step = NextStep {
        text: clip(text, NEXT_MAX_CHARS),
        set_at: now_secs(),
        by: by.to_string(),
        supersedes,
    };
    save_next(vault, project, &step)?;
    Ok((!step.text.is_empty()).then_some(step))
}

fn todo_key(t: &TodoItem) -> String {
    format!("{}\n{}", t.domain, t.raw)
}

#[derive(Debug, Clone, Serialize)]
pub struct NextView {
    pub text: String,
    /// `set` = 사용자/에이전트가 적음, `note` = daily 노트의 `다음 할 일:` 줄.
    pub origin: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub at_ms: i64,
    /// origin = note 일 때 그 줄 — 완료 처리에 쓴다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo: Option<TodoItem>,
}

/// 지금 보여줄 다음 한 걸음.
pub fn next_view(vault: &Path, project: &str) -> Option<NextView> {
    let saved = load_next(vault, project);
    let candidate = note_candidate(vault, project);
    let from_note = |t: TodoItem| NextView {
        text: t.text.clone(),
        origin: "note",
        by: None,
        at_ms: t.modified_ms,
        todo: Some(t),
    };
    match (saved, candidate) {
        (None, Some(t)) => Some(from_note(t)),
        (None, None) => None,
        (Some(s), cand) => {
            let fresh_note = cand.filter(|t| {
                s.supersedes.as_deref() != Some(todo_key(t).as_str())
                    && t.modified_ms > s.set_at * 1000
            });
            if let Some(t) = fresh_note {
                return Some(from_note(t));
            }
            (!s.text.is_empty()).then(|| NextView {
                text: s.text,
                origin: "set",
                by: (!s.by.is_empty()).then_some(s.by),
                at_ms: s.set_at * 1000,
                todo: None,
            })
        }
    }
}

/// 가장 최근 daily 노트의 `다음 할 일` 후보 — 그 노트에서 마지막으로 적힌
/// 묶음(같은 제목 아래 이어진 줄들)의 첫 줄.
fn note_candidate(vault: &Path, project: &str) -> Option<TodoItem> {
    let node = vault::list_project_node(vault, project).ok()??;
    let mut dailies: Vec<(&DomainNode, NaiveDate)> = flatten(&node)
        .into_iter()
        .filter_map(|d| daily_date(&d.name).map(|dt| (d, dt)))
        .collect();
    dailies.sort_by(|a, b| b.1.cmp(&a.1));
    for (d, _) in dailies.into_iter().take(60) {
        let todos = todos_for(vault, project, d);
        let picks: Vec<&RawTodo> = todos.open.iter().filter(|t| t.next_hint).collect();
        let Some(last) = picks.last() else { continue };
        // 마지막 묶음의 첫 줄: 끝에서부터 같은 제목이 이어지는 동안 거슬러 오른다.
        let mut first = *last;
        for t in picks.iter().rev() {
            if t.heading != last.heading {
                break;
            }
            first = t;
        }
        return Some(to_item(project, d, first));
    }
    None
}

// ---------- 할 일 파싱 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoKind {
    /// `- [ ] …`
    Checkbox,
    /// `### TODO` · `## 다음 할 일` 제목 아래의 bullet.
    Section,
    /// `- 다음 할 일: …` · `TODO: …` 한 줄.
    Inline,
}

#[derive(Debug, Clone)]
struct RawTodo {
    /// 0부터 센 줄 번호 (`\n` 기준).
    line: usize,
    /// 원래 줄 그대로 — 고칠 때 이 줄이 맞는지 확인한다.
    raw: String,
    text: String,
    kind: TodoKind,
    heading: Option<String>,
    /// "다음에 할 일" 성격 — `다음 할 일:` 줄이거나 제목에 '다음'이 들어간 섹션.
    next_hint: bool,
}

#[derive(Debug, Clone, Default)]
struct DocTodos {
    open: Vec<RawTodo>,
    open_checkboxes: usize,
    done_checkboxes: usize,
}

fn heading_title(t: &str) -> Option<&str> {
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    rest.strip_prefix(' ').map(str::trim)
}

fn is_todo_heading(title: &str) -> bool {
    let l = title.to_lowercase();
    l.contains("todo") || l.contains("할 일") || l.contains("할일")
}

/// `  - rest` → `(indent, marker, rest)`.
fn split_bullet(line: &str) -> Option<(&str, &str, &str)> {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = body.strip_prefix(m) {
            return Some((indent, m, rest));
        }
    }
    None
}

fn open_box(rest: &str) -> Option<&str> {
    rest.strip_prefix("[ ] ")
}

fn done_box(rest: &str) -> Option<&str> {
    rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] "))
}

/// `다음 할 일: x` · `TODO: x` · `할 일: x` → `x`. 콜론 앞 공백·전각 콜론 허용.
fn inline_todo(rest: &str) -> Option<(&str, bool)> {
    let labels: [(&str, bool); 4] = [("다음 할 일", true), ("다음 할일", true), ("할 일", false), ("TODO", false)];
    for (label, next) in labels {
        let Some(head) = rest.get(..label.len()) else { continue };
        if !head.eq_ignore_ascii_case(label) {
            continue;
        }
        let after = rest[label.len()..].trim_start();
        let after = after
            .strip_prefix(':')
            .or_else(|| after.strip_prefix('：'))
            .map(str::trim)?;
        if after.is_empty() {
            return None;
        }
        return Some((after, next));
    }
    None
}

fn looks_done(rest: &str) -> bool {
    rest.starts_with("~~") || rest.contains('✅') || rest.contains("(완료)") || rest.contains("[완료]")
}

fn parse_todos(md: &str) -> DocTodos {
    let mut out = DocTodos::default();
    let mut in_fence = false;
    let mut in_comment = false;
    let mut heading: Option<String> = None;
    let mut todo_section = false;
    let mut next_section = false;
    for (i, line) in md.split('\n').enumerate() {
        let clean = line.strip_suffix('\r').unwrap_or(line);
        let t = clean.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if in_comment {
            in_comment = !t.contains("-->");
            continue;
        }
        if t.starts_with("<!--") {
            in_comment = !t.contains("-->");
            continue;
        }
        if let Some(h) = heading_title(t) {
            todo_section = is_todo_heading(h);
            next_section = todo_section && h.contains("다음");
            heading = Some(h.to_string());
            continue;
        }
        let mut push = |text: &str, kind: TodoKind, next_hint: bool| {
            // 화면에 보일 글 — 굵게(`**`) 표시는 뗀다. 원래 줄(raw)은 그대로.
            let text = text.trim().replace("**", "");
            let text = text.trim();
            if text.is_empty() {
                return;
            }
            out.open.push(RawTodo {
                line: i,
                raw: line.to_string(),
                text: clip(text, TODO_TEXT_CHARS),
                kind,
                heading: heading.clone(),
                next_hint,
            });
        };
        if let Some((indent, _, rest)) = split_bullet(clean) {
            if let Some(text) = open_box(rest) {
                let next = next_section || inline_todo(text).map(|(_, n)| n).unwrap_or(false);
                push(text, TodoKind::Checkbox, next);
                out.open_checkboxes += 1;
            } else if done_box(rest).is_some() {
                out.done_checkboxes += 1;
            } else if let Some((text, next)) = inline_todo(rest) {
                push(text, TodoKind::Inline, next || next_section);
            } else if todo_section && indent.len() < 2 && !looks_done(rest) {
                push(rest, TodoKind::Section, next_section);
            }
            continue;
        }
        if clean.len() == clean.trim_start().len() {
            if let Some((text, next)) = inline_todo(t) {
                push(text, TodoKind::Inline, next);
            }
        }
    }
    out
}

/// 할 일 줄을 완료(`[x]`)·미완료(`[ ]`)로 바꾼 줄. 체크박스가 아니던 줄은
/// 체크박스가 된다 (`- TODO: x` → `- [x] TODO: x`, `TODO: x` → `- [x] TODO: x`).
pub fn mark_line(raw: &str, done: bool) -> String {
    let (body, cr) = match raw.strip_suffix('\r') {
        Some(b) => (b, "\r"),
        None => (raw, ""),
    };
    let mark = if done { "[x] " } else { "[ ] " };
    let new = match split_bullet(body) {
        Some((indent, marker, rest)) => {
            let text = open_box(rest).or_else(|| done_box(rest)).unwrap_or(rest);
            format!("{indent}{marker}{mark}{text}")
        }
        None => {
            let t = body.trim_start();
            let indent = &body[..body.len() - t.len()];
            format!("{indent}- {mark}{t}")
        }
    };
    format!("{new}{cr}")
}

// 문서별 파싱 결과 캐시 — (mtime, bytes) 가 같으면 다시 읽지 않는다.
struct CachedDoc {
    mtime: Option<u128>,
    bytes: u64,
    todos: DocTodos,
}

fn doc_cache() -> &'static Mutex<HashMap<PathBuf, CachedDoc>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, CachedDoc>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn todos_for(vault: &Path, project: &str, d: &DomainNode) -> DocTodos {
    let key = vault.join(PROJECTS_DIRNAME).join(project).join(&d.name);
    if let Ok(c) = doc_cache().lock() {
        if let Some(hit) = c.get(&key) {
            if hit.mtime == d.modified_ms && hit.bytes == d.bytes && d.modified_ms.is_some() {
                return hit.todos.clone();
            }
        }
    }
    let md = vault::read_doc(vault, project, &d.name).unwrap_or_default();
    let todos = parse_todos(&md);
    if let Ok(mut c) = doc_cache().lock() {
        c.insert(
            key,
            CachedDoc { mtime: d.modified_ms, bytes: d.bytes, todos: todos.clone() },
        );
    }
    todos
}

#[derive(Debug, Clone, Serialize)]
pub struct TodoItem {
    pub project: String,
    pub domain: String,
    pub line: usize,
    pub raw: String,
    pub text: String,
    pub kind: TodoKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    /// 정렬 기준 — daily 는 그 날짜, 다른 문서는 수정 시각.
    pub at_ms: i64,
    pub modified_ms: i64,
    pub daily: bool,
}

fn doc_at_ms(d: &DomainNode) -> i64 {
    daily_date(&d.name)
        .map(day_end_ms)
        .unwrap_or_else(|| d.modified_ms.unwrap_or(0) as i64)
}

fn to_item(project: &str, d: &DomainNode, t: &RawTodo) -> TodoItem {
    TodoItem {
        project: project.to_string(),
        domain: d.name.clone(),
        line: t.line,
        raw: t.raw.clone(),
        text: t.text.clone(),
        kind: t.kind,
        heading: t.heading.clone(),
        at_ms: doc_at_ms(d),
        modified_ms: d.modified_ms.unwrap_or(0) as i64,
        daily: is_daily(&d.name),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Checklist {
    pub project: String,
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub open: usize,
    pub done: usize,
    pub at_ms: i64,
}

/// 프로젝트 하나의 열린 할 일 + 체크리스트 문서. `since_ms` 이전 문서는 뺀다.
fn project_todos(
    vault: &Path,
    node: &ProjectNode,
    since_ms: Option<i64>,
) -> (Vec<TodoItem>, Vec<Checklist>) {
    let mut items = Vec::new();
    let mut lists = Vec::new();
    for d in flatten(node) {
        if is_scaffold(&d.name) {
            continue;
        }
        let at = doc_at_ms(d);
        if since_ms.is_some_and(|s| at < s) {
            continue;
        }
        let todos = todos_for(vault, &node.name, d);
        if todos.open.is_empty() && todos.done_checkboxes == 0 {
            continue;
        }
        let checklist = !is_daily(&d.name) && todos.open_checkboxes >= CHECKLIST_MIN_OPEN;
        if checklist {
            lists.push(Checklist {
                project: node.name.clone(),
                domain: d.name.clone(),
                title: d.title.clone(),
                open: todos.open_checkboxes,
                done: todos.done_checkboxes,
                at_ms: at,
            });
        }
        for t in &todos.open {
            if checklist && t.kind == TodoKind::Checkbox {
                continue;
            }
            items.push(to_item(&node.name, d, t));
        }
    }
    (items, lists)
}

fn sort_items(items: &mut [TodoItem]) {
    items.sort_by(|a, b| {
        b.at_ms
            .cmp(&a.at_ms)
            .then_with(|| a.project.cmp(&b.project))
            .then_with(|| a.domain.cmp(&b.domain))
            .then_with(|| a.line.cmp(&b.line))
    });
}

#[derive(Debug, Clone, Serialize)]
pub struct TodoInbox {
    pub items: Vec<TodoItem>,
    pub checklists: Vec<Checklist>,
    /// 자르기 전 항목 수.
    pub total_items: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    /// 보관해서 뺀 프로젝트 수 (project 를 지정하면 0).
    pub archived_skipped: usize,
}

/// vault 전체(또는 한 프로젝트)의 열린 할 일. 보관한 프로젝트는 빠진다
/// — 직접 고르면 보인다.
pub fn todo_inbox(
    vault: &Path,
    cfg: &DanbiConfig,
    days: Option<u32>,
    project: Option<&str>,
) -> DanbiResult<TodoInbox> {
    let tree = vault::list_tree(vault)?;
    let since = days.filter(|d| *d > 0).map(|d| now_ms() - d as i64 * DAY_MS);
    let mut items = Vec::new();
    let mut lists = Vec::new();
    let mut archived_skipped = 0;
    for node in &tree.projects {
        if let Some(p) = project {
            if node.name != p {
                continue;
            }
        } else if cfg.project_archived.contains(&node.name) {
            archived_skipped += 1;
            continue;
        }
        let (i, l) = project_todos(vault, node, since);
        items.extend(i);
        lists.extend(l);
    }
    sort_items(&mut items);
    lists.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    let total_items = items.len();
    let truncated = total_items > INBOX_LIMIT;
    items.truncate(INBOX_LIMIT);
    Ok(TodoInbox { items, checklists: lists, total_items, truncated, days, archived_skipped })
}

/// 할 일 한 줄을 완료/미완료로 바꾸고 커밋한다. 줄 번호가 밀렸으면 같은
/// 내용의 줄이 문서에 하나뿐일 때만 그 줄을 고친다.
pub fn set_todo_done(
    vault: &Path,
    project: &str,
    domain: &str,
    line: usize,
    raw: &str,
    done: bool,
) -> DanbiResult<String> {
    let _guard = crate::vcs::write_guard();
    let md = vault::read_doc(vault, project, domain)?;
    let mut lines: Vec<&str> = md.split('\n').collect();
    let idx = if lines.get(line) == Some(&raw) {
        line
    } else {
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| **l == raw)
            .map(|(i, _)| i)
            .collect();
        match hits.as_slice() {
            [one] => *one,
            _ => {
                return Err(DanbiError::Other(
                    "문서가 바뀌어서 이 할 일을 찾지 못했어요. 목록을 새로고침해 주세요.".into(),
                ))
            }
        }
    };
    let new_line = mark_line(raw, done);
    if new_line == raw {
        return Ok(new_line);
    }
    lines[idx] = &new_line;
    let next = lines.join("\n");
    crate::vcs::ensure_repo(vault)?;
    let _ = crate::vcs::snapshot(vault, &format!("danbi: todo · {project}/{domain} (pre)"));
    vault::write_doc(vault, project, domain, &next)?;
    let verb = if done { "완료" } else { "다시 열기" };
    if let Err(e) = crate::vcs::snapshot_as(
        vault,
        &format!("danbi: todo {verb} · {project}/{domain}"),
        crate::vcs::CLIENT_HUMAN,
    ) {
        eprintln!("[todo] commit failed: {e}");
    }
    Ok(new_line)
}

// ---------- 쉬는 중 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseState {
    Active,
    /// 오래 손대지 않음 (자동).
    Resting,
    /// 사용자가 보관함.
    Archived,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectPulse {
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_days: Option<i64>,
    pub state: PulseState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// 뼈대 문서를 뺀 가장 최근 수정 시각.
pub fn last_doc_ms(node: &ProjectNode) -> Option<i64> {
    flatten(node)
        .into_iter()
        .filter(|d| !is_scaffold(&d.name))
        .filter_map(|d| d.modified_ms)
        .max()
        .map(|m| m as i64)
}

fn pulse_of(cfg: &DanbiConfig, node: &ProjectNode, now: i64) -> (Option<i64>, Option<i64>, PulseState) {
    let resumed = cfg.project_resumed_at.get(&node.name).map(|s| s * 1000);
    let last = match (last_doc_ms(node), resumed) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    let idle = last.map(|l| ((now - l).max(0)) / DAY_MS);
    let dormant = cfg.dormant_days() as i64;
    let state = if cfg.project_archived.contains(&node.name) {
        PulseState::Archived
    } else if dormant > 0 && idle.is_some_and(|d| d >= dormant) {
        PulseState::Resting
    } else {
        PulseState::Active
    };
    (last, idle, state)
}

pub fn pulse_all(vault: &Path, cfg: &DanbiConfig, tree: &VaultTree) -> Vec<ProjectPulse> {
    let now = now_ms();
    tree.projects
        .iter()
        .map(|node| {
            let (last, idle, state) = pulse_of(cfg, node, now);
            ProjectPulse {
                project: node.name.clone(),
                last_activity_ms: last,
                idle_days: idle,
                state,
                next: load_next(vault, &node.name).map(|n| n.text).filter(|t| !t.is_empty()),
            }
        })
        .collect()
}

// ---------- 재진입 카드 ----------

#[derive(Debug, Clone, Serialize)]
pub struct NoteLine {
    pub domain: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentDoc {
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub modified_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResumeCard {
    pub project: String,
    pub state: PulseState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<NextView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_note: Option<NoteLine>,
    pub todos: Vec<TodoItem>,
    pub todo_total: usize,
    pub checklists: Vec<Checklist>,
    pub decisions: Vec<crate::dashboard::ActiveDecision>,
    pub recent_docs: Vec<RecentDoc>,
}

pub fn resume_card(vault: &Path, cfg: &DanbiConfig, project: &str) -> DanbiResult<ResumeCard> {
    let node = vault::list_project_node(vault, project)?
        .ok_or_else(|| DanbiError::Config(format!("unknown project: {project}")))?;
    let (last, idle, state) = pulse_of(cfg, &node, now_ms());
    let docs = flatten(&node);

    let last_note = docs
        .iter()
        .filter_map(|d| daily_date(&d.name).map(|dt| (d, dt)))
        .max_by_key(|(_, dt)| *dt)
        .and_then(|(d, _)| {
            let md = vault::read_doc(vault, project, &d.name).ok()?;
            crate::session_hooks::daily_one_liner(&md)
                .map(|s| NoteLine { domain: d.name.clone(), summary: clip(&s, 160) })
        });

    let (mut todos, mut checklists) = project_todos(vault, &node, None);
    sort_items(&mut todos);
    let next = next_view(vault, project);
    // 다음 한 걸음으로 이미 보여주는 줄은 목록에서 뺀다.
    if let Some(t) = next.as_ref().and_then(|n| n.todo.as_ref()) {
        todos.retain(|x| !(x.domain == t.domain && x.line == t.line));
    }
    let todo_total = todos.len();
    todos.truncate(CARD_TODOS);
    checklists.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    checklists.truncate(CARD_CHECKLISTS);

    let mut recent: Vec<&&DomainNode> = docs
        .iter()
        .filter(|d| !is_daily(&d.name) && !is_scaffold(&d.name))
        .collect();
    recent.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    let recent_docs = recent
        .into_iter()
        .take(3)
        .map(|d| RecentDoc {
            domain: d.name.clone(),
            title: d.title.clone(),
            modified_ms: d.modified_ms.unwrap_or(0) as i64,
        })
        .collect();

    Ok(ResumeCard {
        project: project.to_string(),
        state,
        last_activity_ms: last,
        idle_days: idle,
        next,
        last_note,
        todos,
        todo_total,
        checklists,
        decisions: crate::dashboard::active_decisions(vault, Some(project), 3),
        recent_docs,
    })
}

// ---------- 시간 분배 ----------

#[derive(Debug, Clone, Serialize)]
pub struct TimeRow {
    pub project: String,
    /// Claude Code 작업 시간 (5분 칸 기준, 분).
    pub minutes: u64,
    pub tokens: u64,
    pub sessions: u64,
    /// vault 기록(커밋) 수.
    pub records: u32,
    pub mcp_calls: u64,
    /// 0~1. 작업 시간이 있으면 시간 기준, 없으면 기록 수 기준.
    pub share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmatchedFolder {
    pub cwd: String,
    pub label: String,
    pub minutes: u64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeSplit {
    pub days: i64,
    pub rows: Vec<TimeRow>,
    pub unmatched: Vec<UnmatchedFolder>,
    pub total_minutes: u64,
    pub unmatched_minutes: u64,
    pub total_records: u32,
    /// 시간 비율이 작업 시간 기준인지 (false 면 기록 수 기준).
    pub by_minutes: bool,
    /// Claude Code 사용량 추적이 켜져 있는지.
    pub tracking: bool,
    /// 연결 파일로 프로젝트에 묶인 코드 폴더 수.
    pub connected_folders: usize,
    /// 사용자가 직접 묶은 폴더 수.
    pub manual_folders: usize,
}

/// 연결 레지스트리(`~/.danbi/connections.json`)의 `(코드 폴더, 프로젝트)`.
pub fn connected_folders(home: &Path) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for c in crate::agent_connect::load_registry(&crate::agent_connect::registry_path(home)) {
        let Some(agent) = crate::agent_connect::Agent::parse(&c.agent) else { continue };
        let depth = Path::new(agent.config_rel_path()).components().count();
        let mut folder = PathBuf::from(&c.path);
        for _ in 0..depth {
            folder.pop();
        }
        if folder.as_os_str().is_empty() {
            continue;
        }
        if !out.iter().any(|(f, p)| *f == folder && *p == c.project) {
            out.push((folder, c.project));
        }
    }
    out
}

/// git worktree 안이면 원래 저장소 폴더 (`.git` 파일의 `gitdir:` 로 찾음).
fn main_repo_of(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors().take(8) {
        let git = dir.join(".git");
        if git.is_dir() {
            return None;
        }
        if git.is_file() {
            let raw = std::fs::read_to_string(&git).ok()?;
            let gitdir = raw.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
            let (main, _) = gitdir.split_once("/.git/worktrees/")?;
            return Some(PathBuf::from(main));
        }
    }
    None
}

fn match_folder<'a>(cwd: &Path, folders: &'a [(PathBuf, String)]) -> Option<&'a str> {
    folders
        .iter()
        .filter(|(f, _)| cwd.starts_with(f))
        .max_by_key(|(f, _)| f.components().count())
        .map(|(_, p)| p.as_str())
}

/// cwd → 프로젝트. 연결 폴더 안이거나, 그 저장소의 worktree 안이면 그 프로젝트.
pub fn project_for_cwd<'a>(cwd: &Path, folders: &'a [(PathBuf, String)]) -> Option<&'a str> {
    match_folder(cwd, folders).or_else(|| main_repo_of(cwd).and_then(|m| match_folder(&m, folders)))
}

fn row_mut<'a>(rows: &'a mut HashMap<String, TimeRow>, project: &str) -> &'a mut TimeRow {
    rows.entry(project.to_string()).or_insert_with(|| TimeRow {
        project: project.to_string(),
        minutes: 0,
        tokens: 0,
        sessions: 0,
        records: 0,
        mcp_calls: 0,
        share: 0.0,
    })
}

pub fn time_split(vault: &Path, cfg: &DanbiConfig, days: i64) -> DanbiResult<TimeSplit> {
    let days = days.clamp(1, 365);
    let tree = vault::list_tree_with(vault, false)?;
    let known: std::collections::HashSet<&str> = tree.projects.iter().map(|p| p.name.as_str()).collect();
    let home = dirs::home_dir().unwrap_or_default();
    // 직접 묶은 폴더는 뒤에 붙인다 — 같은 폴더가 겹치면 match_folder 가
    // (max_by_key 라) 마지막 것을 고르므로 사용자가 고른 쪽이 이긴다.
    let mut folders: Vec<(PathBuf, String)> = connected_folders(&home)
        .into_iter()
        .filter(|(_, p)| known.contains(p.as_str()))
        .collect();
    let connected = folders.len();
    for (folder, project) in &cfg.folder_projects {
        if known.contains(project.as_str()) {
            folders.push((PathBuf::from(folder), project.clone()));
        }
    }

    let mut rows: HashMap<String, TimeRow> = HashMap::new();

    let mut unmatched: HashMap<String, UnmatchedFolder> = HashMap::new();
    let tracking = cfg.usage.claude_code_tracking;
    if tracking {
        let to = now_ms() + 1;
        let from = to - days * DAY_MS;
        for a in crate::claude_code_usage::cwd_activity(from, to) {
            let cwd = PathBuf::from(&a.cwd);
            match project_for_cwd(&cwd, &folders) {
                Some(p) => {
                    let r = row_mut(&mut rows, p);
                    r.minutes += a.minutes;
                    r.tokens += a.tokens;
                    r.sessions += a.sessions;
                }
                None => {
                    // worktree 들은 원래 저장소 하나로 모은다.
                    let key = main_repo_of(&cwd).unwrap_or(cwd);
                    let key_s = key.to_string_lossy().to_string();
                    let label = key
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| key_s.clone());
                    let u = unmatched.entry(key_s.clone()).or_insert(UnmatchedFolder {
                        cwd: key_s,
                        label,
                        minutes: 0,
                        tokens: 0,
                    });
                    u.minutes += a.minutes;
                    u.tokens += a.tokens;
                }
            }
        }
    }

    if let Ok(ov) = crate::dashboard::project_activity_overview(vault, days) {
        for p in ov.by_project {
            if p.commits == 0 && p.mcp_calls == 0 {
                continue;
            }
            let r = row_mut(&mut rows, &p.project);
            r.records = p.commits;
            r.mcp_calls = p.mcp_calls;
        }
    }

    let mut rows: Vec<TimeRow> = rows.into_values().collect();
    let total_minutes: u64 = rows.iter().map(|r| r.minutes).sum();
    let total_records: u32 = rows.iter().map(|r| r.records).sum();
    let by_minutes = total_minutes > 0;
    for r in rows.iter_mut() {
        r.share = if by_minutes {
            r.minutes as f64 / total_minutes as f64
        } else if total_records > 0 {
            r.records as f64 / total_records as f64
        } else {
            0.0
        };
    }
    rows.sort_by(|a, b| {
        b.minutes
            .cmp(&a.minutes)
            .then_with(|| b.records.cmp(&a.records))
            .then_with(|| a.project.cmp(&b.project))
    });
    let mut unmatched: Vec<UnmatchedFolder> = unmatched.into_values().collect();
    unmatched.sort_by(|a, b| b.minutes.cmp(&a.minutes));
    let unmatched_minutes = unmatched.iter().map(|u| u.minutes).sum();
    unmatched.truncate(6);

    Ok(TimeSplit {
        days,
        rows,
        unmatched,
        total_minutes,
        unmatched_minutes,
        total_records,
        by_minutes,
        tracking,
        connected_folders: connected,
        manual_folders: folders.len() - connected,
    })
}

// ---------- 다른 프로젝트의 비슷한 기록 ----------

#[derive(Debug, Clone, Serialize)]
pub struct SimilarHit {
    pub project: String,
    pub domain: String,
    pub snippet: String,
    pub relevance: f32,
}

const PROBLEM_WORDS: [&str; 22] = [
    "버그", "원인", "에러", "오류", "크래시", "실패", "안 됨", "안됨", "안 돼", "안돼", "고침",
    "해결", "재발", "증상", "bug", "error", "crash", "exception", "fix", "panic", "timeout", "failed",
];

/// 버그·원인·해결 성격의 기록인가 — 이때만 다른 프로젝트를 찾아본다.
pub fn looks_like_problem(text: &str) -> bool {
    let l = text.to_lowercase();
    PROBLEM_WORDS.iter().any(|w| l.contains(w))
}

const STOP_WORDS: [&str; 40] = [
    "결정", "원인", "버그", "수정", "완료", "다음", "기록", "세션", "자동", "오늘", "관련", "문제",
    "해결", "이유", "적용", "추가", "확인", "진행", "정리", "작업", "에러", "오류", "증상", "재발",
    "방지", "노하우", "있음", "없음", "때문", "경우", "the", "and", "for", "with", "this", "that",
    "todo", "fix", "bug", "error",
];

/// 검색어로 쓸 핵심 낱말 (제목 먼저, 최대 `max` 개). tantivy 문법 문자는 지운다.
pub fn query_terms(text: &str, max: usize) -> String {
    query_term_list(text, max, &[]).join(" ")
}

/// `query_terms` 의 낱말 목록. `skip` (소문자) 은 빼는 낱말 — 프로젝트 이름처럼
/// 여러 프로젝트 daily 에 흔히 나와 엉뚱한 문서를 끌어오는 것.
fn query_term_list(text: &str, max: usize, skip: &[String]) -> Vec<String> {
    let mut heads = Vec::new();
    let mut body = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || crate::daily::is_log_stamp(t) {
            continue;
        }
        if heading_title(t).is_some() {
            heads.push(t);
        } else if body.len() < 6 {
            body.push(t);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for src in heads.into_iter().chain(body) {
        let cleaned: String = src
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' { c } else { ' ' })
            .collect();
        for tok in cleaned.split_whitespace() {
            let low = tok.to_lowercase();
            if tok.chars().count() < 2
                || tok.chars().all(|c| c.is_ascii_digit())
                || STOP_WORDS.contains(&low.as_str())
                || skip.contains(&low)
                || out.iter().any(|o| o.to_lowercase() == low)
            {
                continue;
            }
            out.push(tok.to_string());
            if out.len() >= max {
                return out;
            }
        }
    }
    out
}

/// vault 의 프로젝트 이름들 (소문자).
fn project_names(vault: &Path) -> Vec<String> {
    std::fs::read_dir(vault.join(PROJECTS_DIRNAME))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// 검색어 중 몇 개가 본문에 실제로 들어 있어야 "비슷하다" 고 볼지.
/// BM25 점수는 1등 기준 상대값이라 그것만으로는 늘 뭔가가 걸린다.
fn min_shared_terms(n: usize) -> usize {
    ((n * 2 + 4) / 5).max(2).min(n)
}

/// `project` 밖에서 `text` 와 비슷한 기록. 키워드 검색(LLM·임베딩 없음).
pub fn similar_elsewhere(vault: &Path, project: &str, text: &str, limit: usize) -> Vec<SimilarHit> {
    let terms = query_term_list(text, 10, &project_names(vault));
    if terms.len() < 2 {
        return Vec::new();
    }
    let need = min_shared_terms(terms.len());
    let lows: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let Ok(hits) = crate::search::full_search(vault, &terms.join(" "), 30) else {
        return Vec::new();
    };
    let mut out: Vec<SimilarHit> = Vec::new();
    for h in hits {
        if h.project == project || is_scaffold(&h.domain) || h.relevance < 0.35 {
            continue;
        }
        if out.iter().any(|o| o.project == h.project && o.domain == h.domain) {
            continue;
        }
        let Ok(body) = vault::read_doc(vault, &h.project, &h.domain) else {
            continue;
        };
        let body = body.to_lowercase();
        if lows.iter().filter(|t| body.contains(t.as_str())).count() < need {
            continue;
        }
        out.push(SimilarHit {
            project: h.project,
            domain: h.domain,
            snippet: clip(h.snippet.trim(), 160),
            relevance: h.relevance,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// 문서 하나를 기준으로 다른 프로젝트의 비슷한 기록 (인스펙터용).
pub fn similar_for_doc(vault: &Path, project: &str, domain: &str, limit: usize) -> DanbiResult<Vec<SimilarHit>> {
    let md = vault::read_doc(vault, project, domain)?;
    // frontmatter 는 건너뛴다.
    let body = match md.strip_prefix("---\n").and_then(|r| r.split_once("\n---")) {
        Some((_, rest)) => rest.to_string(),
        None => md,
    };
    let head: String = body.chars().take(1500).collect();
    Ok(similar_elsewhere(vault, project, &head, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "danbi-pulse-{tag}-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join(PROJECTS_DIRNAME)).unwrap();
        dir
    }

    fn write(vault: &Path, project: &str, domain: &str, body: &str) {
        let p = vault.join(PROJECTS_DIRNAME).join(project).join(domain);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn parses_checkboxes_sections_and_inline_todos() {
        let md = "# 2026-11-02\n\n<!-- daily note\n- [ ] 주석 안 -->\n\n### TODO\n- 배포 확인\n  - 세부 설명\n- [x] 끝난 일\n- ~~취소~~\n\n### 세션 자동 기록 · 13:20\n- 완료: 로그인 수정\n- 다음 할 일: `popover.rs` 좌표 고치기\n- 다음 할 일: 테스트 추가\n\n```\n- [ ] 코드 안\n```\nTODO: 대표 확인\n- [ ] 체크 하나\n";
        let t = parse_todos(md);
        assert_eq!(parse_todos("- [ ] **굵은** 일\n").open[0].text, "굵은 일");
        let texts: Vec<&str> = t.open.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["배포 확인", "`popover.rs` 좌표 고치기", "테스트 추가", "대표 확인", "체크 하나"]
        );
        assert_eq!(t.open[0].kind, TodoKind::Section);
        assert_eq!(t.open[1].kind, TodoKind::Inline);
        assert!(t.open[1].next_hint && t.open[2].next_hint);
        assert!(!t.open[3].next_hint);
        assert_eq!(t.open[4].kind, TodoKind::Checkbox);
        assert_eq!(t.open_checkboxes, 1);
        assert_eq!(t.done_checkboxes, 1);
        // 줄 번호는 \n 기준 0부터.
        assert_eq!(md.split('\n').nth(t.open[0].line), Some("- 배포 확인"));
    }

    #[test]
    fn mark_line_round_trips() {
        assert_eq!(mark_line("- [ ] 하나", true), "- [x] 하나");
        assert_eq!(mark_line("  * [x] 둘", false), "  * [ ] 둘");
        assert_eq!(mark_line("- 다음 할 일: 셋", true), "- [x] 다음 할 일: 셋");
        assert_eq!(mark_line("TODO: 넷", true), "- [x] TODO: 넷");
        assert_eq!(mark_line("- 다섯\r", true), "- [x] 다섯\r");
        // 완료로 바꾼 줄은 다시 파싱해도 열린 할 일이 아니다.
        let md = format!("### TODO\n{}\n{}\n", mark_line("- 배포", true), mark_line("TODO: 확인", true));
        let t = parse_todos(&md);
        assert!(t.open.is_empty());
        assert_eq!(t.done_checkboxes, 2);
    }

    #[test]
    fn inbox_groups_big_plans_into_checklists_and_toggle_writes_file() {
        let vault = temp_vault("inbox");
        let today = Local::now().format("%Y-%m-%d").to_string();
        write(&vault, "알파", &format!("daily/{today}.md"), "# d\n\n### TODO\n- 배포 확인\n- [ ] 로그 보기\n");
        let plan: String = (0..8).map(|i| format!("- [ ] 단계 {i}\n")).collect::<String>() + "- [x] 준비\n";
        write(&vault, "알파", "plan.md", &format!("# 계획\n{plan}"));
        write(&vault, "베타", "notes.md", "# 메모\n- [ ] 베타 일\n");
        let mut cfg = DanbiConfig::default();
        let inbox = todo_inbox(&vault, &cfg, None, None).unwrap();
        let texts: Vec<&str> = inbox.items.iter().map(|i| i.text.as_str()).collect();
        assert!(texts.contains(&"배포 확인") && texts.contains(&"로그 보기") && texts.contains(&"베타 일"));
        assert!(!texts.iter().any(|t| t.starts_with("단계")), "big plans are summarised");
        assert_eq!(inbox.checklists.len(), 1);
        assert_eq!((inbox.checklists[0].open, inbox.checklists[0].done), (8, 1));
        // daily 가 맨 위 (오늘 날짜 기준).
        assert_eq!(inbox.items[0].domain, format!("daily/{today}.md"));

        // 보관한 프로젝트는 빠진다.
        cfg.project_archived.push("베타".into());
        let inbox2 = todo_inbox(&vault, &cfg, None, None).unwrap();
        assert!(!inbox2.items.iter().any(|i| i.project == "베타"));
        assert_eq!(inbox2.archived_skipped, 1);
        assert_eq!(todo_inbox(&vault, &cfg, None, Some("베타")).unwrap().items.len(), 1);

        // 완료 처리 → 파일에 [x], 목록에서 빠짐. 줄이 밀려도 같은 줄이 하나면 찾는다.
        let item = inbox.items.iter().find(|i| i.text == "배포 확인").unwrap().clone();
        let path = vault.join(PROJECTS_DIRNAME).join("알파").join(&item.domain);
        let shifted = format!("맨 위 줄\n{}", std::fs::read_to_string(&path).unwrap());
        std::fs::write(&path, shifted).unwrap();
        set_todo_done(&vault, "알파", &item.domain, item.line, &item.raw, true).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("- [x] 배포 확인"));
        let after = todo_inbox(&vault, &cfg, None, Some("알파")).unwrap();
        assert!(!after.items.iter().any(|i| i.text == "배포 확인"));
        assert!(set_todo_done(&vault, "알파", &item.domain, item.line, "- 없는 줄", true).is_err());
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn next_step_prefers_newer_note_but_not_the_one_it_replaced() {
        let vault = temp_vault("next");
        write(&vault, "알파", "plan.md", "# 계획\n");
        assert!(next_view(&vault, "알파").is_none());

        write(
            &vault,
            "알파",
            "daily/2026-11-01.md",
            "# d\n### 세션 자동 기록 · 10:00\n- 다음 할 일: 옛날 일\n### 세션 자동 기록 · 18:00\n- 다음 할 일: 첫째\n- 다음 할 일: 둘째\n",
        );
        let v = next_view(&vault, "알파").unwrap();
        assert_eq!((v.origin, v.text.as_str()), ("note", "첫째"), "first line of the latest block");

        // 사용자가 적으면 그 값. 같은 노트 후보는 다시 덮지 않는다.
        set_next(&vault, "알파", "직접 적은 일\n둘째 줄", "human").unwrap();
        let v = next_view(&vault, "알파").unwrap();
        assert_eq!((v.origin, v.text.as_str()), ("set", "직접 적은 일"));

        // 완료(빈 값) → 같은 후보가 되살아나지 않는다.
        assert!(set_next(&vault, "알파", "", "human").unwrap().is_none());
        assert!(next_view(&vault, "알파").is_none());

        // 더 새 노트의 다른 후보는 보인다.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&vault, "알파", "daily/2026-11-02.md", "# d\n- 다음 할 일: 새 일\n");
        assert_eq!(next_view(&vault, "알파").unwrap().text, "새 일");
        assert!(set_next(&vault, "없음", "x", "human").is_err());
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn pulse_marks_resting_archived_and_resumed() {
        let vault = temp_vault("pulse");
        write(&vault, "새것", "plan.md", "# x\n");
        write(&vault, "옛것", "plan.md", "# x\n");
        write(&vault, "옛것", "SKILL.md", "# 뼈대는 활동 아님\n");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 86_400);
        let f = std::fs::File::options()
            .write(true)
            .open(vault.join(PROJECTS_DIRNAME).join("옛것/plan.md"))
            .unwrap();
        f.set_modified(old).unwrap();
        let mut cfg = DanbiConfig::default();
        let tree = vault::list_tree(&vault).unwrap();
        let state = |cfg: &DanbiConfig| -> HashMap<String, PulseState> {
            pulse_all(&vault, cfg, &tree).into_iter().map(|p| (p.project, p.state)).collect()
        };
        let s = state(&cfg);
        assert_eq!(s["새것"], PulseState::Active);
        assert_eq!(s["옛것"], PulseState::Resting);

        cfg.dormant_days = Some(0);
        assert_eq!(state(&cfg)["옛것"], PulseState::Active, "0 = off");
        cfg.dormant_days = None;
        cfg.project_resumed_at.insert("옛것".into(), now_secs());
        assert_eq!(state(&cfg)["옛것"], PulseState::Active);
        cfg.project_archived.push("새것".into());
        assert_eq!(state(&cfg)["새것"], PulseState::Archived);
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn cwd_maps_to_connected_folder_longest_first() {
        let folders = vec![
            (PathBuf::from("/w/agent"), "상위".to_string()),
            (PathBuf::from("/w/agent/danbi"), "단비".to_string()),
        ];
        assert_eq!(project_for_cwd(Path::new("/w/agent/danbi/src-tauri"), &folders), Some("단비"));
        assert_eq!(project_for_cwd(Path::new("/w/agent/bonny"), &folders), Some("상위"));
        assert_eq!(project_for_cwd(Path::new("/w/agent/danbi-wt"), &folders), Some("상위"));
        assert_eq!(project_for_cwd(Path::new("/other"), &folders), None);
    }

    #[test]
    fn worktree_resolves_to_main_repo() {
        let root = temp_vault("wt");
        let main = root.join("repo");
        let wt = root.join("repo-wt");
        std::fs::create_dir_all(main.join(".git/worktrees/x")).unwrap();
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}/.git/worktrees/x\n", main.display())).unwrap();
        let folders = vec![(main.clone(), "단비".to_string())];
        assert_eq!(project_for_cwd(&wt.join("src"), &folders), Some("단비"));
        assert_eq!(main_repo_of(&main), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn query_terms_skip_generic_words_and_syntax() {
        assert!(looks_like_problem("### 버그 원인: 팝오버 위치"));
        assert!(!looks_like_problem("### 회의 메모"));
        let q = query_terms("### 버그 원인: macOS 27 팝오버 위치\n_10:00 · claude-code_\n- `TrayIconEvent` rect 가 다른 모니터 기준 (x:2053)", 10);
        assert!(q.starts_with("macOS 팝오버 위치"), "pure numbers dropped: {q}");
        assert!(q.contains("TrayIconEvent") && q.contains("rect"));
        assert!(!q.contains("버그") && !q.contains("원인") && !q.contains(':') && !q.contains('`'));
        assert!(!q.contains("10") && !q.contains("claude"), "stamp line skipped: {q}");
        assert_eq!(query_terms("- a b", 10), "");
        let skip = vec!["보니파트너스".to_string()];
        assert_eq!(query_term_list("### 보니파트너스 쓰레드 게시", 10, &skip), vec!["쓰레드", "게시"]);
        assert_eq!((min_shared_terms(2), min_shared_terms(5), min_shared_terms(10)), (2, 2, 4));
    }

    #[test]
    fn similar_elsewhere_skips_own_project() {
        let vault = temp_vault("similar");
        write(&vault, "알파", "daily/2026-11-01.md", "# d\n### 팝오버 모니터 좌표 버그\n- TrayIconEvent rect 기준이 다른 모니터\n");
        write(&vault, "베타", "notes/tray.md", "# 트레이\n- TrayIconEvent rect 모니터 좌표가 틀어짐, 클릭 위치로 해결\n");
        write(&vault, "감마", "notes/x.md", "# 무관\n- 결제 모듈 리팩터링\n");
        // 한 낱말만 겹치는 문서는 비슷한 기록이 아니다.
        write(&vault, "델타", "notes/once.md", "# 메모\n- 모니터 새로 삼\n");
        let hits = similar_elsewhere(&vault, "알파", "### 팝오버 모니터 좌표\n- TrayIconEvent rect", 3);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!((hits[0].project.as_str(), hits[0].domain.as_str()), ("베타", "notes/tray.md"));
        let _ = std::fs::remove_dir_all(&vault);
    }
}

#[cfg(test)]
mod real_vault {
    use super::*;

    /// 실제 vault 로 숫자만 찍어 본다 (읽기 전용). `cargo test --lib real_vault_pulse -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_vault_pulse() {
        let vault = crate::config::default_vault_path().unwrap();
        let cfg = crate::config::load_config(&vault).unwrap().unwrap();
        let vault = PathBuf::from(cfg.vault_path.clone().unwrap());
        let t = std::time::Instant::now();
        let all = todo_inbox(&vault, &cfg, None, None).unwrap();
        let cold = t.elapsed();
        let t = std::time::Instant::now();
        let d30 = todo_inbox(&vault, &cfg, Some(30), None).unwrap();
        let d7 = todo_inbox(&vault, &cfg, Some(7), None).unwrap();
        println!(
            "inbox all={} (cold {:?}) 30d={} 7d={} (warm {:?}) checklists={}",
            all.total_items, cold, d30.total_items, d7.total_items, t.elapsed(), all.checklists.len()
        );
        for i in d7.items.iter().take(12) {
            println!("  {:?} {}/{} :: {}", i.kind, i.project, i.domain, i.text);
        }
        let tree = vault::list_tree(&vault).unwrap();
        for p in pulse_all(&vault, &cfg, &tree) {
            if p.state != PulseState::Active {
                println!("  rest {:?} {} {:?}일", p.state, p.project, p.idle_days);
            }
        }
        let card = resume_card(&vault, &cfg, "단비").unwrap();
        println!("card next={:?} todos={} total={} decisions={}", card.next.as_ref().map(|n| (&n.text, n.origin)), card.todos.len(), card.todo_total, card.decisions.len());
        let t = std::time::Instant::now();
        let ts = time_split(&vault, &cfg, 7).unwrap();
        println!("time_split {:?} total_min={} unmatched_min={} folders={}", t.elapsed(), ts.total_minutes, ts.unmatched_minutes, ts.connected_folders);
        for r in ts.rows.iter().take(8) {
            println!("  {} {}분 기록{} {:.0}%", r.project, r.minutes, r.records, r.share * 100.0);
        }
        for u in &ts.unmatched {
            println!("  ? {} {}분", u.cwd, u.minutes);
        }
        let s = similar_elsewhere(&vault, "단비", "### macOS 27 팝오버 위치 버그\n- TrayIconEvent rect 가 다른 모니터 기준", 3);
        println!("similar {s:?}");
    }
}
