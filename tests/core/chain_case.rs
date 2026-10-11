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
        depth_truncated: false,
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
fn caller_chain_seeds_use_unique_definition_locations_and_dedup_resolved_files() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    for (path, caller) in [("a.rs", "alpha"), ("c.rs", "gamma")] {
        let symbols = [SymbolRow {
            name: caller.into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 4,
            byte_start: 0,
            byte_end: 0,
        }];
        let calls = [
            CallerRow {
                caller: caller.into(),
                callee: "BETA".into(),
                line_no: 2,
                byte_start: 0,
                byte_end: 0,
            },
            CallerRow {
                caller: caller.into(),
                callee: "external".into(),
                line_no: 3,
                byte_start: 0,
                byte_end: 0,
            },
        ];
        let lines = [
            (1, format!("fn {caller}() {{")),
            (2, "    BETA();".into()),
            (3, "    external();".into()),
            (4, "}".into()),
        ];
        store
            .upsert_file(base(path, &lines, path, &symbols, &calls))
            .unwrap();
    }
    let definition = [SymbolRow {
        name: "beta".into(),
        kind: "function".into(),
        line_start: 4,
        line_end: 6,
        byte_start: 0,
        byte_end: 0,
    }];
    let lines = [
        (4, "fn beta() {".into()),
        (5, "    let exact_definition = 1;".into()),
        (6, "}".into()),
    ];
    store
        .upsert_file(base("b.rs", &lines, "beta-definition", &definition, &[]))
        .unwrap();
    let config = ChainConfig {
        max_depth: 0,
        limit: 8,
        top_n: 8,
        ..ChainConfig::default()
    };
    let seeded = expand_chain(&store, "callers:BETA", &config).unwrap();
    assert_eq!(
        seeded.seeds.len(),
        1,
        "two reference files must resolve to one seed file"
    );
    let seed = &seeded.seeds[0];
    assert_eq!(
        (
            &*seed.file,
            seed.symbol.as_deref(),
            seed.line_start,
            seed.line_end,
            seed.language.as_deref()
        ),
        ("b.rs", Some("beta"), 4, 6, Some("rust")),
        "callee identity must keep its defining location"
    );
    assert_eq!(seed.depth, 0);
    assert_eq!(seeded.node_count, 1);

    let expanded = expand_chain(
        &store,
        "callers:BETA",
        &ChainConfig {
            max_depth: 1,
            ..config.clone()
        },
    )
    .unwrap();
    for (file, symbol) in [("a.rs", "alpha"), ("c.rs", "gamma")] {
        assert!(
            expanded.nodes.iter().any(|node| node.file == file
                && node.symbol.as_deref() == Some(symbol)
                && node.depth == 1),
            "correcting the callee seed must retain the actual caller {symbol}"
        );
        assert!(expanded
            .edges
            .iter()
            .any(|edge| edge.label == EdgeLabel::CalledBy
                && edge.from_file == file
                && edge.from_symbol.as_deref() == Some(symbol)
                && edge.to_file == "b.rs"
                && edge.to_symbol.as_deref() == Some("beta")));
    }

    // With no unique definition, retain the existing reference-site evidence;
    // never choose a path-first duplicate as if it established symbol identity.
    let external = expand_chain(&store, "callers:external", &config).unwrap();
    assert_eq!(external.seeds.len(), 2);
    assert!(external
        .seeds
        .iter()
        .all(|seed| ["a.rs", "c.rs"].contains(&seed.file.as_str())
            && seed.symbol.as_deref() == Some("external")
            && seed.line_start == 3
            && seed.line_end == 3));
    store
        .upsert_file(base(
            "duplicate.rs",
            &lines,
            "duplicate-beta",
            &definition,
            &[],
        ))
        .unwrap();
    let ambiguous = expand_chain(&store, "callers:BETA", &config).unwrap();
    assert_eq!(ambiguous.seeds.len(), 2);
    assert!(ambiguous
        .seeds
        .iter()
        .all(|seed| ["a.rs", "c.rs"].contains(&seed.file.as_str())
            && seed.symbol.as_deref() == Some("BETA")
            && seed.line_start == 2
            && seed.line_end == 2));
}

