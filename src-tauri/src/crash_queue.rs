//! Crash-recovery queue for long-running batch jobs.
//!
//! Scope: bulk ghost scans and compound generations can take minutes.
//! If the app dies mid-run we want the next launch to notice and either
//! resume or at least tell the user "this was interrupted, retry?".
//!
//! Storage: `<vault>/.danbi/queue/<kind>.json`. One file per job kind,
//! overwritten as state transitions. Simpler than one-file-per-run
//! because we only ever have one bulk job of each kind at a time.

use crate::error::DanbiResult;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const QUEUE_DIR: &str = ".danbi/queue";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    Running,
    Failed,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub kind: String,
    pub payload: serde_json::Value,
    pub state: JobState,
    pub started_at: i64,
    #[serde(default)]
    pub finished_at: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

fn job_path(vault: &Path, kind: &str) -> PathBuf {
    vault.join(QUEUE_DIR).join(format!("{kind}.json"))
}

pub fn begin(vault: &Path, kind: &str, payload: serde_json::Value) -> DanbiResult<()> {
    let rec = JobRecord {
        kind: kind.to_string(),
        payload,
        state: JobState::Running,
        started_at: Local::now().timestamp(),
        finished_at: None,
        error: None,
    };
    let path = job_path(vault, kind);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&rec)?)?;
    Ok(())
}

pub fn finish(vault: &Path, kind: &str) -> DanbiResult<()> {
    let path = job_path(vault, kind);
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut rec: JobRecord = serde_json::from_str(&raw)?;
    rec.state = JobState::Done;
    rec.finished_at = Some(Local::now().timestamp());
    std::fs::write(&path, serde_json::to_string_pretty(&rec)?)?;
    Ok(())
}

pub fn fail(vault: &Path, kind: &str, err: String) -> DanbiResult<()> {
    let path = job_path(vault, kind);
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut rec: JobRecord = serde_json::from_str(&raw)?;
    rec.state = JobState::Failed;
    rec.finished_at = Some(Local::now().timestamp());
    rec.error = Some(err);
    std::fs::write(&path, serde_json::to_string_pretty(&rec)?)?;
    Ok(())
}

/// Flips every `Running` record to `Pending` so the caller (or UI) can
/// resume. Called once on app startup.
pub fn reset_stale(vault: &Path) -> DanbiResult<Vec<JobRecord>> {
    let dir = vault.join(QUEUE_DIR);
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let Ok(mut rec) = serde_json::from_str::<JobRecord>(&raw) else {
            continue;
        };
        if rec.state == JobState::Running {
            rec.state = JobState::Pending;
            let _ = std::fs::write(&path, serde_json::to_string_pretty(&rec)?);
            out.push(rec);
        }
    }
    Ok(out)
}
