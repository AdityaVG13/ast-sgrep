//! Bounded interprocedural call paths.
//!
//! This is intentionally not value-flow analysis: it finds a directed path
//! through indexed caller/callee relations and reports how each hop resolved.

use crate::resolution::{Resolution, SymbolId};
use crate::store::{CallEvidenceRow, IndexStore};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct CallPathConfig {
    pub max_depth: u32,
    pub max_nodes: usize,
    pub max_edges: usize,
}

impl Default for CallPathConfig {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_nodes: 10_000,
            max_edges: 50_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallPathHop {
    pub caller: String,
    pub callee: String,
    pub file: String,
    pub line: u32,
    pub resolution: Resolution,
    pub precise: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallPathResponse {
    pub source: String,
    pub sink: String,
    /// True when a symbol-name path was found inside the configured bounds.
    pub found: bool,
    /// Always `call_graph_only`: this API does not track values or sanitizers.
    pub semantics: String,
    pub path: Vec<CallPathHop>,
    pub depth: Option<u32>,
    pub max_depth: u32,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub visited_nodes: usize,
    pub explored_edges: usize,
    /// A node or edge cap stopped exploration. A miss is incomplete when true.
    pub truncated: bool,
}

#[derive(Debug, Clone)]
struct Predecessor {
    parent: String,
    call: CallEvidenceRow,
}

fn normalized(symbol: &str) -> String {
    symbol.to_ascii_lowercase()
}

fn has_unique_definition(store: &IndexStore, symbol: &str) -> Result<bool> {
    Ok(store.symbols_named(symbol, 2)?.len() == 1)
}

fn unresolved_response(source: &str, sink: &str, config: &CallPathConfig) -> CallPathResponse {
    CallPathResponse {
        source: source.to_owned(),
        sink: sink.to_owned(),
        found: false,
        semantics: "call_graph_only".to_owned(),
        path: Vec::new(),
        depth: None,
        max_depth: config.max_depth,
        max_nodes: config.max_nodes,
        max_edges: config.max_edges,
        visited_nodes: usize::from(!source.is_empty()),
        explored_edges: 0,
        truncated: false,
    }
}

fn hop_resolution(store: &IndexStore, call: &CallEvidenceRow) -> Result<Resolution> {
    if call.scip_exact {
        return Ok(Resolution::ScipOccurrence);
    }
    let (by_file, repository_candidates) = store.symbol_name_candidate_counts(&call.callee)?;
    let same_file_candidates = by_file.get(&call.file).copied().unwrap_or(0);
    let candidates = store
        .symbols_named(&call.callee, 4)?
        .into_iter()
        .map(|symbol| SymbolId::new(symbol.path, symbol.name).with_language(symbol.language));
    Ok(Resolution::from_candidates(
        same_file_candidates,
        repository_candidates,
        candidates,
    ))
}

fn reconstruct_path(
    store: &IndexStore,
    source: &str,
    sink: &str,
    predecessors: &HashMap<String, Predecessor>,
) -> Result<Vec<CallPathHop>> {
    let mut current = sink.to_owned();
    let mut calls = Vec::new();
    while current != source {
        let predecessor = predecessors
            .get(&current)
            .expect("visited call-path node has a predecessor");
        calls.push(predecessor.call.clone());
        current.clone_from(&predecessor.parent);
    }
    calls.reverse();
    calls
        .into_iter()
        .map(|call| {
            let resolution = hop_resolution(store, &call)?;
            Ok(CallPathHop {
                caller: call.caller,
                callee: call.callee,
                file: call.file,
                line: call.line,
                precise: resolution.is_precise(),
                resolution,
            })
        })
        .collect()
}

/// Find a shortest directed caller-to-callee path inside strict resource caps.
///
/// The BFS stores one predecessor per symbol instead of cloning whole paths.
/// Resolution queries run only for the returned path, not every explored edge.
pub fn find_call_path(
    store: &IndexStore,
    source: &str,
    sink: &str,
    config: &CallPathConfig,
) -> Result<CallPathResponse> {
    crate::validate_query_len(source).map_err(crate::StoreError::Other)?;
    crate::validate_query_len(sink).map_err(crate::StoreError::Other)?;
    let source = source.trim();
    let sink = sink.trim();
    let mut response = unresolved_response(source, sink, config);
    if source.is_empty() || sink.is_empty() || config.max_nodes == 0 || config.max_edges == 0 {
        response.truncated = config.max_nodes == 0 || config.max_edges == 0;
        return Ok(response);
    }

    let source_key = normalized(source);
    let sink_key = normalized(sink);
    if source_key == sink_key {
        response.found = true;
        response.depth = Some(0);
        return Ok(response);
    }

    let mut seen = HashSet::from([source_key.clone()]);
    let mut predecessors = HashMap::new();
    let mut frontier = VecDeque::from([(
        source.to_owned(),
        0u32,
        has_unique_definition(store, source)?,
    )]);

    'search: while let Some((caller, depth, may_continue)) = frontier.pop_front() {
        if depth >= config.max_depth {
            continue;
        }
        let remaining_edges = config.max_edges.saturating_sub(response.explored_edges);
        if remaining_edges == 0 {
            response.truncated = true;
            break;
        }
        let calls = store.outgoing_calls_with_scip(&caller, remaining_edges.saturating_add(1))?;
        if calls.len() > remaining_edges {
            response.truncated = true;
        }
        for call in calls.into_iter().take(remaining_edges) {
            response.explored_edges += 1;
            let callee_key = normalized(&call.callee);
            if seen.contains(&callee_key) {
                continue;
            }
            if seen.len() >= config.max_nodes {
                response.truncated = true;
                continue;
            }
            seen.insert(callee_key.clone());
            predecessors.insert(
                callee_key.clone(),
                Predecessor {
                    parent: normalized(&call.caller),
                    call: call.clone(),
                },
            );
            if callee_key == sink_key {
                response.path = reconstruct_path(store, &source_key, &sink_key, &predecessors)?;
                response.found = true;
                response.depth = Some(response.path.len() as u32);
                break 'search;
            }
            // Name-only call rows cannot identify which duplicate definition
            // owns outgoing edges. Inspect direct edges honestly, but never
            // continue through an ambiguous source or intermediate symbol.
            if may_continue && has_unique_definition(store, &call.callee)? {
                frontier.push_back((call.callee, depth + 1, true));
            }
        }
    }
    response.visited_nodes = seen.len();
    Ok(response)
}
