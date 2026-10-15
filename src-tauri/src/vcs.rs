use crate::error::{DanbiError, DanbiResult};
use git2::{Commit, IndexAddOption, Repository, Signature};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

const DEFAULT_AUTHOR_NAME: &str = "danbi";
const DEFAULT_AUTHOR_EMAIL: &str = "danbi@local";

/// Paths that are working state, not knowledge: the vector index, caches,
/// ghost/goal stores under `.danbi/` and the app config that saves itself
/// on every click. Keeping them out of snapshots halves the commit count
/// and stops multi-MB index blobs piling up in `.git`.
const IGNORED_PATTERNS: [&str; 2] = [".danbi/", "/config.json"];

/// One process-wide lock for every path that mutates the vault — MCP write
/// tools, the REST PUT, the popover quick log, undo. A file-level lock is
/// overkill for a few dozen writes a day; what matters is that two agents
/// can't interleave read → modify → write on the same daily note and that
/// git never sees two commits racing for `index.lock`.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Takes the vault write lock. A poisoned lock (a writer panicked) is
/// recovered rather than propagated — the vault itself is still fine.
pub fn write_guard() -> MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Pre-edit snapshots carry this marker; they exist only so undo has a
/// known state and should never count as user activity.
pub fn is_pre_snapshot(summary: &str) -> bool {
    if summary.contains("(pre)") {
        return true;
    }
    // Older writers spelled it differently: `quick-capture pre · …`,
    // `capture pre · …`, `pre-edit snapshot · …`, `pre-compound · …`.
    let head = summary_head(summary);
    head.ends_with(" pre") || head.starts_with("pre-")
}

/// First `·`-separated segment after the `danbi:` prefix — the verb.
fn summary_head(summary: &str) -> &str {
    let body = summary
        .trim()
        .strip_prefix("danbi:")
        .map(str::trim)
        .unwrap_or(summary);
    body.split('·').next().unwrap_or("").trim()
}

// ---------- Origin token ----------------------------------------------------
//
// Every writer appends ` [client]` to its commit summary so the activity
// feed can say who wrote: a UA bucket for agents (`claude-code`, `codex`,
// `cursor`), `human` for the app's own editor/popover/undo, `danbi` for
// automated jobs. The token sits last so the older `·`-split parsers and
// the `(pre)` check keep working on the part before it.

pub const CLIENT_HUMAN: &str = "human";
pub const CLIENT_DANBI: &str = "danbi";
/// Agent whose client could not be identified (REST from curl, old UA).
pub const CLIENT_AGENT: &str = "agent";

/// Normalises a `usage::classify_user_agent` bucket into a commit token.
pub fn client_token(ua_bucket: &str) -> String {
    match ua_bucket {
        "" | "unknown" => CLIENT_AGENT.to_string(),
        other => other.replace('_', "-"),
    }
}

/// `msg` with the origin token appended.
pub fn with_client(msg: &str, client: &str) -> String {
    let client = client.trim();
    if client.is_empty() || msg.ends_with(&format!(" [{client}]")) {
        return msg.to_string();
    }
    format!("{msg} [{client}]")
}

/// Splits a summary into (summary without token, token).
pub fn split_client(summary: &str) -> (&str, Option<&str>) {
    let s = summary.trim_end();
    if let Some(open) = s.rfind(" [") {
        if s.ends_with(']') {
            let token = &s[open + 2..s.len() - 1];
            let ok = !token.is_empty()
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if ok {
                return (&s[..open], Some(token));
            }
        }
    }
    (s, None)
}

/// `snapshot` with the origin token on the message.
pub fn snapshot_as(vault: &Path, msg: &str, client: &str) -> DanbiResult<Option<String>> {
    snapshot(vault, &with_client(msg, client))
}

fn open_or_init(path: &Path) -> DanbiResult<Repository> {
    match Repository::open(path) {
        Ok(r) => Ok(r),
        Err(_) => Repository::init(path).map_err(|e| DanbiError::Other(format!("git init: {e}"))),
    }
}

/// Ensures the vault is a git repo and has at least one commit so we can reset
/// to a known state later.
pub fn ensure_repo(vault: &Path) -> DanbiResult<()> {
    let repo = open_or_init(vault)?;

    ensure_gitignore(vault)?;

    if repo.head().is_err() {
        commit_all(&repo, "danbi: initialize vault")?;
    }
    // Reads call this too (list_tree → init_vault), so a transient git
    // failure here (another process holding index.lock) must not fail
    // them — the migration simply runs on the next call.
    if let Err(e) = untrack_ignored(&repo) {
        eprintln!("[vcs] .danbi/config.json untrack deferred: {e}");
    }
    Ok(())
}

/// Writes the managed `.gitignore` on first run and appends any missing
/// managed pattern to an existing one. Lines the user added are kept.
fn ensure_gitignore(vault: &Path) -> DanbiResult<()> {
    let gitignore_path = vault.join(".gitignore");
    let mut text = if gitignore_path.exists() {
        std::fs::read_to_string(&gitignore_path)?
    } else {
        "# managed by danbi\n.DS_Store\n*.tmp\n".to_string()
    };
    let missing: Vec<&str> = IGNORED_PATTERNS
        .iter()
        .copied()
        .filter(|p| !text.lines().any(|l| l.trim() == *p))
        .collect();
    if missing.is_empty() && gitignore_path.exists() {
        return Ok(());
    }
    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    for p in missing {
        text.push_str(p);
        text.push('\n');
    }
    std::fs::write(&gitignore_path, text)?;
    Ok(())
}

/// One-time migration for vaults that already committed `.danbi/` or
/// `config.json`: drop them from the index (`git rm --cached`) and record
/// that as its own commit. History is left untouched. No-op once the index
/// is clean, so calling it on every `ensure_repo` is cheap.
fn untrack_ignored(repo: &Repository) -> DanbiResult<()> {
    let mut index = repo
        .index()
        .map_err(|e| DanbiError::Other(format!("git index: {e}")))?;
    let tracked: Vec<Vec<u8>> = index
        .iter()
        .filter(|e| is_ignored_index_path(&e.path))
        .map(|e| e.path)
        .collect();
    if tracked.is_empty() {
        return Ok(());
    }
    for path in &tracked {
        let p = Path::new(std::str::from_utf8(path).unwrap_or(""));
        index
            .remove_path(p)
            .map_err(|e| DanbiError::Other(format!("git rm --cached: {e}")))?;
    }
    // The freshly extended .gitignore belongs in the same commit.
    let _ = index.add_path(Path::new(".gitignore"));
    index
        .write()
        .map_err(|e| DanbiError::Other(format!("git index write: {e}")))?;
    commit_index(repo, &mut index, "danbi: 스냅샷 범위 정리 · .danbi/ · config.json 추적 해제")?;
    Ok(())
}

fn is_ignored_index_path(path: &[u8]) -> bool {
    path == b"config.json"
        || path.starts_with(b".danbi/")
        || path.windows(8).any(|w| w == b"/.danbi/")
}

fn signature<'a>() -> DanbiResult<Signature<'a>> {
    Signature::now(DEFAULT_AUTHOR_NAME, DEFAULT_AUTHOR_EMAIL)
        .map_err(|e| DanbiError::Other(format!("git signature: {e}")))
}

fn commit_all(repo: &Repository, msg: &str) -> DanbiResult<Option<git2::Oid>> {
    let mut index = repo
        .index()
        .map_err(|e| DanbiError::Other(format!("git index: {e}")))?;
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .map_err(|e| DanbiError::Other(format!("git add: {e}")))?;
    // Files deleted in the working tree have to leave the index too, or
    // the next tree still carries them and undo can't see the deletion.
    index
        .update_all(["*"].iter(), None)
        .map_err(|e| DanbiError::Other(format!("git update: {e}")))?;
    index
        .write()
        .map_err(|e| DanbiError::Other(format!("git index write: {e}")))?;
    commit_index(repo, &mut index, msg)
}

