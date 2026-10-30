//! Light ontology — document types, typed relations and status, all read
//! from the markdown itself so the vault stays the single source of truth.
//!
//!   - **Type**   frontmatter `type: decision`, else inferred from the folder
//!                (`decisions/` → decision, `daily/` → daily, …).
//!   - **Status** frontmatter `status: active | superseded | done`; a doc
//!                that another doc `supersedes` is *effectively* superseded.
//!   - **Relation** a link's meaning, from the line it sits on:
//!                `원인:: [[x]]` (Dataview inline field), a frontmatter list
//!                (`affects: ["[[x]]"]`), or a `## 관련` section.
//!
//! Vocabulary = built-in defaults + whatever the project's `schema.md`
//! adds in a ```yaml fence under an `온톨로지` / `ontology` heading. Nothing
//! here calls an LLM and nothing is enforced on write — unknown values are
//! reported by `healing`, never rejected from a user's own file.

use crate::vault::PROJECTS_DIRNAME;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

// ---- Vocabulary ----

#[derive(Debug, Serialize, Clone)]
pub struct TypeDef {
    pub name: String,
    pub ko: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct RelationDef {
    pub name: String,
    pub ko: Option<String>,
    /// Name of the relation seen from the target's side. `relates_to` is
    /// its own inverse.
    pub inverse: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct Vocabulary {
    pub types: Vec<TypeDef>,
    pub relations: Vec<RelationDef>,
    pub statuses: Vec<TypeDef>,
    /// Extra folder → type mappings declared in schema.md (`folders:`).
    pub folders: BTreeMap<String, String>,
    /// Project whose schema this one inherited from, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inherited_from: Option<String>,
}

const DEFAULT_TYPES: [(&str, &str); 8] = [
    ("decision", "결정"),
    ("bug", "버그"),
    ("feature", "기능"),
    ("note", "메모"),
    ("reference", "참고"),
    ("person", "인물"),
    ("daily", "일지"),
    ("meta", "메타"),
];

/// (name, ko, inverse, inverse_ko)
const DEFAULT_RELATIONS: [(&str, &str, &str, &str); 7] = [
    ("affects", "영향", "affected_by", "영향받음"),
    ("caused_by", "원인", "causes", "결과"),
    ("supersedes", "대체", "superseded_by", "대체됨"),
    ("relates_to", "관련", "relates_to", "관련"),
    ("implements", "구현", "implemented_by", "구현됨"),
    ("blocks", "차단", "blocked_by", "차단됨"),
    ("part_of", "부분", "has_part", "포함"),
];

/// Other spellings of the default types. `project` / `feedback` / `user`
/// come from Claude Code memory files (`type: user|feedback|project|reference`)
/// that already live in many vaults — they're notes, not vocabulary errors.
const TYPE_ALIASES: [(&str, &str); 7] = [
    ("project", "note"),
    ("feedback", "note"),
    ("memo", "note"),
    ("user", "person"),
    ("adr", "decision"),
    ("issue", "bug"),
    ("journal", "daily"),
];

const DEFAULT_STATUSES: [(&str, &str); 3] = [("active", "유효"), ("superseded", "대체됨"), ("done", "완료")];

/// Extra spellings people already use for `relates_to` in plain lines
/// (`관련: [[x]]`, `## 관련 문서`).
const RELATES_ALIASES: [&str; 6] = ["관련 문서", "관련 링크", "관련 노트", "related", "see also", "참고 문서"];

impl Default for Vocabulary {
    fn default() -> Self {
        let mut relations = Vec::new();
        for (name, ko, inv, inv_ko) in DEFAULT_RELATIONS {
            relations.push(RelationDef {
                name: name.into(),
                ko: Some(ko.into()),
                inverse: inv.into(),
            });
            if inv != name {
                relations.push(RelationDef {
                    name: inv.into(),
                    ko: Some(inv_ko.into()),
                    inverse: name.into(),
                });
            }
        }
        Vocabulary {
            types: DEFAULT_TYPES
                .iter()
                .map(|(n, k)| TypeDef { name: (*n).into(), ko: Some((*k).into()) })
                .collect(),
            relations,
            statuses: DEFAULT_STATUSES
                .iter()
                .map(|(n, k)| TypeDef { name: (*n).into(), ko: Some((*k).into()) })
                .collect(),
            folders: BTreeMap::new(),
            inherited_from: None,
        }
    }
}

fn norm(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_lowercase()
}

impl Vocabulary {
    pub fn canonical_type(&self, raw: &str) -> Option<String> {
        let n = norm(raw);
        if let Some(t) = self
            .types
            .iter()
            .find(|t| t.name == n || t.ko.as_deref().map(norm).as_deref() == Some(n.as_str()))
        {
            return Some(t.name.clone());
        }
        TYPE_ALIASES
            .iter()
            .find(|(alias, _)| *alias == n)
            .map(|(_, to)| (*to).to_string())
    }

    pub fn canonical_status(&self, raw: &str) -> Option<String> {
        let n = norm(raw);
        self.statuses
            .iter()
            .find(|t| t.name == n || t.ko.as_deref().map(norm).as_deref() == Some(n.as_str()))
            .map(|t| t.name.clone())
    }

    pub fn canonical_relation(&self, raw: &str) -> Option<String> {
        let n = norm(raw).replace(['-', ' '], "_");
        let spaced = norm(raw);
        if let Some(r) = self.relations.iter().find(|r| {
            r.name == n || r.ko.as_deref().map(norm).as_deref() == Some(spaced.as_str())
        }) {
            return Some(r.name.clone());
        }
        if RELATES_ALIASES.contains(&spaced.as_str()) {
            return Some("relates_to".into());
        }
        None
    }

    pub fn inverse(&self, relation: &str) -> String {
        self.relations
            .iter()
            .find(|r| r.name == relation)
            .map(|r| r.inverse.clone())
            .unwrap_or_else(|| relation.to_string())
    }

    pub fn relation_ko(&self, relation: &str) -> String {
        self.relations
            .iter()
            .find(|r| r.name == relation)
            .and_then(|r| r.ko.clone())
            .unwrap_or_else(|| relation.to_string())
    }

    /// Merge the additions a schema.md fence declares. Defaults are never
    /// removed — a project can only widen the vocabulary.
    fn extend(&mut self, ext: &SchemaFence) {
        for (name, ko) in &ext.types {
            let n = norm(name);
            if n.is_empty() {
                continue;
            }
            match self.types.iter_mut().find(|t| t.name == n) {
                Some(t) => {
                    if ko.is_some() {
                        t.ko = ko.clone();
                    }
                }
                None => self.types.push(TypeDef { name: n, ko: ko.clone() }),
            }
        }
        for r in &ext.relations {
            let name = norm(&r.name).replace(['-', ' '], "_");
            if name.is_empty() {
                continue;
            }
            let inverse = r
                .inverse
                .as_deref()
                .map(|s| norm(s).replace(['-', ' '], "_"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| name.clone());
            self.upsert_relation(&name, r.ko.clone(), &inverse);
            if inverse != name {
                self.upsert_relation(&inverse, r.inverse_ko.clone(), &name);
            }
        }
        for (folder, ty) in &ext.folders {
            let ty = norm(ty);
            if self.canonical_type(&ty).is_none() {
                self.types.push(TypeDef { name: ty.clone(), ko: None });
            }
            self.folders.insert(norm(folder), ty);
        }
    }

    fn upsert_relation(&mut self, name: &str, ko: Option<String>, inverse: &str) {
        match self.relations.iter_mut().find(|r| r.name == name) {
            Some(r) => {
                if ko.is_some() {
                    r.ko = ko;
                }
                r.inverse = inverse.to_string();
            }
            None => self.relations.push(RelationDef {
                name: name.to_string(),
                ko,
                inverse: inverse.to_string(),
            }),
        }
    }
}

// ---- schema.md fence ----

#[derive(Debug, Default, Clone)]
struct FenceRelation {
    name: String,
    ko: Option<String>,
    inverse: Option<String>,
    inverse_ko: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct SchemaFence {
    types: Vec<(String, Option<String>)>,
    relations: Vec<FenceRelation>,
    folders: Vec<(String, String)>,
    inherit: Option<String>,
}

/// The ```yaml block under the first `온톨로지` / `ontology` heading.
fn ontology_fence(schema_md: &str) -> Option<String> {
    let mut in_section = false;
    let mut in_fence = false;
    let mut body = String::new();
    for line in schema_md.lines() {
        let t = line.trim();
        if in_fence {
            if t.starts_with("```") {
                return Some(body);
            }
            body.push_str(line);
            body.push('\n');
            continue;
        }
        if t.starts_with('#') {
            let lower = t.to_lowercase();
            in_section = lower.contains("온톨로지") || lower.contains("ontology");
            continue;
        }
        if in_section && t.starts_with("```") {
            in_fence = true;
        }
    }
    None
}

/// `[a, b]` → ["a", "b"]; a bare scalar → [scalar].
fn flow_list(v: &str) -> Vec<String> {
    let v = v.trim();
    let inner = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(v);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `{ko: 영향, inverse: affected_by}` → map. A bare scalar becomes `ko`.
fn flow_map(v: &str) -> HashMap<String, String> {
    let v = v.trim();
    let mut out = HashMap::new();
    let Some(inner) = v.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        if !v.is_empty() {
            out.insert("ko".into(), v.trim_matches(|c| c == '"' || c == '\'').to_string());
        }
        return out;
    };
    for part in inner.split(',') {
        if let Some((k, val)) = part.split_once(':') {
            out.insert(
                k.trim().to_string(),
                val.trim().trim_matches(|c| c == '"' || c == '\'').to_string(),
            );
        }
    }
    out
}

/// Tiny YAML subset: top-level `key: value`, flow lists/maps, and one level
/// of indented `sub: value` / `- item` under an empty top-level key.
fn parse_fence(src: &str) -> SchemaFence {
    let mut out = SchemaFence::default();
    let mut current: Option<String> = None;
    for raw in src.lines() {
        let line = raw.split(" #").next().unwrap_or("");
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let t = line.trim();
        if !indented {
            let Some((k, v)) = t.split_once(':') else { continue };
            let key = k.trim().to_lowercase();
            let v = v.trim();
            current = Some(key.clone());
            match key.as_str() {
                "types" if !v.is_empty() => {
                    out.types.extend(flow_list(v).into_iter().map(|n| (n, None)));
                }
                "inherit" | "extends" if !v.is_empty() => {
                    out.inherit = Some(v.trim_matches(|c| c == '"' || c == '\'').to_string());
                }
                _ => {}
            }
            continue;
        }
        let section = current.as_deref().unwrap_or("");
        if let Some(item) = t.strip_prefix("- ") {
            if section == "types" {
                out.types.push((item.trim().to_string(), None));
            }
            continue;
        }
        let Some((k, v)) = t.split_once(':') else { continue };
        let (k, v) = (k.trim().to_string(), v.trim());
        match section {
            "types" => {
                let ko = flow_map(v).remove("ko");
                out.types.push((k, ko));
            }
            "relations" => {
                let mut m = flow_map(v);
                out.relations.push(FenceRelation {
                    name: k,
                    ko: m.remove("ko"),
                    inverse: m.remove("inverse"),
                    inverse_ko: m.remove("inverse_ko"),
                });
            }
            "folders" => out.folders.push((k, v.trim_matches(|c| c == '"' || c == '\'').to_string())),
            _ => {}
        }
    }
    out
}

fn read_schema_fence(vault: &Path, project: &str) -> Option<SchemaFence> {
    let path = vault.join(PROJECTS_DIRNAME).join(project).join("schema.md");
    let md = std::fs::read_to_string(path).ok()?;
    ontology_fence(&md).map(|f| parse_fence(&f))
}

/// Defaults + the project's schema.md additions. `inherit: <project>` pulls
/// that project's additions in first (one level — no chains, no cycles).
pub fn vocabulary_for(vault: &Path, project: &str) -> Vocabulary {
    let mut vocab = Vocabulary::default();
    let Some(own) = read_schema_fence(vault, project) else {
        return vocab;
    };
    if let Some(parent) = own.inherit.as_deref().filter(|p| *p != project) {
        if let Some(base) = read_schema_fence(vault, parent) {
            vocab.extend(&base);
            vocab.inherited_from = Some(parent.to_string());
        }
    }
    vocab.extend(&own);
    vocab
}

/// Markdown block for schema.md's `## 온톨로지` section — what
/// `compose_purpose_schema` writes and what agents are shown as the format.
pub fn schema_section_template() -> &'static str {
    "## 온톨로지\n\n\
문서 종류(`type:`)·관계(`관계:: [[문서]]`)·상태(`status:`) 단어 목록. 기본 목록은 단비에 내장돼 있고, \
아래 블록은 이 프로젝트에서 **추가**할 것만 적는다.\n\n\
```yaml\n\
types: []            # 예: [experiment, meeting]\n\
relations:\n\
  # tested_by: {ko: 검증, inverse: tests, inverse_ko: 검증함}\n\
folders:\n\
  # experiments: experiment\n\
# inherit: 다른프로젝트   # 그 프로젝트의 추가 목록을 같이 쓴다\n\
```\n"
}

// ---- Frontmatter ----

#[derive(Debug, Clone)]
pub struct FmEntry {
    pub key: String,
    /// Inline value after `key:` (raw, untrimmed of quotes).
    pub value: String,
    /// Block-list items under the key (`  - x`), raw.
    pub items: Vec<String>,
}

/// Line range of the frontmatter block: (first content line, closing `---`
/// line), both 0-based indices into `md.lines()`. None when the doc has no
/// frontmatter.
fn frontmatter_bounds(lines: &[&str]) -> Option<(usize, usize)> {
    let first = lines.first()?.trim_start_matches('\u{feff}').trim_end();
    if first != "---" {
        return None;
    }
    let close = lines.iter().skip(1).position(|l| l.trim_end() == "---")? + 1;
    // A `---` horizontal rule opening the body, with another rule further
    // down, is not frontmatter: every top-level line between must look
    // like `key: …`.
    let yaml_like = lines[1..close].iter().all(|l| {
        let t = l.trim();
        t.is_empty()
            || t.starts_with('#')
            || l.starts_with(' ')
            || l.starts_with('\t')
            || t.starts_with("- ")
            || t.split_once(':').is_some_and(|(k, _)| {
                let k = k.trim().trim_matches(|c| c == '"' || c == '\'');
                !k.is_empty() && k.chars().count() <= 64
            })
    });
    yaml_like.then_some((1, close))
}

pub fn parse_frontmatter(md: &str) -> Vec<FmEntry> {
    let lines: Vec<&str> = md.lines().collect();
    let Some((start, end)) = frontmatter_bounds(&lines) else {
        return Vec::new();
    };
    let mut out: Vec<FmEntry> = Vec::new();
    for line in &lines[start..end] {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !indented && !t.starts_with("- ") {
            if let Some((k, v)) = t.split_once(':') {
                out.push(FmEntry {
                    key: k.trim().to_string(),
                    value: v.trim().to_string(),
                    items: Vec::new(),
                });
            }
            continue;
        }
        if let Some(item) = t.strip_prefix("- ") {
            if let Some(last) = out.last_mut() {
                last.items.push(item.trim().to_string());
            }
        }
    }
    out
}

fn fm_scalar(entries: &[FmEntry], key: &str) -> Option<String> {
    entries
        .iter()
        .find(|e| e.key.eq_ignore_ascii_case(key))
        .map(|e| {
            let v = e.value.split(" #").next().unwrap_or("");
            v.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string()
        })
        .filter(|v| !v.is_empty())
}

// ---- Document meta ----

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TypeSource {
    /// `type:` in frontmatter.
    Declared,
    /// Inferred from the folder or file name.
    Folder,
}

#[derive(Debug, Serialize, Clone)]
pub struct DocMeta {
    /// Canonical type name, or the raw declared value when it's outside the
    /// vocabulary (`type_known` is then false).
    pub doc_type: String,
    pub type_source: TypeSource,
    pub type_known: bool,
    /// Declared `status:` (canonical when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Declared status, or `superseded` when another doc supersedes this
    /// one. Filled by `links::build_index`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_status: Option<String>,
}

const META_FILES: [&str; 10] = [
    "purpose.md",
    "schema.md",
    "skill.md",
    "memory_index.md",
    "memory.md",
    "index.md",
    "readme.md",
    "claude.md",
    "agents.md",
    "log.md",
];

/// Type a document gets from where it lives when frontmatter says nothing.
pub fn infer_type(vocab: &Vocabulary, domain: &str) -> String {
    let lower = domain.to_lowercase();
    let (folder, file) = match lower.split_once('/') {
        Some((f, rest)) => (Some(f.to_string()), rest.rsplit('/').next().unwrap_or(rest).to_string()),
        None => (None, lower.clone()),
    };
    let Some(folder) = folder else {
        return if META_FILES.contains(&file.as_str()) { "meta" } else { "note" }.into();
    };
    if let Some(t) = vocab.folders.get(&folder) {
        return t.clone();
    }
    let by_folder = match folder.as_str() {
        "daily" | "journal" | "일지" => Some("daily"),
        "decisions" | "decision" | "adr" | "adrs" | "결정" => Some("decision"),
        "bugs" | "bug" | "issues" | "incidents" | "버그" => Some("bug"),
        "features" | "feature" | "specs" | "기능" => Some("feature"),
        "references" | "reference" | "refs" | "ref" | "docs" | "참고" => Some("reference"),
        "people" | "persons" | "person" | "인물" => Some("person"),
        "notes" | "note" | "메모" => Some("note"),
        _ => None,
    };
    if let Some(t) = by_folder {
        return t.into();
    }
    // A singular/plural folder named after a vocabulary type.
    let singular = folder.strip_suffix('s').unwrap_or(&folder);
    vocab
        .canonical_type(&folder)
        .or_else(|| vocab.canonical_type(singular))
        .unwrap_or_else(|| "note".into())
}

pub fn doc_meta(vocab: &Vocabulary, domain: &str, md: &str) -> DocMeta {
    let fm = parse_frontmatter(md);
    let declared = fm_scalar(&fm, "type");
    let (doc_type, type_source, type_known) = match declared {
        Some(raw) => match vocab.canonical_type(&raw) {
            Some(t) => (t, TypeSource::Declared, true),
            None => (raw, TypeSource::Declared, false),
        },
        None => (infer_type(vocab, domain), TypeSource::Folder, true),
    };
    let status = fm_scalar(&fm, "status").map(|s| vocab.canonical_status(&s).unwrap_or(s));
    DocMeta {
        doc_type,
        type_source,
        type_known,
        effective_status: status.clone(),
        status,
    }
}

// ---- Relation context per line ----

/// Relation every line of `md` gives to a `[[link]]` on it, by line start
/// byte offset. Lines inside code fences give none.
pub struct LineRelations {
    starts: Vec<usize>,
    rels: Vec<Option<String>>,
}

impl LineRelations {
    pub fn at(&self, byte: usize) -> Option<&str> {
        let i = match self.starts.binary_search(&byte) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        self.rels.get(i).and_then(|r| r.as_deref())
    }
}

/// `원인:: …` / `- 영향:: …` → ("원인", rest). Keys stay short and bracket-free.
fn inline_field(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let t = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .unwrap_or(t)
        .trim_start();
    let (key, _) = t.split_once("::")?;
    let key = key.trim();
    if key.is_empty() || key.chars().count() > 30 || key.contains('[') || key.contains('`') {
        return None;
    }
    Some(key)
}

/// `관련: [[x]]` — single-colon form, accepted only for relates_to spellings
/// so prose like `결과: …` doesn't turn into a typed edge.
fn relates_prefix(vocab: &Vocabulary, line: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).unwrap_or(t);
    let Some((key, _)) = t.split_once(':') else { return false };
    if key.contains('[') || key.chars().count() > 12 {
        return false;
    }
    vocab.canonical_relation(key).as_deref() == Some("relates_to")
}

pub fn line_relations(vocab: &Vocabulary, md: &str) -> LineRelations {
    let mut starts = Vec::new();
    let mut rels: Vec<Option<String>> = Vec::new();
    let lines: Vec<&str> = md.split('\n').collect();
    let fm = frontmatter_bounds(&lines);
    let mut fm_key: Option<String> = None;
    let mut fence = false;
    let mut section: Option<(usize, String)> = None; // (heading level, relation)
    let mut pos = 0usize;
    for (i, line) in lines.iter().enumerate() {
        starts.push(pos);
        pos += line.len() + 1;
        // Frontmatter: links under a relation-named key take that relation.
        if let Some((s, e)) = fm {
            if i == 0 || i == e {
                rels.push(None);
                continue;
            }
            if i >= s && i < e {
                let indented = line.starts_with(' ') || line.starts_with('\t');
                let t = line.trim();
                if !indented && !t.starts_with("- ") {
                    fm_key = t.split_once(':').map(|(k, _)| k.trim().to_string());
                }
                rels.push(fm_key.as_deref().and_then(|k| vocab.canonical_relation(k)));
                continue;
            }
        }
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            rels.push(None);
            continue;
        }
        if fence {
            rels.push(None);
            continue;
        }
        if t.starts_with('#') {
            let level = t.chars().take_while(|c| *c == '#').count();
            let title = t[level..].trim().trim_end_matches(':').trim();
            if section.as_ref().is_some_and(|(l, _)| level <= *l) {
                section = None;
            }
            if let Some(r) = vocab.canonical_relation(title) {
                section = Some((level, r));
            }
            rels.push(None);
            continue;
        }
        if let Some(key) = inline_field(line) {
            // A known key types the link; an unknown one leaves it plain
            // (healing reports it) rather than borrowing the section's.
            rels.push(vocab.canonical_relation(key));
            continue;
        }
        if relates_prefix(vocab, line) {
            rels.push(Some("relates_to".into()));
            continue;
        }
        rels.push(section.as_ref().map(|(_, r)| r.clone()));
    }
    LineRelations { starts, rels }
}

impl DocIssues {
    pub fn is_empty(&self) -> bool {
        self.unknown_type.is_none() && self.unknown_status.is_none() && self.unknown_relations.is_empty()
    }
}

/// Problems a document's own markdown shows against the vocabulary:
/// an out-of-vocabulary `type:` / `status:` or inline relation key.
#[derive(Debug, Default, Clone)]
pub struct DocIssues {
    pub unknown_type: Option<String>,
    pub unknown_status: Option<String>,
    pub unknown_relations: Vec<String>,
}

pub fn doc_issues(vocab: &Vocabulary, md: &str) -> DocIssues {
    let fm = parse_frontmatter(md);
    let mut out = DocIssues::default();
    if let Some(t) = fm_scalar(&fm, "type") {
        if vocab.canonical_type(&t).is_none() {
            out.unknown_type = Some(t);
        }
    }
    if let Some(s) = fm_scalar(&fm, "status") {
        if vocab.canonical_status(&s).is_none() {
            out.unknown_status = Some(s);
        }
    }
    let mut seen = HashSet::new();
    let mut fence = false;
    let lines: Vec<&str> = md.lines().collect();
    let body_from = frontmatter_bounds(&lines).map(|(_, e)| e + 1).unwrap_or(0);
    for line in &lines[body_from.min(lines.len())..] {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if let Some(key) = inline_field(line) {
            // Only flag keys that point at a link — `key:: value` text fields
            // are ordinary Dataview properties, not relations.
            if line.contains("[[") && vocab.canonical_relation(key).is_none() && seen.insert(key.to_string()) {
                out.unknown_relations.push(key.to_string());
            }
        }
    }
    out
}

// ---- Frontmatter writes (danbi_set_meta) ----

/// What `set_frontmatter` changes. Everything else in the block — other
/// keys, their order, comments, odd YAML — is left byte-for-byte.
#[derive(Debug, Default, Clone)]
pub struct MetaUpdate {
    pub set: Vec<(String, String)>,
    /// (key, item) appended to a list key; skipped when already present.
    pub add_items: Vec<(String, String)>,
}

fn yaml_scalar(v: &str) -> String {
    let needs_quotes = v.is_empty()
        || v.contains(": ")
        || v.contains(" #")
        || v.starts_with(['[', '{', '"', '\'', '!', '&', '*', '|', '>', '%', '@', '`']);
    if needs_quotes {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

fn quote_item(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `value # comment` → ("value", " # comment").
fn split_comment(v: &str) -> (&str, &str) {
    match v.find(" #") {
        Some(i) => (&v[..i], &v[i..]),
        None => (v, ""),
    }
}

fn unquote(v: &str) -> &str {
    v.trim().trim_matches(|c| c == '"' || c == '\'').trim()
}

/// Target of a `[[link]]` inside any string, without alias / heading, so
/// `[[b.md|별칭]]` and `"[[b.md]]"` compare equal.
pub fn link_target(s: &str) -> Option<String> {
    let start = s.find("[[")? + 2;
    let end = s[start..].find("]]")? + start;
    let inner = &s[start..end];
    let t = inner.split(['|', '#']).next().unwrap_or("").trim();
    (!t.is_empty()).then(|| md_name(t))
}

/// One line and the newline it ended with ("" for a last line without one).
struct Line {
    text: String,
    end: String,
}

fn split_lines(md: &str) -> Vec<Line> {
    md.split_inclusive('\n')
        .map(|l| {
            let (text, end) = if let Some(t) = l.strip_suffix("\r\n") {
                (t, "\r\n")
            } else if let Some(t) = l.strip_suffix('\n') {
                (t, "\n")
            } else {
                (l, "")
            };
            Line { text: text.to_string(), end: end.to_string() }
        })
        .collect()
}

fn top_key(text: &str) -> Option<&str> {
    let indented = text.starts_with(' ') || text.starts_with('\t');
    if indented || text.trim_start().starts_with("- ") || text.trim_start().starts_with('#') {
        return None;
    }
    text.split_once(':').map(|(k, _)| k.trim())
}

fn same_key(a: &str, b: &str) -> bool {
    unquote(a).eq_ignore_ascii_case(unquote(b))
}

/// Index range `[key_line, end)` of a top-level key and its continuation
/// lines inside the frontmatter `lines[s..e]`. Err when the key appears
/// twice — editing one copy would leave the other contradicting it.
fn key_span(lines: &[Line], s: usize, e: usize, key: &str) -> Result<Option<(usize, usize)>, String> {
    let mut found: Option<(usize, usize)> = None;
    let mut i = s;
    while i < e {
        if top_key(&lines[i].text).is_some_and(|k| same_key(k, key)) {
            if found.is_some() {
                return Err(format!("frontmatter 에 `{key}` 키가 두 번 있어요 — 하나로 정리한 뒤 다시 시도하세요."));
            }
            let mut j = i + 1;
            while j < e {
                let n = &lines[j].text;
                let cont = n.starts_with(' ') || n.starts_with('\t') || n.trim_start().starts_with("- ");
                if !cont {
                    break;
                }
                j += 1;
            }
            found = Some((i, j));
            i = j;
            continue;
        }
        i += 1;
    }
    Ok(found)
}

/// Rewrites only the keys in `update`. Every other byte — other keys,
/// comments, the body, each line's own newline, a BOM — is kept. Err for
/// shapes it won't guess at (duplicate keys, a flow map where a list goes).
pub fn set_frontmatter(md: &str, update: &MetaUpdate) -> Result<String, String> {
    let mut lines = split_lines(md);
    let nl = lines
        .iter()
        .find(|l| !l.end.is_empty())
        .map(|l| l.end.clone())
        .unwrap_or_else(|| "\n".into());
    let line = |text: String| Line { text, end: nl.clone() };
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let (s, mut e) = match frontmatter_bounds(&texts) {
        Some(b) => b,
        None => {
            // New block on top; a BOM stays the file's first character.
            let bom = md.starts_with('\u{feff}');
            if bom {
                lines[0].text = lines[0].text.trim_start_matches('\u{feff}').to_string();
            }
            let had_body = lines.first().is_some_and(|l| !l.text.trim().is_empty());
            let mut head = vec![line(if bom { "\u{feff}---".into() } else { "---".into() }), line("---".into())];
            if had_body {
                head.push(line(String::new()));
            }
            lines.splice(0..0, head);
            (1, 1)
        }
    };

    for (key, value) in &update.set {
        match key_span(&lines, s, e, key)? {
            Some((a, b)) => {
                let text = lines[a].text.clone();
                let (orig_key, rest) = text.split_once(':').unwrap_or((key.as_str(), ""));
                let (cur, comment) = split_comment(rest);
                if b == a + 1 && unquote(cur) == value {
                    continue;
                }
                let new_text = format!("{}: {}{}", orig_key.trim_end(), yaml_scalar(value), comment);
                let end = lines[b - 1].end.clone();
                lines.splice(a..b, std::iter::once(Line { text: new_text, end }));
                e -= b - a - 1;
            }
            None => {
                lines.insert(e, line(format!("{key}: {}", yaml_scalar(value))));
                e += 1;
            }
        }
    }

    for (key, item) in &update.add_items {
        let quoted = quote_item(item);
        let want = link_target(item);
        match key_span(&lines, s, e, key)? {
            Some((a, b)) => {
                let present = lines[a..b].iter().any(|l| match (&want, link_target(&l.text)) {
                    (Some(w), Some(t)) => *w == t || l.text.contains(item.as_str()),
                    _ => l.text.contains(item.as_str()),
                });
                if present {
                    continue;
                }
                let text = lines[a].text.clone();
                let (orig_key, rest) = text.split_once(':').unwrap_or((key.as_str(), ""));
                let (inline, comment) = split_comment(rest);
                let inline = inline.trim();
                if inline.starts_with('{') {
                    return Err(format!("`{key}` 가 맵 형태라 관계 리스트로 바꿀 수 없어요."));
                }
                if b > a + 1 || inline.is_empty() {
                    // Block list (or empty key): one more `- item`, indented
                    // like the items already there.
                    let indent = (b > a + 1)
                        .then(|| {
                            let l = &lines[a + 1].text;
                            l[..l.len() - l.trim_start().len()].to_string()
                        })
                        .unwrap_or_else(|| "  ".into());
                    lines.insert(b, line(format!("{indent}- {quoted}")));
                    e += 1;
                } else if let Some(inner) = inline.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                    let inner = inner.trim();
                    let joined = if inner.is_empty() { quoted } else { format!("{inner}, {quoted}") };
                    lines[a].text = format!("{}: [{joined}]{comment}", orig_key.trim_end());
                } else {
                    // A scalar becomes a two-item block list.
                    let end = lines[a].end.clone();
                    let k = orig_key.trim_end().to_string();
                    lines.splice(
                        a..b,
                        [
                            line(format!("{k}:{comment}")),
                            line(format!("  - {inline}")),
                            Line { text: format!("  - {quoted}"), end },
                        ],
                    );
                    e += 2;
                }
            }
            None => {
                lines.insert(e, line(format!("{key}:")));
                lines.insert(e + 1, line(format!("  - {quoted}")));
                e += 2;
            }
        }
    }
    let _ = e;
    Ok(lines.iter().map(|l| format!("{}{}", l.text, l.end)).collect())
}

/// Appends `- item` to the document's `## 관련` section (at the end of that
/// section, not the end of the file), creating the section when missing.
/// Keeps the document's newline style and its trailing whitespace.
pub fn append_related_item(md: &str, item: &str) -> String {
    let lines = split_lines(md);
    let nl = lines
        .iter()
        .find(|l| !l.end.is_empty())
        .map(|l| l.end.clone())
        .unwrap_or_else(|| "\n".into());
    let mut fence = false;
    let mut section: Option<usize> = None;
    let mut insert_at: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        let t = l.text.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence || !t.starts_with('#') {
            continue;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        let title = t[level..].trim();
        if section.is_some() && insert_at.is_none() && level <= 2 {
            insert_at = Some(i);
        }
        if section.is_none() && level == 2 && title == "관련" {
            section = Some(i);
        }
    }
    let new_item = format!("- {item}");
    match section {
        Some(h) => {
            let stop = insert_at.unwrap_or(lines.len());
            // After the section's last non-blank line.
            let mut at = stop;
            while at > h + 1 && lines[at - 1].text.trim().is_empty() {
                at -= 1;
            }
            let mut out: Vec<Line> = lines;
            if at > 0 && out[at - 1].end.is_empty() {
                out[at - 1].end = nl.clone();
            }
            out.insert(at, Line { text: new_item, end: nl.clone() });
            out.iter().map(|l| format!("{}{}", l.text, l.end)).collect()
        }
        None => {
            let body = md.trim_end_matches(|c: char| c == '\n' || c == '\r');
            if body.trim().is_empty() {
                return format!("## 관련{nl}{nl}{new_item}{nl}");
            }
            format!("{body}{nl}{nl}## 관련{nl}{nl}{new_item}{nl}")
        }
    }
}

// ---- Queries (MCP danbi_query / danbi_schema, the table view) ----

#[derive(Debug, Default, Clone)]
pub struct QueryArgs {
    pub project: Option<String>,
    pub doc_type: Option<String>,
    pub status: Option<String>,
    pub relation: Option<String>,
    /// "project/domain" or a domain inside `project`.
    pub target: Option<String>,
    pub include_daily: bool,
    pub limit: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct RelationRef {
    pub relation: String,
    pub doc: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DocRow {
    pub project: String,
    pub domain: String,
    pub doc_type: String,
    pub type_source: TypeSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Typed links seen from this document — outgoing as written, incoming
    /// as their inverse.
    pub relations: Vec<RelationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<u128>,
}

fn md_name(d: &str) -> String {
    if d.to_lowercase().ends_with(".md") {
        d.to_string()
    } else {
        format!("{d}.md")
    }
}

/// "P/x.md" stays; "notes/x.md" or "x.md" is read inside `project`.
fn resolve_target(vault: &Path, project: Option<&str>, raw: &str) -> Option<String> {
    let t = raw.trim().trim_start_matches("[[").trim_end_matches("]]").trim();
    if t.is_empty() {
        return None;
    }
    let t = md_name(t);
    if let Some((first, _)) = t.split_once('/') {
        if vault.join(PROJECTS_DIRNAME).join(first).is_dir() {
            return Some(t);
        }
    }
    project.map(|p| format!("{p}/{t}"))
}

fn modified_ms(vault: &Path, id: &str) -> Option<u128> {
    let meta = std::fs::metadata(vault.join(PROJECTS_DIRNAME).join(id)).ok()?;
    let m = meta.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis())
}

/// Typed relations of one document as seen from it.
pub fn relations_of(index: &crate::links::LinkIndex, vocab: &Vocabulary, id: &str) -> Vec<RelationRef> {
    let mut out: Vec<RelationRef> = Vec::new();
    let mut push = |relation: String, doc: String| {
        if !out.iter().any(|r| r.relation == relation && r.doc == doc) {
            out.push(RelationRef { relation, doc });
        }
    };
    for l in index.outgoing.get(id).into_iter().flatten() {
        if let Some(r) = &l.relation {
            push(r.clone(), format!("{}/{}", l.project, l.domain));
        }
    }
    for l in index.incoming.get(id).into_iter().flatten() {
        if let Some(r) = &l.relation {
            push(vocab.inverse(r), format!("{}/{}", l.project, l.domain));
        }
    }
    out
}

/// Documents by type / status / relation. Everything comes from the cached
/// link index — no LLM, no extra file reads beyond mtimes of the hits.
pub fn query(vault: &Path, args: &QueryArgs) -> crate::error::DanbiResult<(Vec<DocRow>, usize)> {
    let index = crate::links::build_index_cached(vault)?;
    let mut vocabs: HashMap<String, Vocabulary> = HashMap::new();
    let target = args
        .target
        .as_deref()
        .and_then(|t| resolve_target(vault, args.project.as_deref(), t));
    let want_daily = args.include_daily
        || args.doc_type.as_deref().map(norm).as_deref() == Some("daily")
        || args.doc_type.as_deref().map(norm).as_deref() == Some("일지");

    let mut rows: Vec<DocRow> = Vec::new();
    for (id, meta) in index.meta.iter() {
        let Some((project, domain)) = id.split_once('/') else { continue };
        if args.project.as_deref().is_some_and(|p| p != project) {
            continue;
        }
        let vocab = vocabs
            .entry(project.to_string())
            .or_insert_with(|| vocabulary_for(vault, project));
        if meta.doc_type == "daily" && !want_daily {
            continue;
        }
        if let Some(t) = &args.doc_type {
            let want = vocab.canonical_type(t).unwrap_or_else(|| norm(t));
            if meta.doc_type != want {
                continue;
            }
        }
        if let Some(st) = &args.status {
            let want = vocab.canonical_status(st).unwrap_or_else(|| norm(st));
            // A doc that declares nothing counts as active unless something
            // supersedes it.
            let have = meta.effective_status.clone().unwrap_or_else(|| "active".into());
            if have != want {
                continue;
            }
        }
        let relations = relations_of(&index, vocab, id);
        if let Some(r) = &args.relation {
            let want = vocab.canonical_relation(r).unwrap_or_else(|| norm(r));
            let hit = relations
                .iter()
                .any(|x| x.relation == want && target.as_ref().map_or(true, |t| &x.doc == t));
            if !hit {
                continue;
            }
        } else if let Some(t) = &target {
            if !relations.iter().any(|x| &x.doc == t) {
                continue;
            }
        }
        rows.push(DocRow {
            project: project.to_string(),
            domain: domain.to_string(),
            doc_type: meta.doc_type.clone(),
            type_source: meta.type_source.clone(),
            status: meta.effective_status.clone(),
            relations,
            modified_ms: None,
        });
    }
    let total = rows.len();
    for r in rows.iter_mut() {
        r.modified_ms = modified_ms(vault, &format!("{}/{}", r.project, r.domain));
    }
    rows.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then_with(|| a.domain.cmp(&b.domain)));
    rows.truncate(args.limit.max(1));
    Ok((rows, total))
}

/// Vocabulary + how many documents of each type the project has — what an
/// agent needs before typing anything.
pub fn schema_summary(vault: &Path, project: &str) -> crate::error::DanbiResult<serde_json::Value> {
    let vocab = vocabulary_for(vault, project);
    let index = crate::links::build_index_cached(vault)?;
    let prefix = format!("{project}/");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut declared = 0usize;
    let mut typed_links = 0usize;
    for (id, meta) in index.meta.iter().filter(|(k, _)| k.starts_with(&prefix)) {
        *counts.entry(meta.doc_type.clone()).or_default() += 1;
        if meta.type_source == TypeSource::Declared {
            declared += 1;
        }
        typed_links += index
            .outgoing
            .get(id)
            .map(|v| v.iter().filter(|l| l.relation.is_some()).count())
            .unwrap_or(0);
    }
    Ok(serde_json::json!({
        "project": project,
        "types": vocab.types,
        "relations": vocab.relations,
        "statuses": vocab.statuses,
        "folders": vocab.folders,
        "inherited_from": vocab.inherited_from,
        "counts_by_type": counts,
        "declared_types": declared,
        "typed_links": typed_links,
        "how_to": [
            "문서 종류·상태: frontmatter `type: decision` · `status: active|superseded|done` — danbi_set_meta 로 본문을 건드리지 않고 넣을 수 있어요. 안 적으면 폴더(decisions/·notes/·daily/)로 판단해요.",
            "관계: 본문 한 줄에 `원인:: [[notes/x.md]]` · `영향:: [[release.md]]` (한글/영문 이름 둘 다 됨). `## 관련` 아래 링크는 relates_to.",
            "기존 문서의 관계 정리: danbi_propose_link 로 제안 → 사용자가 단비 앱에서 승인하면 그때 markdown 에 기록돼요.",
            "단어 추가: schema.md 의 `## 온톨로지` 아래 ```yaml 블록 (types / relations / folders / inherit)."
        ],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only numbers against the user's vault — run by hand:
    /// `cargo test --lib real_vault_ontology -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_vault_ontology() {
        let vault = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Danbi_Vault");
        if !vault.exists() {
            return;
        }
        let t = std::time::Instant::now();
        let idx = crate::links::build_index(&vault).unwrap();
        println!("build_index: {:?}, docs {}", t.elapsed(), idx.meta.len());
        let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
        let mut declared = 0;
        for m in idx.meta.values() {
            *by_type.entry(m.doc_type.clone()).or_default() += 1;
            if m.type_source == TypeSource::Declared {
                declared += 1;
            }
        }
        println!("by type: {by_type:?}, declared {declared}");
        let mut by_rel: BTreeMap<String, usize> = BTreeMap::new();
        let mut total = 0;
        for links in idx.outgoing.values() {
            for l in links {
                total += 1;
                if let Some(r) = &l.relation {
                    *by_rel.entry(r.clone()).or_default() += 1;
                }
            }
        }
        println!("links {total}, typed {by_rel:?}");
        let sup = idx.meta.values().filter(|m| m.effective_status.as_deref() == Some("superseded")).count();
        println!("superseded {sup}, docs with vocab issues {}", idx.issues.len());
        // Cost of the ontology passes alone (files pre-read).
        let mut docs: Vec<(String, String)> = Vec::new();
        for id in idx.meta.keys() {
            if let Ok(md) = std::fs::read_to_string(vault.join(PROJECTS_DIRNAME).join(id)) {
                docs.push((id.clone(), md));
            }
        }
        let v = Vocabulary::default();
        let t = std::time::Instant::now();
        for (id, md) in &docs {
            let _ = line_relations(&v, md);
            let _ = doc_meta(&v, id, md);
            let _ = doc_issues(&v, md);
        }
        println!("ontology passes only: {:?} over {} docs", t.elapsed(), docs.len());
        let t = std::time::Instant::now();
        let (rows, n) = query(&vault, &QueryArgs { doc_type: Some("decision".into()), limit: 5, ..Default::default() }).unwrap();
        println!("query(decision): {:?}, {n} hits, first {:?}", t.elapsed(), rows.first().map(|r| &r.domain));
    }

    #[test]
    fn folder_and_declared_types() {
        let v = Vocabulary::default();
        assert_eq!(doc_meta(&v, "decisions/x.md", "본문").doc_type, "decision");
        assert_eq!(doc_meta(&v, "daily/2026-10-30.md", "").doc_type, "daily");
        assert_eq!(doc_meta(&v, "purpose.md", "").doc_type, "meta");
        assert_eq!(doc_meta(&v, "ui.md", "").doc_type, "note");
        let m = doc_meta(&v, "notes/a.md", "---\ntype: 결정\nstatus: 완료\n---\n본문");
        assert_eq!((m.doc_type.as_str(), m.type_source, m.status.as_deref()), ("decision", TypeSource::Declared, Some("done")));
        let m = doc_meta(&v, "notes/a.md", "---\ntype: project\n---\n");
        assert_eq!((m.doc_type.as_str(), m.type_known), ("note", true));
        let m = doc_meta(&v, "notes/a.md", "---\ntype: experiment\n---\n");
        assert_eq!((m.doc_type.as_str(), m.type_known), ("experiment", false));
    }

    #[test]
    fn line_relations_cover_inline_frontmatter_and_sections() {
        let v = Vocabulary::default();
        let md = "---\ntype: bug\naffects:\n  - \"[[a.md]]\"\n---\n# 제목\n원인:: [[b.md]]\n- 영향:: [[c.md]]\n평문 [[d.md]]\n## 관련\n- [[e.md]]\n```\n원인:: [[f.md]]\n```\n## 다음\n[[g.md]]\n관련: [[h.md]]\n";
        let lr = line_relations(&v, md);
        let rel = |needle: &str| lr.at(md.find(needle).unwrap()).map(str::to_string);
        assert_eq!(rel("[[a.md]]").as_deref(), Some("affects"));
        assert_eq!(rel("[[b.md]]").as_deref(), Some("caused_by"));
        assert_eq!(rel("[[c.md]]").as_deref(), Some("affects"));
        assert_eq!(rel("[[d.md]]"), None);
        assert_eq!(rel("[[e.md]]").as_deref(), Some("relates_to"));
        assert_eq!(rel("[[f.md]]"), None);
        assert_eq!(rel("[[g.md]]"), None);
        assert_eq!(rel("[[h.md]]").as_deref(), Some("relates_to"));
    }

    #[test]
    fn schema_fence_extends_vocabulary() {
        let md = "# s\n## 온톨로지\n```yaml\ntypes: [experiment]\nrelations:\n  tested_by: {ko: 검증, inverse: tests, inverse_ko: 검증함}\nfolders:\n  exp: experiment\n```\n";
        let mut v = Vocabulary::default();
        v.extend(&parse_fence(&ontology_fence(md).unwrap()));
        assert_eq!(v.canonical_type("experiment").as_deref(), Some("experiment"));
        assert_eq!(v.canonical_relation("검증").as_deref(), Some("tested_by"));
        assert_eq!(v.inverse("tests"), "tested_by");
        assert_eq!(infer_type(&v, "exp/a.md"), "experiment");
        assert_eq!(v.canonical_relation("영향").as_deref(), Some("affects"));
    }

    #[test]
    fn set_frontmatter_preserves_everything_else() {
        let md = "---\nname: x\nmetadata:\n  a: 1\ntype: project # 주석\n---\n\n본문 [[a]]\n";
        let out = set_frontmatter(
            md,
            &MetaUpdate {
                set: vec![("status".into(), "active".into())],
                add_items: vec![("affects".into(), "[[b.md]]".into())],
            },
        )
        .unwrap();
        assert_eq!(
            out,
            "---\nname: x\nmetadata:\n  a: 1\ntype: project # 주석\nstatus: active\naffects:\n  - \"[[b.md]]\"\n---\n\n본문 [[a]]\n"
        );
        // Re-adding the same item is a no-op; replacing a scalar keeps order.
        let again = set_frontmatter(
            &out,
            &MetaUpdate {
                set: vec![("type".into(), "decision".into())],
                add_items: vec![("affects".into(), "[[b.md]]".into()), ("affects".into(), "[[c.md]]".into())],
            },
        )
        .unwrap();
        // The replaced line keeps its comment.
        assert!(again.contains("type: decision # 주석\nstatus: active\naffects:\n  - \"[[b.md]]\"\n  - \"[[c.md]]\"\n---"));
        assert!(again.contains("metadata:\n  a: 1\n"));
    }

    #[test]
    fn set_frontmatter_creates_block_and_handles_inline_lists() {
        assert_eq!(
            set_frontmatter("# 제목\n", &MetaUpdate { set: vec![("type".into(), "bug".into())], add_items: vec![] }).unwrap(),
            "---\ntype: bug\n---\n\n# 제목\n"
        );
        let out = set_frontmatter(
            "---\naffects: [\"[[a.md]]\"] # c\n---\n",
            &MetaUpdate { set: vec![], add_items: vec![("affects".into(), "[[b.md]]".into())] },
        )
        .unwrap();
        assert_eq!(out, "---\naffects: [\"[[a.md]]\", \"[[b.md]]\"] # c\n---\n");
    }

    fn set(md: &str, key: &str, v: &str) -> Result<String, String> {
        set_frontmatter(md, &MetaUpdate { set: vec![(key.into(), v.into())], add_items: vec![] })
    }

    #[test]
    fn set_frontmatter_review_edge_cases() {
        // A1: a body that opens with a `---` rule is not frontmatter.
        assert_eq!(
            set("---\n제목 줄\ntype: 설명 텍스트\n---\n더 본문\n", "type", "bug").unwrap(),
            "---\ntype: bug\n---\n\n---\n제목 줄\ntype: 설명 텍스트\n---\n더 본문\n"
        );
        // B1: BOM stays first and the existing block is edited in place.
        assert_eq!(set("\u{feff}---\ntype: note\n---\n본문\n", "type", "bug").unwrap(), "\u{feff}---\ntype: bug\n---\n본문\n");
        assert_eq!(set("\u{feff}# 제목\n", "type", "bug").unwrap(), "\u{feff}---\ntype: bug\n---\n\n# 제목\n");
        // B2: mixed newlines survive untouched outside the edited line.
        assert_eq!(
            set("---\ntype: note\n---\n본문 A\r\n본문 B\n", "type", "bug").unwrap(),
            "---\ntype: bug\n---\n본문 A\r\n본문 B\n"
        );
        assert_eq!(set("---\r\ntype: note\r\n---\r\n본문", "status", "done").unwrap(), "---\r\ntype: note\r\nstatus: done\r\n---\r\n본문");
        // B3: a flow map is refused, not nested.
        assert!(set_frontmatter("---\naffects: {a: 1}\n---\n", &MetaUpdate { set: vec![], add_items: vec![("affects".into(), "[[b.md]]".into())] }).is_err());
        // C1 same value keeps the line (and comment); C2 duplicate keys refused.
        assert_eq!(set("---\ntype: note # 주석\n---\n", "type", "note").unwrap(), "---\ntype: note # 주석\n---\n");
        assert!(set("---\ntype: a\ntype: b\n---\n", "type", "bug").is_err());
        // C3 quoted key, C4 original key spelling.
        assert_eq!(set("---\n\"type\": note\n---\n", "type", "bug").unwrap(), "---\n\"type\": bug\n---\n");
        assert_eq!(set("---\nType: note\n---\n", "type", "bug").unwrap(), "---\nType: bug\n---\n");
        // C5: an aliased link counts as already there.
        let md = "---\naffects:\n  - \"[[b.md|별칭]]\"\n---\n";
        assert_eq!(
            set_frontmatter(md, &MetaUpdate { set: vec![], add_items: vec![("affects".into(), "[[b.md]]".into())] }).unwrap(),
            md
        );
    }

    #[test]
    fn related_item_lands_in_its_section() {
        assert_eq!(
            append_related_item("# T\n## 관련\n- [[a]]\n\n## 다음\n- 할일\n", "영향:: [[P/x.md]]"),
            "# T\n## 관련\n- [[a]]\n- 영향:: [[P/x.md]]\n\n## 다음\n- 할일\n"
        );
        assert_eq!(append_related_item("본문\r\n", "[[x]]"), "본문\r\n\r\n## 관련\r\n\r\n- [[x]]\r\n");
        assert_eq!(append_related_item("", "[[x]]"), "## 관련\n\n- [[x]]\n");
        assert_eq!(
            append_related_item("```\n## 관련\n```\n## 관련\n- [[a]]", "[[x]]"),
            "```\n## 관련\n```\n## 관련\n- [[a]]\n- [[x]]\n"
        );
    }

    #[test]
    fn issues_flag_unknown_values_only() {
        let v = Vocabulary::default();
        let i = doc_issues(&v, "---\ntype: experiment\nstatus: wip\n---\n참고자료:: [[a]]\n원인:: [[b]]\n작성자:: 나\n");
        assert_eq!(i.unknown_type.as_deref(), Some("experiment"));
        assert_eq!(i.unknown_status.as_deref(), Some("wip"));
        assert_eq!(i.unknown_relations, vec!["참고자료".to_string()]);
    }
}
