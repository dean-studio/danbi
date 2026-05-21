use crate::error::{DanbiError, DanbiResult};
use crate::providers::Provider;
use chrono::Local;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoutingContext {
    /// Names of registered projects in the vault.
    pub projects: Vec<String>,
    /// Map project -> list of domain filenames (".md").
    pub domains: std::collections::HashMap<String, Vec<String>>,
    /// Currently selected project (sticky context). Passed through as-is unless
    /// the user's message clearly names a different project.
    pub sticky_project: Option<String>,
    /// Currently selected domain.
    pub sticky_domain: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoutingResult {
    pub intent: String,              // "append" | "rewrite" | "summarize" | "ask" | "unknown"
    pub project: Option<String>,
    pub domain: Option<String>,
    pub confidence: f32,
    pub needs_clarification: bool,
    pub clarification_type: Option<String>, // "project" | "domain" | null
    pub candidate_projects: Vec<String>,
    pub candidate_domains: Vec<String>,
    pub summary: String, // one-line human-readable restatement
}

const ROUTER_SYSTEM: &str = r#"You are the router for Danbi, a local wiki management agent.
You analyze the user's natural-language request and decide three things:
1) intent — one of: append, rewrite, summarize, ask, compound, unknown
2) project — which registered project the user is targeting
3) domain — which markdown file inside that project

You respond with ONLY a JSON object, no prose, no code fences, matching this schema:

{
  "intent": "append" | "rewrite" | "summarize" | "ask" | "compound" | "unknown",
  "project": string | null,
  "domain": string | null,
  "confidence": number,          // 0..1
  "needs_clarification": boolean,
  "clarification_type": "project" | "domain" | null,
  "candidate_projects": string[],
  "candidate_domains": string[],
  "summary": string              // one short Korean sentence restating intent
}

Rules:
- You must only select a project from the "registered_projects" list. If the user's
  wording does not clearly match one, set project to null and
  needs_clarification=true with clarification_type="project", and put your top
  2-4 best candidates into candidate_projects (empty list if none).
- Once a project is chosen (either explicit or via sticky context), choose a domain
  from that project's domain list. Domain filenames always end with ".md". If the
  user's wording does not clearly point to one, set domain to null, set
  needs_clarification=true and clarification_type="domain", and put your top
  2-4 best candidates into candidate_domains (filenames verbatim).
- "compound": when the user wants to collect related content scattered across
  several existing files and synthesize a new consolidated note. Typical
  phrases: "묶어서", "정리해서 새 문서로", "여기저기 흩어져 있는 X 모아줘".
  In this case `domain` is the TARGET filename the user wants to create
  (invent one if unspecified, e.g. "summary-<topic>.md"). The target file
  may not exist yet — that's fine.
- Daily notes: if the user says "오늘", "today", "오늘 메모", "오늘의 작업",
  interpret the target as the project's daily note file
  `daily/YYYY-MM-DD.md` using the today_date value provided in the user
  payload (never invent a date from memory — only use today_date as given).
  The target may not yet exist — that's fine; it will be created on write.
- Honor sticky context: if sticky_project / sticky_domain is given and the user's
  message does not explicitly redirect to a different project/domain, carry them.
- "ask" intent never modifies files. Other intents imply a future write.
- If you cannot determine intent at all, return intent="unknown" with
  needs_clarification=true.
- Keep summary short (<= 80 chars)."#;

fn build_user_prompt(message: &str, ctx: &RoutingContext) -> String {
    let registered = serde_json::to_string(&ctx.projects).unwrap_or_else(|_| "[]".into());
    let domains = serde_json::to_string(&ctx.domains).unwrap_or_else(|_| "{}".into());
    let sticky = serde_json::json!({
        "project": ctx.sticky_project,
        "domain": ctx.sticky_domain,
    })
    .to_string();
    let today = Local::now().format("%Y-%m-%d").to_string();

    format!(
        "today_date = \"{today}\"\n\
         registered_projects = {registered}\n\
         project_domains = {domains}\n\
         sticky = {sticky}\n\
         user_message = {json_msg}\n\n\
         Return the JSON now.",
        json_msg = serde_json::to_string(message).unwrap_or_else(|_| "\"\"".into())
    )
}

/// Strips a surrounding ```json ... ``` fence if the model ignored instructions.
fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.trim_start_matches("json").trim_start();
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    t
}

pub async fn route(
    provider: &dyn Provider,
    model_id: &str,
    message: &str,
    ctx: &RoutingContext,
) -> DanbiResult<RoutingResult> {
    let prompt = build_user_prompt(message, ctx);
    let raw = crate::usage::with_role(
        "routing",
        provider.converse_text(model_id, Some(ROUTER_SYSTEM), &prompt, 512, 0.0),
    )
    .await?;

    let cleaned = strip_code_fence(&raw);
    let parsed: RoutingResult = serde_json::from_str(cleaned).map_err(|e| {
        DanbiError::Other(format!("router JSON parse failed: {e}; raw='{raw}'"))
    })?;
    Ok(parsed)
}