/// Commits whatever the index holds. `None` when the tree equals HEAD's.
fn commit_index(
    repo: &Repository,
    index: &mut git2::Index,
    msg: &str,
) -> DanbiResult<Option<git2::Oid>> {
    let tree_id = index
        .write_tree()
        .map_err(|e| DanbiError::Other(format!("git write tree: {e}")))?;
    let tree = repo
        .find_tree(tree_id)
        .map_err(|e| DanbiError::Other(format!("git find tree: {e}")))?;

    let sig = signature()?;
    let parents: Vec<Commit> = match repo.head() {
        Ok(head_ref) => {
            let parent = head_ref
                .peel_to_commit()
                .map_err(|e| DanbiError::Other(format!("git head peel: {e}")))?;
            // Skip empty commits — nothing changed.
            if parent.tree_id() == tree_id {
                return Ok(None);
            }
            vec![parent]
        }
        Err(_) => Vec::new(),
    };
    let parent_refs: Vec<&Commit> = parents.iter().collect();

    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
        .map_err(|e| DanbiError::Other(format!("git commit: {e}")))?;
    Ok(Some(oid))
}

/// Records the current state as a commit. Returns the new commit id if a
/// commit was actually created (None if there was nothing to record).
pub fn snapshot(vault: &Path, msg: &str) -> DanbiResult<Option<String>> {
    let repo = open_or_init(vault)?;
    let oid = commit_all(&repo, msg)?;
    Ok(oid.map(|o| o.to_string()))
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct CommitSummary {
    pub id: String,
    pub summary: String,
    pub ts: i64, // seconds since epoch
}

/// Returns the newest N commits on the current branch.
pub fn recent_commits(vault: &Path, max: usize) -> DanbiResult<Vec<CommitSummary>> {
    let repo = match Repository::open(vault) {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };
    if revwalk.push_head().is_err() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (i, oid) in revwalk.flatten().enumerate() {
        if i >= max {
            break;
        }
        if let Ok(c) = repo.find_commit(oid) {
            out.push(CommitSummary {
                id: c.id().to_string(),
                summary: c.summary().unwrap_or("").to_string(),
                ts: c.time().seconds(),
            });
        }
    }
    Ok(out)
}

/// Commits newer than `since` (unix seconds), newest first, pre-edit
/// snapshots excluded. Stops walking at the first older commit, so the
/// cost tracks the window rather than the repository size.
pub fn commits_since(vault: &Path, since: i64) -> DanbiResult<Vec<CommitSummary>> {
    let repo = match Repository::open(vault) {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };
    if revwalk.push_head().is_err() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for oid in revwalk.flatten() {
        let Ok(c) = repo.find_commit(oid) else { continue; };
        if c.time().seconds() < since {
            break;
        }
        let summary = c.summary().unwrap_or("").to_string();
        if is_pre_snapshot(&summary) {
            continue;
        }
        out.push(CommitSummary {
            id: c.id().to_string(),
            summary,
            ts: c.time().seconds(),
        });
    }
    Ok(out)
}

/// Counts commits since `since_ts` (unix seconds) that touched files under
/// `Projects/<project>/`, grouped by project. The return map excludes
/// projects with zero updates so callers can iterate it directly.
///
/// This is intentionally lightweight — we walk HEAD's ancestry breadth-
/// first but bail as soon as we pass the timestamp cutoff, and we only
/// inspect the commit's tree diff against its first parent (no merge
/// sophistication). For a vault with a few thousand commits this is
/// well under 50ms.
pub fn commits_per_project_since(
    vault: &Path,
    since: &std::collections::HashMap<String, i64>,
) -> DanbiResult<std::collections::HashMap<String, u32>> {
    let mut out: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    let repo = match Repository::open(vault) {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    if revwalk.push_head().is_err() {
        return Ok(out);
    }

    // The cutoff is the earliest "since" across all projects — past it
    // we stop walking entirely.
    let global_cutoff = since.values().copied().min().unwrap_or(0);

    for oid in revwalk.flatten() {
        let Ok(commit) = repo.find_commit(oid) else { continue; };
        let ts = commit.time().seconds();
        if ts < global_cutoff {
            break;
        }
        // Pre-edit snapshots would double every write.
        if is_pre_snapshot(commit.summary().unwrap_or("")) {
            continue;
        }
        // Diff against the first parent. Initial commit → diff against
        // the empty tree so the very first commit's paths still count.
        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let parent_tree = commit
            .parent(0)
            .ok()
            .and_then(|p| p.tree().ok());
        let diff = match repo.diff_tree_to_tree(
            parent_tree.as_ref(),
            Some(&tree),
            None,
        ) {
            Ok(d) => d,
            Err(_) => continue,
        };
        // A commit can touch multiple projects — but we only want to
        // count it once per project.
        let mut hit: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let _ = diff.foreach(
            &mut |delta, _| {
                let path = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path());
                if let Some(p) = path {
                    if let Some(proj) = project_from_path(p) {
                        hit.insert(proj);
                    }
                }
                true
            },
            None,
            None,
            None,
        );
        for proj in hit {
            // Skip if this commit is older than the project-specific cutoff.
            let cutoff = since.get(&proj).copied().unwrap_or(0);
            if ts < cutoff {
                continue;
            }
            *out.entry(proj).or_insert(0) += 1;
        }
    }
    Ok(out)
}

/// One row of a per-doc change history. `op` is best-effort classification
/// of what kind of write produced the commit — derived from the message
/// prefix that `mcp.rs` and `commands.rs` already emit (e.g. "danbi: mcp
/// upsert_item · …"). When the prefix isn't recognised we fall back to
/// `"edit"`.
#[derive(Debug, serde::Serialize, Clone)]
pub struct DocChangeEntry {
    pub commit: String,
    pub ts: i64,
    pub op: String,
    pub summary: String,
    /// `"update"` / `"add"` for upsert_item; otherwise None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Item key for upsert_item, or section heading for replace_section.
    /// Lets the UI render "이 항목" 라벨 없이도 어떤 부분이 바뀌었는지
    /// 보여줄 수 있다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// Recent changes touching exactly `Projects/<project>/<domain>` — most
/// recent first. Skipped (`pre`) snapshots filtered out so the UI only
/// shows committed states. Limited to the most recent `max` entries.
pub fn doc_history(
    vault: &Path,
    project: &str,
    domain: &str,
    max: usize,
) -> DanbiResult<Vec<DocChangeEntry>> {
    let mut out: Vec<DocChangeEntry> = Vec::new();
    let repo = match Repository::open(vault) {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    if revwalk.push_head().is_err() {
        return Ok(out);
    }

    let want_path = format!("Projects/{project}/{domain}");

    for oid in revwalk.flatten() {
        if out.len() >= max {
            break;
        }
        let Ok(commit) = repo.find_commit(oid) else { continue; };
        let summary = commit.summary().unwrap_or("").to_string();
        // Skip the pre-edit snapshots — they only exist so undo can
        // recover, never as a user-visible state.
        if is_pre_snapshot(&summary) {
            continue;
        }
        // Quick path check via diff name-only against first parent. Fall
        // through if the commit didn't touch our doc.
        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
        let diff = match repo.diff_tree_to_tree(
            parent_tree.as_ref(),
            Some(&tree),
            None,
        ) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let mut touched = false;
        let _ = diff.foreach(
            &mut |delta, _| {
                if touched {
                    return true;
                }
                let path = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path());
                if let Some(p) = path {
                    if p.to_string_lossy() == want_path {
                        touched = true;
                    }
                }
                true
            },
            None,
            None,
            None,
        );
        if !touched {
            continue;
        }

        let (op, mode, target) = classify_commit_summary(&summary);
        out.push(DocChangeEntry {
            commit: commit.id().to_string(),
            ts: commit.time().seconds(),
            op,
            summary,
            mode,
            target,
        });
    }

    Ok(out)
}

/// Recognise the summary patterns that `mcp.rs` / `commands.rs` emit and
/// peel off the operation kind plus any inline metadata. Pattern shapes:
///   `danbi: mcp upsert_item · <project>/<domain> · <mode> · <key>`
///   `danbi: mcp replace_section · <project>/<domain> · <heading>`
///   `danbi: mcp append · <project>/<domain>`
///   `danbi: <op_label> · <project>/<domain> · <summary>`  (UI edits)
fn classify_commit_summary(s: &str) -> (String, Option<String>, Option<String>) {
    let (s, _client) = split_client(s);
    // Tokens after "danbi:" prefix.
    let body = s
        .trim()
        .strip_prefix("danbi:")
        .map(|x| x.trim())
        .unwrap_or(s);
    let parts: Vec<&str> = body.split('·').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return ("edit".into(), None, None);
    }
    // First segment looks like "mcp upsert_item" or "mcp replace_section"
    // or "append" / "replace_section" / "upsert_item" / "rewrite_all" /
    // "insert_after" / "apply" / "undo" / "rest …" depending on origin.
    let head = parts[0];
    let (kind, _origin) = if let Some(rest) = head.strip_prefix("mcp ") {
        (rest.trim(), "mcp")
    } else if let Some(rest) = head.strip_prefix("rest ") {
        (rest.trim(), "rest")
    } else {
        (head, "ui")
    };

    // upsert_item picks up `mode` from parts[2] and `key` from parts[3].
    if kind == "upsert_item" {
        let mode = parts.get(2).map(|s| s.to_string());
        let target = parts.get(3).map(|s| s.to_string());
        return ("upsert_item".into(), mode, target);
    }
    if kind == "replace_section" {
        let target = parts.get(2).map(|s| s.to_string());
        return ("replace_section".into(), None, target);
    }
    (kind.to_string(), None, None)
}

/// Pull the top-level project folder out of a repo-relative path, e.g.
/// `Projects/Bonny/ui.md` → `Bonny`. Returns None for paths outside
/// `Projects/…`.
fn project_from_path(p: &std::path::Path) -> Option<String> {
    let mut comps = p.components();
    let first = comps.next()?.as_os_str().to_str()?;
    if first != "Projects" {
        return None;
    }
    let name = comps.next()?.as_os_str().to_str()?;
    Some(name.to_string())
}

const UNDO_PREFIX: &str = "danbi: undo · ";

/// The commit an undo commit reverted, parsed from its summary
/// (`danbi: undo · <oid> · <original summary>`).
fn undo_target_of(summary: &str) -> Option<git2::Oid> {
    let rest = summary.strip_prefix(UNDO_PREFIX)?;
    let oid = rest.split(" · ").next()?;
    git2::Oid::from_str(oid).ok()
}

/// Reverts the most recent change as a new commit: only the files that
/// commit touched go back to their previous content, everything else in
/// the working tree (notes another agent wrote meanwhile, unsaved editor
/// state) is left alone. Pressing undo again keeps walking back — an undo
/// commit remembers which commit it reverted, so the next call reverts
/// that one's parent rather than redoing. Returns the undo commit's id,
/// `None` when there is nothing left to undo.
pub fn undo_last(vault: &Path) -> DanbiResult<Option<String>> {
    let _guard = write_guard();
    let repo = open_or_init(vault)?;
    let head = match repo.head() {
        Ok(h) => h
            .peel_to_commit()
            .map_err(|e| DanbiError::Other(format!("git head peel: {e}")))?,
        Err(_) => return Ok(None),
    };

    // Cursor: HEAD itself, or — when HEAD is already an undo — the commit
    // before the one it reverted.
    let mut target = head.clone();
    while let Some(reverted) = undo_target_of(target.summary().unwrap_or("")) {
        let Ok(c) = repo.find_commit(reverted) else { break };
        match c.parent(0) {
            Ok(p) => target = p,
            Err(_) => return Ok(None),
        }
    }
    let Ok(parent) = target.parent(0) else {
        // Initial commit — nothing before it to go back to.
        return Ok(None);
    };
    let target_tree = target
        .tree()
        .map_err(|e| DanbiError::Other(format!("git tree: {e}")))?;
    let parent_tree = parent
        .tree()
        .map_err(|e| DanbiError::Other(format!("git tree: {e}")))?;
    let diff = repo
        .diff_tree_to_tree(Some(&parent_tree), Some(&target_tree), None)
        .map_err(|e| DanbiError::Other(format!("git diff: {e}")))?;

    let mut restored = 0usize;
    for delta in diff.deltas() {
        let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) else {
            continue;
        };
        // Paths that are ignored now (config.json, .danbi/) may still sit in
        // old commits; restoring them would resurrect stale state.
        if repo.is_path_ignored(path).unwrap_or(false) {
            continue;
        }
        let abs = vault.join(path);
        match parent_tree.get_path(path) {
            Ok(entry) => {
                let blob = repo
                    .find_blob(entry.id())
                    .map_err(|e| DanbiError::Other(format!("git blob: {e}")))?;
                if let Some(dir) = abs.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&abs, blob.content())?;
            }
            Err(_) => {
                // Added by the target commit → remove it again.
                if abs.exists() {
                    std::fs::remove_file(&abs)?;
                }
            }
        }
        restored += 1;
    }
    if restored == 0 {
        return Ok(None);
    }
    let msg = with_client(
        &format!("{UNDO_PREFIX}{} · {}", target.id(), target.summary().unwrap_or("")),
        CLIENT_HUMAN,
    );
    Ok(commit_all(&repo, &msg)?.map(|o| o.to_string()))
}

