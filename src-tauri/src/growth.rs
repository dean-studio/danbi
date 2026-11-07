//! 기록 비대화 감시 — per-project memory health.
//!
//! Anthropic's "components of agent memory" result: more memory is not
//! better. The model that wrote 970k chars scored lowest; the one that
//! kept 40k did far better. Wrong or useless notes drag retrieval and
//! agent behaviour down. Danbi's vault is fed by agents through MCP
//! (`danbi_log` / `danbi_append`) all day, so a person must be able to see
//! at a glance whether a project's memory is swelling, and what to do.
//!
//! Everything here is computed without an LLM, from the files, the link
//! index and git:
//!
//! - size: document count, bytes, characters, daily-note share
//! - orphans: the same rule `healing::scan` uses (daily and scaffolding
//!   files excluded), as a ratio
//! - growth: net documents / bytes added in the last 7 and 30 days from
//!   one tree diff per window (base commit before the cutoff → HEAD), and
//!   who wrote in that window from the commit summaries' origin tokens
//!   (`[claude-code]`, `[human]`, …). Without git, file birth times.
//! - largest documents and paragraphs repeated across (or inside) files
//! - plain-Korean signals with thresholds and a suggested action
//!
//! Cost control: per-file character / paragraph stats live in
//! `<vault>/.danbi/cache/growth/<project>.json` keyed by (len, mtime) so
//! only changed files are re-read; the assembled vault-wide result sits in
//! one process-wide slot keyed by (vault fingerprint, HEAD, date). The
//! link index is the shared cached one. The first call on a cold cache
//! reads every markdown file once; after that a call is a stat walk plus
//! a short git walk.

use crate::error::DanbiResult;
use crate::vault::{DomainNode, ProjectNode, PROJECTS_DIRNAME};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const WINDOW_SHORT_DAYS: i64 = 7;
pub const WINDOW_LONG_DAYS: i64 = 30;

// ---- Thresholds ------------------------------------------------------------
//
// Judgement calls, anchored on the memory paper: ~40k chars was the
// well-performing memory, ~970k the worst. A project that adds 100k chars
// a month without pruning reaches the harmful range within a few months;
// 500k total is already past the point where retrieval gets noisy.

/// 30-day net growth that counts as "불어나는 중" (chars, with a share floor
/// so a mature project isn't flagged for a normal month).
/// 자주 쓰는 프로젝트라면 한 달 10만 자는 평범해서(실측 39개 중 12개가
/// 걸림) 20만 자로 올렸다.
pub const BLOAT_MONTH_CHARS: i64 = 200_000;
pub const BLOAT_MONTH_SHARE: f64 = 0.25;
/// 7-day burst.
pub const BLOAT_WEEK_CHARS: i64 = 80_000;
pub const BLOAT_WEEK_SHARE: f64 = 0.10;
/// Absolute size past which the memory is "too big" regardless of trend.
/// 크기만으로는 '지켜볼 것' — 크지만 요즘 안 늘어나는 프로젝트는 급하지 않다.
pub const HUGE_TOTAL_CHARS: u64 = 500_000;
/// 30일 전 기록이 이보다 적으면 새 프로젝트 — 처음 쓴 것 전부가 '증가' 로
/// 잡히는 오탐이라 빨리 늘어남 신호에서 뺀다.
pub const NEW_PROJECT_BASE_CHARS: i64 = 20_000;
/// Agent-written share of the month's writes that deserves a human pass —
/// only once the volume is real (a vault fed mostly by agents would
/// otherwise flag every active project).
pub const AGENT_SHARE: f64 = 0.8;
pub const AGENT_MIN_WRITES: u32 = 100;
pub const ORPHAN_SHARE: f64 = 0.5;
pub const ORPHAN_MIN: u32 = 10;
pub const DAILY_SHARE: f64 = 0.7;
pub const DAILY_MIN_BYTES: u64 = 100_000;
/// Same as `healing`'s oversized rule.
pub const OVERSIZED_BYTES: u64 = 60_000;
/// Repeated paragraphs: this many groups, or one paragraph seen this often.
pub const DUP_MIN_GROUPS: usize = 3;
pub const DUP_MIN_REPEATS: u32 = 3;

const LARGEST_N: usize = 5;
const DUP_GROUPS_N: usize = 5;
const DUP_DOCS_N: usize = 4;
/// A paragraph shorter than this (normalised) is boilerplate, not a record.
const PARA_MIN_CHARS: usize = 40;
/// Normalised prefix used as the duplicate key — "the same entry with a
/// longer tail" still matches.
const PARA_KEY_CHARS: usize = 120;
const PARA_PREVIEW_CHARS: usize = 48;
const PARAS_PER_FILE_CAP: usize = 500;

const CACHE_DIR: &str = ".danbi/cache/growth";

// ---- Output ----------------------------------------------------------------

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct ActorSplit {
    pub human: u32,
    pub agent: u32,
    pub danbi: u32,
}

