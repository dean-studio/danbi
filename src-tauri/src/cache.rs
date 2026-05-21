//! Incremental-build cache for expensive LLM passes.
//!
//! The design goal is the same idea llm_wiki uses — if the source files
//! haven't changed since the last run, reuse the stored result and skip
//! the LLM round-trip entirely. We implement it independently against
//! content hashes so it composes cleanly with the rest of Danbi.
//!
//! Storage: `<vault>/.danbi/cache/<kind>.json`. Each file is a
//! `HashMap<FileKey, CacheEntry>` where `FileKey` is "project/domain".
//!
//! `CacheEntry.content_hash` is a SHA-1 (via git2's fast blob hash) of
//! the file body. Compared to mtime, a hash is robust against clock
//! skew and symlink games; compared to SHA-256 it's half the bytes and
//! plenty collision-resistant for this purpose (we're not doing crypto).

use crate::error::DanbiResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CACHE_DIR: &str = ".danbi/cache";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CacheEntry {
    /// Stringified git blob SHA-1 of the file's bytes.
    pub content_hash: String,
    /// Unix seconds of the last time this entry was refreshed.
    pub last_run_at: i64,
}

pub type CacheMap = HashMap<String, CacheEntry>;

fn cache_path(vault: &Path, kind: &str) -> PathBuf {
    vault.join(CACHE_DIR).join(format!("{kind}.json"))
}

pub fn load(vault: &Path, kind: &str) -> DanbiResult<CacheMap> {
    let path = cache_path(vault, kind);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }
    // Cache corruption is recoverable — just start fresh so a user
    // isn't blocked by a bad JSON file.
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

pub fn save(vault: &Path, kind: &str, map: &CacheMap) -> DanbiResult<()> {
    let path = cache_path(vault, kind);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(map)?;
    std::fs::write(&path, body)?;
    Ok(())
}

/// Computes the git-blob SHA-1 of the given bytes. Matches what the
/// vault's existing git repo already uses for object naming, so we're
/// reusing a well-tested hasher instead of pulling in `sha2`.
pub fn content_hash(bytes: &[u8]) -> String {
    // git blob hash = sha1("blob <len>\0<content>"). git2 exposes this
    // directly via `Oid::hash_object` (for the blob kind).
    match git2::Oid::hash_object(git2::ObjectType::Blob, bytes) {
        Ok(oid) => oid.to_string(),
        // Falls back to a deterministic marker so the cache still
        // functions (comparison will simply treat the file as "unknown"
        // and force a rerun, which is the safe direction).
        Err(_) => format!("unhashable-{}", bytes.len()),
    }
}

/// Returns true when the cached entry's hash matches `current_hash`.
pub fn is_fresh(map: &CacheMap, key: &str, current_hash: &str) -> bool {
    match map.get(key) {
        Some(e) => e.content_hash == current_hash,
        None => false,
    }
}

pub fn mark(map: &mut CacheMap, key: String, content_hash: String) {
    map.insert(
        key,
        CacheEntry {
            content_hash,
            last_run_at: chrono::Local::now().timestamp(),
        },
    );
}

// ---- Content-addressed blob cache ----
//
// For callers that want to reuse the LLM's actual output (not just a
// boolean "did we already compute this"), we store the payload keyed by
// its input fingerprint. Used by briefing + compound so identical
// requests skip the roundtrip entirely.
//
// Layout: `<vault>/.danbi/cache/blobs/<kind>/<fingerprint>.txt`

fn blob_dir(vault: &Path, kind: &str) -> PathBuf {
    vault.join(CACHE_DIR).join("blobs").join(kind)
}

pub fn load_blob(vault: &Path, kind: &str, fingerprint: &str) -> Option<String> {
    let path = blob_dir(vault, kind).join(format!("{fingerprint}.txt"));
    std::fs::read_to_string(&path).ok()
}

pub fn save_blob(
    vault: &Path,
    kind: &str,
    fingerprint: &str,
    body: &str,
) -> DanbiResult<()> {
    let dir = blob_dir(vault, kind);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{fingerprint}.txt"));
    std::fs::write(&path, body)?;
    Ok(())
}

/// Removes every cache file under `<vault>/.danbi/cache/`. Used by the
/// "Clear cache" button in Settings for users who want a clean slate
/// (e.g. after switching models or debugging).
pub fn clear_all(vault: &Path) -> DanbiResult<()> {
    let dir = vault.join(CACHE_DIR);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}
