//! One-way vault mirror.
//!
//! Copies the vault tree into a user-chosen destination folder (Dropbox,
//! iCloud Drive, OneDrive, whatever). This is *backup*, not bi-directional
//! sync — edits in the mirror never propagate back.
//!
//! Safety guarantees:
//!   - Atomic file writes (write to `*.danbi-tmp`, rename on success).
//!   - mtime-based skip so re-runs are cheap.
//!   - Destination must not be inside the vault (prevents recursion).
//!   - Symlinks are not followed (prevents cycle loops).
//!   - Excludes `.git/` and `.danbi/` by default (would corrupt the git repo
//!     if edited at the destination).

use crate::error::{DanbiError, DanbiResult};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Serialize, Clone)]
pub struct BackupReport {
    pub copied: usize,
    pub skipped: usize,
    pub removed: usize,
    pub bytes: u64,
    /// Duration in milliseconds so the UI can show "took 340ms".
    pub duration_ms: u128,
}

/// Rejects destinations that would cause the backup to mirror itself.
/// Called both from the Tauri command that sets the path and before every
/// run, so even manual config edits can't wedge us into an infinite loop.
pub fn validate_destination(vault: &Path, dest: &Path) -> DanbiResult<()> {
    if !dest.is_absolute() {
        return Err(DanbiError::Config(
            "백업 경로는 절대 경로여야 해요.".into(),
        ));
    }
    // Canonicalize both sides where possible; fall back to the raw path
    // when the destination doesn't exist yet so first-run creation works.
    let vault_norm = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    let dest_norm = dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf());
    if dest_norm.starts_with(&vault_norm) {
        return Err(DanbiError::Config(
            "백업 경로가 vault 안쪽이에요. 다른 위치를 골라주세요.".into(),
        ));
    }
    Ok(())
}

/// Runs a full backup pass. Returns a report the UI can show.
///
/// Strategy:
///   1. Walk the vault, skipping excluded prefixes + symlinks.
///   2. For each file, compare mtime against the destination file.
///   3. If newer (or missing), write to `<target>.danbi-tmp` then rename.
///   4. After the walk, walk the destination and delete orphans (files that
///      no longer exist in the vault). Symmetric deletion is what makes
///      this behave like `rsync -a --delete`.
pub fn run(
    vault: &Path,
    dest: &Path,
    exclude: &[String],
) -> DanbiResult<BackupReport> {
    validate_destination(vault, dest)?;
    let start = std::time::Instant::now();

    std::fs::create_dir_all(dest)?;

    let mut report = BackupReport {
        copied: 0,
        skipped: 0,
        removed: 0,
        bytes: 0,
        duration_ms: 0,
    };

    let mut vault_files: Vec<PathBuf> = Vec::new();
    walk_collect(vault, vault, exclude, &mut vault_files)?;

    // --- Copy / update ---
    for rel in &vault_files {
        let src = vault.join(rel);
        let tgt = dest.join(rel);
        // Skip if destination is up to date (same mtime + same size).
        if should_skip(&src, &tgt) {
            report.skipped += 1;
            continue;
        }
        if let Some(parent) = tgt.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = atomic_copy(&src, &tgt)?;
        report.copied += 1;
        report.bytes += bytes;
    }

    // --- Prune orphans in destination ---
    // Build a quick lookup of relative paths that SHOULD exist in dest.
    let wanted: std::collections::HashSet<PathBuf> =
        vault_files.iter().cloned().collect();
    let mut dest_files: Vec<PathBuf> = Vec::new();
    walk_collect(dest, dest, exclude, &mut dest_files)?;
    for rel in &dest_files {
        if !wanted.contains(rel) {
            let victim = dest.join(rel);
            if victim.is_file() {
                if std::fs::remove_file(&victim).is_ok() {
                    report.removed += 1;
                }
            }
        }
    }
    // Clean up any stale temp files left over from a previous crash.
    for rel in &dest_files {
        if rel
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with(".danbi-tmp"))
            .unwrap_or(false)
        {
            let _ = std::fs::remove_file(dest.join(rel));
        }
    }

    report.duration_ms = start.elapsed().as_millis();
    Ok(report)
}

/// Walks `root` and pushes file paths (relative to `base`) into `out`.
/// Skips symlinks outright and any directory whose top-level name matches an
/// entry in `exclude`.
fn walk_collect(
    base: &Path,
    current: &Path,
    exclude: &[String],
    out: &mut Vec<PathBuf>,
) -> DanbiResult<()> {
    let rd = match std::fs::read_dir(current) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    for de in rd {
        let de = de?;
        let path = de.path();
        // Never traverse symlinks.
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() {
                continue;
            }
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if exclude.iter().any(|p| p == name) {
            continue;
        }
        if path.is_dir() {
            walk_collect(base, &path, exclude, out)?;
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                // Leave our own stale tmp files out of the "copy me" list.
                if !rel
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.ends_with(".danbi-tmp"))
                    .unwrap_or(false)
                {
                    out.push(rel.to_path_buf());
                }
            }
        }
    }
    Ok(())
}

fn should_skip(src: &Path, tgt: &Path) -> bool {
    let (Ok(sm), Ok(tm)) = (std::fs::metadata(src), std::fs::metadata(tgt)) else {
        return false;
    };
    if sm.len() != tm.len() {
        return false;
    }
    match (sm.modified(), tm.modified()) {
        (Ok(s), Ok(t)) => {
            // Treat equal mtimes as "same" — iCloud / Dropbox preserve mtime
            // on their side so re-runs are O(inspect) instead of O(copy).
            s.duration_since(SystemTime::UNIX_EPOCH).ok()
                == t.duration_since(SystemTime::UNIX_EPOCH).ok()
        }
        _ => false,
    }
}

/// Write `src` to `tgt` atomically: write to `<tgt>.danbi-tmp`, fsync-ish,
/// then rename. Rename on the same filesystem is atomic on POSIX so a
/// reader of `tgt` always sees either the old file or the new one, never
/// a half-written blob.
fn atomic_copy(src: &Path, tgt: &Path) -> DanbiResult<u64> {
    let tmp = tgt.with_extension({
        // `with_extension` replaces the existing extension. We want to ADD
        // `.danbi-tmp` preserving the original extension, so we compute
        // manually here.
        let orig = tgt
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if orig.is_empty() {
            "danbi-tmp".to_string()
        } else {
            format!("{orig}.danbi-tmp")
        }
    });
    let bytes = std::fs::copy(src, &tmp)?;
    // Best-effort: preserve mtime so subsequent `should_skip` works.
    if let Ok(meta) = std::fs::metadata(src) {
        if let Ok(modified) = meta.modified() {
            let _ = filetime::set_file_mtime(
                &tmp,
                filetime::FileTime::from_system_time(modified),
            );
        }
    }
    std::fs::rename(&tmp, tgt)?;
    Ok(bytes)
}