impl ActorSplit {
    pub fn total(&self) -> u32 {
        self.human + self.agent + self.danbi
    }
    fn bump(&mut self, actor: &str) {
        match actor {
            "human" => self.human += 1,
            "danbi" => self.danbi += 1,
            _ => self.agent += 1,
        }
    }
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct GrowthWindow {
    pub days: i64,
    pub docs_added: u32,
    pub docs_removed: u32,
    /// Net bytes (can be negative after a clean-up).
    pub bytes_added: i64,
    /// `bytes_added` scaled by the project's current chars-per-byte ratio.
    pub chars_added: i64,
    /// Writes (commits) in the window by who made them.
    pub writes: ActorSplit,
    /// `git` — tree diff + commit tokens; `mtime` — file birth times only
    /// (no repo or no commits), writes unknown.
    pub source: &'static str,
}

#[derive(Debug, Serialize, Clone)]
pub struct BigDoc {
    pub domain: String,
    pub bytes: u64,
    pub chars: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DuplicateGroup {
    pub preview: String,
    /// Occurrences across the project (a paragraph repeated twice in one
    /// file counts twice).
    pub count: u32,
    pub docs: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Signal {
    /// `fast_growth` · `huge` · `agent_heavy` · `orphans` · `daily_heavy`
    /// · `oversized` · `duplicates`
    pub code: &'static str,
    /// `bloating` | `watch`
    pub level: &'static str,
    pub message: String,
    pub action: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProjectGrowth {
    pub project: String,
    pub docs: u32,
    pub daily_docs: u32,
    pub bytes: u64,
    pub chars: u64,
    pub daily_bytes: u64,
    /// daily bytes / all bytes, 0..1.
    pub daily_share: f64,
    pub orphan_candidates: u32,
    pub orphans: u32,
    /// orphans / candidates, 0..1 (0 when nothing qualifies).
    pub orphan_ratio: f64,
    pub week: GrowthWindow,
    pub month: GrowthWindow,
    pub largest: Vec<BigDoc>,
    pub duplicates: Vec<DuplicateGroup>,
    /// `ok` | `watch` | `bloating`
    pub level: &'static str,
    pub signals: Vec<Signal>,
    pub computed_at: i64,
}

/// The few numbers an agent needs in `danbi_briefing`.
#[derive(Debug, Serialize, Clone)]
pub struct MemoryBrief {
    pub docs: u32,
    pub chars: u64,
    pub daily_pct: u8,
    pub orphan_pct: u8,
    pub chars_added_30d: i64,
    pub agent_pct_30d: u8,
    pub level: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    pub hint: &'static str,
}

const BRIEF_HINT: &str = "기억이 커질수록 검색이 흐려져요. 새 문서를 늘리기보다 기존 문서를 고치고(danbi_replace_section·danbi_upsert_item), 같은 내용은 다시 쓰지 마세요.";

impl ProjectGrowth {
    pub fn brief(&self) -> MemoryBrief {
        let agent_pct = pct(self.month.writes.agent as f64, self.month.writes.total() as f64);
        MemoryBrief {
            docs: self.docs,
            chars: self.chars,
            daily_pct: pct(self.daily_share, 1.0),
            orphan_pct: pct(self.orphan_ratio, 1.0),
            chars_added_30d: self.month.chars_added,
            agent_pct_30d: agent_pct,
            level: self.level,
            warning: self
                .signals
                .first()
                .map(|s| format!("{} → {}", s.message, s.action)),
            hint: BRIEF_HINT,
        }
    }
}

fn pct(part: f64, whole: f64) -> u8 {
    if whole <= 0.0 || part <= 0.0 {
        return 0;
    }
    ((part / whole) * 100.0).round().clamp(0.0, 100.0) as u8
}

// ---- Per-file stat cache ---------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct FileStat {
    len: u64,
    mtime_ms: u128,
    chars: u64,
    /// (16-hex sha1 of the normalised paragraph key, preview)
    #[serde(default)]
    paras: Vec<(String, String)>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ProjectStatCache {
    files: HashMap<String, FileStat>,
}

fn cache_path(vault: &Path, project: &str) -> PathBuf {
    vault.join(CACHE_DIR).join(format!("{project}.json"))
}

fn load_cache(vault: &Path, project: &str) -> ProjectStatCache {
    let path = cache_path(vault, project);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_cache(vault: &Path, project: &str, cache: &ProjectStatCache) {
    let path = cache_path(vault, project);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, body);
    }
}

/// Character count and duplicate keys of one document.
fn stat_text(md: &str) -> (u64, Vec<(String, String)>) {
    let chars = md.chars().count() as u64;
    let mut paras: Vec<(String, String)> = Vec::new();
    let mut buf: Vec<&str> = Vec::new();
    let flush = |buf: &mut Vec<&str>, paras: &mut Vec<(String, String)>| {
        if buf.is_empty() {
            return;
        }
        if paras.len() < PARAS_PER_FILE_CAP {
            if let Some(entry) = para_entry(buf) {
                paras.push(entry);
            }
        }
        buf.clear();
    };
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut buf, &mut paras);
            continue;
        }
        // Headings separate records and are not part of the body — the
        // same entry logged twice under different titles still matches.
        if t.starts_with('#') {
            flush(&mut buf, &mut paras);
            continue;
        }
        // The automatic stamp says nothing about the content.
        if crate::daily::is_log_stamp(t) {
            continue;
        }
        buf.push(t);
    }
    flush(&mut buf, &mut paras);
    (chars, paras)
}

/// Normalised duplicate key + preview for one paragraph, or None when it
/// is too short / boilerplate to count.
fn para_entry(lines: &[&str]) -> Option<(String, String)> {
    let first = lines.first()?;
    if first.starts_with("<!--") || first.starts_with("---") {
        return None;
    }
    let mut norm = String::new();
    for l in lines {
        let body = l
            .trim_start_matches(|c: char| {
                c == '#' || c == '-' || c == '*' || c == '>' || c == '_' || c.is_whitespace()
            })
            .trim();
        for ch in body.chars() {
            if ch.is_alphanumeric() {
                for lc in ch.to_lowercase() {
                    norm.push(lc);
                }
            }
        }
    }
    if norm.chars().count() < PARA_MIN_CHARS {
        return None;
    }
    let key: String = norm.chars().take(PARA_KEY_CHARS).collect();
    let hash = crate::cache::content_hash(key.as_bytes());
    let preview_src = first.trim_start_matches(['#', '-', '*', '>', ' ']).trim();
    let mut preview: String = preview_src.chars().take(PARA_PREVIEW_CHARS).collect();
    if preview_src.chars().count() > PARA_PREVIEW_CHARS {
        preview.push('…');
    }
    Some((hash[..16].to_string(), preview))
}

/// Every markdown file of a project, flattened to (relative name, node).
fn flatten(p: &ProjectNode) -> Vec<&DomainNode> {
    let mut out: Vec<&DomainNode> = p.domains.iter().collect();
    for s in &p.subfolders {
        out.extend(s.domains.iter());
        for n in &s.subfolders {
            out.extend(n.domains.iter());
        }
    }
    out
}

/// Refreshes the per-file cache for one project against the tree and
/// returns (stats by domain, number of files read).
fn project_stats(
    vault: &Path,
    node: &ProjectNode,
    persist: bool,
) -> (HashMap<String, FileStat>, usize) {
    let mut cache = load_cache(vault, &node.name);
    let dir = vault.join(PROJECTS_DIRNAME).join(&node.name);
    let mut fresh: HashMap<String, FileStat> = HashMap::new();
    let mut dirty = false;
    let mut read = 0usize;
    for d in flatten(node) {
        let mtime = d.modified_ms.unwrap_or(0);
        if let Some(c) = cache.files.remove(&d.name) {
            if c.len == d.bytes && c.mtime_ms == mtime {
                fresh.insert(d.name.clone(), c);
                continue;
            }
        }
        dirty = true;
        read += 1;
        let (chars, paras) = match std::fs::read_to_string(dir.join(&d.name)) {
            Ok(md) => stat_text(&md),
            // Unreadable / non-UTF-8: fall back to a byte estimate so the
            // totals still make sense.
            Err(_) => (d.bytes / 2, Vec::new()),
        };
        fresh.insert(
            d.name.clone(),
            FileStat {
                len: d.bytes,
                mtime_ms: mtime,
                chars,
                paras,
            },
        );
    }
    // Anything left in `cache.files` was deleted.
    if !cache.files.is_empty() {
        dirty = true;
    }
    if dirty && persist {
        save_cache(vault, &node.name, &ProjectStatCache { files: fresh.clone() });
    }
    (fresh, read)
}