// ---------- Activity feed ----------------------------------------------------
//
// One row per real commit (pre-edit snapshots skipped), newest first, with
// who/what/where parsed from the summary and the touched paths from the
// tree diff. Cursor pagination by timestamp; the walk itself is bounded so
// a narrow project scope on a long history still returns quickly.

#[derive(Debug, serde::Serialize, Clone)]
pub struct ActivityEntry {
    pub id: String,
    pub ts: i64,
    /// Origin token: `claude-code` / `codex` / `cursor` / `agent` /
    /// `human` / `danbi`.
    pub client: String,
    /// `agent` | `human` | `danbi`.
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Normalised verb: `log`, `append`, `replace_section`, `upsert_item`,
    /// `save`, `quick_capture`, `undo`, `restore`, `revert`, `migration`…
    pub op: String,
    /// Section heading / item key / original summary, when the op has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// First line this commit added to the document (≤ 60 chars).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Repo-relative paths the commit touched (`Projects/<p>/<d>`).
    pub files: Vec<String>,
}

const FEED_MAX_LIMIT: usize = 200;
/// Commits inspected per call before giving up on filling `limit`.
const FEED_WALK_CAP: usize = 5_000;
const PREVIEW_CHARS: usize = 60;

/// Who a legacy (token-less) commit came from, by its verb.
fn actor_for(client: &str, head: &str) -> &'static str {
    match client {
        CLIENT_HUMAN => "human",
        CLIENT_DANBI => "danbi",
        CLIENT_AGENT => "agent",
        "" => {
            if head.starts_with("mcp ") || head.starts_with("rest ") {
                "agent"
            } else if head.starts_with("compound")
                || head.starts_with("weekly")
                || head.starts_with("initialize")
                || head.starts_with("스냅샷")
            {
                "danbi"
            } else {
                "human"
            }
        }
        _ => "agent",
    }
}

