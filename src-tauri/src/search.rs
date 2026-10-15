use crate::providers::Provider;
use crate::error::{DanbiError, DanbiResult};
use crate::vault::PROJECTS_DIRNAME;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, TryLockError};
use tantivy::{
    collector::TopDocs,
    query::QueryParser,
    schema::{Field, Schema, TextFieldIndexing, TextOptions, Value, STORED, STRING},
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
};

#[derive(Debug, Serialize, Clone)]
pub struct VaultDocIndex {
    pub project: String,
    pub domain: String,
    pub chars: usize,
    pub preview: String,
    pub headings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchHit {
    pub project: String,
    pub domain: String,
    #[serde(default)]
    pub relevance: f32,
    #[serde(default)]
    pub snippet: String,
    /// Which retriever found this hit: "keyword", "semantic", or
    /// "keyword+semantic". Lets MCP callers tell a literal match from a
    /// loose embedding neighbour. Empty for non-hybrid paths.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub matched: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub summary: String,
}

const PREVIEW_CHARS: usize = 240;
const MAX_HEADINGS: usize = 8;

// ---------- In-memory Tantivy index ----------

#[derive(Clone)]
struct SearchFields {
    /// "project/domain" — unique per doc, so a changed/deleted file can be
    /// removed with a single `delete_term` instead of a full rebuild.
    doc_id: Field,
    project: Field,
    domain: Field,
    body: Field,
    body_ngram: Field,
    /// Raw markdown stashed for snippet extraction without hitting disk.
    stored_body: Field,
}

/// (size, mtime) of one `.md` — the cheap change signal used by every
/// cache in this crate. Content hashing would cost a full read per file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct FileStamp {
    pub len: u64,
    pub mtime_ms: u128,
}

/// Every `.md` the search/link/graph walks see (depth ≤ 2 below the
/// project folder, hidden and `_assets` folders skipped), keyed by
/// "project/rel.md", plus an order-independent fingerprint of the whole
/// set. One directory walk + one `stat` per file; ~10ms for 1.5k files.
#[derive(Clone, Debug)]
pub(crate) struct VaultSnapshot {
    pub files: HashMap<String, (PathBuf, FileStamp)>,
    pub fingerprint: String,
}

impl VaultSnapshot {
    /// Ids that must be (re)indexed and ids that must be dropped to turn
    /// `old` into `self`.
    fn diff(&self, old: &VaultSnapshot) -> (Vec<String>, Vec<String>) {
        let mut upsert = Vec::new();
        for (id, (_, stamp)) in &self.files {
            match old.files.get(id) {
                Some((_, prev)) if prev == stamp => {}
                _ => upsert.push(id.clone()),
            }
        }
        let removed: Vec<String> = old
            .files
            .keys()
            .filter(|id| !self.files.contains_key(*id))
            .cloned()
            .collect();
        (upsert, removed)
    }
}

