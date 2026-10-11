//! File-granular graph evidence with deterministic, attempt-based work admission.
//! Output trimming is independent of traversal. Work caps bound decoded/admitted
//! evidence and store calls, not SQLite scans/sorts, elapsed time or byte sizes.
use crate::search::{SearchHit, SearchOptions, Searcher};
use crate::store::{IndexStore, SymbolLocationRow};
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
pub const DEFAULT_CHAIN_WORK: usize = 8192;
pub const MAX_CHAIN_WORK: usize = 1_000_000;
const MAX_CALLEE_DEFINITIONS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainTruncation {
    WorkLimit,
    DefinitionLimit,
}

/// Attempted work includes duplicate, unresolved and already-seen evidence.
/// Source pages include one lookahead row and can contain unprocessed tails.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChainWork {
    pub limit: usize,
    pub used: usize,
    pub expansions: usize,
    pub store_calls: usize,
    pub call_rows: usize,
    pub import_rows: usize,
    pub definition_rows: usize,
    pub module_paths: usize,
    pub fetched_source_rows: usize,
    pub peak_source_page_rows: usize,
}

fn is_false(value: &bool) -> bool {
    !*value
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
    /// Work or definition admission omitted exploration. Output-only trimming
    /// remains visible through node_count > nodes.len(), not this flag.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub truncation_reasons: Vec<ChainTruncation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<ChainWork>,
}
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub max_depth: u32,
    pub decay_factor: f64,
    pub limit: usize,
    pub top_n: usize,
    /// Expansion + store-call + processed-row units, after seed selection.
    /// Zero is seeds-only; public host knobs accept 1..=MAX_CHAIN_WORK.
    pub max_work: usize,
}
impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            decay_factor: 0.5,
            limit: 100,
            top_n: 20,
            max_work: DEFAULT_CHAIN_WORK,
        }
    }
}
enum WorkKind {
    Expansion,
    StoreCall,
    CallRow,
    ImportRow,
    DefinitionRow,
    ModulePath,
}

