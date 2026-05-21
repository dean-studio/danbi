use crate::providers::Provider;
use crate::error::{DanbiError, DanbiResult};
use crate::vault::PROJECTS_DIRNAME;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use tantivy::{
    collector::TopDocs,
    query::QueryParser,
    schema::{Field, Schema, TextFieldIndexing, TextOptions, Value, STORED, STRING},
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument,
};

#[derive(Debug, Serialize, Clone)]
pub struct VaultDocIndex {
    pub project: String,
    pub domain: String,
    pub chars: usize,
    pub preview: String,
    pub headings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchHit {
    pub project: String,
    pub domain: String,
    #[serde(default)]
    pub relevance: f32,
    #[serde(default)]
    pub snippet: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub summary: String,
}

const PREVIEW_CHARS: usize = 240;
const MAX_HEADINGS: usize = 8;

// ---------- In-memory Tantivy index ----------

struct SearchFields {
    project: Field,
    domain: Field,
    body: Field,
    body_ngram: Field,
    /// Raw markdown stashed for snippet extraction without hitting disk.
    stored_body: Field,
}

struct IndexedCorpus {
    vault_fingerprint: String,
    index: Index,
    reader: IndexReader,
    fields: SearchFields,
}

/// Global cache of the most recent tantivy index. Rebuilt lazily when the
/// vault contents change.
static CORPUS: Mutex<Option<IndexedCorpus>> = Mutex::new(None);

fn make_schema() -> (Schema, SearchFields) {
    let mut sb = Schema::builder();
    let project = sb.add_text_field("project", STRING | STORED);
    let domain = sb.add_text_field("domain", STRING | STORED);

    // `body` field: default tokenizer (tokenizes on whitespace + lowercases).
    // Works well for Latin text and reasonably for Korean thanks to ngram.
    let body_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let body = sb.add_text_field("body", body_opts);

    // `body_ngram` mirrors the body but uses a character n-gram tokenizer so
    // Korean substrings (which rarely split on whitespace the way English
    // does) still match: "로그인" matches against "로그인하기".
    let ngram_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("danbi_ngram")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let body_ngram = sb.add_text_field("body_ngram", ngram_opts);

    let stored_body = sb.add_text_field("stored_body", TextOptions::default().set_stored());

    let schema = sb.build();
    (
        schema,
        SearchFields {
            project,
            domain,
            body,
            body_ngram,
            stored_body,
        },
    )
}

fn fingerprint_vault(vault: &Path) -> DanbiResult<String> {
    // Sum of (path, modified_ms, len) hashes → cheap change detector.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok("empty".into());
    }
    let mut h = DefaultHasher::new();
    for entry in std::fs::read_dir(&projects_root)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        name.hash(&mut h);
        hash_dir_md(&p, None, &mut h)?;
        for de in std::fs::read_dir(&p)? {
            let de = de?;
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
            hash_dir_md(&sub, Some(sname), &mut h)?;
            // Depth 2.
            for de2 in std::fs::read_dir(&sub)? {
                let de2 = de2?;
                let sub2 = de2.path();
                if !sub2.is_dir() {
                    continue;
                }
                let Some(sname2) = sub2.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if sname2.starts_with('.') || sname2 == "_assets" {
                    continue;
                }
                let combined = format!("{sname}/{sname2}");
                hash_dir_md(&sub2, Some(&combined), &mut h)?;
            }
        }
    }
    Ok(format!("{:x}", h.finish()))
}

fn hash_dir_md(
    dir: &Path,
    prefix: Option<&str>,
    h: &mut std::collections::hash_map::DefaultHasher,
) -> DanbiResult<()> {
    use std::hash::Hash;
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    for de in rd {
        let de = de?;
        let path = de.path();
        if !path.is_file() {
            continue;
        }
        let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !fname.to_lowercase().ends_with(".md") || fname.starts_with('.') {
            continue;
        }
        if let Some(p) = prefix {
            p.hash(h);
        }
        fname.hash(h);
        if let Ok(meta) = std::fs::metadata(&path) {
            meta.len().hash(h);
            if let Ok(m) = meta.modified() {
                if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                    d.as_millis().hash(h);
                }
            }
        }
    }
    Ok(())
}