// ---- Git facts -------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct DeltaSum {
    added: u32,
    removed: u32,
    bytes: i64,
}

#[derive(Debug, Default)]
struct GitFacts {
    head: String,
    /// [7d, 30d] writes per project.
    writes: [HashMap<String, ActorSplit>; 2],
    /// [7d, 30d] net tree change per project.
    deltas: [HashMap<String, DeltaSum>; 2],
}

/// One walk from HEAD back past the 30-day cutoff: attribute each in-window
/// commit to its project + actor from the summary alone, remember the
/// first commit older than each cutoff, then diff that commit's tree
/// against HEAD once per window.
fn git_facts(vault: &Path, now: i64) -> Option<GitFacts> {
    let repo = git2::Repository::open(vault).ok()?;
    let head = repo.head().ok()?.peel_to_commit().ok()?;
    let cut7 = now - WINDOW_SHORT_DAYS * 86_400;
    let cut30 = now - WINDOW_LONG_DAYS * 86_400;

    let mut facts = GitFacts {
        head: head.id().to_string(),
        ..Default::default()
    };
    let mut base7: Option<git2::Commit> = None;
    let mut base30: Option<git2::Commit> = None;

    let mut revwalk = repo.revwalk().ok()?;
    revwalk.push_head().ok()?;
    for oid in revwalk.flatten() {
        let Ok(c) = repo.find_commit(oid) else { continue };
        let ts = c.time().seconds();
        if ts < cut7 && base7.is_none() {
            base7 = Some(c.clone());
        }
        if ts < cut30 {
            base30 = Some(c);
            break;
        }
        let summary = c.summary().unwrap_or("");
        if crate::vcs::is_pre_snapshot(summary) {
            continue;
        }
        let Some((project, _)) = crate::vcs::doc_of_summary(summary) else { continue };
        let actor = crate::vcs::actor_of_summary(summary);
        if ts >= cut7 {
            facts.writes[0].entry(project.clone()).or_default().bump(actor);
        }
        facts.writes[1].entry(project).or_default().bump(actor);
    }

    let head_tree = head.tree().ok()?;
    for (i, base) in [base7, base30].into_iter().enumerate() {
        let base_tree = base.as_ref().and_then(|c| c.tree().ok());
        let mut opts = git2::DiffOptions::new();
        opts.pathspec(format!("{PROJECTS_DIRNAME}/"));
        let Ok(diff) = repo.diff_tree_to_tree(base_tree.as_ref(), Some(&head_tree), Some(&mut opts))
        else {
            continue;
        };
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let Some(project) = project_of_path(&path) else { continue };
            let sum = facts.deltas[i].entry(project).or_default();
            let size = |f: git2::DiffFile| -> i64 {
                if f.id().is_zero() {
                    return 0;
                }
                repo.find_blob(f.id()).map(|b| b.size() as i64).unwrap_or(0)
            };
            match delta.status() {
                git2::Delta::Added => {
                    sum.added += 1;
                    sum.bytes += size(delta.new_file());
                }
                git2::Delta::Deleted => {
                    sum.removed += 1;
                    sum.bytes -= size(delta.old_file());
                }
                _ => {
                    sum.bytes += size(delta.new_file()) - size(delta.old_file());
                }
            }
        }
    }
    Some(facts)
}

/// `Projects/<P>/<rest>.md` → `P`, ignoring hidden folders and non-markdown.
fn project_of_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix(PROJECTS_DIRNAME)?.strip_prefix('/')?;
    let (project, tail) = rest.split_once('/')?;
    if project.starts_with('.') || tail.starts_with('.') || tail.contains("/.") {
        return None;
    }
    if !tail.to_lowercase().ends_with(".md") || tail.starts_with("_assets/") {
        return None;
    }
    Some(project.to_string())
}

/// Without git: documents whose birth time falls in the window.
fn mtime_window(vault: &Path, node: &ProjectNode, now_ms: u128, days: i64) -> GrowthWindow {
    let dir = vault.join(PROJECTS_DIRNAME).join(&node.name);
    let since = now_ms.saturating_sub(days as u128 * 86_400_000);
    let mut w = GrowthWindow {
        days,
        source: "mtime",
        ..Default::default()
    };
    for d in flatten(node) {
        let created = std::fs::metadata(dir.join(&d.name))
            .ok()
            .and_then(|m| m.created().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|t| t.as_millis())
            .or(d.modified_ms);
        if created.is_some_and(|c| c >= since) {
            w.docs_added += 1;
            w.bytes_added += d.bytes as i64;
        }
    }
    w
}

// ---- Assembly --------------------------------------------------------------

