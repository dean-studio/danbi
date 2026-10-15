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
use std::sync::{Arc, Mutex};

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

/// Reads the index from disk every time. Reindexing wants an owned copy
/// it can mutate; everything on the search path should use `load_cached`.
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

/// (path, mtime, size) of vectors.json — what the in-memory copy was
/// parsed from. A reindex (or a sync tool replacing the file) changes
/// the mtime/size, so the next `load_cached` re-reads.
type IndexFileKey = (PathBuf, Option<std::time::SystemTime>, u64);

fn index_file_key(vault: &Path) -> IndexFileKey {
    let path = index_path(vault);
    let meta = std::fs::metadata(&path).ok();
    (
        path,
        meta.as_ref().and_then(|m| m.modified().ok()),
        meta.as_ref().map(|m| m.len()).unwrap_or(0),
    )
}

static INDEX_CACHE: Mutex<Option<(IndexFileKey, Arc<VectorIndex>)>> = Mutex::new(None);

/// The index parsed once and shared until vectors.json changes on disk.
/// Parsing a 23MB file costs ~30ms (release) / hundreds of ms (debug) —
/// too much to pay on every keystroke-driven search.
pub fn load_cached(vault: &Path) -> DanbiResult<Arc<VectorIndex>> {
    let key = index_file_key(vault);
    if let Ok(guard) = INDEX_CACHE.lock() {
        if let Some((k, idx)) = guard.as_ref() {
            if *k == key {
                return Ok(idx.clone());
            }
        }
    }
    let idx = Arc::new(load(vault)?);
    // Re-stat after the read: if the file changed underneath us, cache
    // under the key we actually parsed so the next call re-reads.
    let key = if index_file_key(vault) == key { key } else { (key.0, None, 0) };
    if let Ok(mut guard) = INDEX_CACHE.lock() {
        *guard = Some((key, idx.clone()));
    }
    Ok(idx)
}

fn invalidate_cache(vault: &Path) {
    let path = index_path(vault);
    if let Ok(mut guard) = INDEX_CACHE.lock() {
        if guard.as_ref().map(|(k, _)| k.0 == path).unwrap_or(false) {
            *guard = None;
        }
    }
}

/// Compact JSON (pretty was 3× the bytes for a file nobody reads by
/// hand), written to a temp file and renamed so a crash mid-write can't
/// leave a truncated index — and so the mtime-keyed caches never see a
/// half-written file.
pub fn save(vault: &Path, index: &VectorIndex) -> DanbiResult<()> {
    let path = index_path(vault);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(index)?)?;
    std::fs::rename(&tmp, &path)?;
    invalidate_cache(vault);
    Ok(())
}

pub fn clear(vault: &Path) -> DanbiResult<()> {
    let path = index_path(vault);
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    invalidate_cache(vault);
    Ok(())
}

// ---------- Query embedding LRU ----------
//
// The same query gets embedded again and again: every keystroke-debounced
// app search, every `danbi_context` retry, every agent that re-asks the
// question it just asked. Keying on provider + model keeps a model switch
// from serving vectors of the wrong dimension.

const QUERY_CACHE_CAP: usize = 256;

#[derive(Default)]
struct QueryLru {
    map: HashMap<String, Vec<f32>>,
    /// Least recently used at the front.
    order: std::collections::VecDeque<String>,
}

impl QueryLru {
    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.to_string());
    }

    fn get(&mut self, key: &str) -> Option<Vec<f32>> {
        let hit = self.map.get(key).cloned()?;
        self.touch(key);
        Some(hit)
    }

    fn put(&mut self, key: String, emb: Vec<f32>) {
        if self.map.insert(key.clone(), emb).is_none() {
            while self.map.len() > QUERY_CACHE_CAP {
                let Some(evicted) = self.order.pop_front() else { break };
                self.map.remove(&evicted);
            }
        }
        self.touch(&key);
    }
}

static QUERY_CACHE: Mutex<Option<QueryLru>> = Mutex::new(None);

pub fn query_cache_key(provider_kind: &str, model: &str, query: &str) -> String {
    format!("{provider_kind}\u{1}{model}\u{1}{}", query.trim())
}

pub fn query_cache_get(key: &str) -> Option<Vec<f32>> {
    QUERY_CACHE
        .lock()
        .ok()?
        .get_or_insert_with(QueryLru::default)
        .get(key)
}

pub fn query_cache_put(key: String, embedding: Vec<f32>) {
    if let Ok(mut guard) = QUERY_CACHE.lock() {
        guard.get_or_insert_with(QueryLru::default).put(key, embedding);
    }
}

