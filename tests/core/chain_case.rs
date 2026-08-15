use ast_sgrep_core::call_path::{find_call_path, CallPathConfig};
use ast_sgrep_core::chain::{expand_chain, ChainConfig, EdgeLabel};
use ast_sgrep_core::resolution::Resolution;
use ast_sgrep_core::scip::{ScipDocument, ScipIndex, ScipOccurrence};
use ast_sgrep_core::store::{CallerRow, SymbolRow, UpsertFileInput};
use ast_sgrep_core::IndexStore;
use tempfile::TempDir;

// Regression for bead ast-sgrep-z47q (F-03): symbols_named used WHERE s.name=?1
// (case-sensitive) while calls_matching uses lower()=lower(). In chain.rs
// expand_one, callee strings from outgoing_calls feed symbols_named, so a
// case mismatch between the call site (e.g. "Baz") and the definition
// (e.g. "baz") silently dropped chain nodes. Fix: symbols_named is now
// case-insensitive via lower(s.name)=lower(?1) backed by a functional index
// idx_symbols_name_lower (schema v6).
fn base<'a>(
    path: &'a str,
    lines: &'a [(u32, String)],
    hash: &'a str,
    symbols: &'a [SymbolRow],
    callers: &'a [CallerRow],
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("rust"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols,
        callers,
        imports: &[],
        pattern_nodes: &[],
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
    }
}

#[test]
fn symbols_named_is_case_insensitive() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    // Define a lowercase symbol "baz"; call it from FooBar with uppercase "Baz".
    let symbols = [SymbolRow {
        name: "baz".into(),
        kind: "function".into(),
        line_start: 5,
        line_end: 5,
        byte_start: 0,
        byte_end: 0,
    }];
    let callers = [CallerRow {
        caller: "FooBar".into(),
        callee: "Baz".into(), // case mismatch vs definition "baz"
        line_no: 2,
        byte_start: 0,
        byte_end: 0,
    }];
    let lines = [
        (1u32, "fn FooBar() { Baz(); }".into()),
        (2, "fn baz() {}".into()),
    ];
    store
        .upsert_file(base("case.rs", &lines, "h1", &symbols, &callers))
        .unwrap();

    // No regression: exact case still resolves.
    let exact = store.symbols_named("baz", 10).unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].name, "baz");

    // The fix: uppercase query finds lowercase symbol.
    let upper = store.symbols_named("BAZ", 10).unwrap();
    assert_eq!(
        upper.len(),
        1,
        "symbols_named must be case-insensitive (upper query)"
    );
    assert_eq!(upper[0].name, "baz");

    // Mixed case query also resolves.
    let mixed = store.symbols_named("Baz", 10).unwrap();
    assert_eq!(
        mixed.len(),
        1,
        "symbols_named must be case-insensitive (mixed-case query)"
    );

    // The chain scenario: outgoing_calls returns callee as-written in source
    // ("Baz"); symbols_named must resolve it to the "baz" definition.
    let outgoing = store.outgoing_calls("FooBar").unwrap();
    assert_eq!(outgoing.len(), 1);
    let (_, _, _, callee) = &outgoing[0];
    assert_eq!(callee, "Baz");
    let resolved = store.symbols_named(callee, 8).unwrap();
    assert_eq!(
        resolved.len(),
        1,
        "case-mismatched callee from outgoing_calls must resolve via symbols_named"
    );
    assert_eq!(resolved[0].name, "baz");
}

