//! LLM token usage tracking.
//!
//! Every provider call (chat / embedding) records a single `UsageEvent`
//! line into `usage.jsonl` under the OS config dir (not the vault — the
//! vault is a git repo and we don't want noisy autocommits on every API
//! call). The dashboard reads back this log to compute running totals in
//! KRW using the pricing table in `pricing.rs`.
//!
//! Role is threaded in via a `tokio::task_local` so call sites only need
//! to wrap their provider invocation with `usage::with_role("routing")`;
//! the provider implementations pick the role up when writing the log.
//!
//! ## MCP inbound bucket (v0.4.0)
//!
//! The MCP server also writes events here under the `mcp_inbound` role so
//! the dashboard can show "how much knowledge external agents (Claude
//! Code, Codex) saved into the vault." For these events:
//!
//! - `provider` = `"mcp"` (synthetic — not a real LLM provider)
//! - `model`    = `"cl100k_base"` (the tokenizer used to estimate)
//! - `input_tokens` = estimated content tokens (tiktoken cl100k_base)
//! - `output_tokens` = 0 (this bucket only counts saved content)
//! - `client` / `tool` / `project` / `domain` are populated so the
//!   dashboard can break the count down per agent / per project / per
//!   tool. None of those fields existed in v0.3.x events, hence the
//!   `#[serde(default)]` for backwards compatibility.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tokio::task_local;
use tiktoken_rs::CoreBPE;

/// Synthetic provider name for events that come from the MCP server
/// rather than a real LLM call. Lets the dashboard partition these out
/// trivially with one `provider == "mcp"` filter.
pub const MCP_PROVIDER: &str = "mcp";

/// Role tag for MCP inbound events. Mirrors the existing role buckets
/// ("routing", "writer", "embed", …) so the role filter still works.
pub const MCP_ROLE: &str = "mcp_inbound";

/// Tokenizer name we record in the `model` field for MCP inbound events.
/// Surfacing it in the log makes it obvious that the count is an estimate
/// from cl100k, not a billed Claude/OpenAI count.
pub const MCP_TOKENIZER: &str = "cl100k_base";

/// A single LLM call's billable footprint. One line per call in the JSONL
/// log — append-only so the file tolerates concurrent writers as long as
/// we use a mutex (see `append_event`).
///
/// The `client` / `project` / `domain` / `tool` / `raw_user_agent` fields
/// were added in v0.4.0 for MCP inbound tracking. Older log lines lack
/// them, hence `#[serde(default)]` everywhere — deserialization stays
/// lossless.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    /// Unix epoch milliseconds.
    pub ts_ms: i64,
    /// Provider kind, e.g. "bedrock", "openai", or `MCP_PROVIDER`.
    pub provider: String,
    /// Role bucket: "routing" | "writer" | "embed" | "qa" | "ghost" |
    /// "preview" | "briefing" | "compound" | "search" | `MCP_ROLE` |
    /// "other".
    pub role: String,
    /// Raw model id as sent to the provider (or tokenizer name for
    /// MCP inbound).
    pub model: String,
    /// Input token count. Embedding calls put total input tokens here too.
    /// MCP inbound events put the estimated content tokens here.
    pub input_tokens: u32,
    /// Output token count. 0 for embedding and MCP inbound calls.
    pub output_tokens: u32,

    // -- v0.4.0 MCP inbound fields (all optional / default-empty) -------

    /// Calling client kind: `"claude_code"` | `"codex"` | `"unknown"`.
    /// Only set for MCP inbound events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Vault project the write targeted. Only set for MCP inbound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Domain (file path inside the project) the write targeted. Only
    /// set for MCP inbound; `danbi_create_folder` leaves this `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// MCP tool name (`danbi_log`, `danbi_append`, `danbi_create_file`,
    /// `danbi_create_folder`). Only set for MCP inbound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Raw User-Agent header from the HTTP request, kept as-is so we
    /// can recognise new clients later without a code change. Only set
    /// for MCP inbound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_user_agent: Option<String>,
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
pub fn usage_log_path() -> Option<PathBuf> {
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
        client: None,
        project: None,
        domain: None,
        tool: None,
        raw_user_agent: None,
    };
    if let Err(e) = append_event(&ev) {
        eprintln!("[usage] failed to append: {e}");
    }
}

