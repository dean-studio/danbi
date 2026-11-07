//! Dreaming — 쉬는 시간 정리.
//!
//! vault 가 한동안 조용할 때(또는 Settings 의 "지금 정리") 프로젝트별로
//! 정리 후보를 찾아 검토함(`reviews`)에 올린다. **문서는 절대 자동으로
//! 고치지 않는다** — 사용자가 검토함에서 고른 선택지만 `apply` 가 실행하고
//! git 에 커밋한다.
//!
//! 찾는 것 (전부 휴리스틱, LLM 불필요):
//! 1. 깨진 `[[링크]]` — 이름이 비슷한 파일·폴더 이동·git 이름 변경 이력으로
//!    바꿀 후보를 제시.
//! 2. 다시 확인할 문서 — `stale_after` 가 지난 문서, 대체된 결정을 아직
//!    인용하는 문서.
//! 3. daily → 주제 문서 승격 — daily 에 묻힌 결정·원인·노하우 섹션을
//!    `decisions/`·`bugs/`·`notes/` 로 옮기고 daily 엔 링크만 남기기.
//!
//! 상태는 `<vault>/.danbi/dreaming.json` 하나: 마지막 실행 시각, 프로젝트별
//! 지문(바뀐 프로젝트만 다시 봄), 사용자가 답한 후보 키(다시 올리지 않음).

use crate::error::{DanbiError, DanbiResult};
use crate::links;
use crate::ontology;
use crate::reviews;
use crate::vault::{self, PROJECTS_DIRNAME};
use crate::vcs;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

const STORE_FILE: &str = ".danbi/dreaming.json";

pub const KIND_PREFIX: &str = "dream_";
pub const KIND_FIX_LINK: &str = "dream_fix_link";
pub const KIND_FIX_LINK_BATCH: &str = "dream_fix_link_batch";
pub const KIND_RECHECK: &str = "dream_recheck";
pub const KIND_PROMOTE: &str = "dream_promote";

/// 검토함에 동시에 떠 있을 수 있는 정리 항목 수. 넘으면 이번 실행은 쉰다.
const MAX_PENDING: usize = 20;
/// 바뀐 게 없어도 이 시간이 지나면 다시 본다 (stale_after 는 시간이 지나면
/// 파일 변경 없이도 생긴다).
const RESCAN_AFTER_SECS: i64 = 24 * 3600;
/// 승격 후보로 보는 최근 daily 수 (프로젝트당).
const MAX_DAILY_SCANNED: usize = 60;
/// 승격하려면 본문이 이만큼은 있어야 한다 (스탬프 줄 제외).
const MIN_PROMOTE_CHARS: usize = 60;
/// 링크 후보 최대 수.
const MAX_LINK_OPTIONS: usize = 3;
/// git 이름 변경 이력을 뒤지는 커밋 수 상한.
const GIT_RENAME_COMMITS: usize = 300;
/// "아직 맞아요" 가 stale_after 를 미루는 일수.
const VERIFY_EXTEND_DAYS: i64 = 90;

// ---------------------------------------------------------------- store

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectMark {
    pub fingerprint: String,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    /// "applied" | "dismissed"
    pub status: String,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DreamStore {
    #[serde(default)]
    pub last_run_at: Option<i64>,
    #[serde(default)]
    pub last_message: Option<String>,
    #[serde(default)]
    pub project_marks: HashMap<String, ProjectMark>,
    /// 후보 키 → 사용자의 답. 같은 후보를 다시 올리지 않는다.
    #[serde(default)]
    pub decided: HashMap<String, Decision>,
}

fn store_path(vault: &Path) -> PathBuf {
    vault.join(STORE_FILE)
}

pub fn load_store(vault: &Path) -> DanbiResult<DreamStore> {
    let p = store_path(vault);
    if !p.exists() {
        return Ok(DreamStore::default());
    }
    let raw = std::fs::read_to_string(&p)?;
    if raw.trim().is_empty() {
        return Ok(DreamStore::default());
    }
    serde_json::from_str(&raw).map_err(|e| DanbiError::Other(format!("dreaming parse: {e}")))
}

pub fn save_store(vault: &Path, store: &DreamStore) -> DanbiResult<()> {
    let p = store_path(vault);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(store)?)?;
    Ok(())
}

// ---------------------------------------------------------------- candidates

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkOption {
    /// 바꿀 대상 — 같은 프로젝트면 `notes/x.md`, 다른 프로젝트면 `P/x.md`.
    pub target: String,
    pub confidence: f32,
    pub why: String,
}

/// 묶음 수리의 한 줄 — 확실한 링크 하나 (`[[x.md]]` → `notes/x.md`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchFix {
    /// 개별 후보였을 때의 키 — 묶음을 보류하면 이 키들도 같이 기억한다.
    pub key: String,
    pub broken: String,
    pub target: String,
    pub sources: Vec<String>,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DreamAction {
    /// `broken` 을 가리키는 링크가 `sources` 에 있다. 선택지: `relink`
    /// (target) · `remove` · `keep`.
    FixLink {
        /// 링크에 적힌 대로 정규화한 대상 (`notes/x.md`, 또는 `Q/x.md`).
        broken: String,
        sources: Vec<String>,
        options: Vec<LinkOption>,
        /// 문서가 없는 게 아니라 같은 이름이 여럿이라 해석기가 고르지 못한
        /// 링크 — `options` 가 그 문서들.
        #[serde(default)]
        ambiguous: bool,
    },
    /// 한 프로젝트의 확실한 링크 수리 여러 건 — 폴더가 빠졌거나 표기
    /// (대소문자·NFC·`../`)만 다른 것. 선택지: `apply_all` · `keep`
    /// (묶음 전체 보류).
    FixLinkBatch { items: Vec<BatchFix> },
    /// `stale_after` 가 지났다. 선택지: `verify` (확인 표시 + 연장) ·
    /// `supersede` (status: superseded) · `keep`.
    Stale { domain: String, stale_after: String },
    /// 대체된 문서를 아직 인용한다. 선택지: `relink` (replacement 로) ·
    /// `remove` · `keep`.
    CitesSuperseded {
        source: String,
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replacement: Option<String>,
    },
    /// daily 섹션을 주제 문서로. 선택지: `promote` (title/target 덮어쓰기
    /// 가능) · `keep`.
    Promote {
        daily: String,
        /// 원문 헤딩 줄 (`### 결정: …`) — apply 가 섹션을 다시 찾는 열쇠.
        heading: String,
        title: String,
        target: String,
        doc_type: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamCandidate {
    /// 안정적인 중복 제거 키.
    pub key: String,
    pub project: String,
    pub domain: Option<String>,
    pub reason: String,
    pub action: DreamAction,
    /// 낮을수록 먼저 올린다.
    pub priority: u8,
}

impl DreamCandidate {
    pub fn kind(&self) -> &'static str {
        match self.action {
            DreamAction::FixLink { .. } => KIND_FIX_LINK,
            DreamAction::FixLinkBatch { .. } => KIND_FIX_LINK_BATCH,
            DreamAction::Stale { .. } | DreamAction::CitesSuperseded { .. } => KIND_RECHECK,
            DreamAction::Promote { .. } => KIND_PROMOTE,
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(&self.action).unwrap_or(serde_json::Value::Null);
        if let serde_json::Value::Object(ref mut m) = v {
            m.insert("key".into(), serde_json::Value::String(self.key.clone()));
        }
        v
    }
}

// ---------------------------------------------------------------- walking

/// 프로젝트 안의 .md 문서 (깊이 2 까지, 숨김 폴더·_assets 제외),
/// 프로젝트 루트 기준 경로.
pub fn list_docs(vault: &Path, project: &str) -> Vec<String> {
    let root = vault.join(PROJECTS_DIRNAME).join(project);
    let mut out = Vec::new();
    walk(&root, "", 0, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for de in rd.flatten() {
        let path = de.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if name.starts_with('.') || name == vault::ASSETS_DIRNAME {
            continue;
        }
        let rel = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        if path.is_dir() {
            if depth < 2 {
                walk(&path, &rel, depth + 1, out);
            }
        } else if name.to_lowercase().ends_with(".md") {
            out.push(rel);
        }
    }
}

/// 경로·mtime·크기의 지문. 바뀐 프로젝트만 다시 보기 위한 것이라
/// 내용을 읽지 않는다.
pub fn project_fingerprint(vault: &Path, project: &str) -> String {
    let root = vault.join(PROJECTS_DIRNAME).join(project);
    let mut acc: u64 = 0xcbf29ce484222325;
    let mut n = 0u64;
    for rel in list_docs(vault, project) {
        let meta = std::fs::metadata(root.join(&rel)).ok();
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let len = meta.map(|m| m.len()).unwrap_or(0);
        for b in rel.as_bytes().iter().copied().chain(mtime.to_le_bytes()).chain(len.to_le_bytes()) {
            acc ^= b as u64;
            acc = acc.wrapping_mul(0x100000001b3);
        }
        n += 1;
    }
    format!("{n}:{acc:016x}")
}

// ---------------------------------------------------------------- name matching

/// 파일 이름 비교용 정규화 — 링크 해석기(`links::norm_link_name`)와 같은 규칙.
pub fn norm_name(s: &str) -> String {
    links::norm_link_name(s)
}

fn basename(path: &str) -> &str {
    links::link_basename(path)
}

/// 문자 단위 편집 거리 (인접한 두 글자 바꿔 쓴 오타는 1로 센다).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let w = b.len() + 1;
    let mut d = vec![0usize; (a.len() + 1) * w];
    for i in 0..=a.len() {
        d[i * w] = i;
    }
    for j in 0..=b.len() {
        d[j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[(i - 1) * w + j] + 1).min(d[i * w + j - 1] + 1).min(d[(i - 1) * w + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[(i - 2) * w + j - 2] + 1);
            }
            d[i * w + j] = best;
        }
    }
    d[a.len() * w + b.len()]
}

/// 링크 대상이 문서가 아니라 첨부(이미지·pdf 등)로 보이면 건드리지 않는다.
fn looks_like_asset(domain: &str) -> bool {
    let stem = domain.strip_suffix(".md").unwrap_or(domain).to_lowercase();
    // 그림·데이터 파일, 그리고 코드 파일(`[[promotions.ts]]` 처럼 코드를
    // 가리킨 것) — vault 문서가 아니니 깨진 링크로 올리지 않는다.
    const EXTS: [&str; 30] = [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".pdf", ".mp4", ".mov", ".csv", ".json", ".txt",
        ".ts", ".tsx", ".js", ".jsx", ".rs", ".py", ".sh", ".swift", ".kt", ".java", ".go", ".rb", ".yml",
        ".yaml", ".toml", ".sql", ".html", ".css",
    ];
    EXTS.iter().any(|e| stem.ends_with(e))
}

/// "이름을 포함" 후보가 우연한 겹침이 아닌지. 짧은 쪽이 긴 쪽의 1/3 은
/// 넘어야 하고(`상식이` ⊂ `mixpanel-상식이` 같은 건 탈락), 날짜만 겹치는
/// 경우(`…-2026-10-21` ⊃ `2026-10-21`)도 뺀다.
fn contains_meaningfully(a: &str, b: &str) -> bool {
    let (short, long) = if a.chars().count() <= b.chars().count() { (a, b) } else { (b, a) };
    if short.is_empty() || !long.contains(short) {
        return false;
    }
    let (s, l) = (short.chars().count(), long.chars().count());
    if s * 3 < l {
        return false;
    }
    // 숫자·구분자만으로 된 짧은 쪽(날짜·버전)은 의미 있는 겹침이 아니다.
    !short.chars().all(|c| c.is_ascii_digit() || !c.is_alphanumeric())
}

/// daily 노트 — 링크 대상 후보로는 "이름을 포함" 규칙에서 뺀다 (날짜 이름이라
/// 아무 날짜 들어간 링크와 겹친다).
fn is_daily_doc(domain: &str) -> bool {
    domain.starts_with("daily/")
}

/// 프로젝트마다 하나씩 있을 법한 흔한 이름(`norm_name` 결과) — 다른
/// 프로젝트에서 같은 이름을 찾아도 같은 문서라는 근거가 못 된다.
fn is_generic_name(base: &str) -> bool {
    const GENERIC: &[&str] = &[
        "plan", "readme", "index", "todo", "notes", "note", "purpose", "schema", "skill",
        "memoryindex", "overview", "spec", "design", "ui", "backend", "frontend", "api",
        "changelog", "release", "roadmap", "handoff", "summary", "log", "history", "guide",
        "setup", "architecture", "decisions", "faq", "issues", "tasks", "status", "events",
    ];
    if GENERIC.contains(&base) || base.chars().count() < 4 {
        return true;
    }
    // 날짜·숫자뿐인 이름 (daily 2026-10-21 등)
    base.chars().all(|c| c.is_ascii_digit())
}

/// 후보 정렬·중복 제거용 — `norm_name` 이 같은 두 경로는 같은 후보.
fn push_option(opts: &mut Vec<LinkOption>, opt: LinkOption) {
    if let Some(existing) = opts.iter_mut().find(|o| o.target == opt.target) {
        if opt.confidence > existing.confidence {
            *existing = opt;
        }
        return;
    }
    opts.push(opt);
}

/// `broken` (프로젝트 `project` 안의 경로) 에 가까운 실제 문서들.
/// `docs_by_project` 는 프로젝트 → 문서 목록, `renames` 는 git 이 기억하는
/// `Projects/P/old.md` → `Projects/P/new.md`.
pub fn link_options(
    project: &str,
    broken: &str,
    docs_by_project: &HashMap<String, Vec<String>>,
    renames: &HashMap<String, String>,
) -> Vec<LinkOption> {
    let mut opts: Vec<LinkOption> = Vec::new();
    let broken_base = norm_name(basename(broken));
    let broken_full = norm_name(broken);

    let old_rel = format!("{PROJECTS_DIRNAME}/{project}/{broken}");
    if let Some(new_rel) = renames.get(&old_rel) {
        let prefix = format!("{PROJECTS_DIRNAME}/{project}/");
        let target = match new_rel.strip_prefix(&prefix) {
            Some(same) => same.to_string(),
            None => new_rel.trim_start_matches(&format!("{PROJECTS_DIRNAME}/")).to_string(),
        };
        push_option(&mut opts, LinkOption {
            target,
            confidence: 0.95,
            why: "git 기록에서 이름이 바뀐 파일이에요".into(),
        });
    }

    if let Some(docs) = docs_by_project.get(project) {
        for d in docs {
            let d_base = norm_name(basename(d));
            let d_full = norm_name(d);
            if d_full == broken_full {
                push_option(&mut opts, LinkOption {
                    target: d.clone(),
                    confidence: 0.9,
                    why: "대소문자·띄어쓰기만 달라요".into(),
                });
            } else if d_base == broken_base {
                push_option(&mut opts, LinkOption {
                    target: d.clone(),
                    confidence: 0.85,
                    why: "같은 이름이 다른 폴더에 있어요".into(),
                });
            } else if !broken_base.is_empty() {
                let dist = edit_distance(&d_base, &broken_base);
                let longest = d_base.chars().count().max(broken_base.chars().count());
                let allowed = (longest / 4).max(1);
                // 숫자만 다르면(1.1.0 ↔ 1.1.1, P12 ↔ P11, ep04 ↔ ep05) 다른 판·회차라 오타가 아니다.
                let only_numbers_differ = {
                    let strip = |x: &str| x.chars().filter(|c| !c.is_ascii_digit()).collect::<String>();
                    strip(&d_base) == strip(&broken_base)
                };
                if dist <= allowed && longest >= 3 && !only_numbers_differ {
                    push_option(&mut opts, LinkOption {
                        target: d.clone(),
                        confidence: 0.6 - 0.1 * dist as f32,
                        why: "이름이 비슷해요".into(),
                    });
                } else if longest >= 6
                    && contains_meaningfully(&d_base, &broken_base)
                    && !is_daily_doc(d)
                {
                    push_option(&mut opts, LinkOption {
                        target: d.clone(),
                        confidence: 0.45,
                        why: "이름을 포함하고 있어요".into(),
                    });
                }
            }
        }
    }

    // 다른 프로젝트: 같은 경로면 그대로, 이름만 같으면 그 이름이 흔하지
    // 않고(plan·readme·날짜 등 제외) 다른 프로젝트 전체에 하나뿐일 때만.
    // `상식이: [[docs/garden/plan.md]]` 에 엉뚱한 프로젝트의 plan.md 를
    // 권하지 않으려는 것.
    let mut same_base: Vec<(String, String)> = Vec::new();
    let mut same_path = 0usize;
    for (p, docs) in docs_by_project {
        if p == project {
            continue;
        }
        for d in docs {
            // 다른 프로젝트의 같은 날짜 daily 는 다른 날의 기록일 뿐이다.
            if is_daily_doc(d) {
                continue;
            }
            // 폴더 없는 흔한 이름(release.md)은 경로가 같아도 근거가 못 된다.
            if norm_name(d) == broken_full && (broken.contains('/') || !is_generic_name(&broken_base)) {
                push_option(&mut opts, LinkOption {
                    target: format!("{p}/{d}"),
                    confidence: 0.5,
                    why: format!("{p} 프로젝트에 같은 경로가 있어요"),
                });
                same_path += 1;
            } else if norm_name(basename(d)) == broken_base {
                same_base.push((p.clone(), d.clone()));
            }
        }
    }
    if same_path == 0 && same_base.len() == 1 && !is_generic_name(&broken_base) {
        let (p, d) = &same_base[0];
        push_option(&mut opts, LinkOption {
            target: format!("{p}/{d}"),
            confidence: 0.4,
            why: format!("{p} 프로젝트에 같은 이름이 있어요"),
        });
    }

    opts.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    opts.truncate(MAX_LINK_OPTIONS);
    opts
}

/// git 이 기억하는 이름 변경: `Projects/P/old.md` → `Projects/P/new.md`.
/// `wanted` 에 있는 옛 경로만 찾고, 최근 커밋부터 상한까지만 본다.
pub fn git_renames(vault: &Path, wanted: &HashSet<String>) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    if wanted.is_empty() {
        return out;
    }
    let Ok(repo) = git2::Repository::open(vault) else { return out };
    let Ok(mut walk) = repo.revwalk() else { return out };
    if walk.push_head().is_err() {
        return out;
    }
    let mut remaining: HashSet<String> = wanted.clone();
    for oid in walk.flatten().take(GIT_RENAME_COMMITS) {
        if remaining.is_empty() {
            break;
        }
        let Ok(commit) = repo.find_commit(oid) else { continue };
        let Ok(tree) = commit.tree() else { continue };
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
        let Ok(mut diff) = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) else {
            continue;
        };
        // 삭제된 파일이 하나도 없으면 이름 변경도 없다 — find_similar 생략.
        let has_delete = diff.deltas().any(|d| matches!(d.status(), git2::Delta::Deleted));
        if !has_delete {
            continue;
        }
        let mut fo = git2::DiffFindOptions::new();
        fo.renames(true);
        if diff.find_similar(Some(&mut fo)).is_err() {
            continue;
        }
        for delta in diff.deltas() {
            if delta.status() != git2::Delta::Renamed {
                continue;
            }
            let (Some(old), Some(new)) = (delta.old_file().path(), delta.new_file().path()) else {
                continue;
            };
            let old = old.to_string_lossy().to_string();
            if remaining.remove(&old) {
                out.insert(old, new.to_string_lossy().to_string());
            }
        }
    }
    out
}

// ---------------------------------------------------------------- scan: links

fn doc_exists(vault: &Path, project: &str, domain: &str) -> bool {
    vault.join(PROJECTS_DIRNAME).join(project).join(domain).is_file()
}

