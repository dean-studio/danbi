use crate::providers::Provider;
use crate::error::{DanbiError, DanbiResult};
use crate::links;
use crate::vault::{self, PROJECTS_DIRNAME};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const GHOST_DIRNAME: &str = ".danbi";
const GHOST_FILENAME: &str = "ghost-links.json";
const MAX_EXCERPT_CHARS: usize = 320;
const MAX_DOCS_IN_PROMPT: usize = 40;
const MAX_SUGGESTIONS: usize = 20;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GhostStatus {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GhostLink {
    pub id: String,
    /// Source "project/domain" — where the link would be inserted.
    pub source_domain: String,
    /// Target "project/domain" — what the source document likely refers to.
    pub target_domain: String,
    /// One-sentence rationale in Korean.
    pub reason: String,
    pub status: GhostStatus,
    /// Unix seconds.
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct GhostStore {
    #[serde(default)]
    pub links: Vec<GhostLink>,
    /// Unix seconds — last successful scan timestamp.
    #[serde(default)]
    pub last_scan_at: Option<i64>,
}

fn store_path(vault: &Path, project: &str) -> PathBuf {
    vault
        .join(PROJECTS_DIRNAME)
        .join(project)
        .join(GHOST_DIRNAME)
        .join(GHOST_FILENAME)
}

pub fn load(vault: &Path, project: &str) -> DanbiResult<GhostStore> {
    let path = store_path(vault, project);
    if !path.exists() {
        return Ok(GhostStore::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(GhostStore::default());
    }
    serde_json::from_str(&raw).map_err(|e| DanbiError::Other(format!("ghost-links parse: {e}")))
}

fn save(vault: &Path, project: &str, store: &GhostStore) -> DanbiResult<()> {
    let path = store_path(vault, project);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(store)
        .map_err(|e| DanbiError::Other(format!("ghost-links serialize: {e}")))?;
    std::fs::write(&path, raw)?;
    Ok(())
}

fn rand_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Short first-paragraph excerpt, used as signal for the router.
fn excerpt(md: &str) -> String {
    let trimmed = md.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let chars: String = trimmed.chars().take(MAX_EXCERPT_CHARS).collect();
    chars.replace('\n', " ")
}

fn collect_docs(vault: &Path, project: &str) -> DanbiResult<Vec<(String, String)>> {
    let proj_dir = vault.join(PROJECTS_DIRNAME).join(project);
    if !proj_dir.exists() {
        return Err(DanbiError::Config(format!("project not found: {project}")));
    }
    let mut out: Vec<(String, String)> = Vec::new();
    // Top level.
    if let Ok(rd) = std::fs::read_dir(&proj_dir) {
        for de in rd.flatten() {
            let path = de.path();
            if !path.is_file() {
                continue;
            }
            let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !fname.to_lowercase().ends_with(".md") || fname.starts_with('.') {
                continue;
            }
            let md = std::fs::read_to_string(&path).unwrap_or_default();
            out.push((fname.to_string(), excerpt(&md)));
        }
    }
    // One-level subfolders, skipping hidden / assets / .danbi.
    if let Ok(rd) = std::fs::read_dir(&proj_dir) {
        for de in rd.flatten() {
            let sub = de.path();
            if !sub.is_dir() {
                continue;
            }
            let Some(sname) = sub.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if sname.starts_with('.') || sname == "_assets" {
                continue;
            }
            if let Ok(rd2) = std::fs::read_dir(&sub) {
                for de2 in rd2.flatten() {
                    let path = de2.path();
                    if !path.is_file() {
                        continue;
                    }
                    let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    if !fname.to_lowercase().ends_with(".md") || fname.starts_with('.') {
                        continue;
                    }
                    let md = std::fs::read_to_string(&path).unwrap_or_default();
                    out.push((format!("{sname}/{fname}"), excerpt(&md)));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Returns existing wiki-link targets keyed by "project/domain". Used to
/// suppress suggestions that are already confirmed.
fn existing_links(vault: &Path, project: &str) -> HashMap<String, HashSet<String>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    let idx = match links::build_index(vault) {
        Ok(i) => i,
        Err(_) => return out,
    };
    let prefix = format!("{project}/");
    for (src, targets) in idx.outgoing.iter() {
        if !src.starts_with(&prefix) {
            continue;
        }
        let src_domain = src.trim_start_matches(&prefix).to_string();
        let set = out.entry(src_domain).or_default();
        for t in targets {
            // Only count within-project targets — suggestions cross projects too,
            // but early version keeps same-project only (cheap + safe).
            if t.project == project {
                set.insert(t.domain.clone());
            }
        }
    }
    out
}

const GHOST_SYSTEM: &str = r#"You are Danbi's Ghost Links suggester — you help the
user discover wiki links between their own markdown notes that they haven't
drawn yet.

Input format:
- `project`: the project name
- `docs`: array of { "domain": "file.md", "excerpt": "first 320 chars" }

Your job: suggest pairs (source_domain, target_domain) where the source's
excerpt clearly discusses the subject of the target, and a wiki link
`[[project/target]]` inside the source would help the reader navigate.

Hard rules:
- source_domain and target_domain MUST both appear verbatim in the `docs`
  array. No invented filenames.
- source_domain != target_domain.
- Do NOT suggest a link that the user is likely to reject — only confident
  conceptual overlaps, not coincidental keyword matches.
- Return AT MOST 10 suggestions, most useful first. Fewer is fine.
- Each `reason` is ONE short Korean sentence (<= 80 chars).

Respond with ONLY a JSON object, no prose, no fences:

{
  "suggestions": [
    {
      "source_domain": "ui.md",
      "target_domain": "notes/auth.md",
      "reason": "ui.md의 로그인 흐름 설명이 notes/auth.md의 토큰 정의를 참조함"
    }
  ]
}"#;

#[derive(Debug, Deserialize)]
struct RouterSuggestion {
    source_domain: String,
    target_domain: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Deserialize)]
struct RouterResponse {
    #[serde(default)]
    suggestions: Vec<RouterSuggestion>,
}

fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    // Strip the leading "```" (and optional "json" language tag) even when
    // the closing fence is missing — models sometimes truncate mid-JSON when
    // max_tokens is hit, and the opening fence would otherwise prevent
    // serde from ever seeing a valid JSON prefix.
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.trim_start_matches("json").trim_start();
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
        return rest.trim();
    }
    t
}

/// Salvages a truncated RouterResponse — the LLM hit max_tokens mid-array.
/// We search for the last `},` in the `suggestions` list, cut there, then
/// append `]}` so serde sees a well-formed shape.
fn salvage_truncated(cleaned: &str) -> Option<RouterResponse> {
    // The response shape is `{"suggestions":[ { ... }, { ... }, ... ]}`.
    // Find the last complete object in the array (closing brace followed by
    // a comma or the array terminator) and rebuild from there.
    let last_obj_end = cleaned.rfind("},")?;
    // Include everything up to and including that `}`, drop the trailing
    // comma, then close the array + object ourselves.
    let prefix = &cleaned[..=last_obj_end]; // includes `}` but not `,`
    let patched = format!("{prefix}]}}");
    serde_json::from_str::<RouterResponse>(&patched).ok()
}

/// Runs one Haiku call to propose ghost links for the project, merges them into
/// the on-disk store (preserving previous accept/reject decisions), and returns
/// the updated store.
pub async fn scan_project(
    vault: &Path,
    project: &str,
    provider: &dyn Provider,
    model_id: &str,
) -> DanbiResult<GhostStore> {
    let mut docs = collect_docs(vault, project)?;
    if docs.len() < 2 {
        // Not enough signal; just update the timestamp so the UI can show "no
        // candidates yet" without re-triggering spend.
        let mut store = load(vault, project)?;
        store.last_scan_at = Some(Utc::now().timestamp());
        save(vault, project, &store)?;
        return Ok(store);
    }
    // Cap to keep prompt size bounded. Priority: newest-modified first would
    // be nicer but we don't carry mtime here; alphabetical is deterministic.
    docs.truncate(MAX_DOCS_IN_PROMPT);

    // --- Incremental-build cache (Phase J-2) ---
    //
    // Build a hash of the full input set: every (filename, content_hash)
    // pair, plus the model id (since different models yield different
    // suggestions). When this composite matches what we saved last time,
    // we skip the LLM roundtrip entirely — the model would just produce
    // the same answer again, burning tokens.
    let cache_key = format!("ghost/{project}/{model_id}");
    let mut doc_hashes: Vec<(String, String)> = Vec::new();
    for (name, _excerpt) in &docs {
        let path = vault
            .join(crate::vault::PROJECTS_DIRNAME)
            .join(project)
            .join(name);
        let bytes = std::fs::read(&path).unwrap_or_default();
        doc_hashes.push((name.clone(), crate::cache::content_hash(&bytes)));
    }
    let set_hash = {
        let joined = doc_hashes
            .iter()
            .map(|(n, h)| format!("{n}={h}"))
            .collect::<Vec<_>>()
            .join("|");
        crate::cache::content_hash(joined.as_bytes())
    };
    let mut cache_map = crate::cache::load(vault, "ghost").unwrap_or_default();
    if crate::cache::is_fresh(&cache_map, &cache_key, &set_hash) {
        // No changes since last scan — just nudge the timestamp so the
        // UI can show "refreshed recently" without paying for the LLM.
        let mut store = load(vault, project)?;
        store.last_scan_at = Some(Utc::now().timestamp());
        save(vault, project, &store)?;
        return Ok(store);
    }

    // Record the in-progress scan so a crash leaves breadcrumbs. The
    // startup recovery path flips `Running` → `Pending` on next launch.
    let _ = crate::crash_queue::begin(
        vault,
        &format!("ghost-{project}"),
        serde_json::json!({
            "project": project,
            "model": model_id,
            "docs": docs.len(),
        }),
    );

    // Build the prompt payload.
    let payload = serde_json::json!({
        "project": project,
        "docs": docs
            .iter()
            .map(|(d, e)| serde_json::json!({ "domain": d, "excerpt": e }))
            .collect::<Vec<_>>(),
    });
    let user_prompt = format!(
        "{}\n\nReturn the JSON now.",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
    );

    let raw = crate::usage::with_role(
        "ghost",
        provider.converse_text(model_id, Some(GHOST_SYSTEM), &user_prompt, 2048, 0.1),
    )
    .await?;

    let cleaned = strip_code_fence(&raw);
    let parsed: RouterResponse = match serde_json::from_str::<RouterResponse>(cleaned) {
        Ok(v) => v,
        Err(_) => {
            // Common failure mode: the response hit max_tokens mid-array.
            // Try to salvage the longest prefix that still parses — we
            // walk back until a `}]}` (last complete suggestion + array
            // close + object close) is valid. Anything we recover is
            // strictly better than losing all suggestions.
            match salvage_truncated(cleaned) {
                Some(v) => v,
                None => {
                    // Cap the raw snippet so it fits in the UI notifier.
                    let snippet: String = raw.chars().take(400).collect();
                    return Err(DanbiError::Other(format!(
                        "ghost scan parse: model returned malformed JSON — raw='{snippet}'"
                    )));
                }
            }
        }
    };

    // Deduplicate against existing wiki links and previous decisions.
    let existing = existing_links(vault, project);
    let mut store = load(vault, project)?;
    let mut prior_decisions: HashMap<(String, String), GhostStatus> = HashMap::new();
    for l in &store.links {
        prior_decisions.insert(
            (l.source_domain.clone(), l.target_domain.clone()),
            l.status.clone(),
        );
    }

    let valid_domains: HashSet<String> = docs.iter().map(|(d, _)| d.clone()).collect();
    let mut merged: Vec<GhostLink> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    // Keep previously accepted/rejected links as-is — the user's decision stands.
    for l in &store.links {
        if l.status != GhostStatus::Pending {
            let key = (l.source_domain.clone(), l.target_domain.clone());
            if seen.insert(key) {
                merged.push(l.clone());
            }
        }
    }

    let now = Utc::now().timestamp();
    for s in parsed.suggestions.into_iter().take(MAX_SUGGESTIONS) {
        let src = s.source_domain.trim().to_string();
        let tgt = s.target_domain.trim().to_string();
        if src.is_empty() || tgt.is_empty() || src == tgt {
            continue;
        }
        if !valid_domains.contains(&src) || !valid_domains.contains(&tgt) {
            continue;
        }
        // Already linked in markdown? skip.
        if existing
            .get(&src)
            .map(|set| set.contains(&tgt))
            .unwrap_or(false)
        {
            continue;
        }
        // Previously decided? preserved in the loop above.
        let key = (src.clone(), tgt.clone());
        if prior_decisions.contains_key(&key) {
            continue;
        }
        if !seen.insert(key.clone()) {
            continue;
        }
        merged.push(GhostLink {
            id: rand_id(),
            source_domain: src,
            target_domain: tgt,
            reason: s.reason.trim().to_string(),
            status: GhostStatus::Pending,
            created_at: now,
        });
    }

    store.links = merged;
    store.last_scan_at = Some(now);
    save(vault, project, &store)?;

    // Stamp the cache so the next call with the same set + model skips
    // the LLM roundtrip. `set_hash` and `cache_key` were computed above.
    crate::cache::mark(&mut cache_map, cache_key, set_hash);
    let _ = crate::cache::save(vault, "ghost", &cache_map);

    // Mark the crash-queue entry as completed so a subsequent launch
    // doesn't think this project's scan was interrupted.
    let _ = crate::crash_queue::finish(vault, &format!("ghost-{project}"));

    Ok(store)
}

/// Marks a suggestion accepted and inserts `[[project/target]]` at the bottom
/// of the source document (under a "## 관련" section if present, else appended
/// after a blank line). Returns the updated store.
pub fn accept(
    vault: &Path,
    project: &str,
    id: &str,
) -> DanbiResult<GhostStore> {
    let mut store = load(vault, project)?;
    let Some(idx) = store.links.iter().position(|l| l.id == id) else {
        return Err(DanbiError::Config(format!("ghost link not found: {id}")));
    };
    let link = store.links[idx].clone();
    if link.status == GhostStatus::Accepted {
        return Ok(store);
    }

    // Read source doc, append a wiki link to it.
    let existing = vault::read_doc(vault, project, &link.source_domain)?;
    let insertion = format!("[[{project}/{}]]", link.target_domain);

    // If the doc already contains the exact link substring, don't duplicate —
    // just mark accepted.
    let updated = if existing.contains(&insertion) {
        existing
    } else {
        let trimmed = existing.trim_end().to_string();
        if trimmed.is_empty() {
            format!("{insertion}\n")
        } else if trimmed.contains("\n## 관련") {
            // Append to existing section.
            format!("{trimmed}\n- {insertion}\n")
        } else {
            format!("{trimmed}\n\n## 관련\n\n- {insertion}\n")
        }
    };

    vault::write_doc(vault, project, &link.source_domain, &updated)?;
    store.links[idx].status = GhostStatus::Accepted;
    save(vault, project, &store)?;
    Ok(store)
}

pub fn reject(vault: &Path, project: &str, id: &str) -> DanbiResult<GhostStore> {
    let mut store = load(vault, project)?;
    let Some(idx) = store.links.iter().position(|l| l.id == id) else {
        return Err(DanbiError::Config(format!("ghost link not found: {id}")));
    };
    store.links[idx].status = GhostStatus::Rejected;
    save(vault, project, &store)?;
    Ok(store)
}
