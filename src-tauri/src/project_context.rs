//! Project-level context files: `purpose.md` and `schema.md`.
//!
//! These are the two files from Andrej Karpathy's Wiki-LLM design pattern
//! that turn a vault folder into a project with *intent*. The LLM reads
//! them before every edit so it can:
//!
//!   - `purpose.md` — know *what this project is for* and what's out of
//!     scope, so it doesn't produce output that drifts from the goal.
//!   - `schema.md` — know the project's file naming rules, section
//!     structure, link policy, and style conventions, so every edit
//!     stays consistent.
//!
//! Both files are plain markdown in the project root and are optional
//! (projects created before J-1 simply skip injection). When present,
//! their contents are clipped to a small byte budget and handed to the
//! Writer as a `<project_context>` block in the system prompt.

use crate::error::DanbiResult;
use crate::vault::PROJECTS_DIRNAME;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const PURPOSE_FILENAME: &str = "purpose.md";
pub const SCHEMA_FILENAME: &str = "schema.md";
/// 사용자 정보·지적받은 것·약속 — 세션 시작 브리핑에 매번 그대로 들어간다.
pub const PROMISES_FILENAME: &str = "약속.md";
const MAX_BYTES_PER_FILE: usize = 2048;

/// Snapshot of whatever purpose/schema the project has right now. Empty
/// fields mean "file doesn't exist" — callers should skip injection
/// rather than emit empty XML blocks.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProjectContext {
    pub has_purpose: bool,
    pub has_schema: bool,
    pub purpose: Option<String>,
    pub schema: Option<String>,
    /// Whether either file was clipped due to size. UI can show a
    /// "too long, consider summarizing" hint.
    pub purpose_clipped: bool,
    pub schema_clipped: bool,
    /// Compact ontology vocabulary (types / relations / statuses) for the
    /// Writer. Built from defaults + schema.md's `## 온톨로지` block, so it
    /// survives the 2KB schema clip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ontology: Option<String>,
}

fn project_root(vault: &Path, project: &str) -> PathBuf {
    vault.join(PROJECTS_DIRNAME).join(project)
}

pub fn purpose_path(vault: &Path, project: &str) -> PathBuf {
    project_root(vault, project).join(PURPOSE_FILENAME)
}

pub fn schema_path(vault: &Path, project: &str) -> PathBuf {
    project_root(vault, project).join(SCHEMA_FILENAME)
}

/// `약속.md` 본문을 브리핑용으로: frontmatter·H1·HTML 주석·빈 줄을 빼고
/// `max_chars` 안에서 줄 단위로 자른다. 파일이 없거나 비면 None.
pub fn promises_for_brief(vault: &Path, project: &str, max_chars: usize) -> Option<String> {
    let raw = std::fs::read_to_string(project_root(vault, project).join(PROMISES_FILENAME)).ok()?;
    let mut lines: Vec<&str> = Vec::new();
    let mut in_fm = false;
    let mut in_comment = false;
    for (i, line) in raw.lines().enumerate() {
        let t = line.trim();
        if i == 0 && t == "---" {
            in_fm = true;
            continue;
        }
        if in_fm {
            if t == "---" {
                in_fm = false;
            }
            continue;
        }
        if in_comment {
            if t.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        if t.starts_with("<!--") {
            in_comment = !t.contains("-->");
            continue;
        }
        if t.is_empty() || (t.starts_with("# ") && !t.starts_with("## ")) {
            continue;
        }
        lines.push(line.trim_end());
    }
    // 항목이 없는 섹션 제목은 뺀다 — 골격만 있는 동안 브리핑은 조용하다.
    let is_heading = |l: &str| l.trim_start().starts_with('#');
    let lines: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            !is_heading(l) || lines.get(i + 1).is_some_and(|next| !is_heading(next))
        })
        .map(|(_, l)| *l)
        .collect();
    if lines.iter().all(|l| is_heading(l)) {
        return None;
    }
    let mut out = String::new();
    let mut used = 0usize;
    for line in lines {
        let n = line.chars().count() + 1;
        if used + n > max_chars {
            out.push_str("… (나머지는 약속.md)");
            break;
        }
        out.push_str(line);
        out.push('\n');
        used += n;
    }
    let out = out.trim_end().to_string();
    (!out.is_empty()).then_some(out)
}

