//! Knowledge graph — the core of Danbi's visualization + insight layer.
//!
//! What this module produces:
//!   1. **Nodes** — every `.md` in the vault (plus project / domain metadata).
//!   2. **Edges** — four kinds with distinct visual + semantic meaning:
//!        - `confirmed` — an actual `[[wiki link]]` appears in the source
//!          markdown. These are ground truth.
//!        - `ghost`     — a pending suggestion from `ghost_links`
//!          (AI proposed, user hasn't accepted yet).
//!        - `soft`      — an *implicit* relationship Danbi discovered via
//!          the relevance model (shared vocabulary, co-citation, etc.)
//!          even when no `[[ ]]` exists. Rendered dashed in a distinct
//!          color so it reads as "hint, not fact". This is the J-3 output.
//!        - `ghost` edges also carry their relevance score so the UI can
//!          thicken the "more confident" suggestions.
//!   3. **Communities** — Louvain modularity-optimizing clusters. J-4.
//!   4. **Insights** — isolated pages, bridges, hubs, surprising connections.
//!      J-5.
//!
//! Design philosophy: the graph is the single visual representation of the
//! project's accumulated Wiki-LLM knowledge. The more it fills in, the
//! richer every subsequent grounding-augmented edit becomes.

use crate::error::DanbiResult;
use crate::ghost_links::{self, GhostStatus};
use crate::links::build_index;
use crate::vault::{list_tree, PROJECTS_DIRNAME};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

// ---- Public types ----

#[derive(Debug, Serialize, Clone)]
pub struct GraphNode {
    pub id: String,
    pub project: String,
    pub domain: String,
    pub label: String,
    pub bytes: u64,
    /// Louvain community id. Nodes in the same community cluster together.
    /// Populated by `build_graph`; `-1` means "no edges, isolated".
    #[serde(default)]
    pub community: i32,
    /// Degree = number of edges touching this node. Cheap, useful for
    /// rendering (hubs larger) and insight ranking.
    #[serde(default)]
    pub degree: u32,
}

