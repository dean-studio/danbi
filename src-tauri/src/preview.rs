use crate::providers::Provider;
use crate::edit_ops::EditOp;
use crate::error::DanbiResult;
use crate::grounding::GroundingSnippet;
use crate::project_context::{self, ProjectContext};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlanPreview {
    /// Short human-readable summary, 1 sentence.
    pub summary: String,
    /// Multi-paragraph explanation of what will happen.
    pub detail: String,
    /// Preview draft for the UI to display (markdown snippet).
    pub draft: String,
    /// Structured op — the writer's actual plan. `ask` intent may omit this.
    pub op: Option<EditOp>,
    /// If intent is "ask", this is the answer to show in chat (no file edit).
    pub answer: Option<String>,
}

const PLANNER_SYSTEM: &str = r####"You are the planning step for Danbi, a local markdown wiki agent.
You are NOT allowed to modify files directly. Instead, you decide what to do
and emit a JSON plan that the host code will execute.

Respond with ONLY a JSON object, no prose, no code fences:

{
  "summary":  string,
  "detail":   string,
  "draft":    string,
  "op":       EditOp | null,
  "answer":   string | null
}

EditOp schema (discriminated by "op"):

  {"op":"append",          "content": string}
  {"op":"insert_after",    "heading": string, "content": string}
  {"op":"replace_section", "heading": string, "new_body": string}
  {"op":"rewrite_all",     "content": string}

Rules:
- append: use {"op":"append", "content":"..."}.
- summarize: prefer append with a new "## 요약" section, or replace_section if one already exists.
- rewrite: use replace_section with the exact heading line the user meant. Fall back to rewrite_all only when the whole file must change.
- ask: set op=null and put the answer in "answer". Do NOT modify the file.
- `heading` must include the leading hash marks as they appear in the
  document (for example "## 로그인 흐름"). Match existing headings verbatim.
- `draft` should be a short markdown preview of what WILL be written
  (for replace_section show only the new body; for append show the new chunk).
- Do not rewrite an entire document when a narrower edit suffices.
- Be concise; avoid padding with boilerplate. Write in the same language as
  the existing document when possible (default Korean).
- If `attachments` is non-empty, use their `text` fields as authoritative
  source material. The user typically wants you to summarize / extract /
  merge that content into the target domain. Cite the source filename in
  `summary` when appropriate. Never paste attachment text verbatim beyond
  what the user asked for.
- If `grounding` is non-empty, treat those excerpts as the project's
  accumulated wiki knowledge. Ground your output in what the project
  already knows:
    * reuse existing terminology and decisions from those excerpts rather
      than inventing parallel vocabulary;
    * when a grounding doc is genuinely relevant, reference it inline as
      [[domain.md]] (or [[Project/domain.md]] for cross-project) so the
      wiki's link graph grows over time;
    * do NOT copy grounding text verbatim. Use it as context, not as
      content to paste.
- You may use wiki-style cross references between project domains with the
  syntax [[Project/domain.md]] or [[domain.md]] (same project). Prefer
  linking to an existing `grounding` doc over mentioning it by plain name.
"####;

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

pub struct PlanInput<'a> {
    pub intent: &'a str,
    pub project: &'a str,
    pub domain: &'a str,
    pub user_message: &'a str,
    pub doc_content: &'a str,
    pub attachments: &'a [Attachment],
    /// Related wiki passages pulled from the project. Empty slice disables
    /// grounding for that call (back-compat default).
    pub grounding: &'a [GroundingSnippet],
    /// Purpose + schema for the project. When the project hasn't set them
    /// up yet this is simply `ProjectContext::default()` and injection is
    /// a no-op — the Writer falls back to pre-J1 behavior.
    pub project_ctx: &'a ProjectContext,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Attachment {
    pub filename: String,
    pub kind: String,
    pub text: String,
    pub truncated: bool,
}

pub async fn build_plan(
    provider: &dyn Provider,
    model_id: &str,
    input: PlanInput<'_>,
) -> DanbiResult<PlanPreview> {
    let prompt = serde_json::json!({
        "intent": input.intent,
        "project": input.project,
        "domain": input.domain,
        "user_message": input.user_message,
        "current_document": input.doc_content,
        "attachments": input.attachments,
        "grounding": input.grounding,
    })
    .to_string();

    // Compose the system prompt with the static planner rules plus the
    // project's purpose/schema when present. This is the Wiki-LLM loop's
    // "the wiki IS the system prompt" pillar — every edit runs through
    // the project's declared intent.
    let system_prompt = match project_context::as_prompt_block(
        input.project_ctx,
        Some(input.domain),
    ) {
        Some(ctx_block) => format!("{PLANNER_SYSTEM}{ctx_block}"),
        None => PLANNER_SYSTEM.to_string(),
    };

    let raw = crate::usage::with_role(
        "preview",
        provider.converse_text(model_id, Some(&system_prompt), &prompt, 2048, 0.2),
    )
    .await?;

    let cleaned = strip_code_fence(&raw);
    match serde_json::from_str::<PlanPreview>(cleaned) {
        Ok(p) => Ok(p),
        Err(_) => Ok(PlanPreview {
            summary: "계획을 해석할 수 없어 원문을 표시합니다.".into(),
            detail:
                "모델이 JSON 형식을 지키지 않았어요. 같은 요청을 다시 보내면 보통 해결돼요."
                    .into(),
            draft: raw,
            op: None,
            answer: None,
        }),
    }
}
