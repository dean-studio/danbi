//! Claude Code 세션 훅 지원.
//!
//! - 세션 시작 브리핑: `/api/session-brief` 가 돌려주는 짧은 Markdown.
//!   SessionStart 훅의 stdout 이 그대로 세션 맥락에 들어가므로 LLM 호출
//!   없이 파일 mtime · daily 노트 · 체크박스만으로 빠르게 만든다.
//! - 훅 설치: 코드 폴더의 `.claude/settings.local.json` 에 Danbi 훅을
//!   병합하고, 토큰을 담지 않는 도우미 스크립트(`~/.danbi/bin/danbi-hook`)
//!   를 쓴다. 도우미는 실행 때마다 단비 설정에서 포트·토큰을 읽는다.

use crate::error::{DanbiError, DanbiResult};
use crate::vault::{self, DomainNode, ProjectNode};
use serde_json::{json, Value};
use std::path::Path;

pub const DEFAULT_BRIEF_CHARS: usize = 2500;
const BRIEF_RECENT_DOCS: usize = 8;
const BRIEF_TODO_SCAN_DOCS: usize = 15;
const BRIEF_MAX_TODOS: usize = 5;
const BRIEF_LINE_CHARS: usize = 180;

/// Every Danbi-managed hook command runs this helper, so its file name
/// doubles as the marker that lets re-install find and replace our own
/// entries without touching the user's other hooks.
pub const HOOK_HELPER_NAME: &str = "danbi-hook";
/// Seconds Claude Code waits for the hook. The helper itself gives up
/// after ~2s, this is just the outer guard.
const HOOK_TIMEOUT_SECS: u64 = 5;

// ---------- Session brief ----------

/// Builds the SessionStart brief. `project` = None → short vault-wide
/// brief. Output never exceeds `max_chars` characters.
pub fn session_brief(vault: &Path, project: Option<&str>, max_chars: usize) -> DanbiResult<String> {
    // Names and mtimes only — no per-file heading sniff.
    let tree = vault::list_tree_with(vault, false)?;
    let now_ms = chrono::Local::now().timestamp_millis() as u128;
    let today = chrono::Local::now().date_naive();
    let yesterday = today - chrono::Duration::days(1);
    let mut lines: Vec<String> = Vec::new();

    match project {
        Some(name) => {
            let node = tree
                .projects
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| DanbiError::Config(format!("unknown project: {name}")))?;
            lines.push(format!("# 단비 브리핑 · {name}"));
            let goals = crate::goals::active_titles(vault, name);
            if !goals.is_empty() {
                lines.push(format!("진행 중 목표: {}", goals.join(" / ")));
            }

            let mut docs = flatten_domains(node);
            docs.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
            let recent: Vec<&&DomainNode> = docs
                .iter()
                .filter(|d| !is_daily(&d.name))
                .take(BRIEF_RECENT_DOCS)
                .collect();
            if !recent.is_empty() {
                lines.push(String::new());
                lines.push("## 최근 바뀐 문서".into());
                for d in &recent {
                    lines.push(format!("- {} · {}", d.name, ago(now_ms, d.modified_ms)));
                }
            }

            let mut daily_lines = Vec::new();
            for (label, date) in [("오늘", today), ("어제", yesterday)] {
                let domain = format!("{}/{}.md", crate::daily::DAILY_DIRNAME, date.format("%Y-%m-%d"));
                let md = vault::read_doc(vault, name, &domain).unwrap_or_default();
                if let Some(summary) = daily_one_liner(&md) {
                    daily_lines.push(format!("- {label}({}): {summary}", date.format("%m-%d")));
                }
            }
            if !daily_lines.is_empty() {
                lines.push(String::new());
                lines.push("## daily 노트".into());
                lines.extend(daily_lines);
            }

            let mut todos = Vec::new();
            for d in docs.iter().take(BRIEF_TODO_SCAN_DOCS) {
                let md = vault::read_doc(vault, name, &d.name).unwrap_or_default();
                for t in open_todos(&md) {
                    todos.push(format!("- [ ] {t} ({})", d.name));
                    if todos.len() >= BRIEF_MAX_TODOS {
                        break;
                    }
                }
                if todos.len() >= BRIEF_MAX_TODOS {
                    break;
                }
            }
            if !todos.is_empty() {
                lines.push(String::new());
                lines.push("## 열린 할 일".into());
                lines.extend(todos);
            }
            lines.extend(activity_lines(vault, Some(name)));
        }
        None => {
            lines.push("# 단비 브리핑 (전체)".into());
            let mut all: Vec<(&str, &DomainNode)> = tree
                .projects
                .iter()
                .flat_map(|p| flatten_domains(p).into_iter().map(move |d| (p.name.as_str(), d)))
                .filter(|(_, d)| !is_daily(&d.name))
                .collect();
            all.sort_by(|a, b| b.1.modified_ms.cmp(&a.1.modified_ms));
            if !all.is_empty() {
                lines.push(String::new());
                lines.push("## 최근 바뀐 문서".into());
                for (p, d) in all.iter().take(BRIEF_RECENT_DOCS) {
                    lines.push(format!("- {p}/{} · {}", d.name, ago(now_ms, d.modified_ms)));
                }
            }
            let domain = format!("{}/{}.md", crate::daily::DAILY_DIRNAME, today.format("%Y-%m-%d"));
            let mut daily_lines = Vec::new();
            for p in &tree.projects {
                let md = vault::read_doc(vault, &p.name, &domain).unwrap_or_default();
                if let Some(summary) = daily_one_liner(&md) {
                    daily_lines.push(format!("- {}: {summary}", p.name));
                }
            }
            if !daily_lines.is_empty() {
                lines.push(String::new());
                lines.push("## 오늘 daily 노트".into());
                lines.extend(daily_lines);
            }
            lines.extend(activity_lines(vault, None));
        }
    }

    let hint = "\n> 더 필요한 맥락은 `danbi_context` 에 질문 하나로 물어보세요. 문서 전체는 `danbi_read`.";
    let body = lines
        .iter()
        .map(|l| clip_chars(l, BRIEF_LINE_CHARS))
        .collect::<Vec<_>>()
        .join("\n");
    let budget = max_chars.saturating_sub(hint.chars().count() + 1);
    Ok(format!("{}{hint}\n", clip_lines(&body, budget)))
}

