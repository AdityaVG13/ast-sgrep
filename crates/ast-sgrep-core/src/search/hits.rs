use std::collections::HashSet;

use super::types::{HitKind, SearchHit};

pub fn import_hit(
    path: String,
    language: Option<String>,
    module_path: String,
    line_no: u32,
) -> SearchHit {
    SearchHit {
        kind: HitKind::Import,
        file: path,
        line_start: line_no,
        line_end: line_no,
        symbol: Some(module_path.clone()),
        caller: None,
        callee: None,
        language,
        score: 2.0,
        excerpt: format!("import {module_path}"),
    }
}

pub fn push_caller_and_graph(
    hits: &mut Vec<SearchHit>,
    path: String,
    language: Option<String>,
    caller: String,
    callee: String,
    line_no: u32,
    excerpt: String,
    caller_score: f64,
    include_graph: bool,
) {
    hits.push(SearchHit {
        kind: HitKind::Caller,
        file: path.clone(),
        line_start: line_no,
        line_end: line_no,
        symbol: None,
        caller: Some(caller.clone()),
        callee: Some(callee.clone()),
        language: language.clone(),
        score: caller_score,
        excerpt,
    });
    if include_graph {
        let callee_label = callee.clone();
        hits.push(SearchHit {
            kind: HitKind::Graph,
            file: path,
            line_start: line_no,
            line_end: line_no,
            symbol: Some(callee_label.clone()),
            caller: Some(caller.clone()),
            callee: Some(callee_label),
            language,
            score: crate::rank::SCORE_GRAPH,
            excerpt: format!("{caller} calls {callee}"),
        });
    }
}

pub fn dedup_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for hit in hits {
        let key = (
            hit.kind.as_str(),
            hit.file.clone(),
            hit.line_start,
            hit.line_end,
            hit.symbol.clone(),
            hit.caller.clone(),
            hit.callee.clone(),
        );
        if seen.insert(key) {
            out.push(hit);
        }
    }
    out
}