use links::{is_relative_link, link_dirname as dirname, resolve_relative};

fn is_daily(domain: &str) -> bool {
    domain.starts_with("daily/")
}

/// 프로젝트 안 깨진 링크 후보. `index` 는 vault 전체 링크 색인.
fn scan_broken_links(
    vault: &Path,
    project: &str,
    index: &links::LinkIndex,
    docs_by_project: &HashMap<String, Vec<String>>,
) -> Vec<DreamCandidate> {
    let prefix = format!("{project}/");
    // (broken target "P/domain", 상대 링크면 소스 폴더) → sources. 상대
    // 링크는 소스 폴더마다 다른 문서로 풀리니 폴더별로 따로 묶는다.
    let mut broken: HashMap<(String, Option<String>), BTreeSet<String>> = HashMap::new();
    for (src, outgoing) in &index.outgoing {
        let Some(src_domain) = src.strip_prefix(&prefix) else { continue };
        // SKILL.md 는 스킬 설치용 템플릿 — 안의 `[[…]]` 는 사용법 예시라
        // 고칠 대상이 아니다 (예전 템플릿엔 코드 표기 밖 예시도 있었다).
        if src_domain == crate::vault::SKILL_FILENAME {
            continue;
        }
        let ambiguous_here = index.ambiguous.get(src);
        for l in outgoing {
            if looks_like_asset(&l.domain) {
                continue;
            }
            // 같은 이름이 여럿이라 해석기가 둔 링크는 "없는 문서" 가 아니라
            // "고르기" 항목 (scan_ambiguous_links).
            if ambiguous_here.is_some_and(|list| list.iter().any(|a| a.project == l.project && a.written == l.domain)) {
                continue;
            }
            if !docs_by_project.contains_key(&l.project) {
                // 모르는 프로젝트 — 색인이 이미 하위 폴더로 접었으니 여기 오면
                // 정말 없는 프로젝트. 같은 프로젝트의 경로로 다룬다.
                continue;
            }
            if doc_exists(vault, &l.project, &l.domain) {
                continue;
            }
            let src_dir = (l.project == project && is_relative_link(&l.domain)).then(|| dirname(src_domain).to_string());
            broken
                .entry((format!("{}/{}", l.project, l.domain), src_dir))
                .or_default()
                .insert(src_domain.to_string());
        }
    }
    let mut out = scan_ambiguous_links(project, index);
    if broken.is_empty() {
        out.sort_by(|a, b| a.key.cmp(&b.key));
        return out;
    }
    let wanted: HashSet<String> = broken.keys().map(|(k, _)| format!("{PROJECTS_DIRNAME}/{k}")).collect();
    let renames = git_renames(vault, &wanted);
    for ((target_key, src_dir), sources) in broken {
        let (tproj, tdomain) = target_key.split_once('/').unwrap_or((project, &target_key));
        let resolved = src_dir
            .as_deref()
            .and_then(|d| resolve_relative(d, tdomain))
            .filter(|r| doc_exists(vault, project, r));
        let options = match resolved {
            Some(target) => vec![LinkOption { target, confidence: 0.9, why: "상대 경로를 풀면 이 문서예요".into() }],
            None => link_options(tproj, tdomain, docs_by_project, &renames),
        };
        // 링크에 적을 형태: 같은 프로젝트면 경로만, 다른 프로젝트면 `Q/x.md`.
        let broken_written = if tproj == project { tdomain.to_string() } else { target_key.clone() };
        // 다른 프로젝트를 가리키던 링크면 후보도 `Q/x.md` 꼴로 적어야 한다
        // (후보가 Q 안의 문서일 때만 — 이미 `R/x.md` 꼴인 다른 프로젝트 후보는 그대로).
        let options: Vec<LinkOption> = options
            .into_iter()
            .map(|mut o| {
                if tproj != project && docs_by_project.get(tproj).is_some_and(|d| d.contains(&o.target)) {
                    o.target = format!("{tproj}/{}", o.target);
                }
                o
            })
            .collect();
        let sources: Vec<String> = sources.into_iter().collect();
        let first = sources[0].clone();
        let where_ = if sources.len() > 1 {
            format!("{first} 외 {}곳", sources.len() - 1)
        } else {
            first.clone()
        };
        let reason = match options.first() {
            Some(best) => format!(
                "[[{broken_written}]] 가 가리키는 문서가 없어요 ({where_}). 비슷한 문서: {}",
                best.target
            ),
            None => format!("[[{broken_written}]] 가 가리키는 문서가 없어요 ({where_}). 비슷한 문서를 못 찾았어요."),
        };
        let all_daily = sources.iter().all(|s| is_daily(s));
        let priority = match (options.first().map(|o| o.confidence), all_daily) {
            (Some(c), _) if c >= 0.85 => 0,
            (Some(_), false) => 1,
            (Some(_), true) => 2,
            (None, false) => 3,
            (None, true) => 5,
        };
        let key = match &src_dir {
            Some(d) if !d.is_empty() => format!("fix_link:{project}:{broken_written}@{d}"),
            _ => format!("fix_link:{project}:{broken_written}"),
        };
        out.push(DreamCandidate {
            key,
            project: project.to_string(),
            domain: Some(first),
            reason,
            action: DreamAction::FixLink { broken: broken_written, sources, options, ambiguous: false },
            priority,
        });
    }
    out.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.key.cmp(&b.key)));
    out
}

/// 해석기가 "같은 이름이 둘 이상" 이라 둔 링크 — 그 문서들을 후보로 "어느
/// 쪽인지 고르기" 항목. 확신이 같아 절대 묶음이 되지 않는다.
fn scan_ambiguous_links(project: &str, index: &links::LinkIndex) -> Vec<DreamCandidate> {
    let prefix = format!("{project}/");
    let mut groups: HashMap<String, (links::AmbiguousLink, BTreeSet<String>)> = HashMap::new();
    for (src, list) in &index.ambiguous {
        let Some(src_domain) = src.strip_prefix(&prefix) else { continue };
        for a in list {
            let written = if a.project == project { a.written.clone() } else { format!("{}/{}", a.project, a.written) };
            groups.entry(written).or_insert_with(|| (a.clone(), BTreeSet::new())).1.insert(src_domain.to_string());
        }
    }
    let mut out = Vec::new();
    for (written, (a, sources)) in groups {
        let options: Vec<LinkOption> = a
            .candidates
            .iter()
            .map(|c| LinkOption {
                target: if a.project == project { c.clone() } else { format!("{}/{c}", a.project) },
                confidence: 0.5,
                why: "같은 이름이 여러 폴더에 있어요".into(),
            })
            .collect();
        let sources: Vec<String> = sources.into_iter().collect();
        let first = sources[0].clone();
        let where_ = if sources.len() > 1 { format!("{first} 외 {}곳", sources.len() - 1) } else { first.clone() };
        out.push(DreamCandidate {
            key: format!("fix_link:{project}:{written}"),
            project: project.to_string(),
            domain: Some(first),
            reason: format!(
                "[[{written}]] 와 같은 이름의 문서가 {}개예요 ({where_}). 어느 쪽인지 골라주세요: {}",
                options.len(),
                options.iter().map(|o| o.target.as_str()).collect::<Vec<_>>().join(", ")
            ),
            action: DreamAction::FixLink { broken: written, sources, options, ambiguous: true },
            priority: 1,
        });
    }
    out
}

/// 사람이 고를 게 없는 확실한 수리인가 — 후보가 하나뿐이고(또는 둘째가
/// 한참 낮고) 같은 프로젝트의 문서를 가리킬 때 그 대상.
fn certain_target(project: &str, options: &[LinkOption], docs_by_project: &HashMap<String, Vec<String>>) -> Option<String> {
    let best = options.first()?;
    if best.confidence < 0.85 {
        return None;
    }
    if options.get(1).is_some_and(|o| o.confidence >= 0.85) {
        return None;
    }
    let in_project = docs_by_project.get(project).is_some_and(|d| d.contains(&best.target));
    in_project.then(|| best.target.clone())
}

/// 바로 고쳐도 되는 것(묶음 · 확실한 단일 수리)과 나머지로 나눈다.
fn split_certain(
    project: &str,
    docs_by_project: &HashMap<String, Vec<String>>,
    cands: Vec<DreamCandidate>,
) -> (Vec<DreamCandidate>, Vec<DreamCandidate>) {
    cands.into_iter().partition(|c| match &c.action {
        DreamAction::FixLinkBatch { .. } => true,
        DreamAction::FixLink { options, ambiguous: false, .. } => certain_target(project, options, docs_by_project).is_some(),
        _ => false,
    })
}

/// 한 프로젝트의 확실한 링크 수리가 둘 이상이면 한 항목(`FixLinkBatch`)
/// 으로 묶는다. 애매한 것(후보 여러 개·오타 추정·다른 프로젝트·후보 없음)은
/// 그대로 개별 항목.
pub fn bundle_fix_links(
    project: &str,
    docs_by_project: &HashMap<String, Vec<String>>,
    cands: Vec<DreamCandidate>,
) -> Vec<DreamCandidate> {
    let mut rest: Vec<DreamCandidate> = Vec::new();
    let mut certain: Vec<(DreamCandidate, String)> = Vec::new();
    for c in cands {
        let target = match &c.action {
            DreamAction::FixLink { options, ambiguous: false, .. } => certain_target(project, options, docs_by_project),
            _ => None,
        };
        match target {
            Some(t) => certain.push((c, t)),
            None => rest.push(c),
        }
    }
    if certain.len() < 2 {
        rest.extend(certain.into_iter().map(|(c, _)| c));
        return rest;
    }
    let mut items: Vec<BatchFix> = certain
        .into_iter()
        .map(|(c, target)| match c.action {
            DreamAction::FixLink { broken, sources, options, .. } => BatchFix {
                key: c.key,
                broken,
                target,
                sources,
                why: options.first().map(|o| o.why.clone()).unwrap_or_default(),
            },
            _ => unreachable!("certain candidates are fix-links"),
        })
        .collect();
    items.sort_by(|a, b| a.broken.cmp(&b.broken));
    let docs: BTreeSet<&str> = items.iter().flat_map(|i| i.sources.iter().map(String::as_str)).collect();
    let mut acc: u64 = 0xcbf29ce484222325;
    for b in items.iter().flat_map(|i| i.key.bytes()) {
        acc ^= b as u64;
        acc = acc.wrapping_mul(0x100000001b3);
    }
    rest.push(DreamCandidate {
        key: format!("fix_link_batch:{project}:{acc:016x}"),
        project: project.to_string(),
        domain: None,
        reason: format!(
            "폴더가 빠지거나 표기만 다른 링크 {}종(문서 {}곳)을 한 번에 고칠까요? 예: [[{}]] → {}",
            items.len(),
            docs.len(),
            items[0].broken,
            items[0].target
        ),
        action: DreamAction::FixLinkBatch { items },
        priority: 0,
    });
    rest.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.key.cmp(&b.key)));
    rest
}

// ---------------------------------------------------------------- scan: recheck

fn scan_recheck(
    vault: &Path,
    project: &str,
    index: &links::LinkIndex,
    docs: &[String],
    today: &str,
) -> Vec<DreamCandidate> {
    let mut out = Vec::new();
    let prefix = format!("{project}/");

    for d in docs {
        if is_daily(d) {
            continue;
        }
        let Ok(md) = std::fs::read_to_string(vault.join(PROJECTS_DIRNAME).join(project).join(d)) else {
            continue;
        };
        let trust = ontology::doc_trust(&md, today);
        if !trust.stale {
            continue;
        }
        if ontology::declared_status(&md).as_deref().is_some_and(ontology::is_retired_status) {
            continue;
        }
        let stale_after = trust.stale_after.unwrap_or_default();
        out.push(DreamCandidate {
            key: format!("stale:{project}/{d}:{stale_after}"),
            project: project.to_string(),
            domain: Some(d.clone()),
            reason: format!("stale_after: {stale_after} 이 지났어요. 아직 맞는 내용인지 확인해 주세요."),
            action: DreamAction::Stale { domain: d.clone(), stale_after },
            priority: 2,
        });
    }

    // 대체된 문서를 아직 인용하는 곳. 대체한 문서(supersedes) 자체와 daily 는 제외.
    let mut replacement_of: HashMap<String, String> = HashMap::new();
    for (src, outgoing) in &index.outgoing {
        for l in outgoing {
            match l.relation.as_deref() {
                Some("supersedes") => {
                    replacement_of.insert(format!("{}/{}", l.project, l.domain), src.clone());
                }
                Some("superseded_by") => {
                    replacement_of.insert(src.clone(), format!("{}/{}", l.project, l.domain));
                }
                _ => {}
            }
        }
    }
    for (src, outgoing) in &index.outgoing {
        let Some(src_domain) = src.strip_prefix(&prefix) else { continue };
        if is_daily(src_domain) {
            continue;
        }
        let mut seen: HashSet<String> = HashSet::new();
        for l in outgoing {
            if matches!(l.relation.as_deref(), Some("supersedes") | Some("superseded_by")) {
                continue;
            }
            let target_key = format!("{}/{}", l.project, l.domain);
            let Some(meta) = index.meta.get(&target_key) else { continue };
            let retired = meta
                .effective_status
                .as_deref()
                .is_some_and(ontology::is_retired_status);
            if !retired || !seen.insert(target_key.clone()) {
                continue;
            }
            let replacement = replacement_of.get(&target_key).cloned();
            if replacement.as_deref() == Some(src.as_str()) {
                continue;
            }
            let written = |k: &str| -> String {
                k.strip_prefix(&prefix).map(str::to_string).unwrap_or_else(|| k.to_string())
            };
            let target_written = written(&target_key);
            let replacement_written = replacement.as_deref().map(written);
            let reason = match &replacement_written {
                Some(r) => format!("대체된 문서 {target_written} 를 아직 인용하고 있어요. 새 문서: {r}"),
                None => format!("대체된 문서 {target_written} 를 아직 인용하고 있어요."),
            };
            out.push(DreamCandidate {
                key: format!("cites:{project}/{src_domain}:{target_written}"),
                project: project.to_string(),
                domain: Some(src_domain.to_string()),
                reason,
                action: DreamAction::CitesSuperseded {
                    source: src_domain.to_string(),
                    target: target_written,
                    replacement: replacement_written,
                },
                priority: 2,
            });
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

// ---------------------------------------------------------------- scan: promote

#[derive(Debug, Clone, PartialEq)]
pub struct DailySection {
    /// 원문 헤딩 줄 그대로 (`### 결정: JWT 7일`).
    pub heading: String,
    pub level: usize,
    pub title: String,
    /// 헤딩 다음 줄부터 다음 같은/얕은 헤딩 전까지 (스탬프 줄 포함).
    pub body: String,
}

/// `##`/`###` 섹션으로 나눈다. 코드 펜스 안의 `#` 은 헤딩이 아니다.
pub fn split_sections(md: &str) -> Vec<DailySection> {
    let mut out: Vec<DailySection> = Vec::new();
    let mut cur: Option<(String, usize, Vec<String>)> = None;
    let mut in_fence = false;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        let level = if in_fence { 0 } else { heading_level(line) };
        if level >= 2 {
            if let Some((h, lv, body)) = cur.take() {
                out.push(DailySection { title: section_title(&h), heading: h, level: lv, body: body.join("\n") });
            }
            cur = Some((line.to_string(), level, Vec::new()));
        } else if let Some((_, _, body)) = cur.as_mut() {
            body.push(line.to_string());
        }
    }
    if let Some((h, lv, body)) = cur.take() {
        out.push(DailySection { title: section_title(&h), heading: h, level: lv, body: body.join("\n") });
    }
    out
}

fn heading_level(line: &str) -> usize {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return 0;
    }
    if line[hashes..].starts_with(' ') {
        hashes
    } else {
        0
    }
}

fn section_title(heading: &str) -> String {
    heading.trim_start_matches('#').trim().to_string()
}

/// 승격 대상 분류: (폴더, type). TODO·다음 할 일은 pulse 가 다루니 제외.
pub fn classify_section(title: &str, body: &str) -> Option<(&'static str, &'static str)> {
    let t = title.to_lowercase();
    if ["todo", "할 일", "할일", "다음:", "다음 할", "next"].iter().any(|k| t.contains(k)) {
        return None;
    }
    let body_l = body.to_lowercase();
    let has_rel = |k: &str| body.lines().any(|l| l.trim_start().starts_with(k));
    if ["결정", "decision", "확정", "정함"].iter().any(|k| t.contains(k)) || has_rel("대체::") {
        return Some(("decisions", "decision"));
    }
    if ["원인", "버그", "bug", "재발", "장애", "에러", "error", "fix"].iter().any(|k| t.contains(k))
        || has_rel("원인::")
    {
        return Some(("bugs", "bug"));
    }
    if ["노하우", "know-how", "knowhow", "팁", "tip", "방법", "how to"].iter().any(|k| t.contains(k)) {
        return Some(("notes", "note"));
    }
    if body_l.contains("재발 방지") {
        return Some(("bugs", "bug"));
    }
    None
}

/// 분류 라벨로 쓰는 말 — 제목 앞에 붙은 이것만 떼어낸다.
const TITLE_LABELS: [&str; 16] = [
    "결정", "원인", "버그", "노하우", "팁", "재발 방지", "재발방지", "장애", "에러", "확정",
    "decision", "bug", "fix", "tip", "cause", "know-how",
];

/// `결정` · `결정/완료` · `버그, 재발 방지` 처럼 라벨(들)만으로 된 말머리인가.
fn is_title_label(head: &str) -> bool {
    let parts: Vec<&str> = head.split(['/', '·', ',', '|']).map(str::trim).filter(|p| !p.is_empty()).collect();
    !parts.is_empty()
        && parts.iter().all(|p| {
            let h = p.to_lowercase();
            TITLE_LABELS.iter().any(|l| h == *l) || matches!(h.as_str(), "완료" | "진행" | "보류" | "done" | "wip")
        })
}

/// `결정: X` · `[결정] X` · `결정 — X` → `X`. 라벨이 아닌 말머리
/// (`OG 이미지: X`) 는 제목의 일부라 그대로 둔다.
pub fn strip_title_prefix(title: &str) -> String {
    let mut t = title.trim().to_string();
    if t.starts_with('[') {
        if let Some(end) = t.find(']') {
            if is_title_label(&t[1..end]) {
                t = t[end + 1..].trim().to_string();
            }
        }
    }
    for sep in [": ", "：", " — ", " - ", " · "] {
        if let Some((head, rest)) = t.split_once(sep) {
            if is_title_label(head) && !rest.trim().is_empty() {
                t = rest.trim().to_string();
            }
            break;
        }
    }
    if t.ends_with(':') {
        t.pop();
    }
    t.trim().to_string()
}

/// 제목만으로는 뭔지 알 수 없는 말 — 이런 제목이면 본문 첫 줄에서 따온다.
const GENERIC_TITLES: [&str; 34] = [
    "결정", "결정사항", "결정 사항", "메모", "원인", "버그", "노하우", "팁", "이슈", "문제", "정리", "기록",
    "작업", "노트", "요약", "업데이트", "진행", "진행상황", "진행 상황", "변경", "변경사항", "재발 방지", "에러",
    "장애", "decision", "decisions", "note", "notes", "bug", "bugs", "memo", "log", "update", "summary",
];

fn is_generic_title(t: &str) -> bool {
    let n = norm_name(t);
    n.chars().count() < 3 || GENERIC_TITLES.iter().any(|g| norm_name(g) == n)
}

/// 앞뒤의 `/`·`-`·`*`·따옴표·괄호 같은 기호와 끝의 `:`·`.` 를 뗀다.
fn trim_title_punct(t: &str) -> String {
    let not_word = |c: char| !c.is_alphanumeric() && c != '(' && c != '[' && c != '`';
    let t = t.trim().trim_start_matches(not_word);
    t.trim_end_matches(|c: char| matches!(c, ':' | '.' | '·' | '-' | '—' | ',' | ';')).trim().to_string()
}

/// 본문(스탬프 제외)에서 제목이 될 만한 첫 줄 — bullet 표시·굵게·관계
/// 접두어를 벗기고 첫 문장(40자) 까지.
fn title_from_body(body: &str) -> Option<String> {
    // 항목이 셋 이상인 목록이면 첫 줄이 전체를 대표하지 않는다 — 사람이 짓게 둔다.
    let bullets = body
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            (t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ")) && !l.starts_with(' ') && !l.starts_with('\t')
        })
        .count();
    if bullets >= 3 {
        return None;
    }
    for line in body.lines() {
        let mut l = line.trim();
        if l.is_empty() || l.starts_with("```") || l.starts_with('|') || l.starts_with('>') || crate::daily::is_log_stamp(l) {
            continue;
        }
        // 관계 줄 (`원인:: [[x]]`) 이나 링크만 있는 줄은 제목감이 아니다.
        if l.contains("::") || (l.starts_with("[[") && l.ends_with("]]")) {
            continue;
        }
        l = l.trim_start_matches(['-', '*', '+', '#', '>']).trim_start();
        let l = l
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')')
            .trim_start()
            .replace("**", "");
        let l = trim_title_punct(&l);
        if l.is_empty() {
            continue;
        }
        // 첫 문장만: `. ` · `: ` · ` — ` 앞까지.
        let mut cut = l.clone();
        for sep in [". ", ": ", " — ", " - ", "; "] {
            if let Some((head, _)) = cut.split_once(sep) {
                if head.chars().count() >= 6 {
                    cut = head.to_string();
                }
            }
        }
        let cut: String = cut.chars().take(40).collect();
        let cut = trim_title_punct(&cut);
        return (!is_generic_title(&cut)).then_some(cut);
    }
    None
}