/// Last-7-day activity and healing counts from the same dieted source as
/// `danbi_briefing` (`(pre)` snapshots excluded, scaffolding files not
/// counted as orphans), so the hook's short brief carries signal, not
/// noise. Empty when there is nothing to say.
fn activity_lines(vault: &Path, project: Option<&str>) -> Vec<String> {
    let Ok(b) = crate::dashboard::briefing(vault, project) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if b.activity.commit_count > 0 {
        out.push(String::new());
        out.push(format!("## 최근 7일 활동 · 기록 {}건", b.activity.commit_count));
        for s in b.activity.recent_summaries.iter().take(3) {
            out.push(format!("- {}", s.trim_start_matches("danbi: ")));
        }
    }
    let h = &b.healing;
    if h.empty_projects + h.empty + h.oversized + h.orphans > 0 {
        let mut parts = Vec::new();
        if h.empty > 0 {
            parts.push(format!("빈 파일 {}", h.empty));
        }
        if h.oversized > 0 {
            parts.push(format!("너무 큰 문서 {}", h.oversized));
        }
        if h.orphans > 0 {
            parts.push(format!("링크 없는 문서 {}", h.orphans));
        }
        if h.empty_projects > 0 {
            parts.push(format!("빈 프로젝트 {}", h.empty_projects));
        }
        out.push(format!("정리 제안: {} (자세히: danbi_briefing)", parts.join(" · ")));
    }
    if b.ghost_total > 0 {
        out.push(format!("미확정 링크 제안 {}건 (danbi_briefing 의 ghost_suggestions)", b.ghost_total));
    }
    out
}

