//! Minimal MCP (Model Context Protocol) server exposing the vault to external
//! agents like Claude Code or Cursor.
//!
//! Protocol: JSON-RPC 2.0 over HTTP. Claude Code calls `POST /mcp` with a
//! JSON-RPC envelope. We implement the subset needed for `initialize`,
//! `tools/list`, `tools/call`, and `notifications/initialized`.
//!
//! Security:
//! - Bound to 127.0.0.1 only (never reachable from LAN).
//! - Bearer token required; token is stored in config.json and shown in
//!   Settings for the user to paste into their MCP client.

use crate::config::{self, DanbiConfig};
use crate::daily;
use crate::dashboard;
use crate::edit_ops::{self, EditOp};
use crate::error::DanbiError;
use crate::search;
use crate::vault;
use crate::vcs;
use chrono::Local;
use axum::{
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::async_runtime::{spawn, JoinHandle};

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "danbi";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct McpState {
    token: Arc<String>,
}

pub struct McpServer {
    handle: Mutex<Option<JoinHandle<()>>>,
    shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    info: Mutex<Option<McpInfo>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpInfo {
    pub port: u16,
    pub url: String,
    pub token: String,
}

impl McpServer {
    pub fn new() -> Self {
        Self {
            handle: Mutex::new(None),
            shutdown: Mutex::new(None),
            info: Mutex::new(None),
        }
    }

    pub fn info(&self) -> Option<McpInfo> {
        self.info.lock().ok().and_then(|g| g.clone())
    }

    /// Start/restart the server with the given config. Stops any running
    /// instance first. Uses Tauri's async runtime so callers from sync tauri
    /// commands (which may run outside a tokio reactor) don't panic.
    pub fn start(&self, port: u16, token: String) {
        self.stop();
        let state = McpState {
            token: Arc::new(token.clone()),
        };
        // 32MB upper bound on request body. Daily reports + metric dumps
        // can balloon way past axum's 2MB default, especially when
        // schedulers paste raw markdown blobs with embedded graphs / log
        // tails. Emoji and Korean glyphs are 3-4 bytes per char in UTF-8
        // so we err on the generous side.
        const MAX_BODY: usize = 32 * 1024 * 1024;

        let app = Router::new()
            .route("/mcp", post(handle_rpc))
            .route("/mcp/health", post(handle_health).get(handle_health))
            .route("/mcp/:project_id", post(handle_rpc_scoped))
            // 세션당 도구 수를 줄이는 분리 연결. /mcp/read 는 읽기 도구만
            // (전역 1개), /mcp/<id>/write 는 그 프로젝트로 clamp 된 쓰기
            // 도구만 (코드 폴더마다). 기존 /mcp · /mcp/<id> 는 그대로.
            .route("/mcp/read", post(handle_rpc_read))
            .route("/mcp/:project_id/write", post(handle_rpc_scoped_write))
            // Plain REST endpoints — same auth (Bearer token) and same
            // dispatcher as the JSON-RPC routes, but no envelope. Made
            // for external callers (Paperclip, cron jobs, shell scripts)
            // that don't want to construct JSON-RPC payloads.
            .route("/api/health", post(handle_health).get(handle_health))
            .route("/api/tools", get(handle_api_tools))
            .route("/api/call/:tool", post(handle_api_call))
            .route("/api/projects/:project/call/:tool", post(handle_api_call_scoped))
            // Claude Code SessionStart 훅이 부르는 짧은 Markdown 브리핑.
            .route(
                "/api/session-brief",
                get(handle_session_brief).post(handle_session_brief),
            )
            // SessionEnd 훅 — 바로 202 를 돌려주고 요약·기록은 백그라운드.
            .route("/api/session-end", post(handle_session_end))
            // Plain PUT for raw markdown — Obsidian Local REST API style.
            // Lets Paperclip / cron / curl pipe a prebuilt report file in
            // without JSON wrapping (emoji, special punctuation, multi-MB
            // bodies all pass through cleanly).
            .route("/v1/vault/:project/*path", put(handle_v1_vault_put))
            .layer(DefaultBodyLimit::max(MAX_BODY))
            .with_state(state);

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let info = McpInfo {
            port,
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: token.clone(),
        };

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let handle = spawn(async move {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    eprintln!("[mcp] listening on {addr}");
                    let result = axum::serve(listener, app)
                        .with_graceful_shutdown(async {
                            let _ = rx.await;
                        })
                        .await;
                    if let Err(e) = result {
                        eprintln!("[mcp] serve error: {e}");
                    }
                }
                Err(e) => eprintln!("[mcp] bind {addr} failed: {e}"),
            }
        });

        if let Ok(mut slot) = self.handle.lock() {
            *slot = Some(handle);
        }
        if let Ok(mut slot) = self.shutdown.lock() {
            *slot = Some(tx);
        }
        if let Ok(mut slot) = self.info.lock() {
            *slot = Some(info);
        }
    }

    pub fn stop(&self) {
        if let Ok(mut slot) = self.shutdown.lock() {
            if let Some(tx) = slot.take() {
                let _ = tx.send(());
            }
        }
        if let Ok(mut slot) = self.handle.lock() {
            if let Some(h) = slot.take() {
                h.abort();
            }
        }
        if let Ok(mut slot) = self.info.lock() {
            *slot = None;
        }
    }

    pub fn is_running(&self) -> bool {
        self.info.lock().ok().and_then(|g| g.clone()).is_some()
    }
}

// ---------- HTTP handlers ----------

#[derive(Deserialize)]
struct RpcRequest {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

fn rpc_ok(id: Option<Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn rpc_err(id: Option<Value>, code: i64, msg: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": msg },
    })
}

fn auth_ok(headers: &HeaderMap, token: &str) -> bool {
    let Some(h) = headers.get("authorization") else {
        return false;
    };
    let Ok(s) = h.to_str() else {
        return false;
    };
    let expected = format!("Bearer {token}");
    // Constant-time-ish comparison: tokens are ~44 chars, not critical here.
    s == expected
}

async fn handle_health() -> impl IntoResponse {
    (StatusCode::OK, "danbi-mcp")
}

#[derive(Deserialize, Default)]
struct SessionBriefQuery {
    /// Project name or UUID. Missing → vault-wide brief.
    project: Option<String>,
    max_chars: Option<usize>,
}

/// `GET|POST /api/session-brief?project=<name|uuid>&max_chars=N` — plain
/// Markdown (not JSON) because Claude Code pastes a SessionStart hook's
/// stdout straight into the session. No LLM call, so it stays fast.
async fn handle_session_brief(
    State(state): State<McpState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<SessionBriefQuery>,
) -> axum::response::Response {
    if !auth_ok(&headers, &state.token) {
        return (StatusCode::UNAUTHORIZED, "invalid or missing bearer token").into_response();
    }
    let max_chars = query
        .max_chars
        .unwrap_or(crate::session_hooks::DEFAULT_BRIEF_CHARS)
        .clamp(500, 20_000);
    let result = current_vault_path().and_then(|vault| {
        let project = match query.project.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(p) => Some(
                crate::vault::project_by_id(&vault, p)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| p.to_string()),
            ),
        };
        crate::session_hooks::session_brief(&vault, project.as_deref(), max_chars)
    });
    match result {
        Ok(text) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            text,
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, format!("{e}")).into_response(),
    }
}

/// `POST /api/session-end?project=<name|uuid>` — body is the SessionEnd
/// hook's stdin JSON (`session_id`, `transcript_path`, `cwd`, …). Answers
/// 202 at once so Claude Code can exit; the transcript is read, skipped
/// or summarised and logged in the background (see `session_hooks`).
async fn handle_session_end(
    State(state): State<McpState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<SessionBriefQuery>,
    body: String,
) -> (StatusCode, Json<Value>) {
    if !auth_ok(&headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "invalid or missing bearer token" })),
        );
    }
    let Some(project) = query.project.filter(|p| !p.trim().is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "project required" })),
        );
    };
    let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    tokio::spawn(crate::session_hooks::run_session_end(project, payload));
    (StatusCode::ACCEPTED, Json(json!({ "ok": true, "queued": true })))
}

// ---------- REST handlers ----------
//
// Plain JSON in / plain JSON out. Built so external schedulers and shell
// scripts can talk to Danbi without speaking JSON-RPC. The dispatcher,
// auth, and project-clamp logic are reused verbatim from the MCP path —
// these handlers just rewrap input/output.

/// `GET /api/tools` — list every tool with its schema. Same content as
/// MCP `tools/list` but unwrapped from the JSON-RPC envelope.
async fn handle_api_tools(
    State(state): State<McpState>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !auth_ok(&headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "invalid or missing bearer token" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "tools": tool_catalog() })),
    )
}

/// `POST /api/call/:tool` body = the tool's `arguments` object.
/// Returns `{ ok, result }` on success, `{ ok: false, error }` on failure.
async fn handle_api_call(
    State(state): State<McpState>,
    AxumPath(tool): AxumPath<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> (StatusCode, Json<Value>) {
    let args = body.map(|j| j.0).unwrap_or(Value::Null);
    run_api_call(&state, &headers, tool, args, None).await
}

/// `POST /api/projects/:project/call/:tool` — scoped variant.
/// `:project` may be a project name OR a UUID (matches the MCP scoped
/// route). Write tools have their `project` argument auto-clamped here,
/// just like the JSON-RPC scoped endpoint does.
async fn handle_api_call_scoped(
    State(state): State<McpState>,
    AxumPath((project, tool)): AxumPath<(String, String)>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> (StatusCode, Json<Value>) {
    let args = body.map(|j| j.0).unwrap_or(Value::Null);
    run_api_call(&state, &headers, tool, args, Some(project)).await
}

/// `PUT /v1/vault/{project}/{path...}` — Obsidian Local REST API style.
///
/// Body is raw markdown. Default = overwrite (write file fresh, or
/// replace existing). `?mode=append` adds to the end with a `\n\n`
/// separator. Folder prefixes are auto-created up to vault's depth cap.
///
/// Designed for: Paperclip routines, cron jobs, shell scripts. Anything
/// that already has a fully-formed report and just wants to drop it in.
async fn handle_v1_vault_put(
    State(state): State<McpState>,
    AxumPath((project, path)): AxumPath<(String, String)>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<V1VaultQuery>,
    body: String,
) -> (StatusCode, Json<Value>) {
    if !auth_ok(&headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "invalid or missing bearer token" })),
        );
    }

    // Validate path: non-empty, ends with .md, no traversal.
    let domain = path.trim_matches('/').to_string();
    if domain.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "empty path" })),
        );
    }
    if !domain.to_lowercase().ends_with(".md") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "path must end with .md" })),
        );
    }
    if domain.split('/').any(|seg| seg == ".." || seg == ".") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "path traversal not allowed" })),
        );
    }

    // Project resolution: accept either name or UUID. UUID lookup wins
    // when there's a hit; otherwise we treat the segment as the literal
    // project name.
    let vault_path = match current_vault_path() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("{e}") })),
            );
        }
    };
    let resolved_project = match crate::vault::project_by_id(&vault_path, &project) {
        Ok(Some(name)) => name,
        _ => project,
    };
    // Verify the project actually exists — the spec calls for 404 on
    // unknown projects rather than silently creating one.
    if let Err(e) = require_project(&vault_path, &resolved_project) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("{e}") })),
        );
    }

    let mode = query.mode.as_deref().unwrap_or("overwrite");
    if mode != "overwrite" && mode != "append" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": "mode must be 'overwrite' or 'append'"
            })),
        );
    }

    // Ensure parent folder exists. We rely on the existing tool's path
    // sanitization — it returns 422-ish errors via DanbiError::Config
    // when depth or segment rules are violated.
    if let Some(slash) = domain.rfind('/') {
        let folder = &domain[..slash];
        if let Err(e) = crate::vault::create_folder(&vault_path, &resolved_project, folder)
        {
            // Folder validation failures are policy violations (depth,
            // unsafe chars). Surface them as 422.
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "ok": false, "error": format!("{e}") })),
            );
        }
    }

    // Same lock as the MCP write tools — the append below reads then
    // writes, and the commit must not race another writer's.
    let _write_guard = vcs::write_guard();
    // Snapshot before edit so the user has an undo point.
    let _ = vcs::ensure_repo(&vault_path);
    let _ = vcs::snapshot(
        &vault_path,
        &format!("danbi: rest {mode} · {resolved_project}/{domain} (pre)"),
    );

    let final_bytes = if mode == "append" {
        let current = vault::read_doc(&vault_path, &resolved_project, &domain)
            .unwrap_or_default();
        // create_domain is a no-op if the file already exists — we just
        // need to make sure the path exists before write_doc.
        let _ = vault::create_domain(&vault_path, &resolved_project, &domain);
        let separator = if current.is_empty() { "" } else { "\n\n" };
        let next = format!("{current}{separator}{body}");
        if let Err(e) = vault::write_doc(&vault_path, &resolved_project, &domain, &next) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "ok": false, "error": format!("{e}") })),
            );
        }
        next.len()
    } else {
        // overwrite — replace the file's contents wholesale. write_doc
        // mkdir_p's the parent itself so we don't have to.
        let _ = vault::create_domain(&vault_path, &resolved_project, &domain);
        if let Err(e) = vault::write_doc(&vault_path, &resolved_project, &domain, &body) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "ok": false, "error": format!("{e}") })),
            );
        }
        body.len()
    };

    let (commit, warning) = commit_or_warn(
        &vault_path,
        &format!("danbi: rest {mode} · {resolved_project}/{domain}"),
    );

    // Token-track this write under a synthetic tool name so the
    // dashboard shows v1/vault PUT volume alongside the JSON-RPC
    // tools. The body itself is what got saved (overwrite) or
    // appended; either way `body` is what crossed the wire from the
    // external caller, which is what we want to count.
    //
    // Honours `usage.mcp_tracking` — when off, skip the recording.
    let tracking_on = config::load_config(&vault_path)
        .ok()
        .flatten()
        .map(|c| c.usage.mcp_tracking)
        .unwrap_or(true);
    if tracking_on {
        let user_agent = headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok());
        let client = crate::usage::classify_user_agent(user_agent);
        let tokens = crate::usage::estimate_tokens(&body);
        crate::usage::record_mcp_inbound(
            client,
            "v1_vault_put",
            Some(&resolved_project),
            Some(&domain),
            tokens,
            user_agent,
        );
    }

    let mut body = json!({
        "ok": true,
        "project": resolved_project,
        "domain": domain,
        "bytes": final_bytes,
        "mode": mode,
        "commit": commit,
    });
    if let Some(w) = warning {
        body["warning"] = Value::String(w);
    }
    (StatusCode::OK, Json(body))
}