#[derive(Debug, Serialize, Clone)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    /// "confirmed" | "ghost" | "soft"
    pub kind: &'static str,
    /// Composite relevance score. Higher = stronger relationship.
    /// Confirmed edges are always ≥ the `direct` weight (3.0).
    pub score: f32,
    /// Only populated for ghost edges.
    pub ghost_id: Option<String>,
    pub ghost_project: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct GraphInsights {
    /// Nodes with zero edges — candidates for either deletion or linking.
    pub isolated: Vec<String>,
    /// Communities that contain only 1–2 nodes — likely under-populated
    /// areas that would benefit from more notes.
    pub sparse_communities: Vec<SparseCommunity>,
    /// Nodes whose removal would split the graph (articulation points).
    /// These are load-bearing connectors — treat with care.
    pub bridges: Vec<String>,
    /// Top-N nodes by degree. Likely the project's core concepts.
    pub hubs: Vec<HubNode>,
    /// Edges that connect otherwise-separate communities. The single
    /// thread that ties two topic clusters together.
    pub surprising: Vec<SurprisingEdge>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SparseCommunity {
    pub id: i32,
    pub members: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct HubNode {
    pub id: String,
    pub degree: u32,
}

#[derive(Debug, Serialize, Clone)]
pub struct SurprisingEdge {
    pub source: String,
    pub target: String,
    pub source_community: i32,
    pub target_community: i32,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub insights: GraphInsights,
}

// ---- Entry point ----

pub fn build_graph(vault: &Path, project_filter: Option<&str>) -> DanbiResult<GraphData> {
    let tree = list_tree(vault)?;

    // 1) Nodes
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut node_ids: HashSet<String> = HashSet::new();
    for p in &tree.projects {
        if let Some(f) = project_filter {
            if p.name != f {
                continue;
            }
        }
        for d in &p.domains {
            let id = format!("{}/{}", p.name, d.name);
            if node_ids.insert(id.clone()) {
                nodes.push(GraphNode {
                    id,
                    project: p.name.clone(),
                    domain: d.name.clone(),
                    label: d.name.clone(),
                    bytes: d.bytes,
                    community: -1,
                    degree: 0,
                });
            }
        }
        for sub in &p.subfolders {
            for d in &sub.domains {
                let id = format!("{}/{}", p.name, d.name);
                if node_ids.insert(id.clone()) {
                    let label = d
                        .name
                        .rsplit_once('/')
                        .map(|(_, f)| f.to_string())
                        .unwrap_or_else(|| d.name.clone());
                    nodes.push(GraphNode {
                        id,
                        project: p.name.clone(),
                        domain: d.name.clone(),
                        label,
                        bytes: d.bytes,
                        community: -1,
                        degree: 0,
                    });
                }
            }
        }
    }

    // 2) Confirmed edges (wiki links) — with direct weight.
    let mut edges: Vec<GraphEdge> = Vec::new();
    // Track confirmed pairs so ghost/soft layers don't duplicate them.
    let mut confirmed_pairs: HashSet<(String, String)> = HashSet::new();
    let index = build_index(vault)?;
    for (src_key, targets) in &index.outgoing {
        if !node_ids.contains(src_key) {
            continue;
        }
        for t in targets {
            let tgt_key = format!("{}/{}", t.project, t.domain);
            if !node_ids.contains(&tgt_key) {
                continue;
            }
            confirmed_pairs.insert(canonical_pair(src_key, &tgt_key));
            edges.push(GraphEdge {
                source: src_key.clone(),
                target: tgt_key,
                kind: "confirmed",
                score: WEIGHT_DIRECT,
                ghost_id: None,
                ghost_project: None,
                reason: None,
            });
        }
    }

    // 3) Ghost edges (pending suggestions).
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if projects_root.exists() {
        for entry in std::fs::read_dir(&projects_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(project) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if project.starts_with('.') {
                continue;
            }
            if let Some(f) = project_filter {
                if project != f {
                    continue;
                }
            }
            let store = match ghost_links::load(vault, project) {
                Ok(s) => s,
                Err(_) => continue,
            };
            for link in store.links {
                if !matches!(link.status, GhostStatus::Pending) {
                    continue;
                }
                let src_id = normalize_id(project, &link.source_domain);
                let tgt_id = normalize_id(project, &link.target_domain);
                if !node_ids.contains(&src_id) || !node_ids.contains(&tgt_id) {
                    continue;
                }
                edges.push(GraphEdge {
                    source: src_id,
                    target: tgt_id,
                    kind: "ghost",
                    score: WEIGHT_DIRECT * 0.8, // below confirmed but strong
                    ghost_id: Some(link.id),
                    ghost_project: Some(project.to_string()),
                    reason: Some(link.reason),
                });
            }
        }
    }

    // 4) Soft edges (relevance-inferred). Enriches confirmed/ghost pairs
    //    with relevance signal. Adds *new* edges only when score crosses
    //    SOFT_THRESHOLD and the pair isn't already connected.
    augment_with_relevance(vault, &nodes, &mut edges, &confirmed_pairs)?;

    // 5) Communities (Louvain) — on the full (undirected) edge set.
    let communities = louvain(&nodes, &edges);
    apply_communities(&mut nodes, &communities);

    // 6) Degrees — simple count per node.
    apply_degrees(&mut nodes, &edges);

    // 7) Insights
    let insights = compute_insights(&nodes, &edges);

    Ok(GraphData {
        nodes,
        edges,
        insights,
    })
}

// ---- Relevance model (J-3) ----

const WEIGHT_DIRECT: f32 = 3.0;
const WEIGHT_OVERLAP: f32 = 4.0;
const WEIGHT_ADAMIC_ADAR: f32 = 1.5;
const WEIGHT_PROJECT_AFFINITY: f32 = 1.0;
const SOFT_THRESHOLD: f32 = 1.2;
const MAX_SOFT_EDGES_PER_NODE: usize = 6;

/// Adds `soft` edges to the graph for node pairs that score above
/// SOFT_THRESHOLD on the combined relevance signal and don't already
/// have a confirmed or ghost edge.
fn augment_with_relevance(
    vault: &Path,
    nodes: &[GraphNode],
    edges: &mut Vec<GraphEdge>,
    confirmed_pairs: &HashSet<(String, String)>,
) -> DanbiResult<()> {
    if nodes.len() < 2 {
        return Ok(());
    }

    // Load each node's body once; build a lightweight term frequency
    // table keyed by lowercased word. We skip anything shorter than 3
    // chars, numeric-only tokens, and stop-like artifacts.
    let mut term_sets: HashMap<String, HashSet<String>> = HashMap::new();
    for n in nodes {
        let path = vault
            .join(PROJECTS_DIRNAME)
            .join(&n.project)
            .join(&n.domain);
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        term_sets.insert(n.id.clone(), tokenize(&body));
    }

    // IDF approximation: how many docs contain this term. Rare terms are
    // more informative when two docs share them.
    let mut df: HashMap<String, usize> = HashMap::new();
    for set in term_sets.values() {
        for t in set {
            *df.entry(t.clone()).or_default() += 1;
        }
    }
    let total = nodes.len() as f32;

    // Build a quick neighbor lookup so Adamic-Adar can compute shared
    // neighbors cheaply.
    let mut neighbors: HashMap<String, HashSet<String>> = HashMap::new();
    for e in edges.iter() {
        neighbors
            .entry(e.source.clone())
            .or_default()
            .insert(e.target.clone());
        neighbors
            .entry(e.target.clone())
            .or_default()
            .insert(e.source.clone());
    }

    // Gather candidate soft edges. Only consider same-project pairs in
    // the first cut — cross-project soft edges generate too much noise
    // without a deeper signal.
    let mut per_node_count: HashMap<String, usize> = HashMap::new();
    let mut soft: Vec<GraphEdge> = Vec::new();

    for (i, a) in nodes.iter().enumerate() {
        for b in nodes.iter().skip(i + 1) {
            if a.project != b.project {
                continue;
            }
            let key = canonical_pair(&a.id, &b.id);
            if confirmed_pairs.contains(&key) {
                continue;
            }
            // Skip if we already have a ghost edge for this pair — ghost
            // already carries intent, no need to add a parallel soft line.
            if edges.iter().any(|e| {
                e.kind == "ghost" && canonical_pair(&e.source, &e.target) == key
            }) {
                continue;
            }

            let empty: HashSet<String> = HashSet::new();
            let ta = term_sets.get(&a.id).unwrap_or(&empty);
            let tb = term_sets.get(&b.id).unwrap_or(&empty);
            let overlap = term_overlap_idf(ta, tb, &df, total);

            let na = neighbors.get(&a.id).unwrap_or(&empty);
            let nb = neighbors.get(&b.id).unwrap_or(&empty);
            let aa = adamic_adar(na, nb, &neighbors);

            let score = WEIGHT_OVERLAP * overlap
                + WEIGHT_ADAMIC_ADAR * aa
                + WEIGHT_PROJECT_AFFINITY;
            if score < SOFT_THRESHOLD {
                continue;
            }
            soft.push(GraphEdge {
                source: a.id.clone(),
                target: b.id.clone(),
                kind: "soft",
                score,
                ghost_id: None,
                ghost_project: None,
                reason: None,
            });
        }
    }

    // Keep only the strongest MAX_SOFT_EDGES_PER_NODE per node to avoid
    // a spaghetti graph on projects with homogeneous vocabulary.
    soft.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    for e in soft {
        let a_cnt = per_node_count.get(&e.source).copied().unwrap_or(0);
        let b_cnt = per_node_count.get(&e.target).copied().unwrap_or(0);
        if a_cnt >= MAX_SOFT_EDGES_PER_NODE || b_cnt >= MAX_SOFT_EDGES_PER_NODE {
            continue;
        }
        per_node_count.insert(e.source.clone(), a_cnt + 1);
        per_node_count.insert(e.target.clone(), b_cnt + 1);
        edges.push(e);
    }

    Ok(())
}

/// Extracts a coarse bag-of-terms from a markdown document. Good enough
/// for our IDF-style overlap — we're not building a search engine here.
fn tokenize(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let lower = text.to_lowercase();
    for raw in lower.split(|c: char| !c.is_alphanumeric()) {
        let token = raw.trim();
        if token.len() < 3 || token.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if STOP_EN.contains(&token) {
            continue;
        }
        out.insert(token.to_string());
    }
    out
}

const STOP_EN: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "have", "has",
    "but", "not", "are", "you", "your", "our", "its", "was", "were",
    "been", "will", "they", "them", "their", "which", "who", "whom",
    "can", "could", "should", "would", "about", "into", "onto",
    "these", "those", "there", "here", "when", "then", "than",
];

/// Sum of IDF values of terms both docs contain, normalized by the
/// smaller document's term count. Bounded to [0, 1] roughly.
fn term_overlap_idf(
    a: &HashSet<String>,
    b: &HashSet<String>,
    df: &HashMap<String, usize>,
    total: f32,
) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for t in a.intersection(b) {
        let f = *df.get(t).unwrap_or(&1) as f32;
        // Classic IDF: log(N / df). +1 in denominator avoids div-by-zero.
        let idf = (total / (1.0 + f)).ln().max(0.0);
        sum += idf;
    }
    let denom = (a.len().min(b.len()) as f32).max(1.0);
    (sum / denom).clamp(0.0, 2.0) / 2.0
}