fn read_clipped(path: &Path) -> Option<(String, bool)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() <= MAX_BYTES_PER_FILE {
        Some((trimmed.to_string(), false))
    } else {
        // Clip on a char boundary so we never split a multi-byte UTF-8.
        let mut end = MAX_BYTES_PER_FILE;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        Some((trimmed[..end].to_string(), true))
    }
}

/// Reads purpose + schema for the given project. Missing files return
/// fields cleared — never errors on missing, since both files are
/// optional by design.
pub fn load(vault: &Path, project: &str) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    if let Some((body, clipped)) = read_clipped(&purpose_path(vault, project)) {
        ctx.has_purpose = true;
        ctx.purpose_clipped = clipped;
        ctx.purpose = Some(body);
    }
    if let Some((body, clipped)) = read_clipped(&schema_path(vault, project)) {
        ctx.has_schema = true;
        ctx.schema_clipped = clipped;
        ctx.schema = Some(body);
    }
    ctx.ontology = Some(ontology_block(&crate::ontology::vocabulary_for(vault, project)));
    ctx
}

fn ontology_block(vocab: &crate::ontology::Vocabulary) -> String {
    let named = |name: &str, ko: Option<&str>| match ko {
        Some(k) => format!("{name}({k})"),
        None => name.to_string(),
    };
    let types: Vec<String> = vocab.types.iter().map(|t| named(&t.name, t.ko.as_deref())).collect();
    let statuses: Vec<String> = vocab.statuses.iter().map(|t| named(&t.name, t.ko.as_deref())).collect();
    // One line per forward/inverse pair, forward first.
    let mut seen = std::collections::HashSet::new();
    let mut pairs: Vec<String> = Vec::new();
    for r in &vocab.relations {
        if !seen.insert(r.name.clone()) {
            continue;
        }
        let fwd = named(&r.name, r.ko.as_deref());
        if r.inverse == r.name {
            pairs.push(fwd);
            continue;
        }
        seen.insert(r.inverse.clone());
        let inv_ko = vocab.relations.iter().find(|x| x.name == r.inverse).and_then(|x| x.ko.clone());
        pairs.push(format!("{fwd} ↔ {}", named(&r.inverse, inv_ko.as_deref())));
    }
    format!(
        "Document types (frontmatter `type:`): {}\nRelations (written on the link's line, e.g. `원인:: [[notes/x.md]]`): {}\nStatuses (frontmatter `status:`): {}",
        types.join(", "),
        pairs.join(", "),
        statuses.join(", ")
    )
}