/// Record an MCP inbound write (Claude Code / Codex calling
/// `danbi_log`, `danbi_append`, `danbi_create_file`, or
/// `danbi_create_folder`). The token count is an *estimate* of the
/// stored content using the cl100k_base tokenizer — see module docs.
///
/// Fire-and-forget: failures here must never break the MCP response.
/// Locking errors, disk full, tokenizer crash — all swallowed with a
/// warn log.
pub fn record_mcp_inbound(
    client: &str,
    tool: &str,
    project: Option<&str>,
    domain: Option<&str>,
    content_tokens: u32,
    raw_user_agent: Option<&str>,
) {
    let ev = UsageEvent {
        ts_ms: Utc::now().timestamp_millis(),
        provider: MCP_PROVIDER.to_string(),
        role: MCP_ROLE.to_string(),
        model: MCP_TOKENIZER.to_string(),
        input_tokens: content_tokens,
        output_tokens: 0,
        client: Some(client.to_string()),
        project: project.map(|s| s.to_string()),
        domain: domain.map(|s| s.to_string()),
        tool: Some(tool.to_string()),
        raw_user_agent: raw_user_agent.map(|s| s.to_string()),
    };
    if let Err(e) = append_event(&ev) {
        eprintln!("[usage] failed to append mcp inbound: {e}");
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

// ---------- Retention sweep ----------
//
// Long-running installs accumulate usage events forever. Once a year of
// events is on disk we move the older lines into `usage.archive.jsonl`
// and truncate `usage.jsonl` to just what's still in retention. Done
// in a single rewrite pass to keep concurrent appends safe — the lock
// already guards both files.

/// Path to the archive file (sibling of `usage.jsonl`). Older events
/// move here when the retention sweep runs, so they're not lost — just
/// removed from the live log the dashboard reads.
pub fn archive_log_path() -> Option<PathBuf> {
    let base = dirs::config_dir()?.join("danbi");
    Some(base.join("usage.archive.jsonl"))
}

/// Rotate events older than `retention_days` out of the live log. A
/// retention value of `0` or less is treated as "no retention" and is
/// a no-op. Safe to call from any thread; takes the same lock as
/// `append_event`.
///
/// Returns the number of lines moved to the archive (0 when nothing
/// needed rotation).
pub fn run_retention_sweep(retention_days: i64) -> std::io::Result<usize> {
    if retention_days <= 0 {
        return Ok(0);
    }
    let Some(live_path) = usage_log_path() else {
        return Ok(0);
    };
    if !live_path.exists() {
        return Ok(0);
    }
    let cutoff_ms =
        Utc::now().timestamp_millis() - retention_days * 24 * 60 * 60 * 1_000;

    // Read everything, partition by cutoff, rewrite. Cheap up to the
    // ~50MB cap our retention enforces; no streaming needed.
    let _guard = LOG_LOCK.lock().ok();
    let text = fs::read_to_string(&live_path)?;
    let mut keep: Vec<String> = Vec::new();
    let mut archive: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // We need to look at ts_ms but want to preserve the original
        // formatting (whitespace, key order) on rewrite — so deserialize
        // into a Value, check ts_ms, and keep the original line.
        let ts_ms = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|v| v.get("ts_ms").and_then(|t| t.as_i64()))
            .unwrap_or(i64::MAX);
        if ts_ms < cutoff_ms {
            archive.push(line.to_string());
        } else {
            keep.push(line.to_string());
        }
    }
    if archive.is_empty() {
        return Ok(0);
    }

    // Append-archive then rewrite-live. If archive append fails we
    // bail without touching the live file — losing a sweep cycle is
    // strictly preferable to losing data.
    if let Some(arc_path) = archive_log_path() {
        if let Some(parent) = arc_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut arc = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&arc_path)?;
        for l in &archive {
            writeln!(arc, "{l}")?;
        }
    }
    fs::write(&live_path, keep.join("\n") + if keep.is_empty() { "" } else { "\n" })?;
    Ok(archive.len())
}