/// Adamic-Adar: sum of 1/log(|N(w)|) for every shared neighbor w. Rare
/// connectors contribute more than popular ones.
fn adamic_adar(
    a_neigh: &HashSet<String>,
    b_neigh: &HashSet<String>,
    all: &HashMap<String, HashSet<String>>,
) -> f32 {
    let mut sum = 0.0f32;
    for w in a_neigh.intersection(b_neigh) {
        let deg = all.get(w).map(|s| s.len()).unwrap_or(1) as f32;
        if deg <= 1.0 {
            continue;
        }
        sum += 1.0 / deg.ln();
    }
    sum.clamp(0.0, 3.0) / 3.0
}

// ---- Louvain community detection (J-4) ----
//
// Minimal-dependency implementation of the Louvain modularity method.
// We run one phase of local moves (no multi-level coarsening) — good
// enough for a few thousand nodes which is well above typical vault
// sizes. Uses edge.score as the weight.

fn louvain(nodes: &[GraphNode], edges: &[GraphEdge]) -> HashMap<String, i32> {
    if nodes.is_empty() {
        return HashMap::new();
    }
    // Index nodes.
    let mut idx: HashMap<String, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        idx.insert(n.id.clone(), i);
    }
    let n = nodes.len();
    // Adjacency: adj[i] = Vec<(j, weight)>.
    let mut adj: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    let mut total_weight = 0.0f32;
    for e in edges {
        let (Some(&i), Some(&j)) = (idx.get(&e.source), idx.get(&e.target)) else {
            continue;
        };
        if i == j {
            continue;
        }
        let w = e.score.max(0.01);
        adj[i].push((j, w));
        adj[j].push((i, w));
        total_weight += w;
    }
    // Node strengths.
    let k: Vec<f32> = adj
        .iter()
        .map(|nbrs| nbrs.iter().map(|(_, w)| w).sum())
        .collect();
    // 2m = total_weight * 2 (each edge counted once above; Louvain uses 2m).
    let two_m = (total_weight * 2.0).max(1e-9);

    // Initial: each node in its own community.
    let mut comm: Vec<i32> = (0..n as i32).collect();

    // Greedy local moves: for each node, try to move it into a
    // neighbor's community that maximizes modularity gain. Repeat until
    // stable.
    let mut changed = true;
    let mut passes = 0;
    while changed && passes < 20 {
        changed = false;
        passes += 1;
        for i in 0..n {
            let ki = k[i];
            // Weight of edges from i into each neighbor community.
            let mut c_weight: HashMap<i32, f32> = HashMap::new();
            for &(j, w) in &adj[i] {
                *c_weight.entry(comm[j]).or_default() += w;
            }
            // Sum of k for nodes currently in community c (excluding i).
            // Compute lazily only for candidate comms.
            let current = comm[i];
            let mut best_comm = current;
            let mut best_gain = 0.0f32;
            for (&c, &w_in) in c_weight.iter() {
                if c == current {
                    continue;
                }
                let sum_tot: f32 = (0..n).filter(|&x| comm[x] == c).map(|x| k[x]).sum();
                // Modularity gain approximation.
                let gain = w_in - ki * sum_tot / two_m;
                if gain > best_gain {
                    best_gain = gain;
                    best_comm = c;
                }
            }
            if best_comm != current {
                comm[i] = best_comm;
                changed = true;
            }
        }
    }

    // Renumber communities to contiguous [0..k].
    let mut remap: HashMap<i32, i32> = HashMap::new();
    let mut next_id = 0i32;
    let mut out: HashMap<String, i32> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        let raw = comm[i];
        let id = *remap.entry(raw).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        out.insert(n.id.clone(), id);
    }
    out
}