/// Verb for the feed: the classified op, with the few special commits
/// named explicitly.
fn feed_op(summary: &str, head: &str) -> String {
    if summary.starts_with(UNDO_PREFIX) {
        return "undo".into();
    }
    if head.starts_with("스냅샷 범위") {
        return "migration".into();
    }
    if head == "initialize vault" {
        return "init".into();
    }
    let (kind, _, _) = classify_commit_summary(summary);
    kind.split_whitespace()
        .next()
        .unwrap_or("edit")
        .replace('-', "_")
}

/// `<project>/<domain>` out of the summary's second segment.
fn doc_from_summary(summary: &str) -> Option<(String, String)> {
    let (s, _) = split_client(summary);
    let body = s.trim().strip_prefix("danbi:").map(str::trim).unwrap_or(s);
    let seg = body.split('·').nth(1)?.trim();
    let (p, d) = seg.split_once('/')?;
    if p.is_empty() || d.is_empty() || !d.ends_with(".md") {
        return None;
    }
    Some((p.to_string(), d.to_string()))
}

fn preview_line(old: Option<&str>, new: Option<&str>) -> Option<String> {
    let old_lines: std::collections::HashSet<&str> =
        old.map(|o| o.lines().map(str::trim).collect()).unwrap_or_default();
    let pick = |text: &str, skip: &std::collections::HashSet<&str>| -> Option<String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !skip.contains(l))
            // The automatic `#### HH:MM · client` stamp says nothing.
            .find(|l| !(l.starts_with("#### ") && l[5..].chars().take(5).filter(|c| c.is_ascii_digit() || *c == ':').count() == 5))
            .map(|l| {
                let mut out: String = l.chars().take(PREVIEW_CHARS).collect();
                if l.chars().count() > PREVIEW_CHARS {
                    out.push('…');
                }
                out
            })
    };
    match (old, new) {
        (_, Some(n)) => pick(n, &old_lines).or_else(|| {
            // Nothing added (pure removal / revert): show what went away.
            let new_lines: std::collections::HashSet<&str> =
                n.lines().map(str::trim).collect();
            old.and_then(|o| pick(o, &new_lines))
        }),
        (Some(o), None) => pick(o, &std::collections::HashSet::new()),
        (None, None) => None,
    }
}

pub fn activity_feed(
    vault: &Path,
    scope_project: Option<&str>,
    before: Option<i64>,
    limit: usize,
) -> DanbiResult<Vec<ActivityEntry>> {
    let limit = limit.clamp(1, FEED_MAX_LIMIT);
    let mut out = Vec::new();
    let repo = match Repository::open(vault) {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    if revwalk.push_head().is_err() {
        return Ok(out);
    }
    let scope_prefix = scope_project.map(|p| format!("Projects/{p}/"));

    for (walked, oid) in revwalk.flatten().enumerate() {
        if out.len() >= limit || walked >= FEED_WALK_CAP {
            break;
        }
        let Ok(commit) = repo.find_commit(oid) else { continue; };
        let ts = commit.time().seconds();
        if before.is_some_and(|b| ts >= b) {
            continue;
        }
        let summary = commit.summary().unwrap_or("").to_string();
        if is_pre_snapshot(&summary) {
            continue;
        }
        // Scoped walk: the summary names the document for every writer
        // path, so skip other projects before paying for a tree diff.
        if let (Some(scope), Some((p, _))) = (scope_project, doc_from_summary(&summary)) {
            if p != scope {
                continue;
            }
        }
        let Ok(tree) = commit.tree() else { continue; };
        let parent = commit.parent(0).ok();
        let parent_tree = parent.as_ref().and_then(|p| p.tree().ok());
        let Ok(diff) = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) else {
            continue;
        };
        let mut files: Vec<String> = diff
            .deltas()
            .filter_map(|d| d.new_file().path().or_else(|| d.old_file().path()))
            .map(|p| p.to_string_lossy().to_string())
            .filter(|p| p.starts_with("Projects/"))
            .collect();
        files.sort();
        if let Some(prefix) = scope_prefix.as_deref() {
            if !files.iter().any(|f| f.starts_with(prefix)) {
                continue;
            }
        }

        let (bare, token) = split_client(&summary);
        let head = summary_head(bare).to_string();
        let client = token.unwrap_or("").to_string();
        let actor = actor_for(&client, &head).to_string();
        let client = if client.is_empty() {
            if actor == "agent" { CLIENT_AGENT } else if actor == "danbi" { CLIENT_DANBI } else { CLIENT_HUMAN }.to_string()
        } else {
            client
        };
        let op = feed_op(bare, &head);
        let (_, _, target) = classify_commit_summary(bare);
        let target = if op == "undo" {
            bare.strip_prefix(UNDO_PREFIX).and_then(|r| r.split_once(" · ")).map(|(_, orig)| orig.to_string())
        } else {
            target
        };

        // The document this row is about: from the summary, else the first
        // in-scope markdown file the commit touched.
        let doc = doc_from_summary(bare).or_else(|| {
            files
                .iter()
                .filter(|f| scope_prefix.as_deref().map(|p| f.starts_with(p)).unwrap_or(true))
                .find(|f| f.ends_with(".md"))
                .and_then(|f| {
                    let rest = f.strip_prefix("Projects/")?;
                    let (p, d) = rest.split_once('/')?;
                    Some((p.to_string(), d.to_string()))
                })
        });
        let preview = doc.as_ref().and_then(|(p, d)| {
            let rel = format!("Projects/{p}/{d}");
            let new = blob_text(&repo, &tree, &rel).ok().flatten();
            let old = parent_tree
                .as_ref()
                .and_then(|t| blob_text(&repo, t, &rel).ok().flatten());
            preview_line(old.as_deref(), new.as_deref())
        });

        out.push(ActivityEntry {
            id: commit.id().to_string(),
            ts,
            client,
            actor,
            project: doc.as_ref().map(|(p, _)| p.clone()),
            domain: doc.as_ref().map(|(_, d)| d.clone()),
            op,
            target,
            preview,
            files,
        });
    }
    Ok(out)
}

// ---------- Single-change revert ---------------------------------------------