// ---------- Export ----------

/// Emit the live log as JSON (a single array of every event). Used by
/// the "Export" button in the MCP inbound dashboard. Returns an empty
/// `[]` when no log exists yet so the caller doesn't have to special-
/// case missing files.
pub fn export_json() -> std::io::Result<String> {
    let events = load_all().unwrap_or_default();
    serde_json::to_string_pretty(&events)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}

/// Emit the live log as CSV. Columns match the `UsageEvent` struct,
/// flattened — optional fields become empty strings rather than the
/// JSON `null` so spreadsheet imports stay clean.
pub fn export_csv() -> std::io::Result<String> {
    let events = load_all().unwrap_or_default();
    let mut out = String::new();
    out.push_str("ts_ms,iso8601,provider,role,model,input_tokens,output_tokens,client,project,domain,tool,raw_user_agent\n");
    for ev in events {
        let iso = chrono::DateTime::<Utc>::from_timestamp_millis(ev.ts_ms)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}\n",
            ev.ts_ms,
            iso,
            csv_escape(&ev.provider),
            csv_escape(&ev.role),
            csv_escape(&ev.model),
            ev.input_tokens,
            ev.output_tokens,
            csv_escape(ev.client.as_deref().unwrap_or("")),
            csv_escape(ev.project.as_deref().unwrap_or("")),
            csv_escape(ev.domain.as_deref().unwrap_or("")),
            csv_escape(ev.tool.as_deref().unwrap_or("")),
            csv_escape(ev.raw_user_agent.as_deref().unwrap_or("")),
        ));
    }
    Ok(out)
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

// ---------- Tokenizer ----------
//
// We bundle the cl100k_base BPE encoder (the Claude/GPT-4 family
// tokenizer) and reuse a single instance for the lifetime of the
// process. Loading the merge table costs ~5MB of RAM and ~50ms on
// cold start, so doing it once via `OnceLock` is a worthwhile trade
// for the call-site latency on every MCP write.
//
// Caveat: cl100k_base is the OpenAI tokenizer. Anthropic doesn't
// publish Claude's tokenizer; cl100k diverges from Claude by roughly
// ±5% on English and 5–10% on Korean. The dashboard surfaces this
// caveat via a permanent disclaimer banner — see `dashboard.rs`.

static CL100K: OnceLock<CoreBPE> = OnceLock::new();

/// Returns the shared cl100k_base tokenizer. Initializes lazily on
/// first call. `None` if the bundled tables fail to load (should never
/// happen in practice — included via `tiktoken-rs`'s build-time data).
fn cl100k() -> Option<&'static CoreBPE> {
    if let Some(bpe) = CL100K.get() {
        return Some(bpe);
    }
    match tiktoken_rs::cl100k_base() {
        Ok(bpe) => Some(CL100K.get_or_init(|| bpe)),
        Err(e) => {
            eprintln!("[usage] cl100k init failed: {e}");
            None
        }
    }
}

/// Estimate the token count of a string using cl100k_base.
///
/// Returns `0` for empty strings and on tokenizer-init failure (so a
/// missing tokenizer never blocks a write — we just lose the count for
/// that event). Saturates at `u32::MAX` for absurdly large bodies; the
/// MCP layer caps requests at 32 MB so this is purely defensive.
pub fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let Some(bpe) = cl100k() else { return 0 };
    let n = bpe.encode_with_special_tokens(text).len();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Identify the calling MCP client from a raw User-Agent header value.
