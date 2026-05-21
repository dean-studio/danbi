//! LLM token usage tracking.
//!
//! Every provider call (chat / embedding) records a single `UsageEvent`
//! line into `usage.jsonl` under the OS config dir (not the vault — the
//! vault is a git repo and we don't want noisy autocommits on every API
//! call). The dashboard reads back this log to compute running totals in
//! KRW using the pricing table in `pricing.rs` (next step).
//!
//! Role is threaded in via a `tokio::task_local` so call sites only need
//! to wrap their provider invocation with `usage::with_role("routing")`;
//! the provider implementations pick the role up when writing the log.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::task_local;

/// A single LLM call's billable footprint. One line per call in the JSONL
/// log — append-only so the file tolerates concurrent writers as long as
/// we use a mutex (see `append_event`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    /// Unix epoch milliseconds.
    pub ts_ms: i64,
    /// Provider kind, e.g. "bedrock", "openai".
    pub provider: String,
    /// Role bucket: "routing" | "writer" | "embed" | "qa" | "ghost" |
    /// "preview" | "briefing" | "compound" | "search" | "other".
    pub role: String,
    /// Raw model id as sent to the provider.
    pub model: String,
    /// Input token count. Embedding calls put total input tokens here too.
    pub input_tokens: u32,
    /// Output token count. 0 for embedding calls.
    pub output_tokens: u32,
}

task_local! {
    /// Current call's role tag. Nested calls inherit the outer role, so we
    /// only need to set this at the top-level entry point of each feature.
    static ROLE: String;
}

/// Run `fut` with `role` bound as the current usage role tag. Provider
/// implementations read it back via `current_role()`.
pub async fn with_role<F, T>(role: &str, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    ROLE.scope(role.to_string(), fut).await
}

/// Returns the role tag set by the nearest enclosing `with_role`, or
/// "other" when the call didn't flow through a tagged entry point.
pub fn current_role() -> String {
    ROLE.try_with(|r| r.clone()).unwrap_or_else(|_| "other".to_string())
}

/// Path to the usage log. Per-user config dir — not the vault, not the
/// app bundle. Creates the parent dir lazily on first append.
fn usage_log_path() -> Option<PathBuf> {
    let base = dirs::config_dir()?.join("danbi");
    Some(base.join("usage.jsonl"))
}

/// Append a single event. Errors are swallowed with a warn-level log —
/// usage tracking must never break a real LLM call.
static LOG_LOCK: Mutex<()> = Mutex::new(());

pub fn record(provider: &str, model: &str, input_tokens: u32, output_tokens: u32) {
    let ev = UsageEvent {
        ts_ms: Utc::now().timestamp_millis(),
        provider: provider.to_string(),
        role: current_role(),
        model: model.to_string(),
        input_tokens,
        output_tokens,
    };
    if let Err(e) = append_event(&ev) {
        eprintln!("[usage] failed to append: {e}");
    }
}

fn append_event(ev: &UsageEvent) -> std::io::Result<()> {
    let Some(path) = usage_log_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(ev).unwrap_or_default();
    let _guard = LOG_LOCK.lock().ok();
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Load every event in the log. Returns an empty vec if the file doesn't
/// exist yet.
pub fn load_all() -> std::io::Result<Vec<UsageEvent>> {
    let Some(path) = usage_log_path() else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<UsageEvent>(line) {
            out.push(ev);
        }
    }
    Ok(out)
}