fn apply_communities(nodes: &mut [GraphNode], map: &HashMap<String, i32>) {
    for n in nodes.iter_mut() {
        n.community = *map.get(&n.id).unwrap_or(&-1);
    }
}

fn apply_degrees(nodes: &mut [GraphNode], edges: &[GraphEdge]) {
    let mut deg: HashMap<String, u32> = HashMap::new();
    for e in edges {
        *deg.entry(e.source.clone()).or_default() += 1;
        *deg.entry(e.target.clone()).or_default() += 1;
    }
    for n in nodes.iter_mut() {
        n.degree = *deg.get(&n.id).unwrap_or(&0);
    }
}

// ---- Insights (J-5) ----

fn compute_insights(nodes: &[GraphNode], edges: &[GraphEdge]) -> GraphInsights {
    let mut out = GraphInsights::default();

    // Isolated: degree 0.
    out.isolated = nodes
        .iter()
        .filter(|n| n.degree == 0)
        .map(|n| n.id.clone())
        .collect();

    // Sparse communities: members count ≤ 2.
    let mut community_members: HashMap<i32, Vec<String>> = HashMap::new();
    for n in nodes {
        if n.community < 0 {
            continue;
        }
        community_members
            .entry(n.community)
            .or_default()
            .push(n.id.clone());
    }
    for (id, members) in community_members.iter() {
        if members.len() <= 2 {
            out.sparse_communities.push(SparseCommunity {
                id: *id,
                members: members.clone(),
            });
        }
    }
    out.sparse_communities.sort_by_key(|c| c.id);

    // Hubs: top 5 by degree.
    let mut sorted = nodes.to_vec();
    sorted.sort_by(|a, b| b.degree.cmp(&a.degree));
    out.hubs = sorted
        .iter()
        .take(5)
        .filter(|n| n.degree > 0)
        .map(|n| HubNode {
            id: n.id.clone(),
            degree: n.degree,
        })
        .collect();

    // Surprising edges: cross-community edges where the two communities
    // have only one connecting edge.
    let community_of: HashMap<String, i32> = nodes
        .iter()
        .map(|n| (n.id.clone(), n.community))
        .collect();
    let mut cross_edge_count: HashMap<(i32, i32), Vec<(String, String)>> =
        HashMap::new();
    for e in edges {
        let ca = community_of.get(&e.source).copied().unwrap_or(-1);
        let cb = community_of.get(&e.target).copied().unwrap_or(-1);
        if ca < 0 || cb < 0 || ca == cb {
            continue;
        }
        let key = if ca < cb { (ca, cb) } else { (cb, ca) };
        cross_edge_count
            .entry(key)
            .or_default()
            .push((e.source.clone(), e.target.clone()));
    }
    for ((ca, cb), list) in cross_edge_count.iter() {
        if list.len() == 1 {
            let (s, t) = &list[0];
            out.surprising.push(SurprisingEdge {
                source: s.clone(),
                target: t.clone(),
                source_community: *ca,
                target_community: *cb,
            });
        }
    }
    out.surprising.sort_by(|a, b| {
        (a.source_community, a.target_community)
            .cmp(&(b.source_community, b.target_community))
    });

    // Bridges: articulation points via a simple DFS Tarjan-style.
    out.bridges = articulation_points(nodes, edges);

    out
}