#[test]
fn chain_work_limit_counts_duplicate_case_variant_rows_before_deduplication() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let mut lines = vec![(1, "fn hub() {".into())];
    let calls: Vec<_> = (2..=9001)
        .map(|line_no| {
            let callee = if line_no % 2 == 0 { "LEAF" } else { "leaf" };
            lines.push((line_no, format!("    {callee}();")));
            CallerRow {
                caller: "hub".into(),
                callee: callee.into(),
                line_no,
                byte_start: 0,
                byte_end: 0,
            }
        })
        .collect();
    lines.push((9002, "}".into()));
    let symbol = |name: &str, end| SymbolRow {
        name: name.into(),
        kind: "function".into(),
        line_start: 1,
        line_end: end,
        byte_start: 0,
        byte_end: 0,
    };
    store
        .upsert_file(base(
            "hub.rs",
            &lines,
            "hub",
            &[symbol("hub", 9002)],
            &calls,
        ))
        .unwrap();
    store
        .upsert_file(base(
            "leaf.rs",
            &[(1, "fn leaf() {}".into())],
            "leaf",
            &[symbol("leaf", 1)],
            &[],
        ))
        .unwrap();
    let config = ChainConfig {
        top_n: 1,
        max_depth: 2,
        ..ChainConfig::default()
    };
    let first = serde_json::to_value(expand_chain(&store, "defs:hub", &config).unwrap()).unwrap();
    assert_eq!(
        first["truncated"], true,
        "duplicate-heavy expansion must stop work, not count two deduplicated files as cheap"
    );
    assert_eq!(
        first["truncation_reasons"],
        serde_json::json!(["work_limit"])
    );
    assert_eq!(first["work"]["limit"], 8192);
    assert_eq!(first["work"]["used"], 8192);
    assert!(first["work"]["call_rows"].as_u64().unwrap() > 1);
    assert!(
        first["work"]["peak_source_page_rows"].as_u64().unwrap() <= 8193,
        "SQL admission must precede materialization"
    );
    assert_eq!(first["node_count"], 2);
    assert_eq!(first["seeds"][0]["symbol"], "hub");
    assert_eq!(
        first["edges"].as_array().unwrap().len(),
        1,
        "later reverse-edge exploration was not completed"
    );
    for _ in 0..3 {
        assert_eq!(
            serde_json::to_value(expand_chain(&store, "defs:hub", &config).unwrap()).unwrap(),
            first
        );
    }
}

#[test]
fn chain_reports_the_existing_definition_limit_without_changing_its_prefix() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let symbol = |name: &str| SymbolRow {
        name: name.into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 0,
    };
    let calls = [CallerRow {
        caller: "hub".into(),
        callee: "LEAF".into(),
        line_no: 1,
        byte_start: 0,
        byte_end: 0,
    }];
    store
        .upsert_file(base(
            "hub.rs",
            &[(1, "fn hub() { LEAF(); }".into())],
            "hub",
            &[symbol("hub")],
            &calls,
        ))
        .unwrap();
    for i in (0..9).rev() {
        let path = format!("leaf-{i}.rs");
        store
            .upsert_file(base(
                &path,
                &[(1, "fn leaf() {}".into())],
                &path,
                &[symbol("leaf")],
                &[],
            ))
            .unwrap();
    }
    let response = expand_chain(
        &store,
        "defs:hub",
        &ChainConfig {
            top_n: 1,
            max_depth: 1,
            limit: 20,
            ..ChainConfig::default()
        },
    )
    .unwrap();
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(
        value["truncated"], true,
        "ninth definitions must not disappear without a signal"
    );
    assert_eq!(
        value["truncation_reasons"],
        serde_json::json!(["definition_limit"])
    );
    assert_eq!(response.node_count, 9);
    for i in 0..8 {
        assert!(response
            .nodes
            .iter()
            .any(|n| n.file == format!("leaf-{i}.rs")));
    }
    assert!(!response.nodes.iter().any(|n| n.file == "leaf-8.rs"));
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
    assert_eq!(response.path[0].resolution, Resolution::ScipOccurrence);
    assert!(!response.path[0].precise);

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

#[test]
fn call_path_does_not_splice_duplicate_callee_definitions() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();

    for (path, caller, callee) in [
        ("source.rs", "source", "duplicate"),
        ("first.rs", "duplicate", "sink"),
    ] {
        let symbols = [SymbolRow {
            name: caller.into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            byte_start: 0,
            byte_end: 32,
        }];
        let calls = [CallerRow {
            caller: caller.into(),
            callee: callee.into(),
            line_no: 1,
            byte_start: 14,
            byte_end: 20,
        }];
        let lines = [(1, format!("fn {caller}() {{ {callee}(); }}"))];
        store
            .upsert_file(base(path, &lines, path, &symbols, &calls))
            .unwrap();
    }
    let duplicate = [SymbolRow {
        name: "duplicate".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 17,
    }];
    let lines = [(1, "fn duplicate() {}".into())];
    store
        .upsert_file(base("second.rs", &lines, "second", &duplicate, &[]))
        .unwrap();

    let response = find_call_path(&store, "source", "sink", &CallPathConfig::default()).unwrap();
    assert!(
        !response.found,
        "duplicate definitions must not splice edges"
    );
    assert_eq!(response.explored_edges, 1);
}
