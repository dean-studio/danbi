//! Soft-delete trash for Danbi vault.
//!
//! When a user deletes a domain or sub-folder we move the actual content
//! into `~/Danbi_Vault/.danbi/trash/<id>/` rather than removing it from
//! disk. Each entry carries a `meta.json` describing what it was so we
//! can restore to the same project/path later.
//!
//! Layout:
//! ```
//! .danbi/trash/
//!   ├── 20260517-134022-1f3a4b5c/
//!   │   ├── meta.json
//!   │   └── payload          (the file, or a directory containing it)
//!   └── ...
//! ```
//!
//! The trash directory lives under `.danbi/`, which the watcher's
//! `is_tree_relevant` filter already ignores — so soft-deleting won't
//! cause cascading sidebar refreshes.

use crate::error::{DanbiError, DanbiResult};
use crate::vault::PROJECTS_DIRNAME;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PAYLOAD: &str = "payload";
const META_FILE: &str = "meta.json";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrashEntry {
    /// Filesystem-safe id ("YYYYMMDD-HHMMSS-<hash>"). Doubles as the
    /// directory name under `.danbi/trash/`.
    pub id: String,
    pub project: String,
    /// Path relative to the project root. For files this is the domain
    /// (e.g. `daily/2026-05-17.md`); for folders it's the folder path.
    pub original_path: String,
    /// "file" | "folder".
    pub kind: String,
    pub deleted_at: i64,
    pub size_bytes: u64,
}

fn trash_root(vault: &Path) -> PathBuf {
    vault.join(".danbi").join("trash")
}

fn ensure_trash_root(vault: &Path) -> DanbiResult<PathBuf> {
    let p = trash_root(vault);
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

fn short_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:08x}", h.finish() as u32)
}

fn make_id(seed: &str) -> String {
    let now = chrono::Local::now();
    let stamp = now.format("%Y%m%d-%H%M%S").to_string();
    let hash = short_hash(&format!("{seed}-{}", now.timestamp_millis()));
    format!("{stamp}-{hash}")
}

fn dir_size(p: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    for de in rd.flatten() {
        let path = de.path();
        if path.is_file() {
            total += path.metadata().map(|m| m.len()).unwrap_or(0);
        } else if path.is_dir() {
            total += dir_size(&path);
        }
    }
    total
}

fn write_meta(dir: &Path, entry: &TrashEntry) -> DanbiResult<()> {
    let s = serde_json::to_string_pretty(entry)
        .map_err(|e| DanbiError::Other(format!("trash meta serialize: {e}")))?;
    std::fs::write(dir.join(META_FILE), s)?;
    Ok(())
}

fn read_meta(dir: &Path) -> Option<TrashEntry> {
    let s = std::fs::read_to_string(dir.join(META_FILE)).ok()?;
    serde_json::from_str(&s).ok()
}

/// Move a `.md` domain file into trash. The original_path stored in
/// meta.json is the value the caller passed in (already normalized to
/// include the `.md` extension).
pub fn trash_file(vault: &Path, project: &str, domain: &str) -> DanbiResult<TrashEntry> {
    let proj_dir = vault.join(PROJECTS_DIRNAME).join(project);
    let src = proj_dir.join(domain);
    if !src.is_file() {
        return Err(DanbiError::Config(format!(
            "file not found: {project}/{domain}"
        )));
    }
    let id = make_id(&format!("{project}/{domain}"));
    let dest_dir = ensure_trash_root(vault)?.join(&id);
    std::fs::create_dir_all(&dest_dir)?;
    let size = src.metadata().map(|m| m.len()).unwrap_or(0);
    std::fs::rename(&src, dest_dir.join(PAYLOAD))?;
    let entry = TrashEntry {
        id,
        project: project.to_string(),
        original_path: domain.to_string(),
        kind: "file".into(),
        deleted_at: chrono::Local::now().timestamp(),
        size_bytes: size,
    };
    write_meta(&dest_dir, &entry)?;
    Ok(entry)
}

/// Move an entire project (directory + every domain/sub-folder inside)
/// into trash. The `original_path` field is empty because the project
/// itself is the unit; on restore we recreate `<vault>/Projects/<name>/`.
pub fn trash_project(vault: &Path, project: &str) -> DanbiResult<TrashEntry> {
    let proj_dir = vault.join(PROJECTS_DIRNAME).join(project);
    if !proj_dir.is_dir() {
        return Err(DanbiError::Config(format!("project not found: {project}")));
    }
    let id = make_id(&format!("project::{project}"));
    let dest_dir = ensure_trash_root(vault)?.join(&id);
    std::fs::create_dir_all(&dest_dir)?;
    let size = dir_size(&proj_dir);
    std::fs::rename(&proj_dir, dest_dir.join(PAYLOAD))?;
    let entry = TrashEntry {
        id,
        project: project.to_string(),
        original_path: String::new(),
        kind: "project".into(),
        deleted_at: chrono::Local::now().timestamp(),
        size_bytes: size,
    };
    write_meta(&dest_dir, &entry)?;
    Ok(entry)
}