pub(crate) fn snapshot_vault(vault: &Path) -> DanbiResult<VaultSnapshot> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(VaultSnapshot {
            files: HashMap::new(),
            fingerprint: "empty".into(),
        });
    }
    let mut project_dirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&projects_root)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        match p.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.starts_with('.') => project_dirs.push((n.to_string(), p)),
            _ => {}
        }
    }

    // Two parallel phases. Listing uses `DirEntry::file_type` (free on
    // APFS — it comes back with the directory read) so the only `stat`
    // per file is the one for its stamp, and that phase is chunked so a
    // single large project can't serialise the walk.
    let listed: Vec<(String, PathBuf)> = std::thread::scope(|scope| {
        let handles: Vec<_> = project_dirs
            .iter()
            .map(|(project, dir)| {
                scope.spawn(move || {
                    let mut files = Vec::new();
                    list_project_md_fast(dir, &mut files);
                    files
                        .into_iter()
                        .map(|(rel, path)| (format!("{project}/{rel}"), path))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let stamp_of = |path: &Path| -> Option<FileStamp> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or(0);
        Some(FileStamp { len: meta.len(), mtime_ms })
    };
    let walked: Vec<Vec<(String, PathBuf, FileStamp)>> = if listed.len() < SNAPSHOT_PARALLEL_MIN {
        vec![listed
            .into_iter()
            .filter_map(|(id, path)| stamp_of(&path).map(|st| (id, path, st)))
            .collect()]
    } else {
        let chunk = listed.len().div_ceil(SNAPSHOT_THREADS).max(1);
        std::thread::scope(|scope| {
            let handles: Vec<_> = listed
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || {
                        part.iter()
                            .filter_map(|(id, path)| {
                                stamp_of(path).map(|st| (id.clone(), path.clone(), st))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect()
        })
    };

    let mut files: HashMap<String, (PathBuf, FileStamp)> = HashMap::new();
    // Order-independent combine so read_dir ordering can't fake a change.
    let mut acc: u64 = 0;
    for chunk in walked {
        for (id, path, stamp) in chunk {
            let mut h = DefaultHasher::new();
            id.hash(&mut h);
            stamp.hash(&mut h);
            acc = acc.wrapping_add(h.finish().rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15);
            files.insert(id, (path, stamp));
        }
    }
    let fingerprint = format!("{:x}-{}", acc, files.len());
    Ok(VaultSnapshot { files, fingerprint })
}

/// Cheap change detector shared by the link/graph caches.
pub(crate) fn fingerprint_vault(vault: &Path) -> DanbiResult<String> {
    snapshot_vault(vault).map(|s| s.fingerprint)
}

struct IndexedCorpus {
    vault: PathBuf,
    snapshot: VaultSnapshot,
    index: Index,
    reader: IndexReader,
    fields: SearchFields,
}

/// Global cache of the most recent tantivy index. Searches clone the `Arc`
/// out under a short lock and never hold `CORPUS` while (re)building.
static CORPUS: Mutex<Option<Arc<IndexedCorpus>>> = Mutex::new(None);
/// Serializes builders. A search that finds this busy keeps using the
/// previous corpus instead of queueing behind the rebuild.
static REFRESH: Mutex<()> = Mutex::new(());

/// Past this share of changed files a fresh build beats delete+add.
const FULL_REBUILD_RATIO: f64 = 0.5;
/// Snapshot stat phase: below this many files threads cost more than they save.
const SNAPSHOT_PARALLEL_MIN: usize = 64;
const SNAPSHOT_THREADS: usize = 8;
const WRITER_HEAP_BYTES: usize = 15_000_000;

fn make_schema() -> (Schema, SearchFields) {
    let mut sb = Schema::builder();
    let doc_id = sb.add_text_field("doc_id", STRING);
    let project = sb.add_text_field("project", STRING | STORED);
    let domain = sb.add_text_field("domain", STRING | STORED);

    // `body` field: default tokenizer (tokenizes on whitespace + lowercases).
    // Works well for Latin text and reasonably for Korean thanks to ngram.
    let body_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let body = sb.add_text_field("body", body_opts);

    // `body_ngram` mirrors the body but uses a character n-gram tokenizer so
    // Korean substrings (which rarely split on whitespace the way English
    // does) still match: "로그인" matches against "로그인하기".
    let ngram_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("danbi_ngram")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let body_ngram = sb.add_text_field("body_ngram", ngram_opts);

    let stored_body = sb.add_text_field("stored_body", TextOptions::default().set_stored());

    let schema = sb.build();
    (
        schema,
        SearchFields {
            doc_id,
            project,
            domain,
            body,
            body_ngram,
            stored_body,
        },
    )
}

/// `collect_project_md_files` without a `stat` per entry: same depth and
/// skip rules, but file-vs-dir comes from the directory read. Symlinks
/// fall back to a real `stat` so a linked note still counts as a file.
fn list_project_md_fast(project_dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    fn kind(de: &std::fs::DirEntry) -> (bool, bool) {
        match de.file_type() {
            Ok(ft) if ft.is_symlink() => {
                let p = de.path();
                (p.is_file(), p.is_dir())
            }
            Ok(ft) => (ft.is_file(), ft.is_dir()),
            Err(_) => (false, false),
        }
    }
    fn files_in(dir: &Path, prefix: Option<&str>, out: &mut Vec<(String, PathBuf)>) -> Vec<(String, PathBuf)> {
        let mut subdirs = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else { return subdirs };
        for de in rd.flatten() {
            let (is_file, is_dir) = kind(&de);
            let name = de.file_name();
            let Some(name) = name.to_str() else { continue };
            if is_dir {
                if !name.starts_with('.') && name != "_assets" {
                    subdirs.push((name.to_string(), de.path()));
                }
            } else if is_file && name.to_lowercase().ends_with(".md") && !name.starts_with('.') {
                let full = match prefix {
                    Some(p) => format!("{p}/{name}"),
                    None => name.to_string(),
                };
                out.push((full, de.path()));
            }
        }
        subdirs
    }
    for (sname, sub) in files_in(project_dir, None, out) {
        for (sname2, sub2) in files_in(&sub, Some(&sname), out) {
            let combined = format!("{sname}/{sname2}");
            // Depth 2 is the floor: anything deeper is ignored, as in
            // `collect_project_md_files`.
            files_in(&sub2, Some(&combined), out);
        }
    }
}

/// Walk a project directory and collect all `.md` files up to depth 2
/// (project/, project/<folder>/, project/<folder>/<folder>/).  The
/// returned name is the path relative to the project root, e.g.
/// `daily/2026-01/05.md`. Hidden directories (`.git`, `_assets`, ".") are
/// skipped at every level.
fn collect_project_md_files(
    project_dir: &Path,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> DanbiResult<()> {
    collect_md_files(project_dir, None, out)?;
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
        collect_md_files(&sub, Some(sname), out)?;
        // One more level of nesting (depth 2).
        for de2 in std::fs::read_dir(&sub)? {
            let de2 = de2?;
            let sub2 = de2.path();
            if !sub2.is_dir() {
                continue;
            }
            let Some(sname2) = sub2.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if sname2.starts_with('.') || sname2 == "_assets" {
                continue;
            }
            let combined = format!("{sname}/{sname2}");
            collect_md_files(&sub2, Some(&combined), out)?;
        }
    }
    Ok(())
}

/// Walks one directory, adds its `.md` files into `out` with the given
/// subfolder prefix (or no prefix for the project root).
fn collect_md_files(
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

/// One tantivy document for "project/rel.md". Filename + headings are
/// prepended to the searchable body so tantivy weights them higher.
fn make_doc(fields: &SearchFields, id: &str, path: &Path) -> Option<TantivyDocument> {
    let (project, fname) = id.split_once('/')?;
    let md = std::fs::read_to_string(path).ok()?;
    let headings: Vec<String> = md
        .lines()
        .filter(|l| l.trim_start().starts_with('#'))
        .take(MAX_HEADINGS)
        .map(|l| l.trim().to_string())
        .collect();
    let boosted = format!(
        "{project} {fname} {heads}\n{body}",
        heads = headings.join(" "),
        body = md,
    );
    let mut doc = TantivyDocument::default();
    doc.add_text(fields.doc_id, id);
    doc.add_text(fields.project, project);
    doc.add_text(fields.domain, fname);
    doc.add_text(fields.body, boosted.as_str());
    doc.add_text(fields.body_ngram, boosted.as_str());
    doc.add_text(fields.stored_body, md.as_str());
    Some(doc)
}

fn open_writer(index: &Index) -> DanbiResult<IndexWriter> {
    index
        .writer(WRITER_HEAP_BYTES)
        .map_err(|e| DanbiError::Other(format!("tantivy writer: {e}")))
}

fn build_corpus(vault: &Path, snapshot: VaultSnapshot) -> DanbiResult<IndexedCorpus> {
    use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};

    let (schema, fields) = make_schema();
    let index = Index::create_in_ram(schema);

    // Register our Korean-friendly ngram analyzer on the `body_ngram` field.
    let ngram_analyzer = TextAnalyzer::builder(
        NgramTokenizer::new(2, 3, false)
            .map_err(|e| DanbiError::Other(format!("ngram: {e}")))?,
    )
    .filter(LowerCaser)
    .build();
    index
        .tokenizers()
        .register("danbi_ngram", ngram_analyzer);

    let mut writer = open_writer(&index)?;
    for (id, (path, _)) in &snapshot.files {
        if let Some(doc) = make_doc(&fields, id, path) {
            writer
                .add_document(doc)
                .map_err(|e| DanbiError::Other(format!("tantivy add: {e}")))?;
        }
    }
    writer
        .commit()
        .map_err(|e| DanbiError::Other(format!("tantivy commit: {e}")))?;

    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(|e| DanbiError::Other(format!("tantivy reader: {e}")))?;

    Ok(IndexedCorpus {
        vault: vault.to_path_buf(),
        snapshot,
        index,
        reader,
        fields,
    })
}

/// Applies `snapshot` onto `old` in place: changed and deleted docs are
/// dropped by `doc_id`, changed and new docs re-added, one commit. The
/// `Index`/`IndexReader` handles are shared with `old`, so a search that
/// is still holding the old `Arc` keeps working; it simply sees the new
/// docs after `reader.reload()`.
/// Above this many segments the next refresh rebuilds from scratch.
const MAX_INCREMENTAL_SEGMENTS: usize = 32;

fn update_corpus(old: &IndexedCorpus, snapshot: VaultSnapshot) -> DanbiResult<IndexedCorpus> {
    let (upsert, removed) = snapshot.diff(&old.snapshot);
    let mut writer = open_writer(&old.index)?;
    for id in removed.iter().chain(upsert.iter()) {
        writer.delete_term(Term::from_field_text(old.fields.doc_id, id));
    }
    for id in &upsert {
        let Some((path, _)) = snapshot.files.get(id) else {
            continue;
        };
        if let Some(doc) = make_doc(&old.fields, id, path) {
            writer
                .add_document(doc)
                .map_err(|e| DanbiError::Other(format!("tantivy add: {e}")))?;
        }
    }
    writer
        .commit()
        .map_err(|e| DanbiError::Other(format!("tantivy commit: {e}")))?;
    // Dropping the writer kills its segment updater, which discards any
    // merge the commit just scheduled — every edit would then leave one more
    // segment (and its deleted docs) behind. Wait for the merges instead.
    writer
        .wait_merging_threads()
        .map_err(|e| DanbiError::Other(format!("tantivy merge: {e}")))?;
    old.reader
        .reload()
        .map_err(|e| DanbiError::Other(format!("tantivy reload: {e}")))?;
    Ok(IndexedCorpus {
        vault: old.vault.clone(),
        snapshot,
        index: old.index.clone(),
        reader: old.reader.clone(),
        fields: old.fields.clone(),
    })
}

fn current_corpus() -> DanbiResult<Option<Arc<IndexedCorpus>>> {
    CORPUS
        .lock()
        .map(|slot| slot.clone())
        .map_err(|_| DanbiError::Other("corpus lock".into()))
}

/// Returns a corpus that matches the vault on disk — or, while another
/// thread is already rebuilding, the previous one so searches never
/// queue behind an index build. `None` only before the very first build
/// when the vault has no projects folder.
fn ensure_corpus_fresh(vault: &Path) -> DanbiResult<Option<Arc<IndexedCorpus>>> {
    let is_fresh = |c: &Arc<IndexedCorpus>, snapshot: &VaultSnapshot| {
        c.vault == vault && c.snapshot.fingerprint == snapshot.fingerprint
    };
    let current = current_corpus()?;
    {
        let snapshot = snapshot_vault(vault)?;
        if let Some(c) = current.as_ref().filter(|c| is_fresh(c, &snapshot)) {
            return Ok(Some(c.clone()));
        }
    }

    let _building = match REFRESH.try_lock() {
        Ok(g) => g,
        Err(TryLockError::WouldBlock) => {
            if let Some(c) = current.filter(|c| c.vault == vault) {
                return Ok(Some(c)); // stale but instant; the builder will swap it in.
            }
            REFRESH
                .lock()
                .map_err(|_| DanbiError::Other("refresh lock".into()))?
        }
        Err(TryLockError::Poisoned(_)) => {
            return Err(DanbiError::Other("refresh lock".into()))
        }
    };
    // Snapshot again under the lock: one taken before it could be older
    // than the corpus another builder just swapped in, and applying it
    // would roll that corpus back (a new file would be "removed").
    let snapshot = snapshot_vault(vault)?;
    // Another thread may have finished exactly this build while we waited.
    let current = current_corpus()?;
    if let Some(c) = current.as_ref().filter(|c| is_fresh(c, &snapshot)) {
        return Ok(Some(c.clone()));
    }

    let incremental = current.as_ref().filter(|c| {
        if c.vault != vault {
            return false;
        }
        // Safety net if merges ever fall behind: a fresh build compacts.
        let segments = c.index.searchable_segment_ids().map(|s| s.len()).unwrap_or(0);
        if segments > MAX_INCREMENTAL_SEGMENTS {
            return false;
        }
        let (upsert, removed) = snapshot.diff(&c.snapshot);
        let total = snapshot.files.len().max(c.snapshot.files.len()).max(1);
        ((upsert.len() + removed.len()) as f64) < FULL_REBUILD_RATIO * total as f64
    });
    let next = match incremental {
        Some(old) => Arc::new(update_corpus(old, snapshot)?),
        None => {
            // v0.8.0: 옛 인덱스를 먼저 drop 해서 재빌드 중 2× 메모리 피크를 없앤다.
            // 재빌드가 실패하면 slot 은 None 이 되고, 다음 검색이 다시 빌드를 시도한다.
            drop(current);
            if let Ok(mut slot) = CORPUS.lock() {
                *slot = None;
            }
            Arc::new(build_corpus(vault, snapshot)?)
        }
    };
    let mut slot = CORPUS
        .lock()
        .map_err(|_| DanbiError::Other("corpus lock".into()))?;
    *slot = Some(next.clone());
    Ok(Some(next))
}

/// Builds (or refreshes) the tantivy corpus ahead of the first search.
/// Called from a background task at app start so the first ⌘K after
/// launch doesn't pay the ~2.5s build.
pub fn warm_corpus(vault: &Path) -> DanbiResult<()> {
    ensure_corpus_fresh(vault).map(|_| ())
}

// ---------- Public search ----------

pub fn build_index(vault: &Path) -> DanbiResult<Vec<VaultDocIndex>> {
    // Retained for the AI reranker — gives Haiku a preview-sized directory
    // of every doc regardless of tantivy state.
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
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
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        collect_project_md_files(&p, &mut files)?;

        for (fname, path) in files {
            let Ok(md) = std::fs::read_to_string(&path) else {
                continue;
            };
            let chars = md.chars().count();
            let preview: String = md.chars().take(PREVIEW_CHARS).collect();
            let mut headings = Vec::new();
            for line in md.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with('#') {
                    headings.push(trimmed.to_string());
                    if headings.len() >= MAX_HEADINGS {
                        break;
                    }
                }
            }
            out.push(VaultDocIndex {
                project: project.clone(),
                domain: fname,
                chars,
                preview,
                headings,
            });
        }
    }
    Ok(out)
}

/// Tier-2 full-text search. Uses tantivy BM25 on a default tokenizer plus a
/// character n-gram field for Korean substring coverage.
pub fn full_search(
    vault: &Path,
    query: &str,
    limit: usize,
) -> DanbiResult<Vec<SearchHit>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let Some(corpus) = ensure_corpus_fresh(vault)? else {
        return Ok(Vec::new());
    };
    search_corpus(&corpus, trimmed, limit)
}

fn search_corpus(corpus: &IndexedCorpus, trimmed: &str, limit: usize) -> DanbiResult<Vec<SearchHit>> {
    let searcher = corpus.reader.searcher();

    // Query both the body (tokenized) and the ngram (Korean-friendly).
    let parser = QueryParser::for_index(
        &corpus.index,
        vec![corpus.fields.body, corpus.fields.body_ngram],
    );
    let query = match parser.parse_query(trimmed) {
        Ok(q) => q,
        Err(_) => {
            // Fallback: quote the query so special characters don't trip the parser.
            let escaped = trimmed.replace('"', " ");
            let quoted = format!("\"{}\"", escaped);
            parser
                .parse_query(&quoted)
                .map_err(|e| DanbiError::Other(format!("tantivy parse: {e}")))?
        }
    };

    let top = searcher
        .search(&query, &TopDocs::with_limit(limit.max(1)))
        .map_err(|e| DanbiError::Other(format!("tantivy search: {e}")))?;

    let mut hits = Vec::with_capacity(top.len());
    let max_score = top.first().map(|(s, _)| *s).unwrap_or(1.0).max(0.001);

    for (score, addr) in top {
        let Ok(doc) = searcher.doc::<TantivyDocument>(addr) else {
            continue;
        };
        let project = doc
            .get_first(corpus.fields.project)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let domain = doc
            .get_first(corpus.fields.domain)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let body = doc
            .get_first(corpus.fields.stored_body)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let snippet = excerpt_for_query(body, trimmed);
        hits.push(SearchHit {
            project,
            domain,
            relevance: (score / max_score).min(1.0),
            snippet,
            matched: String::new(),
        });
    }

    Ok(hits)
}

fn excerpt_for_query(body: &str, query: &str) -> String {
    excerpt_match(body, query).unwrap_or_else(|| body.chars().take(80).collect())
}

/// Excerpt around the first case-insensitive occurrence of any whitespace
/// token in `query`: 20 chars before, 60 after. `find` returns a *byte*
/// offset, so the window is cut on char boundaries — slicing by byte
/// count would land far past the match in Korean (3 bytes per char).
fn excerpt_match(text: &str, query: &str) -> Option<String> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths for a few scripts; only reuse
    // offsets from `lower` when the two strings still line up.
    let aligned = lower.len() == text.len();
    for token in query.to_lowercase().split_whitespace() {
        let found = if aligned { lower.find(token) } else { text.find(token) };
        let Some(idx) = found else { continue };
        if !text.is_char_boundary(idx) {
            continue;
        }
        let mut head: Vec<char> = text[..idx].chars().rev().take(20).collect();
        head.reverse();
        let head: String = head.into_iter().collect();
        let tail: String = text[idx..]
            .chars()
            .take(token.chars().count() + 60)
            .collect();
        return Some(format!("… {head}{tail} …"));
    }
    None
}

/// Budgeted excerpt for `danbi_context`. Returns the markdown sections
/// that mention a query token (document order) cut to `max_chars`, plus
/// whether that excerpt is the whole document. A matching section longer
/// than the budget is windowed around its first match so the hit itself
/// is never cut off. With no matching section (e.g. a semantic-only hit)
/// it falls back to the top of the document.
pub fn focused_excerpt(md: &str, query: &str, max_chars: usize) -> (String, bool) {
    let tokens: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .filter(|t| t.chars().count() >= 2)
        .map(|t| t.to_string())
        .collect();

    // Split on ATX headings ("# ", "## " …) outside fenced code blocks —
    // `# comment` lines in code and `#tag` lines are not section breaks.
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_fence = false;
    for line in md.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        let hashes = t.chars().take_while(|c| *c == '#').count();
        let is_heading = !in_fence
            && (1..=6).contains(&hashes)
            && t[hashes..].starts_with(|c: char| c == ' ' || c == '\t');
        if is_heading && !current.trim().is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        sections.push(current);
    }

    let first_match = |sec: &str| -> Option<usize> {
        let lc = sec.to_lowercase();
        let aligned = lc.len() == sec.len();
        tokens
            .iter()
            .filter_map(|t| if aligned { lc.find(t.as_str()) } else { sec.find(t.as_str()) })
            .filter(|i| sec.is_char_boundary(*i))
            .min()
    };
    let matching: Vec<(&String, Option<usize>)> = sections
        .iter()
        .filter_map(|sec| first_match(sec).map(|i| (sec, Some(i))))
        .collect();
    let picked: Vec<(&String, Option<usize>)> = if matching.is_empty() {
        sections.iter().map(|sec| (sec, None)).collect()
    } else {
        matching
    };
    let mut complete = picked.len() == sections.len();

    let mut out = String::new();
    let mut used = 0usize;
    for (sec, hit) in picked {
        let n = sec.chars().count();
        if used + n <= max_chars {
            out.push_str(sec);
            used += n;
            continue;
        }
        complete = false;
        let room = max_chars.saturating_sub(used);
        if room > 0 {
            // Keep the window's start a little before the match so the
            // matched text lands inside it.
            let hit_char = hit.map(|i| sec[..i].chars().count()).unwrap_or(0);
            let skip = if hit_char + 80 > room {
                hit_char.saturating_sub(room / 4)
            } else {
                0
            };
            if skip > 0 {
                out.push_str("…");
            }
            out.extend(sec.chars().skip(skip).take(room));
        }
        out.push_str("\n…(생략)");
        break;
    }
    (out, complete)
}

/// Tier-1 instant search — preview-only substring match. Kept because it's
/// even faster than tantivy on vaults smaller than ~30 files and serves as
/// a safety net while the corpus is being (re)built.
pub fn local_search(
    index: &[VaultDocIndex],
    query: &str,
    limit: usize,
) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let tokens: Vec<&str> = q.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(i32, &VaultDocIndex, String)> = Vec::new();
    for doc in index {
        let mut score: i32 = 0;
        let mut best_snippet = String::new();

        let domain_lc = doc.domain.to_lowercase();
        let project_lc = doc.project.to_lowercase();
        let preview_lc = doc.preview.to_lowercase();
        let headings_lc: Vec<String> =
            doc.headings.iter().map(|h| h.to_lowercase()).collect();

        for t in &tokens {
            if domain_lc.contains(t) {
                score += 10;
                if best_snippet.is_empty() {
                    best_snippet = format!("파일명에 '{t}' 매치");
                }
            }
            if project_lc.contains(t) {
                score += 5;
            }
            for (hi, h) in headings_lc.iter().enumerate() {
                if h.contains(t) {
                    score += 4;
                    if best_snippet.is_empty() {
                        best_snippet = doc.headings[hi].clone();
                    }
                }
            }
            if preview_lc.contains(t) {
                score += 2;
                if best_snippet.is_empty() {
                    best_snippet = excerpt_around(&doc.preview, t);
                }
            }
        }

        if score > 0 {
            scored.push((score, doc, best_snippet));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(score, doc, snippet)| SearchHit {
            project: doc.project.clone(),
            domain: doc.domain.clone(),
            relevance: (score as f32 / 20.0).min(1.0),
            snippet,
            matched: String::new(),
        })
        .collect()
}

fn excerpt_around(text: &str, needle: &str) -> String {
    excerpt_match(text, needle).unwrap_or_else(|| text.chars().take(80).collect())
}

// ---------- Tier-3: Haiku reranker ----------

const SEARCH_SYSTEM: &str = r####"You are Danbi's vault search ranker.
The user will give you a natural-language query and a JSON index of all
markdown files in their vault. You pick the 1–5 most relevant files and
explain why, tersely.

Reply with ONLY a JSON object matching this schema:

{
  "hits": [
    { "project": string, "domain": string, "relevance": number, "snippet": string }
  ],
  "summary": string
}

Rules:
- "project" and "domain" MUST exactly match entries in the provided index.
- "domain" values always end with ".md".
- "relevance" is 0.0 to 1.0 — how confident the file answers the query.
- "snippet" is a 1-sentence Korean reason citing the file's preview/headings.
- If nothing is clearly relevant, return an empty "hits" array and explain
  in "summary".
- Never invent files. Never output prose outside the JSON.
"####;

fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.trim_start_matches("json").trim_start();
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    t
}

pub async fn search(
    provider: &dyn Provider,
    model_id: &str,
    query: &str,
    index: &[VaultDocIndex],
) -> DanbiResult<SearchResponse> {
    if index.is_empty() {
        return Ok(SearchResponse {
            hits: Vec::new(),
            summary: "vault가 비어있어요.".into(),
        });
    }
    let prompt = serde_json::json!({
        "query": query,
        "index": index,
    })
    .to_string();

    let raw = crate::usage::with_role(
        "search",
        provider.converse_text(model_id, Some(SEARCH_SYSTEM), &prompt, 1024, 0.0),
    )
    .await?;

    let cleaned = strip_code_fence(&raw);
    let parsed: SearchResponse = serde_json::from_str(cleaned).map_err(|e| {
        DanbiError::Other(format!("search JSON parse failed: {e}; raw='{raw}'"))
    })?;

    let valid: std::collections::HashSet<(String, String)> = index
        .iter()
        .map(|d| (d.project.clone(), d.domain.clone()))
        .collect();
    let hits = parsed
        .hits
        .into_iter()
        .filter(|h| valid.contains(&(h.project.clone(), h.domain.clone())))
        .collect();
    Ok(SearchResponse {
        hits,
        summary: parsed.summary,
    })
}

// ─── RRF hybrid search ────────────────────────────────────────────────
// Reciprocal Rank Fusion (Cormack et al., 2009): combine ranked result
// lists by summing 1/(k+rank) across systems. k=60 is the long-standing
// default that works well across heterogeneous retrievers.
//
// In Danbi the two retrievers are:
//   1) BM25 + Korean n-gram (tantivy `full_search`)
//   2) Cosine similarity over Gemini-style embeddings (`vector::query`)
//
// When the user has not configured an embed provider we silently skip
// step 2 — the hybrid call collapses to plain BM25, so callers can always
// route through this function without branching on cfg state.

const RRF_K: f64 = 60.0;

/// Fuse a BM25 hit list with a vector hit list using Reciprocal Rank
/// Fusion. Returned hits carry the BM25 snippet/relevance when available
/// (BM25 is the better source of human-readable excerpts), and otherwise
/// fall back to a placeholder snippet. The final relevance field is the
/// RRF score normalized to [0, 1] over the returned set so callers
/// downstream can keep using it as a confidence-ish number.
pub fn rrf_merge(
    bm25_hits: &[SearchHit],
    vector_hits: &[crate::vector::VectorHit],
    limit: usize,
) -> Vec<SearchHit> {
    if limit == 0 {
        return Vec::new();
    }
    use std::collections::HashMap;
    let mut scores: HashMap<(String, String), f64> = HashMap::new();
    // Keep the first SearchHit we see for each key — BM25 hits are
    // preferred because they include a real snippet.
    let mut seed: HashMap<(String, String), SearchHit> = HashMap::new();

    for (rank, hit) in bm25_hits.iter().enumerate() {
        let key = (hit.project.clone(), hit.domain.clone());
        *scores.entry(key.clone()).or_insert(0.0) +=
            1.0 / (RRF_K + (rank + 1) as f64);
        seed.entry(key).or_insert_with(|| SearchHit {
            matched: "keyword".into(),
            ..hit.clone()
        });
    }
    for (rank, hit) in vector_hits.iter().enumerate() {
        let key = (hit.project.clone(), hit.domain.clone());
        if let Some(existing) = seed.get_mut(&key) {
            if existing.matched == "keyword" {
                existing.matched = "keyword+semantic".into();
            }
        } else {
            seed.insert(
                key.clone(),
                SearchHit {
                    project: hit.project.clone(),
                    domain: hit.domain.clone(),
                    relevance: 0.0,
                    snippet: String::new(),
                    matched: "semantic".into(),
                },
            );
        }
        *scores.entry(key).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f64);
    }

    let mut ranked: Vec<((String, String), f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let max = ranked.first().map(|(_, s)| *s).unwrap_or(1.0).max(1e-9);
    ranked
        .into_iter()
        .take(limit)
        .filter_map(|(key, score)| {
            seed.remove(&key).map(|mut hit| {
                hit.relevance = (score / max) as f32;
                hit
            })
        })
        .collect()
}

/// Convenience: run BM25 and (optionally) vector search, merge with RRF.
/// `vector_query` should be the raw query string already embedded by the
/// caller — embedding is async and lives in the IPC layer, so we keep
/// search.rs sync. Pass `None` to skip the vector arm entirely.
pub fn full_search_hybrid(
    vault: &Path,
    query: &str,
    limit: usize,
    vector_embedding: Option<&[f32]>,
) -> DanbiResult<Vec<SearchHit>> {
    full_search_mode(vault, query, limit, vector_embedding, SearchMode::Hybrid)
}

/// Which retrievers a search runs. `Semantic` without an embedding
/// (no provider, timeout, error) degrades to `Keyword` — callers surface
/// that through `commands::QueryEmbedding::semantic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Keyword,
    Semantic,
    Hybrid,
}