/// 승격 문서의 제목. 라벨을 떼고 기호를 정리한 뒤, 그래도 `결정사항`처럼
/// 일반적이면 본문 첫 줄에서 따온다. 쓸 만한 제목이 없으면 None (후보 제외).
pub fn promote_title(heading_title: &str, body: &str) -> Option<String> {
    let t = strip_title_prefix(heading_title);
    // `2. 로고 업로드 …` 같은 번호 매기기.
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit()).strip_prefix(['.', ')']).map(str::trim_start).unwrap_or(&t).to_string();
    let t = trim_title_punct(&t);
    if !t.is_empty() && !is_generic_title(&t) {
        return Some(t);
    }
    title_from_body(body)
}

/// 파일 이름 조각: 글자·숫자만 남기고 나머지는 `-`, 40자 제한.
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.nfc() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.chars().count() >= 40 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn body_without_stamps(body: &str) -> String {
    body.lines()
        .filter(|l| !crate::daily::is_log_stamp(l))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// 이미 주제 문서로 옮겨졌다고 볼 수 있는 섹션 — 주제 폴더로 나가는 링크가
/// 있거나 정리 표식이 있다.
fn already_promoted(body: &str) -> bool {
    body.contains("로 옮겼어요")
        || ["[[decisions/", "[[bugs/", "[[notes/", "[[decision/", "[[bug/", "[[note/"]
            .iter()
            .any(|k| body.contains(k))
}

fn scan_promote(vault: &Path, project: &str, docs: &[String], today: &str) -> Vec<DreamCandidate> {
    let mut dailies: Vec<&String> = docs
        .iter()
        .filter(|d| {
            d.strip_prefix("daily/")
                .and_then(|f| f.strip_suffix(".md"))
                .is_some_and(|date| !date.contains('/') && NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() && date < today)
        })
        .collect();
    dailies.sort_by(|a, b| b.cmp(a));
    dailies.truncate(MAX_DAILY_SCANNED);

    let existing_norm: HashSet<String> = docs.iter().map(|d| norm_name(basename(d))).collect();
    let root = vault.join(PROJECTS_DIRNAME).join(project);
    let mut out = Vec::new();
    for daily in dailies {
        let date = daily.trim_start_matches("daily/").trim_end_matches(".md");
        let Ok(md) = std::fs::read_to_string(root.join(daily)) else { continue };
        for sec in split_sections(&md) {
            let Some((folder, doc_type)) = classify_section(&sec.title, &sec.body) else { continue };
            if already_promoted(&sec.body) {
                continue;
            }
            let clean = body_without_stamps(&sec.body);
            if clean.chars().count() < MIN_PROMOTE_CHARS {
                continue;
            }
            let Some(title) = promote_title(&sec.title, &clean) else { continue };
            let slug = slugify(&title);
            if slug.is_empty() {
                continue;
            }
            // 같은 이름의 주제 문서가 이미 있으면 사람이 손으로 옮긴 것.
            let t_norm = norm_name(&title);
            if existing_norm.iter().any(|e| e == &t_norm || e == &norm_name(&format!("{date}-{slug}"))) {
                continue;
            }
            let mut target = format!("{folder}/{date}-{slug}.md");
            let mut n = 2;
            while docs.contains(&target) {
                target = format!("{folder}/{date}-{slug}-{n}.md");
                n += 1;
            }
            out.push(DreamCandidate {
                key: format!("promote:{project}/{daily}:{}", norm_name(&sec.title)),
                project: project.to_string(),
                domain: Some(daily.clone()),
                reason: format!("'{}' 을 {target} 로 옮기고 daily 엔 링크만 남길까요?", sec.title),
                action: DreamAction::Promote {
                    daily: daily.clone(),
                    heading: sec.heading.clone(),
                    title,
                    target,
                    doc_type: doc_type.to_string(),
                },
                priority: 4,
            });
        }
    }
    out
}

// ---------------------------------------------------------------- scan: all

/// 한 프로젝트의 정리 후보 전부 (LLM 없음, 아무것도 쓰지 않음).
pub fn scan_project(
    vault: &Path,
    project: &str,
    index: &links::LinkIndex,
    docs_by_project: &HashMap<String, Vec<String>>,
    today: &str,
) -> Vec<DreamCandidate> {
    let empty = Vec::new();
    let docs = docs_by_project.get(project).unwrap_or(&empty);
    let mut out = scan_broken_links(vault, project, index, docs_by_project);
    out.extend(scan_recheck(vault, project, index, docs, today));
    out.extend(scan_promote(vault, project, docs, today));
    out
}

pub fn today_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DreamReport {
    pub scanned_projects: usize,
    pub skipped_projects: usize,
    /// 검토 항목 기준 후보 수 (묶음 1건 = 1).
    pub found: usize,
    /// 종류별 — 묶음은 항목 수와 그 안의 링크 종 수를 따로.
    pub found_links: usize,
    pub found_batches: usize,
    pub found_batch_kinds: usize,
    pub found_recheck: usize,
    pub found_promote: usize,
    pub enqueued: usize,
    pub pending_total: usize,
    pub message: String,
    pub ran_at: i64,
    /// `auto_fix_links` 로 바로 고친 것.
    #[serde(default)]
    pub auto_fixed: Vec<AutoFixed>,
}

impl DreamReport {
    /// "링크 527 · 묶음 14 (136종) · 다시 확인 0 · 승격 1,050" — 0 인 종류는 생략.
    fn breakdown(&self) -> String {
        let mut parts = Vec::new();
        if self.found_links > 0 {
            parts.push(format!("링크 {}", fmt_n(self.found_links)));
        }
        if self.found_batches > 0 {
            parts.push(format!("묶음 {} ({}종)", fmt_n(self.found_batches), fmt_n(self.found_batch_kinds)));
        }
        if self.found_recheck > 0 {
            parts.push(format!("다시 확인 {}", fmt_n(self.found_recheck)));
        }
        if self.found_promote > 0 {
            parts.push(format!("승격 {}", fmt_n(self.found_promote)));
        }
        parts.join(" · ")
    }
}

/// 1234 → "1,234".
fn fmt_n(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 승격 제목을 다듬는 선택적 훅 (LLM). `(title, body) -> 새 제목`. 실패하면
/// None 을 돌려주면 되고, 휴리스틱 제목이 그대로 쓰인다.
pub type TitlePolish<'a> = &'a (dyn Fn(&str, &str) -> Option<String> + Send + Sync);

/// 정리 한 바퀴. 바뀐 프로젝트(지문이 달라졌거나 24시간 지난 것)만 보고,
/// 이미 답한 후보·검토함에 떠 있는 후보는 건너뛰며, `max_per_run` 개까지
/// 검토함에 올린다. `force` 면 지문을 무시하고 전부 본다.
pub fn run(
    vault: &Path,
    max_per_run: usize,
    force: bool,
    polish: Option<TitlePolish<'_>>,
) -> DanbiResult<DreamReport> {
    run_with(vault, &RunOptions { max_per_run, force, ..Default::default() }, polish)
}

/// 한 번 돌 때의 선택.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub max_per_run: usize,
    /// 바뀌지 않은 프로젝트도 다시 본다.
    pub force: bool,
    /// 확실한 링크 수리(같은 문서로 정해지는 것)는 검토함에 올리지 않고 바로
    /// 고친다 — 단비 이름으로 커밋, 되돌릴 수 있음. 설정 `auto_fix_links`.
    pub auto_fix_links: bool,
    /// 이 프로젝트만 본다 (에이전트의 `danbi_tidy`). 검토함 상한도 이
    /// 프로젝트의 대기 수로 센다 — 다른 프로젝트가 20건을 채워도 막히지 않게.
    pub only_project: Option<String>,
}

/// 자동으로 고친 확실한 링크 — 로그·보고용.
#[derive(Debug, Clone, Serialize)]
pub struct AutoFixed {
    pub project: String,
    pub message: String,
    pub changed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

pub fn run_with(vault: &Path, opts: &RunOptions, polish: Option<TitlePolish<'_>>) -> DanbiResult<DreamReport> {
    let max_per_run = opts.max_per_run;
    let force = opts.force;
    let now = Local::now().timestamp();
    let today = today_str();
    let mut store = load_store(vault)?;
    let review_store = reviews::load(vault)?;
    let pending_dream = review_store
        .items
        .iter()
        .filter(|it| it.status == reviews::ReviewStatus::Pending && it.kind.starts_with(KIND_PREFIX))
        .filter(|it| opts.only_project.as_ref().map(|p| it.project.as_ref() == Some(p)).unwrap_or(true))
        .count();
    let mut report = DreamReport { ran_at: now, pending_total: pending_dream, ..Default::default() };

    let mut projects = vault::project_names(vault).unwrap_or_default();
    if let Some(only) = &opts.only_project {
        projects.retain(|p| p == only);
    }
    if projects.is_empty() {
        report.message = "프로젝트가 없어요.".into();
        store.last_run_at = Some(now);
        store.last_message = Some(report.message.clone());
        save_store(vault, &store)?;
        return Ok(report);
    }

    let mut docs_by_project: HashMap<String, Vec<String>> = HashMap::new();
    let mut fingerprints: HashMap<String, String> = HashMap::new();
    let mut to_scan: Vec<String> = Vec::new();
    for p in vault::project_names(vault).unwrap_or_default() {
        if !projects.contains(&p) {
            docs_by_project.insert(p.clone(), list_docs(vault, &p));
        }
    }
    for p in &projects {
        docs_by_project.insert(p.clone(), list_docs(vault, p));
        let fp = project_fingerprint(vault, p);
        let changed = match store.project_marks.get(p) {
            Some(mark) => mark.fingerprint != fp || now - mark.at > RESCAN_AFTER_SECS,
            None => true,
        };
        fingerprints.insert(p.clone(), fp);
        if force || changed {
            to_scan.push(p.clone());
        } else {
            report.skipped_projects += 1;
        }
    }

    if to_scan.is_empty() {
        report.message = "바뀐 프로젝트가 없어 쉬었어요.".into();
        store.last_run_at = Some(now);
        store.last_message = Some(report.message.clone());
        save_store(vault, &store)?;
        return Ok(report);
    }

    let budget = max_per_run.min(MAX_PENDING.saturating_sub(pending_dream));
    // 캐시(`build_index_cached`)를 쓰지 않는다: 한 바퀴에 한 번 걷는 비용은
    // 작고, 전역 캐시 한 칸을 다른 호출자와 다투지 않는 편이 낫다.
    let index = links::build_index(vault)?;
    let known: HashSet<String> = covered_keys(&review_store);

    let mut candidates: Vec<DreamCandidate> = Vec::new();
    for p in &to_scan {
        let found = scan_project(vault, p, &index, &docs_by_project, &today);
        report.scanned_projects += 1;
        let fresh: Vec<DreamCandidate> = found
            .into_iter()
            .filter(|c| !store.decided.contains_key(&c.key) && !known.contains(&c.key))
            .collect();
        let bundled = bundle_fix_links(p, &docs_by_project, fresh);
        if opts.auto_fix_links {
            let (certain, rest) = split_certain(p, &docs_by_project, bundled);
            for c in certain {
                let choice = match &c.action {
                    DreamAction::FixLinkBatch { .. } => Choice::ApplyAll,
                    DreamAction::FixLink { options, .. } => match certain_target(p, options, &docs_by_project) {
                        Some(target) => Choice::Relink { target },
                        None => continue,
                    },
                    _ => continue,
                };
                // 이미 손으로 고쳤거나 문서가 바뀌어 실패하면 조용히 넘긴다 — 다음에 다시 본다.
                if let Ok(r) = execute(vault, p, &c.key, &c.action, &choice, &Actor::Danbi) {
                    report.auto_fixed.push(AutoFixed { project: p.clone(), message: r.message, changed: r.changed, commit: r.commit });
                }
            }
            candidates.extend(rest);
        } else {
            candidates.extend(bundled);
        }
        // 이 프로젝트는 본 걸로 표시 — 올리지 못한 후보는 다음 변경 때 다시 본다.
        store.project_marks.insert(
            p.clone(),
            ProjectMark { fingerprint: fingerprints.get(p).cloned().unwrap_or_default(), at: now },
        );
    }
    report.found = candidates.len();
    for c in &candidates {
        match &c.action {
            DreamAction::FixLink { .. } => report.found_links += 1,
            DreamAction::FixLinkBatch { items } => {
                report.found_batches += 1;
                report.found_batch_kinds += items.len();
            }
            DreamAction::Stale { .. } | DreamAction::CitesSuperseded { .. } => report.found_recheck += 1,
            DreamAction::Promote { .. } => report.found_promote += 1,
        }
    }

    for mut c in pick_round_robin(candidates, budget) {
        let project = c.project.clone();
        if let (Some(polish), DreamAction::Promote { daily, heading, title, target, .. }) = (polish, &mut c.action) {
            let body = read_section_body(vault, &project, daily, heading);
            if let Some(better) = polish(title, &body).map(|t| t.trim().to_string()).filter(|t| !t.is_empty() && t.chars().count() <= 60) {
                let slug = slugify(&better);
                if !slug.is_empty() {
                    let folder = target.split('/').next().unwrap_or("notes").to_string();
                    let date = daily.trim_start_matches("daily/").trim_end_matches(".md");
                    *target = format!("{folder}/{date}-{slug}.md");
                    *title = better;
                }
            }
        }
        reviews::enqueue_with_payload(
            vault,
            c.kind(),
            Some(c.project.clone()),
            c.domain.clone(),
            c.reason.clone(),
            Some(c.payload()),
        )?;
        report.enqueued += 1;
    }
    report.pending_total = pending_dream + report.enqueued;
    let auto_note = if report.auto_fixed.is_empty() {
        String::new()
    } else {
        let docs: usize = report.auto_fixed.iter().map(|a| a.changed.len()).sum();
        format!("확실한 링크는 바로 고쳤어요 (문서 {docs}곳). ")
    };
    report.message = if report.found == 0 && !auto_note.is_empty() {
        format!("{auto_note}다른 정리 거리는 없었어요.")
    } else if report.found == 0 {
        format!("{}개 프로젝트를 봤는데 정리할 게 없었어요.", report.scanned_projects)
    } else if report.enqueued == 0 {
        format!("정리 거리 {}건을 찾았지만 검토함이 차 있어 쉬었어요 ({}).", fmt_n(report.found), report.breakdown())
    } else if report.enqueued < report.found {
        format!(
            "검토함에 {}건을 올렸어요. 정리 거리는 모두 {}건 ({}) — 나머지는 다음에.",
            report.enqueued,
            fmt_n(report.found),
            report.breakdown()
        )
    } else {
        format!("정리 거리 {}건을 모두 검토함에 올렸어요 ({}).", report.enqueued, report.breakdown())
    };
    if !auto_note.is_empty() && report.found > 0 {
        report.message = format!("{auto_note}{}", report.message);
    }
    store.last_run_at = Some(now);
    store.last_message = Some(report.message.clone());
    save_store(vault, &store)?;
    Ok(report)
}

/// 검토함에 올라간(어떤 상태든) 정리 항목이 덮는 후보 키 전부 — 묶음이면
/// 그 안의 개별 키까지. 같은 걸 다시 올리지 않기 위한 것.
fn covered_keys(store: &reviews::ReviewStore) -> HashSet<String> {
    let mut out = HashSet::new();
    for it in store.items.iter().filter(|it| it.kind.starts_with(KIND_PREFIX)) {
        let Some(p) = it.payload.as_ref() else { continue };
        if let Some(k) = p.get("key").and_then(|k| k.as_str()) {
            out.insert(k.to_string());
        }
        for item in p.get("items").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(k) = item.get("key").and_then(|k| k.as_str()) {
                out.insert(k.to_string());
            }
        }
    }
    out
}

/// 종류 묶음: 0 링크 · 1 다시 확인 · 2 승격.
fn category(a: &DreamAction) -> usize {
    match a {
        DreamAction::FixLink { .. } | DreamAction::FixLinkBatch { .. } => 0,
        DreamAction::Stale { .. } | DreamAction::CitesSuperseded { .. } => 1,
        DreamAction::Promote { .. } => 2,
    }
}

/// 같은 종류 안에서도 프로젝트를 돌아가며 — 한 프로젝트의 daily 가 많다고
/// 그 프로젝트만 올라오지 않게. 순서는 입력(priority → key)을 따른다.
fn interleave_by_project(cands: Vec<DreamCandidate>) -> Vec<DreamCandidate> {
    let mut lanes: Vec<(String, std::collections::VecDeque<DreamCandidate>)> = Vec::new();
    for c in cands {
        match lanes.iter_mut().find(|(p, _)| *p == c.project) {
            Some((_, q)) => q.push_back(c),
            None => lanes.push((c.project.clone(), std::iter::once(c).collect())),
        }
    }
    let mut out = Vec::new();
    while lanes.iter().any(|(_, q)| !q.is_empty()) {
        for (_, q) in lanes.iter_mut() {
            if let Some(c) = q.pop_front() {
                out.push(c);
            }
        }
    }
    out
}

/// 종류별로 돌아가며 하나씩 뽑는다 — 링크가 수백 건이어도 다시 확인·승격이
/// 굶지 않게. 각 종류 안에서는 priority → key 순, 프로젝트는 번갈아.
pub fn pick_round_robin(cands: Vec<DreamCandidate>, budget: usize) -> Vec<DreamCandidate> {
    let mut buckets: Vec<Vec<DreamCandidate>> = vec![Vec::new(), Vec::new(), Vec::new()];
    for c in cands {
        buckets[category(&c.action)].push(c);
    }
    for b in buckets.iter_mut() {
        b.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.key.cmp(&b.key)));
        *b = interleave_by_project(std::mem::take(b));
        b.reverse(); // pop() 이 앞에서부터 꺼내도록
    }
    let mut out = Vec::new();
    while out.len() < budget && buckets.iter().any(|b| !b.is_empty()) {
        for b in buckets.iter_mut() {
            if out.len() >= budget {
                break;
            }
            if let Some(c) = b.pop() {
                out.push(c);
            }
        }
    }
    out
}

