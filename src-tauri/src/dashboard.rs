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
