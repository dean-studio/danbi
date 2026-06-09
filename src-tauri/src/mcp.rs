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
            // Plain REST endpoints — same auth (Bearer token) and same
            // dispatcher as the JSON-RPC routes, but no envelope. Made
            // for external callers (Paperclip, cron jobs, shell scripts)
            // that don't want to construct JSON-RPC payloads.
            .route("/api/health", post(handle_health).get(handle_health))
            .route("/api/tools", get(handle_api_tools))
            .route("/api/call/:tool", post(handle_api_call))
            .route("/api/projects/:project/call/:tool", post(handle_api_call_scoped))
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
    run_api_call(&state, &headers, tool, args, None)
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
    run_api_call(&state, &headers, tool, args, Some(project))
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
    let known = match crate::vault::list_tree(&vault_path) {
        Ok(t) => t.projects.iter().any(|p| p.name == resolved_project),
        Err(_) => false,
    };
    if !known {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": format!("unknown project: {resolved_project}")
            })),
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

    let commit = vcs::snapshot(
        &vault_path,
        &format!("danbi: rest {mode} · {resolved_project}/{domain}"),
    )
    .unwrap_or_default();

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

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "project": resolved_project,
            "domain": domain,
            "bytes": final_bytes,
            "mode": mode,
            "commit": commit,
        })),
    )
}

#[derive(Deserialize, Default)]
struct V1VaultQuery {
    /// "overwrite" (default) or "append".
    mode: Option<String>,
}

