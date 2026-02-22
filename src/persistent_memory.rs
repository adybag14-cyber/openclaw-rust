use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const DEFAULT_ZVEC_STORE_PATH: &str = ".openclaw-rs/memory/zvec-store.json";
pub const DEFAULT_GRAPHLITE_STORE_PATH: &str = ".openclaw-rs/memory/graphlite-store.json";

const VECTOR_DIM: usize = 192;
const DEFAULT_MAX_ENTRIES: usize = 20_000;
const DEFAULT_RECALL_TOP_K: usize = 8;
const DEFAULT_RECALL_MIN_SCORE: f64 = 0.18;
const MAX_MEMORY_TEXT_CHARS: usize = 6_000;
const MAX_MEMORY_PROMPT_CHARS: usize = 3_200;
const MAX_GRAPH_KEYWORDS: usize = 12;
const MAX_GRAPH_NODES: usize = 50_000;
const MAX_GRAPH_EDGES: usize = 100_000;
const MAX_GRAPH_FACTS: usize = 6;
const MAX_VECTOR_HITS_FOR_PROMPT: usize = 8;

static MEMORY_ENTRY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Default)]
pub struct MemoryRuntimeConfig {
    pub enabled: Option<bool>,
    pub zvec_store_path: Option<String>,
    pub graph_store_path: Option<String>,
    pub max_entries: Option<usize>,
    pub recall_top_k: Option<usize>,
    pub recall_min_score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct MemoryRememberInput {
    pub session_key: String,
    pub source: String,
    pub text: String,
    pub request_id: Option<String>,
    pub at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct MemoryRecallQuery {
    pub session_key: String,
    pub query_text: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryRecallOutcome {
    pub system_prompt: Option<String>,
    pub vector_hits: usize,
    pub graph_facts: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub enabled: bool,
    #[serde(rename = "zvecEntries")]
    pub zvec_entries: usize,
    #[serde(rename = "graphNodes")]
    pub graph_nodes: usize,
    #[serde(rename = "graphEdges")]
    pub graph_edges: usize,
    #[serde(rename = "zvecStorePath")]
    pub zvec_store_path: String,
    #[serde(rename = "graphStorePath")]
    pub graph_store_path: String,
    #[serde(rename = "maxEntries")]
    pub max_entries: usize,
    #[serde(rename = "recallTopK")]
    pub recall_top_k: usize,
}

pub struct PersistentMemoryRegistry {
    runtime: Mutex<MemoryRuntimeState>,
    state: Mutex<MemoryState>,
}

#[derive(Debug, Clone)]
struct MemoryRuntimeState {
    enabled: bool,
    zvec_store_path: String,
    graph_store_path: String,
    max_entries: usize,
    recall_top_k: usize,
    recall_min_score: f64,
}

impl Default for MemoryRuntimeState {
    fn default() -> Self {
        Self {
            enabled: true,
            zvec_store_path: DEFAULT_ZVEC_STORE_PATH.to_owned(),
            graph_store_path: DEFAULT_GRAPHLITE_STORE_PATH.to_owned(),
            max_entries: DEFAULT_MAX_ENTRIES,
            recall_top_k: DEFAULT_RECALL_TOP_K,
            recall_min_score: DEFAULT_RECALL_MIN_SCORE,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct MemoryState {
    zvec_entries: Vec<ZvecMemoryEntry>,
    graph_nodes: HashMap<String, GraphNode>,
    graph_edges: HashMap<String, GraphEdge>,
}

impl MemoryState {
    fn enforce_limits(&mut self, max_entries: usize) {
        if self.zvec_entries.len() > max_entries {
            let overflow = self.zvec_entries.len() - max_entries;
            self.zvec_entries.drain(0..overflow);
        }
        if self.graph_edges.len() > MAX_GRAPH_EDGES {
            let mut all_edges = self.graph_edges.values().cloned().collect::<Vec<_>>();
            all_edges.sort_by(|a, b| {
                b.updated_at_ms
                    .cmp(&a.updated_at_ms)
                    .then_with(|| b.weight.cmp(&a.weight))
            });
            all_edges.truncate(MAX_GRAPH_EDGES);
            self.graph_edges = all_edges
                .into_iter()
                .map(|edge| (graph_edge_key(&edge.from, &edge.relation, &edge.to), edge))
                .collect();
        }
        if self.graph_nodes.len() > MAX_GRAPH_NODES {
            let mut all_nodes = self.graph_nodes.values().cloned().collect::<Vec<_>>();
            all_nodes.sort_by(|a, b| {
                b.updated_at_ms
                    .cmp(&a.updated_at_ms)
                    .then_with(|| b.mentions.cmp(&a.mentions))
            });
            all_nodes.truncate(MAX_GRAPH_NODES);
            self.graph_nodes = all_nodes
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect();
        }
    }

    fn graph_snapshot(&self) -> GraphLiteDiskState {
        let mut nodes = self.graph_nodes.values().cloned().collect::<Vec<_>>();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut edges = self.graph_edges.values().cloned().collect::<Vec<_>>();
        edges.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.from.cmp(&b.from))
                .then_with(|| a.to.cmp(&b.to))
        });
        GraphLiteDiskState {
            version: 1,
            nodes,
            edges,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZvecMemoryEntry {
    id: String,
    session_key: String,
    source: String,
    text: String,
    request_id: Option<String>,
    at_ms: u64,
    vector: Vec<f32>,
    tokens: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphNode {
    id: String,
    label: String,
    kind: String,
    mentions: u64,
    updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphEdge {
    from: String,
    to: String,
    relation: String,
    weight: u64,
    updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ZvecDiskState {
    version: u32,
    entries: Vec<ZvecMemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct GraphLiteDiskState {
    version: u32,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone)]
struct VectorHit {
    score: f64,
    text: String,
    source: String,
    session_key: String,
    at_ms: u64,
}

type GraphStateMaps = (HashMap<String, GraphNode>, HashMap<String, GraphEdge>);

impl PersistentMemoryRegistry {
    pub fn new() -> Self {
        let runtime = MemoryRuntimeState::default();
        let zvec_entries =
            load_zvec_entries_from_disk(&runtime.zvec_store_path).unwrap_or_else(|_| Vec::new());
        let (graph_nodes, graph_edges) = load_graph_from_disk(&runtime.graph_store_path)
            .unwrap_or_else(|_| (HashMap::new(), HashMap::new()));
        let mut state = MemoryState {
            zvec_entries,
            graph_nodes,
            graph_edges,
        };
        state.enforce_limits(runtime.max_entries);
        Self {
            runtime: Mutex::new(runtime),
            state: Mutex::new(state),
        }
    }

    pub async fn apply_runtime_config(&self, config: MemoryRuntimeConfig) -> Result<(), String> {
        let (path_changed, runtime_snapshot) = {
            let mut runtime = self.runtime.lock().await;
            let previous_zvec = runtime.zvec_store_path.clone();
            let previous_graph = runtime.graph_store_path.clone();

            if let Some(enabled) = config.enabled {
                runtime.enabled = enabled;
            }
            if let Some(path) = config
                .zvec_store_path
                .and_then(|value| normalize_optional_text(value, 2_048))
            {
                runtime.zvec_store_path = path;
            }
            if let Some(path) = config
                .graph_store_path
                .and_then(|value| normalize_optional_text(value, 2_048))
            {
                runtime.graph_store_path = path;
            }
            if let Some(max_entries) = config.max_entries {
                runtime.max_entries = max_entries.clamp(64, 500_000);
            }
            if let Some(recall_top_k) = config.recall_top_k {
                runtime.recall_top_k = recall_top_k.clamp(1, 64);
            }
            if let Some(min_score) = config.recall_min_score {
                runtime.recall_min_score = min_score.clamp(-1.0, 1.0);
            }
            let changed = previous_zvec != runtime.zvec_store_path
                || previous_graph != runtime.graph_store_path;
            (changed, runtime.clone())
        };

        if path_changed {
            let zvec_entries = load_zvec_entries_from_disk(&runtime_snapshot.zvec_store_path)?;
            let (graph_nodes, graph_edges) =
                load_graph_from_disk(&runtime_snapshot.graph_store_path)?;
            let mut state = self.state.lock().await;
            state.zvec_entries = zvec_entries;
            state.graph_nodes = graph_nodes;
            state.graph_edges = graph_edges;
            state.enforce_limits(runtime_snapshot.max_entries);
        } else {
            let mut state = self.state.lock().await;
            state.enforce_limits(runtime_snapshot.max_entries);
        }

        self.persist_snapshot().await
    }

    pub async fn remember(&self, input: MemoryRememberInput) -> Result<(), String> {
        let runtime = { self.runtime.lock().await.clone() };
        if !runtime.enabled {
            return Ok(());
        }

        let Some(session_key) = normalize_optional_text(input.session_key, 512) else {
            return Ok(());
        };
        let Some(source) = normalize_optional_text(input.source, 128) else {
            return Ok(());
        };
        let Some(text) = normalize_optional_text(input.text, MAX_MEMORY_TEXT_CHARS) else {
            return Ok(());
        };
        if text.eq_ignore_ascii_case("[attachment]") {
            return Ok(());
        }

        let at_ms = input.at_ms.unwrap_or_else(now_ms);
        let tokens = extract_keywords(&text, MAX_GRAPH_KEYWORDS);
        let vector = embed_text(&text);
        let entry = ZvecMemoryEntry {
            id: format!(
                "mem-{}-{}",
                at_ms,
                MEMORY_ENTRY_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed)
            ),
            session_key: session_key.clone(),
            source,
            text,
            request_id: input
                .request_id
                .and_then(|value| normalize_optional_text(value, 256)),
            at_ms,
            vector,
            tokens: tokens.clone(),
        };

        {
            let mut state = self.state.lock().await;
            state.zvec_entries.push(entry);
            update_graph_state(&mut state, &session_key, &tokens, at_ms);
            state.enforce_limits(runtime.max_entries);
        }

        self.persist_snapshot().await
    }

    pub async fn recall(&self, query: MemoryRecallQuery) -> MemoryRecallOutcome {
        let runtime = { self.runtime.lock().await.clone() };
        if !runtime.enabled {
            return MemoryRecallOutcome::default();
        }
        let Some(session_key) = normalize_optional_text(query.session_key, 512) else {
            return MemoryRecallOutcome::default();
        };
        let Some(query_text) = query
            .query_text
            .and_then(|value| normalize_optional_text(value, MAX_MEMORY_TEXT_CHARS))
        else {
            return MemoryRecallOutcome::default();
        };
        let query_tokens = extract_keywords(&query_text, MAX_GRAPH_KEYWORDS);
        let query_vector = embed_text(&query_text);

        let (vector_hits, graph_facts) = {
            let state = self.state.lock().await;
            let hits = collect_vector_hits(
                &state.zvec_entries,
                &runtime,
                &session_key,
                &query_tokens,
                &query_vector,
            );
            let facts = collect_graph_facts(&state, &session_key, &query_tokens);
            (hits, facts)
        };

        MemoryRecallOutcome {
            system_prompt: build_memory_system_prompt(&vector_hits, &graph_facts),
            vector_hits: vector_hits.len(),
            graph_facts: graph_facts.len(),
        }
    }

    pub async fn stats(&self) -> MemoryStats {
        let runtime = { self.runtime.lock().await.clone() };
        let state = self.state.lock().await;
        MemoryStats {
            enabled: runtime.enabled,
            zvec_entries: state.zvec_entries.len(),
            graph_nodes: state.graph_nodes.len(),
            graph_edges: state.graph_edges.len(),
            zvec_store_path: runtime.zvec_store_path,
            graph_store_path: runtime.graph_store_path,
            max_entries: runtime.max_entries,
            recall_top_k: runtime.recall_top_k,
        }
    }

    async fn persist_snapshot(&self) -> Result<(), String> {
        let runtime = { self.runtime.lock().await.clone() };
        let (zvec_snapshot, graph_snapshot) = {
            let state = self.state.lock().await;
            (
                ZvecDiskState {
                    version: 1,
                    entries: state.zvec_entries.clone(),
                },
                state.graph_snapshot(),
            )
        };
        persist_zvec_entries_to_disk(&runtime.zvec_store_path, &zvec_snapshot)?;
        persist_graph_to_disk(&runtime.graph_store_path, &graph_snapshot)?;
        Ok(())
    }
}

fn collect_vector_hits(
    entries: &[ZvecMemoryEntry],
    runtime: &MemoryRuntimeState,
    session_key: &str,
    query_tokens: &[String],
    query_vector: &[f32],
) -> Vec<VectorHit> {
    let mut hits = Vec::new();
    for entry in entries.iter().rev() {
        if entry.vector.is_empty() {
            continue;
        }
        let mut score = cosine_similarity(query_vector, &entry.vector);
        if entry.session_key.eq_ignore_ascii_case(session_key) {
            score += 0.06;
        }
        let overlap = token_overlap(query_tokens, &entry.tokens);
        if overlap > 0 {
            score += f64::from(overlap as u32) * 0.025;
        }
        if score < runtime.recall_min_score {
            continue;
        }
        hits.push(VectorHit {
            score,
            text: entry.text.clone(),
            source: entry.source.clone(),
            session_key: entry.session_key.clone(),
            at_ms: entry.at_ms,
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.at_ms.cmp(&a.at_ms))
            .then_with(|| a.text.cmp(&b.text))
    });

    let mut dedup = HashSet::new();
    let mut final_hits = Vec::new();
    for hit in hits {
        let signature = normalize_for_key(&hit.text);
        if !dedup.insert(signature) {
            continue;
        }
        final_hits.push(hit);
        if final_hits.len() >= runtime.recall_top_k {
            break;
        }
    }
    final_hits
}

fn collect_graph_facts(
    state: &MemoryState,
    session_key: &str,
    query_tokens: &[String],
) -> Vec<String> {
    let mut facts = Vec::new();
    let session_node_id = session_node_id(session_key);

    let mut session_mentions = state
        .graph_edges
        .values()
        .filter(|edge| edge.from == session_node_id && edge.relation == "mentions")
        .cloned()
        .collect::<Vec<_>>();
    session_mentions.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| b.updated_at_ms.cmp(&a.updated_at_ms))
    });

    if !session_mentions.is_empty() {
        let mut themes = Vec::new();
        for edge in session_mentions.into_iter().take(4) {
            if let Some(node) = state.graph_nodes.get(&edge.to) {
                themes.push(format!("{}({})", node.label, edge.weight));
            }
        }
        if !themes.is_empty() {
            facts.push(format!("Session themes: {}", themes.join(", ")));
        }
    }

    for token in query_tokens.iter().take(3) {
        let node_id = concept_node_id(token);
        let mentions = state
            .graph_edges
            .values()
            .filter(|edge| edge.to == node_id && edge.relation == "mentions")
            .collect::<Vec<_>>();
        if mentions.is_empty() {
            continue;
        }
        let mut sessions = HashSet::new();
        let mut total_mentions = 0u64;
        for edge in mentions {
            sessions.insert(edge.from.clone());
            total_mentions = total_mentions.saturating_add(edge.weight);
        }
        facts.push(format!(
            "\"{}\" appears across {} session nodes (mentions={})",
            token,
            sessions.len(),
            total_mentions
        ));
        if facts.len() >= MAX_GRAPH_FACTS {
            break;
        }
    }

    facts
}

fn build_memory_system_prompt(vector_hits: &[VectorHit], graph_facts: &[String]) -> Option<String> {
    if vector_hits.is_empty() && graph_facts.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Persistent memory context (zvec + graphlite). Use as soft context and prioritize current user instructions if conflict appears.",
    );
    if !vector_hits.is_empty() {
        out.push_str("\nVector recalls:");
        for hit in vector_hits.iter().take(MAX_VECTOR_HITS_FOR_PROMPT) {
            let source = truncate_text(&hit.source, 48);
            let session = truncate_text(&hit.session_key, 96);
            let text = truncate_text(&hit.text, 220);
            out.push_str(&format!(
                "\n- score={:.3} source={} session={} text={}",
                hit.score, source, session, text
            ));
        }
    }
    if !graph_facts.is_empty() {
        out.push_str("\nGraph facts:");
        for fact in graph_facts.iter().take(MAX_GRAPH_FACTS) {
            out.push_str(&format!("\n- {}", truncate_text(fact, 220)));
        }
    }
    Some(truncate_text(&out, MAX_MEMORY_PROMPT_CHARS))
}

fn update_graph_state(state: &mut MemoryState, session_key: &str, tokens: &[String], at_ms: u64) {
    let session_id = session_node_id(session_key);
    state
        .graph_nodes
        .entry(session_id.clone())
        .and_modify(|node| {
            node.updated_at_ms = at_ms;
            node.mentions = node.mentions.saturating_add(1);
        })
        .or_insert_with(|| GraphNode {
            id: session_id.clone(),
            label: session_key.to_owned(),
            kind: "session".to_owned(),
            mentions: 1,
            updated_at_ms: at_ms,
        });

    let mut ordered_concepts = Vec::new();
    let mut seen = HashSet::new();
    for token in tokens.iter().take(MAX_GRAPH_KEYWORDS) {
        if !seen.insert(token.clone()) {
            continue;
        }
        let concept_id = concept_node_id(token);
        ordered_concepts.push(concept_id.clone());
        state
            .graph_nodes
            .entry(concept_id.clone())
            .and_modify(|node| {
                node.updated_at_ms = at_ms;
                node.mentions = node.mentions.saturating_add(1);
            })
            .or_insert_with(|| GraphNode {
                id: concept_id.clone(),
                label: token.clone(),
                kind: "concept".to_owned(),
                mentions: 1,
                updated_at_ms: at_ms,
            });

        upsert_graph_edge(state, &session_id, "mentions", &concept_id, at_ms);
    }

    for pair in ordered_concepts.windows(2) {
        let from = pair[0].as_str();
        let to = pair[1].as_str();
        upsert_graph_edge(state, from, "co_occurs", to, at_ms);
    }
}

fn upsert_graph_edge(state: &mut MemoryState, from: &str, relation: &str, to: &str, at_ms: u64) {
    let key = graph_edge_key(from, relation, to);
    state
        .graph_edges
        .entry(key)
        .and_modify(|edge| {
            edge.updated_at_ms = at_ms;
            edge.weight = edge.weight.saturating_add(1);
        })
        .or_insert_with(|| GraphEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            relation: relation.to_owned(),
            weight: 1,
            updated_at_ms: at_ms,
        });
}

fn graph_edge_key(from: &str, relation: &str, to: &str) -> String {
    format!("{from}|{relation}|{to}")
}

fn session_node_id(session_key: &str) -> String {
    format!("session:{}", normalize_for_key(session_key))
}

fn concept_node_id(token: &str) -> String {
    format!("concept:{}", normalize_for_key(token))
}

fn load_zvec_entries_from_disk(path: &str) -> Result<Vec<ZvecMemoryEntry>, String> {
    if is_memory_path(path) {
        return Ok(Vec::new());
    }
    let file_path = PathBuf::from(path);
    if !file_path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&file_path)
        .map_err(|err| format!("failed reading zvec store {}: {err}", file_path.display()))?;
    let parsed = serde_json::from_str::<ZvecDiskState>(&raw).or_else(|_| {
        serde_json::from_str::<Vec<ZvecMemoryEntry>>(&raw).map(|entries| ZvecDiskState {
            version: 1,
            entries,
        })
    });
    parsed
        .map(|state| state.entries)
        .map_err(|err| format!("failed parsing zvec store {}: {err}", file_path.display()))
}

fn load_graph_from_disk(path: &str) -> Result<GraphStateMaps, String> {
    if is_memory_path(path) {
        return Ok((HashMap::new(), HashMap::new()));
    }
    let file_path = PathBuf::from(path);
    if !file_path.exists() {
        return Ok((HashMap::new(), HashMap::new()));
    }
    let raw = std::fs::read_to_string(&file_path)
        .map_err(|err| format!("failed reading graph store {}: {err}", file_path.display()))?;
    let parsed = serde_json::from_str::<GraphLiteDiskState>(&raw).or_else(|_| {
        #[derive(Deserialize)]
        struct LegacyGraphDiskState {
            nodes: Vec<GraphNode>,
            edges: Vec<GraphEdge>,
        }
        serde_json::from_str::<LegacyGraphDiskState>(&raw).map(|legacy| GraphLiteDiskState {
            version: 1,
            nodes: legacy.nodes,
            edges: legacy.edges,
        })
    });
    let parsed = parsed
        .map_err(|err| format!("failed parsing graph store {}: {err}", file_path.display()))?;
    let nodes = parsed
        .nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect::<HashMap<_, _>>();
    let edges = parsed
        .edges
        .into_iter()
        .map(|edge| (graph_edge_key(&edge.from, &edge.relation, &edge.to), edge))
        .collect::<HashMap<_, _>>();
    Ok((nodes, edges))
}

fn persist_zvec_entries_to_disk(path: &str, snapshot: &ZvecDiskState) -> Result<(), String> {
    if is_memory_path(path) {
        return Ok(());
    }
    let payload = serde_json::to_vec_pretty(snapshot)
        .map_err(|err| format!("failed serializing zvec store: {err}"))?;
    write_json_atomic(path, &payload)
}

fn persist_graph_to_disk(path: &str, snapshot: &GraphLiteDiskState) -> Result<(), String> {
    if is_memory_path(path) {
        return Ok(());
    }
    let payload = serde_json::to_vec_pretty(snapshot)
        .map_err(|err| format!("failed serializing graph store: {err}"))?;
    write_json_atomic(path, &payload)
}

fn write_json_atomic(path: &str, payload: &[u8]) -> Result<(), String> {
    let store_path = PathBuf::from(path);
    if let Some(parent) = store_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed creating memory store directory {}: {err}",
                    parent.display()
                )
            })?;
        }
    }
    let mut temp_path = store_path.clone();
    let temp_extension = format!(
        "{}.tmp.{}",
        store_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("json"),
        now_ms()
    );
    temp_path.set_extension(temp_extension);
    std::fs::write(&temp_path, payload).map_err(|err| {
        format!(
            "failed writing memory store temp file {}: {err}",
            temp_path.display()
        )
    })?;
    if store_path.exists() {
        let _ = std::fs::remove_file(&store_path);
    }
    std::fs::rename(&temp_path, &store_path).map_err(|err| {
        format!(
            "failed moving memory store temp file into place {}: {err}",
            store_path.display()
        )
    })
}

