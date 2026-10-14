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
    let tree = vault::list_tree(vault)?;
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

    #[test]
    fn clip_lines_respects_budget() {
        let s = "aaaa\nbbbb\ncccc";
        assert_eq!(clip_lines(s, 11), "aaaa\nbbbb\n…");
        assert_eq!(clip_lines(s, 10), "aaaa\n…");
        assert_eq!(clip_lines(s, 100), s);
    }
}
