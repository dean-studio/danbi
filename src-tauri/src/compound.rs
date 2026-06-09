use crate::providers::Provider;
use crate::error::{DanbiError, DanbiResult};
use crate::search;
use crate::vault;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompoundSource {
    pub project: String,
    pub domain: String,
    /// Full document contents. Trimmed if excessively long.
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompoundPlan {
    pub summary: String,
    pub detail: String,
    /// Full markdown to write into the target file.
    pub draft: String,
    /// Which source docs the writer actually used (subset of the input list).
    #[serde(default)]
    pub sources: Vec<CompoundCitation>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompoundCitation {
    pub project: String,
    pub domain: String,
    #[serde(default)]
    pub note: String,
}

const MAX_PER_SOURCE_CHARS: usize = 12_000;
const MAX_SOURCES: usize = 12;

/// Gathers up to N most relevant documents for a topic query. Uses Tier 2
/// (tantivy) plus, when a query embedding is provided, RRF-merged vector
/// hits — semantic recall matters here because the synthesizer has to
/// pull in scattered notes that may not share keywords with the topic.
/// Falls back to preview-only Tier 1 when both tiers come up empty.
pub fn gather_sources(
    vault: &Path,
    topic: &str,
    topic_embedding: Option<&[f32]>,
) -> DanbiResult<Vec<CompoundSource>> {
    let hits = match search::full_search_hybrid(vault, topic, MAX_SOURCES, topic_embedding) {
        Ok(h) if !h.is_empty() => h,
        _ => {
            let idx = search::build_index(vault)?;
            search::local_search(&idx, topic, MAX_SOURCES)
        }
    };
    let mut out = Vec::new();
    for h in hits {
        let content = vault::read_doc(vault, &h.project, &h.domain)?;
        let clipped: String = content.chars().take(MAX_PER_SOURCE_CHARS).collect();
        out.push(CompoundSource {
            project: h.project,
            domain: h.domain,
            content: clipped,
        });
    }
    Ok(out)
}

const COMPOUND_SYSTEM: &str = r####"You are Danbi's compounding step — a synthesizer
that reads several scattered markdown notes and produces ONE consolidated
document on a topic the user specified.

Inputs you will receive:
- `topic`: what the user asked to compound
- `target`: the filename being created/updated (e.g. "concepts/auth.md")
- `sources`: array of {project, domain, content} — related notes

Respond with ONLY a JSON object, no prose, no code fences:

{
  "summary": string,         // one Korean sentence about what you built
  "detail":  string,         // short Korean paragraph explaining structure
  "draft":   string,         // the FULL markdown that will be saved
  "sources": [               // which inputs you actually used
    { "project": string, "domain": string, "note": string }
  ]
}

Rules:
- `draft` is the complete final file. Start with a top-level `# heading`
  derived from the topic.
- Organize content into `##` sections by sub-theme.
- Cite sources inline where useful with wiki links: [[project/domain.md]].
- Do NOT paste large verbatim chunks — summarize and restructure.
- If two sources disagree, note the disagreement briefly.
- Do not invent facts that aren't in the sources.
- Write in the same language as the sources (default Korean).
- "sources" array is just the ones you actually cited/used.
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

pub async fn build_plan(
    vault: &std::path::Path,
    provider: &dyn Provider,
    model_id: &str,
    topic: &str,
    target: &str,
    sources: &[CompoundSource],
) -> DanbiResult<CompoundPlan> {
    if sources.is_empty() {
        return Err(DanbiError::Other(
            "관련 문서를 찾지 못했어요. 검색어를 더 구체적으로 바꿔 보세요.".into(),
        ));
    }

    let prompt = serde_json::json!({
        "topic": topic,
        "target": target,
        "sources": sources,
    })
    .to_string();

    // Cache by (model, topic, target, source-content fingerprint). If
    // the inputs are identical to a previous run, reuse the stored plan
    // verbatim instead of paying for another multi-source synthesis.
    let sources_fp = sources
        .iter()
        .map(|s| format!("{}/{}={}", s.project, s.domain, s.content.len()))
        .collect::<Vec<_>>()
        .join("|");
    let fingerprint_input =
        format!("{model_id}|{topic}|{target}|{sources_fp}");
    let fingerprint = crate::cache::content_hash(fingerprint_input.as_bytes());
    if let Some(cached) = crate::cache::load_blob(vault, "compound", &fingerprint) {
        if let Ok(plan) = serde_json::from_str::<CompoundPlan>(&cached) {
            return Ok(plan);
        }
    }

    let raw = crate::usage::with_role(
        "compound",
        provider.converse_text(model_id, Some(COMPOUND_SYSTEM), &prompt, 4096, 0.2),
    )
    .await?;

    let cleaned = strip_code_fence(&raw);
    let parsed: CompoundPlan = serde_json::from_str(cleaned).map_err(|e| {
        DanbiError::Other(format!("compound JSON parse failed: {e}; raw='{raw}'"))
    })?;

    if let Ok(body) = serde_json::to_string(&parsed) {
        let _ = crate::cache::save_blob(vault, "compound", &fingerprint, &body);
    }

    Ok(parsed)
}
