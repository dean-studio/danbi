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

    /// Upsert a list item under a heading. If an existing item with the same
    /// `key` (matched by `[#id]` marker first, then by the first non-blank
    /// line of the item) lives in the section, replace it in place. Otherwise
    /// append the new item to the section. The section is identified by its
    /// markdown heading text (any depth) — same matching rule as
    /// `ReplaceSection`. If the heading is missing, the entire section is
    /// created at EOF.
    UpsertItem {
        heading: String,
        key: String,
        item: String,
    },
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
        EditOp::UpsertItem {
            heading,
            key,
            item,
        } => apply_upsert_item(doc, heading, key, item),
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
        EditOp::UpsertItem { heading, key, item } => {
            if heading.trim().is_empty() {
                return Err(DanbiError::Other("heading is empty".into()));
            }
            if key.trim().is_empty() {
                return Err(DanbiError::Other("key is empty".into()));
            }
            if item.trim().is_empty() {
                return Err(DanbiError::Other("item is empty".into()));
            }
        }
    }
    Ok(())
}

// ---------- UpsertItem helpers ----------

/// Top-level bullet recognizer. We treat lines starting with `- `, `* `, or
/// `+ ` (with optional indentation up to 3 spaces — CommonMark) as the start
/// of a list item. Everything until the next top-level bullet OR a blank
/// line followed by a non-list line OR a heading belongs to the same item.
fn is_list_bullet(line: &str) -> bool {
    let trimmed = line.trim_start_matches(|c: char| c == ' ');
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return false;
    }
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    // numbered list: "1. " or "12) "
    let mut chars = trimmed.chars();
    let mut saw_digit = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        if !saw_digit {
            return false;
        }
        if c == '.' || c == ')' {
            return chars.next() == Some(' ');
        }
        return false;
    }
    false
}

/// Extract the leading `[#id]` marker from an item's first line, if any.
fn extract_id_marker(item_first_line: &str) -> Option<&str> {
    // Strip bullet/number prefix first.
    let trimmed = item_first_line.trim_start_matches(|c: char| c == ' ');
    let body = trimmed
        .trim_start_matches(['-', '*', '+'])
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(['.', ')'])
        .trim_start();
    let bytes = body.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }
    let hash_pos = body.find('#')?;
    if hash_pos != 1 {
        return None;
    }
    let close = body.find(']')?;
    if close <= hash_pos + 1 {
        return None;
    }
    Some(&body[hash_pos + 1..close])
}

