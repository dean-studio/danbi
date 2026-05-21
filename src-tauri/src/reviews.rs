//! Review queue — the "LLM flagged this for human judgment" inbox.
//!
//! Whenever Danbi does something probabilistic (planner uncertain, healing
//! suggests destructive action, ghost scan produces a borderline pair) it
//! can enqueue a review item instead of acting. The UI surfaces the queue
//! in the sidebar so the user can periodically triage.
//!
//! Storage: `<vault>/.danbi/reviews.json`. A single file keeps things
//! portable and easy to diff in git. Items are tagged with a stable id so
//! the UI can update/resolve without refetching the list.

use crate::error::{DanbiError, DanbiResult};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const REVIEW_FILE: &str = ".danbi/reviews.json";
const MAX_ITEMS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    Resolved,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: String,
    /// "unclear_destination" | "suggest_split" | "duplicate_content"
    /// | "broken_link" | "low_confidence_plan" | "ghost_candidate"
    /// — kept as a free-form string so new kinds don't need a schema
    /// migration.
    pub kind: String,
    pub project: Option<String>,
    pub domain: Option<String>,
    pub reason: String,
    pub status: ReviewStatus,
    pub created_at: i64,
    #[serde(default)]
    pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReviewStore {
    #[serde(default)]
    pub items: Vec<ReviewItem>,
}

fn store_path(vault: &Path) -> PathBuf {
    vault.join(REVIEW_FILE)
}

pub fn load(vault: &Path) -> DanbiResult<ReviewStore> {
    let path = store_path(vault);
    if !path.exists() {
        return Ok(ReviewStore::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(ReviewStore::default());
    }
    serde_json::from_str(&raw).map_err(|e| DanbiError::Other(format!("reviews parse: {e}")))
}

pub fn save(vault: &Path, store: &ReviewStore) -> DanbiResult<()> {
    let path = store_path(vault);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(store)?;
    std::fs::write(&path, body)?;
    Ok(())
}

fn rand_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

pub fn enqueue(
    vault: &Path,
    kind: &str,
    project: Option<String>,
    domain: Option<String>,
    reason: String,
) -> DanbiResult<ReviewItem> {
    let mut store = load(vault)?;
    // Cheap deduplication — if the same (kind, project, domain, reason)
    // is already pending, don't add a second copy.
    if store.items.iter().any(|it| {
        it.status == ReviewStatus::Pending
            && it.kind == kind
            && it.project == project
            && it.domain == domain
            && it.reason == reason
    }) {
        return Ok(store
            .items
            .iter()
            .find(|it| {
                it.status == ReviewStatus::Pending
                    && it.kind == kind
                    && it.project == project
                    && it.domain == domain
                    && it.reason == reason
            })
            .cloned()
            .unwrap());
    }
    let item = ReviewItem {
        id: rand_id(),
        kind: kind.to_string(),
        project,
        domain,
        reason,
        status: ReviewStatus::Pending,
        created_at: Local::now().timestamp(),
        resolved_at: None,
    };
    store.items.push(item.clone());
    // Drop oldest resolved/dismissed entries when we cross MAX_ITEMS so
    // the JSON file doesn't grow unbounded.
    if store.items.len() > MAX_ITEMS {
        store.items.sort_by(|a, b| {
            (a.status == ReviewStatus::Pending, b.created_at)
                .cmp(&(b.status == ReviewStatus::Pending, a.created_at))
        });
        store.items.truncate(MAX_ITEMS);
    }
    save(vault, &store)?;
    Ok(item)
}

pub fn resolve(vault: &Path, id: &str, status: ReviewStatus) -> DanbiResult<ReviewStore> {
    let mut store = load(vault)?;
    let now = Local::now().timestamp();
    for it in store.items.iter_mut() {
        if it.id == id {
            it.status = status.clone();
            it.resolved_at = Some(now);
            break;
        }
    }
    save(vault, &store)?;
    Ok(store)
}

pub fn pending_count(store: &ReviewStore) -> usize {
    store
        .items
        .iter()
        .filter(|it| it.status == ReviewStatus::Pending)
        .count()
}
