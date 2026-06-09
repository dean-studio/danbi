//! Per-project goals — short, user-editable statements of "what I'm trying
//! to do in this project right now". Surfaced in the project dashboard
//! and injected into MCP tool responses so external Claude sessions stay
//! oriented even when the user doesn't explicitly remind them.
//!
//! Storage: `<vault>/Projects/<P>/.danbi/goals.json`. JSON list, atomic
//! write via tempfile rename. Drift detection is intentionally NOT
//! implemented — the user decides whether they're on track.

use crate::error::{DanbiError, DanbiResult};
use crate::vault::PROJECTS_DIRNAME;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const GOALS_FILE: &str = ".danbi/goals.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<i64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct GoalFile {
    #[serde(default)]
    goals: Vec<Goal>,
}

fn is_safe_project(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.chars().any(|c| {
            c == '/' || c == '\\' || c == '\0' || c == '\n' || c == '\r'
        })
}

fn goals_path(vault: &Path, project: &str) -> DanbiResult<PathBuf> {
    if !is_safe_project(project) {
        return Err(DanbiError::Config(format!(
            "unsafe project name: {project}"
        )));
    }
    Ok(vault.join(PROJECTS_DIRNAME).join(project).join(GOALS_FILE))
}

fn load_file(vault: &Path, project: &str) -> DanbiResult<GoalFile> {
    let path = goals_path(vault, project)?;
    if !path.exists() {
        return Ok(GoalFile::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(GoalFile::default());
    }
    // Tolerate corruption — start fresh rather than block the user from
    // adding new goals.
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn save_file(vault: &Path, project: &str, file: &GoalFile) -> DanbiResult<()> {
    let path = goals_path(vault, project)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(file)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn fresh_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    now_secs().hash(&mut h);
    std::process::id().hash(&mut h);
    // Cheap entropy from a thread-local counter; collisions are user-
    // visible (you'd see two goals with the same id) but not catastrophic.
    static COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    COUNTER
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .hash(&mut h);
    format!("{:x}", h.finish())
}

pub fn list_active(vault: &Path, project: &str) -> DanbiResult<Vec<Goal>> {
    let file = load_file(vault, project)?;
    Ok(file
        .goals
        .into_iter()
        .filter(|g| g.archived_at.is_none())
        .collect())
}

pub fn list_all(vault: &Path, project: &str) -> DanbiResult<Vec<Goal>> {
    Ok(load_file(vault, project)?.goals)
}

pub fn add(
    vault: &Path,
    project: &str,
    title: &str,
    note: Option<String>,
) -> DanbiResult<Goal> {
    let title = title.trim();
    if title.is_empty() {
        return Err(DanbiError::Config("goal title required".into()));
    }
    let goal = Goal {
        id: fresh_id(),
        title: title.to_string(),
        note: note.and_then(|s| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }),
        created_at: now_secs(),
        archived_at: None,
    };
    let mut file = load_file(vault, project)?;
    file.goals.push(goal.clone());
    save_file(vault, project, &file)?;
    Ok(goal)
}

pub fn edit(
    vault: &Path,
    project: &str,
    id: &str,
    title: Option<String>,
    note: Option<Option<String>>,
) -> DanbiResult<Goal> {
    let mut file = load_file(vault, project)?;
    let g = file
        .goals
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| DanbiError::Config(format!("goal not found: {id}")))?;
    if let Some(t) = title {
        let trimmed = t.trim();
        if trimmed.is_empty() {
            return Err(DanbiError::Config("goal title required".into()));
        }
        g.title = trimmed.to_string();
    }
    if let Some(n) = note {
        g.note = n.and_then(|s| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        });
    }
    let updated = g.clone();
    save_file(vault, project, &file)?;
    Ok(updated)
}

pub fn archive(vault: &Path, project: &str, id: &str) -> DanbiResult<Goal> {
    let mut file = load_file(vault, project)?;
    let g = file
        .goals
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| DanbiError::Config(format!("goal not found: {id}")))?;
    if g.archived_at.is_none() {
        g.archived_at = Some(now_secs());
    }
    let updated = g.clone();
    save_file(vault, project, &file)?;
    Ok(updated)
}

pub fn unarchive(vault: &Path, project: &str, id: &str) -> DanbiResult<Goal> {
    let mut file = load_file(vault, project)?;
    let g = file
        .goals
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| DanbiError::Config(format!("goal not found: {id}")))?;
    g.archived_at = None;
    let updated = g.clone();
    save_file(vault, project, &file)?;
    Ok(updated)
}

pub fn delete(vault: &Path, project: &str, id: &str) -> DanbiResult<()> {
    let mut file = load_file(vault, project)?;
    let before = file.goals.len();
    file.goals.retain(|g| g.id != id);
    if file.goals.len() == before {
        return Err(DanbiError::Config(format!("goal not found: {id}")));
    }
    save_file(vault, project, &file)?;
    Ok(())
}

/// Returns just the active titles — used by MCP banner injection where
/// the full struct isn't needed.
pub fn active_titles(vault: &Path, project: &str) -> Vec<String> {
    list_active(vault, project)
        .unwrap_or_default()
        .into_iter()
        .map(|g| g.title)
        .collect()
}