/// Produces a `<project_context>` XML-ish block to append to a Writer
/// system prompt. Returns `None` when neither file exists, so callers
/// can skip string concat entirely.
///
/// `editing_file` is the target domain path — when it equals
/// `purpose.md` or `schema.md` we suppress self-injection so the model
/// doesn't echo its own grounding back into a rewrite of the grounding.
pub fn as_prompt_block(
    ctx: &ProjectContext,
    editing_file: Option<&str>,
) -> Option<String> {
    let editing = editing_file.unwrap_or("");
    let skip_purpose = editing == PURPOSE_FILENAME
        || editing.ends_with(&format!("/{PURPOSE_FILENAME}"));
    let skip_schema = editing == SCHEMA_FILENAME
        || editing.ends_with(&format!("/{SCHEMA_FILENAME}"));

    let purpose = if skip_purpose { None } else { ctx.purpose.as_deref() };
    let schema = if skip_schema { None } else { ctx.schema.as_deref() };

    if purpose.is_none() && schema.is_none() && ctx.ontology.is_none() {
        return None;
    }

    let mut out = String::from("\n<project_context>\n");
    if let Some(p) = purpose {
        out.push_str("<purpose>\n");
        out.push_str(p);
        out.push_str("\n</purpose>\n");
    }
    if let Some(s) = schema {
        out.push_str("<schema>\n");
        out.push_str(s);
        out.push_str("\n</schema>\n");
    }
    if let Some(o) = ctx.ontology.as_deref() {
        out.push_str("<ontology>\n");
        out.push_str(o);
        out.push_str("\n</ontology>\n");
    }
    out.push_str(
        "</project_context>\n\n\
         Respect the project purpose and schema above:\n\
         - Never produce output that contradicts the stated purpose/scope.\n\
         - Match the naming and section conventions declared in schema.\n\
         - If the user's request falls outside the purpose, prefer to ask a\n  \
           brief clarifying question rather than silently drifting.\n\
         - When the text you write states that one document causes, affects,\n  \
           replaces or implements another, write that link as `관계:: [[target]]`\n  \
           using ONLY the relations in <ontology>. Leave existing frontmatter\n  \
           keys exactly as they are.\n",
    );
    Some(out)
}

/// 새 프로젝트의 `purpose.md` 시드 — 헤더 한 줄만. 본문은 사용자가 직접
/// 채우거나 외부 AI 에이전트 (Claude Code · Codex) 에 위임. DocView 의
/// 안내 배너가 빈 골격일 때 이 사실을 알려준다.
pub fn default_purpose_template(project: &str) -> String {
    format!("# {project} · Purpose\n")
}

/// 새 프로젝트의 `schema.md` 시드 — 헤더 한 줄만. 위와 같은 이유.
pub fn default_schema_template(project: &str) -> String {
    format!("# {project} · Schema\n")
}

/// Writes the default templates if the files don't already exist.
/// Idempotent — safe to call on every project creation / reopen.
/// 새 프로젝트에 깔리는 `약속.md` 골격. 안내는 HTML 주석이라 브리핑에
/// 들어가지 않고, 빈 섹션도 브리핑에서 빠진다.
pub fn default_promises_template() -> String {
    "# 약속\n\n\
<!-- 매 세션 시작 브리핑에 그대로 들어가는 문서예요. 항목이 있는 섹션만 들어가요.\n\
- 사용자: 대표님에 대한 사실·이 프로젝트에서의 역할\n\
- 지적받은 것: 에이전트가 틀려서 고쳐 준 것\n\
- 약속: 하기로/안 하기로 정한 작업 규칙\n\
한 줄에 하나, 짧게. 줄 앞에 [#id] 를 붙이면 같은 항목을 고칠 때 교체돼요.\n\
결정은 decisions/, 할 일은 할 일함에. 전체 900자 안쪽. -->\n\n\
## 사용자\n\n## 지적받은 것\n\n## 약속\n"
        .to_string()
}

pub fn promises_path(vault: &Path, project: &str) -> PathBuf {
    project_root(vault, project).join(PROMISES_FILENAME)
}

const PROMISES_SEEDED_MARKER: &str = ".danbi/promises-seeded";