/// Walk a project directory and collect all `.md` files up to depth 2
/// (project/, project/<folder>/, project/<folder>/<folder>/).  The
/// returned name is the path relative to the project root, e.g.
/// `daily/2026-01/05.md`. Hidden directories (`.git`, `_assets`, ".") are
/// skipped at every level.
fn collect_project_md_files(
    project_dir: &Path,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> DanbiResult<()> {
    collect_md_files(project_dir, None, out)?;
    for de in std::fs::read_dir(project_dir)? {
        let de = de?;
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
        collect_md_files(&sub, Some(sname), out)?;
        // One more level of nesting (depth 2).
        for de2 in std::fs::read_dir(&sub)? {
            let de2 = de2?;
            let sub2 = de2.path();
            if !sub2.is_dir() {
                continue;
            }
            let Some(sname2) = sub2.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if sname2.starts_with('.') || sname2 == "_assets" {
                continue;
            }
            let combined = format!("{sname}/{sname2}");
            collect_md_files(&sub2, Some(&combined), out)?;
        }
    }
    Ok(())
}

/// Walks one directory, adds its `.md` files into `out` with the given
/// subfolder prefix (or no prefix for the project root).
fn collect_md_files(
    dir: &Path,
    prefix: Option<&str>,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> DanbiResult<()> {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    for de in rd {
        let de = de?;
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
        let full = match prefix {
            Some(p) => format!("{p}/{fname}"),
            None => fname.to_string(),
        };
        out.push((full, path));
    }
    Ok(())
}

fn build_corpus(vault: &Path) -> DanbiResult<IndexedCorpus> {
    use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};

    let (schema, fields) = make_schema();
    let index = Index::create_in_ram(schema);

    // Register our Korean-friendly ngram analyzer on the `body_ngram` field.
    let ngram_analyzer = TextAnalyzer::builder(
        NgramTokenizer::new(2, 3, false)
            .map_err(|e| DanbiError::Other(format!("ngram: {e}")))?,
    )
    .filter(LowerCaser)
    .build();
    index
        .tokenizers()
        .register("danbi_ngram", ngram_analyzer);

    let mut writer: IndexWriter = index
        .writer(15_000_000)
        .map_err(|e| DanbiError::Other(format!("tantivy writer: {e}")))?;

    let projects_root = vault.join(PROJECTS_DIRNAME);
    if projects_root.exists() {
        for entry in std::fs::read_dir(&projects_root)? {
            let entry = entry?;
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let project = match p.file_name().and_then(|s| s.to_str()) {
                Some(n) if !n.starts_with('.') => n.to_string(),
                _ => continue,
            };

            // Gather all .md files under this project, walking up to
            // depth 2 (project/, project/<f>/, project/<f>/<f>/).
            let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
            collect_project_md_files(&p, &mut files)?;

            for (fname, path) in files {
                let Ok(md) = std::fs::read_to_string(&path) else {
                    continue;
                };

                // Boost filename + headings by prepending them to the
                // searchable body — tantivy then weights position higher.
                let headings: Vec<String> = md
                    .lines()
                    .filter(|l| l.trim_start().starts_with('#'))
                    .take(MAX_HEADINGS)
                    .map(|l| l.trim().to_string())
                    .collect();
                let boosted = format!(
                    "{project} {fname} {heads}\n{body}",
                    heads = headings.join(" "),
                    body = md,
                );

                let mut doc = TantivyDocument::default();
                doc.add_text(fields.project, project.as_str());
                doc.add_text(fields.domain, fname.as_str());
                doc.add_text(fields.body, boosted.as_str());
                doc.add_text(fields.body_ngram, boosted.as_str());
                doc.add_text(fields.stored_body, md.as_str());
                writer
                    .add_document(doc)
                    .map_err(|e| DanbiError::Other(format!("tantivy add: {e}")))?;
            }
        }
    }

    writer
        .commit()
        .map_err(|e| DanbiError::Other(format!("tantivy commit: {e}")))?;

    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(|e| DanbiError::Other(format!("tantivy reader: {e}")))?;

    Ok(IndexedCorpus {
        vault_fingerprint: fingerprint_vault(vault)?,
        index,
        reader,
        fields,
    })
}

fn ensure_corpus_fresh(vault: &Path) -> DanbiResult<()> {
    let fp = fingerprint_vault(vault)?;
    let mut slot = CORPUS
        .lock()
        .map_err(|_| DanbiError::Other("corpus lock".into()))?;
    if let Some(c) = slot.as_ref() {
        if c.vault_fingerprint == fp {
            return Ok(());
        }
    }
    let corpus = build_corpus(vault)?;
    *slot = Some(corpus);
    Ok(())
}

// ---------- Public search ----------

pub fn build_index(vault: &Path) -> DanbiResult<Vec<VaultDocIndex>> {
    // Retained for the AI reranker — gives Haiku a preview-sized directory
    // of every doc regardless of tantivy state.
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&projects_root)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let project = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        collect_project_md_files(&p, &mut files)?;

        for (fname, path) in files {
            let Ok(md) = std::fs::read_to_string(&path) else {
                continue;
            };
            let chars = md.chars().count();
            let preview: String = md.chars().take(PREVIEW_CHARS).collect();
            let mut headings = Vec::new();
            for line in md.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with('#') {
                    headings.push(trimmed.to_string());
                    if headings.len() >= MAX_HEADINGS {
                        break;
                    }
                }
            }
            out.push(VaultDocIndex {
                project: project.clone(),
                domain: fname,
                chars,
                preview,
                headings,
            });
        }
    }
    Ok(out)
}

