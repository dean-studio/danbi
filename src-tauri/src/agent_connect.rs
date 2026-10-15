//! One-click MCP connection files for coding agents + the machine-local
//! registry of every file Danbi has written, so a token rotation can
//! refresh them in place.
//!
//! Files we write (all inside the code folder the user picked):
//!   - Claude Code: `<repo>/.mcp.json` (project scope, read by every
//!     account that opens the folder). Entries are `{type:"http", url,
//!     headers}`.
//!   - Cursor: `<repo>/.cursor/mcp.json` — same shape minus `type`.
//!   - Codex: `<repo>/.codex/config.toml` (written by `commands::codex_connect`,
//!     registered here so rotation covers it too).
//!
//! The endpoint split from bf0736c is reflected as two servers per
//! file: `danbi_read` → `/mcp/read` (read tools, identical in every
//! folder so a user-scope duplicate collapses to one) and
//! `danbi-<slug>` → `/mcp/<project-id>/write` (write tools clamped to the
//! project). Server names come from the backend only — the old frontend
//! slug turned every Korean project into `danbi` and collided.
//!
//! Registry: `~/.danbi/connections.json`. Never inside the vault (which
//! is a git repo) and never in `~/.claude*.json` — Danbi reads the
//! user's Claude Code config only to *detect* legacy registrations.

use crate::error::{DanbiError, DanbiResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Name of the shared read-only server. Matches the "권장 연결" command in
/// Settings so a user-scope registration and a project-scope one merge.
pub const READ_SERVER_NAME: &str = "danbi_read";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Cursor,
    Codex,
}

impl Agent {
    pub fn parse(s: &str) -> Option<Agent> {
        match s {
            "claude" => Some(Agent::Claude),
            "cursor" => Some(Agent::Cursor),
            "codex" => Some(Agent::Codex),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Cursor => "cursor",
            Agent::Codex => "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Cursor => "Cursor",
            Agent::Codex => "Codex",
        }
    }

    /// Connection file relative to the code folder.
    pub fn config_rel_path(self) -> &'static str {
        match self {
            Agent::Claude => ".mcp.json",
            Agent::Cursor => ".cursor/mcp.json",
            Agent::Codex => ".codex/config.toml",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerEntry {
    pub name: String,
    pub url: String,
}

/// One connection file Danbi wrote. Keyed by `(agent, path)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub agent: String,
    pub project: String,
    /// Absolute path of the connection file.
    pub path: String,
    pub servers: Vec<ServerEntry>,
    /// Unix seconds of the last (re)write.
    pub connected_at: i64,
}

/// MCP server name for a project's write endpoint — ASCII-safe, stable,
/// lower-case so it lines up with names users typed by hand
/// (`danbi-bid`). Korean names hash to `danbi-<8hex>` instead of the old
/// frontend slug that collapsed them all to `danbi`.
pub fn write_server_name(project: &str) -> String {
    crate::commands::skill_dir_for(project).to_lowercase()
}

pub fn read_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp/read")
}

pub fn write_url(port: u16, project_id: &str) -> String {
    format!("http://127.0.0.1:{port}/mcp/{project_id}/write")
}

/// The two servers written for Claude Code / Cursor.
pub fn split_servers(project: &str, project_id: &str, port: u16) -> Vec<ServerEntry> {
    vec![
        ServerEntry {
            name: READ_SERVER_NAME.to_string(),
            url: read_url(port),
        },
        ServerEntry {
            name: write_server_name(project),
            url: write_url(port, project_id),
        },
    ]
}

// ---------- mcp.json merge -------------------------------------------------

fn server_value(agent: Agent, url: &str, token: &str) -> Value {
    let mut obj = Map::new();
    if agent == Agent::Claude {
        obj.insert("type".into(), json!("http"));
    }
    obj.insert("url".into(), json!(url));
    obj.insert(
        "headers".into(),
        json!({ "Authorization": format!("Bearer {token}") }),
    );
    Value::Object(obj)
}