fn run_api_call(
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
        if is_write_tool(&tool) {
            if let Value::Object(ref mut map) = args {
                map.insert("project".into(), Value::String(proj.clone()));
            } else {
                let mut map = serde_json::Map::new();
                map.insert("project".into(), Value::String(proj.clone()));
                args = Value::Object(map);
            }
        }
    }

    let inbound_meta = extract_inbound_meta(&tool, &args);
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let args_for_banner = args.clone();

    match dispatch(&tool, args) {
        Ok(text) => {
            if let Some(meta) = inbound_meta.as_ref() {
                record_inbound_after_success(&tool, meta, &text, user_agent.as_deref());
            }
            let text = inject_active_goals_into_text(
                text,
                &tool,
                &args_for_banner,
                scoped_project.as_deref(),
            );
            // Most tools return a JSON document as a string. Try to
            // surface it as parsed JSON so REST consumers get structured
            // data without an extra parse step. If parsing fails we just
            // pass the raw string through.
            let parsed: Value =
                serde_json::from_str(&text).unwrap_or(Value::String(text));
            (
                StatusCode::OK,
                Json(json!({ "ok": true, "result": parsed })),
            )
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
) -> (StatusCode, Json<Value>) {
    run_rpc(&state, &headers, req, None).await
}

async fn handle_rpc_scoped(
    State(state): State<McpState>,
    AxumPath(project_id): AxumPath<String>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> (StatusCode, Json<Value>) {
    run_rpc(&state, &headers, req, Some(project_id)).await
}

async fn run_rpc(
    state: &McpState,
    headers: &HeaderMap,
    req: RpcRequest,
    scoped_project_id: Option<String>,
) -> (StatusCode, Json<Value>) {
    if req.jsonrpc != "2.0" {
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_err(req.id, -32600, "jsonrpc must be '2.0'")),
        );
    }
    if !auth_ok(headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(rpc_err(req.id, -32000, "invalid or missing bearer token")),
        );
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
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(rpc_err(
                        req.id,
                        -32003,
                        &format!("project lookup failed: {e}"),
                    )),
                );
            }
        },
    };

    match req.method.as_str() {
        "initialize" => {
            let server_name = match scoped_project.as_ref() {
                Some(p) => format!("{SERVER_NAME} · {p}"),
                None => SERVER_NAME.to_string(),
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
            Json(rpc_ok(req.id, json!({ "tools": tool_catalog() }))),
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
            // Project-scoped URLs only clamp WRITE tools to the owning
            // project — read tools are left free so an agent attached to
            // project A can still cross-reference project B. This preserves
            // the "no accidental writes" guarantee while letting a session
            // read the broader vault (e.g. "what's the status of the RAG
            // project?" asked from the i18n project's session).
            if let Some(proj) = scoped_project.as_ref() {
                if is_write_tool(name) {
                    if let Value::Object(ref mut map) = args {
                        map.insert(
                            "project".into(),
                            Value::String(proj.clone()),
                        );
                    } else {
                        let mut map = serde_json::Map::new();
                        map.insert("project".into(), Value::String(proj.clone()));
                        args = Value::Object(map);
                    }
                }
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
            // Snapshot args before dispatch consumes them — banner
            // injection later needs the project hint from the args.
            let args_for_banner = args.clone();

            // danbi_search 만 RRF 하이브리드 fast-path 로 분기. 임베딩
            // provider 가 cfg 에 있으면 BM25 + 벡터 결과를 RRF 로 병합해서
            // 외부 AI 의 자연어 쿼리도 정확히 잡아낸다. 임베딩 없으면
            // 기존 BM25 만으로 동작.
            let result: Result<String, DanbiError> = if name == "danbi_search" {
                search_hybrid_dispatch(args).await
            } else {
                dispatch(name, args)
            };
            match result {
                Ok(text) => {
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
                    (
                    StatusCode::OK,
                    Json(rpc_ok(
                        req.id,
                        json!({
                            "content": [ { "type": "text", "text": text } ],
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
    }
}

// ---------- Tool catalog ----------

fn tool_catalog() -> Vec<Value> {
    vec![
        json!({
            "name": "danbi_list_projects",
            "description": "단비 vault의 모든 프로젝트와 그 안의 도메인 파일 목록을 반환합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {},
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
                    "limit": { "type": "integer", "default": 8 }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_read",
            "description": "특정 프로젝트/도메인 파일의 전체 markdown 내용을 반환합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "domain": { "type": "string", "description": ".md로 끝나는 파일명" }
                },
                "required": ["project", "domain"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_log",
            "description": "지정한 프로젝트의 오늘 daily/YYYY-MM-DD.md 파일에 새 문단을 append합니다. LLM 없이 빠르게 실행됩니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "content": { "type": "string", "description": "추가할 markdown" }
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
            "name": "danbi_recent",
            "description": "최근 수정된 도메인 파일 목록 (기본 10개).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "default": 10 }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_briefing",
            "description": "단비 vault의 현재 상태 요약. 세션 시작 시 한 번 호출해서 최근 활동·제안·고아 파일을 한눈에 파악하세요. ghost 제안·healing 경고·7일 활동·오늘의 daily 노트가 한 JSON으로 돌아옵니다.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        }),
        json!({
            "name": "danbi_daily",
            "description": "오늘 daily 노트 + 회상(1주·1개월·1년 전 같은 날짜 노트)을 반환합니다. '그때 비슷한 걸 뭐 적었지?' 류 질문에 쓰세요.",
            "inputSchema": {
                "type": "object",
                "properties": {},
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
async fn search_hybrid_dispatch(args: Value) -> Result<String, DanbiError> {
    let vault_path = current_vault_path()?;
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DanbiError::Config("query required".into()))?
        .to_string();
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(8) as usize;

    // cfg 의 embed provider 로 query 를 임베딩한다. 키 없거나 호출 실패
    // 면 None — `full_search_hybrid` 가 BM25 only 로 graceful fallback.
    let embedding: Option<Vec<f32>> = match crate::config::load_config(&vault_path)? {
        Some(c) => crate::commands::embed_query_for_search(&c, &query).await,
        None => None,
    };

    let hits =
        search::full_search_hybrid(&vault_path, &query, limit, embedding.as_deref())?;
    Ok(serde_json::to_string_pretty(&hits)?)
}

fn dispatch(name: &str, args: Value) -> Result<String, DanbiError> {
    let vault_path = current_vault_path()?;

    match name {
        "danbi_list_projects" => {
            let tree = vault::list_tree(&vault_path)?;
            Ok(serde_json::to_string_pretty(&tree)?)
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
            Ok(vault::read_doc(&vault_path, &project, &domain)?)
        }
        "danbi_log" => {
            let project = string_arg(&args, "project")?;
            let content = string_arg(&args, "content")?;
            // Make sure project exists; refuse silent side effects.
            let tree = vault::list_tree(&vault_path)?;
            if !tree.projects.iter().any(|p| p.name == project) {
                return Err(DanbiError::Config(format!("unknown project: {project}")));
            }
            let domain = daily::ensure_today_note(&vault_path, &project)?;
            let current = vault::read_doc(&vault_path, &project, &domain)?;
            let op = EditOp::Append {
                content: content.clone(),
            };
            edit_ops::validate(&op)?;
            let next = edit_ops::apply(&current, &op)?;

            vcs::ensure_repo(&vault_path)?;
            let _ = vcs::snapshot(&vault_path, &format!("danbi: mcp log · {project}/{domain} (pre)"));
            vault::write_doc(&vault_path, &project, &domain, &next)?;
            let commit = vcs::snapshot(
                &vault_path,
                &format!("danbi: mcp log · {project}/{domain}"),
            )?;
            Ok(json!({
                "project": project,
                "domain": domain,
                "commit": commit,
                "bytes": next.len(),
            })
            .to_string())
        }
        "danbi_append" => {
            let project = string_arg(&args, "project")?;
            let domain = string_arg(&args, "domain")?;
            let content = string_arg(&args, "content")?;
            let tree = vault::list_tree(&vault_path)?;
            if !tree.projects.iter().any(|p| p.name == project) {
                return Err(DanbiError::Config(format!("unknown project: {project}")));
            }
            let domain_norm = if domain.to_lowercase().ends_with(".md") {
                domain.clone()
            } else {
                format!("{domain}.md")
            };
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
            let commit = vcs::snapshot(
                &vault_path,
                &format!("danbi: mcp append · {project}/{domain_norm}"),
            )?;
            Ok(json!({
                "project": project,
                "domain": domain_norm,
                "commit": commit,
                "bytes": next.len(),
            })
            .to_string())
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
            let tree = vault::list_tree(&vault_path)?;
            if !tree.projects.iter().any(|p| p.name == project) {
                return Err(DanbiError::Config(format!("unknown project: {project}")));
            }
            let domain_norm = if domain.to_lowercase().ends_with(".md") {
                domain.clone()
            } else {
                format!("{domain}.md")
            };

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

            let commit = vcs::snapshot(
                &vault_path,
                &format!("danbi: mcp create_file · {project}/{domain_norm}"),
            )?;
            let final_bytes = vault::read_doc(&vault_path, &project, &domain_norm)
                .unwrap_or_default()
                .len();
            Ok(json!({
                "project": project,
                "domain": domain_norm,
                "commit": commit,
                "bytes": final_bytes,
                "already_existed": already_existed,
            })
            .to_string())
        }
        "danbi_create_folder" => {
            let project = string_arg(&args, "project")?;
            let folder = string_arg(&args, "folder")?;
            let tree = vault::list_tree(&vault_path)?;
            if !tree.projects.iter().any(|p| p.name == project) {
                return Err(DanbiError::Config(format!("unknown project: {project}")));
            }
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
            let commit = vcs::snapshot(
                &vault_path,
                &format!("danbi: mcp create_folder · {project}/{folder}"),
            )?;
            Ok(json!({
                "project": project,
                "folder": folder,
                "commit": commit,
            })
            .to_string())
        }
        "danbi_recent" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            let tree = vault::list_tree(&vault_path)?;
            let mut all: Vec<(String, String, u128)> = Vec::new();
            for p in &tree.projects {
                for d in &p.domains {
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
            let snap = dashboard::snapshot(&vault_path)?;
            Ok(serde_json::to_string_pretty(&snap)?)
        }
        "danbi_daily" => {
            let snap = daily::snapshot(&vault_path)?;
            Ok(serde_json::to_string_pretty(&snap)?)
        }
        other => Err(DanbiError::Config(format!("unknown tool: {other}"))),
    }
}

/// Write tools mutate vault state and must stay clamped to the scoped
/// project. Read tools return information only and are safe to cross
/// project boundaries.
fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "danbi_log"
            | "danbi_append"
            | "danbi_create_folder"
            | "danbi_create_file"
    )
}

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
    let content = args
        .get("content")
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

fn string_arg(args: &Value, key: &str) -> Result<String, DanbiError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DanbiError::Config(format!("{key} required")))
}

fn current_vault_path() -> Result<PathBuf, DanbiError> {
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
