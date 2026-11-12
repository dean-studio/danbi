use crate::error::DanbiResult;
use crate::ontology::{self, DocMeta, Vocabulary};
use crate::vault::PROJECTS_DIRNAME;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Serialize, Clone)]
pub struct WikiLink {
    pub project: String,
    pub domain: String,
    /// Typed relation of this link (`affects`, `caused_by`, …) as the
    /// *source* document states it — from `원인:: [[x]]`, a frontmatter list
    /// or a `## 관련` section. None for a plain link. Incoming entries carry
    /// the same source-side relation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct LinkIndex {
    /// Forward: "project/domain" -> outgoing targets.
    pub outgoing: HashMap<String, Vec<WikiLink>>,
    /// Reverse: "project/domain" -> incoming sources.
    pub incoming: HashMap<String, Vec<WikiLink>>,
    /// "project/domain" -> type/status of every indexed document. Built in
    /// the same walk, so the ontology costs no extra IO.
    pub meta: HashMap<String, DocMeta>,
    /// Vocabulary problems per document (only documents that have any) —
    /// surfaced by `healing`.
    #[serde(skip)]
    pub issues: HashMap<String, ontology::DocIssues>,
    /// Links the resolver could not pin to one document: the same file
    /// name lives in two or more folders. Keyed by source "project/domain";
    /// `outgoing` keeps such a link as written. Dreaming asks the user.
    #[serde(skip)]
    pub ambiguous: HashMap<String, Vec<AmbiguousLink>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AmbiguousLink {
    /// Target project (after project folding).
    pub project: String,
    /// The target as written, normalised to `.md`.
    pub written: String,
    /// Documents in that project it could mean (project-relative).
    pub candidates: Vec<String>,
}

impl LinkIndex {
    /// Meta for a document, falling back to folder inference for files the
    /// walk doesn't reach (deeper than one sub-folder).
    pub fn meta_or_infer(&self, vocab: &Vocabulary, project: &str, domain: &str) -> DocMeta {
        self.meta.get(&key(project, domain)).cloned().unwrap_or_else(|| DocMeta {
            doc_type: ontology::infer_type(vocab, domain),
            type_source: ontology::TypeSource::Folder,
            type_known: true,
            status: None,
            effective_status: None,
        })
    }
}

fn key(project: &str, domain: &str) -> String {
    format!("{project}/{domain}")
}

// ---- Link resolution ----
//
// A `[[link]]` names a document loosely: agents write `[[cron-pipeline.md]]`
// for `notes/cron-pipeline.md`, humans write `[[Auth Flow]]` for
// `auth-flow.md`, and some tools emit `[[../INDEX.md]]`. The resolver
// turns what was written into the one document it can only mean, and
// refuses when it could mean two. Nothing on disk is rewritten.

/// File-name comparison key: NFC → lower-case → drop `.md` → drop
/// whitespace, `_`, `-`, `.` and `·`. `/` is kept, so `notes/x` and `x`
/// stay different. macOS stores Korean names as NFD; links are typed NFC.
pub fn norm_link_name(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    let lower = nfc.to_lowercase();
    let stem = lower.strip_suffix(".md").unwrap_or(&lower);
    stem.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-' | '.' | '·'))
        .collect()
}

pub fn link_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `../notes/x.md` · `./x.md` — written relative to the linking file.
pub fn is_relative_link(domain: &str) -> bool {
    domain.starts_with("../") || domain.starts_with("./") || domain.contains("/../") || domain.contains("/./")
}

pub fn link_dirname(domain: &str) -> &str {
    domain.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// Resolves a relative link against the linking file's folder, to a
/// project-root path. None when it climbs out of the project.
pub fn resolve_relative(src_dir: &str, link: &str) -> Option<String> {
    let mut segs: Vec<&str> = src_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in link.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            other => segs.push(other),
        }
    }
    if segs.is_empty() {
        return None;
    }
    Some(segs.join("/"))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// The one document the link means (project-relative path).
    Found(String),
    /// Two or more documents share that name; the link is left as written.
    Ambiguous(Vec<String>),
    /// Nothing by that name.
    Missing,
}