/// Simple stats shown in Settings so the user knows how fresh / big
/// the index is.
#[derive(Debug, Clone, Serialize)]
pub struct VectorIndexStats {
    pub count: usize,
    pub oldest: Option<i64>,
    pub newest: Option<i64>,
    pub model: Option<String>,
    /// Entry count per embedding model, most common first.
    #[serde(default)]
    pub by_model: Vec<ModelCount>,
    /// Model the next query / reindex would use, when one is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_model: Option<String>,
    /// Entries embedded with some other model — invisible to semantic
    /// search until a full reindex.
    pub other_count: usize,
    pub mixed: bool,
    /// Ready-made sentence for the Settings panel; None when consistent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// `stats` plus the mixed-model census against the configured model.
pub fn stats_with_model(index: &VectorIndex, configured: Option<&str>) -> VectorIndexStats {
    let status = model_status(&model_counts(index), configured);
    let mut out = stats(index);
    out.warning = mixed_warning(&status);
    out.configured_model = status.configured_model;
    out.by_model = status.by_model;
    out.other_count = status.other_count;
    out.mixed = status.mixed;
    out
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
        by_model: Vec::new(),
        configured_model: None,
        other_count: 0,
        mixed: false,
        warning: None,
    }
}

// ---------- Mixed-model detection ----------
//
// Each entry remembers the model that embedded it. After the user
// switches embedding models, entries from the old model linger until a
// full reindex — and they silently drop out of semantic search (different
// dimension → cosine 0, same dimension → meaningless scores).

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelCount {
    pub model: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorModelStatus {
    /// Model the next query / reindex would use. None when no embedding
    /// provider is configured.
    pub configured_model: Option<String>,
    pub total: usize,
    /// Entry count per model, most common first.
    pub by_model: Vec<ModelCount>,
    /// Entries embedded with a model other than `configured_model` (or,
    /// with nothing configured, other than the most common one).
    pub other_count: usize,
    pub mixed: bool,
}

pub fn model_counts(index: &VectorIndex) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for e in index.entries.values() {
        *counts.entry(e.model.clone()).or_default() += 1;
    }
    counts
}

pub fn model_status(
    counts: &HashMap<String, usize>,
    configured: Option<&str>,
) -> VectorModelStatus {
    let mut by_model: Vec<ModelCount> = counts
        .iter()
        .map(|(m, c)| ModelCount { model: m.clone(), count: *c })
        .collect();
    by_model.sort_by(|a, b| b.count.cmp(&a.count).then(a.model.cmp(&b.model)));
    let total: usize = by_model.iter().map(|m| m.count).sum();
    let reference = configured
        .map(|m| counts.get(m).copied().unwrap_or(0))
        .unwrap_or_else(|| by_model.first().map(|m| m.count).unwrap_or(0));
    let other_count = total - reference;
    VectorModelStatus {
        configured_model: configured.map(str::to_string),
        total,
        by_model,
        other_count,
        mixed: other_count > 0,
    }
}

/// One-line warning for MCP output, None when the index is consistent.
pub fn mixed_warning(status: &VectorModelStatus) -> Option<String> {
    status.mixed.then(|| {
        format!(
            "의미 검색 색인에 다른 모델로 만든 문서 {}개가 섞여 있어요 — 이 문서들은 의미 검색에 안 잡혀요. 단비 설정 → AI 연동에서 전체 재색인이 필요해요.",
            status.other_count
        )
    })
}

/// Per-model counts, re-read only when vectors.json changes on disk —
/// the MCP search path asks on every call and the index can be tens of MB.
pub fn cached_model_counts(vault: &Path) -> HashMap<String, usize> {
    type Key = (PathBuf, Option<std::time::SystemTime>, u64);
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(Key, HashMap<String, usize>)>>> =
        std::sync::OnceLock::new();
    let path = index_path(vault);
    let meta = std::fs::metadata(&path).ok();
    let key: Key = (
        path,
        meta.as_ref().and_then(|m| m.modified().ok()),
        meta.as_ref().map(|m| m.len()).unwrap_or(0),
    );
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(guard) = cache.lock() {
        if let Some((k, counts)) = guard.as_ref() {
            if *k == key {
                return counts.clone();
            }
        }
    }
    let counts = load_cached(vault).map(|i| model_counts(&i)).unwrap_or_default();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((key, counts.clone()));
    }
    counts
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

#[cfg(test)]
mod tests {
    use super::*;

    fn index_with(models: &[(&str, usize)]) -> VectorIndex {
        let mut index = VectorIndex::default();
        for (model, n) in models {
            for i in 0..*n {
                let id = format!("p/{model}-{i}.md");
                index.entries.insert(
                    id.clone(),
                    VectorEntry {
                        id,
                        project: "p".into(),
                        domain: format!("{model}-{i}.md"),
                        content_hash: String::new(),
                        model: model.to_string(),
                        embedding: vec![1.0],
                        updated_at: 0,
                    },
                );
            }
        }
        index
    }

