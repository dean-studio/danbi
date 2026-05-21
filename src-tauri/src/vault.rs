use crate::error::{DanbiError, DanbiResult};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const PROJECTS_DIRNAME: &str = "Projects";
pub const LOG_FILENAME: &str = "log.md";
pub const HISTORY_FILENAME: &str = "history.jsonl";
const MD_EXT: &str = ".md";

fn is_safe_segment(s: &str) -> bool {
    if s.is_empty() || s == "." || s == ".." {
        return false;
    }
    !s.chars().any(|c| {
        c == '/'
            || c == '\\'
            || c == '\0'
            || c == '\n'
            || c == '\r'
    })
}

/// Domain names may include up to TWO levels of subdirectory:
///   - "ui.md"
///   - "daily/2026-05-11.md"
///   - "daily/2026-05/01.md"
/// Each segment must pass `is_safe_segment`. No ".." allowed. Anything
/// deeper is rejected — keeps the sidebar navigable and the indexer happy.
const MAX_DOMAIN_DEPTH: usize = 3;

fn normalize_domain(name: &str) -> DanbiResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(DanbiError::Config("empty domain name".into()));
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() > MAX_DOMAIN_DEPTH {
        return Err(DanbiError::Config(format!(
            "domain path is too deep (max {} segments): {trimmed}",
            MAX_DOMAIN_DEPTH,
        )));
    }
    for part in &parts {
        if !is_safe_segment(part) {
            return Err(DanbiError::Config(format!(
                "unsafe domain segment: {part}",
            )));
        }
    }
    let filename_part = parts.last().unwrap();
    let with_ext = if filename_part.to_lowercase().ends_with(MD_EXT) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{MD_EXT}")
    };
    Ok(with_ext)
}

fn projects_root(vault: &Path) -> PathBuf {
    vault.join(PROJECTS_DIRNAME)
}

fn project_path(vault: &Path, project: &str) -> DanbiResult<PathBuf> {
    if !is_safe_segment(project) {
        return Err(DanbiError::Config(format!("unsafe project name: {project}")));
    }
    Ok(projects_root(vault).join(project))
}

fn domain_path(vault: &Path, project: &str, domain: &str) -> DanbiResult<PathBuf> {
    let file = normalize_domain(domain)?;
    Ok(project_path(vault, project)?.join(file))
}

