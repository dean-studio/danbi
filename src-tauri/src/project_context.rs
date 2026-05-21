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
    ctx
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

    if purpose.is_none() && schema.is_none() {
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
    out.push_str(
        "</project_context>\n\n\
         Respect the project purpose and schema above:\n\
         - Never produce output that contradicts the stated purpose/scope.\n\
         - Match the naming and section conventions declared in schema.\n\
         - If the user's request falls outside the purpose, prefer to ask a\n  \
           brief clarifying question rather than silently drifting.\n",
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
pub fn ensure_templates(vault: &Path, project: &str) -> DanbiResult<()> {
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
