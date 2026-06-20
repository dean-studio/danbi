use crate::error::{DanbiError, DanbiResult};
use git2::{Commit, IndexAddOption, ObjectType, Repository, ResetType, Signature};
use std::path::Path;

const DEFAULT_AUTHOR_NAME: &str = "danbi";
const DEFAULT_AUTHOR_EMAIL: &str = "danbi@local";

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

    // Write a minimal .gitignore once, so noisy files don't land in commits.
    let gitignore_path = vault.join(".gitignore");
    if !gitignore_path.exists() {
        std::fs::write(
            &gitignore_path,
            "# managed by danbi\n.DS_Store\n*.tmp\n",
        )?;
    }

    if repo.head().is_err() {
        commit_all(&repo, "danbi: initialize vault")?;
    }
    Ok(())
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
    index
        .write()
        .map_err(|e| DanbiError::Other(format!("git index write: {e}")))?;

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
        if summary.contains("(pre)") {
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

/// Hard-resets the working tree to the previous commit. Returns the new HEAD id.
pub fn undo_last(vault: &Path) -> DanbiResult<Option<String>> {
    let repo = open_or_init(vault)?;
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };
    let head_commit = head
        .peel_to_commit()
        .map_err(|e| DanbiError::Other(format!("git head peel: {e}")))?;
    let parent = match head_commit.parent(0) {
        Ok(p) => p,
        Err(_) => {
            // Nothing to undo back to; we're at the initial commit.
            return Ok(None);
        }
    };
    let target = parent
        .as_object()
        .peel(ObjectType::Commit)
        .map_err(|e| DanbiError::Other(format!("git peel parent: {e}")))?;
    repo.reset(&target, ResetType::Hard, None)
        .map_err(|e| DanbiError::Other(format!("git reset: {e}")))?;
    Ok(Some(parent.id().to_string()))
}