/// Tier-2 full-text search. Uses tantivy BM25 on a default tokenizer plus a
/// character n-gram field for Korean substring coverage.
pub fn full_search(
    vault: &Path,
    query: &str,
    limit: usize,
) -> DanbiResult<Vec<SearchHit>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    ensure_corpus_fresh(vault)?;
    let slot = CORPUS
        .lock()
        .map_err(|_| DanbiError::Other("corpus lock".into()))?;
    let corpus = match slot.as_ref() {
        Some(c) => c,
        None => return Ok(Vec::new()),
    };

    let searcher = corpus.reader.searcher();

    // Query both the body (tokenized) and the ngram (Korean-friendly).
    let parser = QueryParser::for_index(
        &corpus.index,
        vec![corpus.fields.body, corpus.fields.body_ngram],
    );
    let query = match parser.parse_query(trimmed) {
        Ok(q) => q,
        Err(_) => {
            // Fallback: quote the query so special characters don't trip the parser.
            let escaped = trimmed.replace('"', " ");
            let quoted = format!("\"{}\"", escaped);
            parser
                .parse_query(&quoted)
                .map_err(|e| DanbiError::Other(format!("tantivy parse: {e}")))?
        }
    };

    let top = searcher
        .search(&query, &TopDocs::with_limit(limit.max(1)))
        .map_err(|e| DanbiError::Other(format!("tantivy search: {e}")))?;

    let mut hits = Vec::with_capacity(top.len());
    let max_score = top.first().map(|(s, _)| *s).unwrap_or(1.0).max(0.001);

    for (score, addr) in top {
        let Ok(doc) = searcher.doc::<TantivyDocument>(addr) else {
            continue;
        };
        let project = doc
            .get_first(corpus.fields.project)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let domain = doc
            .get_first(corpus.fields.domain)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let body = doc
            .get_first(corpus.fields.stored_body)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let snippet = excerpt_for_query(body, trimmed);
        hits.push(SearchHit {
            project,
            domain,
            relevance: (score / max_score).min(1.0),
            snippet,
        });
    }

    Ok(hits)
}

fn excerpt_for_query(body: &str, query: &str) -> String {
    let lower = body.to_lowercase();
    for token in query.to_lowercase().split_whitespace() {
        if let Some(idx) = lower.find(token) {
            let start = idx.saturating_sub(20);
            let end = (idx + token.len() + 60).min(body.len());
            let slice: String = body.chars().skip(start).take(end - start).collect();
            return format!("… {slice} …");
        }
    }
    body.chars().take(80).collect()
}

