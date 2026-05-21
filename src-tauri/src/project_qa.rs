use crate::providers::Provider;
use crate::error::{DanbiError, DanbiResult};
use crate::search;
use crate::vault;
use serde::{Deserialize, Serialize};
use std::path::Path;

const MAX_SOURCES: usize = 6;
const MAX_CHARS_PER_SOURCE: usize = 6_000;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QaCitation {
    pub project: String,
    pub domain: String,
    pub note: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QaAnswer {
    pub answer: String,
    #[serde(default)]
    pub citations: Vec<QaCitation>,
    /// The exact source documents the router picked (filenames user can open).
    pub sources: Vec<String>,
}

const QA_SYSTEM: &str = r#"You are Danbi's project Q&A — you answer the user's
question using ONLY the provided source documents. You never fall back to
general knowledge; if the documents don't say, you say so in Korean.

You will receive:
- `question`: Korean natural-language question
- `project`: project name
- `sources`: array of {domain, content} — markdown excerpts from this project

Respond with ONLY a JSON object, no prose, no code fences:

{
  "answer": string,         // Korean paragraph, direct and concrete
  "citations": [            // which docs backed which claims
    { "project": string, "domain": string, "note": string }
  ]
}

Rules:
- Cite specifics (filenames, decisions, numbers) rather than paraphrasing vaguely.
- If the sources disagree, point it out and name the conflicting files.
- If nothing in the sources answers the question, answer "이 vault에는 해당
  내용이 없어요." and list `citations: []`.
- `project` in citations must match the provided project.
- `domain` must exactly match one of the source domains.
- Keep the answer under 6 short Korean sentences unless the question
  demands a list."#;

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

pub async fn ask(
    vault: &Path,
    project: &str,
    question: &str,
    provider: &dyn Provider,
    writer_model: &str,
) -> DanbiResult<QaAnswer> {
    // Run Tier-2 search but restrict to this project's files.
    let all_hits = search::full_search(vault, question, 24)?;
    let filtered: Vec<_> = all_hits
        .into_iter()
        .filter(|h| h.project == project)
        .take(MAX_SOURCES)
        .collect();

    if filtered.is_empty() {
        return Ok(QaAnswer {
            answer: "이 프로젝트에서 관련 문서를 찾지 못했어요.".into(),
            citations: Vec::new(),
            sources: Vec::new(),
        });
    }

    #[derive(Serialize)]
    struct SrcDoc<'a> {
        domain: &'a str,
        content: String,
    }

    let mut sources_payload: Vec<SrcDoc> = Vec::new();
    let mut source_names: Vec<String> = Vec::new();
    for h in &filtered {
        let full = vault::read_doc(vault, project, &h.domain).unwrap_or_default();
        let clipped: String = full.chars().take(MAX_CHARS_PER_SOURCE).collect();
        sources_payload.push(SrcDoc {
            domain: &h.domain,
            content: clipped,
        });
        source_names.push(h.domain.clone());
    }

    let prompt = serde_json::json!({
        "question": question,
        "project": project,
        "sources": sources_payload,
    });

    let raw = crate::usage::with_role(
        "qa",
        provider.converse_text(
            writer_model,
            Some(QA_SYSTEM),
            &serde_json::to_string(&prompt).unwrap_or_else(|_| "{}".into()),
            1500,
            0.1,
        ),
    )
    .await?;

    let cleaned = strip_code_fence(&raw);

    #[derive(Deserialize)]
    struct RawAnswer {
        answer: String,
        #[serde(default)]
        citations: Vec<QaCitation>,
    }

    let parsed: RawAnswer = serde_json::from_str(cleaned)
        .map_err(|e| DanbiError::Other(format!("qa parse: {e}; raw='{raw}'")))?;

    // Sanity: only keep citations that actually came from the sources we sent.
    let valid: std::collections::HashSet<String> = source_names.iter().cloned().collect();
    let citations = parsed
        .citations
        .into_iter()
        .filter(|c| c.project == project && valid.contains(&c.domain))
        .collect();

    Ok(QaAnswer {
        answer: parsed.answer,
        citations,
        sources: source_names,
    })
}