struct WorkBudget {
    report: ChainWork,
    exhausted: bool,
    definition_limited: bool,
}
impl WorkBudget {
    fn new(limit: usize) -> Self {
        Self {
            report: ChainWork {
                limit,
                ..ChainWork::default()
            },
            exhausted: false,
            definition_limited: false,
        }
    }
    fn remaining(&self) -> usize {
        self.report.limit - self.report.used
    }
    fn take(&mut self, kind: WorkKind) -> bool {
        if self.remaining() == 0 {
            self.exhausted = true;
            return false;
        }
        self.report.used += 1;
        let counter = match kind {
            WorkKind::Expansion => &mut self.report.expansions,
            WorkKind::StoreCall => &mut self.report.store_calls,
            WorkKind::CallRow => &mut self.report.call_rows,
            WorkKind::ImportRow => &mut self.report.import_rows,
            WorkKind::DefinitionRow => &mut self.report.definition_rows,
            WorkKind::ModulePath => &mut self.report.module_paths,
        };
        *counter += 1;
        true
    }
    fn source_rows<T>(&mut self, fetch: impl FnOnce(usize) -> Result<Vec<T>>) -> Result<Vec<T>> {
        if !self.take(WorkKind::StoreCall) {
            return Ok(Vec::new());
        }
        let rows = fetch(self.remaining() + 1)?;
        self.report.fetched_source_rows += rows.len();
        self.report.peak_source_page_rows = self.report.peak_source_page_rows.max(rows.len());
        Ok(rows)
    }
    fn reasons(&self) -> Vec<ChainTruncation> {
        let mut reasons = Vec::new();
        if self.exhausted {
            reasons.push(ChainTruncation::WorkLimit);
        }
        if self.definition_limited {
            reasons.push(ChainTruncation::DefinitionLimit);
        }
        reasons
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
fn seed_location(store: &IndexStore, hit: &SearchHit) -> Result<Option<SymbolLocationRow>> {
    let at_hit = |name: String, line_start: u32, line_end: u32| SymbolLocationRow {
        path: hit.file.clone(),
        name,
        language: hit.language.clone(),
        line_start,
        line_end,
    };
    if let Some(ref symbol) = hit.symbol {
        return Ok(Some(at_hit(symbol.clone(), hit.line_start, hit.line_end)));
    }
    // Preserve callee-name preference, but do not attribute its definition to
    // the reference site. Resolve only an unambiguous indexed definition;
    // external/duplicate names retain the existing reference-site evidence.
    let name = hit
        .callee
        .as_deref()
        .filter(|name| !name.is_empty())
        .or_else(|| hit.caller.as_deref().filter(|name| !name.is_empty()));
    if let Some(name) = name {
        let mut definitions = store.symbols_named(name, 2)?;
        if definitions.len() == 1 {
            return Ok(definitions.pop());
        }
        return Ok(Some(at_hit(name.to_owned(), hit.line_start, hit.line_end)));
    }
    // Never invent a seed via first_symbol_in_file: unrelated top-of-file
    // declarations are not evidence for an unlabelled search hit.
    Ok(store
        .symbol_at_line(&hit.file, hit.line_start)
        .ok()
        .flatten())
}
fn build_seeds(store: &IndexStore, hits: &[SearchHit], top_n: usize) -> Result<Vec<ChainEntry>> {
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
    let mut seeds = Vec::new();
    for (_, hit) in sorted {
        if let Some(location) = seed_location(store, hit)? {
            seeds.push(ChainEntry {
                file: location.path,
                symbol: location.name,
                language: location.language,
                line_start: location.line_start,
                line_end: location.line_end,
                score: hit.score,
                depth: 0,
            });
        }
    }
    // Different reference files can resolve to the same definition file.
    // Reuse the walk's file-granular score/depth policy after resolution.
    Ok(dedup_by_file(&seeds))
}
fn expand_one(
    store: &IndexStore,
    entry: &ChainEntry,
    depth: u32,
    decay: f64,
    work: &mut WorkBudget,
) -> Result<(Vec<ChainEntry>, Vec<ChainEdge>)> {
    let hop_score = entry.score * decay;
    let mut entries = Vec::new();
    let mut edges = Vec::new();
    'outgoing: for (_call_file, _call_line, _caller, callee) in
        work.source_rows(|limit| store.outgoing_calls_capped(&entry.symbol, limit))?
    {
        if !work.take(WorkKind::CallRow) || !work.take(WorkKind::StoreCall) {
            break;
        }
        let definitions = store.symbols_named(&callee, MAX_CALLEE_DEFINITIONS + 1)?;
        work.definition_limited |= definitions.len() > MAX_CALLEE_DEFINITIONS;
        for def in definitions.into_iter().take(MAX_CALLEE_DEFINITIONS) {
            if !work.take(WorkKind::DefinitionRow) {
                break 'outgoing;
            }
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
    for (call_file, call_line, caller, _callee) in
        work.source_rows(|limit| store.incoming_calls_capped(&entry.symbol, limit))?
    {
        if !work.take(WorkKind::CallRow) {
            break;
        }
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
    'imports: for imp in
        work.source_rows(|limit| store.imports_from_file_capped(&entry.file, limit))?
    {
        if !work.take(WorkKind::ImportRow) || !work.take(WorkKind::StoreCall) {
            break;
        }
        // Module candidates are the existing finite language-rule set, not a
        // repository-wide filename scan. Charge each resolved path and lookup.
        for file_path in store.resolve_module_path(&entry.file, &imp.module_path)? {
            if !work.take(WorkKind::ModulePath) {
                break 'imports;
            }
            if file_path == entry.file {
                continue;
            }
            if !work.take(WorkKind::StoreCall) {
                break 'imports;
            }
            if let Ok(Some(first_sym)) = store.first_symbol_in_file(&file_path) {
                if !work.take(WorkKind::DefinitionRow) {
                    break 'imports;
                }
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
    if config.max_work > MAX_CHAIN_WORK {
        return Err(crate::StoreError::Other(format!(
            "chain max_work exceeds {MAX_CHAIN_WORK}"
        )));
    }
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
            truncated: false,
            truncation_reasons: Vec::new(),
            work: None,
        });
    }
    let searcher = Searcher::new(SearchOptions {
        root: store.root().to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        limit: config.top_n.max(1),
        ..SearchOptions::default()
    })?;
    let seeds = build_seeds(store, &searcher.search(query)?.hits, config.top_n)?;
    let seed_nodes: Vec<ChainNode> = seeds.iter().map(entry_to_node).collect();
    let mut all_entries = seeds.clone();
    let mut all_edges = Vec::new();
    let mut frontier = seeds;
    let mut work = WorkBudget::new(config.max_work);
    let mut seen_files: HashSet<String> = all_entries.iter().map(|e| e.file.clone()).collect();
    for d in 1..=config.max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for entry in &frontier {
            if !work.take(WorkKind::Expansion) {
                break;
            }
            let (entries, edges) = expand_one(store, entry, d, config.decay_factor, &mut work)?;
            next.extend(entries);
            all_edges.extend(edges);
            if work.exhausted {
                break;
            }
        }
        let novel: Vec<_> = dedup_by_file(&next)
            .into_iter()
            .filter(|e| seen_files.insert(e.file.clone()))
            .collect();
        all_entries.extend(novel.clone());
        frontier = novel;
        if work.exhausted {
            break;
        }
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
    all_entries.sort_by(entry_cmp);
    let total_nodes = all_entries.len();
    all_entries.truncate(config.limit);
    // After node truncate, edges must be a subset of remaining nodes.
    let node_files: HashSet<&str> = all_entries.iter().map(|e| e.file.as_str()).collect();
    all_edges.retain(|e| {
        node_files.contains(e.from_file.as_str()) && node_files.contains(e.to_file.as_str())
    });
    let reasons = work.reasons();
    let truncated = !reasons.is_empty();
    Ok(ChainResponse {
        truncated,
        truncation_reasons: reasons,
        work: truncated.then_some(work.report),
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