/// Tier-1 instant search — preview-only substring match. Kept because it's
/// even faster than tantivy on vaults smaller than ~30 files and serves as
/// a safety net while the corpus is being (re)built.
pub fn local_search(
    index: &[VaultDocIndex],
    query: &str,
    limit: usize,
) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let tokens: Vec<&str> = q.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(i32, &VaultDocIndex, String)> = Vec::new();
    for doc in index {
        let mut score: i32 = 0;
        let mut best_snippet = String::new();

        let domain_lc = doc.domain.to_lowercase();
        let project_lc = doc.project.to_lowercase();
        let preview_lc = doc.preview.to_lowercase();
        let headings_lc: Vec<String> =
            doc.headings.iter().map(|h| h.to_lowercase()).collect();

        for t in &tokens {
            if domain_lc.contains(t) {
                score += 10;
                if best_snippet.is_empty() {
                    best_snippet = format!("파일명에 '{t}' 매치");
                }
            }
            if project_lc.contains(t) {
                score += 5;
            }
            for (hi, h) in headings_lc.iter().enumerate() {
                if h.contains(t) {
                    score += 4;
                    if best_snippet.is_empty() {
                        best_snippet = doc.headings[hi].clone();
                    }
                }
            }
            if preview_lc.contains(t) {
                score += 2;
                if best_snippet.is_empty() {
                    best_snippet = excerpt_around(&doc.preview, t);
                }
            }
        }

        if score > 0 {
            scored.push((score, doc, best_snippet));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(score, doc, snippet)| SearchHit {
            project: doc.project.clone(),
            domain: doc.domain.clone(),
            relevance: (score as f32 / 20.0).min(1.0),
            snippet,
        })
        .collect()
}

fn excerpt_around(text: &str, needle: &str) -> String {
    let lower = text.to_lowercase();
    if let Some(idx) = lower.find(needle) {
        let start = idx.saturating_sub(20);
        let end = (idx + needle.len() + 60).min(text.len());
        let slice: String = text.chars().skip(start).take(end - start).collect();
        format!("… {slice} …")
    } else {
        text.chars().take(80).collect()
    }
}

// ---------- Tier-3: Haiku reranker ----------

const SEARCH_SYSTEM: &str = r####"You are Danbi's vault search ranker.
The user will give you a natural-language query and a JSON index of all
markdown files in their vault. You pick the 1–5 most relevant files and
explain why, tersely.

Reply with ONLY a JSON object matching this schema:

{
  "hits": [
    { "project": string, "domain": string, "relevance": number, "snippet": string }
  ],
  "summary": string
}

Rules:
- "project" and "domain" MUST exactly match entries in the provided index.
- "domain" values always end with ".md".
- "relevance" is 0.0 to 1.0 — how confident the file answers the query.
- "snippet" is a 1-sentence Korean reason citing the file's preview/headings.
- If nothing is clearly relevant, return an empty "hits" array and explain
  in "summary".
- Never invent files. Never output prose outside the JSON.
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

pub async fn search(
    provider: &dyn Provider,
    model_id: &str,
    query: &str,
    index: &[VaultDocIndex],
) -> DanbiResult<SearchResponse> {
    if index.is_empty() {
        return Ok(SearchResponse {
            hits: Vec::new(),
            summary: "vault가 비어있어요.".into(),
        });
    }
    let prompt = serde_json::json!({
        "query": query,
        "index": index,
    })
    .to_string();

    let raw = crate::usage::with_role(
        "search",
        provider.converse_text(model_id, Some(SEARCH_SYSTEM), &prompt, 1024, 0.0),
    )
    .await?;

    let cleaned = strip_code_fence(&raw);
    let parsed: SearchResponse = serde_json::from_str(cleaned).map_err(|e| {
        DanbiError::Other(format!("search JSON parse failed: {e}; raw='{raw}'"))
    })?;

    let valid: std::collections::HashSet<(String, String)> = index
        .iter()
        .map(|d| (d.project.clone(), d.domain.clone()))
        .collect();
    let hits = parsed
        .hits
        .into_iter()
        .filter(|h| valid.contains(&(h.project.clone(), h.domain.clone())))
        .collect();
    Ok(SearchResponse {
        hits,
        summary: parsed.summary,
    })
}