/// Merge Danbi's servers into an `mcp.json` body. Other top-level keys
/// and other servers are kept; a server with one of our names is
/// replaced in place (so re-connecting never stacks). Returns the new
/// body and whether any of our entries already existed.
pub fn merge_mcp_json(
    existing: &str,
    agent: Agent,
    servers: &[ServerEntry],
    token: &str,
) -> DanbiResult<(String, bool)> {
    let mut root: Value = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(existing).map_err(|e| {
            DanbiError::Other(format!(
                "{} 을 읽을 수 없어요 (JSON 오류: {e}). 파일을 고친 뒤 다시 연결하세요.",
                agent.config_rel_path()
            ))
        })?
    };
    let Some(obj) = root.as_object_mut() else {
        return Err(DanbiError::Other(format!(
            "{} 최상위가 객체가 아니에요.",
            agent.config_rel_path()
        )));
    };
    let slot = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(map) = slot.as_object_mut() else {
        return Err(DanbiError::Other(format!(
            "{} 의 mcpServers 가 객체가 아니에요.",
            agent.config_rel_path()
        )));
    };
    let mut replaced = false;
    // A folder writes to one project. Re-connecting it to another project
    // must drop the previous project's write server, or the agent could
    // still pick it and write into the old project.
    let ours: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
    let stale: Vec<String> = map
        .iter()
        .filter(|(name, v)| !ours.contains(&name.as_str()) && is_danbi_write_url(v))
        .map(|(name, _)| name.clone())
        .collect();
    for name in stale {
        map.remove(&name);
        replaced = true;
    }
    for s in servers {
        if map.contains_key(&s.name) {
            replaced = true;
        }
        map.insert(s.name.clone(), server_value(agent, &s.url, token));
    }
    let mut body = serde_json::to_string_pretty(&root)
        .map_err(|e| DanbiError::Other(format!("serialize mcp.json: {e}")))?;
    body.push('\n');
    Ok((body, replaced))
}

/// A server entry pointing at a Danbi per-project write endpoint
/// (`http://127.0.0.1:<port>/mcp/<id>/write`).
fn is_danbi_write_url(entry: &Value) -> bool {
    entry
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|u| u.starts_with("http://127.0.0.1:") && u.contains("/mcp/") && u.ends_with("/write"))
}

// ---------- registry -------------------------------------------------------

pub fn registry_path(home: &Path) -> PathBuf {
    home.join(".danbi").join("connections.json")
}

pub fn load_registry(path: &Path) -> Vec<Connection> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_registry(path: &Path, list: &[Connection]) -> DanbiResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DanbiError::Other(format!("create ~/.danbi: {e}")))?;
    }
    let body = serde_json::to_string_pretty(list)
        .map_err(|e| DanbiError::Other(format!("serialize connections: {e}")))?;
    std::fs::write(path, body)
        .map_err(|e| DanbiError::Other(format!("write connections.json: {e}")))
}

/// Insert or replace by `(agent, path)`.
pub fn upsert(list: &mut Vec<Connection>, conn: Connection) {
    if let Some(slot) = list
        .iter_mut()
        .find(|c| c.agent == conn.agent && c.path == conn.path)
    {
        *slot = conn;
    } else {
        list.push(conn);
    }
}

/// Record a connection file in `~/.danbi/connections.json`. Failures are
/// not fatal for the connect itself — the file on disk is what matters —
/// but they do mean a later rotation won't find it, so callers surface
/// the error.
pub fn register(home: &Path, conn: Connection) -> DanbiResult<()> {
    let path = registry_path(home);
    let mut list = load_registry(&path);
    upsert(&mut list, conn);
    save_registry(&path, &list)
}

pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Rewrite one registered file with a new token. The servers' names and
/// URLs are kept as recorded. Returns `Ok(false)` when the file no longer
/// exists (the user deleted the folder) — the registry entry is dropped
/// by the caller.
pub fn rewrite_with_token(conn: &Connection, token: &str) -> DanbiResult<bool> {
    let path = Path::new(&conn.path);
    let existing = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(DanbiError::Other(format!("read {}: {e}", conn.path))),
    };
    let Some(agent) = Agent::parse(&conn.agent) else {
        return Err(DanbiError::Other(format!("unknown agent: {}", conn.agent)));
    };
    // Only refresh entries that are still in the file: a server the user
    // deleted by hand stays deleted. None left → treat like a removed file.
    let present: Vec<ServerEntry> = conn
        .servers
        .iter()
        .filter(|s| match agent {
            Agent::Claude | Agent::Cursor => serde_json::from_str::<Value>(&existing)
                .ok()
                .and_then(|v| v.get("mcpServers").and_then(|m| m.get(&s.name)).cloned())
                .is_some(),
            Agent::Codex => existing.contains(&format!("[mcp_servers.{}]", s.name)),
        })
        .cloned()
        .collect();
    if present.is_empty() {
        return Ok(false);
    }
    let merged = match agent {
        Agent::Claude | Agent::Cursor => merge_mcp_json(&existing, agent, &present, token)?.0,
        Agent::Codex => {
            let mut body = existing;
            for s in &present {
                body = crate::commands::merge_codex_block(&body, &s.name, &s.url, token).0;
            }
            body
        }
    };
    std::fs::write(path, merged)
        .map_err(|e| DanbiError::Other(format!("write {}: {e}", conn.path)))?;
    Ok(true)
}

#[derive(Debug, Default, Serialize)]
pub struct RotateReport {
    /// Paths rewritten with the new token.
    pub updated: Vec<String>,
    /// Paths that could not be rewritten, with the error.
    pub failed: Vec<String>,
    /// Registry entries dropped because the file was gone.
    pub removed: Vec<String>,
}

/// Push a new token into every registered connection file.
pub fn rotate_all(home: &Path, token: &str) -> RotateReport {
    let path = registry_path(home);
    let mut list = load_registry(&path);
    let mut report = RotateReport::default();
    list.retain(|conn| match rewrite_with_token(conn, token) {
        Ok(true) => {
            report.updated.push(conn.path.clone());
            true
        }
        Ok(false) => {
            report.removed.push(conn.path.clone());
            false
        }
        Err(e) => {
            report.failed.push(format!("{} — {e}", conn.path));
            true
        }
    });
    if !report.removed.is_empty() {
        let _ = save_registry(&path, &list);
    }
    report
}

// ---------- legacy Claude Code registrations (read-only) -------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LegacyEntry {
    /// `~/.claude.json` or a `~/.claude-<account>/.claude.json`.
    pub config_file: String,
    /// "local" (this folder only) or "user" (every folder).
    pub scope: String,
    pub name: String,
    pub url: String,
}

/// Candidate Claude Code state files: `~/.claude.json` plus the
/// per-account `~/.claude*/.claude.json` dirs (CLAUDE_CONFIG_DIR).
fn claude_state_files(home: &Path) -> Vec<PathBuf> {
    let mut out = vec![home.join(".claude.json")];
    if let Ok(rd) = std::fs::read_dir(home) {
        for de in rd.flatten() {
            let name = de.file_name();
            let Some(n) = name.to_str() else { continue };
            if n.starts_with(".claude") && de.path().is_dir() {
                out.push(de.path().join(".claude.json"));
            }
        }
    }
    out.sort();
    out
}

fn is_danbi_url(url: &str, port: u16) -> bool {
    url.starts_with(&format!("http://127.0.0.1:{port}/mcp"))
        || url.starts_with(&format!("http://localhost:{port}/mcp"))
}