#[derive(Deserialize, Default)]
struct V1VaultQuery {
    /// "overwrite" (default) or "append".
    mode: Option<String>,
}

async fn run_api_call(
    state: &McpState,
    headers: &HeaderMap,
    tool: String,
    mut args: Value,
    scoped_project_id: Option<String>,
) -> (StatusCode, Json<Value>) {
    if !auth_ok(headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "invalid or missing bearer token" })),
        );
    }

    // Resolve the scoped project the same way handle_rpc_scoped does:
    // accept either a UUID (project_by_id lookup) or, as a convenience
    // for REST callers, the bare project name.
    let scoped_project = match scoped_project_id.as_deref() {
        None => None,
        Some(id) => {
            let by_id = current_vault_path()
                .and_then(|v| crate::vault::project_by_id(&v, id));
            match by_id {
                Ok(Some(name)) => Some(name),
                _ => {
                    // Fall back: assume the path segment is the literal
                    // project name. Lets callers use either form without
                    // having to know the project's UUID.
                    Some(id.to_string())
                }
            }
        }
    };

    if let Some(proj) = scoped_project.as_ref() {
        clamp_or_default_project(&tool, &mut args, proj);
    }

    let inbound_meta = extract_inbound_meta(&tool, &args);
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    attach_client(&tool, &mut args, user_agent.as_deref());
    let args_for_banner = args.clone();

    // danbi_search 는 RRF 하이브리드 fast-path 로. 임베딩 provider 가
    // cfg 에 있으면 BM25 + 벡터 결과를 RRF 로 병합 (JSON-RPC 경로와 일관).
    // dispatch() 직호출 (BM25 only) 보다 자연어 쿼리 정확도가 훨씬 높아서
    // REST 호출자도 같은 품질을 받게.
    let tool_owned = tool.clone();
    // REST handler 는 이미 axum tokio task 안이라 block_on 하면
    // "Cannot start a runtime from within a runtime" panic — await 로.
    let dispatch_result: Result<Dispatched, DanbiError> = dispatch_any(&tool, args).await;
    match dispatch_result {
        Ok(Dispatched { text, warning }) => {
            let tool = &tool_owned;
            if let Some(meta) = inbound_meta.as_ref() {
                record_inbound_after_success(tool, meta, &text, user_agent.as_deref());
            }
            let text = inject_active_goals_into_text(
                text,
                tool,
                &args_for_banner,
                scoped_project.as_deref(),
            );
            // Most tools return a JSON document as a string. Try to
            // surface it as parsed JSON so REST consumers get structured
            // data without an extra parse step. If parsing fails we just
            // pass the raw string through.
            let parsed: Value =
                serde_json::from_str(&text).unwrap_or(Value::String(text));
            let mut body = json!({ "ok": true, "result": parsed });
            if let Some(w) = warning {
                body["warning"] = Value::String(w);
            }
            (StatusCode::OK, Json(body))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": format!("{e}") })),
        ),
    }
}

async fn handle_rpc(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> axum::response::Response {
    run_rpc(&state, &headers, req, None, ToolScope::All).await
}

async fn handle_rpc_scoped(
    State(state): State<McpState>,
    AxumPath(project_id): AxumPath<String>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> axum::response::Response {
    run_rpc(&state, &headers, req, Some(project_id), ToolScope::All).await
}

async fn handle_rpc_read(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> axum::response::Response {
    run_rpc(&state, &headers, req, None, ToolScope::ReadOnly).await
}

async fn handle_rpc_scoped_write(
    State(state): State<McpState>,
    AxumPath(project_id): AxumPath<String>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> axum::response::Response {
    run_rpc(&state, &headers, req, Some(project_id), ToolScope::WriteOnly).await
}

/// Which tools a JSON-RPC endpoint exposes. `All` is what `/mcp` and
/// `/mcp/<id>` have always served; the split endpoints let a machine
/// attach one read server globally plus one write server per code folder
/// instead of N full servers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ToolScope {
    All,
    ReadOnly,
    WriteOnly,
}

impl ToolScope {
    fn allows(self, tool: &str) -> bool {
        match self {
            ToolScope::All => true,
            ToolScope::ReadOnly => !is_write_tool(tool),
            ToolScope::WriteOnly => is_write_tool(tool),
        }
    }

    /// Error text for a tool this endpoint doesn't serve — tells the agent
    /// which endpoint to use instead.
    fn rejection(self, tool: &str) -> Option<String> {
        if self.allows(tool) {
            return None;
        }
        Some(match self {
            ToolScope::ReadOnly => format!(
                "{tool} 은(는) 쓰기 도구라 읽기 전용 연결(/mcp/read)에서는 쓸 수 없어요. 프로젝트별 쓰기 연결(/mcp/<project-id>/write)의 같은 도구로 호출하세요."
            ),
            _ => format!(
                "{tool} 은(는) 읽기 도구라 이 쓰기 전용 연결(/mcp/<project-id>/write)에는 없어요. 읽기 전용 연결(/mcp/read)의 같은 도구로 호출하세요."
            ),
        })
    }
}

/// Read tools whose optional `project` defaults to the endpoint's project
/// on a scoped server. Search stays vault-wide unless asked — cross-project
/// lookups are the main reason to search from inside one project.
const SCOPED_DEFAULT_PROJECT_TOOLS: [&str; 3] = ["danbi_briefing", "danbi_recent", "danbi_daily"];

/// Tool catalog for one endpoint. On a project-scoped server the write
/// tools lose their `project` parameter (the server fills it in — the
/// schema and the skill used to contradict each other here) and say so in
/// their description; the read tools that default to the project say that
/// too.
fn catalog_for_project(scope: ToolScope, scoped_project: Option<&str>) -> Vec<Value> {
    tool_catalog()
        .into_iter()
        .filter(|t| scope.allows(t.get("name").and_then(|n| n.as_str()).unwrap_or("")))
        .map(|mut t| {
            let Some(project) = scoped_project else { return t };
            let name = t["name"].as_str().unwrap_or("").to_string();
            if is_write_tool(&name) {
                if let Some(props) = t["inputSchema"]["properties"].as_object_mut() {
                    props.remove("project");
                }
                if let Some(req) = t["inputSchema"]["required"].as_array_mut() {
                    req.retain(|r| r != "project");
                }
                let desc = t["description"].as_str().unwrap_or("").to_string();
                t["description"] = json!(format!("→ {project} 에 기록. {desc}"));
            } else if SCOPED_DEFAULT_PROJECT_TOOLS.contains(&name.as_str()) {
                if let Some(p) = t["inputSchema"]["properties"]["project"].as_object_mut() {
                    p.insert("description".into(), json!(format!("생략하면 {project}")));
                }
            }
            t
        })
        .collect()
}

/// `initialize.instructions` — Claude Code and Codex put this string into
/// the system context, so the agent knows how to use the vault even when
/// no SKILL.md is installed. Kept to a handful of lines.
fn server_instructions(scope: ToolScope, scoped_project: Option<&str>) -> String {
    let mut lines = vec![
        "단비 vault 는 프로젝트의 외부 장기 기억 — 결정·버그 원인·TODO 가 세션을 넘어 누적됩니다. 일반론 대신 vault 의 맥락으로 답하세요.".to_string(),
    ];
    let read = scope != ToolScope::WriteOnly;
    let write = scope != ToolScope::ReadOnly;
    if read {
        lines.push(match scoped_project {
            Some(p) => format!("새 세션: danbi_briefing 먼저 (project 생략 시 {p}). 질문 전: danbi_context(query) 로 관련 섹션부터, 문서 전체는 danbi_read (heading·max_chars 로 부분 읽기)."),
            None => "새 세션: danbi_briefing(project) 먼저. 질문 전: danbi_context(query) 로 관련 섹션부터, 문서 전체는 danbi_read (heading·max_chars 로 부분 읽기).".to_string(),
        });
    } else {
        lines.push("읽기 도구(briefing·context·read·search)는 읽기 연결 /mcp/read 에 있습니다.".to_string());
    }
    if write {
        lines.push(match scoped_project {
            Some(p) => format!("기록: 결정·버그 원인·TODO 는 즉시 danbi_log(content) — 자동으로 {p} 의 오늘 daily 에 `#### 시각 · 호출자` 머리말과 함께 들어갑니다. `### 제목` + 2~4줄, 관련 문서는 [[파일.md]] 로 연결."),
            None => "기록: 결정·버그 원인·TODO 는 즉시 danbi_log(project, content) — 오늘 daily 에 `#### 시각 · 호출자` 머리말과 함께. `### 제목` + 2~4줄, 관련 문서는 [[파일.md]] 로 연결.".to_string(),
        });
        lines.push("특정 문서 갱신은 danbi_append(누적) · danbi_replace_section(섹션 교체) · danbi_upsert_item(리스트 항목). 같은 내용을 두 번 보내지 말 것 — 응답에 deduped:true 또는 warning 이 오면 이미 저장된 것이니 재시도 금지.".to_string());
    } else {
        lines.push("기록(danbi_log 등)은 프로젝트별 쓰기 연결 /mcp/<project-id>/write 에서 하세요.".to_string());
    }
    lines.join("\n")
}

async fn run_rpc(
    state: &McpState,
    headers: &HeaderMap,
    req: RpcRequest,
    scoped_project_id: Option<String>,
    scope: ToolScope,
) -> axum::response::Response {
    if req.jsonrpc != "2.0" {
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_err(req.id, -32600, "jsonrpc must be '2.0'")),
        )
            .into_response();
    }
    if !auth_ok(headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(rpc_err(req.id, -32000, "invalid or missing bearer token")),
        )
            .into_response();
    }

    // JSON-RPC notifications carry no `id` and MUST NOT receive a response
    // object. MCP Streamable HTTP requires the server to answer 202 Accepted
    // with an empty body. Returning a `{"id":null,...,"result":{}}` object
    // (as we used to for `notifications/initialized`) makes strict clients —
    // notably Codex's rmcp client — treat it as a protocol violation and drop
    // the whole session ("danbi: failed (0 tools)"). Handle every incoming
    // notification uniformly here, before method dispatch.
    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    // Resolve the scoped project (UUID → name) once per request so tool
    // handlers can trust a validated project name and never misroute.
    let scoped_project = match scoped_project_id.as_deref() {
        None => None,
        Some(id) => match current_vault_path()
            .and_then(|v| crate::vault::project_by_id(&v, id))
        {
            Ok(Some(name)) => Some(name),
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(rpc_err(
                        req.id,
                        -32001,
                        &format!("no project matches id: {id}"),
                    )),
                )
                    .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(rpc_err(
                        req.id,
                        -32003,
                        &format!("project lookup failed: {e}"),
                    )),
                )
                    .into_response();
            }
        },
    };

    let (status, body) = match req.method.as_str() {
        "initialize" => {
            let server_name = match (scoped_project.as_ref(), scope) {
                (Some(p), ToolScope::WriteOnly) => format!("{SERVER_NAME} · {p} · write"),
                (Some(p), _) => format!("{SERVER_NAME} · {p}"),
                (None, ToolScope::ReadOnly) => format!("{SERVER_NAME} · read"),
                (None, _) => SERVER_NAME.to_string(),
            };
            (
                StatusCode::OK,
                Json(rpc_ok(
                    req.id,
                    json!({
                        "protocolVersion": PROTOCOL_VERSION,
                        "serverInfo": { "name": server_name, "version": SERVER_VERSION },
                        "capabilities": {
                            "tools": { "listChanged": false },
                        },
                        "instructions": server_instructions(scope, scoped_project.as_deref()),
                    }),
                )),
            )
        }
        "notifications/initialized" => (
            StatusCode::OK,
            Json(rpc_ok(req.id, json!({}))),
        ),
        "tools/list" => (
            StatusCode::OK,
            Json(rpc_ok(
                req.id,
                json!({ "tools": catalog_for_project(scope, scoped_project.as_deref()) }),
            )),
        ),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut args = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            if let Some(msg) = scope.rejection(name) {
                return (
                    StatusCode::OK,
                    Json(rpc_ok(
                        req.id,
                        json!({
                            "content": [ { "type": "text", "text": format!("error: {msg}") } ],
                            "isError": true,
                        }),
                    )),
                )
                    .into_response();
            }
            // Project-scoped URLs only clamp WRITE tools to the owning
            // project — read tools are left free so an agent attached to
            // project A can still cross-reference project B. This preserves
            // the "no accidental writes" guarantee while letting a session
            // read the broader vault (e.g. "what's the status of the RAG
            // project?" asked from the i18n project's session).
            if let Some(proj) = scoped_project.as_ref() {
                clamp_or_default_project(name, &mut args, proj);
            }
            // Capture inbound metadata BEFORE we move `args` into
            // dispatch — write tools need their content for token
            // estimation but dispatch consumes the value. `None` for
            // read tools, which we don't track here.
            let inbound_meta = extract_inbound_meta(name, &args);
            let user_agent = headers
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            attach_client(name, &mut args, user_agent.as_deref());
            // Snapshot args before dispatch consumes them — banner
            // injection later needs the project hint from the args.
            let args_for_banner = args.clone();

            // danbi_search·danbi_context 는 RRF 하이브리드 경로로. 임베딩
            // provider 가 cfg 에 있으면 BM25 + 벡터 결과를 RRF 로 병합해서
            // 외부 AI 의 자연어 쿼리도 정확히 잡아낸다. 임베딩 없으면
            // 기존 BM25 만으로 동작.
            let result: Result<Dispatched, DanbiError> = dispatch_any(name, args).await;
            match result {
                Ok(Dispatched { text, warning }) => {
                    // Token-track *only* on success — failed writes
                    // never reach disk, so they shouldn't inflate
                    // saved-content metrics.
                    if let Some(meta) = inbound_meta.as_ref() {
                        record_inbound_after_success(
                            name,
                            meta,
                            &text,
                            user_agent.as_deref(),
                        );
                    }
                    let text = inject_active_goals_into_text(
                        text,
                        name,
                        &args_for_banner,
                        scoped_project.as_deref(),
                    );
                    let mut content = vec![json!({ "type": "text", "text": text })];
                    if let Some(w) = warning {
                        content.push(json!({ "type": "text", "text": format!("warning: {w}") }));
                    }
                    (
                    StatusCode::OK,
                    Json(rpc_ok(
                        req.id,
                        json!({
                            "content": content,
                            "isError": false,
                        }),
                    )),
                )
                }
                Err(e) => (
                    StatusCode::OK,
                    Json(rpc_ok(
                        req.id,
                        json!({
                            "content": [ { "type": "text", "text": format!("error: {e}") } ],
                            "isError": true,
                        }),
                    )),
                ),
            }
        }
        other => (
            StatusCode::OK,
            Json(rpc_err(
                req.id,
                -32601,
                &format!("method not found: {other}"),
            )),
        ),
    };
    (status, body).into_response()
}

