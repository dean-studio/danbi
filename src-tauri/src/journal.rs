use crate::error::DanbiResult;
use chrono::Local;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub const LOG_FILENAME: &str = "log.md";
pub const HISTORY_FILENAME: &str = "history.jsonl";

/// Appends a single bullet to `log.md` — the human-readable timeline.
pub fn append_log(vault: &Path, entry: &str) -> DanbiResult<()> {
    let path = vault.join(LOG_FILENAME);
    let now = Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("- `{now}` {entry}\n");
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Appends a JSON line to `history.jsonl` — short-term memory for chat context
/// and undo support.
pub fn append_history<T: Serialize>(vault: &Path, event: &T) -> DanbiResult<()> {
    let path = vault.join(HISTORY_FILENAME);
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}