// ─── RRF hybrid search ────────────────────────────────────────────────
// Reciprocal Rank Fusion (Cormack et al., 2009): combine ranked result
// lists by summing 1/(k+rank) across systems. k=60 is the long-standing
// default that works well across heterogeneous retrievers.
//
// In Danbi the two retrievers are:
//   1) BM25 + Korean n-gram (tantivy `full_search`)
//   2) Cosine similarity over Gemini-style embeddings (`vector::query`)
//
// When the user has not configured an embed provider we silently skip
// step 2 — the hybrid call collapses to plain BM25, so callers can always
// route through this function without branching on cfg state.

const RRF_K: f64 = 60.0;

/// Fuse a BM25 hit list with a vector hit list using Reciprocal Rank
/// Fusion. Returned hits carry the BM25 snippet/relevance when available
/// (BM25 is the better source of human-readable excerpts), and otherwise
/// fall back to a placeholder snippet. The final relevance field is the
/// RRF score normalized to [0, 1] over the returned set so callers
/// downstream can keep using it as a confidence-ish number.
pub fn rrf_merge(
    bm25_hits: &[SearchHit],
    vector_hits: &[crate::vector::VectorHit],
    limit: usize,
) -> Vec<SearchHit> {
    if limit == 0 {
        return Vec::new();
    }
    use std::collections::HashMap;
    let mut scores: HashMap<(String, String), f64> = HashMap::new();
    // Keep the first SearchHit we see for each key — BM25 hits are
    // preferred because they include a real snippet.
    let mut seed: HashMap<(String, String), SearchHit> = HashMap::new();

    for (rank, hit) in bm25_hits.iter().enumerate() {
        let key = (hit.project.clone(), hit.domain.clone());
        *scores.entry(key.clone()).or_insert(0.0) +=
            1.0 / (RRF_K + (rank + 1) as f64);
        seed.entry(key).or_insert_with(|| hit.clone());
    }
    for (rank, hit) in vector_hits.iter().enumerate() {
        let key = (hit.project.clone(), hit.domain.clone());
        *scores.entry(key.clone()).or_insert(0.0) +=
            1.0 / (RRF_K + (rank + 1) as f64);
        seed.entry(key).or_insert_with(|| SearchHit {
            project: hit.project.clone(),
            domain: hit.domain.clone(),
            relevance: 0.0,
            snippet: String::new(),
        });
    }

    let mut ranked: Vec<((String, String), f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let max = ranked.first().map(|(_, s)| *s).unwrap_or(1.0).max(1e-9);
    ranked
        .into_iter()
        .take(limit)
        .filter_map(|(key, score)| {
            seed.remove(&key).map(|mut hit| {
                hit.relevance = (score / max) as f32;
                hit
            })
        })
        .collect()
}

/// Convenience: run BM25 and (optionally) vector search, merge with RRF.
/// `vector_query` should be the raw query string already embedded by the
/// caller — embedding is async and lives in the IPC layer, so we keep
/// search.rs sync. Pass `None` to skip the vector arm entirely.
pub fn full_search_hybrid(
    vault: &Path,
    query: &str,
    limit: usize,
    vector_embedding: Option<&[f32]>,
) -> DanbiResult<Vec<SearchHit>> {
    // Pull more candidates per arm than we'll keep — RRF works best when
    // each retriever contributes a fuller list, so depth=3x final limit
    // is a common starting point.
    let depth = limit.saturating_mul(3).max(8);
    let bm25 = full_search(vault, query, depth)?;
    let vector_hits = match vector_embedding {
        Some(emb) => {
            let idx = crate::vector::load(vault).unwrap_or_else(|_| {
                crate::vector::VectorIndex::default()
            });
            crate::vector::query(&idx, emb, depth)
        }
        None => Vec::new(),
    };
    Ok(rrf_merge(&bm25, &vector_hits, limit))
}
