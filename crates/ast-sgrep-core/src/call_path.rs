//! Bounded interprocedural call paths.
//!
//! This is intentionally not value-flow analysis: it finds a directed path
//! through indexed caller/callee relations and reports how each hop resolved.

use crate::resolution::{Resolution, SymbolId};
use crate::search::NameIntern;
use crate::store::{CallEvidenceRow, IndexStore};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

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
    parent: u32,
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
    let same_file_candidates = crate::store::bundle_graph::file_tally(&by_file, &call.file);
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
    source: u32,
    sink: u32,
    predecessors: &[Option<Predecessor>],
) -> Result<Vec<CallPathHop>> {
    let mut current = sink;
    let mut calls = Vec::new();
    while current != source {
        let predecessor = predecessors[current as usize]
            .as_ref()
            .expect("visited call-path node has a predecessor");
        calls.push(predecessor.call.clone());
        current = predecessor.parent;
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

    let mut intern = NameIntern::new(config.max_nodes.min(256));
    let source_id = intern.intern(&source_key);
    let sink_id = intern.intern(&sink_key);
    let mut seen = vec![false; intern.len()];
    seen[source_id as usize] = true;
    let mut visited = 1usize;
    let mut predecessors: Vec<Option<Predecessor>> = vec![None; intern.len()];
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
        let parent_id = intern.intern(&normalized(&caller));
        for call in calls.into_iter().take(remaining_edges) {
            response.explored_edges += 1;
            let callee_key = normalized(&call.callee);
            let callee_id = intern.intern(&callee_key);
            let idx = callee_id as usize;
            if seen.len() <= idx {
                seen.resize(idx + 1, false);
                predecessors.resize(idx + 1, None);
            }
            if seen[idx] {
                continue;
            }
            if visited >= config.max_nodes {
                response.truncated = true;
                continue;
            }
            seen[idx] = true;
            visited += 1;
            predecessors[idx] = Some(Predecessor {
                parent: parent_id,
                call: call.clone(),
            });
            if callee_id == sink_id {
                response.path = reconstruct_path(store, source_id, sink_id, &predecessors)?;
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
    response.visited_nodes = visited;
    Ok(response)
}