/// Strip an item's textual signature for fuzzy matching: lowercase, drop
/// `[#id]` marker, drop bullet prefix, collapse whitespace.
fn item_signature(item_first_line: &str) -> String {
    let trimmed = item_first_line.trim_start_matches(|c: char| c == ' ');
    let no_bullet = trimmed
        .trim_start_matches(['-', '*', '+'])
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(['.', ')'])
        .trim_start();
    // Drop [#id] marker if present.
    let after_marker = if no_bullet.starts_with("[#") {
        if let Some(close) = no_bullet.find(']') {
            no_bullet[close + 1..].trim_start()
        } else {
            no_bullet
        }
    } else {
        no_bullet
    };
    after_marker
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a section body (lines from just after the heading to just before the
/// next sibling/parent heading) into individual list-item ranges, expressed
/// as `(start_idx, end_idx)` pairs over the input slice. Non-list lines
/// between items belong to the preceding item.
fn split_into_items(body: &[&str]) -> Vec<(usize, usize)> {
    let mut items: Vec<(usize, usize)> = Vec::new();
    let mut cur_start: Option<usize> = None;
    for (i, line) in body.iter().enumerate() {
        if is_list_bullet(line) {
            if let Some(s) = cur_start {
                items.push((s, i));
            }
            cur_start = Some(i);
        }
    }
    if let Some(s) = cur_start {
        items.push((s, body.len()));
    }
    items
}

/// Pre-flight inspector: would `UpsertItem { heading, key, .. }` replace an
/// existing item or just add a new one? Lets callers report `update` vs
/// `add` to humans without re-running the diff.
pub fn upsert_item_would_replace(doc: &str, heading: &str, key: &str) -> bool {
    let lines: Vec<&str> = doc.split_inclusive('\n').collect();
    let clean_lines: Vec<&str> = lines.iter().map(|s| s.trim_end_matches('\n')).collect();
    let Some((start, end)) = locate_section(&clean_lines, heading) else {
        return false;
    };
    let body = &clean_lines[start + 1..end];
    let mut leading_blanks = 0;
    while leading_blanks < body.len() && body[leading_blanks].trim().is_empty() {
        leading_blanks += 1;
    }
    let body_items = &body[leading_blanks..];
    let items = split_into_items(body_items);
    let target_key = key.trim();
    let target_id_only = target_key.trim_start_matches("[#").trim_end_matches(']');
    let target_sig = item_signature(target_key);
    for (s, _e) in &items {
        let first = body_items[*s];
        if let Some(id) = extract_id_marker(first) {
            if id == target_id_only {
                return true;
            }
        }
    }
    if target_sig.is_empty() {
        return false;
    }
    items
        .iter()
        .any(|(s, _)| item_signature(body_items[*s]) == target_sig)
}

fn apply_upsert_item(
    doc: &str,
    heading: &str,
    key: &str,
    item: &str,
) -> DanbiResult<String> {
    let lines: Vec<&str> = doc.split_inclusive('\n').collect();
    let clean_lines: Vec<&str> = lines.iter().map(|s| s.trim_end_matches('\n')).collect();

    // Section missing → create it at EOF and put this single item under it.
    let Some((start, end)) = locate_section(&clean_lines, heading) else {
        let appended = format!(
            "{heading}\n\n{item}",
            item = item.trim_end_matches('\n')
        );
        return apply(doc, &EditOp::Append { content: appended });
    };

    // Body of the section = lines (start+1..end). Heading line stays put.
    let body = &clean_lines[start + 1..end];

    // Strip any leading blank lines from the body so the "items" list starts
    // at the first content line — but remember the count to preserve spacing.
    let mut leading_blanks = 0;
    while leading_blanks < body.len() && body[leading_blanks].trim().is_empty() {
        leading_blanks += 1;
    }
    let body_items = &body[leading_blanks..];
    let items = split_into_items(body_items);

    // Find the item to replace. Priority: [#id] marker exact match; else
    // signature fuzzy match.
    let target_key = key.trim();
    let target_id_only = target_key.trim_start_matches("[#").trim_end_matches(']');
    let target_sig = item_signature(target_key);

    let mut hit: Option<usize> = None;
    for (idx, (s, _e)) in items.iter().enumerate() {
        let first = body_items[*s];
        if let Some(id) = extract_id_marker(first) {
            if id == target_id_only {
                hit = Some(idx);
                break;
            }
        }
    }
    if hit.is_none() && !target_sig.is_empty() {
        for (idx, (s, _e)) in items.iter().enumerate() {
            let sig = item_signature(body_items[*s]);
            if sig == target_sig {
                hit = Some(idx);
                break;
            }
        }
    }

    let item_block = item.trim_end_matches('\n').to_string();

    let mut out = String::new();
    // Re-emit lines up to and including the heading.
    for l in &lines[..=start] {
        out.push_str(l);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }

    // Re-emit leading blanks of the body verbatim.
    for l in &lines[start + 1..start + 1 + leading_blanks] {
        out.push_str(l);
    }

    let body_items_offset = start + 1 + leading_blanks;
    match hit {
        Some(replace_idx) => {
            // Re-emit items before the hit.
            let (hs, he) = items[replace_idx];
            for l in &lines[body_items_offset..body_items_offset + hs] {
                out.push_str(l);
            }
            // Replace the hit's lines with the new item block.
            out.push_str(&item_block);
            if !item_block.ends_with('\n') {
                out.push('\n');
            }
            // Re-emit items after the hit.
            for l in &lines[body_items_offset + he..end] {
                out.push_str(l);
            }
        }
        None => {
            // Re-emit all body items.
            for l in &lines[body_items_offset..end] {
                out.push_str(l);
            }
            // Ensure a blank line between the previous content and the new
            // item if the body didn't already end with one.
            if !out.ends_with("\n\n") {
                if out.ends_with('\n') {
                    out.push('\n');
                } else {
                    out.push_str("\n\n");
                }
            }
            out.push_str(&item_block);
            if !item_block.ends_with('\n') {
                out.push('\n');
            }
        }
    }

    // Re-emit anything after the section.
    for l in &lines[end..] {
        out.push_str(l);
    }

    ensure_trailing_newline(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_section_basic() {
        let doc = "# Top\n\nintro\n\n## A\n\nold body\n\n## B\n\nstays\n";
        let out = apply(
            doc,
            &EditOp::ReplaceSection {
                heading: "## A".into(),
                new_body: "fresh".into(),
            },
        )
        .unwrap();
        assert!(out.contains("## A\nfresh\n"));
        assert!(out.contains("## B\n\nstays\n"));
        assert!(!out.contains("old body"));
    }

    #[test]
    fn upsert_item_replaces_by_id_marker() {
        let doc = "## 알림톡 리스트\n\n- [#a] 결제 완료\n- [#b] 장바구니 미완료\n";
        let out = apply(
            doc,
            &EditOp::UpsertItem {
                heading: "## 알림톡 리스트".into(),
                key: "a".into(),
                item: "- [#a] 결제 완료 (수정됨)".into(),
            },
        )
        .unwrap();
        assert!(out.contains("- [#a] 결제 완료 (수정됨)\n"));
        assert!(out.contains("- [#b] 장바구니 미완료\n"));
        assert!(!out.contains("- [#a] 결제 완료\n"));
    }

    #[test]
    fn upsert_item_replaces_by_signature_fallback() {
        let doc = "## list\n\n- 결제 완료\n- 장바구니 미완료\n";
        let out = apply(
            doc,
            &EditOp::UpsertItem {
                heading: "## list".into(),
                key: "결제 완료".into(),
                item: "- 결제 완료 — body 변경".into(),
            },
        )
        .unwrap();
        assert!(out.contains("- 결제 완료 — body 변경\n"));
        assert!(!out.contains("- 결제 완료\n- 장바구니"));
    }

    #[test]
    fn upsert_item_appends_when_no_match() {
        let doc = "## list\n\n- 기존 항목\n";
        let out = apply(
            doc,
            &EditOp::UpsertItem {
                heading: "## list".into(),
                key: "신규".into(),
                item: "- 신규 항목".into(),
            },
        )
        .unwrap();
        assert!(out.contains("- 기존 항목\n"));
        assert!(out.contains("- 신규 항목\n"));
    }

    #[test]
    fn upsert_item_creates_section_when_missing() {
        let doc = "# Top\n\nintro\n";
        let out = apply(
            doc,
            &EditOp::UpsertItem {
                heading: "## new".into(),
                key: "x".into(),
                item: "- first".into(),
            },
        )
        .unwrap();
        assert!(out.contains("## new\n"));
        assert!(out.contains("- first\n"));
    }

    #[test]
    fn upsert_item_preserves_following_section() {
        let doc = "## list\n\n- a\n\n## next\n\nstays\n";
        let out = apply(
            doc,
            &EditOp::UpsertItem {
                heading: "## list".into(),
                key: "a".into(),
                item: "- a (rev)".into(),
            },
        )
        .unwrap();
        assert!(out.contains("- a (rev)\n"));
        assert!(out.contains("## next\n\nstays\n"));
    }
}
