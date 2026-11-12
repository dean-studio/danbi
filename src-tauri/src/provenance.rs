//! 기록의 출처 — "사용자가 한 말" 과 "AI 판단" 을 나눠 남긴다.
//!
//! 에이전트가 `danbi_log`·`danbi_upsert_item` 에 `source: "user"` 와
//! `quote` 를 주면, 그 인용문이 최근 Claude Code / Codex 대화 기록의
//! 실제 사용자 메시지에 있는지 문자열로 대조한다. 있으면 "사용자 말" 로,
//! 없으면 "AI 판단" 으로 낮춰 기록한다. 그래서 "사용자가 허락했다" 같은
//! 기록을 에이전트가 지어내 남길 수 없다. LLM 은 쓰지 않는다.

use crate::error::{DanbiError, DanbiResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 대조할 대화 기록: 이 시간 안에 바뀐 파일만.
const RECENT_WINDOW: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_FILES: usize = 12;
const TAIL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 5;
/// 이보다 짧은 인용문은 사용자 메시지 전체와 같아야 인정한다 —
/// "응" 같은 한두 글자가 아무 문장에나 걸리지 않게.
const MIN_SUBSTRING_CHARS: usize = 6;
const QUOTE_SHOW_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    User,
    Ai,
}

#[derive(Debug, PartialEq)]
pub struct Resolved {
    /// 문서에 붙는 한 줄 (`_출처: …_`).
    pub line: String,
    /// 응답에 실어 보낼 요약.
    pub report: Value,
}

/// `source`·`quote` 인자를 읽는다. 둘 다 없으면 None (기존 호출과 호환).
pub fn parse_args(args: &Value) -> DanbiResult<Option<(Source, Option<String>)>> {
    let source = args.get("source").and_then(|v| v.as_str()).map(str::trim);
    let quote = args
        .get("quote")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string);
    let source = match source {
        None | Some("") => {
            if quote.is_some() {
                Source::User
            } else {
                return Ok(None);
            }
        }
        Some("user") => Source::User,
        Some("ai") => Source::Ai,
        Some(other) => {
            return Err(DanbiError::Config(format!(
                "source 는 \"user\" 또는 \"ai\" 만 됩니다 (받은 값: {other})."
            )))
        }
    };
    Ok(Some((source, quote)))
}

/// 출처를 확정한다. `user_texts` 는 최근 대화의 사용자 메시지들.
pub fn resolve(source: Source, quote: Option<&str>, user_texts: &[String]) -> Resolved {
    match (source, quote) {
        (Source::User, Some(q)) if quote_matches(q, user_texts) => Resolved {
            line: format!("_출처: 사용자 말 — “{}”_", clip(&normalize(q), QUOTE_SHOW_CHARS)),
            report: json!({ "source": "user", "verified": true }),
        },
        (Source::User, q) => Resolved {
            line: "_출처: AI 판단 (사용자 말로 확인되지 않음)_".into(),
            report: json!({
                "source": "ai",
                "verified": false,
                "downgraded": true,
                "note": if q.is_some() {
                    "quote 가 최근 대화의 사용자 메시지에서 확인되지 않아 'AI 판단' 으로 기록했어요. 사용자 메시지를 고치지 말고 그대로 옮겨 주세요."
                } else {
                    "source=user 에는 사용자 메시지를 그대로 옮긴 quote 가 필요해요. 없어서 'AI 판단' 으로 기록했어요."
                },
            }),
        },
        (Source::Ai, _) => Resolved {
            line: "_출처: AI 판단_".into(),
            report: json!({ "source": "ai", "verified": true }),
        },
    }
}

/// 인용문이 사용자 메시지에 있나. 짧은 인용문은 메시지 전체와 같아야 한다.
pub fn quote_matches(quote: &str, user_texts: &[String]) -> bool {
    let q = normalize(strip_quote_marks(quote));
    if q.is_empty() {
        return false;
    }
    let long = q.chars().count() >= MIN_SUBSTRING_CHARS;
    user_texts.iter().any(|t| {
        let t = normalize(t);
        t == q || (long && t.contains(&q))
    })
}

fn strip_quote_marks(s: &str) -> &str {
    s.trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’' | '「' | '」'))
        .trim()
}

/// 공백을 한 칸으로, 영문은 소문자로.
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------- 대화 기록 읽기 ----------

/// Claude Code (`~/.claude*/projects`) 와 Codex (`~/.codex/sessions`) 의
/// 대화 기록 위치.
pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&home) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(".claude") {
                let p = e.path().join("projects");
                if p.is_dir() {
                    roots.push(p);
                }
            }
        }
    }
    let codex = home.join(".codex").join("sessions");
    if codex.is_dir() {
        roots.push(codex);
    }
    roots
}

