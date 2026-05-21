use crate::error::DanbiResult;
use crate::links::build_index;
use crate::vault::{list_tree, PROJECTS_DIRNAME};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind")]
pub enum Suggestion {
    /// A markdown file exists but nothing (including itself) references it.
    Orphan {
        project: String,
        domain: String,
    },
    /// The file is still empty long after creation — prompt to fill or delete.
    Empty {
        project: String,
        domain: String,
    },
    /// A project has no domain files.
    EmptyProject { project: String },
    /// A domain is unusually large and could be split.
    Oversized {
        project: String,
        domain: String,
        bytes: u64,
    },
}

const OVERSIZED_BYTES: u64 = 60_000; // ~20k tokens — reasonable review trigger

pub fn scan(vault: &Path) -> DanbiResult<Vec<Suggestion>> {
    let mut out: Vec<Suggestion> = Vec::new();
    let tree = list_tree(vault)?;
    let index = build_index(vault)?;

    for p in &tree.projects {
        // A project is "empty" only when it has neither top-level files nor
        // any subfolder files (e.g. daily notes).
        let total_files = p.domains.len()
            + p.subfolders.iter().map(|s| s.domains.len()).sum::<usize>();
        if total_files == 0 {
            out.push(Suggestion::EmptyProject {
                project: p.name.clone(),
            });
            continue;
        }

        // Flatten top-level + sub-folder docs for uniform checks.
        let mut all: Vec<&crate::vault::DomainNode> = p.domains.iter().collect();
        for s in &p.subfolders {
            for d in &s.domains {
                all.push(d);
            }
        }
        for d in all {
            let key = format!("{}/{}", p.name, d.name);
            let incoming = index.incoming.get(&key).map(|v| v.len()).unwrap_or(0);

            if d.bytes == 0 {
                out.push(Suggestion::Empty {
                    project: p.name.clone(),
                    domain: d.name.clone(),
                });
                continue;
            }
            // Don't flag daily notes as orphans — they're timestamped and rarely cross-linked.
            if incoming == 0 && !is_index_like(&d.name) && !d.name.starts_with("daily/") {
                out.push(Suggestion::Orphan {
                    project: p.name.clone(),
                    domain: d.name.clone(),
                });
            }
            if d.bytes > OVERSIZED_BYTES {
                out.push(Suggestion::Oversized {
                    project: p.name.clone(),
                    domain: d.name.clone(),
                    bytes: d.bytes,
                });
            }
        }
    }
    // Filter out suggestions that sit in hidden/dotfiles or the _assets dir.
    let _ = vault.join(PROJECTS_DIRNAME); // referenced for type safety
    Ok(out)
}

fn is_index_like(name: &str) -> bool {
    // Treat these as "always valid entry points" — no orphan flag.
    matches!(
        name.to_lowercase().as_str(),
        "index.md" | "readme.md" | "overview.md" | "plan.md" | "ui.md"
    )
}