fn read_section_body(vault: &Path, project: &str, daily: &str, heading: &str) -> String {
    let md = vault::read_doc(vault, project, daily).unwrap_or_default();
    split_sections(&md)
        .into_iter()
        .find(|s| s.heading == heading)
        .map(|s| body_without_stamps(&s.body))
        .unwrap_or_default()
}

// ---------------------------------------------------------------- apply

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub changed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub message: String,
}

/// 사용자의 선택. `relink` 는 FixLink/CitesSuperseded 의 새 대상, `promote`
/// 는 제목·경로를 덮어쓸 수 있다. `keep` 은 아무것도 바꾸지 않고 다시
/// 올리지 않도록 기억만 한다.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "choice", rename_all = "snake_case")]
pub enum Choice {
    Relink { target: String },
    Remove,
    Keep,
    /// 묶음 수리 전부 적용.
    ApplyAll,
    Verify,
    Supersede,
    Promote {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        target: Option<String>,
    },
}

fn remember_all(vault: &Path, keys: impl IntoIterator<Item = String>, status: &str) -> DanbiResult<()> {
    let mut store = load_store(vault)?;
    let now = Local::now().timestamp();
    for key in keys {
        store.decided.insert(key, Decision { status: status.into(), at: now });
    }
    save_store(vault, &store)
}

/// 항목 키 + (묶음이면) 안의 개별 키.
fn keys_of(key: &str, action: &DreamAction) -> Vec<String> {
    let mut v = vec![key.to_string()];
    if let DreamAction::FixLinkBatch { items } = action {
        v.extend(items.iter().map(|i| i.key.clone()));
    }
    v
}

/// 검토함에서 항목이 해결/무시될 때 — `dream_*` 이면 키를 기억한다.
pub fn on_review_resolved(vault: &Path, item: &reviews::ReviewItem, status: &reviews::ReviewStatus) {
    if !item.kind.starts_with(KIND_PREFIX) {
        return;
    }
    let Ok((key, action)) = parse_action(item) else { return };
    let s = match status {
        reviews::ReviewStatus::Resolved => "applied",
        reviews::ReviewStatus::Dismissed => "dismissed",
        reviews::ReviewStatus::Pending => return,
    };
    let _ = remember_all(vault, keys_of(&key, &action), s);
}

fn parse_action(item: &reviews::ReviewItem) -> DanbiResult<(String, DreamAction)> {
    let payload = item
        .payload
        .clone()
        .ok_or_else(|| DanbiError::Config("정리 항목에 실행 정보가 없어요.".into()))?;
    let key = payload.get("key").and_then(|k| k.as_str()).unwrap_or("").to_string();
    let action: DreamAction =
        serde_json::from_value(payload).map_err(|e| DanbiError::Config(format!("정리 항목을 읽을 수 없어요: {e}")))?;
    Ok((key, action))
}

/// 검토함 항목 `id` 에 선택을 적용한다. 파일을 고치는 선택은 git 에
/// `(pre)` 스냅샷 → 쓰기 → 커밋 순서로 남긴다 (undo 가능). 끝나면 항목을
/// resolved/dismissed 로 바꾸고 키를 기억한다.
pub fn apply(vault: &Path, id: &str, choice: Choice) -> DanbiResult<ApplyResult> {
    apply_as(vault, id, choice, &Actor::Human)
}

/// 누가 정리했는지 — 커밋 주체와 `verified.by` 에 남는다.
#[derive(Debug, Clone)]
pub enum Actor {
    /// 검토함에서 사람이 고름.
    Human,
    /// MCP 의 에이전트. `client` 는 커밋 꼬리표(`claude-code` 등), `note` 는
    /// 왜 그렇게 골랐는지(근거) — 커밋 메시지에 남는다.
    Agent { client: String, note: Option<String> },
    /// 단비가 혼자 (확실한 링크 자동 수리).
    Danbi,
}

impl Actor {
    fn client(&self) -> &str {
        match self {
            Actor::Human => vcs::CLIENT_HUMAN,
            Actor::Agent { client, .. } => client,
            Actor::Danbi => vcs::CLIENT_DANBI,
        }
    }
    fn verified_by(&self) -> &'static str {
        match self {
            Actor::Human => "human",
            Actor::Agent { .. } => "agent",
            Actor::Danbi => "danbi",
        }
    }
    fn note(&self) -> Option<&str> {
        match self {
            Actor::Agent { note, .. } => note.as_deref().map(str::trim).filter(|n| !n.is_empty()),
            _ => None,
        }
    }
}

/// `apply` 를 주체를 밝혀 실행. 에이전트에게는 두 가지를 막는다:
/// "그대로 두기"(사람이 다시는 못 보게 되므로 — 애매하면 손대지 말고
/// 남겨 두면 된다)와 근거 없는 "아직 맞아요/대체됨"(틀린 기억에 확인
/// 도장을 찍는 게 가장 나쁜 경우).
pub fn apply_as(vault: &Path, id: &str, choice: Choice, actor: &Actor) -> DanbiResult<ApplyResult> {
    if let Actor::Agent { .. } = actor {
        if matches!(choice, Choice::Keep) {
            return Err(DanbiError::Config(
                "에이전트는 '그대로 두기' 를 고르지 않아요 — 확신이 없으면 이 항목은 손대지 말고 두세요. 사람이 검토함에서 고릅니다.".into(),
            ));
        }
        if matches!(choice, Choice::Verify | Choice::Supersede) && actor.note().is_none() {
            return Err(DanbiError::Config(
                "verify·supersede 는 실제로 확인한 근거(evidence)가 있어야 해요 — 코드·문서를 확인했다면 무엇을 봤는지 한 줄로 적어 다시 부르세요. 확인할 수 없으면 사람에게 남기세요.".into(),
            ));
        }
    }
    let store = reviews::load(vault)?;
    let item = reviews::find(&store, id)
        .cloned()
        .ok_or_else(|| DanbiError::Config(format!("정리 항목을 찾을 수 없어요: {id}")))?;
    if item.status != reviews::ReviewStatus::Pending {
        return Err(DanbiError::Config("이미 처리한 항목이에요.".into()));
    }
    let project = item
        .project
        .clone()
        .ok_or_else(|| DanbiError::Config("정리 항목에 프로젝트가 없어요.".into()))?;
    let (key, action) = parse_action(&item)?;

    if matches!(choice, Choice::Keep) {
        reviews::resolve(vault, id, reviews::ReviewStatus::Dismissed)?;
        remember_all(vault, keys_of(&key, &action), "dismissed")?;
        return Ok(ApplyResult { changed: Vec::new(), commit: None, message: "그대로 두고 다시 묻지 않을게요.".into() });
    }

    let res = execute(vault, &project, &key, &action, &choice, actor)?;
    reviews::resolve(vault, id, reviews::ReviewStatus::Resolved)?;
    Ok(res)
}

/// 선택을 실제로 실행: `(pre)` 스냅샷 → 쓰기 → 주체 이름으로 커밋 → 키 기억.
/// 검토함 항목과 무관하게 부를 수 있다 (자동 수리).
fn execute(
    vault: &Path,
    project: &str,
    key: &str,
    action: &DreamAction,
    choice: &Choice,
    actor: &Actor,
) -> DanbiResult<ApplyResult> {
    let project = project.to_string();
    let _guard = vcs::write_guard();
    vcs::ensure_repo(vault)?;
    let label = short_label(action);
    let _ = vcs::snapshot(vault, &format!("danbi: dreaming {label} · {project} (pre)"));
    let (changed, message) = execute_inner(vault, &project, action, choice, actor)?;
    let msg = match actor.note() {
        Some(n) => format!("danbi: dreaming {label} · {project} — {}", n.chars().take(200).collect::<String>().replace('\n', " ")),
        None => format!("danbi: dreaming {label} · {project}"),
    };
    let commit = vcs::snapshot_as(vault, &msg, actor.client())?;
    remember_all(vault, keys_of(key, action), "applied")?;
    Ok(ApplyResult { changed, commit, message })
}

/// 파일만 고친다 — 잠금·스냅샷·커밋·기억은 부르는 쪽이.
fn execute_inner(
    vault: &Path,
    project: &str,
    action: &DreamAction,
    choice: &Choice,
    actor: &Actor,
) -> DanbiResult<(Vec<String>, String)> {
    let project = project.to_string();
    let out = match (action, choice) {
        (DreamAction::FixLink { broken, sources, .. }, Choice::Relink { target }) => {
            let n = rewrite_links(vault, &project, sources, broken, Some(target))?;
            (n, format!("[[{broken}]] → [[{target}]] 로 바꿨어요."))
        }
        (DreamAction::FixLink { broken, sources, .. }, Choice::Remove) => {
            let n = rewrite_links(vault, &project, sources, broken, None)?;
            (n, format!("[[{broken}]] 링크를 풀어 글자만 남겼어요."))
        }
        (DreamAction::FixLinkBatch { items }, Choice::ApplyAll) => {
            let mut changed: BTreeSet<String> = BTreeSet::new();
            let mut fixed = 0usize;
            for it in items {
                // 이미 손으로 고친 줄은 건너뛰고 나머지를 계속한다.
                if let Ok(files) = rewrite_links(vault, &project, &it.sources, &it.broken, Some(&it.target)) {
                    changed.extend(files);
                    fixed += 1;
                }
            }
            if changed.is_empty() {
                return Err(DanbiError::Config("바꿀 링크를 찾지 못했어요 — 이미 고쳐졌을 수 있어요.".into()));
            }
            let n = changed.len();
            (changed.into_iter().collect(), format!("링크 {fixed}종을 고쳤어요 (문서 {n}곳)."))
        }
        (DreamAction::CitesSuperseded { source, target, .. }, Choice::Relink { target: new }) => {
            let n = rewrite_links(vault, &project, std::slice::from_ref(source), target, Some(new))?;
            (n, format!("[[{target}]] → [[{new}]] 로 바꿨어요."))
        }
        (DreamAction::CitesSuperseded { source, target, .. }, Choice::Remove) => {
            let n = rewrite_links(vault, &project, std::slice::from_ref(source), target, None)?;
            (n, format!("[[{target}]] 링크를 풀어 글자만 남겼어요."))
        }
        (DreamAction::Stale { domain, .. }, Choice::Verify) => {
            let today = today_str();
            let until = (Local::now().date_naive() + chrono::Duration::days(VERIFY_EXTEND_DAYS))
                .format("%Y-%m-%d")
                .to_string();
            let update = ontology::MetaUpdate {
                set: vec![("stale_after".into(), until.clone())],
                set_raw: vec![("verified".into(), format!("{{ by: {}, at: {today} }}", actor.verified_by()))],
                add_items: Vec::new(),
            };
            set_meta(vault, &project, domain, &update)?;
            (vec![domain.clone()], format!("확인 표시를 남기고 stale_after 를 {until} 로 미뤘어요."))
        }
        (DreamAction::Stale { domain, .. }, Choice::Supersede) => {
            let vocab = ontology::vocabulary_for(vault, &project);
            let status = vocab.canonical_status("superseded").unwrap_or_else(|| "superseded".into());
            let update = ontology::MetaUpdate { set: vec![("status".into(), status)], ..Default::default() };
            set_meta(vault, &project, domain, &update)?;
            (vec![domain.clone()], "status: superseded 로 표시했어요.".into())
        }
        (DreamAction::Promote { daily, heading, title, target, doc_type }, Choice::Promote { title: t2, target: g2 }) => {
            let title = t2.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(title);
            let target = g2.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(normalize_target).unwrap_or_else(|| target.clone());
            promote(vault, &project, daily, heading, title, &target, doc_type)?;
            (vec![daily.clone(), target.clone()], format!("{target} 를 만들고 daily 엔 링크만 남겼어요."))
        }
        _ => {
            return Err(DanbiError::Config("이 항목에는 맞지 않는 선택이에요.".into()));
        }
    };
    Ok(out)
}

/// 검토함의 "추천대로 적용" — 여러 항목을 한 번에. `(pre)` 스냅샷 하나와
/// 커밋 하나로 묶어 한 번에 되돌릴 수 있게 한다. 실패한 항목(이미 손으로
/// 고쳤거나 문서가 바뀜)은 건너뛰고 대기 상태로 남긴다.
#[derive(Debug, Clone, Serialize)]
pub struct BulkResult {
    pub ok: usize,
    pub failed: Vec<BulkFailure>,
    pub changed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BulkFailure {
    pub id: String,
    pub error: String,
}

pub fn apply_many(vault: &Path, picks: Vec<(String, Choice)>) -> DanbiResult<BulkResult> {
    let actor = Actor::Human;
    let _guard = vcs::write_guard();
    vcs::ensure_repo(vault)?;
    let _ = vcs::snapshot(vault, &format!("danbi: dreaming 일괄 {}건 (pre)", picks.len()));
    let store = reviews::load(vault)?;
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let mut ok = 0usize;
    let mut failed = Vec::new();
    let mut projects: BTreeSet<String> = BTreeSet::new();
    for (id, choice) in picks {
        let res = (|| -> DanbiResult<Vec<String>> {
            if matches!(choice, Choice::Keep) {
                return Err(DanbiError::Config("일괄 적용에는 '그대로 두기' 를 넣지 않아요.".into()));
            }
            let item = reviews::find(&store, &id)
                .cloned()
                .ok_or_else(|| DanbiError::Config("정리 항목을 찾을 수 없어요.".into()))?;
            if item.status != reviews::ReviewStatus::Pending {
                return Err(DanbiError::Config("이미 처리한 항목이에요.".into()));
            }
            let project = item.project.clone().ok_or_else(|| DanbiError::Config("프로젝트가 없어요.".into()))?;
            let (key, action) = parse_action(&item)?;
            let (files, _) = execute_inner(vault, &project, &action, &choice, &actor)?;
            remember_all(vault, keys_of(&key, &action), "applied")?;
            reviews::resolve(vault, &id, reviews::ReviewStatus::Resolved)?;
            projects.insert(project.clone());
            Ok(files.into_iter().map(|f| format!("{project}/{f}")).collect())
        })();
        match res {
            Ok(files) => {
                ok += 1;
                changed.extend(files);
            }
            Err(e) => failed.push(BulkFailure { id, error: e.to_string() }),
        }
    }
    let commit = if ok > 0 {
        let names: Vec<&str> = projects.iter().map(String::as_str).collect();
        vcs::snapshot_as(vault, &format!("danbi: dreaming 일괄 {ok}건 · {}", names.join(", ")), actor.client())?
    } else {
        None
    };
    let message = if failed.is_empty() {
        format!("{ok}건을 적용했어요 (문서 {}곳).", changed.len())
    } else {
        format!("{ok}건을 적용하고 {}건은 이미 바뀌어 건너뛰었어요.", failed.len())
    };
    Ok(BulkResult { ok, failed, changed: changed.into_iter().collect(), commit, message })
}

fn short_label(a: &DreamAction) -> &'static str {
    match a {
        DreamAction::FixLink { .. } => "fix-link",
        DreamAction::FixLinkBatch { .. } => "fix-links",
        DreamAction::Stale { .. } => "recheck",
        DreamAction::CitesSuperseded { .. } => "relink",
        DreamAction::Promote { .. } => "promote",
    }
}

fn normalize_target(s: &str) -> String {
    let s = s.trim().trim_start_matches("[[").trim_end_matches("]]").trim();
    if s.to_lowercase().ends_with(".md") {
        s.to_string()
    } else {
        format!("{s}.md")
    }
}

fn set_meta(vault: &Path, project: &str, domain: &str, update: &ontology::MetaUpdate) -> DanbiResult<()> {
    let Some((current, _)) = vault::read_doc_opt(vault, project, domain)? else {
        return Err(DanbiError::Config(format!("문서가 없어요: {project}/{domain}")));
    };
    let next = ontology::set_frontmatter(&current, update).map_err(DanbiError::Config)?;
    if next != current {
        vault::write_doc(vault, project, domain, &next)?;
    }
    Ok(())
}

/// 링크 대상이 `broken` 과 같은 문서를 가리키는지 — `x.md`, `x`, `P/x.md`
/// 어느 표기든.
fn same_target(inner_target: &str, project: &str, broken: &str) -> bool {
    let want = normalize_target(broken);
    let got = normalize_target(inner_target);
    if got == want {
        return true;
    }
    // 링크에 프로젝트가 붙어 있고 want 에는 없는 경우, 또는 그 반대.
    let strip = |s: &str| -> String {
        s.strip_prefix(&format!("{project}/")).map(str::to_string).unwrap_or_else(|| s.to_string())
    };
    strip(&got) == strip(&want)
}

/// 문서 본문에서 `[[broken]]` 계열 링크를 `[[new]]` 로 바꾸거나 (None 이면)
/// 대괄호를 풀어 글자만 남긴다. 별칭(`|별칭`)은 유지.
pub fn rewrite_links_in(md: &str, project: &str, broken: &str, new_target: Option<&str>) -> (String, usize) {
    let mut out = String::with_capacity(md.len());
    let mut last = 0;
    let mut n = 0;
    for (at, _) in links::extract_links_at(md, project) {
        if at < last {
            continue;
        }
        let Some(rel_end) = md[at + 2..].find("]]") else { break };
        let end = at + 2 + rel_end;
        let inner = &md[at + 2..end];
        let (target, alias) = match inner.split_once('|') {
            Some((t, a)) => (t.trim(), Some(a)),
            None => (inner.trim(), None),
        };
        let target_no_anchor = target.split('#').next().unwrap_or(target);
        if !same_target(target_no_anchor, project, broken) {
            continue;
        }
        out.push_str(&md[last..at]);
        match new_target {
            Some(nt) => {
                // 원래 `.md` 없이 적었으면 새 링크도 그 표기를 따른다.
                let nt = if target_no_anchor.to_lowercase().ends_with(".md") {
                    nt
                } else {
                    nt.strip_suffix(".md").unwrap_or(nt)
                };
                out.push_str("[[");
                out.push_str(nt);
                if let Some(anchor) = target.strip_prefix(target_no_anchor) {
                    out.push_str(anchor);
                }
                if let Some(a) = alias {
                    out.push('|');
                    out.push_str(a);
                }
                out.push_str("]]");
            }
            None => {
                let text = alias.map(str::trim).unwrap_or(target_no_anchor);
                out.push_str(text.strip_suffix(".md").unwrap_or(text));
            }
        }
        last = end + 2;
        n += 1;
    }
    out.push_str(&md[last..]);
    (out, n)
}

fn rewrite_links(
    vault: &Path,
    project: &str,
    sources: &[String],
    broken: &str,
    new_target: Option<&str>,
) -> DanbiResult<Vec<String>> {
    if let Some(nt) = new_target {
        let (p, d) = match nt.split_once('/') {
            Some((p, d)) if vault::project_exists(vault, p) => (p.to_string(), d.to_string()),
            _ => (project.to_string(), nt.to_string()),
        };
        if !doc_exists(vault, &p, &d) {
            return Err(DanbiError::Config(format!("바꿀 대상 문서가 없어요: {nt}")));
        }
    }
    let mut changed = Vec::new();
    for src in sources {
        let Some((md, _)) = vault::read_doc_opt(vault, project, src)? else { continue };
        let (next, n) = rewrite_links_in(&md, project, broken, new_target);
        if n > 0 && next != md {
            vault::write_doc(vault, project, src, &next)?;
            changed.push(src.clone());
        }
    }
    if changed.is_empty() {
        return Err(DanbiError::Config("바꿀 링크를 찾지 못했어요 — 이미 고쳐졌을 수 있어요.".into()));
    }
    Ok(changed)
}