/// Find `claude mcp add` registrations (which live in the user's state
/// file, not in the repo) that point at this Danbi server and would
/// overlap with a `.mcp.json` in `target`. Pure read; nothing is
/// modified. `target_keys` are the spellings of the folder path to look
/// up under `projects` (raw + canonical).
pub fn detect_claude_legacy(
    state_files: &[PathBuf],
    target_keys: &[String],
    port: u16,
) -> Vec<LegacyEntry> {
    let mut out = Vec::new();
    for file in state_files {
        let Ok(raw) = std::fs::read_to_string(file) else {
            continue;
        };
        let Ok(root) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let file_s = file.to_string_lossy().to_string();
        let mut push = |scope: &str, servers: &Value| {
            let Some(map) = servers.as_object() else { return };
            for (name, s) in map {
                let url = s.get("url").and_then(|u| u.as_str()).unwrap_or("");
                if is_danbi_url(url, port) {
                    out.push(LegacyEntry {
                        config_file: file_s.clone(),
                        scope: scope.to_string(),
                        name: name.clone(),
                        url: url.to_string(),
                    });
                }
            }
        };
        if let Some(servers) = root.get("mcpServers") {
            push("user", servers);
        }
        if let Some(projects) = root.get("projects").and_then(|p| p.as_object()) {
            for key in target_keys {
                if let Some(servers) = projects.get(key).and_then(|p| p.get("mcpServers")) {
                    push("local", servers);
                }
            }
        }
    }
    out.sort_by(|a, b| (&a.scope, &a.name).cmp(&(&b.scope, &b.name)));
    out.dedup();
    out
}