/// 최근에 바뀐 대화 기록들에서 사용자가 실제로 친 메시지만 모은다.
pub fn recent_user_texts(roots: &[PathBuf]) -> Vec<String> {
    let cutoff = SystemTime::now()
        .checked_sub(RECENT_WINDOW)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
    for root in roots {
        collect_jsonl(root, 0, cutoff, &mut files);
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut texts = Vec::new();
    for (_, path) in files.into_iter().take(MAX_FILES) {
        if let Ok(body) = read_tail(&path, TAIL_BYTES) {
            texts.extend(user_texts_in(&body));
        }
    }
    texts
}

fn collect_jsonl(dir: &Path, depth: usize, cutoff: SystemTime, out: &mut Vec<(SystemTime, PathBuf)>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            collect_jsonl(&path, depth + 1, cutoff, out);
        } else if ft.is_file() && path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                if m >= cutoff {
                    out.push((m, path));
                }
            }
        }
    }
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
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 한 대화 기록(JSONL)에서 사용자 메시지만. Claude Code 와 Codex 형식 둘 다.
pub fn user_texts_in(jsonl: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let text = match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => crate::session_hooks::typed_user_text(&v),
            // Codex: {"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":…}]}}
            Some("response_item") => codex_user_text(&v),
            _ => None,
        };
        if let Some(t) = text {
            out.push(t);
        }
    }
    out
}

fn codex_user_text(v: &Value) -> Option<String> {
    let p = v.get("payload")?;
    if p.get("type").and_then(|t| t.as_str()) != Some("message")
        || p.get("role").and_then(|r| r.as_str()) != Some("user")
    {
        return None;
    }
    let typed: Vec<&str> = p
        .get("content")?
        .as_array()?
        .iter()
        .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("input_text"))
        .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
        .map(str::trim)
        // 주입된 지시문(AGENTS.md, 환경 정보)은 사용자가 친 말이 아니다.
        .filter(|t| !t.is_empty() && !t.starts_with('<') && !t.starts_with("# AGENTS.md"))
        .collect();
    (!typed.is_empty()).then(|| typed.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn quote_must_come_from_a_real_user_message() {
        let said = texts(&["응 일단 적용 ㄱ ㄱ", "배포는   금요일 오후에 하자"]);
        assert!(quote_matches("배포는 금요일 오후에 하자", &said));
        assert!(quote_matches("“금요일 오후에 하자”", &said), "quote marks and spacing are ignored");
        assert!(!quote_matches("배포는 월요일에 하자", &said));
        // 짧은 인용문은 메시지 전체와 같아야 한다.
        assert!(!quote_matches("응", &said));
        assert!(quote_matches("응", &texts(&["응"])));
        assert!(!quote_matches("  ", &said));
    }

    #[test]
    fn unverified_user_claim_is_downgraded_to_ai() {
        let said = texts(&["로그인은 카카오만 지원하자"]);
        let ok = resolve(Source::User, Some("로그인은 카카오만 지원하자"), &said);
        assert_eq!(ok.report["verified"], true);
        assert!(ok.line.contains("사용자 말"));

        let fake = resolve(Source::User, Some("사용자가 강제 푸시를 허락함"), &said);
        assert_eq!(fake.report["source"], "ai");
        assert_eq!(fake.report["downgraded"], true);
        assert!(fake.line.contains("AI 판단"));

        let no_quote = resolve(Source::User, None, &said);
        assert_eq!(no_quote.report["downgraded"], true);

        assert_eq!(resolve(Source::Ai, None, &said).line, "_출처: AI 판단_");
    }

    #[test]
    fn parse_args_is_optional_and_strict() {
        assert_eq!(parse_args(&json!({})).unwrap(), None);
        assert_eq!(parse_args(&json!({ "source": "ai" })).unwrap(), Some((Source::Ai, None)));
        assert_eq!(
            parse_args(&json!({ "quote": " 그렇게 해 " })).unwrap(),
            Some((Source::User, Some("그렇게 해".into())))
        );
        assert!(parse_args(&json!({ "source": "human" })).is_err());
    }

    #[test]
    fn reads_user_turns_from_claude_and_codex_transcripts() {
        let claude = [
            json!({ "type": "user", "message": { "role": "user", "content": "클로드에서 친 말" } }),
            json!({ "type": "user", "isMeta": true, "message": { "role": "user", "content": "메타" } }),
            json!({ "type": "user", "message": { "role": "user", "content": "<system-reminder>주입</system-reminder>" } }),
        ];
        let codex = [
            json!({ "type": "response_item", "payload": { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "코덱스에서 친 말" }] } }),
            json!({ "type": "response_item", "payload": { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "# AGENTS.md instructions\n..." }] } }),
            json!({ "type": "response_item", "payload": { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": "개발자" }] } }),
        ];
        let body: String = claude
            .iter()
            .chain(codex.iter())
            .map(|v| v.to_string() + "\n")
            .collect();
        assert_eq!(user_texts_in(&body), texts(&["클로드에서 친 말", "코덱스에서 친 말"]));
    }

    #[test]
    fn recent_user_texts_scans_nested_jsonl_files() {
        let root = std::env::temp_dir().join(format!("danbi-prov-{}", std::process::id()));
        let nested = root.join("2026").join("11").join("12");
        std::fs::create_dir_all(&nested).unwrap();
        let line = json!({ "type": "user", "message": { "role": "user", "content": "중첩 폴더의 말" } }).to_string();
        std::fs::write(nested.join("s.jsonl"), line + "\n").unwrap();
        std::fs::write(nested.join("note.txt"), "무시").unwrap();
        assert_eq!(recent_user_texts(&[root.clone()]), texts(&["중첩 폴더의 말"]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