    fn temp_vault(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "danbi-vector-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn load_cached_follows_save_clear_and_external_writes() {
        let v = temp_vault("cache");
        assert_eq!(load_cached(&v).unwrap().entries.len(), 0);

        save(&v, &index_with(&[("m1", 3)])).unwrap();
        let a = load_cached(&v).unwrap();
        assert_eq!(a.entries.len(), 3);
        assert!(Arc::ptr_eq(&a, &load_cached(&v).unwrap()), "same Arc while unchanged");

        // Another process replacing the file (sync tool, older build) must
        // be picked up too — key is mtime + size, not just our own save().
        let mut pretty = index_with(&[("m1", 3), ("m2", 2)]);
        pretty.entries.values_mut().for_each(|e| e.embedding = vec![0.5; 4]);
        std::fs::write(index_path(&v), serde_json::to_string_pretty(&pretty).unwrap()).unwrap();
        let b = load_cached(&v).unwrap();
        assert_eq!(b.entries.len(), 5, "pretty (legacy) file still parses");
        assert_eq!(b.entries.values().next().unwrap().embedding.len(), 4);

        clear(&v).unwrap();
        assert_eq!(load_cached(&v).unwrap().entries.len(), 0);
        assert!(!index_path(&v).with_extension("json.tmp").exists(), "no temp file left behind");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn save_is_compact_and_round_trips() {
        let v = temp_vault("compact");
        let idx = index_with(&[("m", 2)]);
        save(&v, &idx).unwrap();
        let raw = std::fs::read_to_string(index_path(&v)).unwrap();
        assert!(!raw.contains('\n'), "compact JSON");
        assert_eq!(load(&v).unwrap().entries.len(), 2);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn query_lru_evicts_least_recently_used() {
        let mut lru = QueryLru::default();
        for i in 0..QUERY_CACHE_CAP {
            lru.put(format!("k{i}"), vec![i as f32]);
        }
        assert!(lru.get("k0").is_some(), "touch k0 so k1 is the oldest");
        lru.put("fresh".into(), vec![9.0]);
        assert!(lru.get("k1").is_none(), "k1 evicted");
        assert!(lru.get("k0").is_some());
        assert!(lru.get("fresh").is_some());
        assert_eq!(lru.map.len(), QUERY_CACHE_CAP);
        assert_eq!(lru.order.len(), QUERY_CACHE_CAP);
        // Re-putting an existing key updates in place, no growth.
        lru.put("k0".into(), vec![1.0]);
        assert_eq!(lru.map.len(), QUERY_CACHE_CAP);
        assert_eq!(lru.get("k0"), Some(vec![1.0]));
    }

    #[test]
    fn query_cache_key_separates_provider_model_and_trims() {
        assert_eq!(query_cache_key("a", "m", " q "), query_cache_key("a", "m", "q"));
        assert_ne!(query_cache_key("a", "m1", "q"), query_cache_key("a", "m2", "q"));
        assert_ne!(query_cache_key("a", "m", "q"), query_cache_key("b", "m", "q"));
    }

    #[test]
    fn single_model_index_is_not_mixed() {
        let counts = model_counts(&index_with(&[("m1", 4)]));
        let s = model_status(&counts, Some("m1"));
        assert!(!s.mixed);
        assert_eq!(s.other_count, 0);
        assert_eq!(s.total, 4);
        assert!(mixed_warning(&s).is_none());
    }

    #[test]
    fn entries_from_another_model_are_counted_against_configured() {
        let counts = model_counts(&index_with(&[("new", 5), ("old", 3)]));
        let s = model_status(&counts, Some("new"));
        assert!(s.mixed);
        assert_eq!(s.other_count, 3);
        assert_eq!(s.by_model[0], ModelCount { model: "new".into(), count: 5 });
        assert!(mixed_warning(&s).unwrap().contains("3개"));

        // Switched model, nothing reindexed yet: everything is "other".
        let s = model_status(&counts, Some("newest"));
        assert_eq!(s.other_count, 8);
        assert!(s.mixed);
    }

    #[test]
    fn stats_with_model_carries_the_census() {
        let s = stats_with_model(&index_with(&[("new", 5), ("old", 3)]), Some("new"));
        assert_eq!(s.count, 8);
        assert!(s.mixed);
        assert_eq!(s.other_count, 3);
        assert_eq!(s.configured_model.as_deref(), Some("new"));
        assert!(s.warning.unwrap().contains("3개"));
        let s = stats_with_model(&index_with(&[("new", 5)]), Some("new"));
        assert!(!s.mixed && s.warning.is_none());
    }

    #[test]
    fn without_configured_model_majority_is_the_reference() {
        let counts = model_counts(&index_with(&[("a", 2), ("b", 6)]));
        let s = model_status(&counts, None);
        assert!(s.mixed);
        assert_eq!(s.other_count, 2);
        assert!(!model_status(&HashMap::new(), Some("a")).mixed, "empty index");
    }
}