/// Detect against the real home. Separate from the pure function so
/// tests can feed their own files.
pub fn detect_claude_legacy_in_home(home: &Path, target: &Path, port: u16) -> Vec<LegacyEntry> {
    let mut keys = vec![target.to_string_lossy().to_string()];
    if let Ok(c) = std::fs::canonicalize(target) {
        let s = c.to_string_lossy().to_string();
        if !keys.contains(&s) {
            keys.push(s);
        }
    }
    detect_claude_legacy(&claude_state_files(home), &keys, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "danbi-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn servers() -> Vec<ServerEntry> {
        split_servers("보니_앨범", "abc-123", 47921)
    }

    #[test]
    fn server_names_are_backend_derived_and_lowercase() {
        assert_eq!(write_server_name("Bid"), "danbi-bid");
        let a = write_server_name("보니_앨범");
        let b = write_server_name("보니_핑퐁");
        assert!(a.starts_with("danbi-"), "{a}");
        assert_ne!(a, b, "korean names must not collide");
        assert_ne!(a, "danbi");
        let s = servers();
        assert_eq!(s[0].name, "danbi_read");
        assert_eq!(s[0].url, "http://127.0.0.1:47921/mcp/read");
        assert_eq!(s[1].url, "http://127.0.0.1:47921/mcp/abc-123/write");
    }

    #[test]
    fn reconnecting_a_folder_to_another_project_drops_the_old_write_server() {
        let (a, _) = merge_mcp_json("", Agent::Claude, &servers(), "TOK").unwrap();
        let b_servers = split_servers("Other", "def-456", 47921);
        let (out, replaced) = merge_mcp_json(&a, Agent::Claude, &b_servers, "TOK").unwrap();
        assert!(replaced);
        let v: Value = serde_json::from_str(&out).unwrap();
        let names: Vec<&String> = v["mcpServers"].as_object().unwrap().keys().collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(v["mcpServers"]["danbi-other"]["url"].as_str().unwrap().ends_with("/def-456/write"));
        // Unrelated servers stay.
        let with_other = out.replace("\"mcpServers\": {", "\"mcpServers\": {\"github\": {\"url\": \"https://x\"},");
        let (out, _) = merge_mcp_json(&with_other, Agent::Claude, &b_servers, "TOK").unwrap();
        assert!(out.contains("github"));
    }

    #[test]
    fn rotation_does_not_resurrect_servers_the_user_deleted() {
        let home = temp_dir("rotate-deleted-home");
        let repo = temp_dir("rotate-deleted-repo");
        let path = repo.join(".mcp.json");
        std::fs::write(&path, "{\"mcpServers\": {\"github\": {\"url\": \"https://x\"}}}\n").unwrap();
        register(
            &home,
            Connection {
                agent: "claude".into(),
                project: "보니_앨범".into(),
                path: path.to_string_lossy().to_string(),
                servers: servers(),
                connected_at: 1,
            },
        )
        .unwrap();
        let report = rotate_all(&home, "NEW");
        assert_eq!(report.removed.len(), 1, "{report:?}");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("danbi"), "{body}");
    }

    #[test]
    fn claude_mcp_json_from_empty() {
        let (out, replaced) = merge_mcp_json("", Agent::Claude, &servers(), "TOK").unwrap();
        assert!(!replaced);
        let v: Value = serde_json::from_str(&out).unwrap();
        let r = &v["mcpServers"]["danbi_read"];
        assert_eq!(r["type"], "http");
        assert_eq!(r["url"], "http://127.0.0.1:47921/mcp/read");
        assert_eq!(r["headers"]["Authorization"], "Bearer TOK");
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn cursor_mcp_json_has_no_type_field() {
        let (out, _) = merge_mcp_json("", Agent::Cursor, &servers(), "TOK").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["mcpServers"]["danbi_read"].get("type").is_none());
        assert_eq!(v["mcpServers"]["danbi_read"]["url"], "http://127.0.0.1:47921/mcp/read");
    }

    #[test]
    fn merge_keeps_other_servers_and_keys_and_replaces_ours() {
        let existing = r#"{
  "mcpServers": {
    "github": { "command": "npx", "args": ["-y", "gh-mcp"] },
    "danbi_read": { "type": "http", "url": "http://127.0.0.1:47921/mcp/read", "headers": { "Authorization": "Bearer OLD" } }
  },
  "somethingElse": { "keep": true }
}"#;
        let (out, replaced) = merge_mcp_json(existing, Agent::Claude, &servers(), "NEW").unwrap();
        assert!(replaced);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["github"]["command"], "npx");
        assert_eq!(v["somethingElse"]["keep"], true);
        assert_eq!(v["mcpServers"]["danbi_read"]["headers"]["Authorization"], "Bearer NEW");
        assert_eq!(v["mcpServers"].as_object().unwrap().len(), 3);
        assert!(!out.contains("OLD"));
        // Idempotent: merging again changes nothing.
        let (again, replaced2) = merge_mcp_json(&out, Agent::Claude, &servers(), "NEW").unwrap();
        assert!(replaced2);
        assert_eq!(again, out);
    }

    #[test]
    fn merge_rejects_broken_json_without_touching_it() {
        assert!(merge_mcp_json("{ not json", Agent::Claude, &servers(), "T").is_err());
        assert!(merge_mcp_json("[]", Agent::Claude, &servers(), "T").is_err());
        assert!(merge_mcp_json(r#"{"mcpServers": 3}"#, Agent::Claude, &servers(), "T").is_err());
    }

    #[test]
    fn registry_upsert_by_agent_and_path() {
        let mut list = Vec::new();
        let c1 = Connection {
            agent: "claude".into(),
            project: "A".into(),
            path: "/x/.mcp.json".into(),
            servers: servers(),
            connected_at: 1,
        };
        upsert(&mut list, c1.clone());
        upsert(&mut list, Connection { connected_at: 2, ..c1.clone() });
        upsert(&mut list, Connection { agent: "cursor".into(), ..c1.clone() });
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].connected_at, 2);
    }

    #[test]
    fn registry_roundtrip_and_missing_file() {
        let home = temp_dir("registry");
        let path = registry_path(&home);
        assert!(load_registry(&path).is_empty());
        let c = Connection {
            agent: "codex".into(),
            project: "A".into(),
            path: "/x/.codex/config.toml".into(),
            servers: vec![ServerEntry { name: "danbi".into(), url: "http://u".into() }],
            connected_at: 5,
        };
        register(&home, c.clone()).unwrap();
        assert_eq!(load_registry(&path), vec![c]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rotate_rewrites_every_registered_file_and_prunes_missing() {
        let home = temp_dir("rotate-home");
        let repo = temp_dir("rotate-repo");
        let claude_path = repo.join(".mcp.json");
        let cursor_path = repo.join(".cursor/mcp.json");
        let codex_path = repo.join(".codex/config.toml");
        std::fs::create_dir_all(cursor_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
        let (c, _) = merge_mcp_json("", Agent::Claude, &servers(), "OLD").unwrap();
        std::fs::write(&claude_path, c).unwrap();
        let (u, _) = merge_mcp_json("", Agent::Cursor, &servers(), "OLD").unwrap();
        std::fs::write(&cursor_path, u).unwrap();
        let codex_servers = vec![ServerEntry {
            name: "danbi".into(),
            url: "http://127.0.0.1:47921/mcp/abc-123".into(),
        }];
        let (t, _) = crate::commands::merge_codex_block(
            "model = \"gpt\"\n",
            "danbi",
            &codex_servers[0].url,
            "OLD",
        );
        std::fs::write(&codex_path, t).unwrap();

        let mk = |agent: &str, path: &Path, servers: Vec<ServerEntry>| Connection {
            agent: agent.into(),
            project: "보니_앨범".into(),
            path: path.to_string_lossy().to_string(),
            servers,
            connected_at: 1,
        };
        register(&home, mk("claude", &claude_path, servers())).unwrap();
        register(&home, mk("cursor", &cursor_path, servers())).unwrap();
        register(&home, mk("codex", &codex_path, codex_servers)).unwrap();
        register(&home, mk("claude", &repo.join("gone/.mcp.json"), servers())).unwrap();

        let report = rotate_all(&home, "NEW");
        assert_eq!(report.updated.len(), 3, "{report:?}");
        assert_eq!(report.removed.len(), 1);
        assert!(report.failed.is_empty());
        for p in [&claude_path, &cursor_path, &codex_path] {
            let body = std::fs::read_to_string(p).unwrap();
            assert!(body.contains("NEW"), "{}", p.display());
            assert!(!body.contains("OLD"), "{}", p.display());
        }
        // Codex: unrelated key survived, exactly one danbi block.
        let codex = std::fs::read_to_string(&codex_path).unwrap();
        assert!(codex.contains("model = \"gpt\""));
        assert_eq!(codex.matches("[mcp_servers.danbi]").count(), 1);
        // The missing one was pruned from the registry.
        assert_eq!(load_registry(&registry_path(&home)).len(), 3);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn legacy_detection_finds_local_and_user_scope_only_for_danbi() {
        let home = temp_dir("legacy");
        let state = home.join(".claude.json");
        std::fs::write(
            &state,
            r#"{
  "mcpServers": { "uibowl": { "type": "http", "url": "https://uibowl.io/api/mcp" } },
  "projects": {
    "/code/app": { "mcpServers": { "danbi": { "type": "http", "url": "http://127.0.0.1:47921/mcp/f898" } } },
    "/code/other": { "mcpServers": { "danbi": { "type": "http", "url": "http://127.0.0.1:47921/mcp/zzz" } } }
  }
}"#,
        )
        .unwrap();
        let acct = home.join(".claude-work");
        std::fs::create_dir_all(&acct).unwrap();
        std::fs::write(
            acct.join(".claude.json"),
            r#"{ "mcpServers": { "danbi_read": { "type": "http", "url": "http://localhost:47921/mcp/read" } } }"#,
        )
        .unwrap();
        let found = detect_claude_legacy(
            &claude_state_files(&home),
            &["/code/app".to_string()],
            47921,
        );
        let names: Vec<(String, String)> =
            found.iter().map(|e| (e.scope.clone(), e.name.clone())).collect();
        assert_eq!(
            names,
            vec![
                ("local".to_string(), "danbi".to_string()),
                ("user".to_string(), "danbi_read".to_string()),
            ]
        );
        // Different port → nothing.
        assert!(detect_claude_legacy(&claude_state_files(&home), &["/code/app".to_string()], 1).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
