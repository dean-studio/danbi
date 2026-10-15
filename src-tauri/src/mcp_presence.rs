//! "Is anything actually talking to us?" — the last authenticated MCP
//! request per project, kept in memory. The usage log only records
//! *writes*, so a project whose agent has only read so far would look
//! disconnected; this covers reads too. Resets with the app, which is
//! fine: the project home combines it with the persisted last write.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct Presence {
    pub ts_ms: i64,
    /// `usage::classify_user_agent` label ("claude_code", "codex", …).
    pub client: String,
}

/// Key for requests on the unscoped `/mcp` and `/mcp/read` endpoints.
pub const ANY: &str = "*";

fn table() -> &'static Mutex<HashMap<String, Presence>> {
    static T: OnceLock<Mutex<HashMap<String, Presence>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Called once per authenticated JSON-RPC request. `project_id` is the
/// path segment of a scoped endpoint, `None` for `/mcp` and `/mcp/read`.
pub fn note(project_id: Option<&str>, user_agent: Option<&str>) {
    let p = Presence {
        ts_ms: chrono::Utc::now().timestamp_millis(),
        client: crate::usage::classify_user_agent(user_agent).to_string(),
    };
    if let Ok(mut t) = table().lock() {
        t.insert(ANY.to_string(), p.clone());
        if let Some(id) = project_id {
            t.insert(id.to_string(), p);
        }
    }
}

pub fn last_for(project_id: &str) -> Option<Presence> {
    table().lock().ok().and_then(|t| t.get(project_id).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_records_scoped_and_any() {
        note(Some("pid-test-1"), Some("claude-code/2.0"));
        let p = last_for("pid-test-1").unwrap();
        assert_eq!(p.client, "claude_code");
        assert!(last_for(ANY).is_some());
        assert!(last_for("pid-never").is_none());
    }
}