/// daily 섹션을 새 문서로 옮기고 daily 엔 링크 한 줄만 남긴다.
fn promote(
    vault: &Path,
    project: &str,
    daily: &str,
    heading: &str,
    title: &str,
    target: &str,
    doc_type: &str,
) -> DanbiResult<()> {
    if doc_exists(vault, project, target) {
        return Err(DanbiError::Config(format!("이미 있는 문서예요: {target}")));
    }
    if !target.to_lowercase().ends_with(".md") || target.contains("..") || target.starts_with('/') {
        return Err(DanbiError::Config(format!("문서 경로가 이상해요: {target}")));
    }
    let md = vault::read_doc(vault, project, daily)?;
    let sections = split_sections(&md);
    let Some(sec) = sections.iter().find(|s| s.heading == heading) else {
        return Err(DanbiError::Config("daily 에서 그 섹션을 찾지 못했어요 — 이미 바뀌었을 수 있어요.".into()));
    };
    let stamp = sec.body.lines().find(|l| crate::daily::is_log_stamp(l)).map(str::to_string);
    let body = body_without_stamps(&sec.body);

    let mut doc = String::new();
    doc.push_str("---\n");
    doc.push_str(&format!("type: {doc_type}\n"));
    if doc_type == "decision" {
        doc.push_str("status: active\n");
    }
    doc.push_str("---\n\n");
    doc.push_str(&format!("# {title}\n\n"));
    doc.push_str(&body);
    doc.push_str(&format!("\n\n관련:: [[{daily}]]\n"));
    vault::write_doc(vault, project, target, &doc)?;

    // daily: 헤딩은 그대로, 본문은 스탬프 + 링크 한 줄.
    let mut replacement = String::new();
    if let Some(s) = stamp {
        replacement.push_str(&s);
        replacement.push('\n');
    }
    replacement.push_str(&format!("→ [[{target}]] 로 옮겼어요.\n"));
    let next = replace_section_body(&md, heading, &replacement);
    vault::write_doc(vault, project, daily, &next)?;
    Ok(())
}

/// `heading` 줄 다음부터 다음 같은/얕은 헤딩 전까지를 `new_body` 로 바꾼다.
fn replace_section_body(md: &str, heading: &str, new_body: &str) -> String {
    let level = heading_level(heading);
    let mut out: Vec<&str> = Vec::new();
    let mut lines = md.lines().peekable();
    let mut in_fence = false;
    let mut replaced = false;
    while let Some(line) = lines.next() {
        out.push(line);
        if replaced || line != heading {
            continue;
        }
        replaced = true;
        out.push(new_body.trim_end_matches('\n'));
        // 옛 본문을 건너뛴다.
        while let Some(next) = lines.peek() {
            let t = next.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                in_fence = !in_fence;
            }
            let lv = if in_fence { 0 } else { heading_level(next) };
            if lv >= 1 && lv <= level {
                break;
            }
            lines.next();
        }
        out.push("");
    }
    let mut s = out.join("\n");
    if md.ends_with('\n') && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// 검토함의 pending `dream_*` 건수 (프로젝트 범위 선택).
/// 에이전트(`danbi_tidy`)가 한 번에 받아 보는 최대 건수 — 응답을 작게.
pub const AGENT_TIDY_LIMIT: usize = 10;

/// 에이전트가 처리할 이 프로젝트의 정리 거리. `scan` 이면 먼저 이 프로젝트만
/// 다시 훑어 검토함에 올린다. 항목마다 고를 수 있는 선택지를 붙여서
/// 에이전트가 payload 구조를 몰라도 `danbi_tidy_apply` 를 부를 수 있게.
pub fn agent_tidy_list(vault: &Path, project: &str, scan: bool) -> DanbiResult<serde_json::Value> {
    use serde_json::json;
    let mut scanned = None;
    if scan {
        let opts = RunOptions {
            max_per_run: AGENT_TIDY_LIMIT,
            force: true,
            auto_fix_links: false,
            only_project: Some(project.to_string()),
        };
        scanned = Some(run_with(vault, &opts, None)?.message);
    }
    let store = reviews::load(vault)?;
    let pending: Vec<&reviews::ReviewItem> = store
        .items
        .iter()
        .filter(|it| it.status == reviews::ReviewStatus::Pending && it.kind.starts_with(KIND_PREFIX))
        .filter(|it| it.project.as_deref() == Some(project))
        .collect();
    let mut items = Vec::new();
    for it in pending.iter().take(AGENT_TIDY_LIMIT) {
        let Ok((_, action)) = parse_action(it) else { continue };
        let (what, choices) = match &action {
            DreamAction::FixLink { broken, sources, options, ambiguous } => (
                json!({ "broken": broken, "in": sources, "ambiguous": ambiguous,
                        "candidates": options.iter().map(|o| json!({ "target": o.target, "why": o.why })).collect::<Vec<_>>() }),
                json!(["relink(target) — 후보 중 맞는 문서, 또는 직접 찾은 문서", "remove — 링크를 풀고 글자만 남김"]),
            ),
            DreamAction::FixLinkBatch { items } => (
                json!({ "fixes": items.iter().map(|i| json!({ "broken": i.broken, "target": i.target })).collect::<Vec<_>>() }),
                json!(["apply_all — 목록의 링크를 모두 바꿈"]),
            ),
            DreamAction::Stale { domain, stale_after } => (
                json!({ "doc": domain, "stale_after": stale_after }),
                json!(["verify(evidence) — 내용이 지금도 맞다는 걸 코드·문서로 확인했을 때만", "supersede(evidence) — 더 이상 맞지 않음"]),
            ),
            DreamAction::CitesSuperseded { source, target, replacement } => (
                json!({ "doc": source, "cites": target, "replacement": replacement }),
                json!(["relink(target) — 대체한 문서로", "remove"]),
            ),
            DreamAction::Promote { daily, heading, title, target, .. } => (
                json!({ "daily": daily, "heading": heading, "title": title, "target": target }),
                json!(["promote(title?, target?) — 주제 문서로 옮기고 daily 엔 링크만 남김. 제목·경로를 더 낫게 바꿔도 됨"]),
            ),
        };
        items.push(json!({ "id": it.id, "kind": it.kind, "reason": it.reason, "item": what, "choices": choices }));
    }
    Ok(json!({
        "project": project,
        "pending": pending.len(),
        "items": items,
        "scanned": scanned,
        "note": "확실한 것만 danbi_tidy_apply(id, choice, …) 로 처리하고, 애매한 항목은 손대지 말고 두세요 — 사람이 단비 검토함에서 고릅니다.",
    }))
}

pub fn pending_count(vault: &Path, project: Option<&str>) -> usize {
    let Ok(store) = reviews::load(vault) else { return 0 };
    store
        .items
        .iter()
        .filter(|it| it.status == reviews::ReviewStatus::Pending && it.kind.starts_with(KIND_PREFIX))
        .filter(|it| project.map(|p| it.project.as_deref() == Some(p)).unwrap_or(true))
        .count()
}

#[derive(Debug, Clone, Serialize)]
pub struct DreamStatus {
    pub last_run_at: Option<i64>,
    pub last_message: Option<String>,
    pub pending: usize,
}

