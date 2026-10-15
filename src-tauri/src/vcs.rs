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
    crate::vault::write_doc(vault, project, domain, &content)?;
    snapshot(
        vault,
        &format!("danbi: restore {mode} · {project}/{domain} ← {short7}"),
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
