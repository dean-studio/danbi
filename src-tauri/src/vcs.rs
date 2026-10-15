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
    summary.contains("(pre)")
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
    let msg = format!(
        "{UNDO_PREFIX}{} · {}",
        target.id(),
        target.summary().unwrap_or("")
    );
    Ok(commit_all(&repo, &msg)?.map(|o| o.to_string()))
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
        assert!(head.summary.ends_with("· danbi: c2"));

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
}