/// Tarjan articulation-point algorithm on the undirected edge set.
fn articulation_points(nodes: &[GraphNode], edges: &[GraphEdge]) -> Vec<String> {
    let mut idx: HashMap<String, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        idx.insert(n.id.clone(), i);
    }
    let n = nodes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        let (Some(&i), Some(&j)) = (idx.get(&e.source), idx.get(&e.target)) else {
            continue;
        };
        if i == j {
            continue;
        }
        adj[i].push(j);
        adj[j].push(i);
    }

    let mut visited = vec![false; n];
    let mut disc = vec![0i32; n];
    let mut low = vec![0i32; n];
    let mut parent = vec![-1i32; n];
    let mut timer = 0i32;
    let mut articulation = vec![false; n];

    fn dfs(
        u: usize,
        adj: &Vec<Vec<usize>>,
        visited: &mut [bool],
        disc: &mut [i32],
        low: &mut [i32],
        parent: &mut [i32],
        timer: &mut i32,
        articulation: &mut [bool],
    ) {
        visited[u] = true;
        *timer += 1;
        disc[u] = *timer;
        low[u] = *timer;
        let mut children = 0;
        for &v in &adj[u] {
            if !visited[v] {
                children += 1;
                parent[v] = u as i32;
                dfs(v, adj, visited, disc, low, parent, timer, articulation);
                low[u] = low[u].min(low[v]);
                if parent[u] == -1 && children > 1 {
                    articulation[u] = true;
                }
                if parent[u] != -1 && low[v] >= disc[u] {
                    articulation[u] = true;
                }
            } else if v as i32 != parent[u] {
                low[u] = low[u].min(disc[v]);
            }
        }
    }

    for u in 0..n {
        if !visited[u] {
            dfs(
                u,
                &adj,
                &mut visited,
                &mut disc,
                &mut low,
                &mut parent,
                &mut timer,
                &mut articulation,
            );
        }
    }
    nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| articulation[*i])
        .map(|(_, n)| n.id.clone())
        .collect()
}

// ---- Helpers ----

fn canonical_pair(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn normalize_id(project: &str, raw: &str) -> String {
    if raw.contains('/') {
        raw.to_string()
    } else {
        format!("{project}/{raw}")
    }
}
