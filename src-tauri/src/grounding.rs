//! Wiki grounding — pulls relevant passages from the vault to attach to a
//! planning prompt so the Writer model grounds its output in accumulated
//! project knowledge rather than starting from a blank slate.
//!
//! This is the crucial piece of the Wiki-LLM methodology the app was built
//! around: the wiki isn't just a storage destination, it's the system
//! prompt. Every edit/append touches the full corpus through these
//! snippets.

use crate::error::DanbiResult;
use crate::search;
use crate::vault;
use serde::Serialize;
use std::path::Path;

/// A single grounding passage surfaced from the vault. `excerpt` is the
/// first ~800 chars of the file — enough to give the Writer the gist
/// without blowing up the token budget.
#[derive(Debug, Clone, Serialize)]
pub struct GroundingSnippet {
    pub project: String,
    pub domain: String,
    pub excerpt: String,
    /// tantivy relevance score, forwarded so the prompt can hint at priority.
    pub relevance: f32,
}

/// Collects up to `max_docs` related passages for the given query. The
/// document the user is currently editing is excluded so the model doesn't
/// echo itself back.
///
/// Scope:
///   - `project_filter = Some(p)` limits results to a single project's
///     files. This is the safe default for edits — we don't want the
///     Writer to drag in noise from unrelated projects.
///   - `project_filter = None` scans the whole vault (useful for cross-
///     project Q&A flows later).
///
/// `query_embedding` is optional — when present, BM25 hits are RRF-merged
/// with cosine-similarity hits from the persisted vector index so the
/// Writer also sees semantically related notes that wouldn't surface from
/// keyword overlap alone. Pass None when no embed provider is configured.
pub fn gather_grounding(
    vault: &Path,
    project_filter: Option<&str>,
    query: &str,
    query_embedding: Option<&[f32]>,
    exclude_domain: Option<&str>,
    max_docs: usize,
    max_chars_per_doc: usize,
) -> DanbiResult<Vec<GroundingSnippet>> {
    let query_trim = query.trim();
    if query_trim.is_empty() || max_docs == 0 {
        return Ok(Vec::new());
    }

    // tantivy + (optional) vector RRF; over-fetch a bit so the project
    // filter below still leaves enough candidates.
    let raw = search::full_search_hybrid(
        vault,
        query_trim,
        max_docs.saturating_mul(3).max(8),
        query_embedding,
    )?;

    let mut out: Vec<GroundingSnippet> = Vec::new();
    for hit in raw {
        if let Some(proj) = project_filter {
            if hit.project != proj {
                continue;
            }
        }
        if let Some(excl) = exclude_domain {
            // Match both raw domain and normalized (stripped daily/ prefix),
            // because callers pass whatever the UI has on hand.
            if hit.domain == excl {
                continue;
            }
        }

        // Read the actual file for a fresh excerpt — snippet in the hit is
        // only ~240 chars and loses context.
        let body = match vault::read_doc(vault, &hit.project, &hit.domain) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let excerpt: String = body.chars().take(max_chars_per_doc).collect();

        out.push(GroundingSnippet {
            project: hit.project,
            domain: hit.domain,
            excerpt,
            relevance: hit.relevance,
        });
        if out.len() >= max_docs {
            break;
        }
    }

    Ok(out)
}