fn build_project(
    vault: &Path,
    node: &ProjectNode,
    index: &crate::links::LinkIndex,
    git: Option<&GitFacts>,
    now: i64,
    persist: bool,
) -> ProjectGrowth {
    let (stats, _read) = project_stats(vault, node, persist);
    let files = flatten(node);

    let mut bytes = 0u64;
    let mut chars = 0u64;
    let mut daily_bytes = 0u64;
    let mut daily_docs = 0u32;
    let mut candidates = 0u32;
    let mut orphans = 0u32;
    let mut largest: Vec<BigDoc> = Vec::new();
    let mut groups: HashMap<&str, (String, u32, Vec<String>)> = HashMap::new();

    for d in &files {
        let st = stats.get(&d.name);
        let c = st.map(|s| s.chars).unwrap_or(d.bytes / 2);
        bytes += d.bytes;
        chars += c;
        if d.name.starts_with("daily/") {
            daily_docs += 1;
            daily_bytes += d.bytes;
        }
        if d.bytes > 0 && crate::healing::is_orphan_candidate(&d.name) {
            candidates += 1;
            let key = format!("{}/{}", node.name, d.name);
            if index.incoming.get(&key).map(|v| v.is_empty()).unwrap_or(true) {
                orphans += 1;
            }
        }
        largest.push(BigDoc {
            domain: d.name.clone(),
            bytes: d.bytes,
            chars: c,
        });
        if let Some(st) = st {
            for (hash, preview) in &st.paras {
                let g = groups
                    .entry(hash.as_str())
                    .or_insert_with(|| (preview.clone(), 0, Vec::new()));
                g.1 += 1;
                if !g.2.contains(&d.name) && g.2.len() < DUP_DOCS_N {
                    g.2.push(d.name.clone());
                }
            }
        }
    }
    largest.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.domain.cmp(&b.domain)));
    largest.truncate(LARGEST_N);

    let mut duplicates: Vec<DuplicateGroup> = groups
        .into_values()
        .filter(|(_, n, _)| *n >= 2)
        .map(|(preview, count, docs)| DuplicateGroup {
            preview,
            count,
            docs,
        })
        .collect();
    duplicates.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.preview.cmp(&b.preview)));
    let dup_groups_total = duplicates.len();
    duplicates.truncate(DUP_GROUPS_N);

    let ratio = if bytes > 0 { chars as f64 / bytes as f64 } else { 0.5 };
    let window = |i: usize, days: i64| -> GrowthWindow {
        match git {
            Some(g) => {
                let d = g.deltas[i].get(&node.name).cloned().unwrap_or_default();
                let w = g.writes[i].get(&node.name).cloned().unwrap_or_default();
                GrowthWindow {
                    days,
                    docs_added: d.added,
                    docs_removed: d.removed,
                    bytes_added: d.bytes,
                    chars_added: (d.bytes as f64 * ratio).round() as i64,
                    writes: w,
                    source: "git",
                }
            }
            None => {
                let mut w = mtime_window(vault, node, now as u128 * 1000, days);
                w.chars_added = (w.bytes_added as f64 * ratio).round() as i64;
                w
            }
        }
    };

    let mut out = ProjectGrowth {
        project: node.name.clone(),
        docs: files.len() as u32,
        daily_docs,
        bytes,
        chars,
        daily_bytes,
        daily_share: if bytes > 0 { daily_bytes as f64 / bytes as f64 } else { 0.0 },
        orphan_candidates: candidates,
        orphans,
        orphan_ratio: if candidates > 0 { orphans as f64 / candidates as f64 } else { 0.0 },
        week: window(0, WINDOW_SHORT_DAYS),
        month: window(1, WINDOW_LONG_DAYS),
        largest,
        duplicates,
        level: "ok",
        signals: Vec::new(),
        computed_at: now,
    };
    assess(&mut out, dup_groups_total);
    out
}

fn fmt_chars(n: i64) -> String {
    let a = n.unsigned_abs();
    let s = if a >= 10_000 {
        let man = a as f64 / 10_000.0;
        if man >= 10.0 {
            format!("{:.0}만", man)
        } else {
            format!("{:.1}만", man)
        }
    } else if a >= 1_000 {
        format!("{:.1}천", a as f64 / 1_000.0)
    } else {
        a.to_string()
    };
    if n < 0 {
        format!("-{s}")
    } else {
        s
    }
}

/// Fills `signals` / `level` from the numbers. Pure, so thresholds are
/// unit-testable without a vault.
pub fn assess(g: &mut ProjectGrowth, dup_groups_total: usize) {
    let mut signals: Vec<Signal> = Vec::new();
    let total = g.chars.max(1) as f64;

    let month_share = g.month.chars_added as f64 / total;
    let week_share = g.week.chars_added as f64 / total;
    let base_30d_ago = g.chars as i64 - g.month.chars_added;
    let is_new = g.month.source == "git" && base_30d_ago < NEW_PROJECT_BASE_CHARS;
    if is_new {
        // 새 프로젝트: 증가 신호 없음.
    } else if g.month.chars_added >= BLOAT_MONTH_CHARS && month_share >= BLOAT_MONTH_SHARE {
        signals.push(Signal {
            code: "fast_growth",
            level: "bloating",
            message: format!(
                "최근 한 달 사이 {}자가 늘었어요 (전체의 {}%)",
                fmt_chars(g.month.chars_added),
                pct(month_share, 1.0)
            ),
            action: "검토함의 정리 거리(daily → 주제 문서 옮기기)를 처리하고, 더 이상 맞지 않는 기록은 지우세요".into(),
        });
    } else if g.week.chars_added >= BLOAT_WEEK_CHARS && week_share >= BLOAT_WEEK_SHARE {
        signals.push(Signal {
            code: "fast_growth",
            level: "bloating",
            message: format!(
                "이번 주에만 {}자가 늘었어요 (전체의 {}%)",
                fmt_chars(g.week.chars_added),
                pct(week_share, 1.0)
            ),
            action: "검토함의 정리 거리를 처리하고, 이번 주 기록 중 같은 내용·틀린 내용은 하나로 합치세요".into(),
        });
    }
    if g.chars >= HUGE_TOTAL_CHARS {
        signals.push(Signal {
            code: "huge",
            level: "watch",
            message: format!("전체 기록이 {}자예요 — 검색이 흐려질 만큼 커요", fmt_chars(g.chars as i64)),
            action: "큰 문서를 나누고, 지난 달 이전 daily 는 요약만 남기세요".into(),
        });
    }
    let writes = g.month.writes.total();
    let agent_share = if writes > 0 { g.month.writes.agent as f64 / writes as f64 } else { 0.0 };
    if writes >= AGENT_MIN_WRITES && agent_share >= AGENT_SHARE {
        signals.push(Signal {
            code: "agent_heavy",
            level: "watch",
            message: format!(
                "최근 한 달 기록 {}건 중 {}%를 에이전트가 썼어요",
                writes,
                pct(agent_share, 1.0)
            ),
            action: "사람이 한 번 훑어 틀린 것·필요 없는 것을 지우세요".into(),
        });
    }
    if g.orphans >= ORPHAN_MIN && g.orphan_ratio >= ORPHAN_SHARE {
        signals.push(Signal {
            code: "orphans",
            level: "watch",
            message: format!(
                "문서 {}개 중 {}개가 어디서도 링크되지 않아요",
                g.orphan_candidates, g.orphans
            ),
            action: "관련 문서에 [[링크]]를 걸거나, 안 쓰는 문서는 지우세요".into(),
        });
    }
    if g.daily_bytes >= DAILY_MIN_BYTES && g.daily_share >= DAILY_SHARE {
        signals.push(Signal {
            code: "daily_heavy",
            level: "watch",
            message: format!("기록의 {}%가 daily 노트에만 쌓여 있어요", pct(g.daily_share, 1.0)),
            action: "결정·노하우는 주제 문서로 옮기고 daily 에는 링크만 남기세요".into(),
        });
    }
    if let Some(big) = g.largest.iter().find(|d| d.bytes > OVERSIZED_BYTES) {
        signals.push(Signal {
            code: "oversized",
            level: "watch",
            message: format!("{} 이 {}자로 너무 커요", big.domain, fmt_chars(big.chars as i64)),
            action: "주제별로 나누고 서로 [[링크]]로 이으세요".into(),
        });
    }
    let max_repeat = g.duplicates.first().map(|d| d.count).unwrap_or(0);
    if dup_groups_total >= DUP_MIN_GROUPS || max_repeat >= DUP_MIN_REPEATS {
        signals.push(Signal {
            code: "duplicates",
            level: "watch",
            message: format!("비슷한 문단이 {}곳에서 반복돼요", dup_groups_total),
            action: "하나로 합치고 나머지는 지우세요".into(),
        });
    }
    g.level = if signals.iter().any(|s| s.level == "bloating") {
        "bloating"
    } else if signals.is_empty() {
        "ok"
    } else {
        "watch"
    };
    g.signals = signals;
}