// ---------- Tool catalog ----------

fn tool_catalog() -> Vec<Value> {
    vec![
        json!({
            "name": "danbi_list_projects",
            "description": "vault 의 프로젝트 목록. 기본은 프로젝트마다 한 줄(이름·파일 수·최근 수정·최상위 파일·폴더별 개수)이고, project 를 주면 그 프로젝트의 파일 트리 전체를 돌려줍니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "이 프로젝트의 파일 트리만 자세히" },
                    "include_daily": { "type": "boolean", "default": false, "description": "project 지정 시 daily/ 노트도 포함" },
                    "titles": { "type": "boolean", "default": false, "description": "project 지정 시 파일마다 첫 제목(title)·kind 도 읽어 포함 (느림)" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_search",
            "description": "vault 안에서 관련 도메인 파일을 찾습니다. tantivy BM25 + 한국어 n-gram 키워드 검색이 기본이고, 사용자가 임베딩 provider 를 설정해뒀으면 벡터 검색을 RRF 로 병합한 하이브리드 결과를 돌려줍니다. 자연어 쿼리도 잘 받습니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "검색어 (키워드 또는 자연어 질문)" },
                    "limit": { "type": "integer", "default": 8 },
                    "project": { "type": "string", "description": "이 프로젝트 안에서만. 생략하면 vault 전체" }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_context",
            "description": "질문 하나로 답에 필요한 맥락을 한 번에 가져옵니다. 하이브리드 검색 상위 문서에서 질문과 관련된 섹션만 글자 예산 안에서 잘라 주고, 그 문서들에 [[위키링크]]로 연결된 문서 목록도 같이 줍니다. search → read 를 여러 번 반복하는 대신 먼저 이걸 부르세요. 전체 본문이 필요하면 그다음 danbi_read.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "질문 또는 키워드" },
                    "limit": { "type": "integer", "default": 4, "description": "가져올 문서 수 (1~10)" },
                    "max_chars": { "type": "integer", "default": 8000, "description": "본문 발췌 총 글자 예산 (1000~40000)" }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_related",
            "description": "문서 하나와 연결된 모든 것을 돌려줍니다: 이 문서가 거는 [[링크]], 이 문서를 거는 백링크(다른 프로젝트 포함), 링크는 없지만 내용상 가까운 문서(soft·ghost, 점수순), 같은 주제 묶음(community) 문서. 읽은 문서에서 맥락을 넓힐 때 쓰세요.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "domain": { "type": "string", "description": ".md로 끝나는 파일명 (하위 폴더면 notes/x.md 형태)" },
                    "limit": { "type": "integer", "default": 10, "description": "related 최대 개수" }
                },
                "required": ["project", "domain"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_read",
            "description": "특정 프로젝트/도메인 파일의 markdown 본문. 첫 줄은 메타 주석(bytes·chars·modified·links_in). 큰 문서는 heading 으로 섹션만, 또는 max_chars/offset 으로 나눠 읽으세요 — 잘리면 메타 주석에 다음 offset 이 적혀 있습니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "domain": { "type": "string", "description": ".md로 끝나는 파일명 (하위 폴더면 notes/x.md 형태)" },
                    "heading": { "type": "string", "description": "이 markdown 헤딩의 섹션만 (예: \"## 결정\" 또는 \"결정\")" },
                    "max_chars": { "type": "integer", "description": "본문 최대 글자 수 (기본 전체, 최소 200)" },
                    "offset": { "type": "integer", "default": 0, "description": "본문 시작 글자 위치 — 이어 읽기용" }
                },
                "required": ["project", "domain"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_log",
            "description": "지정한 프로젝트의 오늘 daily/YYYY-MM-DD.md 파일에 새 문단을 append합니다. 시각·호출자 머리말이 자동으로 붙고, 같은 날 같은 본문을 다시 보내면 쓰지 않고 deduped:true 를 돌려줍니다. 응답의 commit 이 null 이고 warning 이 있으면 파일은 저장된 것이니 재시도하지 마세요.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "content": { "type": "string", "description": "추가할 markdown. `### 제목` + 2~4줄, 관련 문서는 [[파일.md]] 로 연결" },
                    "stamp": { "type": "boolean", "default": true, "description": "false 면 자동 머리말(`#### HH:MM · client`)을 붙이지 않음" }
                },
                "required": ["project", "content"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_append",
            "description": "지정한 프로젝트/도메인 파일에 markdown을 append합니다. 파일이 없으면 만듭니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "domain": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["project", "domain", "content"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_create_folder",
            "description": "프로젝트 안에 1~2단계 sub-folder 를 만듭니다 (예: stats, daily/2026-05). 이미 있으면 no-op. 만든 뒤 danbi_append(domain=\"<folder>/<file>.md\", …) 로 그 안에 파일을 채울 수 있어요.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "folder": { "type": "string", "description": "폴더 경로. 1~2단계 가능 (예: stats, daily/2026-05)" }
                },
                "required": ["project", "folder"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_create_file",
            "description": "폴더와 파일을 한 번에 보장합니다. 부모 폴더가 없으면 자동 생성하고 (1~2단계까지), 파일이 없으면 빈 파일로 만들고, content 가 있으면 거기에 그 내용을 씁니다 (덮어쓰기 아닌 append). danbi_create_folder + danbi_append 를 따로 부르는 대신 한 호출로 끝낼 때 쓰세요. 매일 통계 / 카테고리별 누적 같은 자동화에 적합합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "domain": {
                        "type": "string",
                        "description": "폴더 prefix 포함 가능 (예: stats/2026-05-17.md, daily/2026-05/17.md). .md 확장자는 자동 추가됨."
                    },
                    "content": {
                        "type": "string",
                        "description": "초기 / 추가 내용. 비워두면 빈 파일만 만듭니다."
                    }
                },
                "required": ["project", "domain"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_replace_section",
            "description": "지정한 markdown 헤딩(예: \"## 알림톡 리스트\") 아래 본문 전체를 새 내용으로 교체합니다. 헤딩 줄 자체는 유지되고 헤딩 직후부터 다음 동급/상위 헤딩 직전까지가 교체 대상입니다. 대상 헤딩이 없으면 파일 끝에 새 섹션을 추가합니다. 기존 항목이 변경될 때 append 로 누적 안 시키고 깔끔하게 갱신하고 싶은 리스트성 문서에 적합합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "domain": { "type": "string", "description": ".md 로 끝나는 도메인 파일명" },
                    "heading": {
                        "type": "string",
                        "description": "교체할 섹션의 markdown 헤딩 줄. 예: \"## 알림톡 리스트\" — `##` 포함 또는 텍스트만 둘 다 허용."
                    },
                    "new_body": {
                        "type": "string",
                        "description": "그 섹션 안에 들어갈 새 markdown 본문. 헤딩 줄은 다시 적지 말 것."
                    }
                },
                "required": ["project", "domain", "heading", "new_body"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_upsert_item",
            "description": "리스트 섹션 안의 한 항목을 in-place 교체하거나 (없으면) 추가합니다. 매칭은 1) 항목 첫 줄에 `[#id]` 마커가 있으면 그 id 정확 매치, 2) 없으면 항목 첫 줄 텍스트(공백/대소문자 무시) 매치. 같은 항목이 변경되어도 append 가 누적되지 않습니다. 알림톡 / 체크리스트 / 카드 리스트처럼 항목 단위로 진행 상태가 바뀌는 문서에 사용하세요.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "domain": { "type": "string", "description": ".md 로 끝나는 도메인 파일명" },
                    "heading": {
                        "type": "string",
                        "description": "항목이 속한 섹션의 markdown 헤딩. 섹션이 없으면 파일 끝에 만들어집니다."
                    },
                    "key": {
                        "type": "string",
                        "description": "항목 식별자. `[#abc]` 마커의 id (`abc`) 또는 항목 첫 줄 텍스트."
                    },
                    "item": {
                        "type": "string",
                        "description": "넣을 항목 markdown. 보통 `- ` 로 시작하는 bullet 한 덩어리. ID 마커를 쓰려면 `- [#abc] ...` 형태."
                    }
                },
                "required": ["project", "domain", "heading", "key", "item"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_recent",
            "description": "최근 수정된 문서 목록 (기본 10개, 하위 폴더 포함). daily 노트는 기본 제외.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "default": 10 },
                    "project": { "type": "string", "description": "이 프로젝트만. 생략하면 vault 전체" },
                    "exclude_daily": { "type": "boolean", "default": true, "description": "false 면 daily/ 노트도 포함" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_briefing",
            "description": "세션 시작용 상태 요약 한 JSON: 오늘 daily 노트, 7일 활동(기록 수·최근 요약), 정리 제안 건수와 상위 10건, 미확정 링크 제안. project 를 주면 그 프로젝트만 (훨씬 짧음).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "이 프로젝트만. 생략하면 vault 전체" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_daily",
            "description": "오늘 daily 노트 + 회상(1주·1개월·1년 전 같은 날짜 노트)을 반환합니다. '그때 비슷한 걸 뭐 적었지?' 류 질문에 쓰세요.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "이 프로젝트만. 생략하면 vault 전체" }
                },
                "additionalProperties": false
            }
        }),
    ]
}

// ---------- Goal banner injection -----------------------------------------
//
// Per-project goals surface inside MCP tool responses so external Claude
// sessions stay oriented. The banner is best-effort: if no project is
// identifiable from the args/scope/result, we skip injection. We also
// preserve compatibility — JSON arrays are returned untouched (clients
// like Claude Code parse arrays as array), JSON objects gain an
// `_active_goals` field, and raw markdown gets a single-line HTML
// comment prefix.