#[test]
fn case_mismatched_callee_expands_to_definition_node() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let caller_symbols = [SymbolRow {
        name: "FooBar".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 24,
    }];
    let callers = [CallerRow {
        caller: "FooBar".into(),
        callee: "Baz".into(),
        line_no: 1,
        byte_start: 14,
        byte_end: 17,
    }];
    let caller_lines = [(1, "fn FooBar() { Baz(); }".into())];
    store
        .upsert_file(base(
            "caller.rs",
            &caller_lines,
            "caller-hash",
            &caller_symbols,
            &callers,
        ))
        .unwrap();

    let callee_symbols = [SymbolRow {
        name: "baz".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 11,
    }];
    let callee_lines = [(1, "fn baz() {}".into())];
    store
        .upsert_file(base(
            "callee.rs",
            &callee_lines,
            "callee-hash",
            &callee_symbols,
            &[],
        ))
        .unwrap();

    let response = expand_chain(
        &store,
        "defs:foobar",
        &ChainConfig {
            max_depth: 1,
            top_n: 4,
            limit: 8,
            ..ChainConfig::default()
        },
    )
    .unwrap();
    assert!(response
        .seeds
        .iter()
        .any(|node| node.symbol.as_deref() == Some("FooBar")));
    assert!(response.nodes.iter().any(|node| {
        node.file == "callee.rs" && node.symbol.as_deref() == Some("baz") && node.depth == 1
    }));
    assert!(response.edges.iter().any(|edge| {
        edge.label == EdgeLabel::Calls
            && edge.from_symbol.as_deref() == Some("FooBar")
            && edge.to_symbol.as_deref() == Some("baz")
    }));
}

#[test]
fn bounded_call_path_reports_scip_evidence_without_claiming_value_flow() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let symbols = [
        SymbolRow {
            name: "source".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            byte_start: 0,
            byte_end: 23,
        },
        SymbolRow {
            name: "middle".into(),
            kind: "function".into(),
            line_start: 2,
            line_end: 2,
            byte_start: 24,
            byte_end: 45,
        },
        SymbolRow {
            name: "sink".into(),
            kind: "function".into(),
            line_start: 3,
            line_end: 3,
            byte_start: 46,
            byte_end: 58,
        },
    ];
    let callers = [
        CallerRow {
            caller: "source".into(),
            callee: "middle".into(),
            line_no: 1,
            byte_start: 14,
            byte_end: 20,
        },
        CallerRow {
            caller: "middle".into(),
            callee: "sink".into(),
            line_no: 2,
            byte_start: 38,
            byte_end: 42,
        },
        CallerRow {
            caller: "sink".into(),
            callee: "source".into(),
            line_no: 3,
            byte_start: 0,
            byte_end: 0,
        },
    ];
    let lines = [
        (1, "fn source() { middle(); }".into()),
        (2, "fn middle() { sink(); }".into()),
        (3, "fn sink() {}".into()),
    ];
    store
        .upsert_file(base("graph.rs", &lines, "graph-hash", &symbols, &callers))
        .unwrap();
    let applied = store
        .apply_scip(&ScipIndex {
            documents: vec![ScipDocument {
                relative_path: "graph.rs".into(),
                occurrences: vec![ScipOccurrence {
                    symbol: "rust+fixture+middle().".into(),
                    symbol_roles: 0,
                    range: vec![0, 14, 0, 20],
                }],
            }],
        })
        .unwrap();
    assert_eq!(applied.refs_upgraded, 1);

    let too_shallow = find_call_path(
        &store,
        "source",
        "sink",
        &CallPathConfig {
            max_depth: 1,
            max_nodes: 10,
            max_edges: 10,
        },
    )
    .unwrap();
    assert!(!too_shallow.found);
    assert!(!too_shallow.truncated);

    let response = find_call_path(
        &store,
        "SOURCE",
        "sink",
        &CallPathConfig {
            max_depth: 2,
            max_nodes: 10,
            max_edges: 10,
        },
    )
    .unwrap();
    assert!(response.found);
    assert_eq!(response.semantics, "call_graph_only");
    assert_eq!(response.depth, Some(2));
    assert_eq!(response.path.len(), 2);
    assert_eq!(response.path[0].resolution, Resolution::ScipExact);
    assert!(response.path.iter().all(|hop| hop.precise));

    let node_capped = find_call_path(
        &store,
        "source",
        "sink",
        &CallPathConfig {
            max_depth: 2,
            max_nodes: 2,
            max_edges: 10,
        },
    )
    .unwrap();
    assert!(!node_capped.found);
    assert!(node_capped.truncated);
}