pub fn status(vault: &Path) -> DreamStatus {
    let store = load_store(vault).unwrap_or_default();
    DreamStatus { last_run_at: store.last_run_at, last_message: store.last_message, pending: pending_count(vault, None) }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(label: &str) -> PathBuf {
        let unique = format!(
            "danbi-dream-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
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

    fn read(vault: &Path, rel: &str) -> String {
        std::fs::read_to_string(vault.join(PROJECTS_DIRNAME).join(rel)).unwrap()
    }

    fn docs_map(vault: &Path) -> HashMap<String, Vec<String>> {
        vault::project_names(vault)
            .unwrap()
            .into_iter()
            .map(|p| {
                let d = list_docs(vault, &p);
                (p, d)
            })
            .collect()
    }

    fn scan(vault: &Path, project: &str, today: &str) -> Vec<DreamCandidate> {
        let index = links::build_index(vault).unwrap();
        scan_project(vault, project, &index, &docs_map(vault), today)
    }

    fn fix_links(c: &[DreamCandidate]) -> Vec<&DreamCandidate> {
        c.iter().filter(|c| matches!(c.action, DreamAction::FixLink { .. })).collect()
    }

    fn options(c: &DreamCandidate) -> Vec<String> {
        match &c.action {
            DreamAction::FixLink { options, .. } => options.iter().map(|o| o.target.clone()).collect(),
            _ => panic!("not a fix-link"),
        }
    }

    fn pending(vault: &Path) -> Vec<reviews::ReviewItem> {
        reviews::load(vault)
            .unwrap()
            .items
            .into_iter()
            .filter(|i| i.status == reviews::ReviewStatus::Pending)
            .collect()
    }

    const TODAY: &str = "2026-11-07";

    // ---- name matching ----

    #[test]
    fn norm_name_folds_case_separators_and_unicode_form() {
        assert_eq!(norm_name("Auth_Flow.md"), "authflow");
        assert_eq!(norm_name("auth-flow"), "authflow");
        // NFD (macOS 파일명) 과 NFC (링크에 적은 한글) 가 같아진다.
        let nfd: String = "결정.md".nfd().collect();
        assert_eq!(norm_name(&nfd), norm_name("결정.md"));
        assert_eq!(edit_distance("auth", "auht"), 1);
        assert_eq!(edit_distance("auth", "aut"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("한글", "한글"), 0);
        assert_eq!(edit_distance("", "ab"), 2);
    }

    #[test]
    fn link_options_rank_rename_folder_move_case_and_typo() {
        let mut docs = HashMap::new();
        docs.insert("A".to_string(), vec!["notes/auth.md".into(), "Auth Flow.md".into(), "notes/authz.md".into(), "plan.md".into()]);
        docs.insert("B".to_string(), vec!["auth.md".into()]);
        let mut renames = HashMap::new();
        renames.insert("Projects/A/auth.md".to_string(), "Projects/A/notes/auth.md".to_string());

        // 폴더 이동 + git 이름 변경 → 같은 대상이 한 번, 가장 높은 확신으로.
        let opts = link_options("A", "auth.md", &docs, &renames);
        assert_eq!(opts[0].target, "notes/auth.md");
        assert!((opts[0].confidence - 0.95).abs() < 1e-6);
        assert_eq!(opts.iter().filter(|o| o.target == "notes/auth.md").count(), 1);

        // 대소문자·띄어쓰기만 다른 것.
        let opts = link_options("A", "auth_flow.md", &docs, &HashMap::new());
        assert_eq!(opts[0].target, "Auth Flow.md");
        assert!((opts[0].confidence - 0.9).abs() < 1e-6);

        // 오타 (편집 거리 1) 와 다른 프로젝트의 같은 이름.
        let opts = link_options("A", "notes/auht.md", &docs, &HashMap::new());
        assert!(opts.iter().any(|o| o.target == "notes/auth.md" && o.why == "이름이 비슷해요"));
        let opts = link_options("B", "notes/authz.md", &docs, &HashMap::new());
        assert!(opts.iter().any(|o| o.target == "A/notes/authz.md"));

        // 아무것도 닮지 않은 이름은 후보 없음.
        assert!(link_options("A", "zzz-unrelated-thing.md", &docs, &HashMap::new()).is_empty());
    }

    #[test]
    fn korean_nfd_file_in_another_folder_is_resolved_not_broken() {
        let v = temp_vault("ko");
        // 파일은 NFD (macOS 스타일), 링크는 NFC — 해석기가 잇는다.
        let nfd_name: String = "인증-결정.md".nfd().collect();
        write(&v, &format!("A/decisions/{nfd_name}"), "# 인증 결정\n본문");
        write(&v, "A/index.md", "참고: [[인증-결정.md]]\n");
        let found = scan(&v, "A", TODAY);
        assert!(fix_links(&found).is_empty(), "{found:?}");
        std::fs::remove_dir_all(v).ok();
    }

    /// 두 파일을 git 이 이름 변경으로 기억하도록 옮긴다 (내용 동일, 새 이름은
    /// 옛 이름과 달라 해석기로는 못 찾는 것).
    fn renamed_pair(v: &Path) {
        write(v, "A/cron.md", "# cron\n파이프라인 설명이 길게 들어있는 문서\n여러 줄\n");
        write(v, "A/auth.md", "# auth\n인증 흐름 설명이 길게 들어있는 문서\n여러 줄\n");
        vcs::ensure_repo(v).unwrap();
        vcs::snapshot(v, "init").unwrap();
        std::fs::create_dir_all(v.join("Projects/A/notes")).unwrap();
        std::fs::rename(v.join("Projects/A/cron.md"), v.join("Projects/A/notes/cron-pipeline.md")).unwrap();
        std::fs::rename(v.join("Projects/A/auth.md"), v.join("Projects/A/notes/auth-flow.md")).unwrap();
        vcs::snapshot(v, "move").unwrap();
    }

    #[test]
    fn scan_skips_assets_existing_docs_and_groups_sources() {
        let v = temp_vault("broken");
        write(&v, "A/a.md", "[[missing.md]] and [[b.md]] and [[img.png]] and [[B/x.md]]");
        write(&v, "A/notes/c.md", "[[missing]] again");
        write(&v, "A/b.md", "b");
        write(&v, "B/x.md", "x");
        let found = scan(&v, "A", TODAY);
        let links = fix_links(&found);
        assert_eq!(links.len(), 1, "{found:?}");
        match &links[0].action {
            DreamAction::FixLink { broken, sources, options, .. } => {
                assert_eq!(broken, "missing.md");
                assert_eq!(sources, &vec!["a.md".to_string(), "notes/c.md".to_string()]);
                assert!(options.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert!(links[0].reason.contains("a.md 외 1곳"));
        assert_eq!(links[0].key, "fix_link:A:missing.md");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn cross_project_broken_link_offers_prefixed_targets() {
        let v = temp_vault("cross");
        write(&v, "A/index.md", "[[B/auth.md]] [[B/unique.md]]\n");
        write(&v, "B/notes/auth.md", "x");
        write(&v, "B/decisions/auth.md", "x");
        write(&v, "B/notes/unique.md", "x");
        // 유일한 이름은 다른 프로젝트 안에서도 해석된다.
        let idx = links::build_index(&v).unwrap();
        assert!(idx.outgoing["A/index.md"].iter().any(|l| l.project == "B" && l.domain == "notes/unique.md"));
        let found = scan(&v, "A", TODAY);
        let links = fix_links(&found);
        assert_eq!(links.len(), 1, "{found:?}");
        match &links[0].action {
            DreamAction::FixLink { broken, options, ambiguous, .. } => {
                assert!(ambiguous);
                assert_eq!(broken, "B/auth.md");
                assert_eq!(options[0].target, "B/decisions/auth.md");
                assert_eq!(options[1].target, "B/notes/auth.md");
            }
            _ => unreachable!(),
        }
        // 적용하면 A 쪽 링크가 B 의 새 경로를 가리킨다.
        run(&v, 8, false, None).unwrap();
        let item = pending(&v).remove(0);
        apply(&v, &item.id, Choice::Relink { target: "B/notes/auth.md".into() }).unwrap();
        assert_eq!(read(&v, "A/index.md"), "[[B/notes/auth.md]] [[B/unique.md]]\n");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn project_fingerprint_changes_only_with_its_own_files() {
        let v = temp_vault("fp");
        write(&v, "A/a.md", "a");
        write(&v, "B/b.md", "b");
        let a1 = project_fingerprint(&v, "A");
        let b1 = project_fingerprint(&v, "B");
        write(&v, "B/b.md", "b 바뀜");
        assert_eq!(project_fingerprint(&v, "A"), a1);
        assert_ne!(project_fingerprint(&v, "B"), b1);
        write(&v, "A/.danbi/hidden.md", "무시");
        assert_eq!(project_fingerprint(&v, "A"), a1);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn relative_links_are_resolved_by_the_index_and_only_missing_ones_remain() {
        let v = temp_vault("relative");
        write(&v, "A/INDEX.md", "# index");
        write(&v, "A/ep01/a.md", "[[../INDEX.md]] [[../INDEX]]");
        write(&v, "A/ep02/c.md", "[[../nope.md]]");
        let found = scan(&v, "A", TODAY);
        let links = fix_links(&found);
        assert_eq!(links.len(), 1, "{found:?}");
        assert_eq!(links[0].key, "fix_link:A:../nope.md@ep02");
        assert!(options(links[0]).is_empty());
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn ambiguous_same_name_in_two_folders_asks_which_one() {
        let v = temp_vault("ambiguous");
        write(&v, "A/notes/auth.md", "n");
        write(&v, "A/decisions/auth.md", "d");
        write(&v, "A/a.md", "[[auth.md]] 와 [[auth|인증]]");
        write(&v, "A/b.md", "[[auth.md]]");
        write(&v, "B/x.md", "[[A/auth.md]]");
        let found = scan(&v, "A", TODAY);
        let links = fix_links(&found);
        assert_eq!(links.len(), 1, "{found:?}");
        match &links[0].action {
            DreamAction::FixLink { broken, sources, options, ambiguous } => {
                assert!(ambiguous);
                assert_eq!(broken, "auth.md");
                assert_eq!(sources, &vec!["a.md".to_string(), "b.md".to_string()]);
                assert_eq!(options.iter().map(|o| o.target.clone()).collect::<Vec<_>>(), vec!["decisions/auth.md", "notes/auth.md"]);
            }
            _ => unreachable!(),
        }
        assert!(links[0].reason.contains("같은 이름의 문서가 2개"));
        // B 쪽은 `A/auth.md` 로 적었으니 A 의 후보에 `A/` 를 붙여서.
        let found_b = scan(&v, "B", TODAY);
        let lb = fix_links(&found_b);
        assert_eq!(lb.len(), 1);
        assert_eq!(options(lb[0]), vec!["A/decisions/auth.md", "A/notes/auth.md"]);
        // 모호한 건 묶이지 않고, 고르면 그 문서로.
        let r = run(&v, 8, false, None).unwrap();
        assert_eq!(r.found_batches, 0);
        let item = pending(&v).into_iter().find(|i| i.project.as_deref() == Some("A")).unwrap();
        apply(&v, &item.id, Choice::Relink { target: "notes/auth.md".into() }).unwrap();
        assert_eq!(read(&v, "A/a.md"), "[[notes/auth.md]] 와 [[notes/auth|인증]]");
        assert_eq!(read(&v, "A/b.md"), "[[notes/auth.md]]");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn certain_fixes_bundle_into_one_item_and_ambiguous_stay_separate() {
        let v = temp_vault("bundle");
        renamed_pair(&v); // cron.md → notes/cron-pipeline.md, auth.md → notes/auth-flow.md (git 기억)
        write(&v, "A/notes/plan.md", "x");
        write(&v, "A/decisions/plan.md", "x"); // plan.md 는 두 폴더에 있어 애매
        write(&v, "A/a.md", "[[cron.md]] [[auth.md]] [[plan.md]] [[typo-crom.md]] [[zzz.md]]");
        write(&v, "A/b.md", "[[cron.md]]");

        let raw = scan(&v, "A", TODAY);
        assert_eq!(fix_links(&raw).len(), 5, "{raw:?}");
        let bundled = bundle_fix_links("A", &docs_map(&v), raw);
        let batch = bundled.iter().find(|c| matches!(c.action, DreamAction::FixLinkBatch { .. })).unwrap();
        match &batch.action {
            DreamAction::FixLinkBatch { items } => {
                let pairs: Vec<(String, String)> = items.iter().map(|i| (i.broken.clone(), i.target.clone())).collect();
                assert_eq!(
                    pairs,
                    vec![("auth.md".into(), "notes/auth-flow.md".into()), ("cron.md".into(), "notes/cron-pipeline.md".into())]
                );
                assert_eq!(items[1].sources, vec!["a.md".to_string(), "b.md".to_string()]);
                assert_eq!(items[0].why, "git 기록에서 이름이 바뀐 파일이에요");
            }
            _ => unreachable!(),
        }
        assert!(batch.reason.contains("2종(문서 2곳)"), "{}", batch.reason);
        let singles: Vec<&str> = bundled
            .iter()
            .filter_map(|c| match &c.action {
                DreamAction::FixLink { broken, .. } => Some(broken.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(singles, vec!["plan.md", "typo-crom.md", "zzz.md"]);

        // run → 묶음 1 + 개별 3. 묶음 승인 → 두 줄 모두 고쳐지고 커밋 1개.
        let r = run(&v, 8, false, None).unwrap();
        assert_eq!((r.found, r.enqueued), (4, 4));
        assert_eq!((r.found_links, r.found_batches, r.found_batch_kinds), (3, 1, 2));
        assert_eq!(r.message, "정리 거리 4건을 모두 검토함에 올렸어요 (링크 3 · 묶음 1 (2종)).");
        let items = pending(&v);
        let b = items.iter().find(|i| i.kind == KIND_FIX_LINK_BATCH).unwrap();
        let commits_before = vcs::recent_commits(&v, 50).unwrap().len();
        let res = apply(&v, &b.id, Choice::ApplyAll).unwrap();
        assert_eq!(res.changed, vec!["a.md".to_string(), "b.md".to_string()]);
        assert!(res.message.contains("링크 2종"));
        assert_eq!(read(&v, "A/a.md"), "[[notes/cron-pipeline.md]] [[notes/auth-flow.md]] [[plan.md]] [[typo-crom.md]] [[zzz.md]]");
        assert_eq!(read(&v, "A/b.md"), "[[notes/cron-pipeline.md]]");
        let commits = vcs::recent_commits(&v, 50).unwrap();
        assert_eq!(commits.len(), commits_before + 2); // (pre) + 본 커밋
        assert!(commits[0].summary.contains("dreaming fix-links"));
        let decided = load_store(&v).unwrap().decided;
        assert_eq!(decided["fix_link:A:cron.md"].status, "applied");
        assert_eq!(decided["fix_link:A:auth.md"].status, "applied");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn auto_fix_links_applies_certain_fixes_without_a_review_item() {
        let v = temp_vault("autofix");
        renamed_pair(&v); // cron.md → notes/cron-pipeline.md, auth.md → notes/auth-flow.md
        write(&v, "A/notes/plan.md", "x");
        write(&v, "A/decisions/plan.md", "x");
        write(&v, "A/a.md", "[[cron.md]] [[auth.md]] [[plan.md]]");
        let opts = RunOptions { max_per_run: 8, auto_fix_links: true, ..Default::default() };
        let r = run_with(&v, &opts, None).unwrap();
        assert_eq!(r.auto_fixed.len(), 1, "{r:?}");
        assert_eq!(read(&v, "A/a.md"), "[[notes/cron-pipeline.md]] [[notes/auth-flow.md]] [[plan.md]]");
        // 확실한 것은 검토함에 없고, 애매한 plan.md 만 올라간다.
        let items = pending(&v);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].kind, KIND_FIX_LINK);
        assert!(r.message.starts_with("확실한 링크는 바로 고쳤어요"), "{}", r.message);
        // 단비 이름으로 커밋.
        let commits = vcs::recent_commits(&v, 10).unwrap();
        assert!(commits.iter().any(|c| c.summary.contains("dreaming fix-links") && c.summary.contains("[danbi]")), "{:?}", commits.iter().map(|c| &c.summary).collect::<Vec<_>>());
        // 다시 돌려도 같은 걸 또 고치지 않는다.
        let r2 = run_with(&v, &RunOptions { force: true, ..opts.clone() }, None).unwrap();
        assert!(r2.auto_fixed.is_empty(), "{r2:?}");
        // 끄면(기본) 예전처럼 검토함으로.
        let v2 = temp_vault("autofix-off");
        renamed_pair(&v2);
        write(&v2, "A/a.md", "[[cron.md]]");
        let r = run(&v2, 8, false, None).unwrap();
        assert!(r.auto_fixed.is_empty());
        assert_eq!(read(&v2, "A/a.md"), "[[cron.md]]");
        assert_eq!(pending(&v2).len(), 1);
        std::fs::remove_dir_all(v).ok();
        std::fs::remove_dir_all(v2).ok();
    }

    #[test]
    fn agent_cannot_keep_or_verify_without_evidence_and_is_credited() {
        let v = temp_vault("agent");
        write(&v, "A/notes/old.md", "---\nstale_after: 2026-01-01\n---\n옛날");
        write(&v, "A/a.md", "[[zzz.md]]");
        run(&v, 8, false, None).unwrap();
        let items = pending(&v);
        let stale = items.iter().find(|i| i.kind == KIND_RECHECK).unwrap();
        let link = items.iter().find(|i| i.kind == KIND_FIX_LINK).unwrap();
        let agent = |note: Option<&str>| Actor::Agent { client: "claude-code".into(), note: note.map(String::from) };

        let e = apply_as(&v, &link.id, Choice::Keep, &agent(None)).unwrap_err().to_string();
        assert!(e.contains("그대로 두기"), "{e}");
        let e = apply_as(&v, &stale.id, Choice::Verify, &agent(None)).unwrap_err().to_string();
        assert!(e.contains("근거"), "{e}");
        let e = apply_as(&v, &stale.id, Choice::Verify, &agent(Some("   "))).unwrap_err().to_string();
        assert!(e.contains("근거"), "{e}");
        // 거절된 항목은 그대로 대기 중.
        assert_eq!(pending(&v).len(), 2);

        let r = apply_as(&v, &stale.id, Choice::Verify, &agent(Some("src/auth.rs 의 만료 로직이 문서와 같음"))).unwrap();
        assert!(r.commit.is_some());
        let doc = read(&v, "A/notes/old.md");
        assert!(doc.contains("by: agent"), "{doc}");
        let c = &vcs::recent_commits(&v, 5).unwrap()[0];
        assert!(c.summary.contains("src/auth.rs 의 만료 로직") && c.summary.contains("[claude-code]"), "{}", c.summary);
        assert_eq!(pending(&v).len(), 1);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn only_project_scan_is_not_blocked_by_other_projects_queue() {
        let v = temp_vault("only");
        for i in 0..25 {
            write(&v, &format!("B/n{i}.md"), &format!("[[missing-{i}.md]]"));
        }
        write(&v, "A/a.md", "정상");
        // 전체 실행으로 B 가 검토함 상한(20)을 채운다.
        run(&v, 50, false, None).unwrap();
        assert_eq!(pending(&v).len(), MAX_PENDING);
        // 그 뒤 A 에 깨진 링크가 생긴다 — 전체 실행으로는 상한 때문에 못 올라간다.
        write(&v, "A/a.md", "[[zzz.md]]");
        assert_eq!(run(&v, 50, false, None).unwrap().enqueued, 0);
        // 에이전트가 A 만 훑으면 A 의 항목은 올라온다.
        let out = agent_tidy_list(&v, "A", true).unwrap();
        assert_eq!(out["pending"], 1, "{out}");
        assert_eq!(out["items"][0]["kind"], KIND_FIX_LINK);
        assert!(out["items"][0]["choices"].as_array().unwrap().len() >= 2);
        assert!(out["items"].as_array().unwrap().iter().all(|i| i["id"].is_string()));
        // B 의 항목은 A 의 목록에 섞이지 않는다.
        let b = agent_tidy_list(&v, "B", false).unwrap();
        assert_eq!(b["items"].as_array().unwrap().len(), AGENT_TIDY_LIMIT);
        assert_eq!(b["pending"], MAX_PENDING);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn apply_many_uses_one_commit_and_skips_stale_items() {
        let v = temp_vault("bulk");
        renamed_pair(&v); // cron.md → notes/cron-pipeline.md, auth.md → notes/auth-flow.md
        write(&v, "A/a.md", "[[cron.md]]");
        write(&v, "B/notes/demo-script.md", "# 데모");
        write(&v, "B/b.md", "[[notes/demo-scrpt.md]]");
        run(&v, 8, false, None).unwrap();
        let items = pending(&v);
        assert_eq!(items.len(), 2, "{items:?}");
        let commits_before = vcs::recent_commits(&v, 50).unwrap().len();
        // 하나는 미리 손으로 처리해 둔다 → 일괄에서 건너뛰어야 함.
        let a = items.iter().find(|i| i.project.as_deref() == Some("A")).unwrap();
        let b = items.iter().find(|i| i.project.as_deref() == Some("B")).unwrap();
        let target_of = |it: &reviews::ReviewItem| match parse_action(it).unwrap().1 {
            DreamAction::FixLink { options, .. } => options[0].target.clone(),
            _ => unreachable!(),
        };
        let (ta, tb) = (target_of(a), target_of(b));
        apply(&v, &a.id, Choice::Relink { target: ta.clone() }).unwrap();
        let commits_mid = vcs::recent_commits(&v, 50).unwrap().len();
        assert_eq!(commits_mid, commits_before + 2);
        let res = apply_many(
            &v,
            vec![
                (a.id.clone(), Choice::Relink { target: ta }),
                (b.id.clone(), Choice::Relink { target: tb }),
                ("없는-id".into(), Choice::Remove),
                (b.id.clone(), Choice::Keep),
            ],
        )
        .unwrap();
        assert_eq!(res.ok, 1, "{res:?}");
        assert_eq!(res.failed.len(), 3, "{res:?}");
        assert_eq!(res.changed, vec!["B/b.md".to_string()]);
        assert_eq!(read(&v, "B/b.md"), "[[notes/demo-script.md]]");
        // (pre) 1 + 본 커밋 1.
        let commits = vcs::recent_commits(&v, 50).unwrap();
        assert_eq!(commits.len(), commits_mid + 2);
        assert!(commits[0].summary.contains("dreaming 일괄 1건 · B"), "{}", commits[0].summary);
        assert!(pending(&v).is_empty());
        // 아무것도 적용 못 하면 커밋도 없다.
        let res = apply_many(&v, vec![("없는-id".into(), Choice::Remove)]).unwrap();
        assert_eq!((res.ok, res.commit.is_none()), (0, true));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn keeping_a_bundle_remembers_every_item_in_it() {
        let v = temp_vault("bundle-keep");
        renamed_pair(&v);
        write(&v, "A/a.md", "[[cron.md]] [[auth.md]]");
        run(&v, 8, false, None).unwrap();
        let b = pending(&v).into_iter().find(|i| i.kind == KIND_FIX_LINK_BATCH).unwrap();
        apply(&v, &b.id, Choice::Keep).unwrap();
        let decided = load_store(&v).unwrap().decided;
        assert_eq!(decided["fix_link:A:cron.md"].status, "dismissed");
        assert_eq!(decided["fix_link:A:auth.md"].status, "dismissed");
        // 새 링크가 하나 생겨도 보류한 둘은 안 끼고, 혼자라 묶음도 안 된다.
        write(&v, "A/a.md", "[[cron.md]] [[auth.md]] [[zzz.md]]");
        let r = run(&v, 8, false, None).unwrap();
        assert_eq!(r.enqueued, 1);
        let items = pending(&v);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, KIND_FIX_LINK);
        // 검토함에서 무시해도 같은 기억.
        reviews::resolve(&v, &items[0].id, reviews::ReviewStatus::Dismissed).unwrap();
        on_review_resolved(&v, &items[0], &reviews::ReviewStatus::Dismissed);
        assert_eq!(load_store(&v).unwrap().decided["fix_link:A:zzz.md"].status, "dismissed");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn pending_bundle_items_are_not_reproposed_individually() {
        let v = temp_vault("bundle-pending");
        renamed_pair(&v);
        write(&v, "A/a.md", "[[cron.md]] [[auth.md]]");
        run(&v, 8, false, None).unwrap();
        assert_eq!(pending(&v).len(), 1);
        // 파일이 바뀌어 지문이 달라져도, 묶음이 대기 중인 동안은 안 겹친다.
        write(&v, "A/a.md", "[[cron.md]] [[auth.md]] 수정");
        let r = run(&v, 8, false, None).unwrap();
        assert_eq!((r.scanned_projects, r.enqueued), (1, 0));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn round_robin_mixes_kinds_so_links_do_not_starve_the_rest() {
        let mk = |key: &str, action: DreamAction, priority: u8| DreamCandidate {
            key: key.into(),
            project: "A".into(),
            domain: None,
            reason: String::new(),
            action,
            priority,
        };
        let link = |k: &str, p: u8| mk(k, DreamAction::FixLink { broken: k.into(), sources: vec![], options: vec![], ambiguous: false }, p);
        let stale = |k: &str| mk(k, DreamAction::Stale { domain: k.into(), stale_after: "2026-01-01".into() }, 2);
        let promote = |k: &str| {
            mk(k, DreamAction::Promote { daily: "daily/x.md".into(), heading: "### 결정".into(), title: k.into(), target: "decisions/x.md".into(), doc_type: "decision".into() }, 4)
        };
        let cands = vec![link("l3", 3), link("l1", 0), link("l2", 1), stale("s1"), stale("s2"), promote("p1"), link("l4", 5)];
        let picked: Vec<String> = pick_round_robin(cands, 5).into_iter().map(|c| c.key).collect();
        assert_eq!(picked, vec!["l1", "s1", "p1", "l2", "s2"]);
        // 같은 종류 안에서는 프로젝트를 번갈아: A 의 승격 3개 + B 의 승격 1개.
        let in_proj = |k: &str, p: &str| DreamCandidate { project: p.into(), ..promote(k) };
        let mixed = vec![in_proj("a1", "A"), in_proj("a2", "A"), in_proj("a3", "A"), in_proj("b1", "B")];
        let picked: Vec<String> = pick_round_robin(mixed, 3).into_iter().map(|c| c.key).collect();
        assert_eq!(picked, vec!["a1", "b1", "a2"]);
        // 한 종류뿐이면 그 종류로 채운다.
        let only: Vec<String> = pick_round_robin(vec![link("b", 1), link("a", 0)], 5).into_iter().map(|c| c.key).collect();
        assert_eq!(only, vec!["a", "b"]);

        // 실제 run 에서도: 링크 10 + stale 1 + 승격 1, 예산 4 → 세 종류 다 올라온다.
        let v = temp_vault("rr");
        let mut body = String::new();
        for i in 0..10 {
            body.push_str(&format!("[[missing-{i}.md]]\n"));
        }
        write(&v, "A/a.md", &body);
        write(&v, "A/notes/old.md", "---\nstale_after: 2026-01-01\n---\n옛날");
        let long = "- 모바일 세션 유지와 보안의 트레이드오프를 따져 7일로 정했다.\n- 만료 처리는 refresh 흐름에서 다룬다.";
        write(&v, "A/daily/2026-11-01.md", &format!("### 결정: JWT refresh 7일\n{long}\n"));
        let r = run(&v, 4, false, None).unwrap();
        assert_eq!(r.enqueued, 4);
        let kinds: BTreeSet<String> = pending(&v).into_iter().map(|i| i.kind).collect();
        assert_eq!(kinds.into_iter().collect::<Vec<_>>(), vec![KIND_FIX_LINK, KIND_PROMOTE, KIND_RECHECK]);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn git_rename_history_resolves_a_moved_file() {
        let v = temp_vault("git");
        write(&v, "A/auth.md", "# auth\n인증 흐름 설명이 길게 들어있는 문서\n여러 줄\n");
        write(&v, "A/index.md", "[[auth.md]]\n");
        vcs::ensure_repo(&v).unwrap();
        vcs::snapshot(&v, "init").unwrap();
        std::fs::create_dir_all(v.join("Projects/A/notes")).unwrap();
        std::fs::rename(v.join("Projects/A/auth.md"), v.join("Projects/A/notes/auth-flow.md")).unwrap();
        vcs::snapshot(&v, "move").unwrap();

        let wanted: HashSet<String> = ["Projects/A/auth.md".to_string()].into_iter().collect();
        let renames = git_renames(&v, &wanted);
        assert_eq!(renames.get("Projects/A/auth.md").map(String::as_str), Some("Projects/A/notes/auth-flow.md"));

        let found = scan(&v, "A", TODAY);
        let links = fix_links(&found);
        assert_eq!(links.len(), 1);
        let opts = match &links[0].action {
            DreamAction::FixLink { options, .. } => options.clone(),
            _ => unreachable!(),
        };
        assert_eq!(opts[0].target, "notes/auth-flow.md");
        assert_eq!(opts[0].why, "git 기록에서 이름이 바뀐 파일이에요");
        assert_eq!(links[0].priority, 0);
        std::fs::remove_dir_all(v).ok();
    }

    // ---- recheck ----

    #[test]
    fn stale_docs_are_flagged_but_retired_and_daily_are_not() {
        let v = temp_vault("stale");
        write(&v, "A/notes/old.md", "---\nstale_after: 2026-01-01\n---\n옛날");
        write(&v, "A/notes/fresh.md", "---\nstale_after: 2027-01-01\n---\n새것");
        write(&v, "A/notes/gone.md", "---\nstale_after: 2026-01-01\nstatus: superseded\n---\n");
        write(&v, "A/daily/2026-01-01.md", "---\nstale_after: 2026-01-01\n---\n# 일지");
        let found = scan(&v, "A", TODAY);
        let stale: Vec<_> = found.iter().filter(|c| matches!(c.action, DreamAction::Stale { .. })).collect();
        assert_eq!(stale.len(), 1, "{found:?}");
        assert_eq!(stale[0].domain.as_deref(), Some("notes/old.md"));
        assert_eq!(stale[0].key, "stale:A/notes/old.md:2026-01-01");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn citing_a_superseded_decision_is_flagged_with_its_replacement() {
        let v = temp_vault("cites");
        write(&v, "A/decisions/new.md", "---\nstatus: active\n---\n대체:: [[decisions/old.md]]\n");
        write(&v, "A/decisions/old.md", "---\nstatus: active\n---\n옛 결정");
        write(&v, "A/plan.md", "근거: [[decisions/old.md]]\n");
        write(&v, "A/daily/2026-10-01.md", "[[decisions/old.md]]\n");
        let found = scan(&v, "A", TODAY);
        let cites: Vec<_> = found.iter().filter(|c| matches!(c.action, DreamAction::CitesSuperseded { .. })).collect();
        assert_eq!(cites.len(), 1, "{found:?}");
        match &cites[0].action {
            DreamAction::CitesSuperseded { source, target, replacement } => {
                assert_eq!(source, "plan.md");
                assert_eq!(target, "decisions/old.md");
                assert_eq!(replacement.as_deref(), Some("decisions/new.md"));
            }
            _ => unreachable!(),
        }
        std::fs::remove_dir_all(v).ok();
    }

    // ---- promote ----

    #[test]
    fn sections_split_and_classify() {
        let md = "# 2026-11-01\n\n### 결정: JWT refresh 7일\n_14:02 · claude-code_\n- 이유\n\n### TODO\n- x\n\n```\n### not a heading\n```\n\n## 원인 — CORS\n본문\n";
        let secs = split_sections(md);
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[0].title, "결정: JWT refresh 7일");
        assert_eq!(secs[0].level, 3);
        assert!(secs[0].body.contains("- 이유"));
        assert_eq!(secs[2].heading, "## 원인 — CORS");
        assert_eq!(classify_section("결정: JWT", ""), Some(("decisions", "decision")));
        assert_eq!(classify_section("TODO", ""), None);
        assert_eq!(classify_section("CORS 에러", ""), Some(("bugs", "bug")));
        assert_eq!(classify_section("아무거나", "원인:: [[x.md]]"), Some(("bugs", "bug")));
        assert_eq!(classify_section("노하우", ""), Some(("notes", "note")));
        assert_eq!(classify_section("회고", "잡담"), None);
        assert_eq!(strip_title_prefix("결정: JWT refresh 7일"), "JWT refresh 7일");
        assert_eq!(strip_title_prefix("[결정] 캐시 끔"), "캐시 끔");
        assert_eq!(strip_title_prefix("원인 — CORS preflight"), "CORS preflight");
        assert_eq!(strip_title_prefix("OG 이미지: gpt-image-2 하이브리드 방식 확정"), "OG 이미지: gpt-image-2 하이브리드 방식 확정");
        assert_eq!(strip_title_prefix("[메모] 그냥 제목"), "[메모] 그냥 제목");
        assert_eq!(strip_title_prefix("재발 방지: 락 순서"), "락 순서");
        // 기호·일반적인 제목 정리.
        assert_eq!(promote_title("/promotion 페이지 구조 바꿈", "").as_deref(), Some("promotion 페이지 구조 바꿈"));
        assert_eq!(promote_title("결정: — 캐시 끔:", "").as_deref(), Some("캐시 끔"));
        assert_eq!(
            promote_title("결정사항", "_14:02 · x_\n- **JWT refresh 7일로 확정**. 이유: 모바일 세션\n- 둘째").as_deref(),
            Some("JWT refresh 7일로 확정")
        );
        assert_eq!(promote_title("### 메모", "원인:: [[x.md]]\n1. 배포 파이프라인이 두 번 돌던 문제 해결").as_deref(), Some("배포 파이프라인이 두 번 돌던 문제 해결"));
        assert_eq!(promote_title("결정", "- 메모\n"), None);
        assert_eq!(promote_title("2. 로고 업로드 S3 리전 버그 수정", "").as_deref(), Some("로고 업로드 S3 리전 버그 수정"));
        assert_eq!(promote_title("[결정/완료] Jenkins prod 잡 2개 생성", "").as_deref(), Some("Jenkins prod 잡 2개 생성"));
        assert_eq!(promote_title("[메모] 그냥 제목", "").as_deref(), Some("[메모] 그냥 제목"));
        // 항목 셋 이상인 목록은 첫 줄이 대표가 아니라 후보에서 뺀다.
        assert_eq!(promote_title("결정사항", "- 한 회사 내부 도구 — 권한 없음\n- Next.js 16\n- 비용 제한 없음\n"), None);
        assert_eq!(promote_title("결정사항", "- 한 회사 내부 도구 — 권한 없음\n- Next.js 16\n").as_deref(), Some("한 회사 내부 도구"));
        assert_eq!(promote_title("결정", ""), None);
        assert_eq!(fmt_n(1591), "1,591");
        assert_eq!(fmt_n(12), "12");
        assert_eq!(slugify("JWT refresh 7일로 확정!"), "jwt-refresh-7일로-확정");
    }

    #[test]
    fn promote_candidates_skip_today_short_and_already_moved_sections() {
        let v = temp_vault("promote");
        let long = "- 모바일 세션 유지와 보안의 트레이드오프를 따져 7일로 정했다.\n- 만료 처리는 refresh 흐름에서 다룬다.";
        write(&v, "A/daily/2026-11-01.md", &format!("# 2026-11-01\n\n### 결정: JWT refresh 7일\n_14:02 · claude-code_\n{long}\n\n### 결정: 짧음\n- 짧다\n\n### 결정: 이미 옮김\n→ [[decisions/x.md]] 로 옮겼어요.\n{long}\n"));
        write(&v, &format!("A/daily/{TODAY}.md"), &format!("### 결정: 오늘 것\n{long}\n"));
        write(&v, "A/daily/2026-10-30.md", &format!("### 결정: 캐시 끔\n{long}\n"));
        write(&v, "A/decisions/캐시-끔.md", "# 캐시 끔\n이미 손으로 옮긴 것");
        let found = scan(&v, "A", TODAY);
        let promotes: Vec<_> = found.iter().filter(|c| matches!(c.action, DreamAction::Promote { .. })).collect();
        assert_eq!(promotes.len(), 1, "{found:?}");
        match &promotes[0].action {
            DreamAction::Promote { daily, heading, title, target, doc_type } => {
                assert_eq!(daily, "daily/2026-11-01.md");
                assert_eq!(heading, "### 결정: JWT refresh 7일");
                assert_eq!(title, "JWT refresh 7일");
                assert_eq!(target, "decisions/2026-11-01-jwt-refresh-7일.md");
                assert_eq!(doc_type, "decision");
            }
            _ => unreachable!(),
        }
        std::fs::remove_dir_all(v).ok();
    }

    // ---- apply ----

    #[test]
    fn rewrite_links_keeps_alias_anchor_and_other_links() {
        let md = "a [[x.md]] b [[x|별칭]] c [[x.md#h]] d [[y.md]] e [[A/x.md]]";
        let (out, n) = rewrite_links_in(md, "A", "x.md", Some("notes/x.md"));
        assert_eq!(n, 4);
        // `.md` 없이 적은 링크는 그 표기 그대로 (`[[x|별칭]]` → `[[notes/x|별칭]]`).
        assert_eq!(out, "a [[notes/x.md]] b [[notes/x|별칭]] c [[notes/x.md#h]] d [[y.md]] e [[notes/x.md]]");
        let (out, n) = rewrite_links_in(md, "A", "x.md", None);
        assert_eq!(n, 4);
        assert_eq!(out, "a x b 별칭 c x d [[y.md]] e A/x");
        let (same, n) = rewrite_links_in("none [[z.md]]", "A", "x.md", Some("y.md"));
        assert_eq!((same.as_str(), n), ("none [[z.md]]", 0));
    }

    #[test]
    fn run_enqueues_and_apply_relink_commits_without_llm() {
        let v = temp_vault("apply-link");
        write(&v, "A/index.md", "참고: [[auth.md]] 와 [[auth.md|인증]]\n");
        write(&v, "A/notes/auth.md", "# auth\n");
        write(&v, "A/decisions/auth.md", "# 두 번째 auth — 모호하게\n");
        write(&v, "A/notes/other.md", "[[auth.md]]\n");
        vcs::ensure_repo(&v).unwrap();
        vcs::snapshot(&v, "init").unwrap();

        let report = run(&v, 8, false, None).unwrap();
        assert_eq!(report.enqueued, 1, "{report:?}");
        let items = pending(&v);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, KIND_FIX_LINK);
        assert_eq!(items[0].payload.as_ref().unwrap()["key"], "fix_link:A:auth.md");

        // 다시 돌려도 (지문 같음 → 건너뜀 / 강제 → 이미 올라가 있음) 중복 없음.
        let again = run(&v, 8, false, None).unwrap();
        assert_eq!((again.scanned_projects, again.enqueued), (0, 0));
        let forced = run(&v, 8, true, None).unwrap();
        assert_eq!((forced.scanned_projects, forced.enqueued), (1, 0));
        assert_eq!(pending(&v).len(), 1);

        let head_before = vcs::recent_commits(&v, 1).unwrap()[0].id.clone();
        let res = apply(&v, &items[0].id, Choice::Relink { target: "notes/auth.md".into() }).unwrap();
        assert_eq!(res.changed, vec!["index.md".to_string(), "notes/other.md".to_string()]);
        assert!(res.commit.is_some());
        assert_eq!(read(&v, "A/index.md"), "참고: [[notes/auth.md]] 와 [[notes/auth.md|인증]]\n");
        assert_eq!(read(&v, "A/notes/other.md"), "[[notes/auth.md]]\n");
        let head_after = vcs::recent_commits(&v, 1).unwrap().remove(0);
        assert_ne!(head_after.id, head_before);
        assert!(head_after.summary.contains("dreaming fix-link"), "{}", head_after.summary);
        assert!(pending(&v).is_empty());
        assert_eq!(load_store(&v).unwrap().decided["fix_link:A:auth.md"].status, "applied");

        // 적용 뒤 다시 돌려도 고친 링크가 다시 올라오지 않는다.
        let after = run(&v, 8, true, None).unwrap();
        assert_eq!(after.enqueued, 0, "{after:?}");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn keep_is_remembered_and_never_reproposed() {
        let v = temp_vault("keep");
        write(&v, "A/index.md", "[[nope.md]]\n");
        run(&v, 8, false, None).unwrap();
        let items = pending(&v);
        assert_eq!(items.len(), 1);
        let res = apply(&v, &items[0].id, Choice::Keep).unwrap();
        assert!(res.changed.is_empty() && res.commit.is_none());
        assert_eq!(read(&v, "A/index.md"), "[[nope.md]]\n");
        assert!(pending(&v).is_empty());
        assert_eq!(load_store(&v).unwrap().decided["fix_link:A:nope.md"].status, "dismissed");
        // 파일이 바뀌어 지문이 달라져도 같은 후보는 다시 안 올라온다.
        write(&v, "A/index.md", "[[nope.md]] 수정\n");
        let again = run(&v, 8, false, None).unwrap();
        assert_eq!((again.scanned_projects, again.enqueued), (1, 0));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn dismissing_in_the_review_inbox_is_remembered_too() {
        let v = temp_vault("dismiss");
        write(&v, "A/index.md", "[[nope.md]]\n");
        run(&v, 8, false, None).unwrap();
        let item = pending(&v).remove(0);
        reviews::resolve(&v, &item.id, reviews::ReviewStatus::Dismissed).unwrap();
        on_review_resolved(&v, &item, &reviews::ReviewStatus::Dismissed);
        assert_eq!(load_store(&v).unwrap().decided["fix_link:A:nope.md"].status, "dismissed");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn apply_remove_unlinks_text() {
        let v = temp_vault("remove");
        write(&v, "A/index.md", "참고 [[nope.md|옛 메모]] 끝\n");
        run(&v, 8, false, None).unwrap();
        let item = pending(&v).remove(0);
        apply(&v, &item.id, Choice::Remove).unwrap();
        assert_eq!(read(&v, "A/index.md"), "참고 옛 메모 끝\n");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn apply_verify_and_supersede_rewrite_frontmatter_only() {
        let v = temp_vault("verify");
        write(&v, "A/notes/old.md", "---\ntype: note\nstale_after: 2026-01-01\n---\n본문 그대로\n");
        write(&v, "A/notes/old2.md", "---\nstale_after: 2026-01-01\n---\n본문\n");
        run(&v, 8, false, None).unwrap();
        let items = pending(&v);
        assert_eq!(items.len(), 2);
        let old = items.iter().find(|i| i.domain.as_deref() == Some("notes/old.md")).unwrap();
        let old2 = items.iter().find(|i| i.domain.as_deref() == Some("notes/old2.md")).unwrap();

        let res = apply(&v, &old.id, Choice::Verify).unwrap();
        let md = read(&v, "A/notes/old.md");
        let trust = ontology::doc_trust(&md, &today_str());
        assert!(!trust.stale && trust.verified, "{md}");
        assert_eq!(trust.verified_by.as_deref(), Some("human"));
        assert!(md.ends_with("본문 그대로\n"));
        assert!(res.message.contains("stale_after"));

        apply(&v, &old2.id, Choice::Supersede).unwrap();
        assert_eq!(ontology::declared_status(&read(&v, "A/notes/old2.md")).as_deref(), Some("superseded"));

        // 연장된 날짜로 바뀌었으니 같은 키로는 다시 안 올라오고, 새 키도 아직 stale 아님.
        let again = run(&v, 8, true, None).unwrap();
        assert_eq!(again.enqueued, 0, "{again:?}");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn apply_promote_moves_section_and_leaves_a_link() {
        let v = temp_vault("promote-apply");
        let long = "- 모바일 세션 유지와 보안의 트레이드오프를 따져 7일로 정했다.\n- 만료 처리는 refresh 흐름에서 다룬다.";
        let daily = format!("# 2026-11-01\n\n### 결정: JWT refresh 7일\n_14:02 · claude-code_\n{long}\n\n### 메모\n- 그대로 남는 것\n");
        write(&v, "A/daily/2026-11-01.md", &daily);
        vcs::ensure_repo(&v).unwrap();
        vcs::snapshot(&v, "init").unwrap();

        // LLM 훅: 제목을 다듬는다 (없어도 동작하는 건 다른 테스트가 본다).
        let polish = |title: &str, body: &str| -> Option<String> {
            assert!(body.contains("트레이드오프"));
            Some(format!("{title} 확정"))
        };
        let report = run(&v, 8, false, Some(&polish)).unwrap();
        assert_eq!(report.enqueued, 1, "{report:?}");
        let item = pending(&v).remove(0);
        assert_eq!(item.kind, KIND_PROMOTE);
        assert_eq!(item.payload.as_ref().unwrap()["title"], "JWT refresh 7일 확정");
        assert_eq!(item.payload.as_ref().unwrap()["target"], "decisions/2026-11-01-jwt-refresh-7일-확정.md");

        let res = apply(&v, &item.id, Choice::Promote { title: None, target: None }).unwrap();
        assert_eq!(res.changed, vec!["daily/2026-11-01.md".to_string(), "decisions/2026-11-01-jwt-refresh-7일-확정.md".to_string()]);
        let doc = read(&v, "A/decisions/2026-11-01-jwt-refresh-7일-확정.md");
        assert!(doc.starts_with("---\ntype: decision\nstatus: active\n---\n\n# JWT refresh 7일 확정\n\n- 모바일"), "{doc}");
        assert!(doc.ends_with("관련:: [[daily/2026-11-01.md]]\n"));
        assert!(!doc.contains("_14:02"));
        let after = read(&v, "A/daily/2026-11-01.md");
        assert_eq!(
            after,
            "# 2026-11-01\n\n### 결정: JWT refresh 7일\n_14:02 · claude-code_\n→ [[decisions/2026-11-01-jwt-refresh-7일-확정.md]] 로 옮겼어요.\n\n### 메모\n- 그대로 남는 것\n"
        );
        let head = &vcs::recent_commits(&v, 1).unwrap()[0];
        assert!(head.summary.contains("dreaming promote"));
        // 링크 색인에서 새 문서가 daily 를 가리킨다.
        let idx = links::build_index(&v).unwrap();
        assert!(idx.incoming.contains_key("A/daily/2026-11-01.md"));
        // 다시 돌려도 옮긴 섹션은 후보가 아니다.
        let again = run(&v, 8, true, None).unwrap();
        assert_eq!(again.enqueued, 0, "{again:?}");
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn promote_with_user_overrides_and_conflicts() {
        let v = temp_vault("promote-override");
        let long = "원인은 preflight 가 /api/auth 에 설정되지 않아서였다. next.config.js headers 에 추가해 해결했다.";
        write(&v, "A/daily/2026-11-01.md", &format!("### 원인: CORS\n{long}\n"));
        write(&v, "A/bugs/exists.md", "x");
        run(&v, 8, false, None).unwrap();
        let item = pending(&v).remove(0);
        let err = apply(&v, &item.id, Choice::Promote { title: None, target: Some("bugs/exists".into()) }).unwrap_err();
        assert!(err.to_string().contains("이미 있는"));
        // 실패해도 항목은 pending 그대로.
        assert_eq!(pending(&v).len(), 1);
        apply(&v, &item.id, Choice::Promote { title: Some("CORS preflight 누락".into()), target: Some("bugs/cors".into()) }).unwrap();
        let doc = read(&v, "A/bugs/cors.md");
        assert!(doc.starts_with("---\ntype: bug\n---\n\n# CORS preflight 누락\n"));
        assert!(read(&v, "A/daily/2026-11-01.md").contains("→ [[bugs/cors.md]] 로 옮겼어요."));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn wrong_choice_for_an_action_is_rejected() {
        let v = temp_vault("wrong");
        write(&v, "A/index.md", "[[nope.md]]\n");
        run(&v, 8, false, None).unwrap();
        let item = pending(&v).remove(0);
        assert!(apply(&v, &item.id, Choice::Verify).is_err());
        assert!(apply(&v, &item.id, Choice::Relink { target: "ghost.md".into() }).is_err());
        assert_eq!(pending(&v).len(), 1);
        assert!(apply(&v, "no-such-id", Choice::Keep).is_err());
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn budget_caps_per_run_and_pending_total() {
        let v = temp_vault("budget");
        let mut body = String::new();
        for i in 0..30 {
            body.push_str(&format!("[[missing-{i}.md]]\n"));
        }
        write(&v, "A/index.md", &body);
        let r1 = run(&v, 5, false, None).unwrap();
        assert_eq!((r1.found, r1.enqueued), (30, 5));
        assert_eq!(r1.message, "검토함에 5건을 올렸어요. 정리 거리는 모두 30건 (링크 30) — 나머지는 다음에.");
        // 지문이 같아도 강제하면 나머지를 이어서 올린다 — 전체 상한 20.
        let r2 = run(&v, 100, true, None).unwrap();
        assert_eq!(r2.enqueued, 15);
        let r3 = run(&v, 100, true, None).unwrap();
        assert_eq!(r3.enqueued, 0);
        assert!(r3.message.contains("검토함이 차"));
        assert_eq!(pending(&v).len(), 20);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn empty_vault_and_project_without_docs_are_fine() {
        let v = temp_vault("empty");
        let r = run(&v, 8, false, None).unwrap();
        assert_eq!(r.enqueued, 0);
        assert!(r.message.contains("프로젝트가 없어요"));
        std::fs::create_dir_all(v.join("Projects/빈프로젝트")).unwrap();
        let r = run(&v, 8, true, None).unwrap();
        assert_eq!((r.scanned_projects, r.enqueued), (1, 0));
        let s = status(&v);
        assert!(s.last_run_at.is_some());
        assert_eq!(s.pending, 0);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn large_vault_scan_is_incremental_and_bounded() {
        let v = temp_vault("large");
        for p in 0..5 {
            for i in 0..80 {
                write(&v, &format!("P{p}/notes/n{i}.md"), &format!("# n{i}\n[[notes/n{}.md]]\n", (i + 1) % 80));
            }
            for d in 1..=70 {
                write(&v, &format!("P{p}/daily/2026-01-{:02}.md", (d % 28) + 1), "# 일지\n### 메모\n짧게\n");
            }
        }
        let start = std::time::Instant::now();
        let r = run(&v, 8, false, None).unwrap();
        assert_eq!((r.scanned_projects, r.enqueued), (5, 0));
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        // 한 프로젝트만 바꾸면 그것만 다시 본다.
        write(&v, "P2/index.md", "[[zzz.md]]\n");
        let r = run(&v, 8, false, None).unwrap();
        assert_eq!((r.scanned_projects, r.skipped_projects, r.enqueued), (1, 4, 1));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn replace_section_body_respects_levels_and_fences() {
        let md = "## A\nold\n```\n### fake\n```\n#### deeper\n## B\nkeep\n";
        let out = replace_section_body(md, "## A", "new\n");
        assert_eq!(out, "## A\nnew\n\n## B\nkeep\n");
    }
}

#[cfg(test)]
fn raw_ambiguous_items(docs: &HashMap<String, Vec<String>>, index: &links::LinkIndex) -> usize {
    docs.keys()
        .map(|p| scan_ambiguous_links(p, index).len())
        .sum()
}

#[cfg(test)]
mod contains_rule_tests {
    use super::*;

    #[test]
    fn contains_needs_real_overlap() {
        // 날짜만 겹침
        assert!(!contains_meaningfully("2026-10-21", "25-요금제-대시보드-매칭-배포-2026-10-21"));
        // 짧은 쪽이 절반도 안 됨
        assert!(!contains_meaningfully("상식이", "mixpanel-상식이"));
        // 진짜 줄임/덧붙임
        assert!(contains_meaningfully("cron-pipeline", "cron-pipeline-v2"));
        assert!(contains_meaningfully("낙서파이터-기획서", "기획서"));
        assert!(!contains_meaningfully("abc", "xyz"));
        assert!(is_daily_doc("daily/2026-10-21.md"));
        assert!(!is_daily_doc("notes/daily-brief.md"));
    }

    fn docs(v: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        v.iter()
            .map(|(p, ds)| (p.to_string(), ds.iter().map(|d| d.to_string()).collect()))
            .collect()
    }

    #[test]
    fn cross_project_needs_a_distinctive_unique_name() {
        let renames = HashMap::new();
        // 흔한 이름(plan.md)은 다른 프로젝트에서 권하지 않는다.
        let d = docs(&[("상식이", &["notes/museum.md"]), ("상식이_유튜브", &["dino/plan.md"])]);
        assert!(link_options("상식이", "docs/garden/plan.md", &d, &renames).is_empty());
        // 날짜 이름도.
        let d = docs(&[("A", &["notes/x.md"]), ("B", &["daily/2026-10-21.md"])]);
        assert!(link_options("A", "2026-10-21.md", &d, &renames).is_empty());
        assert!(link_options("A", "daily/2026-10-21.md", &d, &renames).is_empty());
        // 다른 두 프로젝트에 같은 이름이 있으면 고를 근거가 없다.
        let d = docs(&[("A", &[]), ("B", &["notes/help-hub.md"]), ("C", &["help-hub.md"])]);
        assert!(link_options("A", "notes/help-hub.md", &d, &renames)
            .iter()
            .all(|o| o.target == "B/notes/help-hub.md")); // 같은 경로만 남음
        // 숫자만 다른 이름은 '비슷한 이름' 이 아니다 (다른 판).
        let d = docs(&[("A", &["releases/1.1.1.md", "phase/P11.md", "notes/auth-flwo.md"])]);
        assert!(link_options("A", "releases/1.1.0.md", &d, &renames).is_empty());
        assert!(link_options("A", "phase/P12.md", &d, &renames).is_empty());
        assert_eq!(link_options("A", "notes/auth-flow.md", &d, &renames)[0].target, "notes/auth-flwo.md");
        // 폴더 없는 흔한 이름은 경로가 같아도 권하지 않는다.
        let d = docs(&[("A", &[]), ("B", &["release.md"])]);
        assert!(link_options("A", "release.md", &d, &renames).is_empty());
        // 폴더까지 같으면 권한다.
        let d = docs(&[("A", &[]), ("B", &["docs/garden/plan.md"])]);
        assert_eq!(link_options("A", "docs/garden/plan.md", &d, &renames)[0].target, "B/docs/garden/plan.md");
        // 특색 있는 이름이 한 곳에만 있으면 권한다.
        let d = docs(&[("A", &[]), ("B", &["guide/teamcity-운영가이드.md"])]);
        let o = link_options("A", "teamcity-운영가이드.md", &d, &renames);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].target, "B/guide/teamcity-운영가이드.md");
    }
}

#[cfg(test)]
mod scaffold_and_code_link_tests {
    use super::*;

    #[test]
    fn code_file_links_are_not_documents() {
        assert!(looks_like_asset("promotions.ts.md"));
        assert!(looks_like_asset("src/main.rs"));
        assert!(looks_like_asset("deploy.yml.md"));
        assert!(!looks_like_asset("notes/typescript-tips.md"));
        assert!(!looks_like_asset("release.md"));
    }

    #[test]
    fn skill_md_examples_are_not_broken_links() {
        let v = std::env::temp_dir().join(format!("danbi-dream-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&v);
        let p = v.join("Projects").join("A");
        std::fs::create_dir_all(p.join("notes")).unwrap();
        std::fs::write(p.join("SKILL.md"), "## Link — 언제 [[wiki-link]] 를 넣는가\n[[notes/auth-security.md]]\n").unwrap();
        std::fs::write(p.join("plan.md"), "[[gone.md]]\n").unwrap();
        let index = links::build_index(&v).unwrap();
        let docs: HashMap<String, Vec<String>> = [("A".to_string(), list_docs(&v, "A"))].into();
        let found = scan_broken_links(&v, "A", &index, &docs);
        let broken: Vec<String> = found
            .iter()
            .filter_map(|c| match &c.action {
                DreamAction::FixLink { broken, .. } => Some(broken.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(broken, vec!["gone.md".to_string()], "{broken:?}");
        let _ = std::fs::remove_dir_all(&v);
    }
}

/// 실제 vault 의 **복제본**으로 정리 한 바퀴 + 종류별 승인 1건씩 — 손으로:
/// `DANBI_DREAM_VAULT=<복제본> cargo test --lib dream_real_clone -- --ignored --nocapture`.
/// 복제본에 쓰므로 실제 vault 경로를 넘기지 말 것.
#[cfg(test)]
mod real_clone {
    use super::*;

    /// 남은 깨진 링크가 진짜인지 분류만 (읽기 전용).
    #[test]
    #[ignore]
    fn dream_clone_breakdown() {
        let Some(dir) = std::env::var_os("DANBI_DREAM_VAULT") else { return };
        let vault = std::path::PathBuf::from(dir);
        let index = links::build_index(&vault).unwrap();
        let projects = vault::project_names(&vault).unwrap();
        let docs: HashMap<String, Vec<String>> = projects.iter().map(|p| (p.clone(), list_docs(&vault, p))).collect();
        let mut cats: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        let mut sources = 0usize;
        for p in &projects {
            for c in scan_broken_links(&vault, p, &index, &docs) {
                match &c.action {
                    DreamAction::FixLink { broken, sources: s, options, .. } => {
                        sources += s.len();
                        let cat = options.first().map(|o| o.why.clone()).unwrap_or_else(|| "후보 없음".into());
                        cats.entry(cat).or_default().push(format!("{p}: [[{broken}]] ← {}", s.join(", ")));
                    }
                    DreamAction::FixLinkBatch { items } => {
                        cats.entry("묶음".into()).or_default().push(format!("{p}: {}종", items.len()));
                    }
                    _ => {}
                }
            }
        }
        println!("문서에서 깨진 링크가 나오는 곳(소스) 합계: {sources}");
        for (k, v) in &cats {
            println!("== {k}: {}", v.len());
            for line in v.iter().take(8) { println!("   {}", line.chars().take(160).collect::<String>()); }
        }
    }

    /// 검토함 UI 가 부르는 경로 그대로: 그대로 두기 · 다시 확인(아직 맞아요) ·
    /// 애매한 링크 고르기 · 폴더 없는 링크는 후보가 안 되는지. 복제본 전용.
    #[test]
    #[ignore]
    fn ui_flows_on_clone() {
        let Some(dir) = std::env::var_os("DANBI_DREAM_VAULT") else { return };
        let vault = std::path::PathBuf::from(dir);
        assert!(vault.to_string_lossy().contains("scratchpad"), "복제본만");
        // 다른 대기 항목이 상한을 채우지 않게 비우고 강제 실행.
        let mut st = reviews::load(&vault).unwrap();
        st.items.retain(|it| !(it.status == reviews::ReviewStatus::Pending && it.kind.starts_with(KIND_PREFIX)));
        reviews::save(&vault, &st).unwrap();
        let rep = run(&vault, 200, true, None).unwrap();
        println!("run: {}", rep.message);
        let mine = |needle: &str| -> Vec<reviews::ReviewItem> {
            reviews::load(&vault).unwrap().items.into_iter()
                .filter(|it| it.status == reviews::ReviewStatus::Pending && it.project.as_deref() == Some("단비"))
                .filter(|it| it.payload.as_ref().map(|p| p.to_string().contains(needle)).unwrap_or(false) || it.reason.contains(needle))
                .collect()
        };
        // 폴더 없는 링크·코드 안 예시는 후보가 아니어야 한다.
        assert!(mine("comparison-obsidian-raycast").iter().all(|it| !it.reason.contains("[[comparison-obsidian-raycast.md]]")), "폴더 없는 링크가 깨진 링크로 올라옴");
        assert!(mine("nothing-here").is_empty(), "코드 안 예시가 링크로 잡힘");
        // 1) 애매한 링크 → 고르기
        let amb = mine("dup-test");
        println!("dup-test 항목: {:?}", amb.iter().map(|i| (&i.kind, &i.reason)).collect::<Vec<_>>());
        let amb = amb.into_iter().find(|i| i.payload.as_ref().map(|p| p["ambiguous"] == true || p.to_string().contains("\"ambiguous\":true")).unwrap_or(false)).expect("애매한 링크 항목");
        let r = apply(&vault, &amb.id, Choice::Relink { target: "notes/dup-test.md".into() }).unwrap();
        println!("고르기 → {} {:?}", r.message, r.commit);
        let txt = std::fs::read_to_string(vault.join("Projects/단비/notes/dream-link-test.md")).unwrap();
        assert!(txt.contains("[[notes/dup-test.md]]"), "고른 경로로 바뀌어야 함: {txt}");
        assert!(txt.contains("`[[nothing-here.md]]`"), "코드 안은 그대로");
        // 검토함 상한(20) 때문에 단비 항목이 다 못 올라올 수 있다 — 다른 프로젝트
        // 대기 항목을 비우고 몇 번 더 돌려 채운다 (결정 기록은 건드리지 않음).
        for _ in 0..6 {
            if !mine("demo-scrpt").is_empty() && !mine("dream-stale-test").is_empty() { break; }
            let mut st = reviews::load(&vault).unwrap();
            st.items.retain(|it| !(it.status == reviews::ReviewStatus::Pending && it.kind.starts_with(KIND_PREFIX) && it.project.as_deref() != Some("단비")));
            reviews::save(&vault, &st).unwrap();
            let _ = run(&vault, 200, true, None).unwrap();
        }
        // 2) 오타 링크 → 그대로 두기
        let typo = mine("demo-scrpt");
        println!("demo-scrpt 항목: {:?}", typo.iter().map(|i| &i.reason).collect::<Vec<_>>());
        let typo = typo.into_iter().next().expect("오타 링크 항목");
        let before = std::fs::read_to_string(vault.join("Projects/단비/notes/dream-link-test.md")).unwrap();
        let r = apply(&vault, &typo.id, Choice::Keep).unwrap();
        println!("그대로 두기 → {} changed={:?} commit={:?}", r.message, r.changed, r.commit);
        assert!(r.changed.is_empty() && r.commit.is_none());
        assert_eq!(before, std::fs::read_to_string(vault.join("Projects/단비/notes/dream-link-test.md")).unwrap());
        // 3) 기한 지난 문서 → 아직 맞아요
        let stale = mine("dream-stale-test").into_iter().next().expect("다시 확인 항목");
        println!("stale 항목: {} / {}", stale.kind, stale.reason);
        let r = apply(&vault, &stale.id, Choice::Verify).unwrap();
        println!("아직 맞아요 → {} {:?}", r.message, r.commit);
        let head = std::fs::read_to_string(vault.join("Projects/단비/notes/dream-stale-test.md")).unwrap();
        println!("--- 확인 후 앞부분 ---\n{}", head.lines().take(6).collect::<Vec<_>>().join("\n"));
        assert!(!head.contains("stale_after: 2026-10-01"), "기한이 연장돼야 함");
        // 4) 다시 돌려도 결정한 것은 안 올라온다.
        let rep = run(&vault, 200, true, None).unwrap();
        println!("재실행: {}", rep.message);
        assert!(mine("demo-scrpt").is_empty(), "그대로 두기 한 항목이 다시 올라옴");
        assert!(mine("dream-stale-test").is_empty(), "확인한 문서가 다시 올라옴");
        assert!(mine("dup-test").is_empty(), "고른 링크가 다시 올라옴");
        println!("OK");
    }

    /// 복제본: 자동 수리 켜고 한 바퀴 → 에이전트가 한 프로젝트 정리 목록을 받아 promote 1건 처리.
    #[test]
    #[ignore]
    fn auto_fix_and_agent_tidy_on_clone() {
        let Some(dir) = std::env::var_os("DANBI_DREAM_VAULT") else { return };
        let vault = std::path::PathBuf::from(dir);
        assert!(vault.to_string_lossy().contains("scratchpad"), "복제본만");
        let t = std::time::Instant::now();
        let r = run_with(&vault, &RunOptions { max_per_run: 8, force: true, auto_fix_links: true, only_project: None }, None).unwrap();
        println!("자동 수리 실행 {:?}: {}", t.elapsed(), r.message);
        for a in &r.auto_fixed { println!("  · {} {} {:?}", a.project, a.message, a.changed); }
        let t = std::time::Instant::now();
        let list = agent_tidy_list(&vault, "단비", true).unwrap();
        println!("danbi_tidy(단비, scan) {:?} · {}B · pending {}", t.elapsed(), list.to_string().len(), list["pending"]);
        for it in list["items"].as_array().unwrap().iter().take(4) {
            println!("  - {} | {}", it["kind"], it["reason"].as_str().unwrap_or("").chars().take(90).collect::<String>());
        }
        if let Some(p) = list["items"].as_array().unwrap().iter().find(|i| i["kind"] == KIND_PROMOTE) {
            let actor = Actor::Agent { client: "claude-code".into(), note: Some("결정 섹션이라 decisions 로".into()) };
            let res = apply_as(&vault, p["id"].as_str().unwrap(), Choice::Promote { title: None, target: None }, &actor).unwrap();
            println!("에이전트 promote → {} {:?}", res.message, res.commit);
            let c = &vcs::recent_commits(&vault, 3).unwrap()[0];
            println!("  커밋: {}", c.summary);
        }
    }

    /// 복제본에서 briefing 의 정리·기억 요약이 어떻게 나가는지 (키체인 안 거침).
    #[test]
    #[ignore]
    fn briefing_on_clone() {
        let Some(dir) = std::env::var_os("DANBI_DREAM_VAULT") else { return };
        let vault = std::path::PathBuf::from(dir);
        let t = std::time::Instant::now();
        let b = crate::dashboard::briefing(&vault, Some("단비")).unwrap();
        let v = serde_json::to_value(&b).unwrap();
        println!("briefing(단비) {:?} · {}B", t.elapsed(), v.to_string().len());
        println!("tidy_pending={} memory={}", v["tidy_pending"], v["memory"]);
        let t = std::time::Instant::now();
        let b = crate::dashboard::briefing(&vault, None).unwrap();
        let v = serde_json::to_value(&b).unwrap();
        println!("briefing(전체) {:?} · {}B · bloating={}", t.elapsed(), v.to_string().len(), v["bloating_projects"]);
    }

    #[test]
    #[ignore]
    fn dream_real_clone() {
        let Some(dir) = std::env::var_os("DANBI_DREAM_VAULT") else { return };
        let vault = std::path::PathBuf::from(dir);
        assert!(!vault.ends_with("Danbi_Vault") || vault.to_string_lossy().contains("scratchpad"), "복제본만");
        // 묶음 전/후 종류별 후보 수.
        {
            let index = links::build_index(&vault).unwrap();
            let docs: HashMap<String, Vec<String>> =
                vault::project_names(&vault).unwrap().into_iter().map(|p| { let d = list_docs(&vault, &p); (p, d) }).collect();
            let today = today_str();
            let mut raw = [0usize; 3];
            let mut bundled = [0usize; 4];
            for p in docs.keys() {
                let found = scan_project(&vault, p, &index, &docs, &today);
                for c in &found { raw[category(&c.action)] += 1; }
                for c in bundle_fix_links(p, &docs, found) {
                    let i = match c.action { DreamAction::FixLinkBatch { .. } => 3, ref a => category(a) };
                    bundled[i] += 1;
                }
            }
            // 해석기 전 기준 고아 수 (적힌 그대로의 링크만 incoming 으로 침).
            let index_like = ["index.md", "readme.md", "overview.md", "plan.md", "ui.md", "purpose.md", "schema.md", "skill.md", "memory_index.md"];
            let mut targets_raw: HashSet<String> = HashSet::new();
            let mut ambiguous_links = 0usize;
            for (p, list) in &docs {
                for d in list {
                    let md = vault::read_doc(&vault, p, d).unwrap_or_default();
                    for (_, l) in links::extract_links_at(&md, p) {
                        if docs.contains_key(&l.project) {
                            targets_raw.insert(format!("{}/{}", l.project, l.domain));
                        } else {
                            targets_raw.insert(format!("{p}/{}/{}", l.project, l.domain));
                        }
                    }
                }
            }
            for list in index.ambiguous.values() {
                ambiguous_links += list.len();
            }
            let orphan_count = |targets: &HashSet<String>| -> usize {
                docs.iter()
                    .flat_map(|(p, list)| list.iter().map(move |d| (p, d)))
                    .filter(|(_, d)| !d.starts_with("daily/") && !index_like.contains(&d.to_lowercase().as_str()))
                    .filter(|(p, d)| !targets.contains(&format!("{p}/{d}")))
                    .count()
            };
            let targets_resolved: HashSet<String> = index.incoming.keys().cloned().collect();
            println!("orphans: before(raw links)={} after(resolver)={} · ambiguous links={}", orphan_count(&targets_raw), orphan_count(&targets_resolved), ambiguous_links);
            let amb_items = raw_ambiguous_items(&docs, &index);
            println!("ambiguous pick-items={amb_items}");
            println!("raw: links={} recheck={} promote={}", raw[0], raw[1], raw[2]);
            println!("bundled: single-links={} recheck={} promote={} batches={}", bundled[0], bundled[1], bundled[2], bundled[3]);
        }
        let t = std::time::Instant::now();
        let report = run(&vault, 30, true, None).unwrap();
        println!("run: {:?} in {:?}", report, t.elapsed());
        let t = std::time::Instant::now();
        let again = run(&vault, 30, false, None).unwrap();
        println!("rerun(증분): {} in {:?}", again.message, t.elapsed());
        let store = reviews::load(&vault).unwrap();
        let items: Vec<_> = store.items.iter().filter(|i| i.kind.starts_with(KIND_PREFIX)).cloned().collect();
        for it in &items {
            println!("- [{}] {} {:?} | {}", it.kind, it.project.clone().unwrap_or_default(), it.domain, it.reason);
            println!("    {}", it.payload.as_ref().map(|p| p.to_string()).unwrap_or_default().chars().take(300).collect::<String>());
        }
        let mut done = HashSet::new();
        for it in &items {
            if !done.insert(it.kind.clone()) { continue; }
            let p = it.payload.clone().unwrap_or_default();
            let choice = match p["kind"].as_str().unwrap_or("") {
                "fix_link_batch" => Choice::ApplyAll,
                "fix_link" => match p["options"][0]["target"].as_str() {
                    Some(t) => Choice::Relink { target: t.to_string() },
                    None => Choice::Remove,
                },
                "stale" => Choice::Verify,
                "cites_superseded" => match p["replacement"].as_str() {
                    Some(t) => Choice::Relink { target: t.to_string() },
                    None => Choice::Remove,
                },
                "promote" => Choice::Promote { title: None, target: None },
                _ => continue,
            };
            println!("apply {} {:?}", it.kind, choice);
            match apply(&vault, &it.id, choice) {
                Ok(r) => println!("  → {:?}", r),
                Err(e) => println!("  ✗ {e}"),
            }
        }
        let third = run(&vault, 30, true, None).unwrap();
        println!("재실행(강제): found={} enqueued={} — 승인·대기 항목은 다시 안 올라와야 함", third.found, third.enqueued);
    }
}