/// Identify the project(s) whose active goals should be surfaced for a
/// tool response. Order matters — we keep the args/scope source first
/// because that's the most authoritative ("you asked about project P").
fn identify_projects_for_banner(
    name: &str,
    args: &Value,
    scoped_project: Option<&str>,
    parsed_result: Option<&Value>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim();
        if s.is_empty() {
            return;
        }
        if !out.iter().any(|p| p == s) {
            out.push(s.to_string());
        }
    };
    if let Some(p) = scoped_project {
        push(p);
    }
    if let Some(p) = args.get("project").and_then(|v| v.as_str()) {
        push(p);
    }
    // For search/recent results, the top hit's project is a strong
    // signal — the user is clearly working in that area right now.
    if matches!(name, "danbi_search" | "danbi_recent") {
        if let Some(arr) = parsed_result.and_then(|v| v.as_array()) {
            if let Some(first) = arr.first() {
                if let Some(p) = first.get("project").and_then(|v| v.as_str()) {
                    push(p);
                }
            }
        }
    }
    out
}

fn inject_active_goals_into_text(
    text: String,
    name: &str,
    args: &Value,
    scoped_project: Option<&str>,
) -> String {
    let vault_path = match current_vault_path() {
        Ok(p) => p,
        Err(_) => return text,
    };
    // Try parsing — most tools return JSON. read returns raw markdown.
    let parsed: Option<Value> = serde_json::from_str(&text).ok();
    let projects =
        identify_projects_for_banner(name, args, scoped_project, parsed.as_ref());
    if projects.is_empty() {
        return text;
    }
    // Collect (project, [titles]) once per project; skip projects with no
    // active goals so the field isn't noise on the wire.
    let mut by_project: Vec<(String, Vec<String>)> = Vec::new();
    for p in &projects {
        let titles = crate::goals::active_titles(&vault_path, p);
        if !titles.is_empty() {
            by_project.push((p.clone(), titles));
        }
    }
    if by_project.is_empty() {
        return text;
    }
    let goals_value = json!(
        by_project
            .iter()
            .map(|(p, titles)| json!({ "project": p, "titles": titles }))
            .collect::<Vec<_>>()
    );

    match parsed {
        Some(Value::Object(mut map)) => {
            map.insert("_active_goals".into(), goals_value);
            serde_json::to_string_pretty(&Value::Object(map)).unwrap_or(text)
        }
        // Arrays stay arrays for backward-compat — clients that read
        // hits[i] from the response shouldn't suddenly get an object.
        Some(Value::Array(_)) => text,
        // Raw markdown (read tool) — one-line HTML comment prefix so the
        // model sees the goals without breaking the markdown body.
        _ => {
            let banner_lines: Vec<String> = by_project
                .iter()
                .map(|(p, titles)| {
                    format!("danbi · [{}] active goals: {}", p, titles.join(" / "))
                })
                .collect();
            format!("<!-- {} -->\n{}", banner_lines.join(" | "), text)
        }
    }
}

// ---------- Tool dispatcher ----------

/// `danbi_search` 만의 async 처리 경로. 임베딩 provider 가 cfg 에 설정돼
/// 있으면 BM25 + 벡터 검색을 RRF 로 병합해서 외부 AI 의 자연어 쿼리도
/// 정확히 잡아낸다. rate-limit / network 실패 시 BM25 결과로 자동
/// fallback — Claude Code 가 절대 빈 결과를 받지 않게.
async fn search_hybrid_dispatch(args: Value) -> Result<Dispatched, DanbiError> {
    let vault_path = current_vault_path()?;
    let query = string_arg(&args, "query")?;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(8) as usize;
    let project = opt_string_arg(&args, "project");
    if let Some(p) = project.as_deref() {
        require_project(&vault_path, p)?;
    }

    let embedding = query_embedding(&vault_path, &query).await?;
    // A project filter is applied after fusion, so ask for more.
    let depth = if project.is_some() { limit.saturating_mul(4) } else { limit };
    let mut hits =
        search::full_search_hybrid(&vault_path, &query, depth, embedding.as_deref())?;
    if let Some(p) = project.as_deref() {
        hits.retain(|h| h.project == p);
        hits.truncate(limit);
    }
    Ok(Dispatched {
        text: serde_json::to_string_pretty(&hits)?,
        warning: semantic_arm_warning(&vault_path, embedding.is_some()),
    })
}

/// A tool result plus an optional warning that travels outside the
/// result body: `danbi_search` returns a bare array (clients index into
/// it), so the mixed-model notice goes out as a second MCP content block
/// / a top-level REST field instead of changing the array's shape.
struct Dispatched {
    text: String,
    warning: Option<String>,
}

/// When the semantic arm ran against a mixed index, say how many docs it
/// could not see (bug #6: cosine over mismatched dimensions is a silent 0).
fn semantic_arm_warning(vault_path: &std::path::Path, used_embedding: bool) -> Option<String> {
    if !used_embedding {
        return None;
    }
    let counts = crate::vector::cached_model_counts(vault_path);
    if counts.is_empty() {
        return None;
    }
    let configured = config::load_config(vault_path)
        .ok()
        .flatten()
        .and_then(|cfg| configured_embed_model(&cfg));
    let status = crate::vector::model_status(&counts, configured.as_deref());
    status.mixed.then(|| {
        format!(
            "의미 검색 결과에서 문서 {}개가 빠졌어요 (다른 임베딩 모델로 색인됨 — 키워드 검색에는 잡힘). 단비 설정 → AI 연동에서 전체 재색인이 필요해요.",
            status.other_count
        )
    })
}

/// Routes the tools that need async work (query embedding) and falls
/// through to the sync `dispatch` for everything else. Both the JSON-RPC
/// and REST paths call this so they can't drift apart.
async fn dispatch_any(name: &str, args: Value) -> Result<Dispatched, DanbiError> {
    match name {
        "danbi_search" => search_hybrid_dispatch(args).await,
        "danbi_context" => context_dispatch(args).await,
        _ => dispatch(name, args).map(|text| Dispatched { text, warning: None }),
    }
}

/// cfg 의 embed provider 로 query 를 임베딩한다. 키 없거나 호출 실패
/// 면 None — `full_search_hybrid` 가 BM25 only 로 graceful fallback.
async fn query_embedding(
    vault_path: &std::path::Path,
    query: &str,
) -> Result<Option<Vec<f32>>, DanbiError> {
    Ok(match crate::config::load_config(vault_path)? {
        Some(c) => crate::commands::embed_query_for_search(&c, query).await,
        None => None,
    })
}

const CONTEXT_MAX_LINKED: usize = 10;

/// `danbi_context`: hybrid search → per-hit focused excerpt within a char
/// budget → wiki-link neighbours of the hits. One call replaces the usual
/// search → read → read chain.
async fn context_dispatch(args: Value) -> Result<Dispatched, DanbiError> {
    let vault_path = current_vault_path()?;
    let query = string_arg(&args, "query")?;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(4)
        .clamp(1, 10) as usize;
    let budget = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(8000)
        .clamp(1000, 40000) as usize;

    let embedding = query_embedding(&vault_path, &query).await?;
    let hits =
        search::full_search_hybrid(&vault_path, &query, limit, embedding.as_deref())?;

    let per_doc = budget / hits.len().max(1);
    let mut used = 0usize;
    let mut docs = Vec::with_capacity(hits.len());
    for hit in &hits {
        let md = vault::read_doc(&vault_path, &hit.project, &hit.domain)?;
        let (excerpt, complete) = search::focused_excerpt(&md, &query, per_doc);
        used += excerpt.chars().count();
        docs.push(json!({
            "project": hit.project,
            "domain": hit.domain,
            "matched": hit.matched,
            "relevance": hit.relevance,
            "total_chars": md.chars().count(),
            "complete": complete,
            "excerpt": excerpt,
        }));
    }

    let hit_keys: std::collections::HashSet<String> = hits
        .iter()
        .map(|h| format!("{}/{}", h.project, h.domain))
        .collect();
    let index = crate::links::build_index(&vault_path)?;
    let existing = crate::graph::existing_doc_ids(&vault_path)?;
    let mut seen = hit_keys.clone();
    let mut linked = Vec::new();
    'outer: for key in hits.iter().map(|h| format!("{}/{}", h.project, h.domain)) {
        let out = index.outgoing.get(&key).into_iter().flatten();
        let inc = index.incoming.get(&key).into_iter().flatten();
        for (dir, l) in out.map(|l| ("links_to", l)).chain(inc.map(|l| ("linked_from", l))) {
            let k = format!("{}/{}", l.project, l.domain);
            if !existing.contains(&k) || !seen.insert(k) {
                continue;
            }
            linked.push(json!({
                "project": l.project,
                "domain": l.domain,
                "via": key,
                "direction": dir,
            }));
            if linked.len() >= CONTEXT_MAX_LINKED {
                break 'outer;
            }
        }
    }

    let mut out = json!({
        "query": query,
        "used_chars": used,
        "docs": docs,
        "linked": linked,
        "hint": "전체 본문은 danbi_read, 한 문서의 주변 연결은 danbi_related",
    });
    // Object result → the warning fits inside it.
    if let Some(w) = semantic_arm_warning(&vault_path, embedding.is_some()) {
        out["warning"] = Value::String(w);
    }
    Ok(Dispatched {
        text: serde_json::to_string_pretty(&out)?,
        warning: None,
    })
}

pub(crate) fn dispatch(name: &str, args: Value) -> Result<String, DanbiError> {
    let vault_path = current_vault_path()?;
    dispatch_in(&vault_path, name, args)
}