// ---- Vault-wide, cached ----------------------------------------------------

type Slot = Option<(PathBuf, String, Arc<Vec<ProjectGrowth>>)>;
static CACHE: Mutex<Slot> = Mutex::new(None);

/// Growth of every project, newest computation cached until any markdown
/// file, the git HEAD or the calendar day changes.
pub fn overview(vault: &Path) -> DanbiResult<Arc<Vec<ProjectGrowth>>> {
    let fp = crate::search::fingerprint_vault(vault)?;
    let head = git2::Repository::open(vault)
        .ok()
        .and_then(|r| r.head().ok()?.target())
        .map(|o| o.to_string())
        .unwrap_or_default();
    let key = format!("{fp}|{head}|{}", Local::now().format("%Y-%m-%d"));
    if let Ok(guard) = CACHE.lock() {
        if let Some((v, k, rows)) = guard.as_ref() {
            if v == vault && *k == key {
                return Ok(rows.clone());
            }
        }
    }
    let rows = Arc::new(compute_all(vault)?);
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((vault.to_path_buf(), key, rows.clone()));
    }
    Ok(rows)
}

/// Uncached computation for the whole vault.
pub fn compute_all(vault: &Path) -> DanbiResult<Vec<ProjectGrowth>> {
    compute_all_with(vault, true)
}

/// `persist = false` skips writing the per-file stat cache — for
/// read-only measurements against a vault that must not be touched.
pub fn compute_all_with(vault: &Path, persist: bool) -> DanbiResult<Vec<ProjectGrowth>> {
    let tree = crate::vault::list_tree_with(vault, false)?;
    let index = crate::links::build_index_cached(vault)?;
    let now = Local::now().timestamp();
    let git = git_facts(vault, now);
    let mut out: Vec<ProjectGrowth> = tree
        .projects
        .iter()
        .map(|p| build_project(vault, p, &index, git.as_ref(), now, persist))
        .collect();
    out.sort_by(|a, b| {
        level_rank(b.level)
            .cmp(&level_rank(a.level))
            .then_with(|| b.month.chars_added.cmp(&a.month.chars_added))
            .then_with(|| a.project.cmp(&b.project))
    });
    Ok(out)
}

fn level_rank(level: &str) -> u8 {
    match level {
        "bloating" => 2,
        "watch" => 1,
        _ => 0,
    }
}

pub fn for_project(vault: &Path, project: &str) -> DanbiResult<Option<ProjectGrowth>> {
    let rows = overview(vault)?;
    Ok(rows.iter().find(|r| r.project == project).cloned())
}