#[derive(Debug, serde::Serialize, Clone)]
pub struct RevertResult {
    /// False when later commits touched the file and `force` was not set —
    /// nothing was written; call again with `force` to proceed anyway.
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Commits after `commit` that also changed the file.
    pub later_commits: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Undoes what one commit did to one file: the file goes back to that
/// commit's parent version (or is removed if the commit created it) and
/// the result is a new commit — nothing else in the vault moves. When
/// newer commits changed the same file the caller is warned; without
/// `force` the revert is not applied.
pub fn revert_doc_change(
    vault: &Path,
    commit: &str,
    rel_path: &str,
    force: bool,
) -> DanbiResult<RevertResult> {
    let rel = rel_path.trim_matches('/');
    if !rel.starts_with("Projects/") || rel.split('/').any(|s| s == "..") {
        return Err(DanbiError::Config(format!(
            "path 는 Projects/<프로젝트>/<파일> 형식이어야 해요 (받은 값: {rel_path})"
        )));
    }
    let _guard = write_guard();
    let repo = Repository::open(vault)
        .map_err(|e| DanbiError::Other(format!("git open: {e}")))?;
    let target = resolve_commit(&repo, commit)?;
    let target_tree = target
        .tree()
        .map_err(|e| DanbiError::Other(format!("git tree: {e}")))?;
    let parent_tree = match target.parent(0) {
        Ok(p) => Some(p.tree().map_err(|e| DanbiError::Other(format!("git tree: {e}")))?),
        Err(_) => None,
    };
    let before = match parent_tree.as_ref() {
        Some(t) => blob_text(&repo, t, rel)?,
        None => None,
    };
    let after = blob_text(&repo, &target_tree, rel)?;
    if before.as_deref() == after.as_deref() {
        return Err(DanbiError::Config(format!(
            "커밋 {} 은 {rel} 을 바꾸지 않았어요.",
            &target.id().to_string()[..7]
        )));
    }

    // Newer commits that touched the same file.
    let mut later_commits = 0usize;
    if let Ok(mut walk) = repo.revwalk() {
        if walk.push_head().is_ok() {
            for oid in walk.flatten() {
                if oid == target.id() {
                    break;
                }
                let Ok(c) = repo.find_commit(oid) else { continue; };
                let Ok(t) = c.tree() else { continue; };
                let pt = c.parent(0).ok().and_then(|p| p.tree().ok());
                let a = pt.as_ref().and_then(|t| t.get_path(Path::new(rel)).ok()).map(|e| e.id());
                let b = t.get_path(Path::new(rel)).ok().map(|e| e.id());
                if a != b {
                    later_commits += 1;
                }
            }
        }
    }
    let warning = (later_commits > 0).then(|| {
        format!(
            "그 뒤에 이 문서를 바꾼 기록이 {later_commits}건 더 있어요 — 되돌리면 그 변경도 함께 사라질 수 있어요."
        )
    });
    if later_commits > 0 && !force {
        return Ok(RevertResult {
            applied: false,
            commit: None,
            later_commits,
            warning,
        });
    }

    let short7: String = target.id().to_string().chars().take(7).collect();
    drop(target_tree);
    drop(parent_tree);
    drop(target);
    drop(repo);
    let abs = vault.join(rel);
    match before {
        Some(content) => {
            if let Some(dir) = abs.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&abs, content)?;
        }
        None => {
            if abs.exists() {
                std::fs::remove_file(&abs)?;
            }
        }
    }
    let commit = snapshot_as(vault, &format!("danbi: revert · {short7} · {rel}"), CLIENT_HUMAN)?;
    Ok(RevertResult {
        applied: true,
        commit,
        later_commits,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(label: &str) -> std::path::PathBuf {
        let unique = format!(
            "danbi-vcs-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(path.join("Projects/p")).unwrap();
        path
    }

    fn write(vault: &Path, rel: &str, body: &str) {
        let p = vault.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn read(vault: &Path, rel: &str) -> Option<String> {
        std::fs::read_to_string(vault.join(rel)).ok()
    }

    #[test]
    fn gitignore_gets_managed_patterns_and_keeps_user_lines() {
        let vault = temp_vault("ignore");
        write(&vault, ".gitignore", "# mine\nscratch/\n");
        ensure_repo(&vault).unwrap();
        let text = read(&vault, ".gitignore").unwrap();
        assert!(text.contains("scratch/"));
        assert!(text.lines().any(|l| l == ".danbi/"));
        assert!(text.lines().any(|l| l == "/config.json"));
        // Second run leaves the file alone.
        let before = std::fs::metadata(vault.join(".gitignore")).unwrap().modified().unwrap();
        ensure_repo(&vault).unwrap();
        let after = std::fs::metadata(vault.join(".gitignore")).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn migration_untracks_existing_danbi_and_config_once() {
        let vault = temp_vault("migrate");
        // Old-style repo: .danbi/ and config.json already committed.
        let repo = Repository::init(&vault).unwrap();
        write(&vault, ".gitignore", "# managed by danbi\n.DS_Store\n*.tmp\n");
        write(&vault, "config.json", "{}");
        write(&vault, ".danbi/vectors.json", "{}");
        write(&vault, "Projects/p/.danbi/ghost-links.json", "{}");
        write(&vault, "Projects/p/a.md", "# a\n");
        commit_all(&repo, "old").unwrap();
        let tracked = |repo: &Repository| -> Vec<String> {
            let idx = repo.index().unwrap();
            let mut v: Vec<String> = idx
                .iter()
                .map(|e| String::from_utf8(e.path).unwrap())
                .collect();
            v.sort();
            v
        };
        assert!(tracked(&repo).iter().any(|p| p == "config.json"));

        ensure_repo(&vault).unwrap();
        let repo = Repository::open(&vault).unwrap();
        let now = tracked(&repo);
        assert_eq!(now, vec![".gitignore", "Projects/p/a.md"]);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert!(head.summary().unwrap().contains("추적 해제"));
        // Files stay on disk, history is untouched (old commit still parent).
        assert!(vault.join("config.json").exists());
        assert_eq!(head.parent(0).unwrap().summary().unwrap(), "old");

        // Idempotent: a further ensure_repo + snapshot adds no commit and
        // never re-adds the ignored files.
        let n_before = recent_commits(&vault, 50).unwrap().len();
        write(&vault, "config.json", "{\"changed\":1}");
        write(&vault, ".danbi/vectors.json", "{\"changed\":1}");
        ensure_repo(&vault).unwrap();
        assert_eq!(snapshot(&vault, "danbi: noop").unwrap(), None);
        assert_eq!(recent_commits(&vault, 50).unwrap().len(), n_before);
    }

    #[test]
    fn undo_reverts_only_the_last_commits_files() {
        let vault = temp_vault("undo");
        ensure_repo(&vault).unwrap();
        write(&vault, "Projects/p/a.md", "a1\n");
        write(&vault, "Projects/p/b.md", "b1\n");
        snapshot(&vault, "danbi: c1").unwrap();
        write(&vault, "Projects/p/a.md", "a2\n");
        write(&vault, "Projects/p/new.md", "new\n");
        snapshot(&vault, "danbi: c2").unwrap();
        // Another agent's note written after c2 but not yet committed —
        // the old reset --hard would have wiped it.
        write(&vault, "Projects/p/other.md", "keep me\n");

        let undo1 = undo_last(&vault).unwrap().expect("undo commit");
        assert_eq!(read(&vault, "Projects/p/a.md").unwrap(), "a1\n");
        assert!(!vault.join("Projects/p/new.md").exists(), "added file removed");
        assert_eq!(read(&vault, "Projects/p/b.md").unwrap(), "b1\n");
        assert_eq!(read(&vault, "Projects/p/other.md").unwrap(), "keep me\n");
        let head = recent_commits(&vault, 1).unwrap().remove(0);
        assert_eq!(head.id, undo1);
        assert!(head.summary.starts_with("danbi: undo · "));
        assert!(head.summary.ends_with("· danbi: c2 [human]"), "{}", head.summary);

        // Second undo walks further back (reverts c1), not the undo itself.
        undo_last(&vault).unwrap().expect("second undo");
        assert!(!vault.join("Projects/p/a.md").exists());
        assert!(!vault.join("Projects/p/b.md").exists());
        assert_eq!(read(&vault, "Projects/p/other.md").unwrap(), "keep me\n");
        // Only the init commit is left to stand on → nothing more to undo.
        assert_eq!(undo_last(&vault).unwrap(), None);
    }

    #[test]
    fn commits_since_skips_pre_snapshots_and_old_commits() {
        let vault = temp_vault("since");
        ensure_repo(&vault).unwrap();
        write(&vault, "Projects/p/a.md", "1");
        snapshot(&vault, "danbi: x (pre)").unwrap();
        write(&vault, "Projects/p/a.md", "2");
        snapshot(&vault, "danbi: x").unwrap();
        let now = chrono::Utc::now().timestamp();
        let recent = commits_since(&vault, now - 60).unwrap();
        assert_eq!(recent.len(), 2, "init + real commit");
        assert!(recent.iter().all(|c| !is_pre_snapshot(&c.summary)));
        assert!(commits_since(&vault, now + 3600).unwrap().is_empty());

        let since: std::collections::HashMap<String, i64> =
            [("p".to_string(), now - 60)].into_iter().collect();
        let counts = commits_per_project_since(&vault, &since).unwrap();
        assert_eq!(counts.get("p"), Some(&1));
    }

    #[test]
    fn write_guard_serialises_writers() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let inside = Arc::new(AtomicUsize::new(0));
        let overlap = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let inside = inside.clone();
                let overlap = overlap.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        let _g = write_guard();
                        if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                            overlap.fetch_add(1, Ordering::SeqCst);
                        }
                        std::thread::yield_now();
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(overlap.load(Ordering::SeqCst), 0);
    }

    // ---------- origin token · activity feed · revert ----------

    #[test]
    fn client_token_round_trips_and_parsers_ignore_it() {
        assert_eq!(client_token("claude_code"), "claude-code");
        assert_eq!(client_token("unknown"), CLIENT_AGENT);
        assert_eq!(client_token(""), CLIENT_AGENT);
        let msg = with_client("danbi: mcp log · p/daily/2026-10-15.md", "claude-code");
        assert_eq!(msg, "danbi: mcp log · p/daily/2026-10-15.md [claude-code]");
        assert_eq!(with_client(&msg, "claude-code"), msg, "no double token");
        assert_eq!(split_client(&msg), ("danbi: mcp log · p/daily/2026-10-15.md", Some("claude-code")));
        assert_eq!(split_client("danbi: mcp upsert_item · p/a.md · add · [#abc] x"), ("danbi: mcp upsert_item · p/a.md · add · [#abc] x", None));
        assert_eq!(split_client("danbi: x · p/a.md · note [draft 2]"), ("danbi: x · p/a.md · note [draft 2]", None));
        let (op, mode, target) = classify_commit_summary("danbi: mcp upsert_item · p/a.md · update · key1 [codex]");
        assert_eq!((op.as_str(), mode.as_deref(), target.as_deref()), ("upsert_item", Some("update"), Some("key1")));
        assert!(is_pre_snapshot("danbi: mcp log · p/a.md (pre) [codex]"));
        for legacy in ["danbi: quick-capture pre · p/a.md", "danbi: capture pre · p/a.md", "danbi: pre-edit snapshot · p/a.md", "danbi: pre-compound · p/a.md", "danbi: pre-ghost-accept · p"] {
            assert!(is_pre_snapshot(legacy), "{legacy}");
        }
        assert!(!is_pre_snapshot("danbi: compound · p/a.md · x"));
        assert!(!is_pre_snapshot("danbi: mcp append · p/prepare.md"));
    }

    #[test]
    fn preview_skips_stamp_and_unchanged_lines() {
        let old = "# day\n\n## 결정\n- a\n";
        let new = "# day\n\n## 결정\n- a\n\n#### 14:32 · claude-code\n### 완료: MCP 그래프 공개\n- 상세\n";
        assert_eq!(preview_line(Some(old), Some(new)).as_deref(), Some("### 완료: MCP 그래프 공개"));
        assert_eq!(preview_line(None, Some("# new doc\n")).as_deref(), Some("# new doc"));
        // Pure removal falls back to what disappeared.
        assert_eq!(preview_line(Some("- gone\n- kept\n"), Some("- kept\n")).as_deref(), Some("- gone"));
        let long = "x".repeat(80);
        assert_eq!(preview_line(None, Some(&long)).unwrap().chars().count(), PREVIEW_CHARS + 1);
    }

    #[test]
    fn activity_feed_rows_carry_actor_op_preview_and_paginate() {
        let vault = temp_vault("feed");
        std::fs::create_dir_all(vault.join("Projects/q")).unwrap();
        ensure_repo(&vault).unwrap();
        write(&vault, "Projects/p/daily/d.md", "# d\n");
        snapshot(&vault, "danbi: mcp log · p/daily/d.md (pre)").unwrap();
        write(&vault, "Projects/p/daily/d.md", "# d\n\n#### 10:00 · claude-code\n### 결정 A\n");
        let c1 = snapshot_as(&vault, "danbi: mcp log · p/daily/d.md", "claude-code").unwrap().unwrap();
        write(&vault, "Projects/q/notes/n.md", "## API\n- v2\n");
        snapshot(&vault, "danbi: mcp replace_section · q/notes/n.md · ## API").unwrap();
        write(&vault, "Projects/p/ui.md", "saved\n");
        snapshot_as(&vault, "danbi: save · p/ui.md", CLIENT_HUMAN).unwrap();
        write(&vault, "Projects/p/ui.md", "compounded\n");
        snapshot_as(&vault, "danbi: compound · p/ui.md · 요약", CLIENT_DANBI).unwrap();
        undo_last(&vault).unwrap().unwrap();

        let rows = activity_feed(&vault, None, None, 50).unwrap();
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert_eq!(ops, vec!["undo", "compound", "save", "replace_section", "log", "init"], "{ops:?}");
        assert!(rows.iter().all(|r| !r.op.contains("pre")));

        let undo = &rows[0];
        assert_eq!((undo.actor.as_str(), undo.client.as_str()), ("human", "human"));
        assert_eq!(undo.target.as_deref(), Some("danbi: compound · p/ui.md · 요약 [danbi]"));
        assert_eq!(undo.preview.as_deref(), Some("saved"));
        assert_eq!(rows[1].actor, "danbi");
        assert_eq!(rows[2].actor, "human");
        assert_eq!(rows[2].domain.as_deref(), Some("ui.md"));
        let legacy_mcp = &rows[3];
        assert_eq!((legacy_mcp.actor.as_str(), legacy_mcp.client.as_str()), ("agent", CLIENT_AGENT));
        assert_eq!(legacy_mcp.target.as_deref(), Some("## API"));
        assert_eq!(legacy_mcp.files, vec!["Projects/q/notes/n.md"]);
        let log = &rows[4];
        assert_eq!(log.id, c1);
        assert_eq!((log.client.as_str(), log.project.as_deref(), log.domain.as_deref()), ("claude-code", Some("p"), Some("daily/d.md")));
        assert_eq!(log.preview.as_deref(), Some("### 결정 A"), "stamp line skipped");
        assert_eq!(rows[5].actor, "danbi");

        // Scope + cursor.
        let q = activity_feed(&vault, Some("q"), None, 50).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].op, "replace_section");
        let first = activity_feed(&vault, None, None, 2).unwrap();
        assert_eq!(first.len(), 2);
        let next = activity_feed(&vault, None, Some(first[1].ts), 50).unwrap();
        assert!(next.iter().all(|r| r.ts < first[1].ts));
        assert!(activity_feed(&vault, Some("nope"), None, 5).unwrap().is_empty());
    }

    #[test]
    fn revert_doc_change_touches_one_file_and_warns_about_later_commits() {
        let vault = temp_vault("revert");
        ensure_repo(&vault).unwrap();
        write(&vault, "Projects/p/a.md", "a1\n");
        write(&vault, "Projects/p/b.md", "b1\n");
        snapshot(&vault, "danbi: c1").unwrap();
        write(&vault, "Projects/p/a.md", "a2\n");
        write(&vault, "Projects/p/new.md", "new\n");
        let c2 = snapshot(&vault, "danbi: c2").unwrap().unwrap();
        write(&vault, "Projects/p/b.md", "b2\n");
        snapshot(&vault, "danbi: c3").unwrap();

        // a.md: nothing after c2 touched it → applied without warning.
        let r = revert_doc_change(&vault, &c2, "Projects/p/a.md", false).unwrap();
        assert!(r.applied && r.warning.is_none() && r.later_commits == 0);
        assert_eq!(read(&vault, "Projects/p/a.md").unwrap(), "a1\n");
        assert_eq!(read(&vault, "Projects/p/b.md").unwrap(), "b2\n", "other file untouched");
        let head = recent_commits(&vault, 1).unwrap().remove(0);
        assert_eq!(head.id, r.commit.unwrap());
        assert_eq!(head.summary, format!("danbi: revert · {} · Projects/p/a.md [human]", &c2[..7]));
        let row = &activity_feed(&vault, None, None, 1).unwrap()[0];
        assert_eq!((row.op.as_str(), row.actor.as_str(), row.domain.as_deref()), ("revert", "human", Some("a.md")));

        // new.md was created by c2 → reverting removes it (short hash ok).
        let r = revert_doc_change(&vault, &c2[..7], "Projects/p/new.md", false).unwrap();
        assert!(r.applied);
        assert!(!vault.join("Projects/p/new.md").exists());

        // b.md changed again in c3 → not applied without force.
        let c1 = recent_commits(&vault, 10).unwrap().into_iter().find(|c| c.summary == "danbi: c1").unwrap().id;
        let r = revert_doc_change(&vault, &c1, "Projects/p/b.md", false).unwrap();
        assert!(!r.applied && r.commit.is_none() && r.later_commits == 1);
        assert!(r.warning.as_deref().unwrap().contains("1건"));
        assert_eq!(read(&vault, "Projects/p/b.md").unwrap(), "b2\n");
        let r = revert_doc_change(&vault, &c1, "Projects/p/b.md", true).unwrap();
        assert!(r.applied && r.warning.is_some());
        assert!(!vault.join("Projects/p/b.md").exists(), "c1 created b.md → removed");

        assert!(revert_doc_change(&vault, &c2, "Projects/p/b.md", false).is_err(), "c2 did not touch b.md");
        assert!(revert_doc_change(&vault, &c2, "../x", false).is_err());
    }

    #[test]
    fn editor_save_command_commits_as_human() {
        let vault = temp_vault("save");
        ensure_repo(&vault).unwrap();
        let vp = vault.to_string_lossy().to_string();
        crate::commands::write_doc(vp.clone(), "p".into(), "ui.md".into(), "# v1\n".into(), None).unwrap();
        let head = recent_commits(&vault, 1).unwrap().remove(0);
        assert_eq!(head.summary, "danbi: save · p/ui.md [human]");
        // Unchanged content → no new commit.
        crate::commands::write_doc(vp, "p".into(), "ui.md".into(), "# v1\n".into(), None).unwrap();
        assert_eq!(recent_commits(&vault, 1).unwrap().remove(0).id, head.id);
    }

    #[test]
    fn editor_save_refuses_when_file_changed_since_load() {
        let vault = temp_vault("save-conflict");
        ensure_repo(&vault).unwrap();
        let vp = vault.to_string_lossy().to_string();
        crate::commands::write_doc(vp.clone(), "p".into(), "ui.md".into(), "# v1\n".into(), None).unwrap();
        // An agent appends while the editor still holds "# v1\n".
        crate::vault::write_doc(&vault, "p", "ui.md", "# v1\n- agent note\n").unwrap();
        let err = crate::commands::write_doc(
            vp.clone(),
            "p".into(),
            "ui.md".into(),
            "# v1 edited\n".into(),
            Some("# v1\n".into()),
        )
        .unwrap_err();
        assert!(err.to_string().starts_with(crate::commands::DOC_CONFLICT), "{err}");
        assert_eq!(
            crate::vault::read_doc(&vault, "p", "ui.md").unwrap(),
            "# v1\n- agent note\n",
            "agent text must survive"
        );
        // With the current disk text as base the save goes through.
        crate::commands::write_doc(
            vp,
            "p".into(),
            "ui.md".into(),
            "# v1 edited\n".into(),
            Some("# v1\n- agent note\n".into()),
        )
        .unwrap();
        assert_eq!(crate::vault::read_doc(&vault, "p", "ui.md").unwrap(), "# v1 edited\n");
    }
}

// ---------- 문서 단위 diff · 복원 ------------------------------------------
//
// `undo_last` 는 vault 전체 reset 이라 다른 에이전트가 그 사이 쓴 노트까지
// 날린다. 인스펙터·활동 피드의 [diff]·[이 변경 되돌리기] 는 아래 두 함수로
// 해당 파일 하나만 다룬다. 쓰기는 기존 저장 경로(`vault::write_doc`) 를
// 그대로 타서 새 커밋을 남긴다 — repo reset 은 절대 하지 않는다.

/// 커밋 1개(또는 두 커밋 사이)에서 문서 하나의 unified diff.
#[derive(Debug, serde::Serialize, Clone)]
pub struct DocDiff {
    pub commit: String,
    /// 비교 기준 커밋. 루트 커밋이면 None (빈 파일 대비).
    pub base: Option<String>,
    /// `Projects/<project>/<domain>`
    pub path: String,
    pub unified: String,
    pub added: u32,
    pub removed: u32,
    pub binary: bool,
}

/// unified 텍스트 상한. 큰 문서의 전체 재작성도 UI 에 그대로 쏟지 않게.
const DOC_DIFF_MAX_BYTES: usize = 200 * 1024;

fn doc_rel_path(project: &str, domain: &str) -> String {
    format!("Projects/{project}/{domain}")
}

fn resolve_commit<'r>(repo: &'r Repository, spec: &str) -> DanbiResult<Commit<'r>> {
    repo.revparse_single(spec)
        .and_then(|o| o.peel_to_commit())
        .map_err(|e| DanbiError::Other(format!("커밋 {spec} 을 찾을 수 없어요: {e}")))
}