/// `dispatch` against an explicit vault — what the tests drive.
fn dispatch_in(vault_path: &std::path::Path, name: &str, args: Value) -> Result<String, DanbiError> {
    let vault_path = vault_path.to_path_buf();
    // One writer at a time across MCP, REST and the popover: the read →
    // edit → write → commit sequence below must not interleave.
    let _write_guard = is_write_tool(name).then(vcs::write_guard);

    match name {
        "danbi_list_projects" => {
            let project = opt_string_arg(&args, "project");
            let include_daily = bool_arg(&args, "include_daily", false);
            let titles = bool_arg(&args, "titles", false);
            let out = list_projects(&vault_path, project.as_deref(), include_daily, titles)?;
            Ok(rows_json(&out))
        }
        "danbi_search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DanbiError::Config("query required".into()))?
                .to_string();
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(8) as usize;
            let hits = search::full_search(&vault_path, &query, limit)?;
            Ok(serde_json::to_string_pretty(&hits)?)
        }
        "danbi_read" => {
            let project = string_arg(&args, "project")?;
            let domain = string_arg(&args, "domain")?;
            let heading = opt_string_arg(&args, "heading");
            let max_chars = args
                .get("max_chars")
                .and_then(|v| v.as_u64())
                .map(|n| (n as usize).max(200));
            let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            read_doc_tool(&vault_path, &project, &domain, heading.as_deref(), max_chars, offset)
        }
        "danbi_related" => {
            let project = string_arg(&args, "project")?;
            let mut domain = string_arg(&args, "domain")?;
            if !domain.to_lowercase().ends_with(".md") {
                domain.push_str(".md");
            }
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .clamp(1, 50) as usize;
            let out = crate::graph::related(&vault_path, &project, &domain, limit)?;
            Ok(serde_json::to_string_pretty(&out)?)
        }
        "danbi_log" => {
            let project = string_arg(&args, "project")?;
            let content = string_arg(&args, "content")?;
            require_project(&vault_path, &project)?;
            let body = content.trim();
            if body.is_empty() {
                return Err(DanbiError::Config(
                    "content 가 비어 있어요. 기록할 markdown 을 주세요 (예: \"### 결정\\n- JWT refresh 7일\").".into(),
                ));
            }
            let domain = daily::ensure_today_note(&vault_path, &project)?;
            let current = vault::read_doc(&vault_path, &project, &domain)?;
            // Same body twice on the same day (an agent retrying, two
            // agents told the same thing) → acknowledge, don't duplicate.
            if log_is_duplicate(&current, body) {
                return Ok(json!({
                    "project": project,
                    "domain": domain,
                    "deduped": true,
                    "commit": Value::Null,
                    "bytes": current.len(),
                })
                .to_string());
            }
            let stamp = args.get("stamp").and_then(|v| v.as_bool()).unwrap_or(true);
            let client = args.get("_client").and_then(|v| v.as_str());
            let entry = if stamp {
                format!("{}\n{body}", log_stamp(Local::now().format("%H:%M").to_string(), client))
            } else {
                body.to_string()
            };
            let op = EditOp::Append { content: entry };
            edit_ops::validate(&op)?;
            let next = edit_ops::apply(&current, &op)?;

            vcs::ensure_repo(&vault_path)?;
            let _ = vcs::snapshot(&vault_path, &format!("danbi: mcp log · {project}/{domain} (pre)"));
            vault::write_doc(&vault_path, &project, &domain, &next)?;
            let (commit, warning) = commit_or_warn(
                &vault_path,
                &format!("danbi: mcp log · {project}/{domain}"),
            );
            Ok(write_result(
                json!({
                    "project": project,
                    "domain": domain,
                    "stamped": stamp,
                    "bytes": next.len(),
                }),
                commit,
                warning,
            ))
        }
        "danbi_append" => {
            let project = string_arg(&args, "project")?;
            let domain = string_arg(&args, "domain")?;
            let content = string_arg(&args, "content")?;
            require_project(&vault_path, &project)?;
            let domain_norm = normalize_md(&domain);
            let current =
                vault::read_doc(&vault_path, &project, &domain_norm).unwrap_or_default();
            let op = EditOp::Append { content };
            edit_ops::validate(&op)?;
            let next = edit_ops::apply(&current, &op)?;

            vcs::ensure_repo(&vault_path)?;
            let _ = vcs::snapshot(
                &vault_path,
                &format!("danbi: mcp append · {project}/{domain_norm} (pre)"),
            );
            // Create-on-demand so /mcp writers can target new files.
            let _ = vault::create_domain(&vault_path, &project, &domain_norm);
            vault::write_doc(&vault_path, &project, &domain_norm, &next)?;
            let (commit, warning) = commit_or_warn(
                &vault_path,
                &format!("danbi: mcp append · {project}/{domain_norm}"),
            );
            Ok(write_result(
                json!({
                    "project": project,
                    "domain": domain_norm,
                    "bytes": next.len(),
                }),
                commit,
                warning,
            ))
        }
        "danbi_replace_section" => {
            let project = string_arg(&args, "project")?;
            let domain = string_arg(&args, "domain")?;
            let heading = string_arg(&args, "heading")?;
            let new_body = string_arg(&args, "new_body")?;
            require_project(&vault_path, &project)?;
            let domain_norm = normalize_md(&domain);
            let current =
                vault::read_doc(&vault_path, &project, &domain_norm).unwrap_or_default();
            let op = EditOp::ReplaceSection {
                heading: heading.clone(),
                new_body,
            };
            edit_ops::validate(&op)?;
            let next = edit_ops::apply(&current, &op)?;

            vcs::ensure_repo(&vault_path)?;
            let _ = vcs::snapshot(
                &vault_path,
                &format!(
                    "danbi: mcp replace_section · {project}/{domain_norm} (pre)"
                ),
            );
            let _ = vault::create_domain(&vault_path, &project, &domain_norm);
            vault::write_doc(&vault_path, &project, &domain_norm, &next)?;
            let (commit, warning) = commit_or_warn(
                &vault_path,
                &format!(
                    "danbi: mcp replace_section · {project}/{domain_norm} · {heading}"
                ),
            );
            Ok(write_result(
                json!({
                    "project": project,
                    "domain": domain_norm,
                    "heading": heading,
                    "bytes": next.len(),
                }),
                commit,
                warning,
            ))
        }
        "danbi_upsert_item" => {
            let project = string_arg(&args, "project")?;
            let domain = string_arg(&args, "domain")?;
            let heading = string_arg(&args, "heading")?;
            let key = string_arg(&args, "key")?;
            let item = string_arg(&args, "item")?;
            require_project(&vault_path, &project)?;
            let domain_norm = normalize_md(&domain);
            let current =
                vault::read_doc(&vault_path, &project, &domain_norm).unwrap_or_default();
            // Detect whether the key already lives in the section so we can
            // tell the caller whether this was an in-place update or an add.
            let was_replace = edit_ops::upsert_item_would_replace(&current, &heading, &key);
            let op = EditOp::UpsertItem {
                heading: heading.clone(),
                key: key.clone(),
                item,
            };
            edit_ops::validate(&op)?;
            let next = edit_ops::apply(&current, &op)?;

            vcs::ensure_repo(&vault_path)?;
            let mode = if was_replace { "update" } else { "add" };
            let _ = vcs::snapshot(
                &vault_path,
                &format!(
                    "danbi: mcp upsert_item · {project}/{domain_norm} (pre)"
                ),
            );
            let _ = vault::create_domain(&vault_path, &project, &domain_norm);
            vault::write_doc(&vault_path, &project, &domain_norm, &next)?;
            let (commit, warning) = commit_or_warn(
                &vault_path,
                &format!(
                    "danbi: mcp upsert_item · {project}/{domain_norm} · {mode} · {key}"
                ),
            );
            Ok(write_result(
                json!({
                    "project": project,
                    "domain": domain_norm,
                    "heading": heading,
                    "key": key,
                    "mode": mode,
                    "bytes": next.len(),
                }),
                commit,
                warning,
            ))
        }
        "danbi_create_file" => {
            // Convenience for "make sure this folder + file exist, here's
            // the content". Combines create_folder + create_domain +
            // append into one round-trip. The agent doesn't need to know
            // whether the folder/file already exists.
            let project = string_arg(&args, "project")?;
            let domain = string_arg(&args, "domain")?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            require_project(&vault_path, &project)?;
            let domain_norm = normalize_md(&domain);

            vcs::ensure_repo(&vault_path)?;
            let _ = vcs::snapshot(
                &vault_path,
                &format!("danbi: mcp create_file · {project}/{domain_norm} (pre)"),
            );

            // Step 1: ensure intermediate folder(s) (idempotent — pulls
            // the folder prefix out of the domain).
            if let Some(slash) = domain_norm.rfind('/') {
                let folder = &domain_norm[..slash];
                vault::create_folder(&vault_path, &project, folder)?;
            }

            // Step 2: probe + ensure the file. `read_doc` returns "" for
            // missing files (not an error), so we check the actual fs path
            // through create_domain — it errors with "already exists" when
            // the file is there, which is the signal we want to flip into
            // `already_existed = true`.
            let create_result = vault::create_domain(&vault_path, &project, &domain_norm);
            let already_existed = create_result.is_err();
            // Surface unexpected errors (anything other than "already
            // exists"). The error string is fixed in vault.rs, so a
            // substring match is fine.
            if let Err(ref e) = create_result {
                let msg = format!("{e}");
                if !msg.contains("already exists") {
                    return Err(create_result.unwrap_err());
                }
            }

            // Step 3: append content if any. Empty content = leave the
            // file as-is so re-running the tool is a true no-op.
            if !content.is_empty() {
                let current =
                    vault::read_doc(&vault_path, &project, &domain_norm).unwrap_or_default();
                let op = EditOp::Append { content };
                edit_ops::validate(&op)?;
                let next = edit_ops::apply(&current, &op)?;
                vault::write_doc(&vault_path, &project, &domain_norm, &next)?;
            }

            let (commit, warning) = commit_or_warn(
                &vault_path,
                &format!("danbi: mcp create_file · {project}/{domain_norm}"),
            );
            let final_bytes = vault::read_doc(&vault_path, &project, &domain_norm)
                .unwrap_or_default()
                .len();
            Ok(write_result(
                json!({
                    "project": project,
                    "domain": domain_norm,
                    "bytes": final_bytes,
                    "already_existed": already_existed,
                }),
                commit,
                warning,
            ))
        }
        "danbi_create_folder" => {
            let project = string_arg(&args, "project")?;
            let folder = string_arg(&args, "folder")?;
            require_project(&vault_path, &project)?;
            vcs::ensure_repo(&vault_path)?;
            let _ = vcs::snapshot(
                &vault_path,
                &format!("danbi: mcp create_folder · {project}/{folder} (pre)"),
            );
            vault::create_folder(&vault_path, &project, &folder)?;
            // Drop a hidden marker so git tracks the otherwise-empty folder.
            let placeholder_path = vault_path
                .join("Projects")
                .join(&project)
                .join(&folder)
                .join(".gitkeep");
            if !placeholder_path.exists() {
                let _ = std::fs::write(&placeholder_path, b"");
            }
            let (commit, warning) = commit_or_warn(
                &vault_path,
                &format!("danbi: mcp create_folder · {project}/{folder}"),
            );
            Ok(write_result(
                json!({
                    "project": project,
                    "folder": folder,
                }),
                commit,
                warning,
            ))
        }
        "danbi_recent" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            let project = opt_string_arg(&args, "project");
            let exclude_daily = bool_arg(&args, "exclude_daily", true);
            let tree = vault::list_tree_with(&vault_path, false)?;
            let mut all: Vec<(String, String, u128)> = Vec::new();
            for p in &tree.projects {
                if project.as_deref().map(|want| want != p.name).unwrap_or(false) {
                    continue;
                }
                for d in flatten_domains(p) {
                    if exclude_daily && is_daily_domain(&d.name) {
                        continue;
                    }
                    all.push((
                        p.name.clone(),
                        d.name.clone(),
                        d.modified_ms.unwrap_or(0),
                    ));
                }
            }
            all.sort_by(|a, b| b.2.cmp(&a.2));
            let trimmed: Vec<Value> = all
                .into_iter()
                .take(limit)
                .map(|(p, d, ts)| json!({ "project": p, "domain": d, "modified_ms": ts }))
                .collect();
            Ok(serde_json::to_string_pretty(&trimmed)?)
        }
        "danbi_briefing" => {
            let project = opt_string_arg(&args, "project");
            if let Some(p) = project.as_deref() {
                require_project(&vault_path, p)?;
            }
            let mut snap = dashboard::briefing(&vault_path, project.as_deref())?;
            snap.vector_warning = vector_mixed_warning(&vault_path);
            Ok(rows_json(&serde_json::to_value(&snap)?))
        }
        "danbi_daily" => {
            let project = opt_string_arg(&args, "project");
            let mut snap = daily::snapshot(&vault_path)?;
            if let Some(p) = project.as_deref() {
                require_project(&vault_path, p)?;
                for list in [
                    &mut snap.today_notes,
                    &mut snap.one_week_ago,
                    &mut snap.one_month_ago,
                    &mut snap.one_year_ago,
                ] {
                    list.retain(|n| n.project == p);
                }
            }
            Ok(serde_json::to_string_pretty(&snap)?)
        }
        other => Err(DanbiError::Config(format!("unknown tool: {other}"))),
    }
}

/// Write tools mutate vault state and must stay clamped to the scoped
/// project. Read tools return information only and are safe to cross
/// project boundaries.
pub(crate) fn is_write_tool(name: &str) -> bool {
    WRITE_TOOLS.contains(&name)
}

pub(crate) const WRITE_TOOLS: [&str; 6] = [
    "danbi_log",
    "danbi_append",
    "danbi_create_folder",
    "danbi_create_file",
    "danbi_replace_section",
    "danbi_upsert_item",
];

/// Snapshot of a write-tool invocation that we'll need *after* dispatch
/// has consumed `args`. We pull these out before dispatch so the post-
/// success token bookkeeping doesn't have to re-parse the request body.
struct InboundMeta {
    project: Option<String>,
    domain: Option<String>,
    /// Pre-tokenized content for this call. Empty for `danbi_create_folder`.
    content: String,
}