/// The documents of one project as the index walked them, with the lookup
/// tables the resolver needs. Built once per project per index build —
/// no IO beyond the walk `build_index` already does.
#[derive(Debug, Clone, Default)]
pub struct ProjectDocs {
    root: PathBuf,
    set: HashSet<String>,
    by_norm_path: HashMap<String, Vec<String>>,
    by_basename: HashMap<String, Vec<String>>,
}

impl ProjectDocs {
    pub fn new(root: PathBuf, docs: impl IntoIterator<Item = String>) -> Self {
        let mut out = ProjectDocs { root, ..Default::default() };
        for d in docs {
            out.by_norm_path.entry(norm_link_name(&d)).or_default().push(d.clone());
            out.by_basename.entry(norm_link_name(link_basename(&d))).or_default().push(d.clone());
            out.set.insert(d);
        }
        for v in out.by_norm_path.values_mut().chain(out.by_basename.values_mut()) {
            v.sort();
        }
        out
    }

    fn exists(&self, domain: &str) -> bool {
        self.set.contains(domain) || self.root.join(domain).is_file()
    }

    /// `raw` as written in `src_domain` (both project-relative, `raw`
    /// already ends in `.md`). Rules, first hit wins:
    /// 1. the written path exists;
    /// 2. a relative path (`../x.md`) resolved from the source folder exists;
    /// 3. exactly one document differs only in `.md`, case, separators or
    ///    Unicode form;
    /// 4. exactly one document in the project has that file name;
    /// 5. two or more → ambiguous; none → missing.
    pub fn resolve(&self, src_domain: &str, raw: &str) -> Resolution {
        if self.exists(raw) {
            return Resolution::Found(raw.to_string());
        }
        if is_relative_link(raw) {
            if let Some(r) = resolve_relative(link_dirname(src_domain), raw) {
                if self.exists(&r) {
                    return Resolution::Found(r);
                }
            }
        }
        if let Some(v) = self.by_norm_path.get(&norm_link_name(raw)) {
            return pick(v);
        }
        match self.by_basename.get(&norm_link_name(link_basename(raw))) {
            Some(v) => pick(v),
            None => Resolution::Missing,
        }
    }
}

fn pick(v: &[String]) -> Resolution {
    match v {
        [one] => Resolution::Found(one.clone()),
        [] => Resolution::Missing,
        many => Resolution::Ambiguous(many.to_vec()),
    }
}