/// 트리에서 문서 blob 의 내용. 없으면 None, 바이너리면 Err.
fn blob_text(repo: &Repository, tree: &git2::Tree, rel: &str) -> DanbiResult<Option<String>> {
    let entry = match tree.get_path(Path::new(rel)) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let obj = entry
        .to_object(repo)
        .map_err(|e| DanbiError::Other(format!("git object: {e}")))?;
    let Some(blob) = obj.as_blob() else {
        return Ok(None);
    };
    if blob.is_binary() {
        return Err(DanbiError::Other("바이너리 파일은 되돌릴 수 없어요.".into()));
    }
    Ok(Some(String::from_utf8_lossy(blob.content()).to_string()))
}

/// `commit` 이 문서에 가한 변경(기본: 첫 부모 대비). `base` 를 주면 그
/// 커밋과 비교. 짧은 해시도 받는다.
pub fn doc_diff(
    vault: &Path,
    project: &str,
    domain: &str,
    commit: &str,
    base: Option<&str>,
) -> DanbiResult<DocDiff> {
    let repo = Repository::open(vault)
        .map_err(|e| DanbiError::Other(format!("git open: {e}")))?;
    let rel = doc_rel_path(project, domain);
    let target = resolve_commit(&repo, commit)?;
    let base_commit = match base {
        Some(b) => Some(resolve_commit(&repo, b)?),
        None => target.parent(0).ok(),
    };
    let new_tree = target
        .tree()
        .map_err(|e| DanbiError::Other(format!("git tree: {e}")))?;
    let old_tree = match base_commit.as_ref() {
        Some(c) => Some(c.tree().map_err(|e| DanbiError::Other(format!("git tree: {e}")))?),
        None => None,
    };

    let mut opts = git2::DiffOptions::new();
    opts.pathspec(&rel);
    opts.context_lines(3);
    let diff = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))
        .map_err(|e| DanbiError::Other(format!("git diff: {e}")))?;

    let mut unified = String::new();
    let mut added = 0u32;
    let mut removed = 0u32;
    let mut binary = false;
    let mut truncated = false;
    diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        if delta.flags().is_binary() {
            binary = true;
        }
        match line.origin() {
            '+' => added += 1,
            '-' => removed += 1,
            _ => {}
        }
        if unified.len() < DOC_DIFF_MAX_BYTES {
            match line.origin() {
                '+' | '-' | ' ' => unified.push(line.origin()),
                _ => {}
            }
            unified.push_str(&String::from_utf8_lossy(line.content()));
        } else {
            truncated = true;
        }
        true
    })
    .map_err(|e| DanbiError::Other(format!("git diff print: {e}")))?;
    if truncated {
        unified.push_str("\n… (truncated)");
    }

    Ok(DocDiff {
        commit: target.id().to_string(),
        base: base_commit.map(|c| c.id().to_string()),
        path: rel,
        unified,
        added,
        removed,
        binary,
    })
}