/// 기존 프로젝트에 `약속.md` 골격을 한 번 깐다 (없는 곳만). 끝나면 표식을
/// 남겨 다시 돌지 않으니, 사용자가 지운 파일을 되살리지 않는다. 파일
/// 수정 시각은 그 프로젝트의 purpose.md 에 맞춰 "최근 바뀐 문서" 를
/// 어지럽히지 않고, 커밋은 `scaffold` 라 활동·새 변경으로 세지 않는다.
pub fn backfill_promises(vault: &Path) -> DanbiResult<usize> {
    let marker = vault.join(PROMISES_SEEDED_MARKER);
    if marker.exists() {
        return Ok(0);
    }
    let _guard = crate::vcs::write_guard();
    let mut created = 0usize;
    if let Ok(entries) = std::fs::read_dir(vault.join(PROJECTS_DIRNAME)) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !e.path().is_dir() {
                continue;
            }
            let path = promises_path(vault, &name);
            if path.exists() {
                continue;
            }
            std::fs::write(&path, default_promises_template())?;
            let anchor = std::fs::metadata(purpose_path(vault, &name))
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if let Ok(f) = std::fs::File::options().write(true).open(&path) {
                let _ = f.set_modified(anchor);
            }
            created += 1;
        }
    }
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&marker, chrono::Local::now().to_rfc3339())?;
    drop(_guard);
    if created > 0 {
        crate::vcs::ensure_repo(vault)?;
        let _ = crate::vcs::snapshot_as(
            vault,
            &format!("danbi: scaffold · 약속.md 골격 {created}개"),
            crate::vcs::CLIENT_DANBI,
        );
    }
    Ok(created)
}

pub fn ensure_templates(vault: &Path, project: &str) -> DanbiResult<()> {
    let k = promises_path(vault, project);
    if !k.exists() {
        if let Some(parent) = k.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&k, default_promises_template())?;
    }
    let p = purpose_path(vault, project);
    if !p.exists() {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, default_purpose_template(project))?;
    }
    let s = schema_path(vault, project);
    if !s.exists() {
        if let Some(parent) = s.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&s, default_schema_template(project))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(tag: &str) -> PathBuf {
        let v = std::env::temp_dir().join(format!(
            "danbi-pc-{tag}-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(v.join(PROJECTS_DIRNAME)).unwrap();
        v
    }

    #[test]
    fn skeleton_stays_out_of_the_brief_until_it_has_items() {
        let vault = temp_vault("skel");
        std::fs::create_dir_all(vault.join(PROJECTS_DIRNAME).join("알파")).unwrap();
        ensure_templates(&vault, "알파").unwrap();
        assert!(promises_path(&vault, "알파").exists());
        assert_eq!(promises_for_brief(&vault, "알파", 900), None, "empty skeleton is silent");

        let filled = default_promises_template().replace("## 약속\n", "## 약속\n- [#dmg] DMG 는 요청할 때만\n");
        std::fs::write(promises_path(&vault, "알파"), filled).unwrap();
        let brief = promises_for_brief(&vault, "알파", 900).unwrap();
        assert_eq!(brief, "## 약속\n- [#dmg] DMG 는 요청할 때만", "only sections with items, no guide comment");
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn backfill_seeds_missing_files_once_and_keeps_mtime_old() {
        let vault = temp_vault("backfill");
        let root = vault.join(PROJECTS_DIRNAME);
        for p in ["알파", "베타"] {
            std::fs::create_dir_all(root.join(p)).unwrap();
        }
        std::fs::write(root.join("베타").join(PROMISES_FILENAME), "## 약속\n- 있던 것\n").unwrap();
        std::fs::write(root.join("알파").join(PURPOSE_FILENAME), "# 목적\n").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(root.join("알파").join(PURPOSE_FILENAME))
            .unwrap()
            .set_modified(old)
            .unwrap();

        assert_eq!(backfill_promises(&vault).unwrap(), 1);
        let seeded = promises_path(&vault, "알파");
        assert_eq!(std::fs::read_to_string(&seeded).unwrap(), default_promises_template());
        assert_eq!(std::fs::metadata(&seeded).unwrap().modified().unwrap(), old);
        assert_eq!(std::fs::read_to_string(promises_path(&vault, "베타")).unwrap(), "## 약속\n- 있던 것\n");

        // 한 번만 — 사용자가 지운 파일을 되살리지 않는다.
        std::fs::remove_file(&seeded).unwrap();
        assert_eq!(backfill_promises(&vault).unwrap(), 0);
        assert!(!seeded.exists());
        let _ = std::fs::remove_dir_all(&vault);
    }
}