fn walk_project_docs(project_dir: &Path) -> DanbiResult<Vec<(String, PathBuf)>> {
    let mut md_entries: Vec<(String, PathBuf)> = Vec::new();
    walk_md_for_links(project_dir, None, &mut md_entries)?;
    for de in std::fs::read_dir(project_dir)? {
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
    Ok(md_entries)
}

/// The resolver for one project, walking its folder now. For one-off
/// lookups (MCP read, ghost accept); the index builds its own.
pub fn project_docs(vault: &Path, project: &str) -> DanbiResult<ProjectDocs> {
    let root = vault.join(PROJECTS_DIRNAME).join(project);
    let docs = walk_project_docs(&root)?.into_iter().map(|(n, _)| n);
    Ok(ProjectDocs::new(root, docs))
}

/// `raw` (as a `[[link]]` target, `.md` optional) inside `project`, seen from
/// `src_domain` when there is one. Errors only when the project folder
/// can't be read.
pub fn resolve_doc(vault: &Path, project: &str, src_domain: Option<&str>, raw: &str) -> DanbiResult<Resolution> {
    let raw = raw.trim().trim_start_matches("[[").trim_end_matches("]]").split(['|', '#']).next().unwrap_or("").trim();
    if raw.is_empty() {
        return Ok(Resolution::Missing);
    }
    let raw = if raw.to_lowercase().ends_with(".md") { raw.to_string() } else { format!("{raw}.md") };
    Ok(project_docs(vault, project)?.resolve(src_domain.unwrap_or(""), &raw))
}

/// Parses wiki links from a single markdown blob. Supported syntaxes:
///   [[Project/domain.md]]
///   [[Project/domain]]
///   [[domain.md]]        (within-project, uses fallback_project)
/// Each link comes with its byte offset so the caller can look up the
/// line's relation.
pub fn extract_links_at(md: &str, fallback_project: &str) -> Vec<(usize, WikiLink)> {
    let mut out = Vec::new();
    let bytes = md.as_bytes();
    let code = code_ranges(md);
    let mut next_code = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        // 코드 블록·인라인 코드 안의 `[[…]]` 는 예시일 뿐 링크가 아니다
        // (SKILL.md·CLAUDE.md 템플릿의 `[[wiki-link]]` 같은 것).
        while next_code < code.len() && code[next_code].1 <= i {
            next_code += 1;
        }
        if let Some(&(start, end)) = code.get(next_code) {
            if start <= i {
                i = end;
                continue;
            }
        }
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(end) = find_double_close(&bytes[i + 2..]) {
                let inner_bytes = &bytes[i + 2..i + 2 + end];
                if let Ok(inner) = std::str::from_utf8(inner_bytes) {
                    if let Some(link) = parse_link(inner, fallback_project) {
                        out.push((i, link));
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

/// 마크다운에서 코드인 바이트 구간들 (정렬됨, 겹치지 않음): ``` / ~~~ 펜스
/// 블록 전체(펜스 줄 포함, 닫히지 않으면 끝까지)와 한 줄 안의 인라인 코드
/// (같은 길이의 백틱 묶음 사이).
fn code_ranges(md: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut fence: Option<(u8, usize, usize)> = None; // (문자, 길이, 시작)
    let mut pos = 0;
    for line in md.split_inclusive('\n') {
        let start = pos;
        pos += line.len();
        let trimmed = line.trim_start_matches([' ', '\t']);
        let marker = trimmed.bytes().next().filter(|b| *b == b'`' || *b == b'~');
        let run = marker.map(|m| trimmed.bytes().take_while(|b| *b == m).count()).unwrap_or(0);
        match fence {
            Some((m, len, fstart)) => {
                if marker == Some(m) && run >= len && trimmed[run..].trim().is_empty() {
                    out.push((fstart, pos));
                    fence = None;
                }
            }
            None if run >= 3 => fence = Some((marker.unwrap(), run, start)),
            None => {
                let b = line.as_bytes();
                let mut j = 0;
                while j < b.len() {
                    if b[j] != b'`' {
                        j += 1;
                        continue;
                    }
                    let open = j;
                    while j < b.len() && b[j] == b'`' {
                        j += 1;
                    }
                    let n = j - open;
                    // 같은 길이의 닫는 백틱 묶음 찾기
                    let mut k = j;
                    let mut close = None;
                    while k < b.len() {
                        if b[k] == b'`' {
                            let s = k;
                            while k < b.len() && b[k] == b'`' {
                                k += 1;
                            }
                            if k - s == n {
                                close = Some(k);
                                break;
                            }
                        } else {
                            k += 1;
                        }
                    }
                    match close {
                        Some(c) => {
                            out.push((start + open, start + c));
                            j = c;
                        }
                        None => break,
                    }
                }
            }
        }
    }
    if let Some((_, _, fstart)) = fence {
        out.push((fstart, md.len()));
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
    Some(WikiLink { project, domain, relation: None })
}

/// Walks the whole vault, extracts wiki links from every .md under every
/// project, and builds a bidirectional index.
pub fn build_index(vault: &Path) -> DanbiResult<LinkIndex> {
    let mut index = LinkIndex::default();
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(index);
    }
    // `[[notes/x.md]]` parses as project "notes"; when no such project
    // exists it is really a sub-folder doc of the linking project.
    let project_names: HashSet<String> = std::fs::read_dir(&projects_root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| !n.starts_with('.'))
        .collect();
    // First pass: every project's files, so a cross-project link can be
    // resolved inside its target project in the second pass.
    let mut entries: Vec<(String, PathBuf, Vec<(String, PathBuf)>)> = Vec::new();
    let mut docs: HashMap<String, ProjectDocs> = HashMap::new();
    for project in &project_names {
        let p = projects_root.join(project);
        let md_entries = walk_project_docs(&p)?;
        docs.insert(project.clone(), ProjectDocs::new(p.clone(), md_entries.iter().map(|(n, _)| n.clone())));
        entries.push((project.clone(), p, md_entries));
    }
    for (project, _p, md_entries) in entries {
        let vocab = ontology::vocabulary_for(vault, &project);
        for (fname, path) in md_entries {
            let Ok(md) = std::fs::read_to_string(&path) else {
                continue;
            };
            let rels = ontology::line_relations(&vocab, &md);
            let src = key(&project, &fname);
            let outgoing: Vec<WikiLink> = extract_links_at(&md, &project)
                .into_iter()
                .map(|(at, l)| {
                    let relation = rels.at(at).map(str::to_string);
                    let mut link = if project_names.contains(&l.project) {
                        WikiLink { relation, ..l }
                    } else {
                        WikiLink {
                            domain: format!("{}/{}", l.project, l.domain),
                            project: project.clone(),
                            relation,
                        }
                    };
                    let src_domain = if link.project == project { fname.as_str() } else { "" };
                    match docs.get(&link.project).map(|d| d.resolve(src_domain, &link.domain)) {
                        Some(Resolution::Found(d)) => link.domain = d,
                        Some(Resolution::Ambiguous(candidates)) => index.ambiguous.entry(src.clone()).or_default().push(AmbiguousLink {
                            project: link.project.clone(),
                            written: link.domain.clone(),
                            candidates,
                        }),
                        Some(Resolution::Missing) | None => {}
                    }
                    link
                })
                .collect();
            for target in &outgoing {
                index
                    .incoming
                    .entry(key(&target.project, &target.domain))
                    .or_default()
                    .push(WikiLink {
                        project: project.clone(),
                        domain: fname.clone(),
                        relation: target.relation.clone(),
                    });
            }
            index.meta.insert(src.clone(), ontology::doc_meta(&vocab, &fname, &md));
            let issues = ontology::doc_issues(&vocab, &md);
            if !issues.is_empty() {
                index.issues.insert(src.clone(), issues);
            }
            index.outgoing.insert(src, outgoing);
        }
    }
    apply_supersession(&mut index);
    Ok(index)
}

/// A document another one `supersedes` (or that names its own
/// `superseded_by`) is effectively superseded, whatever it declares —
/// unless it's already `done`.
fn apply_supersession(index: &mut LinkIndex) {
    let mut superseded: Vec<String> = Vec::new();
    for (src, links) in &index.outgoing {
        for l in links {
            match l.relation.as_deref() {
                Some("supersedes") => superseded.push(key(&l.project, &l.domain)),
                Some("superseded_by") => superseded.push(src.clone()),
                _ => {}
            }
        }
    }
    for k in superseded {
        if let Some(m) = index.meta.get_mut(&k) {
            if m.effective_status.as_deref() != Some("done") {
                m.effective_status = Some("superseded".into());
            }
        }
    }
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

/// `build_index` result shared until any `.md` in the vault changes
/// (`search::fingerprint_vault`). `danbi_context`, `danbi_related`, the
/// graph and the sidebar's link index all ask for the same thing within
/// seconds of each other.
static CACHE: Mutex<Option<(PathBuf, String, Arc<LinkIndex>)>> = Mutex::new(None);

pub fn build_index_cached(vault: &Path) -> DanbiResult<Arc<LinkIndex>> {
    let fp = crate::search::fingerprint_vault(vault)?;
    build_index_cached_with(vault, &fp)
}

/// Same as `build_index_cached` with a fingerprint the caller already
/// computed, so one request doesn't stat the vault three times.
pub(crate) fn build_index_cached_with(vault: &Path, fingerprint: &str) -> DanbiResult<Arc<LinkIndex>> {
    if let Ok(guard) = CACHE.lock() {
        if let Some((v, fp, idx)) = guard.as_ref() {
            if v == vault && fp == fingerprint {
                return Ok(idx.clone());
            }
        }
    }
    // Build outside the lock — a slow walk must not stall other readers
    // that still have a valid entry.
    let idx = Arc::new(build_index(vault)?);
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((vault.to_path_buf(), fingerprint.to_string(), idx.clone()));
    }
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(label: &str) -> std::path::PathBuf {
        let unique = format!(
            "danbi-links-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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

    #[test]
    fn cached_index_refreshes_when_a_file_changes() {
        let v = temp_vault("cache");
        write(&v, "A/a.md", "[[b.md]]");
        write(&v, "A/b.md", "");
        let first = build_index_cached(&v).unwrap();
        assert_eq!(first.outgoing["A/a.md"].len(), 1);
        assert!(Arc::ptr_eq(&first, &build_index_cached(&v).unwrap()));

        write(&v, "A/b.md", "[[a.md]] back-link");
        let second = build_index_cached(&v).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.incoming["A/a.md"].len(), 1);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn resolver_rules_in_order() {
        let docs = ProjectDocs::new(
            PathBuf::from("/nonexistent"),
            ["notes/auth.md", "decisions/auth.md", "notes/Cron Pipeline.md", "INDEX.md", "ep01/plan.md", "plan.md"]
                .into_iter()
                .map(String::from),
        );
        // ① 적힌 경로가 있으면 그것.
        assert_eq!(docs.resolve("a.md", "notes/auth.md"), Resolution::Found("notes/auth.md".into()));
        // ② 상대경로는 소스 폴더 기준.
        assert_eq!(docs.resolve("ep01/x.md", "../INDEX.md"), Resolution::Found("INDEX.md".into()));
        assert_eq!(docs.resolve("ep01/x.md", "./plan.md"), Resolution::Found("ep01/plan.md".into()));
        // ③ 대소문자·구분자·NFC 만 다른 것.
        assert_eq!(docs.resolve("a.md", "notes/cron-pipeline.md"), Resolution::Found("notes/Cron Pipeline.md".into()));
        assert_eq!(docs.resolve("a.md", "index.md"), Resolution::Found("INDEX.md".into()));
        // ④ 같은 파일명이 딱 하나면 그것.
        assert_eq!(docs.resolve("a.md", "cron_pipeline.md"), Resolution::Found("notes/Cron Pipeline.md".into()));
        // ⑤ 둘 이상이면 모호 — `plan.md` 는 루트에 있으니 ① 로 먼저 잡힌다.
        assert_eq!(docs.resolve("a.md", "plan.md"), Resolution::Found("plan.md".into()));
        assert_eq!(
            docs.resolve("a.md", "auth.md"),
            Resolution::Ambiguous(vec!["decisions/auth.md".into(), "notes/auth.md".into()])
        );
        assert_eq!(docs.resolve("a.md", "nope.md"), Resolution::Missing);
        // NFD 파일명 ↔ NFC 링크.
        let nfd: String = "notes/인증-결정.md".nfd().collect();
        let ko = ProjectDocs::new(PathBuf::from("/nonexistent"), [nfd]);
        assert!(matches!(ko.resolve("a.md", "인증-결정.md"), Resolution::Found(_)));
    }

    #[test]
    fn index_resolves_loose_links_and_keeps_ambiguous_ones_as_written() {
        let v = temp_vault("resolve");
        write(&v, "A/index.md", "[[cron-pipeline.md]] [[auth.md]] [[B/Spec]] [[nope.md]]");
        write(&v, "A/notes/cron-pipeline.md", "c");
        write(&v, "A/notes/auth.md", "n");
        write(&v, "A/decisions/auth.md", "d");
        write(&v, "A/ep01/a.md", "[[../index.md]]");
        write(&v, "B/docs/spec.md", "s");

        let idx = build_index(&v).unwrap();
        let out: Vec<String> = idx.outgoing["A/index.md"].iter().map(|l| format!("{}/{}", l.project, l.domain)).collect();
        assert_eq!(out, vec!["A/notes/cron-pipeline.md", "A/auth.md", "B/docs/spec.md", "A/nope.md"]);
        assert!(idx.incoming.contains_key("A/notes/cron-pipeline.md"));
        assert!(idx.incoming.contains_key("B/docs/spec.md"));
        assert_eq!(
            idx.ambiguous["A/index.md"],
            vec![AmbiguousLink {
                project: "A".into(),
                written: "auth.md".into(),
                candidates: vec!["decisions/auth.md".into(), "notes/auth.md".into()],
            }]
        );
        assert_eq!(idx.outgoing["A/ep01/a.md"][0].domain, "index.md");
        // 고아 판정: 느슨한 링크로 이어진 문서는 고아가 아니다.
        let orphans: Vec<String> = crate::healing::scan(&v)
            .unwrap()
            .into_iter()
            .filter_map(|s| match s {
                crate::healing::Suggestion::Orphan { project, domain } => Some(format!("{project}/{domain}")),
                _ => None,
            })
            .collect();
        assert!(!orphans.contains(&"A/notes/cron-pipeline.md".to_string()), "{orphans:?}");
        assert!(!orphans.contains(&"B/docs/spec.md".to_string()));
        assert!(orphans.contains(&"A/notes/auth.md".to_string()));
        // 관계 링크가 두 단계 아래 폴더의 문서를 가리켜도 "없는 문서" 가 아니다.
        write(&v, "A/plan/phase/p01.md", "p");
        write(&v, "A/plan/top.md", "relates_to:: [[plan/phase/p01.md]]\nrelates_to:: [[plan/phase/gone.md]]");
        let dangling: Vec<String> = crate::healing::scan(&v)
            .unwrap()
            .into_iter()
            .filter_map(|s| match s {
                crate::healing::Suggestion::DanglingTypedLink { target, .. } => Some(target),
                _ => None,
            })
            .collect();
        assert_eq!(dangling, vec!["A/plan/phase/gone.md".to_string()]);
        // 단건 해석 API.
        assert_eq!(resolve_doc(&v, "A", None, "[[cron-pipeline|별칭]]").unwrap(), Resolution::Found("notes/cron-pipeline.md".into()));
        assert!(matches!(resolve_doc(&v, "A", None, "auth").unwrap(), Resolution::Ambiguous(_)));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn subfolder_link_resolves_to_linking_project() {
        let v = temp_vault("subfolder");
        write(&v, "A/index.md", "[[notes/x.md]] and [[B/y.md]]");
        write(&v, "A/notes/x.md", "x");
        write(&v, "B/y.md", "y");

        let idx = build_index(&v).unwrap();
        let out: Vec<String> = idx.outgoing["A/index.md"]
            .iter()
            .map(|l| format!("{}/{}", l.project, l.domain))
            .collect();
        assert_eq!(out, vec!["A/notes/x.md", "B/y.md"]);
        assert!(idx.incoming.contains_key("A/notes/x.md"));
        assert!(!idx.incoming.contains_key("notes/x.md"));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn index_carries_relations_types_and_supersession() {
        let v = temp_vault("ontology");
        write(&v, "A/decisions/new.md", "---\nstatus: active\n---\n대체:: [[decisions/old.md]]\n영향:: [[B/ui.md]]\n");
        write(&v, "A/decisions/old.md", "---\nstatus: active\n---\n옛 결정");
        write(&v, "B/ui.md", "## 관련\n- [[A/decisions/new.md]]\n");

        let idx = build_index(&v).unwrap();
        let out = &idx.outgoing["A/decisions/new.md"];
        assert_eq!(out[0].relation.as_deref(), Some("supersedes"));
        assert_eq!(out[1].relation.as_deref(), Some("affects"));
        assert_eq!(idx.incoming["B/ui.md"][0].relation.as_deref(), Some("affects"));
        assert_eq!(idx.incoming["A/decisions/new.md"][0].relation.as_deref(), Some("relates_to"));
        assert_eq!(idx.meta["A/decisions/new.md"].doc_type, "decision");
        assert_eq!(idx.meta["B/ui.md"].doc_type, "note");
        assert_eq!(idx.meta["A/decisions/old.md"].effective_status.as_deref(), Some("superseded"));
        assert_eq!(idx.meta["A/decisions/new.md"].effective_status.as_deref(), Some("active"));
        std::fs::remove_dir_all(v).ok();
    }
}

#[cfg(test)]
mod code_span_tests {
    use super::*;

    fn targets(md: &str) -> Vec<String> {
        extract_links_at(md, "P").into_iter().map(|(_, l)| l.domain).collect()
    }

    #[test]
    fn links_inside_code_are_examples_not_links() {
        let md = "진짜 [[a.md]] 와 `[[wiki-link]]` 예시\n\n```markdown\n### 예시\n[[notes/auth.md]]\n```\n\n~~~\n[[tilde.md]]\n~~~\n뒤 [[b.md]] 와 ``[[double]]`` 끝";
        assert_eq!(targets(md), vec!["a.md", "b.md"]);
    }

    #[test]
    fn unclosed_backtick_and_fence_edges() {
        // 짝 없는 백틱은 코드가 아니다
        assert_eq!(targets("가격 `5 [[a.md]]"), vec!["a.md"]);
        // 닫히지 않은 펜스는 끝까지 코드
        assert_eq!(targets("[[a.md]]\n```\n[[b.md]]\n"), vec!["a.md"]);
        // 들여쓴 펜스, 더 긴 닫는 펜스
        assert_eq!(targets("  ```\n[[x.md]]\n  ````\n[[y.md]]"), vec!["y.md"]);
        // 한 줄에 인라인 코드 여러 개
        assert_eq!(targets("`a` [[m.md]] `[[n.md]]` [[o.md]]"), vec!["m.md", "o.md"]);
    }

    #[test]
    fn link_offsets_still_point_at_the_brackets() {
        let md = "`[[x]]` 그리고 [[real.md]]";
        let (at, _) = extract_links_at(md, "P")[0];
        assert_eq!(&md[at..at + 2], "[[");
        assert!(md[at..].starts_with("[[real.md]]"));
    }
}

#[cfg(test)]
mod real_clone_measure {
    use super::*;

    /// 복제 vault 에서 링크가 어느 규칙으로 풀리는지 센다 (README 숫자용).
    /// `DANBI_DREAM_VAULT=<복제본> cargo test --release --lib resolver_rule_counts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn resolver_rule_counts() {
        let Some(dir) = std::env::var_os("DANBI_DREAM_VAULT") else { return };
        let vault = PathBuf::from(dir);
        assert!(vault.to_string_lossy().contains("scratchpad"), "복제본만");
        let root = vault.join(PROJECTS_DIRNAME);
        let (mut written, mut relative, mut loose, mut basename, mut amb, mut miss) = (0, 0, 0, 0, 0, 0);
        let mut by_project: HashMap<String, usize> = HashMap::new();
        let mut names: Vec<String> = std::fs::read_dir(&root).unwrap().flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        let all: HashMap<String, ProjectDocs> = names.iter().map(|p| (p.clone(), project_docs(&vault, p).unwrap())).collect();
        for p in &names {
            for (fname, path) in walk_project_docs(&root.join(p)).unwrap() {
                if fname.ends_with(crate::vault::SKILL_FILENAME) { continue; }
                let Ok(md) = std::fs::read_to_string(&path) else { continue };
                for (_, l) in extract_links_at(&md, p) {
                    // build_index 와 같게: 모르는 프로젝트 이름은 이 프로젝트 안의 폴더로 본다.
                    let l = if all.contains_key(&l.project) { l } else {
                        WikiLink { domain: format!("{}/{}", l.project, l.domain), project: p.clone(), relation: None }
                    };
                    let Some(docs) = all.get(&l.project) else { continue };
                    let src = if &l.project == p { fname.as_str() } else { "" };
                    if docs.exists(&l.domain) { written += 1; continue; }
                    if is_relative_link(&l.domain) {
                        if let Some(r) = resolve_relative(link_dirname(src), &l.domain) {
                            if docs.exists(&r) { relative += 1; continue; }
                        }
                    }
                    match docs.by_norm_path.get(&norm_link_name(&l.domain)).map(|v| v.len()) {
                        Some(1) => { loose += 1; continue; }
                        Some(_) => { amb += 1; continue; }
                        None => {}
                    }
                    match docs.by_basename.get(&norm_link_name(link_basename(&l.domain))).map(|v| v.len()) {
                        Some(1) => { basename += 1; *by_project.entry(p.clone()).or_default() += 1; }
                        Some(_) => amb += 1,
                        None => miss += 1,
                    }
                }
            }
        }
        println!("쓴 그대로={written} 상대경로={relative} 표기차(.md·대소문자·구분자)={loose} 파일이름만={basename} 애매={amb} 없음={miss}");
        let mut v: Vec<_> = by_project.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        println!("파일이름만 상위: {:?}", &v[..v.len().min(6)]);
    }
}