/// 문서 하나를 `commit` 직전(`before`) 또는 그 시점(`at`) 상태로 되돌린다.
/// 기존 저장 경로로 쓰고 새 커밋을 남기므로 다른 파일은 건드리지 않는다.
/// 되돌린 결과가 지금과 같으면(커밋 없음) None.
pub fn doc_restore(
    vault: &Path,
    project: &str,
    domain: &str,
    commit: &str,
    mode: &str,
) -> DanbiResult<Option<String>> {
    let repo = Repository::open(vault)
        .map_err(|e| DanbiError::Other(format!("git open: {e}")))?;
    let rel = doc_rel_path(project, domain);
    let target = resolve_commit(&repo, commit)?;
    let content = match mode {
        "before" => {
            let parent = target.parent(0).map_err(|_| {
                DanbiError::Other("이 커밋이 문서를 만들었어요 — 삭제는 휴지통으로".into())
            })?;
            let tree = parent
                .tree()
                .map_err(|e| DanbiError::Other(format!("git tree: {e}")))?;
            blob_text(&repo, &tree, &rel)?.ok_or_else(|| {
                DanbiError::Other("이 커밋이 문서를 만들었어요 — 삭제는 휴지통으로".into())
            })?
        }
        "at" => {
            let tree = target
                .tree()
                .map_err(|e| DanbiError::Other(format!("git tree: {e}")))?;
            blob_text(&repo, &tree, &rel)?.ok_or_else(|| {
                DanbiError::Other("그 커밋에는 이 문서가 없어요.".into())
            })?
        }
        other => {
            return Err(DanbiError::Other(format!(
                "restore mode 는 before 또는 at 이어야 해요 (받은 값: {other})"
            )))
        }
    };
    let short7: String = target.id().to_string().chars().take(7).collect();
    drop(target);
    drop(repo);
    // Same lock and origin token as every other writer — the feed shows
    // a restore as a human row.
    let _guard = write_guard();
    crate::vault::write_doc(vault, project, domain, &content)?;
    snapshot_as(
        vault,
        &format!("danbi: restore {mode} · {project}/{domain} ← {short7}"),
        CLIENT_HUMAN,
    )
}