/// Extract the project/domain/content needed for inbound token tracking.
/// Returns `None` for read tools (we don't track those) and for malformed
/// requests where the args we'd need are missing — better to skip the
/// metric than to crash on bad input.
fn extract_inbound_meta(tool: &str, args: &Value) -> Option<InboundMeta> {
    if !is_write_tool(tool) {
        return None;
    }
    let project = args
        .get("project")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let domain = args
        .get("domain")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // Different write tools name the payload field differently. We grab
    // whichever is present so token accounting reflects what crossed the
    // wire — `content` (log/append/create_file), `new_body`
    // (replace_section), or `item` (upsert_item).
    let content = args
        .get("content")
        .or_else(|| args.get("new_body"))
        .or_else(|| args.get("item"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(InboundMeta {
        project,
        domain,
        content,
    })
}

/// Record an MCP inbound write event after `dispatch` succeeded.
///
/// `dispatch_text` is the JSON string the tool returned — for
/// `danbi_log`, the daily note's filename gets resolved server-side and
/// only shows up there, not in the request args. We look it up so the
/// per-domain breakdown still attributes the write to the right file.
///
/// Honours the `usage.mcp_tracking` config flag — when off, this is a
/// no-op so the user can fully disable inbound tracking without losing
/// a release.
fn record_inbound_after_success(
    tool: &str,
    meta: &InboundMeta,
    dispatch_text: &str,
    user_agent: Option<&str>,
) {
    // Tracking opt-out: read the config inline. Cheap (file is tiny)
    // and avoids threading state into every dispatch call. If the
    // config can't be loaded we err on the side of recording — better
    // a stray data point than a silent gap.
    if let Ok(vault) = config::default_vault_path() {
        if let Ok(Some(cfg)) = config::load_config(&vault) {
            if !cfg.usage.mcp_tracking {
                return;
            }
        }
    }
    // For `danbi_log` the request only carries the project — the domain
    // (`daily/YYYY-MM-DD.md`) is decided by the dispatcher. Pull it back
    // from the structured response so the dashboard can attribute.
    let resolved_domain: Option<String> = if tool == "danbi_log" {
        serde_json::from_str::<Value>(dispatch_text)
            .ok()
            .and_then(|v| {
                v.get("domain")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| meta.domain.clone())
    } else {
        meta.domain.clone()
    };

    let tokens = crate::usage::estimate_tokens(&meta.content);
    let client = crate::usage::classify_user_agent(user_agent);
    crate::usage::record_mcp_inbound(
        client,
        tool,
        meta.project.as_deref(),
        resolved_domain.as_deref(),
        tokens,
        user_agent,
    );
}

// ---------- Read helpers ----------

/// JSON for list-shaped tool output: objects stay indented so the agent
/// can see the structure, but every array of objects is written one
/// compact row per line. Pretty-printing 30 project rows costs ~3× the
/// characters for no extra information.
fn rows_json(v: &Value) -> String {
    fn render(v: &Value, indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        match v {
            Value::Object(map) if !map.is_empty() => {
                out.push_str("{\n");
                let n = map.len();
                for (i, (k, val)) in map.iter().enumerate() {
                    out.push_str(&pad);
                    out.push_str("  ");
                    out.push_str(&Value::String(k.clone()).to_string());
                    out.push_str(": ");
                    render(val, indent + 2, out);
                    out.push_str(if i + 1 < n { ",\n" } else { "\n" });
                }
                out.push_str(&pad);
                out.push('}');
            }
            Value::Array(arr) if arr.iter().any(|x| x.is_object()) => {
                out.push_str("[\n");
                let n = arr.len();
                for (i, row) in arr.iter().enumerate() {
                    out.push_str(&pad);
                    out.push_str("  ");
                    out.push_str(&row.to_string());
                    out.push_str(if i + 1 < n { ",\n" } else { "\n" });
                }
                out.push_str(&pad);
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut out = String::new();
    render(v, 0, &mut out);
    out
}

/// On a project-scoped endpoint: write tools are always clamped to the
/// project (whatever the caller sent), the session-oriented read tools
/// get it as a default when `project` is absent.
fn clamp_or_default_project(tool: &str, args: &mut Value, project: &str) {
    let clamp = is_write_tool(tool);
    let default = SCOPED_DEFAULT_PROJECT_TOOLS.contains(&tool);
    if !clamp && !default {
        return;
    }
    if !args.is_object() {
        *args = Value::Object(serde_json::Map::new());
    }
    let map = args.as_object_mut().expect("object");
    if clamp || !map.get("project").and_then(|v| v.as_str()).is_some_and(|s| !s.trim().is_empty()) {
        map.insert("project".into(), Value::String(project.to_string()));
    }
}

fn opt_string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn bool_arg(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn is_daily_domain(domain: &str) -> bool {
    domain.starts_with(&format!("{}/", daily::DAILY_DIRNAME))
}

/// Every markdown file of a project, top-level and nested subfolders.
fn flatten_domains(p: &vault::ProjectNode) -> Vec<&vault::DomainNode> {
    fn walk<'a>(subs: &'a [vault::SubfolderNode], out: &mut Vec<&'a vault::DomainNode>) {
        for s in subs {
            out.extend(s.domains.iter());
            walk(&s.subfolders, out);
        }
    }
    let mut out: Vec<&vault::DomainNode> = p.domains.iter().collect();
    walk(&p.subfolders, &mut out);
    out
}

/// `danbi_list_projects`. Default: one short row per project (name, file
/// count, last change, top-level file names, per-folder counts). With
/// `project`: that project's full file tree, daily notes only on
/// `include_daily`, heading sniff only on `titles`.
fn list_projects(
    vault_path: &std::path::Path,
    project: Option<&str>,
    include_daily: bool,
    titles: bool,
) -> Result<Value, DanbiError> {
    let tree = vault::list_tree_with(vault_path, titles && project.is_some())?;
    let Some(want) = project else {
        let rows: Vec<Value> = tree
            .projects
            .iter()
            .map(|p| {
                let docs = flatten_domains(p);
                let mut folders = serde_json::Map::new();
                fn count(subs: &[vault::SubfolderNode], out: &mut serde_json::Map<String, Value>) {
                    for s in subs {
                        let n: usize = s.domains.len();
                        if n > 0 || !s.subfolders.is_empty() {
                            out.insert(s.name.clone(), json!(n));
                        }
                        count(&s.subfolders, out);
                    }
                }
                count(&p.subfolders, &mut folders);
                json!({
                    "name": p.name,
                    "files": docs.len(),
                    "last_modified_ms": docs.iter().filter_map(|d| d.modified_ms).max(),
                    "top_level": p.domains.iter().map(|d| d.name.clone()).collect::<Vec<_>>(),
                    "folders": folders,
                })
            })
            .collect();
        return Ok(json!({
            "projects": rows,
            "hint": "한 프로젝트의 파일 전체는 danbi_list_projects(project=\"이름\", include_daily?, titles?)",
        }));
    };
    let node = tree
        .projects
        .iter()
        .find(|p| p.name == want)
        .ok_or_else(|| unknown_project_error(vault_path, want))?;
    let mut node = node.clone();
    if !include_daily {
        node.subfolders.retain(|s| !is_daily_domain(&format!("{}/", s.name)));
    }
    let files = flatten_domains(&node).len();
    let daily_count = flatten_domains(node_ref(&tree, want)).iter().filter(|d| is_daily_domain(&d.name)).count();
    let mut out = serde_json::to_value(&node)?;
    if let Value::Object(ref mut map) = out {
        map.insert("files".into(), json!(files));
        map.insert("daily_notes".into(), json!(daily_count));
        if !include_daily && daily_count > 0 {
            map.insert("hint".into(), json!("daily 노트는 include_daily=true 로, 오늘·회상은 danbi_daily(project)"));
        }
    }
    Ok(out)
}

fn node_ref<'a>(tree: &'a vault::VaultTree, name: &str) -> &'a vault::ProjectNode {
    tree.projects.iter().find(|p| p.name == name).expect("checked above")
}

/// One-line warning when the vector index mixes embedding models, None
/// when it is consistent or there is no index at all.
fn vector_mixed_warning(vault_path: &std::path::Path) -> Option<String> {
    let counts = crate::vector::cached_model_counts(vault_path);
    if counts.is_empty() {
        return None;
    }
    let configured = config::load_config(vault_path)
        .ok()
        .flatten()
        .and_then(|cfg| configured_embed_model(&cfg));
    let status = crate::vector::model_status(&counts, configured.as_deref());
    crate::vector::mixed_warning(&status)
}

/// The embedding model a query would use right now, None when no embed
/// provider/model is configured.
pub(crate) fn configured_embed_model(cfg: &DanbiConfig) -> Option<String> {
    let provider = crate::commands::resolve_embed_provider(cfg).ok()?;
    let model = crate::commands::resolve_embed_model(cfg, provider.as_ref(), None);
    (!model.trim().is_empty()).then_some(model)
}

/// `danbi_read`: metadata comment + (a section of) the document, cut to a
/// character window. Missing files are an error with name suggestions
/// rather than an empty string the agent would mistake for an empty doc.
fn read_doc_tool(
    vault_path: &std::path::Path,
    project: &str,
    domain: &str,
    heading: Option<&str>,
    max_chars: Option<usize>,
    offset: usize,
) -> Result<String, DanbiError> {
    require_project(vault_path, project)?;
    let domain_norm = normalize_md(domain);
    let Some((md, meta)) = vault::read_doc_opt(vault_path, project, &domain_norm)? else {
        return Err(doc_not_found_error(vault_path, project, &domain_norm));
    };

    // Section cut first, then the char window applies to the section.
    let (body, section_note) = match heading {
        None => (md.as_str(), String::new()),
        Some(h) => {
            let lines: Vec<&str> = md.lines().collect();
            match edit_ops::locate_section(&lines, h) {
                Some((start, end)) => {
                    let section = line_span(&md, start, end);
                    (section, format!(" · section \"{}\"", h.trim()))
                }
                None => {
                    let headings: Vec<String> = lines
                        .iter()
                        .filter(|l| l.starts_with('#'))
                        .map(|l| l.trim().to_string())
                        .take(20)
                        .collect();
                    return Err(DanbiError::Config(format!(
                        "heading not found: {h}. 이 문서의 헤딩: {}",
                        if headings.is_empty() { "(없음)".to_string() } else { headings.join(" | ") }
                    )));
                }
            }
        }
    };

    let total = body.chars().count();
    let start = offset.min(total);
    let take = max_chars.unwrap_or(usize::MAX);
    let window: String = body.chars().skip(start).take(take).collect();
    let end = start + window.chars().count();
    let range_note = if start > 0 || end < total {
        let more = if end < total {
            format!(", 이어 읽기: offset={end}")
        } else {
            String::new()
        };
        format!(" · chars {start}..{end} of {total}{more}")
    } else {
        String::new()
    };

    let modified = meta
        .modified()
        .ok()
        .map(|t| chrono::DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "?".into());
    let links_in = crate::links::build_index(vault_path)
        .map(|idx| {
            idx.incoming
                .get(&format!("{project}/{domain_norm}"))
                .map(|v| v.len())
                .unwrap_or(0)
        })
        .unwrap_or(0);
    Ok(format!(
        "<!-- danbi: {project}/{domain_norm} · {} bytes · {} chars · modified {modified} · links_in {links_in}{section_note}{range_note} -->\n{window}",
        meta.len(),
        md.chars().count(),
    ))
}

/// Text of lines `[start, end)` of `md`, newlines preserved.
fn line_span(md: &str, start: usize, end: usize) -> &str {
    let mut offsets = Vec::new();
    let mut pos = 0usize;
    for line in md.split_inclusive('\n') {
        offsets.push(pos);
        pos += line.len();
    }
    offsets.push(pos);
    let a = offsets.get(start).copied().unwrap_or(md.len());
    let b = offsets.get(end).copied().unwrap_or(md.len());
    &md[a..b]
}

fn doc_not_found_error(vault_path: &std::path::Path, project: &str, domain: &str) -> DanbiError {
    let names: Vec<String> = vault::list_tree_with(vault_path, false)
        .ok()
        .and_then(|t| t.projects.into_iter().find(|p| p.name == project))
        .map(|p| flatten_domains(&p).iter().map(|d| d.name.clone()).collect())
        .unwrap_or_default();
    let similar = closest_names(domain, &names, 3);
    let mut msg = format!("not found: {project}/{domain}.");
    if !similar.is_empty() {
        msg.push_str(&format!(" 혹시 {}?", similar.join(" / ")));
    }
    msg.push_str(&format!(
        " 파일 목록은 danbi_list_projects(project=\"{project}\"), 새로 만들려면 danbi_create_file."
    ));
    DanbiError::Config(msg)
}

// ---------- Write helpers ----------

/// `danbi_log` stamps each entry with the calling client; the name is
/// derived from the User-Agent here and passed down as a private arg so
/// the sync dispatcher stays header-free.
fn attach_client(tool: &str, args: &mut Value, user_agent: Option<&str>) {
    if tool != "danbi_log" {
        return;
    }
    if let Value::Object(map) = args {
        let client = crate::usage::classify_user_agent(user_agent);
        map.insert("_client".into(), Value::String(client.to_string()));
    }
}

fn normalize_md(domain: &str) -> String {
    if domain.to_lowercase().ends_with(".md") {
        domain.to_string()
    } else {
        format!("{domain}.md")
    }
}

/// Existence check by directory, not `list_tree` (which opens every
/// markdown file). Unknown names come back with the closest matches and
/// the full list so the agent can fix a typo without another round trip.
fn require_project(vault: &std::path::Path, project: &str) -> Result<(), DanbiError> {
    if vault::project_exists(vault, project) {
        return Ok(());
    }
    Err(unknown_project_error(vault, project))
}

fn unknown_project_error(vault: &std::path::Path, project: &str) -> DanbiError {
    let names = vault::project_names(vault).unwrap_or_default();
    let similar = closest_names(project, &names, 3);
    let mut msg = format!("unknown project: {project}.");
    if !similar.is_empty() {
        msg.push_str(&format!(" 혹시 {}?", similar.join(" / ")));
    }
    if names.is_empty() {
        msg.push_str(" 등록된 프로젝트가 없어요 — 단비 앱에서 먼저 만드세요.");
    } else {
        msg.push_str(&format!(" 등록된 프로젝트: {}", names.join(", ")));
    }
    DanbiError::Config(msg)
}

/// Up to `max` candidates ranked by edit distance (case-insensitive),
/// substring matches first. Only names within a reasonable distance of
/// the query are returned.
fn closest_names(query: &str, names: &[String], max: usize) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .filter_map(|n| {
            let lc = n.to_lowercase();
            if lc.contains(&q) || q.contains(&lc) {
                return Some((0, n));
            }
            let d = edit_distance(&q, &lc);
            let allowed = (q.chars().count().max(lc.chars().count()) / 2).max(2);
            (d <= allowed).then_some((d, n))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
    scored.into_iter().take(max).map(|(_, n)| n.clone()).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Post-edit commit. The file is already on disk by now, so a git failure
/// (index.lock held by another process, odd permissions) is reported as a
/// warning on the successful response — never as an error that would make
/// the agent retry and duplicate the write.
fn commit_or_warn(vault: &std::path::Path, msg: &str) -> (Option<String>, Option<String>) {
    match vcs::snapshot(vault, msg) {
        Ok(c) => (c, None),
        Err(e) => (
            None,
            Some(format!(
                "파일은 저장됐지만 git 커밋에 실패했어요 ({e}). 재시도하지 마세요 — 다음 쓰기 때 함께 커밋돼요."
            )),
        ),
    }
}

fn write_result(mut body: Value, commit: Option<String>, warning: Option<String>) -> String {
    if let Value::Object(ref mut map) = body {
        map.insert("commit".into(), commit.map(Value::String).unwrap_or(Value::Null));
        if let Some(w) = warning {
            map.insert("warning".into(), Value::String(w));
        }
    }
    body.to_string()
}

/// `#### HH:MM · <client>` heading for a `danbi_log` entry, so a daily
/// note several agents wrote stays readable for a human.
fn log_stamp(hhmm: String, client: Option<&str>) -> String {
    match client.filter(|c| !c.is_empty() && *c != "unknown") {
        Some(c) => format!("#### {hhmm} · {}", c.replace('_', "-")),
        None => format!("#### {hhmm}"),
    }
}

/// Minimum body length for dedupe — anything shorter ("ok", "done") is
/// too generic to treat a repeat as a retry.
const LOG_DEDUPE_MIN_CHARS: usize = 8;

fn log_is_duplicate(current: &str, body: &str) -> bool {
    body.chars().count() >= LOG_DEDUPE_MIN_CHARS && current.contains(body)
}

fn string_arg(args: &Value, key: &str) -> Result<String, DanbiError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            let example = match key {
                "project" => "\"보니\" (목록: danbi_list_projects)",
                "domain" => "\"notes/auth.md\"",
                "query" => "\"refresh token 만료 처리\"",
                "content" => "\"### 결정\\n- JWT refresh 7일\"",
                "heading" => "\"## 결정\"",
                "new_body" => "\"- 항목 1\\n- 항목 2\"",
                "key" => "\"abc\" ([#abc] 마커) 또는 항목 첫 줄 텍스트",
                "item" => "\"- [#abc] 알림톡 발송 — 완료\"",
                "folder" => "\"notes\" 또는 \"stats/2026-05\"",
                _ => "",
            };
            if example.is_empty() {
                DanbiError::Config(format!("{key} required"))
            } else {
                DanbiError::Config(format!("{key} required (예: {example})"))
            }
        })
}

pub(crate) fn current_vault_path() -> Result<PathBuf, DanbiError> {
    let vault = config::default_vault_path()?;
    let cfg = config::load_config(&vault)?
        .ok_or_else(|| DanbiError::Config("vault not configured".into()))?;
    let vault_path = cfg
        .vault_path
        .ok_or_else(|| DanbiError::Config("vault path missing".into()))?;
    Ok(PathBuf::from(vault_path))
}

// ---------- Token helpers ----------

pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    // URL-safe base64 without padding, via the engine from the `base64` crate
    // (already a dependency).
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn ensure_token(cfg: &DanbiConfig) -> String {
    if cfg.mcp.token.is_empty() {
        generate_token()
    } else {
        cfg.mcp.token.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(scope: ToolScope) -> Vec<String> {
        catalog_for_project(scope, None)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn read_scope_lists_only_read_tools() {
        let read = names(ToolScope::ReadOnly);
        assert!(read.iter().all(|n| !is_write_tool(n)));
        for n in ["danbi_search", "danbi_context", "danbi_read", "danbi_related"] {
            assert!(read.iter().any(|r| r == n), "{n} missing");
        }
    }

    #[test]
    fn write_scope_lists_exactly_the_write_tools() {
        let mut write = names(ToolScope::WriteOnly);
        let mut expected: Vec<String> = WRITE_TOOLS.iter().map(|s| s.to_string()).collect();
        write.sort();
        expected.sort();
        assert_eq!(write, expected);
    }

    #[test]
    fn all_scope_is_unchanged_and_partitions_into_read_plus_write() {
        assert_eq!(names(ToolScope::All).len(), tool_catalog().len());
        assert_eq!(
            names(ToolScope::ReadOnly).len() + names(ToolScope::WriteOnly).len(),
            tool_catalog().len()
        );
        assert!(ToolScope::All.rejection("danbi_log").is_none());
        assert!(ToolScope::All.rejection("danbi_search").is_none());
    }

    #[test]
    fn rejections_point_to_the_other_endpoint() {
        let r = ToolScope::ReadOnly.rejection("danbi_log").unwrap();
        assert!(r.contains("/write"));
        assert!(ToolScope::ReadOnly.rejection("danbi_search").is_none());
        let w = ToolScope::WriteOnly.rejection("danbi_search").unwrap();
        assert!(w.contains("/mcp/read"));
        assert!(ToolScope::WriteOnly.rejection("danbi_append").is_none());
    }

    async fn call(scope: ToolScope, body: Value) -> Value {
        let state = McpState { token: Arc::new("t".into()) };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer t".parse().unwrap());
        let req: RpcRequest = serde_json::from_value(body).unwrap();
        let resp = run_rpc(&state, &headers, req, None, scope).await;
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn read_endpoint_filters_list_and_rejects_writes_before_dispatch() {
        let list = call(
            ToolScope::ReadOnly,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        )
        .await;
        let tools = list["result"]["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        assert!(tools.iter().all(|t| !is_write_tool(t["name"].as_str().unwrap())));

        let rejected = call(
            ToolScope::ReadOnly,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": "danbi_log", "arguments": { "project": "p", "content": "x" } } }),
        )
        .await;
        assert_eq!(rejected["result"]["isError"], true);
        let text = rejected["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("/mcp/<project-id>/write"), "{text}");

        let init = call(
            ToolScope::ReadOnly,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "initialize" }),
        )
        .await;
        assert_eq!(init["result"]["serverInfo"]["name"], "danbi · read");
        assert!(init["result"]["instructions"].as_str().unwrap().contains("danbi_context"));
    }

    // ---------- write path ----------

    fn temp_vault(label: &str) -> std::path::PathBuf {
        let unique = format!(
            "danbi-mcp-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(path.join("Projects/보니/notes")).unwrap();
        std::fs::create_dir_all(path.join("Projects/상식이")).unwrap();
        vault::init_vault(&path).unwrap();
        path
    }

    fn log(vault: &std::path::Path, project: &str, content: &str) -> Value {
        let out = dispatch_in(
            vault,
            "danbi_log",
            json!({ "project": project, "content": content, "_client": "claude_code" }),
        )
        .unwrap();
        serde_json::from_str(&out).unwrap()
    }

    fn today_note(vault: &std::path::Path, project: &str) -> String {
        let domain = format!("daily/{}.md", chrono::Local::now().format("%Y-%m-%d"));
        vault::read_doc(vault, project, &domain).unwrap()
    }

    #[test]
    fn log_stamps_client_and_time_and_dedupes_same_body() {
        let vault = temp_vault("log");
        let first = log(&vault, "보니", "### 결정\n- JWT refresh 7일");
        assert_eq!(first["stamped"], true);
        assert!(first["commit"].is_string(), "{first}");
        assert!(first.get("warning").is_none());
        let note = today_note(&vault, "보니");
        assert!(note.contains("· claude-code\n### 결정\n- JWT refresh 7일"), "{note}");
        assert!(note.lines().any(|l| l.starts_with("#### ") && l.contains(" · claude-code")));

        let again = log(&vault, "보니", "  ### 결정\n- JWT refresh 7일\n");
        assert_eq!(again["deduped"], true);
        assert!(again["commit"].is_null());
        assert_eq!(today_note(&vault, "보니"), note, "no second entry");

        // Short bodies are never deduped (too generic to be a retry).
        log(&vault, "보니", "ok");
        log(&vault, "보니", "ok");
        assert_eq!(today_note(&vault, "보니").matches("\nok\n").count(), 2);

        // stamp:false writes the body verbatim.
        let out = dispatch_in(
            &vault,
            "danbi_log",
            json!({ "project": "상식이", "content": "plain body here", "stamp": false }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["stamped"], false);
        let note = today_note(&vault, "상식이");
        assert!(note.ends_with("\nplain body here\n"), "{note}");
        assert!(!note.contains("#### "));
    }

    #[test]
    fn log_stamp_formats() {
        assert_eq!(log_stamp("14:32".into(), Some("claude_code")), "#### 14:32 · claude-code");
        assert_eq!(log_stamp("14:32".into(), Some("unknown")), "#### 14:32");
        assert_eq!(log_stamp("14:32".into(), None), "#### 14:32");
    }

    #[test]
    fn unknown_project_lists_similar_and_all_names() {
        let vault = temp_vault("unknown");
        let err = dispatch_in(&vault, "danbi_log", json!({ "project": "보니앨범", "content": "x" }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown project: 보니앨범"), "{err}");
        assert!(err.contains("혹시 보니?"), "{err}");
        assert!(err.contains("등록된 프로젝트: 보니, 상식이"), "{err}");
        // A file was not created for the unknown project.
        assert!(!vault.join("Projects/보니앨범").exists());

        assert_eq!(closest_names("bony", &["Bonny".into(), "Zed".into()], 3), vec!["Bonny"]);
        assert!(closest_names("zzzzzz", &["Bonny".into()], 3).is_empty());
    }

    #[test]
    fn commit_failure_is_a_warning_not_an_error() {
        let vault = temp_vault("lock");
        log(&vault, "보니", "first entry to init repo");
        // Simulate another git process holding the index.
        std::fs::write(vault.join(".git/index.lock"), b"").unwrap();
        let v = log(&vault, "보니", "second entry while locked");
        assert!(v["commit"].is_null(), "{v}");
        assert!(v["warning"].as_str().unwrap().contains("커밋"), "{v}");
        assert!(today_note(&vault, "보니").contains("second entry while locked"), "saved anyway");
        std::fs::remove_file(vault.join(".git/index.lock")).unwrap();
        // The next successful write commits the earlier one along.
        let v = log(&vault, "보니", "third entry after unlock");
        assert!(v["commit"].is_string());
    }

    #[test]
    fn concurrent_logs_never_lose_an_entry() {
        let vault = temp_vault("race");
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let vault = vault.clone();
                std::thread::spawn(move || {
                    for j in 0..5 {
                        log(&vault, "보니", &format!("entry from writer {i} round {j}"));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let note = today_note(&vault, "보니");
        for i in 0..6 {
            for j in 0..5 {
                assert!(note.contains(&format!("entry from writer {i} round {j}")), "{i}/{j}");
            }
        }
    }

    #[test]
    fn append_creates_file_and_returns_commit() {
        let vault = temp_vault("append");
        let out = dispatch_in(
            &vault,
            "danbi_append",
            json!({ "project": "보니", "domain": "notes/auth", "content": "- refresh 7d" }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["domain"], "notes/auth.md");
        assert!(v["commit"].is_string());
        assert_eq!(vault::read_doc(&vault, "보니", "notes/auth.md").unwrap(), "- refresh 7d\n");
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;

    fn temp_vault(label: &str) -> std::path::PathBuf {
        let unique = format!(
            "danbi-mcp-read-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).unwrap();
        vault::init_vault(&path).unwrap();
        path
    }

    fn write(vault: &std::path::Path, project: &str, domain: &str, body: &str) {
        let p = vault.join("Projects").join(project).join(domain);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// 30 projects × (4 top-level + 20 daily + 6 notes) files, every doc an
    /// orphan — the shape that blew the old outputs past 250k chars.
    fn big_vault() -> std::path::PathBuf {
        let vault = temp_vault("big");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        for i in 0..30 {
            let p = format!("proj{i:02}");
            for f in ["purpose.md", "schema.md", "SKILL.md", "ui.md"] {
                write(&vault, &p, f, &format!("# {f}\n\n{}", "본문 ".repeat(40)));
            }
            for d in 0..20 {
                write(&vault, &p, &format!("daily/2026-09-{:02}.md", d + 1), "# day\n\n## 결정\n- x\n");
            }
            write(&vault, &p, &format!("daily/{today}.md"), "# today\n\n## 오늘\n- 작업\n");
            for n in 0..6 {
                write(&vault, &p, &format!("notes/topic-{n}.md"), &format!("# topic {n}\n\n{}", "내용 ".repeat(60)));
            }
        }
        vault
    }

    #[test]
    fn list_projects_default_is_short_and_detail_is_opt_in() {
        let vault = big_vault();
        let out = dispatch_in(&vault, "danbi_list_projects", json!({})).unwrap();
        assert!(out.chars().count() < 5_000, "{} chars", out.chars().count());
        let v: Value = serde_json::from_str(&out).unwrap();
        let rows = v["projects"].as_array().unwrap();
        assert_eq!(rows.len(), 30);
        assert_eq!(rows[0]["files"], 31);
        assert_eq!(rows[0]["folders"]["daily"], 21);
        assert_eq!(rows[0]["folders"]["notes"], 6);
        assert!(rows[0].get("domains").is_none(), "no per-file rows by default");

        let out = dispatch_in(&vault, "danbi_list_projects", json!({ "project": "proj03" })).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["name"], "proj03");
        assert_eq!(v["daily_notes"], 21);
        assert_eq!(v["files"], 10, "daily excluded by default");
        assert!(v["subfolders"].as_array().unwrap().iter().all(|s| s["name"] != "daily"));
        assert!(v["domains"][0].get("title").is_none(), "titles only on request");

        let out = dispatch_in(
            &vault,
            "danbi_list_projects",
            json!({ "project": "proj03", "include_daily": true, "titles": true }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["files"], 31);
        assert_eq!(v["domains"][0]["title"], "purpose.md");

        let err = dispatch_in(&vault, "danbi_list_projects", json!({ "project": "proj3" }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("혹시 proj03"), "{err}");
    }

    #[test]
    fn briefing_is_a_few_kilobytes_and_scopes_to_a_project() {
        let vault = big_vault();
        vcs::ensure_repo(&vault).unwrap();
        for i in 0..40 {
            write(&vault, "proj01", "ui.md", &format!("# ui {i}\n"));
            vcs::snapshot(&vault, &format!("danbi: mcp append · proj01/ui.md (pre)")).unwrap();
            write(&vault, "proj01", "ui.md", &format!("# ui {i} done\n"));
            vcs::snapshot(&vault, &format!("danbi: mcp append · proj01/ui.md")).unwrap();
        }
        let out = dispatch_in(&vault, "danbi_briefing", json!({})).unwrap();
        // Worst case: every one of the 30 projects has a note today.
        assert!(out.chars().count() < 5_500, "{} chars", out.chars().count());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["daily"]["today_notes"].as_array().unwrap().len(), 30);
        // 30 × 6 notes + ui.md are orphans; scaffolding never is.
        assert_eq!(v["healing"]["orphans"], 30 * 6);
        assert_eq!(v["healing"]["top"].as_array().unwrap().len(), dashboard::BRIEFING_TOP);
        assert_eq!(v["activity"]["commit_count"], 40 + 1, "(pre) excluded, init included");
        assert_eq!(v["activity"]["recent_summaries"].as_array().unwrap().len(), dashboard::BRIEFING_SUMMARIES);
        assert!(v.get("vector_warning").is_none());

        let out = dispatch_in(&vault, "danbi_briefing", json!({ "project": "proj05" })).unwrap();
        assert!(out.chars().count() < 2_000, "{} chars", out.chars().count());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["project"], "proj05");
        assert_eq!(v["daily"]["today_notes"].as_array().unwrap().len(), 1);
        assert_eq!(v["healing"]["orphans"], 6);
        assert_eq!(v["activity"]["commit_count"], 0);

        assert!(dispatch_in(&vault, "danbi_briefing", json!({ "project": "nope" })).is_err());
    }

    #[test]
    fn recent_walks_subfolders_and_hides_daily_by_default() {
        let vault = temp_vault("recent");
        write(&vault, "a", "ui.md", "x");
        write(&vault, "a", "notes/n.md", "x");
        write(&vault, "a", "daily/2026-10-01.md", "x");
        write(&vault, "b", "plan.md", "x");
        let v: Value = serde_json::from_str(&dispatch_in(&vault, "danbi_recent", json!({})).unwrap()).unwrap();
        let names: Vec<String> = v.as_array().unwrap().iter().map(|h| format!("{}/{}", h["project"].as_str().unwrap(), h["domain"].as_str().unwrap())).collect();
        assert!(names.contains(&"a/notes/n.md".to_string()));
        assert!(!names.iter().any(|n| n.contains("daily/")));
        let v: Value = serde_json::from_str(
            &dispatch_in(&vault, "danbi_recent", json!({ "project": "a", "exclude_daily": false })).unwrap(),
        )
        .unwrap();
        let names: Vec<String> = v.as_array().unwrap().iter().map(|h| h["domain"].as_str().unwrap().to_string()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"daily/2026-10-01.md".to_string()));
    }

    #[test]
    fn read_returns_meta_sections_and_windows() {
        let vault = temp_vault("read");
        let md = "# Auth\n\n## 결정\n- refresh 7d\n- access 1h\n\n## 미정\n- rotation\n";
        write(&vault, "a", "notes/auth.md", md);
        write(&vault, "a", "ui.md", "see [[notes/auth.md]]\n");

        let out = dispatch_in(&vault, "danbi_read", json!({ "project": "a", "domain": "notes/auth" })).unwrap();
        let (head, body) = out.split_once('\n').unwrap();
        assert!(head.starts_with("<!-- danbi: a/notes/auth.md · "), "{head}");
        assert!(head.contains(&format!("{} bytes", md.len())));
        assert!(head.contains("links_in 1"), "{head}");
        assert!(!head.contains("chars 0.."), "no range note when whole doc");
        assert_eq!(body, md);

        let out = dispatch_in(&vault, "danbi_read", json!({ "project": "a", "domain": "notes/auth.md", "heading": "결정" })).unwrap();
        let (head, body) = out.split_once('\n').unwrap();
        assert!(head.contains("section \"결정\""));
        assert_eq!(body, "## 결정\n- refresh 7d\n- access 1h\n\n");

        let out = dispatch_in(&vault, "danbi_read", json!({ "project": "a", "domain": "notes/auth.md", "max_chars": 200, "offset": 2 })).unwrap();
        let (head, body) = out.split_once('\n').unwrap();
        // max_chars floors at 200, which here covers the rest of the doc.
        assert!(head.contains(&format!("chars 2..{} of {}", md.chars().count(), md.chars().count())), "{head}");
        assert_eq!(body, &md[2..]);

        let err = dispatch_in(&vault, "danbi_read", json!({ "project": "a", "domain": "notes/auth.md", "heading": "없음" })).unwrap_err().to_string();
        assert!(err.contains("heading not found") && err.contains("## 결정"), "{err}");

        let err = dispatch_in(&vault, "danbi_read", json!({ "project": "a", "domain": "notes/auth-flow.md" })).unwrap_err().to_string();
        assert!(err.contains("not found: a/notes/auth-flow.md"), "{err}");
        assert!(err.contains("혹시 notes/auth.md"), "{err}");
        assert!(err.contains("danbi_list_projects(project=\"a\")"));

        let err = dispatch_in(&vault, "danbi_read", json!({ "project": "a" })).unwrap_err().to_string();
        assert!(err.contains("domain required (예:"), "{err}");
    }

    #[test]
    fn rows_json_keeps_structure_but_flattens_rows() {
        let v = json!({ "a": [ { "x": 1 }, { "y": [1, 2] } ], "b": { "c": [1, 2], "d": "s" }, "e": [] });
        let out = rows_json(&v);
        assert_eq!(
            out,
            "{\n  \"a\": [\n    {\"x\":1},\n    {\"y\":[1,2]}\n  ],\n  \"b\": {\n    \"c\": [1,2],\n    \"d\": \"s\"\n  },\n  \"e\": []\n}"
        );
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), v);
    }

    #[test]
    fn line_span_covers_ranges() {
        let md = "a\nb\nc";
        assert_eq!(line_span(md, 0, 1), "a\n");
        assert_eq!(line_span(md, 1, 3), "b\nc");
        assert_eq!(line_span(md, 2, 9), "c");
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    fn by_name(cat: &[Value], name: &str) -> Value {
        cat.iter().find(|t| t["name"] == name).unwrap().clone()
    }

    #[test]
    fn scoped_catalog_drops_project_from_write_tools_only() {
        let cat = catalog_for_project(ToolScope::All, Some("보니"));
        let log = by_name(&cat, "danbi_log");
        assert!(log["inputSchema"]["properties"].get("project").is_none());
        assert_eq!(log["inputSchema"]["required"], json!(["content"]));
        assert!(log["description"].as_str().unwrap().starts_with("→ 보니 에 기록."));
        let upsert = by_name(&cat, "danbi_upsert_item");
        assert_eq!(upsert["inputSchema"]["required"], json!(["domain", "heading", "key", "item"]));

        // Read tools keep `project`; the session ones say it defaults.
        let read = by_name(&cat, "danbi_read");
        assert_eq!(read["inputSchema"]["required"], json!(["project", "domain"]));
        let brief = by_name(&cat, "danbi_briefing");
        assert_eq!(brief["inputSchema"]["properties"]["project"]["description"], "생략하면 보니");
        let search = by_name(&cat, "danbi_search");
        assert!(search["inputSchema"]["properties"]["project"]["description"].as_str().unwrap().contains("vault 전체"));

        // Unscoped catalog is untouched.
        let cat = catalog_for_project(ToolScope::All, None);
        assert_eq!(by_name(&cat, "danbi_log")["inputSchema"]["required"], json!(["project", "content"]));
    }

    #[test]
    fn write_only_scoped_catalog_is_exactly_the_write_tools_without_project() {
        let cat = catalog_for_project(ToolScope::WriteOnly, Some("보니"));
        assert_eq!(cat.len(), WRITE_TOOLS.len());
        for t in &cat {
            assert!(t["inputSchema"]["properties"].get("project").is_none(), "{}", t["name"]);
        }
        let cat = catalog_for_project(ToolScope::ReadOnly, None);
        assert!(cat.iter().all(|t| !is_write_tool(t["name"].as_str().unwrap())));
        assert_eq!(cat.len() + WRITE_TOOLS.len(), tool_catalog().len());
    }

    #[test]
    fn clamp_overrides_writes_and_defaults_session_reads() {
        let mut args = json!({ "project": "다른곳", "content": "x" });
        clamp_or_default_project("danbi_log", &mut args, "보니");
        assert_eq!(args["project"], "보니");

        let mut args = json!({});
        clamp_or_default_project("danbi_briefing", &mut args, "보니");
        assert_eq!(args["project"], "보니");
        let mut args = json!({ "project": "상식이" });
        clamp_or_default_project("danbi_daily", &mut args, "보니");
        assert_eq!(args["project"], "상식이", "explicit read scope wins");

        let mut args = json!({ "query": "q" });
        clamp_or_default_project("danbi_search", &mut args, "보니");
        assert!(args.get("project").is_none(), "search stays vault-wide");
        let mut args = Value::Null;
        clamp_or_default_project("danbi_append", &mut args, "보니");
        assert_eq!(args["project"], "보니");
    }

    #[test]
    fn instructions_fit_the_scope() {
        let all = server_instructions(ToolScope::All, Some("보니"));
        assert!(all.lines().count() <= 5, "{all}");
        assert!(all.contains("danbi_briefing 먼저 (project 생략 시 보니)"));
        assert!(all.contains("danbi_log(content)"));
        let read = server_instructions(ToolScope::ReadOnly, None);
        assert!(read.contains("/mcp/<project-id>/write"));
        assert!(!read.contains("danbi_log(project"));
        let write = server_instructions(ToolScope::WriteOnly, Some("보니"));
        assert!(write.contains("/mcp/read"));
        assert!(write.contains("보니 의 오늘 daily"));
    }
}

/// Read-only measurements against the user's real vault — run by hand:
/// `cargo test --lib real_vault_sizes -- --ignored --nocapture`.
#[cfg(test)]
mod real_vault {
    use super::*;

    #[test]
    #[ignore]
    fn real_vault_sizes() {
        let home = std::env::var("HOME").unwrap();
        let vault = std::path::PathBuf::from(home).join("Danbi_Vault");
        if !vault.exists() {
            return;
        }
        let t = std::time::Instant::now();
        let tree = vault::list_tree(&vault).unwrap();
        let tree_t = t.elapsed();
        let old_list = serde_json::to_string_pretty(&tree).unwrap();
        println!("list_tree: {:?}, old list_projects json: {} chars", tree_t, old_list.chars().count());
        let t = std::time::Instant::now();
        let snap = dashboard::snapshot(&vault).unwrap();
        let snap_t = t.elapsed();
        let old_brief = serde_json::to_string_pretty(&snap).unwrap();
        println!("dashboard::snapshot: {:?}, old briefing json: {} chars (healing {}, ghost {}, commits {})",
            snap_t, old_brief.chars().count(), snap.healing.len(), snap.ghost_suggestions.len(), snap.activity.commit_count);
        let t = std::time::Instant::now();
        let _ = crate::healing::scan(&vault).unwrap();
        println!("healing::scan: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let _ = crate::links::build_index(&vault).unwrap();
        println!("links::build_index: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let _ = vault::list_tree_with(&vault, false).unwrap();
        println!("list_tree_with(false): {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let brief = crate::session_hooks::session_brief(&vault, Some("단비"), 2500).unwrap();
        println!("session_brief(단비): {:?}, {} chars", t.elapsed(), brief.chars().count());
        let t = std::time::Instant::now();
        let w = vector_mixed_warning(&vault);
        println!("vector_mixed_warning (cold): {:?} → {:?}", t.elapsed(), w);
        let t = std::time::Instant::now();
        let _ = vector_mixed_warning(&vault);
        println!("vector_mixed_warning (warm): {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let _ = dashboard::briefing(&vault, None).unwrap();
        println!("dashboard::briefing(None): {:?}", t.elapsed());
        for name in ["danbi_list_projects", "danbi_briefing", "danbi_recent", "danbi_daily"] {
            let t = std::time::Instant::now();
            let out = dispatch_in(&vault, name, json!({})).unwrap();
            println!("{name}: {:?}, {} chars", t.elapsed(), out.chars().count());
        }
        let t = std::time::Instant::now();
        let out = dispatch_in(&vault, "danbi_briefing", json!({ "project": "단비" })).unwrap_or_default();
        println!("danbi_briefing(단비): {:?}, {} chars", t.elapsed(), out.chars().count());
        if std::env::var("DANBI_DUMP").is_ok() {
            let out = dispatch_in(&vault, "danbi_briefing", json!({})).unwrap_or_default();
            let v: Value = serde_json::from_str(&out).unwrap();
            for (k, val) in v.as_object().unwrap() {
                println!("  briefing.{k}: {} chars", val.to_string().chars().count());
            }
            let out = dispatch_in(&vault, "danbi_list_projects", json!({})).unwrap_or_default();
            println!("{}", &out[..out.len().min(1500)]);
        }
        let t = std::time::Instant::now();
        let out = dispatch_in(&vault, "danbi_list_projects", json!({ "project": "단비" })).unwrap_or_default();
        println!("danbi_list_projects(단비): {:?}, {} chars", t.elapsed(), out.chars().count());
    }
}