/// Every markdown file of a project, top-level and nested subfolders.
fn flatten_domains(p: &ProjectNode) -> Vec<&DomainNode> {
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

fn is_daily(domain: &str) -> bool {
    domain.starts_with(&format!("{}/", crate::daily::DAILY_DIRNAME))
}

/// One line for a daily note: its last three section headings, or the
/// first content lines when the note has no headings.
fn daily_one_liner(md: &str) -> Option<String> {
    let mut headings = Vec::new();
    let mut first_lines = Vec::new();
    let mut in_comment = false;
    for raw in md.lines() {
        let line = raw.trim();
        if in_comment {
            in_comment = !line.contains("-->");
            continue;
        }
        if line.starts_with("<!--") {
            in_comment = !line.contains("-->");
            continue;
        }
        if line.is_empty() || line.starts_with("# ") {
            continue;
        }
        if line.starts_with("##") {
            let h = line.trim_start_matches('#').trim();
            if !h.is_empty() {
                headings.push(h.to_string());
            }
        } else if first_lines.len() < 2 {
            first_lines.push(line.trim_start_matches("- ").to_string());
        }
    }
    let parts = if headings.is_empty() {
        first_lines
    } else {
        let skip = headings.len().saturating_sub(3);
        headings.split_off(skip)
    };
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

/// Unchecked markdown checkboxes (`- [ ] …`).
fn open_todos(md: &str) -> Vec<String> {
    md.lines()
        .filter_map(|l| {
            let t = l.trim_start();
            t.strip_prefix("- [ ] ")
                .or_else(|| t.strip_prefix("* [ ] "))
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn ago(now_ms: u128, modified_ms: Option<u128>) -> String {
    let Some(m) = modified_ms else {
        return "시간 모름".into();
    };
    let mins = now_ms.saturating_sub(m) / 60_000;
    match mins {
        0 => "방금".into(),
        1..=59 => format!("{mins}분 전"),
        60..=1439 => format!("{}시간 전", mins / 60),
        _ => format!("{}일 전", mins / 1440),
    }
}

fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Keeps whole lines while they fit in `max` characters (an ellipsis
/// line marks the cut).
fn clip_lines(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for line in s.lines() {
        let n = line.chars().count() + 1;
        if used + n + 1 > max {
            out.push('…');
            break;
        }
        out.push_str(line);
        out.push('\n');
        used += n;
    }
    out.trim_end_matches('\n').to_string()
}

// ---------- Session end ----------

/// Only the tail of a long transcript is read — enough to see recent
/// writes and to build the summary excerpt.
const TRANSCRIPT_TAIL_BYTES: u64 = 16 * 1024 * 1024;
const EXCERPT_CHARS: usize = 24_000;
const TURN_CHARS: usize = 1_500;
const SEEN_FILE: &str = ".danbi/session-end-seen.json";
const SEEN_CAP: usize = 500;
const MAX_BULLETS: usize = 5;

/// What the session-end logger needs to know about a transcript.
#[derive(Debug, Default)]
pub struct TranscriptDigest {
    /// The session already called a Danbi write tool (MCP or REST).
    pub wrote_danbi: bool,
    /// Prompts the user actually typed (tool results / meta excluded).
    pub user_turns: usize,
    pub tool_uses: usize,
    /// Characters of user + assistant prose.
    pub text_chars: usize,
    /// Latest turns, oldest first, within `EXCERPT_CHARS`.
    pub excerpt: String,
}

#[derive(Debug, PartialEq)]
pub enum SessionEndDecision {
    /// The agent already wrote to Danbi itself — don't double-log.
    AlreadyLogged,
    /// Too short to be worth a note.
    Trivial,
    Summarize,
}

pub fn decide(d: &TranscriptDigest) -> SessionEndDecision {
    if d.wrote_danbi {
        SessionEndDecision::AlreadyLogged
    } else if d.user_turns == 0 || (d.text_chars < 500 && d.tool_uses < 3) {
        SessionEndDecision::Trivial
    } else {
        SessionEndDecision::Summarize
    }
}

/// A tool call that writes into Danbi: an MCP write tool on any danbi
/// server (`mcp__danbi_x__danbi_log`), or a shell call hitting the local
/// REST write endpoints.
fn is_danbi_write_call(name: &str, input: &Value) -> bool {
    if let Some(rest) = name.strip_prefix("mcp__") {
        let mut parts = rest.rsplitn(2, "__");
        let tool = parts.next().unwrap_or("");
        let server = parts.next().unwrap_or("");
        return server.contains("danbi") && crate::mcp::is_write_tool(tool);
    }
    if name == "Bash" {
        let cmd = input.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let local = cmd.contains("127.0.0.1") || cmd.contains("localhost");
        let rest_write = cmd.contains("/v1/vault/")
            || (cmd.contains("/call/")
                && crate::mcp::WRITE_TOOLS.iter().any(|t| cmd.contains(t)));
        return local && rest_write;
    }
    false
}

/// Reads a Claude Code transcript (JSONL). Unknown or malformed lines
/// are skipped.
pub fn digest_transcript(jsonl: &str) -> TranscriptDigest {
    let mut d = TranscriptDigest::default();
    let mut turns: Vec<(&'static str, String)> = Vec::new();
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let sidechain = v.get("isSidechain").and_then(|b| b.as_bool()).unwrap_or(false);
        let content = v.get("message").and_then(|m| m.get("content"));
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                if sidechain || v.get("isMeta").and_then(|b| b.as_bool()).unwrap_or(false) {
                    continue;
                }
                let texts: Vec<&str> = match content {
                    Some(Value::String(s)) => vec![s.as_str()],
                    Some(Value::Array(items)) => items
                        .iter()
                        .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                        .collect(),
                    _ => Vec::new(),
                };
                // Slash-command echoes, local command output and injected
                // reminders all start with a tag — not typed prompts.
                let typed: Vec<&str> = texts
                    .into_iter()
                    .map(str::trim)
                    .filter(|t| !t.is_empty() && !t.starts_with('<'))
                    .collect();
                if !typed.is_empty() {
                    let text = typed.join("\n");
                    d.user_turns += 1;
                    d.text_chars += text.chars().count();
                    turns.push(("사용자", text));
                }
            }
            Some("assistant") => {
                let Some(Value::Array(items)) = content else {
                    continue;
                };
                for item in items {
                    match item.get("type").and_then(|t| t.as_str()) {
                        Some("tool_use") => {
                            d.tool_uses += 1;
                            let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let input = item.get("input").cloned().unwrap_or(Value::Null);
                            if is_danbi_write_call(name, &input) {
                                d.wrote_danbi = true;
                            }
                        }
                        Some("text") if !sidechain => {
                            let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
                            if !text.is_empty() {
                                d.text_chars += text.chars().count();
                                turns.push(("AI", text.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let mut picked: Vec<String> = Vec::new();
    let mut used = 0usize;
    for (role, text) in turns.iter().rev() {
        let block = format!("[{role}] {}", clip_chars(text, TURN_CHARS));
        let n = block.chars().count() + 2;
        if used + n > EXCERPT_CHARS {
            break;
        }
        used += n;
        picked.push(block);
    }
    picked.reverse();
    d.excerpt = picked.join("\n\n");
    d
}

/// Accepts only Claude Code transcript files: an existing `.jsonl` under
/// the user's home inside a `.claude*/projects/` tree. The endpoint is
/// token-protected, but it should still never read arbitrary files.
fn allowed_transcript_path(raw: &str) -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    let path = std::fs::canonicalize(raw).ok()?;
    let home = std::fs::canonicalize(home).ok()?;
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") || !path.is_file() {
        return None;
    }
    let rel = path.strip_prefix(&home).ok()?;
    let mut comps = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string());
    let first = comps.next()?;
    if !first.starts_with(".claude") || !comps.any(|c| c == "projects") {
        return None;
    }
    Some(path)
}

fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len > max {
        f.seek(SeekFrom::Start(len - max))?;
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    // A cut in the middle of a line just becomes one unparsable line.
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn load_seen(vault: &Path) -> Vec<String> {
    std::fs::read_to_string(vault.join(SEEN_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_seen(vault: &Path, session_id: &str) -> DanbiResult<()> {
    let mut seen = load_seen(vault);
    if !seen.iter().any(|s| s == session_id) {
        seen.push(session_id.to_string());
    }
    let skip = seen.len().saturating_sub(SEEN_CAP);
    let path = vault.join(SEEN_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string(&seen[skip..])?)?;
    Ok(())
}

/// Sessions currently being summarised — guards against the same
/// SessionEnd arriving twice before the first run has finished.
fn in_flight() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

/// Keeps only well-formed bullets from the model's answer.
fn clean_bullets(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- ") && l.len() > 2)
        .take(MAX_BULLETS)
        .map(str::to_string)
        .collect()
}

const SESSION_END_SYSTEM: &str = "Claude Code 코딩 세션 대화를 받아 단비 daily 노트에 남길 기록을 씁니다.\n\n규칙:\n- 한국어, bullet 최대 5개, 각 한 줄.\n- 각 bullet 은 '- 결정: ', '- 완료: ', '- 다음 할 일: ' 중 하나로 시작.\n- 대화에 실제로 나온 사실만. 추측·인사·감상 금지.\n- 코드명·파일명은 backtick.\n- 남길 만한 내용이 없으면 '없음' 한 단어만.\n- bullet 외 다른 말은 쓰지 않습니다.";

/// Background half of `POST /api/session-end`. Never surfaces errors to
/// the hook (it has already returned); failures only go to the log.
pub async fn run_session_end(project_hint: String, payload: Value) {
    if let Err(e) = session_end_inner(&project_hint, &payload).await {
        eprintln!("[session-end] {e}");
    }
}

async fn session_end_inner(project_hint: &str, payload: &Value) -> DanbiResult<()> {
    let vault = crate::mcp::current_vault_path()?;
    let project = vault::project_by_id(&vault, project_hint)?
        .unwrap_or_else(|| project_hint.to_string());
    if !vault::list_tree(&vault)?.projects.iter().any(|p| p.name == project) {
        return Ok(());
    }
    let session_id = payload
        .get("session_id")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if session_id.is_empty() || load_seen(&vault).contains(&session_id) {
        return Ok(());
    }
    let Some(path) = payload
        .get("transcript_path")
        .and_then(|p| p.as_str())
        .and_then(allowed_transcript_path)
    else {
        return Ok(());
    };
    let digest = digest_transcript(&read_tail(&path, TRANSCRIPT_TAIL_BYTES)?);
    if decide(&digest) != SessionEndDecision::Summarize {
        return Ok(());
    }

    // No LLM configured → nothing to do, silently.
    let Some(cfg) = crate::config::load_config(&vault)? else {
        return Ok(());
    };
    let Some((model, _)) = crate::commands::resolve_summarize_model(&cfg) else {
        return Ok(());
    };
    let Ok(provider) = crate::commands::resolve_embed_provider(&cfg) else {
        return Ok(());
    };

    if !in_flight().lock().map(|mut s| s.insert(session_id.clone())).unwrap_or(false) {
        return Ok(());
    }
    let user = format!(
        "프로젝트: {project}\n다음 세션 대화를 규칙대로 정리해줘.\n\n---\n{}\n---",
        digest.excerpt
    );
    let answer = crate::usage::with_role(
        "session_end",
        provider.converse_text(&model, Some(SESSION_END_SYSTEM), &user, 600, 0.2),
    )
    .await;
    let result = answer.and_then(|raw| {
        let bullets = clean_bullets(&raw);
        // Recorded before the write so the same commit carries it.
        save_seen(&vault, &session_id)?;
        if bullets.is_empty() {
            return Ok(());
        }
        let content = format!(
            "### 세션 자동 기록 · {}\n{}\n<!-- danbi:session-end {session_id} -->",
            chrono::Local::now().format("%H:%M"),
            bullets.join("\n"),
        );
        // 본문에 이미 '### 세션 자동 기록 · HH:MM' 머리말이 있어 danbi_log 의
        // 자동 시각·호출자 줄('_HH:MM · danbi_')은 끈다 — 두 줄 중복 방지.
        crate::mcp::dispatch(
            "danbi_log",
            json!({ "project": project, "content": content, "_client": crate::vcs::CLIENT_DANBI, "stamp": false }),
        )
            .map(|_| ())
    });
    if let Ok(mut s) = in_flight().lock() {
        s.remove(&session_id);
    }
    result
}

// ---------- Hook installer ----------

/// The helper script. It holds no secret: every run reads the port and
/// token from Danbi's config, and any failure (app closed, config gone,
/// timeout) ends silently with exit 0 so a Claude Code session is never
/// blocked or broken by Danbi.
pub fn helper_script(config_path: &Path) -> String {
    let cfg = config_path.to_string_lossy().replace('\'', "'\\''");
    format!(
        r#"#!/bin/bash
# danbi-hook — 단비 앱이 만든 Claude Code 훅 도우미. 앱이 다시 쓰므로 직접 고치지 마세요.
# 토큰은 이 파일에 없어요. 실행할 때마다 단비 설정에서 포트·토큰을 읽고,
# 앱이 꺼져 있는 등 무엇이든 실패하면 조용히 끝나 세션을 막지 않아요.
CONFIG='{cfg}'
kind="$1"
project="$2"
case "$project" in *[!A-Za-z0-9_-]*) exit 0 ;; esac
[ -r "$CONFIG" ] || exit 0
port=$(/usr/bin/plutil -extract mcp.port raw -o - "$CONFIG" 2>/dev/null) || exit 0
token=$(/usr/bin/plutil -extract mcp.token raw -o - "$CONFIG" 2>/dev/null) || exit 0
case "$port" in ''|*[!0-9]*) exit 0 ;; esac
[ -n "$token" ] || exit 0
base="http://127.0.0.1:$port"
auth() {{ printf 'Authorization: Bearer %s\n' "$token"; }}
case "$kind" in
  session-start)
    /usr/bin/curl -sf --max-time 2 -H @<(auth) "$base/api/session-brief?project=$project" 2>/dev/null
    ;;
  session-end)
    /usr/bin/curl -sf --max-time 2 -o /dev/null -H @<(auth) -H 'Content-Type: application/json' --data-binary @- "$base/api/session-end?project=$project" 2>/dev/null
    ;;
esac
exit 0
"#
    )
}

/// Shell command Claude Code runs for one hook event.
pub fn hook_command(helper: &Path, kind: &str, project_id: &str) -> String {
    let path = helper.to_string_lossy().replace('"', "\\\"");
    format!("\"{path}\" {kind} {project_id}")
}

fn is_danbi_hook(entry: &Value) -> bool {
    entry
        .get("command")
        .and_then(|c| c.as_str())
        .map(|c| c.contains(HOOK_HELPER_NAME))
        .unwrap_or(false)
}

/// Merges Danbi's hooks into a `.claude/settings.local.json` body.
///
/// Every existing hook entry whose command runs `danbi-hook` is dropped
/// first (empty matcher groups and events go with it), then the wanted
/// events are re-added — so re-installing replaces instead of stacking,
/// and passing `None` for an event removes Danbi's hook for it. Other
/// keys and hooks are kept. Returns `(body, replaced)`; `replaced` is
/// true when a Danbi hook was already there.
pub fn merge_hook_settings(
    existing: &str,
    session_start: Option<&str>,
    session_end: Option<&str>,
) -> DanbiResult<(String, bool)> {
    let mut root: Value = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(existing).map_err(|e| {
            DanbiError::Other(format!(
                "settings.local.json 을 읽을 수 없어요 (JSON 오류: {e}). 파일을 고친 뒤 다시 설치하세요."
            ))
        })?
    };
    let Some(obj) = root.as_object_mut() else {
        return Err(DanbiError::Other(
            "settings.local.json 최상위가 객체가 아니에요.".into(),
        ));
    };
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else {
        return Err(DanbiError::Other(
            "settings.local.json 의 hooks 가 객체가 아니에요.".into(),
        ));
    };

    let mut replaced = false;
    for (event, command) in [("SessionStart", session_start), ("SessionEnd", session_end)] {
        let mut groups: Vec<Value> = hooks
            .get(event)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        groups.retain_mut(|group| {
            let Some(list) = group.get_mut("hooks").and_then(|h| h.as_array_mut()) else {
                return true;
            };
            let before = list.len();
            list.retain(|h| !is_danbi_hook(h));
            if list.len() == before {
                return true;
            }
            replaced = true;
            !list.is_empty()
        });
        if let Some(cmd) = command {
            groups.push(json!({
                "hooks": [{ "type": "command", "command": cmd, "timeout": HOOK_TIMEOUT_SECS }]
            }));
        }
        if groups.is_empty() {
            hooks.remove(event);
        } else {
            hooks.insert(event.to_string(), Value::Array(groups));
        }
    }
    if hooks.is_empty() {
        obj.remove("hooks");
    }
    let mut body = serde_json::to_string_pretty(&root)?;
    body.push('\n');
    Ok((body, replaced))
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "\"/Users/x/.danbi/bin/danbi-hook\" session-start abc";

    #[test]
    fn merge_into_empty_adds_session_start() {
        let (body, replaced) = merge_hook_settings("", Some(START), None).unwrap();
        assert!(!replaced);
        let v: Value = serde_json::from_str(&body).unwrap();
        let groups = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["hooks"][0]["command"], START);
        assert!(v["hooks"].get("SessionEnd").is_none());
    }

    #[test]
    fn merge_is_idempotent_and_keeps_user_hooks() {
        let existing = r#"{
  "permissions": { "allow": ["Bash(ls:*)"] },
  "hooks": {
    "SessionStart": [
      { "matcher": "startup", "hooks": [{ "type": "command", "command": "echo hi" }] }
    ],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "guard.sh" }] }
    ]
  }
}"#;
        let (once, replaced1) = merge_hook_settings(existing, Some(START), None).unwrap();
        assert!(!replaced1);
        let (twice, replaced2) = merge_hook_settings(&once, Some(START), None).unwrap();
        assert!(replaced2);
        assert_eq!(once, twice, "re-install must not stack entries");

        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls:*)");
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "guard.sh");
        let start = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 2);
        assert_eq!(start[0]["hooks"][0]["command"], "echo hi");
        assert_eq!(start[1]["hooks"][0]["command"], START);
    }

    #[test]
    fn merge_with_none_removes_only_danbi_entries() {
        let (installed, _) = merge_hook_settings("{}", Some(START), Some(START)).unwrap();
        let (removed, replaced) = merge_hook_settings(&installed, None, None).unwrap();
        assert!(replaced);
        let v: Value = serde_json::from_str(&removed).unwrap();
        assert!(v.get("hooks").is_none());
    }

    #[test]
    fn merge_refuses_invalid_json() {
        assert!(merge_hook_settings("{ not json", Some(START), None).is_err());
        assert!(merge_hook_settings("[1,2]", Some(START), None).is_err());
    }

    #[test]
    fn helper_script_has_no_token_and_always_exits_zero() {
        let s = helper_script(Path::new("/tmp/v'ault/config.json"));
        assert!(s.contains("CONFIG='/tmp/v'\\''ault/config.json'"));
        assert!(s.trim_end().ends_with("exit 0"));
        assert!(s.contains("--max-time 2"));
    }

    #[test]
    fn daily_one_liner_prefers_last_headings() {
        let md = "# 2026-10-15\n\n<!-- daily note -->\n\n### a\nx\n### b\n### c\n### d\n";
        assert_eq!(daily_one_liner(md).unwrap(), "b · c · d");
        assert_eq!(daily_one_liner("# 2026-10-15\n\n<!-- x -->\n"), None);
        assert_eq!(daily_one_liner("# t\n- 첫 줄\n- 둘째\n- 셋째").unwrap(), "첫 줄 · 둘째");
    }

    #[test]
    fn open_todos_reads_unchecked_boxes_only() {
        let md = "- [ ] 하나\n- [x] 끝남\n  * [ ] 둘\n";
        assert_eq!(open_todos(md), vec!["하나", "둘"]);
    }

    fn temp_vault(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "danbi-session-{tag}-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn session_brief_lists_docs_daily_and_todos_within_budget() {
        let vault = temp_vault("brief");
        let proj = vault.join(vault::PROJECTS_DIRNAME).join("알파");
        std::fs::create_dir_all(proj.join("notes")).unwrap();
        std::fs::create_dir_all(proj.join("daily")).unwrap();
        std::fs::write(proj.join("plan.md"), "# 계획\n- [ ] 배포 체크\n- [x] 끝\n").unwrap();
        std::fs::write(proj.join("notes/auth.md"), "# 인증\n").unwrap();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        std::fs::write(
            proj.join(format!("daily/{today}.md")),
            format!("# {today}\n\n### 로그인 버그 원인\n본문\n"),
        )
        .unwrap();

        let brief = session_brief(&vault, Some("알파"), DEFAULT_BRIEF_CHARS).unwrap();
        assert!(brief.starts_with("# 단비 브리핑 · 알파"));
        assert!(brief.contains("- plan.md · "));
        assert!(brief.contains("- notes/auth.md · "));
        assert!(!brief.contains(&format!("daily/{today}.md ·")), "daily notes are summarised, not listed");
        assert!(brief.contains("로그인 버그 원인"));
        assert!(brief.contains("- [ ] 배포 체크 (plan.md)"));
        assert!(brief.contains("danbi_context"));

        let wide = session_brief(&vault, None, DEFAULT_BRIEF_CHARS).unwrap();
        assert!(wide.contains("- 알파/plan.md · "));
        assert!(wide.contains("- 알파: 로그인 버그 원인"));

        let tiny = session_brief(&vault, Some("알파"), 500).unwrap();
        assert!(tiny.chars().count() <= 500);
        assert!(tiny.contains("danbi_context"), "hint survives truncation");

        assert!(session_brief(&vault, Some("없는프로젝트"), 2500).is_err());
        let _ = std::fs::remove_dir_all(&vault);
    }

    fn user(text: &str) -> String {
        json!({ "type": "user", "message": { "role": "user", "content": text } }).to_string()
    }
    fn assistant(items: Value) -> String {
        json!({ "type": "assistant", "message": { "role": "assistant", "content": items } })
            .to_string()
    }
    fn long(n: usize) -> String {
        "가".repeat(n)
    }

    #[test]
    fn session_end_skips_when_danbi_write_tool_was_called() {
        let t = [
            user("로그인 버그 고쳐줘"),
            assistant(json!([{ "type": "text", "text": long(800) }])),
            assistant(json!([{ "type": "tool_use", "name": "mcp__danbi_bonny__danbi_log", "input": {} }])),
        ]
        .join("\n");
        let d = digest_transcript(&t);
        assert!(d.wrote_danbi);
        assert_eq!(decide(&d), SessionEndDecision::AlreadyLogged);

        // REST write through the shell counts too; a read does not.
        let rest = assistant(json!([{ "type": "tool_use", "name": "Bash",
            "input": { "command": "curl -X POST http://127.0.0.1:47921/api/call/danbi_append -d @x" } }]));
        assert!(digest_transcript(&rest).wrote_danbi);
        let read = assistant(json!([{ "type": "tool_use", "name": "mcp__danbi__danbi_search", "input": {} }]));
        assert!(!digest_transcript(&read).wrote_danbi);
        let other = assistant(json!([{ "type": "tool_use", "name": "mcp__notes__danbi_log", "input": {} }]));
        assert!(!digest_transcript(&other).wrote_danbi, "only danbi servers count");
    }

    #[test]
    fn session_end_skips_trivial_sessions() {
        let t = [user("안녕"), assistant(json!([{ "type": "text", "text": "안녕하세요" }]))].join("\n");
        assert_eq!(decide(&digest_transcript(&t)), SessionEndDecision::Trivial);

        // Only meta / command echoes / tool results → no typed prompt at all.
        let meta = [
            json!({ "type": "user", "isMeta": true, "message": { "content": long(900) } }).to_string(),
            user("<command-name>/clear</command-name>"),
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "content": long(900) }] } }).to_string(),
            "not json".to_string(),
        ]
        .join("\n");
        let d = digest_transcript(&meta);
        assert_eq!(d.user_turns, 0);
        assert_eq!(decide(&d), SessionEndDecision::Trivial);
    }

    #[test]
    fn session_end_summarizes_real_sessions_with_recent_excerpt() {
        let mut lines = vec![user("결제 모듈 리팩터링 하자")];
        for i in 0..40 {
            lines.push(assistant(json!([
                { "type": "text", "text": format!("단계 {i} {}", long(900)) },
                { "type": "tool_use", "name": "Edit", "input": {} }
            ])));
        }
        lines.push(user("좋아 마무리하자"));
        let d = digest_transcript(&lines.join("\n"));
        assert_eq!(decide(&d), SessionEndDecision::Summarize);
        assert_eq!(d.user_turns, 2);
        assert_eq!(d.tool_uses, 40);
        assert!(d.excerpt.chars().count() <= EXCERPT_CHARS);
        assert!(d.excerpt.ends_with("[사용자] 좋아 마무리하자"), "latest turns are kept");
        assert!(!d.excerpt.contains("단계 0 "), "oldest turns drop out first");
    }

    #[test]
    fn clean_bullets_keeps_at_most_five_bullets() {
        let raw = "요약입니다\n- 결정: a\n- 완료: b\n-\n- 완료: c\n- 다음 할 일: d\n- 완료: e\n- 완료: f";
        assert_eq!(clean_bullets(raw).len(), 5);
        assert!(clean_bullets("없음").is_empty());
    }

    #[test]
    fn transcript_path_must_be_claude_jsonl_under_home() {
        assert!(allowed_transcript_path("/etc/passwd").is_none());
        let tmp = temp_vault("transcript").join("x.jsonl");
        std::fs::write(&tmp, "{}").unwrap();
        assert!(allowed_transcript_path(tmp.to_str().unwrap()).is_none(), "outside ~/.claude*/projects");
        let _ = std::fs::remove_dir_all(tmp.parent().unwrap());
    }

    #[test]
    fn merge_adds_and_drops_session_end() {
        let end = "\"/Users/x/.danbi/bin/danbi-hook\" session-end abc";
        let (with_end, _) = merge_hook_settings("", Some(START), Some(end)).unwrap();
        let v: Value = serde_json::from_str(&with_end).unwrap();
        assert_eq!(v["hooks"]["SessionEnd"][0]["hooks"][0]["command"], end);
        let (without, replaced) = merge_hook_settings(&with_end, Some(START), None).unwrap();
        assert!(replaced);
        let v: Value = serde_json::from_str(&without).unwrap();
        assert!(v["hooks"].get("SessionEnd").is_none());
        assert_eq!(v["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn clip_lines_respects_budget() {
        let s = "aaaa\nbbbb\ncccc";
        assert_eq!(clip_lines(s, 11), "aaaa\nbbbb\n…");
        assert_eq!(clip_lines(s, 10), "aaaa\n…");
        assert_eq!(clip_lines(s, 100), s);
    }
}
