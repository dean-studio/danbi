//! Lightweight in-process vector index.
//!
//! We deliberately avoid pulling in LanceDB / Qdrant / anything with a
//! native build dependency (protoc, etc.) — those would triple the
//! install headache for what's ultimately a hobby-scale corpus.
//!
//! Storage: `<vault>/.danbi/vectors.json`. The whole index is loaded
//! into RAM at query time and linearly scanned with cosine similarity.
//! For vaults up to a few thousand files this is instant; anyone past
//! that point probably wants a real vector DB anyway.
//!
//! Incremental indexing: each entry carries the file's content hash
//! (reusing `cache::content_hash`). When a file's hash matches the
//! stored entry we skip the embedding call.

use crate::error::DanbiResult;
use crate::providers::Provider;
use crate::vault::PROJECTS_DIRNAME;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const INDEX_FILE: &str = ".danbi/vectors.json";
/// Hard cap on chunk size fed into the embedding model. Most providers
/// accept 8K+ tokens but embedding quality degrades on very long
/// inputs; 2KB of characters ≈ 500 tokens is a safe, consistent budget.
const MAX_CHARS_PER_ITEM: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEntry {
    /// "project/domain" — matches the node id in graph.rs.
    pub id: String,
    pub project: String,
    pub domain: String,
    pub content_hash: String,
    pub model: String,
    pub embedding: Vec<f32>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VectorIndex {
    /// Keyed by id for O(1) lookup during incremental updates.
    #[serde(default)]
    pub entries: HashMap<String, VectorEntry>,
}

fn index_path(vault: &Path) -> PathBuf {
    vault.join(INDEX_FILE)
}

pub fn load(vault: &Path) -> DanbiResult<VectorIndex> {
    let path = index_path(vault);
    if !path.exists() {
        return Ok(VectorIndex::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(VectorIndex::default());
    }
    // Corrupt? start fresh; no user-facing failure for index rot.
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

pub fn save(vault: &Path, index: &VectorIndex) -> DanbiResult<()> {
    let path = index_path(vault);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(index)?)?;
    Ok(())
}

pub fn clear(vault: &Path) -> DanbiResult<()> {
    let path = index_path(vault);
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}

/// Simple stats shown in Settings so the user knows how fresh / big
/// the index is.
#[derive(Debug, Clone, Serialize)]
pub struct VectorIndexStats {
    pub count: usize,
    pub oldest: Option<i64>,
    pub newest: Option<i64>,
    pub model: Option<String>,
}

pub fn stats(index: &VectorIndex) -> VectorIndexStats {
    let mut oldest = i64::MAX;
    let mut newest = 0i64;
    let mut model: Option<String> = None;
    for e in index.entries.values() {
        if e.updated_at < oldest {
            oldest = e.updated_at;
        }
        if e.updated_at > newest {
            newest = e.updated_at;
        }
        if model.is_none() {
            model = Some(e.model.clone());
        }
    }
    VectorIndexStats {
        count: index.entries.len(),
        oldest: if oldest == i64::MAX { None } else { Some(oldest) },
        newest: if newest == 0 { None } else { Some(newest) },
        model,
    }
}

/// Walks the vault, embeds files whose hash changed, drops entries for
/// deleted files. Returns a summary the UI can display.
///
/// `batch_size` bounds how many docs are embedded per provider call.
/// Too small = too many round-trips; too large = one slow call can
/// block the progress ticker. 16 is a good compromise for most APIs.
#[derive(Debug, Clone, Serialize)]
pub struct ReindexReport {
    pub total: usize,
    pub embedded: usize,
    pub skipped: usize,
    pub removed: usize,
}

pub async fn reindex(
    vault: &Path,
    provider: &dyn Provider,
    model_id: &str,
    batch_size: usize,
    on_progress: Option<&(dyn Fn(ReindexProgress) + Send + Sync)>,
) -> DanbiResult<ReindexReport> {
    reindex_inner(vault, provider, model_id, batch_size, None, on_progress).await
}

/// Per-project reindex. Only walks `Projects/<name>/` and only prunes
/// stale entries that belong to that project — entries from other
/// projects in the index are left alone.
pub async fn reindex_project(
    vault: &Path,
    provider: &dyn Provider,
    model_id: &str,
    batch_size: usize,
    project: &str,
    on_progress: Option<&(dyn Fn(ReindexProgress) + Send + Sync)>,
) -> DanbiResult<ReindexReport> {
    reindex_inner(
        vault,
        provider,
        model_id,
        batch_size,
        Some(project),
        on_progress,
    )
    .await
}

/// Per-step progress signal pushed by the reindex loop. The host wires
/// this into a Tauri event so the UI can render a smooth progress bar
/// and a "다음 호출까지 N초" countdown without polling.
#[derive(Debug, Clone, Serialize)]
pub struct ReindexProgress {
    /// Lifecycle phase. Used by the UI to switch between "준비 중",
    /// "embedding…", "rate-limit 대기", and "완료".
    pub phase: String,
    /// Files completed so far (already in vector store).
    pub done: usize,
    /// Total pending files when the run started. `done + remaining`
    /// stays constant for the duration.
    pub total: usize,
    /// Last filename embedded — useful as a "currently processing" line.
    pub last_file: Option<String>,
    /// When `phase == "waiting"`, how long we'll sleep before the next
    /// batch (rate-limit cooldown for Gemini etc.).
    pub wait_secs: Option<u64>,
}

async fn reindex_inner(
    vault: &Path,
    provider: &dyn Provider,
    model_id: &str,
    batch_size: usize,
    only_project: Option<&str>,
    on_progress: Option<&(dyn Fn(ReindexProgress) + Send + Sync)>,
) -> DanbiResult<ReindexReport> {
    let mut index = load(vault)?;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut embedded = 0usize;
    let mut skipped = 0usize;
    let mut pending: Vec<(String, String, String, String, String)> = Vec::new();
    // Tuple: (id, project, domain, content_hash, clipped_body)

    let projects_root = vault.join(PROJECTS_DIRNAME);
    if projects_root.exists() {
        for entry in std::fs::read_dir(&projects_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(project) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if project.starts_with('.') {
                continue;
            }
            if let Some(only) = only_project {
                if project != only {
                    continue;
                }
            }
            collect_md_files(&path, project, None, &mut seen, &mut pending, &index, model_id)?;
            // Subfolders up to 2 levels (daily/, daily/2026-01/, …).
            for sub in std::fs::read_dir(&path)? {
                let sub = sub?.path();
                if !sub.is_dir() {
                    continue;
                }
                let Some(sname) = sub.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if sname.starts_with('.') || sname == "_assets" {
                    continue;
                }
                collect_md_files(
                    &sub,
                    project,
                    Some(sname),
                    &mut seen,
                    &mut pending,
                    &index,
                    model_id,
                )?;
                for sub2 in std::fs::read_dir(&sub)? {
                    let sub2 = sub2?.path();
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
                    collect_md_files(
                        &sub2,
                        project,
                        Some(&combined),
                        &mut seen,
                        &mut pending,
                        &index,
                        model_id,
                    )?;
                }
            }
        }
    }

    // Count how many are cache hits (already in index with matching hash
    // and model). We computed that during collection by checking index.
    for (id, _proj, _dom, hash, _body) in &pending {
        if let Some(existing) = index.entries.get(id) {
            if existing.content_hash == *hash && existing.model == model_id {
                skipped += 1;
            }
        }
    }

    // Run pending (non-fresh) through the provider in batches.
    let mut to_embed: Vec<&(String, String, String, String, String)> = pending
        .iter()
        .filter(|(id, _p, _d, h, _b)| {
            match index.entries.get(id) {
                Some(e) => e.content_hash != *h || e.model != model_id,
                None => true,
            }
        })
        .collect();

    // Gemini 무료 티어 embedding 모델은 분당 5회 제한이 빡빡해서
    // 기본 batch=16 으로 빠르게 돌리면 두 번째 batch 부터 거의 매번 429
    // 가 떨어진다. provider kind 가 google 일 때는 batch 크기를 작게
    // 줄이고 batch 사이에 12 초씩 잠재워 분당 5회 한도 안에 머무르게
    // 한다. (5 RPM = 12s/req). 다른 provider 는 기존 동작 유지.
    let is_google = provider.kind() == "google";
    let effective_batch = if is_google { 4 } else { batch_size.max(1) };
    let interbatch_delay_ms: u64 = if is_google { 12_500 } else { 0 };
    let total_pending = to_embed.len();

    if let Some(cb) = on_progress {
        cb(ReindexProgress {
            phase: "embedding".into(),
            done: 0,
            total: total_pending,
            last_file: None,
            wait_secs: None,
        });
    }

    let now = chrono::Local::now().timestamp();
    let mut first_batch = true;
    while !to_embed.is_empty() {
        if !first_batch && interbatch_delay_ms > 0 {
            // 사용자에게 "왜 멈춰있는지" 보여주기 위해 1초 단위로 카운트다운.
            let total_secs = (interbatch_delay_ms / 1000).max(1);
            for remaining in (1..=total_secs).rev() {
                if let Some(cb) = on_progress {
                    cb(ReindexProgress {
                        phase: "waiting".into(),
                        done: embedded,
                        total: total_pending,
                        last_file: None,
                        wait_secs: Some(remaining),
                    });
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
        first_batch = false;
        let take = to_embed.len().min(effective_batch);
        let batch: Vec<&(String, String, String, String, String)> =
            to_embed.drain(..take).collect();
        // Gemini 의 batchEmbedContents 는 빈 Part 를 거부한다 (400
        // INVALID_ARGUMENT). 빈 .md / 공백만 있는 .md 도 vault 자료엔 흔하니
        // 거기에 도메인 경로를 토큰으로 박은 fallback 텍스트를 넣어 의미
        // 검색 결과 망가뜨리지 않으면서 embedding 호출은 성공하게 한다.
        let inputs: Vec<String> = batch
            .iter()
            .map(|(_i, p, d, _h, b)| {
                let trimmed = b.trim();
                if trimmed.is_empty() {
                    // path 자체에 의미가 있을 때가 있어 (예: stats/2026-05-17.md)
                    // 빈 파일이라도 그 path 로는 잡히게.
                    format!("(empty) {p}/{d}")
                } else {
                    b.clone()
                }
            })
            .collect();
        if let Some(cb) = on_progress {
            cb(ReindexProgress {
                phase: "embedding".into(),
                done: embedded,
                total: total_pending,
                last_file: batch.first().map(|(_, p, d, _, _)| format!("{p}/{d}")),
                wait_secs: None,
            });
        }
        let vectors =
            crate::usage::with_role("embed", provider.embed(model_id, &inputs)).await?;
        if vectors.len() != batch.len() {
            return Err(crate::error::DanbiError::Other(format!(
                "embed returned {} vectors for {} inputs",
                vectors.len(),
                batch.len()
            )));
        }
        for ((id, project, domain, hash, _body), vec) in batch.iter().zip(vectors.into_iter()) {
            index.entries.insert(
                id.clone(),
                VectorEntry {
                    id: id.clone(),
                    project: project.clone(),
                    domain: domain.clone(),
                    content_hash: hash.clone(),
                    model: model_id.to_string(),
                    embedding: vec,
                    updated_at: now,
                },
            );
            embedded += 1;
        }
        // Persist incrementally so a crash mid-reindex doesn't lose
        // everything that was already embedded.
        let _ = save(vault, &index);
    }

    // Remove entries whose file no longer exists. When scoped to a single
    // project, only prune entries belonging to that project — leave other
    // projects' entries untouched.
    let before = index.entries.len();
    if let Some(only) = only_project {
        index
            .entries
            .retain(|k, e| e.project != only || seen.contains(k));
    } else {
        index.entries.retain(|k, _| seen.contains(k));
    }
    let removed = before.saturating_sub(index.entries.len());
    save(vault, &index)?;

    Ok(ReindexReport {
        total: seen.len(),
        embedded,
        skipped,
        removed,
    })
}

fn collect_md_files(
    dir: &Path,
    project: &str,
    prefix: Option<&str>,
    seen: &mut std::collections::HashSet<String>,
    pending: &mut Vec<(String, String, String, String, String)>,
    _index: &VectorIndex,
    _model_id: &str,
) -> DanbiResult<()> {
    for de in std::fs::read_dir(dir)? {
        let de = de?;
        let path = de.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.to_lowercase().ends_with(".md") || name.starts_with('.') {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap_or_default();
        let hash = crate::cache::content_hash(&bytes);
        let rel = match prefix {
            Some(p) => format!("{p}/{name}"),
            None => name.to_string(),
        };
        let id = format!("{project}/{rel}");
        seen.insert(id.clone());
        let body = String::from_utf8_lossy(&bytes);
        let clipped: String = body.chars().take(MAX_CHARS_PER_ITEM).collect();
        pending.push((id, project.to_string(), rel, hash, clipped));
    }
    Ok(())
}

/// Up-front estimate of how many files and tokens a reindex would cost,
/// broken down into "fresh" (unchanged, cache hit) vs "pending" (changed
/// or new, would actually be sent to the embedding provider).
///
/// Token count is a rough 3-chars-per-token heuristic. Real tokenizers
/// vary (Korean bloats, English deflates) but for a pre-flight "expect
/// about ₩N" estimate the error is well under the variance from FX and
/// per-model price rounding.
#[derive(Debug, Clone, Serialize)]
pub struct ReindexEstimate {
    pub total_files: usize,
    pub fresh_files: usize,
    pub pending_files: usize,
    pub pending_chars: u64,
    pub pending_tokens: u64,
    pub model: String,
}

const CHARS_PER_TOKEN_ESTIMATE: f64 = 3.0;

pub fn estimate_reindex(vault: &Path, model_id: &str) -> DanbiResult<ReindexEstimate> {
    let index = load(vault)?;
    let mut total_files = 0usize;
    let mut fresh_files = 0usize;
    let mut pending_files = 0usize;
    let mut pending_chars: u64 = 0;

    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(ReindexEstimate {
            total_files: 0,
            fresh_files: 0,
            pending_files: 0,
            pending_chars: 0,
            pending_tokens: 0,
            model: model_id.to_string(),
        });
    }

    // Reuses the same walk pattern as reindex() but only accumulates
    // counts — no embedding, no hashing beyond cache lookup.
    let walk = |dir: &Path,
                project: &str,
                prefix: Option<&str>,
                total: &mut usize,
                fresh: &mut usize,
                pending: &mut usize,
                chars: &mut u64|
     -> DanbiResult<()> {
        for de in std::fs::read_dir(dir)? {
            let de = de?;
            let path = de.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.to_lowercase().ends_with(".md") || name.starts_with('.') {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap_or_default();
            let hash = crate::cache::content_hash(&bytes);
            let rel = match prefix {
                Some(p) => format!("{p}/{name}"),
                None => name.to_string(),
            };
            let id = format!("{project}/{rel}");
            let body = String::from_utf8_lossy(&bytes);
            let clipped_char_count =
                body.chars().take(MAX_CHARS_PER_ITEM).count() as u64;
            *total += 1;
            match index.entries.get(&id) {
                Some(e) if e.content_hash == hash && e.model == model_id => {
                    *fresh += 1;
                }
                _ => {
                    *pending += 1;
                    *chars += clipped_char_count;
                }
            }
        }
        Ok(())
    };

    for entry in std::fs::read_dir(&projects_root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(project) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if project.starts_with('.') {
            continue;
        }
        walk(
            &path,
            project,
            None,
            &mut total_files,
            &mut fresh_files,
            &mut pending_files,
            &mut pending_chars,
        )?;
        for sub in std::fs::read_dir(&path)? {
            let sub = sub?.path();
            if !sub.is_dir() {
                continue;
            }
            let Some(sname) = sub.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if sname.starts_with('.') || sname == "_assets" {
                continue;
            }
            walk(
                &sub,
                project,
                Some(sname),
                &mut total_files,
                &mut fresh_files,
                &mut pending_files,
                &mut pending_chars,
            )?;
        }
    }

    let pending_tokens =
        (pending_chars as f64 / CHARS_PER_TOKEN_ESTIMATE).ceil() as u64;

    Ok(ReindexEstimate {
        total_files,
        fresh_files,
        pending_files,
        pending_chars,
        pending_tokens,
        model: model_id.to_string(),
    })
}

/// Cosine similarity. Returns 0 when either vector is empty / zero-length.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorHit {
    pub project: String,
    pub domain: String,
    pub score: f32,
}

pub fn query(
    index: &VectorIndex,
    query_embedding: &[f32],
    limit: usize,
) -> Vec<VectorHit> {
    let mut scored: Vec<(f32, &VectorEntry)> = index
        .entries
        .values()
        .map(|e| (cosine(query_embedding, &e.embedding), e))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .filter(|(s, _)| *s > 0.0)
        .map(|(s, e)| VectorHit {
            project: e.project.clone(),
            domain: e.domain.clone(),
            score: s,
        })
        .collect()
}
