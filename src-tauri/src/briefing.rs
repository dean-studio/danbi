use crate::providers::Provider;
use crate::error::{DanbiError, DanbiResult};
use crate::vault::{self, PROJECTS_DIRNAME};
use chrono::{DateTime, Duration, Local, Utc};
use git2::{Diff, DiffOptions, Repository};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

const MAX_CHANGED_FILES: usize = 12;
const MAX_SNIPPET_CHARS: usize = 2_500;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BriefingRange {
    /// Human label ("today" | "yesterday" | "last_week").
    pub range: String,
    pub since_ts: i64,
    pub until_ts: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BriefingCommit {
    pub id: String,
    pub summary: String,
    pub ts: i64,
    pub files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BriefingResult {
    pub project: String,
    pub range: BriefingRange,
    pub commits: Vec<BriefingCommit>,
    pub changed_files: Vec<String>,
    /// LLM-authored Korean paragraph; empty string if the range had no activity.
    pub summary: String,
}

fn compute_range(range: &str) -> BriefingRange {
    let now_local = Local::now();
    let today_start = now_local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_else(|| now_local.naive_local());
    let today_start_ts = today_start
        .and_local_timezone(Local)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or_else(|| now_local.timestamp());
    let now_ts = now_local.timestamp();

    match range {
        "yesterday" => BriefingRange {
            range: "yesterday".into(),
            since_ts: today_start_ts - 86_400,
            until_ts: today_start_ts,
        },
        "last_week" => BriefingRange {
            range: "last_week".into(),
            since_ts: now_ts - 7 * 86_400,
            until_ts: now_ts,
        },
        _ => BriefingRange {
            range: "today".into(),
            since_ts: today_start_ts,
            until_ts: now_ts + 1,
        },
    }
}

fn commit_touches_project(
    repo: &Repository,
    commit: &git2::Commit,
    project_prefix: &str,
) -> DanbiResult<Vec<String>> {
    let tree = commit
        .tree()
        .map_err(|e| DanbiError::Other(format!("git tree: {e}")))?;
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let mut opts = DiffOptions::new();
    opts.include_typechange(true);
    let diff: Diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))
        .map_err(|e| DanbiError::Other(format!("git diff: {e}")))?;

    let mut files: BTreeSet<String> = BTreeSet::new();
    diff.foreach(
        &mut |delta, _| {
            for path in [
                delta.new_file().path(),
                delta.old_file().path(),
            ]
            .iter()
            .flatten()
            {
                let s = path.to_string_lossy().to_string();
                if s.starts_with(project_prefix) && s.to_lowercase().ends_with(".md") {
                    // Strip the project prefix so callers see "notes/x.md".
                    let rel = s[project_prefix.len()..].to_string();
                    if !rel.is_empty() {
                        files.insert(rel);
                    }
                }
            }
            true
        },
        None,
        None,
        None,
    )
    .map_err(|e| DanbiError::Other(format!("git diff foreach: {e}")))?;

    Ok(files.into_iter().collect())
}

fn collect_git_history(
    vault: &Path,
    project_prefix: &str,
    window: &BriefingRange,
) -> DanbiResult<(Vec<BriefingCommit>, Vec<String>, Option<String>)> {
    let repo = match Repository::open(vault) {
        Ok(r) => r,
        Err(_) => {
            return Ok((
                Vec::new(),
                Vec::new(),
                Some("git 저장소가 아직 초기화되지 않았어요.".into()),
            ));
        }
    };

    let mut revwalk = repo
        .revwalk()
        .map_err(|e| DanbiError::Other(format!("revwalk: {e}")))?;
    if revwalk.push_head().is_err() {
        return Ok((
            Vec::new(),
            Vec::new(),
            Some("아직 커밋이 없어요.".into()),
        ));
    }

    let mut commits: Vec<BriefingCommit> = Vec::new();
    let mut changed: BTreeSet<String> = BTreeSet::new();

    for oid in revwalk.flatten() {
        let Ok(c) = repo.find_commit(oid) else {
            continue;
        };
        let ts = c.time().seconds();
        if ts > window.until_ts {
            continue;
        }
        if ts < window.since_ts {
            break;
        }
        let files = commit_touches_project(&repo, &c, project_prefix).unwrap_or_default();
        if files.is_empty() {
            continue;
        }
        for f in &files {
            changed.insert(f.clone());
        }
        commits.push(BriefingCommit {
            id: c.id().to_string(),
            summary: c.summary().unwrap_or("").to_string(),
            ts,
            files,
        });
    }
    let changed_files: Vec<String> = changed.iter().take(MAX_CHANGED_FILES).cloned().collect();
    Ok((commits, changed_files, None))
}