pub fn init_vault(vault: &Path) -> DanbiResult<()> {
    std::fs::create_dir_all(vault)?;
    std::fs::create_dir_all(projects_root(vault))?;

    let log = vault.join(LOG_FILENAME);
    if !log.exists() {
        std::fs::write(
            &log,
            "# Danbi Log\n\n단비가 기록하는 작업 타임라인입니다.\n",
        )?;
    }
    let hist = vault.join(HISTORY_FILENAME);
    if !hist.exists() {
        std::fs::write(&hist, "")?;
    }

    // Initialize git repository so every subsequent edit is undoable.
    crate::vcs::ensure_repo(vault)?;
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct DomainNode {
    /// Full path relative to the project folder, e.g. "ui.md" or "daily/2026-05-11.md".
    pub name: String,
    pub bytes: u64,
    pub modified_ms: Option<u128>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SubfolderNode {
    /// Folder name relative to the project root. Top-level subfolders use
    /// just the segment ("daily"). Nested subfolders use the joined path
    /// ("daily/2026-01-05") so the frontend can address them uniquely.
    pub name: String,
    pub domains: Vec<DomainNode>,
    /// One more level of nesting. Currently capped at depth 2 — that is,
    /// the children themselves do NOT carry further `subfolders`. We keep
    /// this as a Vec for forward-compat in case we ever lift the cap, but
    /// every current code path treats it as terminal.
    #[serde(default)]
    pub subfolders: Vec<SubfolderNode>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProjectNode {
    pub name: String,
    /// Top-level .md files in the project folder.
    pub domains: Vec<DomainNode>,
    /// One-level subfolders with their markdown files (daily/, notes/, …).
    /// Hidden folders (`_assets`, names starting with `.`) are excluded.
    #[serde(default)]
    pub subfolders: Vec<SubfolderNode>,
}

#[derive(Debug, Serialize, Clone)]
pub struct VaultTree {
    pub vault_path: String,
    pub projects: Vec<ProjectNode>,
}

fn mtime_ms(meta: &std::fs::Metadata) -> Option<u128> {
    let m = meta.modified().ok()?;
    let dur = m.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis())
}

/// Subfolders that are managed internally and should stay hidden from the
/// sidebar (user can still see them in Finder).
fn is_hidden_folder(name: &str) -> bool {
    name.starts_with('.') || name == ASSETS_DIRNAME
}

fn read_md_domains(dir: &Path, prefix: Option<&str>) -> DanbiResult<Vec<DomainNode>> {
    let mut out: Vec<DomainNode> = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    for de in rd {
        let de = de?;
        let path = de.path();
        if !path.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !fname.to_lowercase().ends_with(MD_EXT) || fname.starts_with('.') {
            continue;
        }
        let meta = de.metadata()?;
        let name = match prefix {
            Some(p) => format!("{p}/{fname}"),
            None => fname.to_string(),
        };
        out.push(DomainNode {
            name,
            bytes: meta.len(),
            modified_ms: mtime_ms(&meta),
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

pub fn list_tree(vault: &Path) -> DanbiResult<VaultTree> {
    init_vault(vault)?;
    let root = projects_root(vault);

    let mut projects: Vec<ProjectNode> = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let project_name = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };

        // Top-level .md files.
        let domains = read_md_domains(&p, None)?;
        let subfolders = read_subfolders(&p, None)?;

        projects.push(ProjectNode {
            name: project_name,
            domains,
            subfolders,
        });
    }
    projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(VaultTree {
        vault_path: vault.to_string_lossy().to_string(),
        projects,
    })
}

/// Walk one level of subfolders under `dir`. `parent_prefix` is what we
/// prepend to each subfolder's `name` so the frontend gets a fully-qualified
/// path it can pass back ("daily" or "daily/2026-01"). Set to `None` at the
/// project root, `Some("daily")` when we recurse into `daily/`.
///
/// Stops recursing after `MAX_DOMAIN_DEPTH - 1` levels (currently 2 folder
/// levels under a project root, since the third path segment is reserved
/// for the .md file). Hidden / `_assets` folders are skipped.
fn read_subfolders(dir: &Path, parent_prefix: Option<&str>) -> DanbiResult<Vec<SubfolderNode>> {
    let depth_so_far = parent_prefix
        .map(|p| p.split('/').count())
        .unwrap_or(0);
    // Each .md sits at depth `n+1`, so folders themselves can go up to
    // `MAX_DOMAIN_DEPTH - 1`.
    let allow_recurse = depth_so_far + 1 < MAX_DOMAIN_DEPTH - 1;

    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out: Vec<SubfolderNode> = Vec::new();
    for de in rd {
        let de = de?;
        let sub = de.path();
        if !sub.is_dir() {
            continue;
        }
        let Some(sname) = sub.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_hidden_folder(sname) {
            continue;
        }
        let qualified = match parent_prefix {
            Some(p) => format!("{p}/{sname}"),
            None => sname.to_string(),
        };
        let domains = read_md_domains(&sub, Some(&qualified))?;
        let nested = if allow_recurse {
            read_subfolders(&sub, Some(&qualified))?
        } else {
            Vec::new()
        };
        out.push(SubfolderNode {
            name: qualified,
            domains,
            subfolders: nested,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

pub fn create_project(vault: &Path, name: &str, default_domains: &[String]) -> DanbiResult<()> {
    create_project_with_folders(vault, name, default_domains, &[])
}

/// Creates a project, its optional starter domain files, and any declared
/// sub-folders (e.g. "daily", "notes"). Sub-folders are created empty; files
/// inside them are populated on demand (daily notes, compound outputs, etc.).
pub fn create_project_with_folders(
    vault: &Path,
    name: &str,
    default_domains: &[String],
    folders: &[String],
) -> DanbiResult<()> {
    let dir = project_path(vault, name)?;
    if dir.exists() {
        return Err(DanbiError::Config(format!("project already exists: {name}")));
    }
    std::fs::create_dir_all(&dir)?;
    for d in default_domains {
        let rel = normalize_domain(d)?;
        let file = dir.join(&rel);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !file.exists() {
            std::fs::write(&file, "")?;
        }
    }
    for folder in folders {
        let trimmed = folder.trim().trim_matches('/');
        if trimmed.is_empty() || !is_safe_segment(trimmed) {
            continue;
        }
        let sub = dir.join(trimmed);
        std::fs::create_dir_all(&sub)?;
    }
    // Seed purpose.md and schema.md so the Wiki-LLM loop has grounding
    // material from day one. Idempotent on re-create.
    let _ = crate::project_context::ensure_templates(vault, name);
    Ok(())
}

pub fn rename_project(vault: &Path, old: &str, new: &str) -> DanbiResult<()> {
    let from = project_path(vault, old)?;
    let to = project_path(vault, new)?;
    if !from.exists() {
        return Err(DanbiError::Config(format!("project not found: {old}")));
    }
    if to.exists() {
        return Err(DanbiError::Config(format!("target exists: {new}")));
    }
    std::fs::rename(from, to)?;
    Ok(())
}

pub fn delete_project(vault: &Path, name: &str) -> DanbiResult<()> {
    let dir = project_path(vault, name)?;
    if !dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

pub fn create_domain(vault: &Path, project: &str, domain: &str) -> DanbiResult<String> {
    let proj_dir = project_path(vault, project)?;
    if !proj_dir.exists() {
        return Err(DanbiError::Config(format!("project not found: {project}")));
    }
    let file = normalize_domain(domain)?;
    let path = proj_dir.join(&file);
    if path.exists() {
        return Err(DanbiError::Config(format!(
            "domain already exists: {project}/{file}"
        )));
    }
    // Ensure any nested folder prefix exists ("daily/2026-01/05.md" needs
    // both `daily/` and `daily/2026-01/`). idempotent if already present.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, "")?;
    Ok(file)
}

pub fn rename_domain(vault: &Path, project: &str, old: &str, new: &str) -> DanbiResult<String> {
    let from = domain_path(vault, project, old)?;
    let new_file = normalize_domain(new)?;
    let to = project_path(vault, project)?.join(&new_file);
    if !from.exists() {
        return Err(DanbiError::Config(format!(
            "domain not found: {project}/{old}"
        )));
    }
    if to.exists() {
        return Err(DanbiError::Config(format!(
            "target exists: {project}/{new_file}"
        )));
    }
    std::fs::rename(from, to)?;
    Ok(new_file)
}

pub fn delete_domain(vault: &Path, project: &str, domain: &str) -> DanbiResult<()> {
    let path = domain_path(vault, project, domain)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Validate a folder path that may include up to one nested level
/// (`parent/child`). Returns the canonical, slash-joined path with leading
/// / trailing slashes stripped. Each segment must be a safe filesystem
/// segment and must not be hidden.
fn normalize_folder(folder: &str) -> DanbiResult<String> {
    let trimmed = folder.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err(DanbiError::Config("empty folder name".into()));
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    // Folders themselves can be at most `MAX_DOMAIN_DEPTH - 1` levels deep —
    // the deepest segment is always the .md filename.
    if parts.len() >= MAX_DOMAIN_DEPTH {
        return Err(DanbiError::Config(format!(
            "folder path is too deep (max {} segments): {trimmed}",
            MAX_DOMAIN_DEPTH - 1,
        )));
    }
    for part in &parts {
        if !is_safe_segment(part) || is_hidden_folder(part) {
            return Err(DanbiError::Config(format!(
                "unsafe or hidden folder segment: {part}",
            )));
        }
    }
    Ok(parts.join("/"))
}

/// Create a sub-folder under a project. Accepts up to two levels
/// (e.g. `daily`, `daily/2026-01`). Idempotent — Ok if it already exists.
pub fn create_folder(vault: &Path, project: &str, folder: &str) -> DanbiResult<()> {
    let canonical = normalize_folder(folder)?;
    let proj_dir = project_path(vault, project)?;
    if !proj_dir.exists() {
        return Err(DanbiError::Config(format!("project not found: {project}")));
    }
    let path = proj_dir.join(&canonical);
    if path.exists() {
        if path.is_file() {
            return Err(DanbiError::Config(format!(
                "name conflicts with existing file: {project}/{canonical}"
            )));
        }
        return Ok(());
    }
    std::fs::create_dir_all(&path)?;
    Ok(())
}

/// Rename a sub-folder in place. Both `old` and `new` carry the full path
/// from the project root (e.g. `daily/2026-01`). Renaming across levels
/// (e.g. moving from a top-level folder to a nested one) is not supported
/// — the segment count of `old` and `new` must match.
pub fn rename_folder(
    vault: &Path,
    project: &str,
    old: &str,
    new: &str,
) -> DanbiResult<()> {
    let old_canon = normalize_folder(old)?;
    let new_canon = normalize_folder(new)?;
    if old_canon.split('/').count() != new_canon.split('/').count() {
        return Err(DanbiError::Config(
            "rename across folder levels is not supported".into(),
        ));
    }
    let proj_dir = project_path(vault, project)?;
    let from = proj_dir.join(&old_canon);
    let to = proj_dir.join(&new_canon);
    if !from.is_dir() {
        return Err(DanbiError::Config(format!(
            "folder not found: {project}/{old_canon}"
        )));
    }
    if to.exists() {
        return Err(DanbiError::Config(format!(
            "target exists: {project}/{new_canon}"
        )));
    }
    std::fs::rename(from, to)?;
    Ok(())
}

/// Recursively delete a sub-folder. Caller is expected to confirm with
/// the user — destructive.
pub fn delete_folder(vault: &Path, project: &str, folder: &str) -> DanbiResult<()> {
    let canonical = normalize_folder(folder)?;
    let path = project_path(vault, project)?.join(&canonical);
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Move a markdown file between a project's top-level and one of its
/// sub-folders (or between sub-folders). `from` and `to` follow the
/// `domain` convention: bare filename for top-level, `<folder>/<file>.md`
/// for a sub-folder. Returns the new domain name on success.
pub fn move_domain(
    vault: &Path,
    project: &str,
    from: &str,
    to_folder: Option<&str>,
) -> DanbiResult<String> {
    let from_path = domain_path(vault, project, from)?;
    if !from_path.is_file() {
        return Err(DanbiError::Config(format!(
            "source not found: {project}/{from}"
        )));
    }
    // The filename part stays the same — only the folder changes.
    let filename = from_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| DanbiError::Config("invalid source path".into()))?
        .to_string();

    let proj_dir = project_path(vault, project)?;
    // Resolve `to_folder` (which may be nested like "daily/2026-01") into a
    // canonical path string + concrete dir path. Empty / None drops the
    // file back to the project root.
    let canonical_folder: Option<String> = match to_folder {
        Some(f) if !f.trim().trim_matches('/').is_empty() => {
            Some(normalize_folder(f)?)
        }
        _ => None,
    };
    let target_dir = match &canonical_folder {
        Some(canonical) => {
            let dir = proj_dir.join(canonical);
            if !dir.exists() {
                std::fs::create_dir_all(&dir)?;
            }
            if !dir.is_dir() {
                return Err(DanbiError::Config(format!(
                    "destination is not a folder: {canonical}"
                )));
            }
            dir
        }
        None => proj_dir,
    };
    let to_path = target_dir.join(&filename);
    if to_path == from_path {
        let new_domain = match &canonical_folder {
            Some(f) => format!("{f}/{filename}"),
            None => filename,
        };
        return Ok(new_domain);
    }
    if to_path.exists() {
        return Err(DanbiError::Config(format!(
            "target exists: {project}/{}",
            to_path
                .strip_prefix(project_path(vault, project)?)
                .unwrap_or(&to_path)
                .display()
        )));
    }
    std::fs::rename(&from_path, &to_path)?;
    Ok(match canonical_folder {
        Some(f) => format!("{f}/{filename}"),
        None => filename,
    })
}

pub fn read_doc(vault: &Path, project: &str, domain: &str) -> DanbiResult<String> {
    let path = domain_path(vault, project, domain)?;
    if !path.exists() {
        return Ok(String::new());
    }
    Ok(std::fs::read_to_string(path)?)
}

pub fn write_doc(vault: &Path, project: &str, domain: &str, content: &str) -> DanbiResult<()> {
    let path = domain_path(vault, project, domain)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

pub const ASSETS_DIRNAME: &str = "_assets";
pub const PROJECT_ID_FILENAME: &str = ".danbi-id";

/// Returns a stable UUID for the project, creating one on first access.
/// Stored in `<project>/.danbi-id` so it survives project rename.
pub fn ensure_project_id(vault: &Path, project: &str) -> DanbiResult<String> {
    let proj_dir = project_path(vault, project)?;
    if !proj_dir.exists() {
        return Err(DanbiError::Config(format!("project not found: {project}")));
    }
    let id_path = proj_dir.join(PROJECT_ID_FILENAME);
    if let Ok(existing) = std::fs::read_to_string(&id_path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    // Lightweight UUID v4 — we don't need cryptographic quality here, just
    // uniqueness across a user's vault. 128 random bits formatted RFC-4122-ish.
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant RFC4122
    let id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    std::fs::write(&id_path, &id)?;
    Ok(id)
}

pub const SKILL_FILENAME: &str = "SKILL.md";

/// Returns the path to `<project>/SKILL.md`, seeding it from the default
/// template the first time so the user has something concrete to edit.
/// The seeded file keeps `{{PROJECT}}` / `{{MCP_URL}}` placeholders —
/// substitution happens at install time so one customised skill can
/// produce different `~/.claude/skills/danbi-<slug>/SKILL.md` for each
/// project.
pub fn ensure_project_skill(vault: &Path, project: &str) -> DanbiResult<PathBuf> {
    let proj_dir = project_path(vault, project)?;
    if !proj_dir.exists() {
        return Err(DanbiError::Config(format!("project not found: {project}")));
    }
    let skill_path = proj_dir.join(SKILL_FILENAME);
    if !skill_path.exists() {
        std::fs::write(&skill_path, crate::skill::DEFAULT_SKILL_TEMPLATE)?;
    }
    Ok(skill_path)
}

/// Looks up which project owns the given UUID. Walks the vault once.
pub fn project_by_id(vault: &Path, id: &str) -> DanbiResult<Option<String>> {
    let root = projects_root(vault);
    if !root.exists() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let id_path = p.join(PROJECT_ID_FILENAME);
        if let Ok(content) = std::fs::read_to_string(&id_path) {
            if content.trim() == id {
                return Ok(Some(name.to_string()));
            }
        }
    }
    Ok(None)
}

fn sanitize_filename(name: &str) -> String {
    // Keep unicode letters/digits, collapse everything else to '-'.
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(|c: char| c == '-' || c == '.').to_string();
    if trimmed.is_empty() {
        "file".into()
    } else {
        trimmed
    }
}

/// Saves raw bytes as an asset for the given project. Ensures no path-escape
/// and resolves filename collisions by appending a numeric suffix.
/// Returns the path relative to the project folder (e.g. "_assets/foo.png").
pub fn save_asset(
    vault: &Path,
    project: &str,
    original_filename: &str,
    bytes: &[u8],
) -> DanbiResult<String> {
    let proj_dir = project_path(vault, project)?;
    if !proj_dir.exists() {
        return Err(DanbiError::Config(format!("project not found: {project}")));
    }
    let assets_dir = proj_dir.join(ASSETS_DIRNAME);
    std::fs::create_dir_all(&assets_dir)?;

    let safe = sanitize_filename(original_filename);
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((s, e)) if !e.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (safe.clone(), String::new()),
    };

    let mut candidate = format!("{stem}{ext}");
    let mut n = 1;
    while assets_dir.join(&candidate).exists() {
        candidate = format!("{stem}-{n}{ext}");
        n += 1;
    }

    let final_path = assets_dir.join(&candidate);
    std::fs::write(&final_path, bytes)?;
    Ok(format!("{ASSETS_DIRNAME}/{candidate}"))
}

/// Resolves a relative asset path (as emitted by save_asset) into an absolute
/// filesystem path. Validates the resolved path stays inside the vault.
pub fn resolve_asset_absolute(
    vault: &Path,
    project: &str,
    rel_path: &str,
) -> DanbiResult<std::path::PathBuf> {
    if rel_path.contains("..") {
        return Err(DanbiError::Config("unsafe asset path".into()));
    }
    let base = project_path(vault, project)?;
    let abs = base.join(rel_path);
    let canon_base = base
        .canonicalize()
        .map_err(|e| DanbiError::Other(format!("canonicalize base: {e}")))?;
    if let Ok(canon_abs) = abs.canonicalize() {
        if !canon_abs.starts_with(&canon_base) {
            return Err(DanbiError::Config("asset escapes project root".into()));
        }
        return Ok(canon_abs);
    }
    // Fall back to the joined path if it doesn't exist yet — still validated via canonicalized base.
    Ok(abs)
}