/// Move a sub-folder (with everything inside) into trash.
pub fn trash_folder(vault: &Path, project: &str, folder: &str) -> DanbiResult<TrashEntry> {
    let proj_dir = vault.join(PROJECTS_DIRNAME).join(project);
    let src = proj_dir.join(folder);
    if !src.is_dir() {
        return Err(DanbiError::Config(format!(
            "folder not found: {project}/{folder}"
        )));
    }
    let id = make_id(&format!("{project}/{folder}"));
    let dest_dir = ensure_trash_root(vault)?.join(&id);
    std::fs::create_dir_all(&dest_dir)?;
    let size = dir_size(&src);
    std::fs::rename(&src, dest_dir.join(PAYLOAD))?;
    let entry = TrashEntry {
        id,
        project: project.to_string(),
        original_path: folder.to_string(),
        kind: "folder".into(),
        deleted_at: chrono::Local::now().timestamp(),
        size_bytes: size,
    };
    write_meta(&dest_dir, &entry)?;
    Ok(entry)
}

/// All trash entries, newest first.
pub fn list(vault: &Path) -> DanbiResult<Vec<TrashEntry>> {
    let root = trash_root(vault);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for de in std::fs::read_dir(&root)? {
        let de = de?;
        let p = de.path();
        if !p.is_dir() {
            continue;
        }
        if let Some(entry) = read_meta(&p) {
            out.push(entry);
        }
    }
    out.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
    Ok(out)
}

/// Move payload back to its original location. Errors out if the target
/// already exists (we don't silently overwrite — the user can sort it
/// out manually). The trash directory is removed on success.
pub fn restore(vault: &Path, id: &str) -> DanbiResult<TrashEntry> {
    let dir = trash_root(vault).join(id);
    let entry = read_meta(&dir)
        .ok_or_else(|| DanbiError::Config(format!("trash entry not found: {id}")))?;
    let target = if entry.kind == "project" {
        // 프로젝트 단위 복원: original_path 가 비어있고 target 은
        // <vault>/Projects/<project>.
        vault.join(PROJECTS_DIRNAME).join(&entry.project)
    } else {
        vault
            .join(PROJECTS_DIRNAME)
            .join(&entry.project)
            .join(&entry.original_path)
    };
    if target.exists() {
        return Err(DanbiError::Config(format!(
            "cannot restore — target already exists: {}/{}",
            entry.project, entry.original_path
        )));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(dir.join(PAYLOAD), &target)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(entry)
}

/// Permanently remove a single trash entry.
pub fn purge(vault: &Path, id: &str) -> DanbiResult<()> {
    let dir = trash_root(vault).join(id);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Permanently empty the trash. Returns how many entries were removed.
pub fn empty_all(vault: &Path) -> DanbiResult<usize> {
    let root = trash_root(vault);
    if !root.exists() {
        return Ok(0);
    }
    let mut count = 0usize;
    for de in std::fs::read_dir(&root)? {
        let de = de?;
        let p = de.path();
        if p.is_dir() {
            std::fs::remove_dir_all(&p)?;
            count += 1;
        }
    }
    Ok(count)
}

/// 자동 만료 — 휴지통에 들어간 지 30일 넘은 entry 들을 영구 삭제.
/// 앱 시작 시 한 번 호출돼서 사용자가 의식하지 않아도 vault 가
/// 무한 비대해지지 않게 한다.
///
/// idempotent + best-effort: 한 entry 의 삭제 실패가 다음 entry 처리를
/// 막지 않는다. 로그·UI 표시 없이 조용히 동작 (사용자가 휴지통 카운트
/// 줄어든 걸로 자연스럽게 파악).
const TRASH_TTL_DAYS: i64 = 30;

pub fn expire_old(vault: &Path) -> DanbiResult<usize> {
    let root = trash_root(vault);
    if !root.exists() {
        return Ok(0);
    }
    let cutoff = chrono::Local::now().timestamp() - TRASH_TTL_DAYS * 24 * 3600;
    let mut count = 0usize;
    for de in std::fs::read_dir(&root)? {
        let Ok(de) = de else { continue };
        let p = de.path();
        if !p.is_dir() {
            continue;
        }
        let Some(entry) = read_meta(&p) else {
            // meta.json 없는 디렉토리는 손대지 않는다 — 사람이 직접
            // 만져둔 거나 프로토콜 변경 흔적일 수 있어서.
            continue;
        };
        if entry.deleted_at < cutoff {
            if std::fs::remove_dir_all(&p).is_ok() {
                count += 1;
            }
        }
    }
    Ok(count)
}
