use crate::error::{DanbiError, DanbiResult};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct Extracted {
    pub filename: String,
    pub mime: String,
    pub kind: String, // "pdf" | "docx" | "text" | "unknown"
    pub text: String,
    pub bytes: usize,
    pub truncated: bool,
}

/// Hard cap on extracted text — prevents pathological files from blowing up
/// the LLM prompt. 200k chars is roughly 50k tokens, still enough for most
/// single-document cases. Larger files should be summarized before routing.
const MAX_CHARS: usize = 200_000;

fn clip(mut text: String) -> (String, bool) {
    let mut truncated = false;
    if text.chars().count() > MAX_CHARS {
        let end = text.char_indices().nth(MAX_CHARS).map(|(i, _)| i).unwrap_or(text.len());
        text.truncate(end);
        truncated = true;
    }
    (text, truncated)
}

fn ext_lower(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn extract_from_path(path: &Path) -> DanbiResult<Extracted> {
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let ext = ext_lower(path);

    match ext.as_str() {
        "pdf" => extract_pdf(path, filename),
        "docx" => extract_docx(path, filename),
        "txt" | "md" | "markdown" | "csv" | "json" | "yaml" | "yml" | "toml" | "log" => {
            extract_text(path, filename, &ext)
        }
        other => Err(DanbiError::Config(format!(
            "unsupported file type: {other}"
        ))),
    }
}

pub fn extract_from_bytes(filename: &str, bytes: &[u8]) -> DanbiResult<Extracted> {
    // Dispatch by filename extension. The bytes path is used when the UI
    // hands us a File object rather than a filesystem path (drag-drop, paste).
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        let raw = pdf_extract::extract_text_from_mem(bytes)
            .map_err(|e| DanbiError::Other(format!("pdf extract: {e}")))?;
        let (text, truncated) = clip(raw);
        return Ok(Extracted {
            filename: filename.to_string(),
            mime: "application/pdf".into(),
            kind: "pdf".into(),
            text,
            bytes: bytes.len(),
            truncated,
        });
    }
    if lower.ends_with(".docx") {
        let raw = extract_docx_bytes(bytes)?;
        let (text, truncated) = clip(raw);
        return Ok(Extracted {
            filename: filename.to_string(),
            mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
            kind: "docx".into(),
            text,
            bytes: bytes.len(),
            truncated,
        });
    }
    // Fallback: treat as UTF-8 text, replacing invalid sequences.
    let text = String::from_utf8_lossy(bytes).into_owned();
    let (text, truncated) = clip(text);
    Ok(Extracted {
        filename: filename.to_string(),
        mime: guess_text_mime(filename),
        kind: "text".into(),
        text,
        bytes: bytes.len(),
        truncated,
    })
}

fn guess_text_mime(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        "text/markdown".into()
    } else if lower.ends_with(".csv") {
        "text/csv".into()
    } else if lower.ends_with(".json") {
        "application/json".into()
    } else {
        "text/plain".into()
    }
}

fn extract_pdf(path: &Path, filename: String) -> DanbiResult<Extracted> {
    let raw = pdf_extract::extract_text(path)
        .map_err(|e| DanbiError::Other(format!("pdf extract: {e}")))?;
    let bytes = std::fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0);
    let (text, truncated) = clip(raw);
    Ok(Extracted {
        filename,
        mime: "application/pdf".into(),
        kind: "pdf".into(),
        text,
        bytes,
        truncated,
    })
}

fn extract_docx(path: &Path, filename: String) -> DanbiResult<Extracted> {
    let bytes_vec = std::fs::read(path)?;
    let raw = extract_docx_bytes(&bytes_vec)?;
    let (text, truncated) = clip(raw);
    Ok(Extracted {
        filename,
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
        kind: "docx".into(),
        text,
        bytes: bytes_vec.len(),
        truncated,
    })
}

fn extract_docx_bytes(bytes: &[u8]) -> DanbiResult<String> {
    use docx_rs::*;
    let doc = read_docx(bytes).map_err(|e| DanbiError::Other(format!("docx parse: {e}")))?;
    let json = doc.json();
    // docx-rs 0.4 exposes structured JSON but no plain-text extractor; walk the
    // JSON for "text" fields to assemble a reasonable plain-text rendering.
    let v: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| DanbiError::Other(format!("docx json: {e}")))?;

    fn walk(node: &serde_json::Value, out: &mut String) {
        match node {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(s)) = map.get("text") {
                    out.push_str(s);
                }
                if let Some(serde_json::Value::String(ty)) = map.get("type") {
                    // Insert line breaks at paragraph boundaries.
                    if ty == "paragraph" {
                        out.push('\n');
                    }
                }
                for (_, child) in map {
                    walk(child, out);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }

    let mut out = String::new();
    walk(&v, &mut out);
    // Collapse triple+ newlines to double for readability.
    let normalized = out
        .split_inclusive('\n')
        .fold(String::with_capacity(out.len()), |mut acc, line| {
            if line.trim().is_empty()
                && acc.ends_with("\n\n")
            {
                // skip extra blank line
            } else {
                acc.push_str(line);
            }
            acc
        });
    Ok(normalized)
}

fn extract_text(path: &Path, filename: String, ext: &str) -> DanbiResult<Extracted> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (text, truncated) = clip(text);
    let mime = if ext == "md" || ext == "markdown" {
        "text/markdown"
    } else if ext == "csv" {
        "text/csv"
    } else if ext == "json" {
        "application/json"
    } else {
        "text/plain"
    };
    Ok(Extracted {
        filename,
        mime: mime.into(),
        kind: "text".into(),
        text,
        bytes: bytes.len(),
        truncated,
    })
}
