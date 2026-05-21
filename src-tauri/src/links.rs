use crate::error::DanbiResult;
use crate::vault::PROJECTS_DIRNAME;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct WikiLink {
    pub project: String,
    pub domain: String,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct LinkIndex {
    /// Forward: "project/domain" -> outgoing targets.
    pub outgoing: HashMap<String, Vec<WikiLink>>,
    /// Reverse: "project/domain" -> incoming sources.
    pub incoming: HashMap<String, Vec<WikiLink>>,
}

fn key(project: &str, domain: &str) -> String {
    format!("{project}/{domain}")
}

/// Parses wiki links from a single markdown blob. Supported syntaxes:
///   [[Project/domain.md]]
///   [[Project/domain]]
///   [[domain.md]]        (within-project, uses fallback_project)
pub fn extract_links(md: &str, fallback_project: &str) -> Vec<WikiLink> {
    let mut out = Vec::new();
    let bytes = md.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(end) = find_double_close(&bytes[i + 2..]) {
                let inner_bytes = &bytes[i + 2..i + 2 + end];
                if let Ok(inner) = std::str::from_utf8(inner_bytes) {
                    if let Some(link) = parse_link(inner, fallback_project) {
                        out.push(link);
                    }
                }
                i += 2 + end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn find_double_close(tail: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < tail.len() {
        if tail[i] == b']' && tail[i + 1] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_link(inner: &str, fallback_project: &str) -> Option<WikiLink> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Support "Proj/file" and bare "file" forms. Pipe aliasing ("[[X|alias]]")
    // is also tolerated — the portion before '|' is the target.
    let target = trimmed.split('|').next()?.trim();
    let (project, domain) = match target.split_once('/') {
        Some((p, d)) => (p.trim().to_string(), d.trim().to_string()),
        None => (fallback_project.to_string(), target.to_string()),
    };
    if project.is_empty() || domain.is_empty() {
        return None;
    }
    let domain = if domain.to_lowercase().ends_with(".md") {
        domain
    } else {
        format!("{domain}.md")
    };
    Some(WikiLink { project, domain })
}

/// Walks the whole vault, extracts wiki links from every .md under every
/// project, and builds a bidirectional index.
pub fn build_index(vault: &Path) -> DanbiResult<LinkIndex> {
    let mut index = LinkIndex::default();
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(index);
    }
    for entry in std::fs::read_dir(&projects_root)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let project = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let mut md_entries: Vec<(String, std::path::PathBuf)> = Vec::new();
        walk_md_for_links(&p, None, &mut md_entries)?;
        for de in std::fs::read_dir(&p)? {
            let de = de?;
            let sub = de.path();
            if !sub.is_dir() {
                continue;
            }
            let Some(sname) = sub.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if sname.starts_with('.') || sname == "_assets" {
                continue;
            }
            walk_md_for_links(&sub, Some(sname), &mut md_entries)?;
        }

        for (fname, path) in md_entries {
            let Ok(md) = std::fs::read_to_string(&path) else {
                continue;
            };
            let outgoing = extract_links(&md, &project);
            let src = key(&project, &fname);
            for target in &outgoing {
                index
                    .incoming
                    .entry(key(&target.project, &target.domain))
                    .or_default()
                    .push(WikiLink {
                        project: project.clone(),
                        domain: fname.clone(),
                    });
            }
            index.outgoing.insert(src, outgoing);
        }
    }
    Ok(index)
}

fn walk_md_for_links(
    dir: &Path,
    prefix: Option<&str>,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> DanbiResult<()> {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
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
        if !fname.to_lowercase().ends_with(".md") || fname.starts_with('.') {
            continue;
        }
        let full = match prefix {
            Some(p) => format!("{p}/{fname}"),
            None => fname.to_string(),
        };
        out.push((full, path));
    }
    Ok(())
}