fn is_memory_path(path: &str) -> bool {
    path.trim().to_ascii_lowercase().starts_with("memory://")
}

fn embed_text(text: &str) -> Vec<f32> {
    let mut vector = vec![0f32; VECTOR_DIM];
    for token in tokenize(text) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut hasher);
        let hash = hasher.finish();
        let idx = (hash as usize) % VECTOR_DIM;
        let sign = if (hash >> 63) == 0 { 1.0 } else { -1.0 };
        let magnitude = 1.0 + (((hash >> 32) & 0x7f) as f32) / 256.0;
        vector[idx] += sign * magnitude;
    }
    normalize_vector(&mut vector);
    vector
}

fn normalize_vector(vector: &mut [f32]) {
    let norm_sq = vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>();
    if norm_sq <= 0.0 {
        return;
    }
    let norm = norm_sq.sqrt() as f32;
    if norm <= 0.0 {
        return;
    }
    for value in vector.iter_mut() {
        *value /= norm;
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f64 {
    let len = left.len().min(right.len());
    if len == 0 {
        return 0.0;
    }
    let mut dot = 0f64;
    for idx in 0..len {
        dot += f64::from(left[idx]) * f64::from(right[idx]);
    }
    dot
}

fn token_overlap(left: &[String], right: &[String]) -> usize {
    if left.is_empty() || right.is_empty() {
        return 0;
    }
    let right_set = right
        .iter()
        .map(|value| normalize_for_key(value))
        .collect::<HashSet<_>>();
    left.iter()
        .filter(|token| right_set.contains(&normalize_for_key(token)))
        .count()
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if current.len() < 64 {
                current.push(ch.to_ascii_lowercase());
            }
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn extract_keywords(text: &str, max_keywords: usize) -> Vec<String> {
    let mut counts = HashMap::<String, usize>::new();
    for token in tokenize(text) {
        if token.len() < 3 || is_stopword(&token) {
            continue;
        }
        *counts.entry(token).or_insert(0) += 1;
    }
    let mut items = counts.into_iter().collect::<Vec<_>>();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items
        .into_iter()
        .map(|(token, _)| token)
        .take(max_keywords)
        .collect()
}

fn is_stopword(token: &str) -> bool {
    const STOPWORDS: &[&str] = &[
        "the",
        "and",
        "for",
        "with",
        "that",
        "this",
        "from",
        "have",
        "been",
        "you",
        "your",
        "are",
        "was",
        "were",
        "not",
        "but",
        "all",
        "any",
        "can",
        "had",
        "has",
        "into",
        "its",
        "our",
        "out",
        "per",
        "via",
        "who",
        "why",
        "how",
        "when",
        "where",
        "what",
        "which",
        "would",
        "should",
        "could",
        "about",
        "after",
        "before",
        "then",
        "than",
        "there",
        "their",
        "they",
        "them",
        "here",
        "also",
        "just",
        "like",
        "make",
        "made",
        "many",
        "more",
        "most",
        "other",
        "some",
        "such",
        "very",
        "only",
        "over",
        "under",
        "onto",
        "each",
        "every",
        "while",
        "because",
        "between",
        "without",
        "within",
        "across",
        "still",
        "again",
        "agent",
        "assistant",
        "user",
    ];
    STOPWORDS
        .iter()
        .any(|value| value.eq_ignore_ascii_case(token))
}

fn truncate_text(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_owned();
    }
    let mut end = max_len;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = value[..end].to_owned();
    out.push_str("...");
    out
}

fn normalize_optional_text(value: String, max_len: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() <= max_len {
        return Some(trimmed.to_owned());
    }
    let mut end = max_len;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    Some(trimmed[..end].to_owned())
}

fn normalize_for_key(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{
        MemoryRecallQuery, MemoryRememberInput, MemoryRuntimeConfig, PersistentMemoryRegistry,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_memory_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("openclaw-rs-memory-{name}-{stamp}"))
    }

    #[tokio::test]
    async fn memory_recall_includes_vector_hits_and_graph_facts() {
        let root = temp_memory_root("recall");
        let zvec = root.join("zvec.json");
        let graph = root.join("graph.json");
        let registry = PersistentMemoryRegistry::new();
        registry
            .apply_runtime_config(MemoryRuntimeConfig {
                enabled: Some(true),
                zvec_store_path: Some(zvec.to_string_lossy().to_string()),
                graph_store_path: Some(graph.to_string_lossy().to_string()),
                max_entries: Some(128),
                recall_top_k: Some(4),
                recall_min_score: Some(0.05),
            })
            .await
            .expect("apply runtime config");

        registry
            .remember(MemoryRememberInput {
                session_key: "main".to_owned(),
                source: "agent.user".to_owned(),
                text: "Project codename is iron claw and target is ubuntu 20.04".to_owned(),
                request_id: Some("run-1".to_owned()),
                at_ms: None,
            })
            .await
            .expect("remember user");

        registry
            .remember(MemoryRememberInput {
                session_key: "main".to_owned(),
                source: "agent.assistant".to_owned(),
                text: "We should keep iron claw parity and validate with docker.".to_owned(),
                request_id: Some("run-1".to_owned()),
                at_ms: None,
            })
            .await
            .expect("remember assistant");

        let recall = registry
            .recall(MemoryRecallQuery {
                session_key: "main".to_owned(),
                query_text: Some("what is the project codename".to_owned()),
            })
            .await;
        assert!(recall.vector_hits >= 1);
        assert!(recall.graph_facts >= 1);
        let prompt = recall.system_prompt.unwrap_or_default();
        assert!(prompt.to_ascii_lowercase().contains("iron"));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn memory_persists_across_registry_instances() {
        let root = temp_memory_root("persist");
        let zvec = root.join("zvec.json");
        let graph = root.join("graph.json");

        let first = PersistentMemoryRegistry::new();
        first
            .apply_runtime_config(MemoryRuntimeConfig {
                enabled: Some(true),
                zvec_store_path: Some(zvec.to_string_lossy().to_string()),
                graph_store_path: Some(graph.to_string_lossy().to_string()),
                max_entries: Some(128),
                recall_top_k: Some(6),
                recall_min_score: Some(0.05),
            })
            .await
            .expect("apply runtime config");
        first
            .remember(MemoryRememberInput {
                session_key: "agent:main:discord:group:test".to_owned(),
                source: "agent.user".to_owned(),
                text: "Release tag for this week is v1.6.4".to_owned(),
                request_id: Some("run-2".to_owned()),
                at_ms: None,
            })
            .await
            .expect("remember text");

        let restarted = PersistentMemoryRegistry::new();
        restarted
            .apply_runtime_config(MemoryRuntimeConfig {
                enabled: Some(true),
                zvec_store_path: Some(zvec.to_string_lossy().to_string()),
                graph_store_path: Some(graph.to_string_lossy().to_string()),
                max_entries: Some(128),
                recall_top_k: Some(6),
                recall_min_score: Some(0.05),
            })
            .await
            .expect("apply runtime config on restart");

        let stats = restarted.stats().await;
        assert!(stats.zvec_entries >= 1);
        assert!(stats.graph_nodes >= 1);

        let recall = restarted
            .recall(MemoryRecallQuery {
                session_key: "agent:main:discord:group:test".to_owned(),
                query_text: Some("what release tag did we choose".to_owned()),
            })
            .await;
        assert!(recall.vector_hits >= 1);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
