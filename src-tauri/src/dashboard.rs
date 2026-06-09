//! Briefing dashboard — single aggregated snapshot for the "오늘의 단비" card.
//!
//! Everything here is derived from existing subsystems (daily notes, healing
//! scan, ghost link store, git history). The goal is to package one UI-ready
//! payload so the frontend doesn't have to orchestrate 4+ round trips on
//! every render.
//!
//! Output is intentionally shallow: we don't try to summarize or prioritize —
//! the UI decides what to surface.

use crate::daily::DailySnapshot;
use crate::error::DanbiResult;
use crate::ghost_links::{self, GhostLink, GhostStatus};
use crate::healing::Suggestion;
use crate::vault::PROJECTS_DIRNAME;
use crate::vcs;
use chrono::Local;
use serde::Serialize;
use std::path::Path;

/// A ghost-link suggestion enriched with its owning project so the UI can
/// jump to the right pane when the user clicks it.
#[derive(Debug, Serialize, Clone)]
pub struct GhostSuggestion {
    pub project: String,
    pub id: String,
    pub source_domain: String,
    pub target_domain: String,
    pub reason: String,
    pub created_at: i64,
}

/// Commit-level activity summary for the last N days — mirrors what
/// `briefing::build` shows inline but without the LLM-written prose, so it's
/// safe to render without spending tokens.
#[derive(Debug, Serialize, Clone)]
pub struct ActivityWindow {
    /// Days covered, exclusive of the current in-progress day but inclusive
    /// of today-so-far. For "last 7 days" we pass 7.
    pub days: i64,
    pub commit_count: usize,
    pub changed_files: Vec<String>,
    /// Most recent commit summaries (newest first), capped for UI.
    pub recent_summaries: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DashboardSnapshot {
    /// Local date stamp at the time of snapshot ("YYYY-MM-DD").
    pub generated_at: String,
    /// Pending ghost-link suggestions across all projects.
    pub ghost_suggestions: Vec<GhostSuggestion>,
    /// Healing signals (orphans / empty / oversized / empty projects).
    pub healing: Vec<Suggestion>,
    /// Reminiscence + today's daily notes.
    pub daily: DailySnapshot,
    /// Rolling 7-day activity across the whole vault.
    pub activity: ActivityWindow,
}

/// Collects ghost suggestions across every project's store, filtering to
/// `Pending` only (accepted/rejected ones aren't actionable anymore) and
/// ordering newest first.
fn collect_ghost_suggestions(vault: &Path) -> DanbiResult<Vec<GhostSuggestion>> {
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<GhostSuggestion> = Vec::new();
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
        // Missing stores are normal — just skip.
        let store = match ghost_links::load(vault, project) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for link in store.links.into_iter().filter(is_pending) {
            let GhostLink {
                id,
                source_domain,
                target_domain,
                reason,
                created_at,
                ..
            } = link;
            out.push(GhostSuggestion {
                project: project.to_string(),
                id,
                source_domain,
                target_domain,
                reason,
                created_at,
            });
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

fn is_pending(link: &GhostLink) -> bool {
    matches!(link.status, GhostStatus::Pending)
}

fn rolling_activity(vault: &Path, days: i64) -> DanbiResult<ActivityWindow> {
    // Window: [now - days * 86400, now]. We use vcs::recent_commits and then
    // filter by timestamp locally; this avoids adding a new "since" argument
    // to the commit iterator.
    let now = Local::now().timestamp();
    let since = now - days * 86_400;

    let commits = match vcs::recent_commits(vault, 200) {
        Ok(c) => c,
        Err(_) => Vec::new(),
    };
    let windowed: Vec<_> = commits.into_iter().filter(|c| c.ts >= since).collect();

    // Changed-file enumeration needs repo access — recent_commits doesn't
    // include file lists. For the UI we only need a sampled set, so we just
    // leave it empty when git2 isn't available. A future refactor can share
    // `briefing::collect_git_history`'s richer walker.
    let changed_files: Vec<String> = Vec::new();
    let recent_summaries: Vec<String> =
        windowed.iter().take(6).map(|c| c.summary.clone()).collect();

    Ok(ActivityWindow {
        days,
        commit_count: windowed.len(),
        changed_files,
        recent_summaries,
    })
}

// ---------- Activity overview cache ---------------------------------------
//
// Computing project_activity_overview walks 500 commits + scans the MCP
// usage.jsonl, both of which we'd rather not pay on every popover open.
// 60s TTL is fine — commits are at human typing pace, MCP writes too.
// Cache keyed by `days` so 7d/30d/90d toggles don't share a slot.

const ACTIVITY_TTL_MS: i64 = 60_000;

struct ActivityCacheEntry {
    fetched_at_ms: i64,
    overview: ActivityOverview,
}

static ACTIVITY_CACHE: std::sync::OnceLock<
    std::sync::RwLock<std::collections::HashMap<i64, ActivityCacheEntry>>,
> = std::sync::OnceLock::new();

fn activity_cache(
) -> &'static std::sync::RwLock<std::collections::HashMap<i64, ActivityCacheEntry>>
{
    ACTIVITY_CACHE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
}

fn now_ms_safe() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Force-fill the cache for the given window. Called by the background
/// prefetch task at startup + every 5min so the first popover open is
/// always warm.
pub fn warm_activity_cache(vault: &Path, days: i64) -> DanbiResult<()> {
    let overview = compute_project_activity_overview(vault, days)?;
    if let Ok(mut map) = activity_cache().write() {
        map.insert(
            days,
            ActivityCacheEntry {
                fetched_at_ms: now_ms_safe(),
                overview,
            },
        );
    }
    Ok(())
}

/// Per-project activity over a rolling window. Combines two cheap signals
/// the vault already keeps:
///  - git commit count touching `Projects/<P>/...`
///  - MCP inbound calls + tokens written to `<P>` in the same window
///
/// Neither is "time worked" — that would need a real timer — but together
/// they paint a usable picture of where the user's energy is going. The
/// frontend renders this as a donut + ranked bar list (see Home dashboard).
#[derive(Debug, Serialize, Clone)]
pub struct ProjectActivity {
    pub project: String,
    pub commits: u32,
    pub mcp_calls: u64,
    pub mcp_tokens: u64,
    /// commits + mcp_calls. Used as the donut weight so a lot of small
    /// commits balances against a few big MCP writes — both are evidence
    /// of attention. Tokens are exposed separately for the labels.
    pub activity_score: u64,
    /// Most recent commit/MCP timestamp within the window (unix seconds).
    /// `None` means the project has files but no signal in this window.
    pub last_activity_at: Option<i64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ActivityOverview {
    pub days: i64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub total_commits: u32,
    pub total_mcp_calls: u64,
    pub total_mcp_tokens: u64,
    /// One entry per known project, sorted by `activity_score` desc.
    /// Projects with zero activity in the window are still included so
    /// the UI can show "조용한 프로젝트" alongside active ones.
    pub by_project: Vec<ProjectActivity>,
}

pub fn project_activity_overview(
    vault: &Path,
    days: i64,
) -> DanbiResult<ActivityOverview> {
    // Serve from cache when fresh — popover/Home open many times an hour
    // and the underlying signals (commits, MCP writes) move at human
    // pace, so a 60s TTL is invisible to the user.
    if let Ok(map) = activity_cache().read() {
        if let Some(entry) = map.get(&days) {
            if now_ms_safe() - entry.fetched_at_ms < ACTIVITY_TTL_MS {
                return Ok(entry.overview.clone());
            }
        }
    }
    let overview = compute_project_activity_overview(vault, days)?;
    if let Ok(mut map) = activity_cache().write() {
        map.insert(
            days,
            ActivityCacheEntry {
                fetched_at_ms: now_ms_safe(),
                overview: overview.clone(),
            },
        );
    }
    Ok(overview)
}

fn compute_project_activity_overview(
    vault: &Path,
    days: i64,
) -> DanbiResult<ActivityOverview> {
    use std::collections::HashMap;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let from_ms = now_ms - days * 86_400_000;
    let now_s = now_ms / 1000;
    let from_s = from_ms / 1000;

    // Discover known projects from the filesystem so quiet projects still
    // show up in the list.
    let mut known: Vec<String> = Vec::new();
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if projects_root.exists() {
        for entry in std::fs::read_dir(&projects_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            known.push(name.to_string());
        }
    }

    // commits per project — share the same `since` for every project.
    let since_map: HashMap<String, i64> =
        known.iter().map(|p| (p.clone(), from_s)).collect();
    let commit_counts =
        vcs::commits_per_project_since(vault, &since_map).unwrap_or_default();

    // Last commit timestamp per project — needed for `last_activity_at`.
    // We walk the recent commit list once and remember the newest hit per
    // project. 500 commits is plenty for a 30/90-day window.
    let mut last_commit_ts: HashMap<String, i64> = HashMap::new();
    if let Ok(commits) = vcs::recent_commits(vault, 500) {
        for c in commits {
            if c.ts < from_s {
                break;
            }
            // commit summary format: "danbi: <verb> · <project>/<domain>"
            // — extract project from the segment after `· `.
            if let Some(after) = c.summary.split("· ").nth(1) {
                let proj = after.split('/').next().unwrap_or("").to_string();
                if !proj.is_empty() {
                    last_commit_ts.entry(proj).or_insert(c.ts);
                }
            }
        }
    }

    // MCP inbound — pick the closest available range.
    let range = if days <= 1 {
        crate::mcp_inbound::Range::Today
    } else if days <= 7 {
        crate::mcp_inbound::Range::Days7
    } else if days <= 30 {
        crate::mcp_inbound::Range::Days30
    } else if days <= 90 {
        crate::mcp_inbound::Range::Days90
    } else {
        crate::mcp_inbound::Range::All
    };
    let mcp = crate::mcp_inbound::summarize_vault(range);
    let mcp_by_project: HashMap<String, (u64, u64)> = mcp
        .by_project
        .iter()
        .map(|p| (p.project.clone(), (p.calls, p.tokens)))
        .collect();

    let mut by_project: Vec<ProjectActivity> = Vec::new();
    for proj in &known {
        let commits = commit_counts.get(proj).copied().unwrap_or(0);
        let (mcp_calls, mcp_tokens) =
            mcp_by_project.get(proj).copied().unwrap_or((0, 0));
        let activity_score = commits as u64 + mcp_calls;
        let last_activity_at = last_commit_ts.get(proj).copied();
        by_project.push(ProjectActivity {
            project: proj.clone(),
            commits,
            mcp_calls,
            mcp_tokens,
            activity_score,
            last_activity_at,
        });
    }
    by_project.sort_by(|a, b| {
        b.activity_score
            .cmp(&a.activity_score)
            .then_with(|| a.project.cmp(&b.project))
    });

    let total_commits: u32 = by_project.iter().map(|p| p.commits).sum();
    let total_mcp_calls: u64 = by_project.iter().map(|p| p.mcp_calls).sum();
    let total_mcp_tokens: u64 = by_project.iter().map(|p| p.mcp_tokens).sum();

    Ok(ActivityOverview {
        days,
        from_ms,
        to_ms: now_ms,
        total_commits,
        total_mcp_calls,
        total_mcp_tokens,
        by_project,
    })
}

pub fn snapshot(vault: &Path) -> DanbiResult<DashboardSnapshot> {
    let generated_at = Local::now().format("%Y-%m-%d").to_string();
    let ghost_suggestions = collect_ghost_suggestions(vault).unwrap_or_default();
    let healing = crate::healing::scan(vault).unwrap_or_default();
    let daily = crate::daily::snapshot(vault)?;
    let activity = rolling_activity(vault, 7).unwrap_or(ActivityWindow {
        days: 7,
        commit_count: 0,
        changed_files: Vec::new(),
        recent_summaries: Vec::new(),
    });

    Ok(DashboardSnapshot {
        generated_at,
        ghost_suggestions,
        healing,
        daily,
        activity,
    })
}
