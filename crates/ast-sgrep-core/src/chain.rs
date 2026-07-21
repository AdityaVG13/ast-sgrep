use crate::search::{SearchHit, SearchOptions, Searcher};
use crate::store::IndexStore;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeLabel {
    Calls,
    CalledBy,
    Imports,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainNode {
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub score: f64,
    pub depth: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainEdge {
    pub from_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_symbol: Option<String>,
    pub to_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_symbol: Option<String>,
    pub label: EdgeLabel,
    pub depth: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainResponse {
    pub query: String,
    pub seeds: Vec<ChainNode>,
    pub nodes: Vec<ChainNode>,
    pub edges: Vec<ChainEdge>,
    pub max_depth: u32,
    pub decay_factor: f64,
    pub node_count: usize,
    pub edge_count: usize,
}
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub max_depth: u32,
    pub decay_factor: f64,
    pub limit: usize,
    pub top_n: usize,
}
impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            decay_factor: 0.5,
            limit: 100,
            top_n: 20,
        }
    }
}
#[derive(Debug, Clone)]
struct ChainEntry {
    file: String,
    symbol: String,
    language: Option<String>,
    line_start: u32,
    line_end: u32,
    score: f64,
    depth: u32,
}
fn hit_symbol(store: &IndexStore, hit: &SearchHit) -> Option<(String, u32, u32)> {
    if let Some(ref sym) = hit.symbol {
        return Some((sym.clone(), hit.line_start, hit.line_end));
    }
    if let Ok(Some(row)) = store.symbol_at_line(&hit.file, hit.line_start) {
        return Some((row.name, row.line_start, row.line_end));
    }
    store
        .first_symbol_in_file(&hit.file)
        .ok()
        .flatten()
        .map(|r| (r.name, r.line_start, r.line_end))
}
fn build_seeds(store: &IndexStore, hits: &[SearchHit], top_n: usize) -> Vec<ChainEntry> {
    let mut best_per_file: HashMap<&str, (&SearchHit, usize)> = HashMap::new();
    for (idx, hit) in hits.iter().enumerate() {
        let key = hit.file.as_str();
        match best_per_file.get(key) {
            Some(&(_, existing_idx)) if hits[existing_idx].score >= hit.score => {}
            _ => {
                best_per_file.insert(key, (hit, idx));
            }
        }
        if best_per_file.len() >= top_n {
            break;
        }
    }
    let mut sorted: Vec<_> = best_per_file
        .values()
        .map(|(hit, _)| (hit.file.as_str(), *hit))
        .collect();
    sorted.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    sorted
        .into_iter()
        .filter_map(|(_, hit)| {
            hit_symbol(store, hit).map(|(sym, ls, le)| ChainEntry {
                file: hit.file.clone(),
                symbol: sym,
                language: hit.language.clone(),
                line_start: ls,
                line_end: le,
                score: hit.score,
                depth: 0,
            })
        })
        .collect()
}
fn expand_one(
    store: &IndexStore,
    entry: &ChainEntry,
    depth: u32,
    decay: f64,
) -> Result<(Vec<ChainEntry>, Vec<ChainEdge>)> {
    let hop_score = entry.score * decay;
    let mut entries = Vec::new();
    let mut edges = Vec::new();
    for (_call_file, _call_line, _caller, callee) in store.outgoing_calls(&entry.symbol)? {
        for def in store.symbols_named(&callee, 8)? {
            entries.push(ChainEntry {
                file: def.path.clone(),
                symbol: def.name.clone(),
                language: def.language.clone(),
                line_start: def.line_start,
                line_end: def.line_end,
                score: hop_score,
                depth,
            });
            edges.push(ChainEdge {
                from_file: entry.file.clone(),
                from_symbol: Some(entry.symbol.clone()),
                to_file: def.path.clone(),
                to_symbol: Some(def.name.clone()),
                label: EdgeLabel::Calls,
                depth: entry.depth,
            });
        }
    }
    for (call_file, call_line, caller, _callee) in store.incoming_calls(&entry.symbol)? {
        entries.push(ChainEntry {
            file: call_file.clone(),
            symbol: caller.clone(),
            language: None,
            line_start: call_line,
            line_end: call_line,
            score: hop_score,
            depth,
        });
        edges.push(ChainEdge {
            from_file: call_file,
            from_symbol: Some(caller),
            to_file: entry.file.clone(),
            to_symbol: Some(entry.symbol.clone()),
            label: EdgeLabel::CalledBy,
            depth: entry.depth,
        });
    }
    for imp in store.imports_from_file(&entry.file)? {
        for file_path in store.resolve_module_path(&entry.file, &imp.module_path)? {
            if file_path == entry.file {
                continue;
            }
            if let Ok(Some(first_sym)) = store.first_symbol_in_file(&file_path) {
                entries.push(ChainEntry {
                    file: file_path.clone(),
                    symbol: first_sym.name.clone(),
                    language: first_sym.language.clone(),
                    line_start: first_sym.line_start,
                    line_end: first_sym.line_end,
                    score: hop_score,
                    depth,
                });
                edges.push(ChainEdge {
                    from_file: entry.file.clone(),
                    from_symbol: Some(entry.symbol.clone()),
                    to_file: file_path,
                    to_symbol: Some(first_sym.name),
                    label: EdgeLabel::Imports,
                    depth: entry.depth,
                });
            }
        }
    }
    Ok((entries, edges))
}
fn dedup_by_file(entries: &[ChainEntry]) -> Vec<ChainEntry> {
    let mut best: HashMap<&str, &ChainEntry> = HashMap::new();
    for e in entries {
        match best.get(e.file.as_str()) {
            Some(&existing)
                if existing.score > e.score
                    || (existing.score == e.score && existing.depth <= e.depth) => {}
            _ => {
                best.insert(e.file.as_str(), e);
            }
        }
    }
    let mut result: Vec<ChainEntry> = best.values().cloned().cloned().collect();
    result.sort_by(entry_cmp);
    result
}
fn entry_cmp(a: &ChainEntry, b: &ChainEntry) -> std::cmp::Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.file.cmp(&b.file))
        .then_with(|| a.symbol.cmp(&b.symbol))
        .then_with(|| a.line_start.cmp(&b.line_start))
}
pub fn expand_chain(
    store: &IndexStore,
    query: &str,
    config: &ChainConfig,
) -> Result<ChainResponse> {
    if query.trim().is_empty() {
        return Ok(ChainResponse {
            query: query.to_string(),
            seeds: vec![],
            nodes: vec![],
            edges: vec![],
            max_depth: config.max_depth,
            decay_factor: config.decay_factor,
            node_count: 0,
            edge_count: 0,
        });
    }
    let searcher = Searcher::new(SearchOptions {
        root: store.root().to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        limit: config.top_n.max(1),
        ..SearchOptions::default()
    })?;
    let seeds = build_seeds(store, &searcher.search(query)?.hits, config.top_n);
    let seed_nodes: Vec<ChainNode> = seeds.iter().map(entry_to_node).collect();
    let mut all_entries = seeds.clone();
    let mut all_edges = Vec::new();
    let mut frontier = seeds;
    let mut seen_files: HashSet<String> = all_entries.iter().map(|e| e.file.clone()).collect();
    for d in 1..=config.max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for entry in &frontier {
            let (entries, edges) = expand_one(store, entry, d, config.decay_factor)?;
            next.extend(entries);
            all_edges.extend(edges);
        }
        let novel: Vec<_> = dedup_by_file(&next)
            .into_iter()
            .filter(|e| seen_files.insert(e.file.clone()))
            .collect();
        all_entries.extend(novel.clone());
        frontier = novel;
    }
    let mut edge_set = HashSet::new();
    all_edges.retain(|e| {
        edge_set.insert((
            e.from_file.clone(),
            e.from_symbol.clone(),
            e.to_file.clone(),
            e.to_symbol.clone(),
            e.label,
        ))
    });
    let node_files: HashSet<&str> = all_entries.iter().map(|e| e.file.as_str()).collect();
    all_edges.retain(|e| {
        node_files.contains(e.from_file.as_str()) && node_files.contains(e.to_file.as_str())
    });
    all_entries.sort_by(entry_cmp);
    let total_nodes = all_entries.len();
    all_entries.truncate(config.limit);
    Ok(ChainResponse {
        query: query.to_string(),
        seeds: seed_nodes,
        node_count: total_nodes,
        edge_count: all_edges.len(),
        nodes: all_entries.iter().map(entry_to_node).collect(),
        edges: all_edges,
        max_depth: config.max_depth,
        decay_factor: config.decay_factor,
    })
}
fn entry_to_node(e: &ChainEntry) -> ChainNode {
    ChainNode {
        file: e.file.clone(),
        line_start: e.line_start,
        line_end: e.line_end,
        symbol: Some(e.symbol.clone()),
        language: e.language.clone(),
        score: e.score,
        depth: e.depth,
    }
}