pub async fn build(
    vault: &Path,
    project: &str,
    range: &str,
    provider: &dyn Provider,
    writer_model: &str,
) -> DanbiResult<BriefingResult> {
    let window = compute_range(range);
    let project_prefix = format!("{PROJECTS_DIRNAME}/{project}/");

    let (commits, changed_files, early_summary) =
        collect_git_history(vault, &project_prefix, &window)?;

    if let Some(summary) = early_summary {
        return Ok(BriefingResult {
            project: project.to_string(),
            range: window,
            commits,
            changed_files,
            summary,
        });
    }

    if commits.is_empty() || changed_files.is_empty() {
        return Ok(BriefingResult {
            project: project.to_string(),
            range: window,
            commits,
            changed_files,
            summary: "이 기간에 바뀐 내용이 없어요.".into(),
        });
    }

    // Build a tight payload: commit messages + current snippet of each changed doc.
    #[derive(Serialize)]
    struct FileSnap<'a> {
        domain: &'a str,
        snippet: String,
    }
    let mut snaps: Vec<FileSnap> = Vec::new();
    for f in &changed_files {
        let content = vault::read_doc(vault, project, f).unwrap_or_default();
        let snippet: String = content.chars().take(MAX_SNIPPET_CHARS).collect();
        snaps.push(FileSnap {
            domain: f,
            snippet,
        });
    }

    let payload = serde_json::json!({
        "project": project,
        "range": window.range,
        "window": {
            "since": format_ts(window.since_ts),
            "until": format_ts(window.until_ts),
        },
        "commits": commits.iter().map(|c| serde_json::json!({
            "id": c.id[..8].to_string(),
            "summary": c.summary,
            "ts": format_ts(c.ts),
            "files": c.files,
        })).collect::<Vec<_>>(),
        "changed_files": snaps,
    });

    let prompt = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into());

    // --- Incremental cache (Phase J-2) ---
    //
    // Key by (project, model, commits-in-window). If the same (range,
    // commit set) has been summarized before with the same model, reuse
    // the stored summary — neither the commit list nor the file
    // snapshots can have changed, so the Writer would produce
    // essentially the same prose.
    let fingerprint_input = format!(
        "{project}|{writer_model}|{}|{}",
        window.range,
        commits.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(",")
    );
    let fingerprint = crate::cache::content_hash(fingerprint_input.as_bytes());
    if let Some(cached) = crate::cache::load_blob(vault, "briefing", &fingerprint) {
        return Ok(BriefingResult {
            project: project.to_string(),
            range: window,
            commits,
            changed_files,
            summary: cached.trim().to_string(),
        });
    }

    let raw = crate::usage::with_role(
        "briefing",
        provider.converse_text(writer_model, Some(BRIEFING_SYSTEM), &prompt, 900, 0.2),
    )
    .await?;

    let summary = raw.trim().to_string();
    let _ = crate::cache::save_blob(vault, "briefing", &fingerprint, &summary);

    Ok(BriefingResult {
        project: project.to_string(),
        range: window,
        commits,
        changed_files,
        summary,
    })
}

fn format_ts(ts: i64) -> String {
    let dt = DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|d| d.with_timezone(&Local))
        .unwrap_or_else(Local::now);
    dt.format("%Y-%m-%d %H:%M").to_string()
}

const BRIEFING_SYSTEM: &str = r#"You are Danbi's change briefer. The user just
came back to a project and wants a quick recap of what changed in a time range.

You receive JSON with:
- `project`, `range`, `window`
- `commits`: recent commit messages touching this project
- `changed_files`: [{domain, snippet}] — current content of files that changed

Write a short Korean briefing in plain text (no JSON, no code fences):
- 2~5 bullet points using "- " (hyphen-space)
- Each bullet names the concrete file(s) and the substantive change
- If commit messages are noisy ("danbi: apply · ..." boilerplate), use the
  file snippets to infer what actually changed
- Skip cosmetic commits (timestamp updates only)
- End with one line starting "다음: " that suggests where the user likely
  wants to continue (one specific file + action). If nothing obvious,
  omit that line.

Do NOT wrap in fences. Do NOT return JSON. Plain markdown-ish text only."#;

/// Duration helper used when we migrate `last_week` to explicit weekday boundaries.
#[allow(dead_code)]
fn _last_n_days(n: i64) -> Duration {
    Duration::days(n)
}