/// Projects whose level is `bloating`, for the vault-wide briefing.
pub fn bloating_projects(vault: &Path) -> Vec<String> {
    overview(vault)
        .map(|rows| {
            rows.iter()
                .filter(|r| r.level == "bloating")
                .map(|r| r.project.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(label: &str) -> PathBuf {
        let unique = format!(
            "danbi-growth-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(root.join(PROJECTS_DIRNAME)).unwrap();
        root
    }

    fn write(vault: &Path, rel: &str, body: &str) {
        let path = vault.join(PROJECTS_DIRNAME).join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// Commit everything with a chosen timestamp so 7/30-day windows can
    /// be exercised without waiting.
    fn commit_at(vault: &Path, msg: &str, ts: i64) {
        let repo = git2::Repository::open(vault)
            .or_else(|_| git2::Repository::init(vault))
            .unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None).unwrap();
        idx.update_all(["*"].iter(), None).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::new("t", "t@x", &git2::Time::new(ts, 0)).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents).unwrap();
    }

    fn blank(project: &str) -> ProjectGrowth {
        ProjectGrowth {
            project: project.into(),
            docs: 0,
            daily_docs: 0,
            bytes: 0,
            chars: 0,
            daily_bytes: 0,
            daily_share: 0.0,
            orphan_candidates: 0,
            orphans: 0,
            orphan_ratio: 0.0,
            week: GrowthWindow { days: 7, source: "git", ..Default::default() },
            month: GrowthWindow { days: 30, source: "git", ..Default::default() },
            largest: Vec::new(),
            duplicates: Vec::new(),
            level: "ok",
            signals: Vec::new(),
            computed_at: 0,
        }
    }

    #[test]
    fn counts_docs_bytes_chars_daily_share_and_orphans() {
        let v = temp_vault("counts");
        // 10 Korean chars = 30 bytes; ui.md is scaffolding, daily excluded
        // from orphans; notes/a.md is linked from ui.md, notes/b.md is not.
        write(&v, "P/ui.md", "[[notes/a.md]] 가나다라마바사아자차");
        write(&v, "P/notes/a.md", "가나다라마바사아자차");
        write(&v, "P/notes/b.md", "가나다라마바사아자차");
        write(&v, "P/daily/2026-01-01.md", "가나다라마바사아자차가나다라마바사아자차");
        write(&v, "P/daily/2026-01/x.md", "가나다라마바사아자차");

        let rows = compute_all(&v).unwrap();
        let p = rows.iter().find(|r| r.project == "P").unwrap();
        assert_eq!(p.docs, 5);
        assert_eq!(p.daily_docs, 2);
        assert_eq!(p.chars, (14 + 1 + 10) + 10 + 10 + 20 + 10);
        assert_eq!(p.bytes, (14 + 1 + 30) + 30 + 30 + 60 + 30);
        assert!((p.daily_share - 90.0 / 195.0).abs() < 1e-9);
        assert_eq!(p.orphan_candidates, 2, "notes/a, notes/b");
        assert_eq!(p.orphans, 1, "only notes/b");
        assert!((p.orphan_ratio - 0.5).abs() < 1e-9);
        assert_eq!(p.largest[0].domain, "daily/2026-01-01.md");
        // list_tree initialised the repo with one commit just now, so
        // every file is "added this week".
        assert_eq!(p.week.source, "git");
        assert_eq!(p.week.docs_added, 5, "all files were just created");
        assert_eq!(p.level, "ok");

        // Without git facts the birth-time fallback says the same.
        let tree = crate::vault::list_tree_with(&v, false).unwrap();
        let node = tree.projects.iter().find(|n| n.name == "P").unwrap();
        let index = crate::links::build_index(&v).unwrap();
        let g = build_project(&v, node, &index, None, Local::now().timestamp(), true);
        assert_eq!(g.week.source, "mtime");
        assert_eq!(g.week.docs_added, 5);
        assert_eq!(g.week.bytes_added, 195);
        assert_eq!(g.week.writes.total(), 0, "writers unknown without git");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn empty_project_and_daily_only_project() {
        let v = temp_vault("empty");
        std::fs::create_dir_all(v.join(PROJECTS_DIRNAME).join("Empty")).unwrap();
        write(&v, "DailyOnly/daily/2026-02-02.md", "오늘 한 일\n\n내일 할 일");
        let rows = compute_all(&v).unwrap();
        let e = rows.iter().find(|r| r.project == "Empty").unwrap();
        assert_eq!(e.docs, 0);
        assert_eq!(e.bytes, 0);
        assert_eq!(e.daily_share, 0.0);
        assert_eq!(e.orphan_ratio, 0.0);
        assert!(e.largest.is_empty());
        assert_eq!(e.level, "ok");
        let d = rows.iter().find(|r| r.project == "DailyOnly").unwrap();
        assert_eq!(d.docs, 1);
        assert_eq!(d.daily_docs, 1);
        assert_eq!(d.daily_share, 1.0);
        assert_eq!(d.orphan_candidates, 0);
        assert_eq!(d.level, "ok", "tiny daily-only project is fine");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn git_windows_split_7_and_30_days_and_attribute_writers() {
        let v = temp_vault("git");
        let now = Local::now().timestamp();
        let d = 86_400;
        write(&v, "P/old.md", "x".repeat(1_000).as_str());
        commit_at(&v, "initialize vault", now - 60 * d);
        // 20 days ago: an agent adds a 600-byte note (in 30d, not 7d).
        write(&v, "P/notes/mid.md", "y".repeat(600).as_str());
        commit_at(&v, "danbi: mcp append · P/notes/mid.md [claude-code]", now - 20 * d);
        // 2 days ago: a person grows old.md by 400 bytes and adds a daily.
        write(&v, "P/old.md", "x".repeat(1_400).as_str());
        commit_at(&v, "danbi: save · P/old.md [human]", now - 2 * d);
        write(&v, "P/daily/t.md", "z".repeat(200).as_str());
        commit_at(&v, "danbi: mcp log · P/daily/t.md [codex]", now - 1 * d);
        // Pre-edit snapshots and another project never count for P.
        commit_at(&v, "danbi: mcp log · P/daily/t.md (pre) [codex]", now - 1 * d + 1);
        write(&v, "Q/q.md", "q".repeat(5_000).as_str());
        commit_at(&v, "danbi: mcp create_file · Q/q.md [agent]", now - 3 * d);

        let rows = compute_all(&v).unwrap();
        let p = rows.iter().find(|r| r.project == "P").unwrap();
        assert_eq!(p.week.source, "git");
        assert_eq!(p.week.docs_added, 1, "daily/t.md");
        assert_eq!(p.week.bytes_added, 400 + 200);
        assert_eq!(p.week.writes, ActorSplit { human: 1, agent: 1, danbi: 0 });
        assert_eq!(p.month.docs_added, 2, "notes/mid.md + daily/t.md");
        assert_eq!(p.month.bytes_added, 600 + 400 + 200);
        assert_eq!(p.month.writes, ActorSplit { human: 1, agent: 2, danbi: 0 });
        // ASCII: one char per byte.
        assert_eq!(p.month.chars_added, 1_200);
        let q = rows.iter().find(|r| r.project == "Q").unwrap();
        assert_eq!(q.week.docs_added, 1);
        assert_eq!(q.week.bytes_added, 5_000);
        assert_eq!(q.week.writes, ActorSplit { human: 0, agent: 1, danbi: 0 });
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn deleted_docs_count_as_removed_and_negative_bytes() {
        let v = temp_vault("removed");
        let now = Local::now().timestamp();
        write(&v, "P/a.md", "a".repeat(900).as_str());
        write(&v, "P/b.md", "b".repeat(100).as_str());
        commit_at(&v, "initialize vault", now - 40 * 86_400);
        std::fs::remove_file(v.join(PROJECTS_DIRNAME).join("P/a.md")).unwrap();
        commit_at(&v, "danbi: delete · P/a.md [human]", now - 86_400);
        let rows = compute_all(&v).unwrap();
        let p = rows.iter().find(|r| r.project == "P").unwrap();
        assert_eq!(p.month.docs_removed, 1);
        assert_eq!(p.month.bytes_added, -900);
        assert_eq!(p.docs, 1);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn duplicates_are_found_across_and_inside_files() {
        let v = temp_vault("dups");
        // ~130 Hangul chars: longer than the 120-char key, so a copy with
        // an extra tail still matches.
        let para = "결정: 로그인 토큰 만료는 7일로 둔다. 이유는 모바일 세션 유지와 보안의 균형이고, 리프레시 토큰은 키체인에만 두며 서버는 만료 시각을 응답 헤더로 내려준다. 클라이언트는 만료 하루 전부터 조용히 갱신을 시도하고 실패하면 로그인 화면으로 보낸다. 테스트 환경에서는 만료를 10분으로 줄여 흐름을 빠르게 확인한다.";
        write(&v, "P/daily/2026-03-01.md", &format!("# d1\n\n### 결정\n_10:00 · claude-code_\n{para}\n\n### 다른 제목으로 또\n_10:05 · claude-code_\n{para}\n"));
        write(&v, "P/daily/2026-03-02.md", &format!("# d2\n\n- {para} (추가 꼬리말)\n"));
        write(&v, "P/notes/short.md", "짧은 글\n\n짧은 글\n");
        let rows = compute_all(&v).unwrap();
        let p = rows.iter().find(|r| r.project == "P").unwrap();
        assert_eq!(p.duplicates.len(), 1, "{:?}", p.duplicates);
        assert_eq!(p.duplicates[0].count, 3);
        assert_eq!(p.duplicates[0].docs.len(), 2);
        assert!(p.duplicates[0].preview.starts_with("결정: 로그인"));
        assert!(p.signals.iter().any(|s| s.code == "duplicates"), "{:?}", p.signals);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn file_stat_cache_refreshes_when_a_file_changes_or_disappears() {
        let v = temp_vault("cache");
        write(&v, "P/a.md", "가나다");
        write(&v, "P/b.md", "라마바사");
        // The first call also initialises the vault's git repo (one
        // commit), which changes the key once; from then on it is stable.
        let _ = overview(&v).unwrap();
        let first = overview(&v).unwrap();
        assert_eq!(first[0].chars, 7);
        assert!(Arc::ptr_eq(&first, &overview(&v).unwrap()), "same fingerprint → same Arc");

        // Different length → stat cache miss → re-read.
        write(&v, "P/a.md", "가나다라마바사아");
        let second = overview(&v).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second[0].chars, 12);
        assert_eq!(second[0].docs, 2);
        let cache = load_cache(&v, "P");
        assert_eq!(cache.files["P/a.md".trim_start_matches("P/")].chars, 8);

        std::fs::remove_file(v.join(PROJECTS_DIRNAME).join("P/b.md")).unwrap();
        let third = overview(&v).unwrap();
        assert_eq!(third[0].docs, 1);
        assert_eq!(third[0].chars, 8);
        assert!(!load_cache(&v, "P").files.contains_key("b.md"), "deleted entry dropped");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn thresholds_fast_growth_month() {
        let mut g = blank("P");
        g.chars = 600_000;
        g.month.chars_added = 250_000; // 42% of total, > 200k
        assess(&mut g, 0);
        assert_eq!(g.level, "bloating");
        assert_eq!(g.signals[0].code, "fast_growth");
        assert!(g.signals[0].message.contains("한 달"), "{}", g.signals[0].message);

        // Same absolute growth on a huge mature project is a normal month.
        let mut g = blank("P");
        g.chars = 480_000;
        g.month.chars_added = 110_000; // 23% < 25%
        assess(&mut g, 0);
        assert_eq!(g.level, "ok", "{:?}", g.signals);

        // 자주 쓰는 프로젝트의 평범한 달(10만 자대)은 걸리지 않는다.
        let mut g = blank("P");
        g.chars = 300_000;
        g.month.chars_added = 150_000;
        assess(&mut g, 0);
        assert_eq!(g.level, "ok", "{:?}", g.signals);

        // 한 달 안에 새로 만든 프로젝트: 처음 쓴 것 전부가 '증가' 라도 신호 없음.
        let mut g = blank("P");
        g.chars = 300_000;
        g.month.chars_added = 295_000;
        g.week.chars_added = 100_000;
        assess(&mut g, 0);
        assert!(g.signals.iter().all(|s| s.code != "fast_growth"), "{:?}", g.signals);
    }

    #[test]
    fn thresholds_week_burst_huge_total_and_ranking() {
        let mut g = blank("P");
        g.chars = 400_000;
        g.week.chars_added = 90_000;
        assess(&mut g, 0);
        assert_eq!(g.level, "bloating");
        assert!(g.signals[0].message.contains("이번 주"));

        // 크기만으로는 '지켜볼 것'.
        let mut g = blank("P");
        g.chars = 600_000;
        assess(&mut g, 0);
        assert_eq!(g.level, "watch");
        assert_eq!(g.signals[0].code, "huge");
        assert_eq!(g.brief().level, "watch");
        assert!(g.brief().warning.unwrap().contains("→"));
    }

    #[test]
    fn thresholds_watch_signals_and_floors() {
        // Agent-heavy: 100+ writes, ≥ 80% agent.
        let mut g = blank("P");
        g.chars = 10_000;
        g.month.writes = ActorSplit { human: 15, agent: 120, danbi: 0 };
        assess(&mut g, 0);
        assert_eq!(g.level, "watch");
        assert_eq!(g.signals[0].code, "agent_heavy");
        assert_eq!(g.brief().agent_pct_30d, 89);

        // Below the write floor: nothing, even at 100% agent.
        let mut g = blank("P");
        g.month.writes = ActorSplit { human: 0, agent: 60, danbi: 0 };
        assess(&mut g, 0);
        assert_eq!(g.level, "ok");

        // Orphans need both the share and the count.
        let mut g = blank("P");
        g.orphan_candidates = 20;
        g.orphans = 12;
        g.orphan_ratio = 0.6;
        assess(&mut g, 0);
        assert_eq!(g.signals[0].code, "orphans");
        let mut g = blank("P");
        g.orphan_candidates = 4;
        g.orphans = 4;
        g.orphan_ratio = 1.0;
        assess(&mut g, 0);
        assert_eq!(g.level, "ok", "4 orphans is not a trend");

        // Daily-heavy needs 100KB of daily.
        let mut g = blank("P");
        g.bytes = 200_000;
        g.daily_bytes = 180_000;
        g.daily_share = 0.9;
        assess(&mut g, 0);
        assert_eq!(g.signals[0].code, "daily_heavy");

        // Oversized reuses healing's 60KB rule.
        let mut g = blank("P");
        g.largest = vec![BigDoc { domain: "big.md".into(), bytes: 70_000, chars: 25_000 }];
        assess(&mut g, 0);
        assert_eq!(g.signals[0].code, "oversized");
        assert!(g.signals[0].message.starts_with("big.md"));

        // Duplicates: 3 groups, or one paragraph seen 3 times.
        let mut g = blank("P");
        assess(&mut g, 3);
        assert_eq!(g.signals[0].code, "duplicates");
        let mut g = blank("P");
        g.duplicates = vec![DuplicateGroup { preview: "x".into(), count: 3, docs: vec![] }];
        assess(&mut g, 1);
        assert_eq!(g.signals[0].code, "duplicates");
        let mut g = blank("P");
        g.duplicates = vec![DuplicateGroup { preview: "x".into(), count: 2, docs: vec![] }];
        assess(&mut g, 1);
        assert_eq!(g.level, "ok");
    }

    #[test]
    fn overview_ranks_bloating_first_and_briefing_carries_memory() {
        let v = temp_vault("rank");
        let now = chrono::Local::now().timestamp();
        write(&v, "Calm/a.md", "조용한 프로젝트");
        // 40일 전에 이미 10만 자 → 새 프로젝트가 아님.
        write(&v, "Loud/notes/base.md", "b".repeat(100_000).as_str());
        commit_at(&v, "base", now - 40 * 86_400);
        // 이번 달 50만 자 추가 → 전체 60만: 빨리 늘어남 + 너무 큼.
        write(&v, "Loud/notes/big.md", "w".repeat(500_000).as_str());
        commit_at(&v, "burst", now - 86_400);
        let rows = overview(&v).unwrap();
        assert_eq!(rows[0].project, "Loud");
        assert_eq!(rows[0].level, "bloating");
        let codes: Vec<&str> = rows[0].signals.iter().map(|s| s.code).collect();
        assert!(codes.contains(&"huge") && codes.contains(&"fast_growth"), "{codes:?}");
        assert_eq!(bloating_projects(&v), vec!["Loud".to_string()]);

        let snap = crate::dashboard::briefing(&v, Some("Loud")).unwrap();
        let mem = snap.memory.clone().expect("project briefing has memory");
        assert_eq!(mem.docs, 2);
        assert_eq!(mem.level, "bloating");
        assert!(mem.warning.unwrap().contains("→"), "message → action");
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"memory\":{"));

        let all = crate::dashboard::briefing(&v, None).unwrap();
        assert!(all.memory.is_none());
        assert_eq!(all.bloating_projects, vec!["Loud".to_string()]);
        let json = serde_json::to_string(&all).unwrap();
        assert!(json.contains("\"bloating_projects\":[\"Loud\"]"));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn helpers() {
        assert_eq!(project_of_path("Projects/P/daily/x.md"), Some("P".into()));
        assert_eq!(project_of_path("Projects/P/.danbi/next.json"), None);
        assert_eq!(project_of_path("Projects/P/_assets/a.md"), None);
        assert_eq!(project_of_path("Projects/P/a.png"), None);
        assert_eq!(project_of_path("config.json"), None);
        assert_eq!(fmt_chars(123), "123");
        assert_eq!(fmt_chars(4_500), "4.5천");
        assert_eq!(fmt_chars(32_000), "3.2만");
        assert_eq!(fmt_chars(970_000), "97만");
        assert_eq!(fmt_chars(-20_000), "-2.0만");
        assert_eq!(pct(0.456, 1.0), 46);
        let (chars, paras) = stat_text("# 제목\n\n_10:00 · claude-code_\n짧음\n");
        assert_eq!(chars, 31);
        assert!(paras.is_empty());
    }

    /// Read-only timing against the real vault; `cargo test --lib growth -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_vault_timing() {
        // `DANBI_GROWTH_VAULT=<copy>` measures the warm (persisted stat
        // cache) path on a throwaway copy; the real vault is never written.
        let (vault, persist) = match std::env::var("DANBI_GROWTH_VAULT") {
            Ok(p) => (PathBuf::from(p), true),
            Err(_) => {
                let home = std::env::var("HOME").unwrap();
                (PathBuf::from(home).join("Danbi_Vault"), false)
            }
        };
        if !vault.exists() {
            return;
        }
        let t = std::time::Instant::now();
        let git = git_facts(&vault, Local::now().timestamp());
        println!("git_facts: {:?} (head {:?})", t.elapsed(), git.as_ref().map(|g| &g.head));
        // Read-only: no stat cache is written, so every call here is the
        // cold path (all files read) — the in-app warm path only re-reads
        // changed files.
        let t = std::time::Instant::now();
        let index = crate::links::build_index_cached(&vault).unwrap();
        println!("links index (shared cache, cold): {:?}, {} docs", t.elapsed(), index.outgoing.len());
        let t = std::time::Instant::now();
        let rows = compute_all_with(&vault, persist).unwrap();
        println!("compute_all (cold: every file read, persist={persist}): {:?}, {} projects", t.elapsed(), rows.len());
        let t = std::time::Instant::now();
        let rows2 = compute_all_with(&vault, persist).unwrap();
        println!("compute_all (again; stat cache {}): {:?}", if persist { "warm" } else { "not persisted → cold" }, t.elapsed());
        if persist {
            let t = std::time::Instant::now();
            let _ = overview(&vault).unwrap();
            println!("overview (first, fills memory slot): {:?}", t.elapsed());
            let t = std::time::Instant::now();
            let _ = overview(&vault).unwrap();
            println!("overview (memory hit: fingerprint + HEAD only): {:?}", t.elapsed());
            let t = std::time::Instant::now();
            let snap = crate::dashboard::briefing(&vault, Some("단비")).unwrap();
            println!("dashboard::briefing(단비) with memory: {:?}, {} chars", t.elapsed(), serde_json::to_string(&snap).unwrap().chars().count());
        }
        let t = std::time::Instant::now();
        let fp = crate::search::fingerprint_vault(&vault).unwrap();
        println!("fingerprint_vault (cache key): {:?} ({} chars)", t.elapsed(), fp.len());
        let total_docs: u32 = rows2.iter().map(|r| r.docs).sum();
        let total_chars: u64 = rows2.iter().map(|r| r.chars).sum();
        println!("vault: {} docs, {} chars, bloating={:?}", total_docs, total_chars,
            rows2.iter().filter(|r| r.level == "bloating").map(|r| r.project.as_str()).collect::<Vec<_>>());
        if let Some(r) = rows2.iter().find(|r| r.project == "단비") {
            println!("brief(단비)={}", serde_json::to_string(&r.brief()).unwrap());
        }
        for r in rows2.iter().take(12) {
            println!(
                "{:<22} {:>4} docs {:>8} chars daily {:>3}% orphan {:>3}% ({}/{}) 7d +{} 30d +{} chars writes30 h{}/a{}/d{} dups {} level {} {:?}",
                r.project, r.docs, r.chars, pct(r.daily_share, 1.0), pct(r.orphan_ratio, 1.0), r.orphans, r.orphan_candidates,
                r.week.chars_added, r.month.chars_added, r.month.writes.human, r.month.writes.agent, r.month.writes.danbi,
                r.duplicates.len(), r.level, r.signals.iter().map(|s| s.code).collect::<Vec<_>>()
            );
        }
    }
}

#[cfg(test)]
mod real_clone_growth {
    /// 복제본에서 "불어나는 중" 이 어떤 이유로 걸리는지 (기준값 설명용).
    #[test]
    #[ignore]
    fn growth_reasons_on_clone() {
        let Some(dir) = std::env::var_os("DANBI_DREAM_VAULT") else { return };
        let vault = std::path::PathBuf::from(dir);
        for g in super::compute_all_with(&vault, false).unwrap() {
            if g.level != "bloating" { continue; }
            let codes: Vec<String> = g.signals.iter().filter(|s| s.level == "bloating").map(|s| format!("{}: {}", s.code, s.message)).collect();
            println!("{} | 전체 {} · 30일 +{} · 7일 +{} | {}", g.project, g.chars, g.month.chars_added, g.week.chars_added, codes.join(" / "));
        }
    }
}