impl SearchMode {
    /// `None`/empty → `Hybrid`; anything else must be one of the three names.
    pub fn parse(raw: Option<&str>) -> DanbiResult<SearchMode> {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("hybrid") => Ok(SearchMode::Hybrid),
            Some("keyword") => Ok(SearchMode::Keyword),
            Some("semantic") => Ok(SearchMode::Semantic),
            Some(other) => Err(DanbiError::Config(format!(
                "mode must be 'hybrid', 'keyword' or 'semantic' (got '{other}')"
            ))),
        }
    }

    pub fn wants_embedding(self) -> bool {
        !matches!(self, SearchMode::Keyword)
    }
}

/// `full_search_hybrid` with an explicit retriever choice. `Keyword`
/// skips the vector arm even when an embedding is supplied; `Semantic`
/// skips BM25 when an embedding is available and otherwise behaves like
/// `Keyword` — also when the vector index has nothing to match (not built
/// yet, or embedded with another model) — so the caller never gets an
/// empty page.
pub fn full_search_mode(
    vault: &Path,
    query: &str,
    limit: usize,
    vector_embedding: Option<&[f32]>,
    mode: SearchMode,
) -> DanbiResult<Vec<SearchHit>> {
    // Pull more candidates per arm than we'll keep — RRF works best when
    // each retriever contributes a fuller list, so depth=3x final limit
    // is a common starting point.
    let depth = limit.saturating_mul(3).max(8);
    let embedding = if mode.wants_embedding() { vector_embedding } else { None };
    let bm25 = if mode == SearchMode::Semantic && embedding.is_some() {
        Vec::new()
    } else {
        full_search(vault, query, depth)?
    };
    let vector_hits = match embedding {
        Some(emb) => {
            let idx = crate::vector::load_cached(vault)
                .unwrap_or_else(|_| Arc::new(crate::vector::VectorIndex::default()));
            crate::vector::query(&idx, emb, depth)
        }
        None => Vec::new(),
    };
    // Semantic with an empty (or other-model) vector index matches nothing;
    // fall back to keyword hits rather than an empty page.
    let bm25 = if bm25.is_empty() && vector_hits.is_empty() && embedding.is_some() {
        full_search(vault, query, depth)?
    } else {
        bm25
    };
    let mut hits = rrf_merge(&bm25, &vector_hits, limit);
    // Semantic-only hits carry no BM25 excerpt; fill one from the file so
    // callers never get an empty snippet to judge the hit by.
    for hit in hits.iter_mut().filter(|h| h.snippet.is_empty()) {
        if let Ok(md) = crate::vault::read_doc(vault, &hit.project, &hit.domain) {
            hit.snippet = excerpt_for_query(&md, query);
        }
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::VectorHit;

    fn temp_vault(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "danbi-search-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(PROJECTS_DIRNAME)).unwrap();
        root
    }

    fn write(vault: &Path, rel: &str, body: &str) {
        let path = vault.join(PROJECTS_DIRNAME).join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// Sorted "project/domain" of a fresh, uncached build — the reference
    /// the incremental path must match exactly.
    fn reference_hits(vault: &Path, query: &str) -> Vec<String> {
        let corpus = build_corpus(vault, snapshot_vault(vault).unwrap()).unwrap();
        let mut ids: Vec<String> = search_corpus(&corpus, query, 50)
            .unwrap()
            .into_iter()
            .map(|h| format!("{}/{}", h.project, h.domain))
            .collect();
        ids.sort();
        ids
    }

    fn live_hits(vault: &Path, query: &str) -> Vec<String> {
        let mut ids: Vec<String> = full_search(vault, query, 50)
            .unwrap()
            .into_iter()
            .map(|h| format!("{}/{}", h.project, h.domain))
            .collect();
        ids.sort();
        ids
    }

    /// Bump mtime past the stamp's millisecond resolution so a same-size
    /// edit done within the same ms is still seen as a change.
    fn touch_old(vault: &Path, rel: &str) {
        let path = vault.join(PROJECTS_DIRNAME).join(rel);
        let t = filetime::FileTime::from_unix_time(1_600_000_000, 0);
        filetime::set_file_mtime(path, t).unwrap();
    }

    #[test]
    fn incremental_refresh_matches_full_rebuild() {
        let v = temp_vault("incr");
        write(&v, "A/one.md", "# 인증\n리프레시 토큰은 7일");
        write(&v, "A/notes/two.md", "# 배포\n릴리스 절차 문서");
        write(&v, "B/three.md", "# 그래프\n커뮤니티 탐지");
        assert_eq!(live_hits(&v, "리프레시"), reference_hits(&v, "리프레시"));
        // The global slot is shared with other tests, so drive the
        // incremental path on a private corpus.
        let first = build_corpus(&v, snapshot_vault(&v).unwrap()).unwrap();

        // Add + modify (same byte length, older mtime) + delete, then every
        // query must agree with a from-scratch build.
        write(&v, "B/four.md", "# 새 문서\n리프레시 토큰 회전");
        write(&v, "A/one.md", "# 인증\n액세스 토큰은 7일");
        touch_old(&v, "A/one.md");
        std::fs::remove_file(v.join(PROJECTS_DIRNAME).join("A/notes/two.md")).unwrap();
        let snap = snapshot_vault(&v).unwrap();
        let (mut upsert, removed) = snap.diff(&first.snapshot);
        upsert.sort();
        assert_eq!(upsert, vec!["A/one.md".to_string(), "B/four.md".to_string()]);
        assert_eq!(removed, vec!["A/notes/two.md".to_string()]);
        let second = update_corpus(&first, snap).unwrap();
        let sorted = |hits: Vec<SearchHit>| {
            let mut ids: Vec<String> = hits
                .into_iter()
                .map(|h| format!("{}/{}", h.project, h.domain))
                .collect();
            ids.sort();
            ids
        };
        for q in ["리프레시", "토큰", "릴리스", "커뮤니티", "액세스"] {
            assert_eq!(sorted(search_corpus(&second, q, 50).unwrap()), reference_hits(&v, q), "query {q}");
            assert_eq!(live_hits(&v, q), reference_hits(&v, q), "live query {q}");
        }
        assert_eq!(sorted(search_corpus(&second, "릴리스", 50).unwrap()), Vec::<String>::new(), "deleted doc gone");
        assert_eq!(sorted(search_corpus(&second, "리프레시", 50).unwrap()), vec!["B/four.md".to_string()]);
        assert_eq!(second.snapshot.files.len(), 3);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn repeated_incremental_edits_do_not_pile_up_segments() {
        let v = temp_vault("segments");
        for i in 0..40 {
            write(&v, &format!("A/doc{i}.md"), &format!("# 문서 {i}\n본문"));
        }
        let mut corpus = build_corpus(&v, snapshot_vault(&v).unwrap()).unwrap();
        // One small edit per refresh to a different doc each time, like a day
        // of autosaves across notes: each refresh adds a one-doc segment that
        // stays live, so only merging keeps the count down.
        for i in 0..40 {
            write(&v, &format!("A/doc{i}.md"), &format!("# 문서 {i}\n편집한 회차 내용 {i}"));
            corpus = update_corpus(&corpus, snapshot_vault(&v).unwrap()).unwrap();
        }
        let segments = corpus.index.searchable_segment_ids().unwrap().len();
        assert!(segments <= 16, "segments kept growing: {segments}");
        assert_eq!(search_corpus(&corpus, "회차", 50).unwrap().len(), 40);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn semantic_mode_falls_back_to_keyword_without_vectors() {
        let v = temp_vault("semantic-empty");
        write(&v, "A/one.md", "# 배포\n릴리스 절차 문서");
        // No vectors.json in this vault: the vector arm matches nothing.
        let hits = full_search_mode(&v, "릴리스", 10, Some(&[0.1, 0.2, 0.3]), SearchMode::Semantic).unwrap();
        assert_eq!(hits.len(), 1);
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn switching_vaults_rebuilds_from_scratch() {
        let a = temp_vault("switch-a");
        write(&a, "P/x.md", "고유단어알파");
        let b = temp_vault("switch-b");
        write(&b, "P/x.md", "고유단어베타");
        assert_eq!(live_hits(&a, "고유단어알파"), vec!["P/x.md".to_string()]);
        assert_eq!(live_hits(&b, "고유단어알파"), Vec::<String>::new());
        assert_eq!(live_hits(&b, "고유단어베타"), vec!["P/x.md".to_string()]);
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    #[test]
    fn snapshot_fingerprint_tracks_content_and_set() {
        let v = temp_vault("fp");
        write(&v, "A/a.md", "hello");
        let fp0 = fingerprint_vault(&v).unwrap();
        assert_eq!(fp0, fingerprint_vault(&v).unwrap(), "stable without changes");
        write(&v, "A/a.md", "hello!");
        let fp1 = fingerprint_vault(&v).unwrap();
        assert_ne!(fp0, fp1, "size change");
        write(&v, "A/.hidden/skip.md", "x");
        write(&v, "A/_assets/skip.md", "x");
        assert_eq!(fp1, fingerprint_vault(&v).unwrap(), "hidden/_assets ignored");
        write(&v, "A/n/d/deep.md", "x");
        let fp2 = fingerprint_vault(&v).unwrap();
        assert_ne!(fp1, fp2, "depth-2 file counts");
        let snap = snapshot_vault(&v).unwrap();
        assert!(snap.files.contains_key("A/n/d/deep.md"));
        assert!(!snap.files.contains_key("A/_assets/skip.md"));
        std::fs::remove_dir_all(v).ok();
    }

    #[test]
    fn search_mode_parses_and_rejects() {
        assert_eq!(SearchMode::parse(None).unwrap(), SearchMode::Hybrid);
        assert_eq!(SearchMode::parse(Some(" Keyword ")).unwrap(), SearchMode::Keyword);
        assert_eq!(SearchMode::parse(Some("semantic")).unwrap(), SearchMode::Semantic);
        assert!(SearchMode::parse(Some("vector")).is_err());
        assert!(!SearchMode::Keyword.wants_embedding());
    }

    #[test]
    fn keyword_mode_ignores_embedding_and_semantic_falls_back() {
        let v = temp_vault("mode");
        write(&v, "A/k.md", "키워드전용문서");
        let emb = vec![1.0f32, 0.0];
        let kw = full_search_mode(&v, "키워드전용문서", 5, Some(&emb), SearchMode::Keyword).unwrap();
        assert_eq!(kw.len(), 1);
        assert_eq!(kw[0].matched, "keyword");
        // No vector index on disk → semantic mode still returns BM25 hits.
        let sem = full_search_mode(&v, "키워드전용문서", 5, None, SearchMode::Semantic).unwrap();
        assert_eq!(sem.len(), 1);
        std::fs::remove_dir_all(v).ok();
    }

    fn bm25(project: &str, domain: &str) -> SearchHit {
        SearchHit {
            project: project.into(),
            domain: domain.into(),
            relevance: 1.0,
            snippet: "… hit …".into(),
            matched: String::new(),
        }
    }

    fn vec_hit(project: &str, domain: &str, score: f32) -> VectorHit {
        VectorHit {
            project: project.into(),
            domain: domain.into(),
            score,
        }
    }

    #[test]
    fn excerpt_centers_on_korean_match() {
        let body = "가".repeat(100) + "리프레시 토큰은 7일로 정했다" + &"나".repeat(100);
        let ex = excerpt_for_query(&body, "리프레시");
        assert!(ex.contains("리프레시 토큰은 7일로 정했다"), "{ex}");
        // 20 chars of lead-in, not hundreds of bytes' worth.
        assert!(ex.starts_with(&format!("… {}리프레시", "가".repeat(20))), "{ex}");
    }

    #[test]
    fn excerpt_falls_back_to_head_without_match() {
        let ex = excerpt_for_query("첫 줄입니다\n둘째 줄", "없는단어");
        assert_eq!(ex, "첫 줄입니다\n둘째 줄");
    }

    #[test]
    fn rrf_labels_semantic_only_hits() {
        let merged = rrf_merge(&[], &[vec_hit("A", "close.md", 0.55)], 8);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].matched, "semantic");
        assert!(merged[0].snippet.is_empty(), "filled later from the file");
    }

    #[test]
    fn rrf_keeps_keyword_hits_and_labels_overlap() {
        let merged = rrf_merge(
            &[bm25("A", "both.md"), bm25("A", "kw.md")],
            // A weak vector score never removes a keyword hit.
            &[vec_hit("A", "both.md", 0.05)],
            8,
        );
        let both = merged.iter().find(|h| h.domain == "both.md").unwrap();
        let kw = merged.iter().find(|h| h.domain == "kw.md").unwrap();
        assert_eq!(both.matched, "keyword+semantic");
        assert_eq!(kw.matched, "keyword");
        assert_eq!(merged[0].domain, "both.md");
    }

    #[test]
    fn focused_excerpt_picks_matching_sections() {
        let md = "# 제목\n소개\n## 인증\nJWT refresh 7일\n## 배포\n릴리스 절차\n";
        let (ex, complete) = focused_excerpt(md, "refresh", 1000);
        assert!(ex.contains("## 인증"));
        assert!(!ex.contains("## 배포"));
        assert!(!complete);
    }

    #[test]
    fn focused_excerpt_whole_doc_is_complete() {
        let md = "# 제목\r\n본문\r\n";
        let (ex, complete) = focused_excerpt(md, "없음", 1000);
        assert_eq!(ex, md);
        assert!(complete);
    }

    #[test]
    fn focused_excerpt_windows_long_section_around_match() {
        let md = format!("{}refresh token 7일{}", "가".repeat(3000), "나".repeat(3000));
        let (ex, complete) = focused_excerpt(&md, "refresh", 500);
        assert!(ex.contains("refresh token 7일"), "{ex}");
        assert!(ex.ends_with("…(생략)"));
        assert!(!complete);
    }

    #[test]
    fn focused_excerpt_ignores_code_comments_and_tags() {
        let md = "## 설치\n```sh\n# refresh cache\nrun\n```\n#태그\n## 기타\n없음\n";
        let (ex, _) = focused_excerpt(md, "refresh", 1000);
        assert!(ex.starts_with("## 설치"), "{ex}");
        assert!(ex.contains("run\n```"));
        assert!(!ex.contains("## 기타"));
    }

    #[test]
    fn focused_excerpt_respects_budget() {
        let md = format!("# A\n{}\n", "가".repeat(500));
        let (ex, complete) = focused_excerpt(&md, "없음", 100);
        assert!(ex.ends_with("…(생략)"));
        assert!(ex.chars().count() <= 100 + "\n…(생략)".chars().count());
        assert!(!complete);
    }
}
