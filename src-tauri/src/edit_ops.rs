use crate::error::{DanbiError, DanbiResult};
use serde::{Deserialize, Serialize};

/// Operations the Writer model can emit. Rust — not the model — is responsible
/// for turning these into the new document contents.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Append text to the end of the document. A blank line is inserted
    /// between existing content and the new chunk so sections don't collide.
    Append { content: String },

    /// Replace the body under an exact markdown heading with new text.
    /// The heading line itself is preserved. The body is the text from the
    /// heading up to (but not including) the next heading of equal or
    /// shallower depth. If the heading is missing, this behaves like
    /// `insert_after` using the last top-level heading, or `append` if none.
    ReplaceSection {
        heading: String,
        new_body: String,
    },

    /// Insert new text immediately after the given heading's body, before the
    /// next sibling heading.
    InsertAfter {
        heading: String,
        content: String,
    },

    /// Replace the entire file. Use sparingly.
    RewriteAll { content: String },
}

/// Parses a Markdown heading line (`#`, `##`, etc). Returns (depth, title).
fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let depth = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&depth) {
        return None;
    }
    let rest = &trimmed[depth..];
    if !rest.starts_with(' ') {
        return None;
    }
    Some((depth, rest.trim()))
}

/// Locates the `[start, end)` line range of a section identified by its
/// heading text. The range covers the heading line and its body up to (but
/// excluding) the next heading of equal or shallower depth.
fn locate_section(lines: &[&str], heading: &str) -> Option<(usize, usize)> {
    let target = heading.trim_start_matches('#').trim();
    let mut found: Option<(usize, usize)> = None; // (start, depth)
    for (i, line) in lines.iter().enumerate() {
        if let Some((depth, title)) = parse_heading(line) {
            if title == target {
                found = Some((i, depth));
                break;
            }
        }
    }
    let (start, depth) = found?;
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if let Some((d, _)) = parse_heading(line) {
            if d <= depth {
                end = i;
                break;
            }
        }
    }
    Some((start, end))
}

fn ensure_trailing_newline(s: &mut String) {
    if !s.ends_with('\n') {
        s.push('\n');
    }
}

pub fn apply(doc: &str, op: &EditOp) -> DanbiResult<String> {
    match op {
        EditOp::RewriteAll { content } => {
            let mut out = content.clone();
            ensure_trailing_newline(&mut out);
            Ok(out)
        }
        EditOp::Append { content } => {
            let mut out = String::with_capacity(doc.len() + content.len() + 2);
            out.push_str(doc);
            if !doc.is_empty() && !doc.ends_with('\n') {
                out.push('\n');
            }
            if !doc.trim_end().is_empty() {
                out.push('\n');
            }
            out.push_str(content);
            ensure_trailing_newline(&mut out);
            Ok(out)
        }
        EditOp::ReplaceSection { heading, new_body } => {
            let lines: Vec<&str> = doc.split_inclusive('\n').collect();
            // split_inclusive keeps the trailing '\n'; locate_section below
            // expects lines without them for the heading match.
            let clean_lines: Vec<&str> = lines
                .iter()
                .map(|s| s.trim_end_matches('\n'))
                .collect();

            match locate_section(&clean_lines, heading) {
                Some((start, end)) => {
                    let mut out = String::new();
                    for l in &lines[..=start] {
                        out.push_str(l);
                    }
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    let mut body = new_body.trim_end_matches('\n').to_string();
                    body.push('\n');
                    out.push_str(&body);
                    for l in &lines[end..] {
                        out.push_str(l);
                    }
                    ensure_trailing_newline(&mut out);
                    Ok(out)
                }
                None => apply(doc, &EditOp::Append {
                    content: format!("{}\n\n{}", heading, new_body.trim()),
                }),
            }
        }
        EditOp::InsertAfter { heading, content } => {
            let lines: Vec<&str> = doc.split_inclusive('\n').collect();
            let clean_lines: Vec<&str> = lines
                .iter()
                .map(|s| s.trim_end_matches('\n'))
                .collect();
            match locate_section(&clean_lines, heading) {
                Some((_, end)) => {
                    let mut out = String::new();
                    for l in &lines[..end] {
                        out.push_str(l);
                    }
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    let mut chunk = content.trim_end_matches('\n').to_string();
                    chunk.push('\n');
                    out.push_str(&chunk);
                    for l in &lines[end..] {
                        out.push_str(l);
                    }
                    ensure_trailing_newline(&mut out);
                    Ok(out)
                }
                None => apply(doc, &EditOp::Append {
                    content: content.clone(),
                }),
            }
        }
    }
}

/// Validates a user-supplied op minimally. Writer may emit garbage; we refuse
/// empty content to avoid wiping files by accident.
pub fn validate(op: &EditOp) -> DanbiResult<()> {
    match op {
        EditOp::Append { content } | EditOp::InsertAfter { content, .. } => {
            if content.trim().is_empty() {
                return Err(DanbiError::Other("content is empty".into()));
            }
        }
        EditOp::ReplaceSection { heading, new_body } => {
            if heading.trim().is_empty() {
                return Err(DanbiError::Other("heading is empty".into()));
            }
            if new_body.trim().is_empty() {
                return Err(DanbiError::Other("new_body is empty".into()));
            }
        }
        EditOp::RewriteAll { content } => {
            if content.trim().is_empty() {
                return Err(DanbiError::Other("content is empty".into()));
            }
        }
    }
    Ok(())
}