#[cfg(test)]
mod doc_restore_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_vault(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "danbi-vcs-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(path.join("Projects/A")).unwrap();
        ensure_repo(&path).unwrap();
        path
    }

    fn write_and_commit(vault: &Path, domain: &str, body: &str, msg: &str) -> String {
        crate::vault::write_doc(vault, "A", domain, body).unwrap();
        snapshot(vault, msg).unwrap().expect("commit created")
    }

    fn read(vault: &Path, domain: &str) -> String {
        std::fs::read_to_string(vault.join("Projects/A").join(domain)).unwrap()
    }

    #[test]
    fn diff_counts_added_and_removed_lines() {
        let v = temp_vault("diff");
        write_and_commit(&v, "ui.md", "# UI\n\n- a\n- b\n", "c1");
        let c2 = write_and_commit(&v, "ui.md", "# UI\n\n- a\n- c\n- d\n", "c2");
        let d = doc_diff(&v, "A", "ui.md", &c2, None).unwrap();
        assert_eq!(d.path, "Projects/A/ui.md");
        assert_eq!(d.added, 2);
        assert_eq!(d.removed, 1);
        assert!(!d.binary);
        assert!(d.base.is_some());
        assert!(d.unified.contains("-- b"), "{}", d.unified);
        assert!(d.unified.contains("+- c"), "{}", d.unified);
        // 짧은 해시도 받는다.
        let short = &c2[..7];
        assert_eq!(doc_diff(&v, "A", "ui.md", short, None).unwrap().added, 2);
        let _ = std::fs::remove_dir_all(&v);
    }

    #[test]
    fn diff_against_explicit_base_and_file_creation() {
        let v = temp_vault("diff-base");
        let c1 = write_and_commit(&v, "ui.md", "one\n", "c1");
        write_and_commit(&v, "ui.md", "two\n", "c2");
        let c3 = write_and_commit(&v, "ui.md", "three\n", "c3");
        let d = doc_diff(&v, "A", "ui.md", &c3, Some(&c1)).unwrap();
        assert_eq!(d.base.as_deref(), Some(c1.as_str()));
        assert!(d.unified.contains("-one"));
        assert!(d.unified.contains("+three"));
        assert!(!d.unified.contains("two"));
        // 파일을 만든 커밋: 전부 + 줄.
        let d1 = doc_diff(&v, "A", "ui.md", &c1, None).unwrap();
        assert_eq!(d1.added, 1);
        assert_eq!(d1.removed, 0);
        let _ = std::fs::remove_dir_all(&v);
    }

    #[test]
    fn restore_before_reverts_only_that_file() {
        let v = temp_vault("restore-before");
        write_and_commit(&v, "ui.md", "v1\n", "c1");
        write_and_commit(&v, "plan.md", "plan v1\n", "c1b");
        let c2 = write_and_commit(&v, "ui.md", "v2\n", "c2");
        // 다른 에이전트가 그 사이 다른 파일에 쓴 것.
        write_and_commit(&v, "plan.md", "plan v2\n", "c3");

        let new_commit = doc_restore(&v, "A", "ui.md", &c2, "before").unwrap();
        assert!(new_commit.is_some());
        assert_eq!(read(&v, "ui.md"), "v1\n");
        assert_eq!(read(&v, "plan.md"), "plan v2\n", "other file untouched");
        let head = recent_commits(&v, 1).unwrap().remove(0);
        assert_eq!(head.id, new_commit.unwrap());
        assert!(head.summary.starts_with("danbi: restore before · A/ui.md ← "));
        assert!(head.summary.ends_with(" [human]"), "{}", head.summary);
        let row = &activity_feed(&v, None, None, 1).unwrap()[0];
        assert_eq!((row.op.as_str(), row.actor.as_str()), ("restore", "human"));
        let _ = std::fs::remove_dir_all(&v);
    }

    #[test]
    fn restore_at_brings_back_that_version() {
        let v = temp_vault("restore-at");
        let c1 = write_and_commit(&v, "ui.md", "first\n", "c1");
        write_and_commit(&v, "ui.md", "second\n", "c2");
        write_and_commit(&v, "ui.md", "third\n", "c3");
        doc_restore(&v, "A", "ui.md", &c1, "at").unwrap().unwrap();
        assert_eq!(read(&v, "ui.md"), "first\n");
        // 이미 같은 내용이면 커밋 없음.
        assert!(doc_restore(&v, "A", "ui.md", &c1, "at").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&v);
    }

    #[test]
    fn restore_before_creation_commit_errors() {
        let v = temp_vault("restore-root");
        let c1 = write_and_commit(&v, "ui.md", "born\n", "c1");
        let err = doc_restore(&v, "A", "ui.md", &c1, "before").unwrap_err();
        assert!(format!("{err}").contains("휴지통"), "{err}");
        assert_eq!(read(&v, "ui.md"), "born\n");
        assert!(doc_restore(&v, "A", "ui.md", &c1, "sideways").is_err());
        let _ = std::fs::remove_dir_all(&v);
    }
}