///
/// Returns the canonical bucket name. Pattern matching is intentionally
/// permissive — different Claude Code / Codex versions ship slightly
/// different UA strings, and we'd rather over-match into a known bucket
/// than route everything to "unknown". Unrecognised UAs land in
/// `"unknown"`; the raw string is still kept on the event so we can
/// recognise new clients in the data without a code change.
pub fn classify_user_agent(ua: Option<&str>) -> &'static str {
    let Some(ua) = ua else {
        return "unknown";
    };
    let lower = ua.to_lowercase();
    // Claude Code: official UA looks like "claude-code/1.x.y" but the
    // MCP transport sometimes sends "anthropic-mcp-…" — match both.
    if lower.contains("claude-code")
        || lower.contains("claude_code")
        || lower.starts_with("anthropic-")
        || lower.contains("anthropic-mcp")
    {
        return "claude_code";
    }
    // Codex (OpenAI): "codex/…" or "openai-codex/…".
    if lower.contains("codex")
        || lower.contains("openai-codex")
        || lower.starts_with("openai-")
    {
        return "codex";
    }
    // Cursor / Continue / other MCP clients we may want to add later.
    if lower.contains("cursor") {
        return "cursor";
    }
    if lower.contains("continue") {
        return "continue";
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_empty_returns_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_english_within_bounds() {
        // OpenAI rule of thumb: ~4 chars per token for English. We
        // assert a generous range so this test stays stable across
        // tokenizer-data minor revisions.
        let n = estimate_tokens("Hello, world! This is a test sentence.");
        assert!(n > 0 && n < 30, "got {n}");
    }

    #[test]
    fn estimate_tokens_korean_nonzero() {
        // Korean tokenizes worse on cl100k (1–2 tokens per syllable
        // block) but should never round to 0 for non-empty input.
        let n = estimate_tokens("단비는 위키 관리 에이전트입니다.");
        assert!(n > 0, "got {n}");
    }

    #[test]
    fn estimate_tokens_mixed_korean_english_consistent() {
        // Concatenating English + Korean should not be lossy: the
        // total should be at least the larger of the two parts.
        let en = estimate_tokens("This is the JWT refresh decision.");
        let ko = estimate_tokens("JWT refresh 7일로 정함");
        let mixed = estimate_tokens("This is the JWT refresh decision.\nJWT refresh 7일로 정함");
        assert!(mixed >= en, "mixed {mixed} < en {en}");
        assert!(mixed >= ko, "mixed {mixed} < ko {ko}");
    }

    #[test]
    fn classify_user_agent_claude_code() {
        assert_eq!(classify_user_agent(Some("claude-code/1.0.42")), "claude_code");
        assert_eq!(classify_user_agent(Some("Claude-Code/2.x (macOS)")), "claude_code");
        assert_eq!(
            classify_user_agent(Some("anthropic-mcp/0.1.0 claude-code")),
            "claude_code"
        );
    }

    #[test]
    fn classify_user_agent_codex() {
        assert_eq!(classify_user_agent(Some("openai-codex/0.7")), "codex");
        assert_eq!(classify_user_agent(Some("codex-cli/1.0")), "codex");
    }

    #[test]
    fn classify_user_agent_unknown_and_missing() {
        assert_eq!(classify_user_agent(None), "unknown");
        assert_eq!(classify_user_agent(Some("curl/8.4")), "unknown");
        assert_eq!(classify_user_agent(Some("")), "unknown");
    }

    #[test]
    fn record_mcp_inbound_serializes_with_optional_fields() {
        // Round-trip the event through serde to confirm the new
        // optional fields are emitted and that an old-style line
        // without them still deserializes (forward-compat).
        let ev = UsageEvent {
            ts_ms: 1_700_000_000_000,
            provider: MCP_PROVIDER.to_string(),
            role: MCP_ROLE.to_string(),
            model: MCP_TOKENIZER.to_string(),
            input_tokens: 42,
            output_tokens: 0,
            client: Some("claude_code".to_string()),
            project: Some("단비".to_string()),
            domain: Some("daily/2026-06-02.md".to_string()),
            tool: Some("danbi_log".to_string()),
            raw_user_agent: Some("claude-code/1.0".to_string()),
        };
        let line = serde_json::to_string(&ev).unwrap();
        assert!(line.contains("\"client\":\"claude_code\""));
        assert!(line.contains("\"tool\":\"danbi_log\""));

        // Old-style line (no MCP fields) still parses.
        let old = r#"{"ts_ms":1,"provider":"bedrock","role":"writer","model":"claude-sonnet-4-6","input_tokens":100,"output_tokens":50}"#;
        let parsed: UsageEvent = serde_json::from_str(old).unwrap();
        assert!(parsed.client.is_none());
        assert!(parsed.tool.is_none());
        assert_eq!(parsed.input_tokens, 100);
    }
}
