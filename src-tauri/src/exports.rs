//! Daily 노트 요약 / HTML export 의 영구 보관 + 인덱스.
//!
//! 단비가 한 번 만든 HTML 요약은 사용자가 나중에 다시 보고 싶을 수 있다.
//! 매번 LLM 호출해서 재생성하면 토큰 낭비 + 결과가 매번 미세하게 다름.
//! → vault 안의 `.danbi/exports/` 에 markdown · html 페어를 저장하고
//!   `index.jsonl` 에 메타데이터를 한 줄씩 append.
//!
//! ## Layout
//! ```
//! .danbi/exports/
//!   ├── index.jsonl                        # one record per line
//!   └── <project>/
//!       ├── <slug>-<timestamp>.md          # 요약 markdown 원본
//!       └── <slug>-<timestamp>.html        # 렌더링된 페이지
//! ```
//!
//! 위치는 `.danbi/` 안이라 watcher 가 무시 + git 자동 커밋으로 버전 관리
//! 무료. vault 트리뷰는 깨끗하게 유지되고, 사용자는 단비의 export
//! 패널에서만 history 를 본다.

use crate::error::{DanbiError, DanbiResult};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const EXPORTS_DIR: &str = "exports";
const INDEX_FILE: &str = "index.jsonl";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExportRecord {
    /// "<timestamp>-<short-hash>" — 파일명·인덱스 키 모두로 사용.
    pub id: String,
    pub project: String,
    /// 원본 markdown 노트의 vault 상 경로 (e.g. "daily/2026-05-17.md").
    pub source_domain: String,
    pub created_at: i64,
    pub provider: String,
    pub model: String,
    /// 생성된 markdown 요약 파일 경로 (vault 상대 경로,
    /// "<project>/<slug>-<ts>.md" 형태).
    pub md_path: String,
    /// 생성된 HTML 파일 경로 (같은 패턴).
    pub html_path: String,
    /// Markdown 본문 길이 (사용자에게 "이 정도 길이" 표시용).
    pub md_bytes: u64,
}

fn root(vault: &Path) -> PathBuf {
    vault.join(".danbi").join(EXPORTS_DIR)
}

fn ensure_root(vault: &Path) -> DanbiResult<PathBuf> {
    let p = root(vault);
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

fn slugify(domain: &str) -> String {
    // "daily/2026-05-17.md" → "daily-2026-05-17"
    domain
        .trim_end_matches(".md")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn short_hash(seed: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    format!("{:08x}", h.finish() as u32)
}

/// Save a freshly-generated summary + html. Returns the persisted record.
pub fn save_export(
    vault: &Path,
    project: &str,
    source_domain: &str,
    summary_md: &str,
    html: &str,
    provider: &str,
    model: &str,
) -> DanbiResult<ExportRecord> {
    let root = ensure_root(vault)?;
    let proj_dir = root.join(project);
    std::fs::create_dir_all(&proj_dir)?;

    let now = Local::now();
    let ts_label = now.format("%Y%m%d-%H%M%S").to_string();
    let slug = slugify(source_domain);
    let id_seed = format!("{project}/{source_domain}/{}", now.timestamp_millis());
    let id = format!("{ts_label}-{}", short_hash(&id_seed));
    let stem = if slug.is_empty() {
        id.clone()
    } else {
        format!("{slug}-{ts_label}")
    };

    let md_rel = format!("{project}/{stem}.md");
    let html_rel = format!("{project}/{stem}.html");
    std::fs::write(proj_dir.join(format!("{stem}.md")), summary_md)?;
    std::fs::write(proj_dir.join(format!("{stem}.html")), html)?;

    let record = ExportRecord {
        id,
        project: project.to_string(),
        source_domain: source_domain.to_string(),
        created_at: now.timestamp(),
        provider: provider.to_string(),
        model: model.to_string(),
        md_path: md_rel,
        html_path: html_rel,
        md_bytes: summary_md.as_bytes().len() as u64,
    };
    append_index(vault, &record)?;
    Ok(record)
}

fn append_index(vault: &Path, rec: &ExportRecord) -> DanbiResult<()> {
    use std::io::Write;
    let path = root(vault).join(INDEX_FILE);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let line = serde_json::to_string(rec)
        .map_err(|e| DanbiError::Other(format!("export index serialize: {e}")))?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// All exports, newest first. Optionally filtered by project + source_domain
/// so the daily-note header can show "이 노트의 이전 요약 N개".
pub fn list(
    vault: &Path,
    project: Option<&str>,
    source_domain: Option<&str>,
) -> DanbiResult<Vec<ExportRecord>> {
    let path = root(vault).join(INDEX_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut out: Vec<ExportRecord> = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<ExportRecord>(line) else {
            continue;
        };
        if let Some(p) = project {
            if rec.project != p {
                continue;
            }
        }
        if let Some(d) = source_domain {
            if rec.source_domain != d {
                continue;
            }
        }
        out.push(rec);
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// Read the persisted HTML file by export id. Used by the "다시 열기"
/// button so we don't have to keep the html in memory.
pub fn read_html(vault: &Path, id: &str) -> DanbiResult<String> {
    let entries = list(vault, None, None)?;
    let rec = entries
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| DanbiError::Config(format!("export not found: {id}")))?;
    let abs = root(vault)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(EXPORTS_DIR)
        .join(&rec.html_path);
    // root() = .danbi/exports — so .html_path 가 "<project>/<file>.html"
    // 형태일 때 정확한 절대 경로는 root().join(html_path).
    let primary = root(vault).join(&rec.html_path);
    let chosen = if primary.exists() { primary } else { abs };
    Ok(std::fs::read_to_string(chosen)?)
}
