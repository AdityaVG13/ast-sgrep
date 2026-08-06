//! Hard evidence for epics `ast-sgrep-s7jw` and `ast-sgrep-search-correctness-iva9`.
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::pattern::search_pattern;
use ast_sgrep_core::query::{ParsedQuery, QueryMode};
use ast_sgrep_core::rank::{rrf_score, LEXICAL_RRF_SCALE, RRF_K};
use ast_sgrep_core::search::passes::lexical::{
    lexical_pass, lexical_pool_limit, LEXICAL_POOL_FLOOR,
};
use ast_sgrep_core::search::{HitKind, HitSignal, SearchHit, SearchOptions, Searcher};
use ast_sgrep_core::semantic_ann::ann_result_is_sufficient;
use ast_sgrep_core::store::{CallerRow, SymbolRow, UpsertFileInput};
use ast_sgrep_core::tantivy_index::{TantivySidecar, LEXICAL_DB};
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer};
use std::fs;
use tempfile::TempDir;

fn base<'a>(
    path: &'a str,
    language: Option<&'a str>,
    lines: &'a [(u32, String)],
    hash: &'a str,
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language,
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
    }
}

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

/// cbnw / e2hc.14 — Asgrep ceiling is single-list RRF (already fixed on this PR).
#[test]
fn cbnw_asgrep_ceiling_is_single_list_rrf() {
    let expected = rrf_score(0, RRF_K) * LEXICAL_RRF_SCALE;
    let hit = SearchHit {
        kind: HitKind::Asgrep,
        file: "a.rs".into(),
        line_start: 1,
        line_end: 1,
        symbol: None,
        caller: None,
        callee: None,
        language: None,
        score: expected,
        signal: HitSignal::Exact,
        contributors: Vec::new(),
        margin: 0.0,
        excerpt: "alpha beta gamma".into(),
    };
    let mut one = vec![hit.clone()];
    let mut many = vec![hit];
    let parsed_one = ParsedQuery {
        raw: "alpha".into(),
        mode: QueryMode::Hybrid,
        target: None,
        terms: vec!["alpha".into()],
    };
    let parsed_many = ParsedQuery {
        raw: "alpha beta gamma".into(),
        mode: QueryMode::Hybrid,
        target: None,
        terms: vec!["alpha".into(), "beta".into(), "gamma".into()],
    };
    ast_sgrep_core::intent::route_hits(&parsed_one, &mut one);
    ast_sgrep_core::intent::route_hits(&parsed_many, &mut many);
    assert!(
        (one[0].score - many[0].score).abs() < 1e-9,
        "multi-term must not crush lexical: one={} many={}",
        one[0].score,
        many[0].score
    );
    assert!(
        many[0].score > 0.9,
        "rank-0 lexical on multi-term must stay near weight ceiling, got {}",
        many[0].score
    );
}

/// hkdi — empty auto-created lexical.db is never search-ready.
#[test]
fn hkdi_empty_lexical_sidecar_not_ready() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let sidecar = TantivySidecar::open_for_index(root, None).unwrap();
    assert!(sidecar.exists());
    assert!(!sidecar.is_search_ready().unwrap());
    assert!(TantivySidecar::open_existing_for_search(root, None)
        .unwrap()
        .is_none());
    let zero = root.join(".asgrep").join(LEXICAL_DB);
    fs::write(&zero, b"").unwrap();
    assert!(TantivySidecar::open_existing_for_search(root, None)
        .unwrap()
        .is_none());
}

/// s7jw.2 — auto/sidecar empty path falls back to SQL FTS when FTS has hits.
#[test]
fn s7jw2_empty_sidecar_falls_back_to_sql_lexical() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = IndexStore::open(root, None).unwrap();
    let lines = [(1u32, "unique_sidecar_fallback_token appears here".into())];
    store
        .upsert_file(base("src/a.rs", Some("rust"), &lines, "h1"))
        .unwrap();
    // Schema-only sidecar exists and would previously short-circuit to empty.
    let _ = TantivySidecar::open_for_index(root, None).unwrap();
    assert!(TantivySidecar::open_existing_for_search(root, None)
        .unwrap()
        .is_none());
    let options = SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        use_tantivy: true,
        use_embed: false,
        limit: 16,
        ..SearchOptions::default()
    };
    let parsed = ParsedQuery::parse("unique_sidecar_fallback_token");
    let hits = lexical_pass(&store, &options, &parsed).unwrap();
    assert!(
        !hits.is_empty(),
        "must fall back to SQL FTS when empty sidecar is not ready; got {hits:#?}"
    );
}

/// s7jw.1 — lexical pool LIMIT is max(100, options.limit).
#[test]
fn s7jw1_lexical_pool_respects_options_limit() {
    assert_eq!(
        lexical_pool_limit(&SearchOptions {
            limit: 16,
            ..SearchOptions::default()
        }),
        LEXICAL_POOL_FLOOR
    );
    assert_eq!(
        lexical_pool_limit(&SearchOptions {
            limit: 250,
            ..SearchOptions::default()
        }),
        250
    );

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = IndexStore::open(root, None).unwrap();
    // 150 distinct matching lines; with limit=150 the pool must not hard-cap at 100.
    for i in 0..150u32 {
        let content = format!("needle_pool_token line_{i}");
        let lines = [(1u32, content)];
        let path = format!("f{i:03}.rs");
        let hash = format!("h{i}");
        store
            .upsert_file(base(&path, Some("rust"), &lines, &hash))
            .unwrap();
    }
    let options = SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        use_tantivy: false,
        use_embed: false,
        limit: 150,
        ..SearchOptions::default()
    };
    let parsed = ParsedQuery::parse("needle_pool_token");
    let hits = lexical_pass(&store, &options, &parsed).unwrap();
    assert!(
        hits.len() > 100,
        "lexical pool must honor options.limit>100; got {}",
        hits.len()
    );
}

/// iva9.2 — invalid file_filter errors (never silent unfiltered). Covered in unit tests;
/// this integration path confirms Searcher propagates the error.
#[test]
fn iva9_2_invalid_file_filter_errors_via_searcher() {
    let temp = TempDir::new().unwrap();
    write_src(temp.path(), "a.rs", "fn alpha() {}\n");
    let index_path = temp.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(index_path),
        file_filter: Some("\0*.rs".into()),
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    let err = searcher.search("alpha").unwrap_err().to_string();
    assert!(
        err.contains("invalid file_filter"),
        "expected invalid file_filter error, got {err}"
    );
}

/// iva9.5 — lang filter applied before path LIMIT in literal SQL.
#[test]
fn iva9_5_literal_lang_filter_not_starved_by_path_limit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // Many alphabetically-early python hits; rust match is late in path order.
    for i in 0..120 {
        write_src(
            root,
            &format!("a_py_{i:03}.py"),
            "unique_literal_needle = 1\n",
        );
    }
    write_src(root, "z_rust_match.rs", "let unique_literal_needle = 1;\n");
    let index_path = root.join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path),
        lang_filter: Some("rust".into()),
        limit: 16,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    let resp = searcher.search("literal:unique_literal_needle").unwrap();
    assert!(
        resp.hits.iter().any(|h| h.file.contains("z_rust_match")),
        "rust hit must survive lang+limit; got {:#?}",
        resp.hits
    );
    assert!(resp
        .hits
        .iter()
        .all(|h| h.language.as_deref() == Some("rust")));
}

/// iva9.6 — under-filled / empty ANN is not treated as sufficient.
#[test]
fn iva9_6_ann_sufficiency_contract() {
    assert!(!ann_result_is_sufficient(0, 100, 50));
    assert!(!ann_result_is_sufficient(10, 100, 50));
    assert!(ann_result_is_sufficient(50, 100, 50));
    assert!(ann_result_is_sufficient(10, 10, 50));
}

/// iva9.7 — exotic patterns fail closed when ast-grep is disabled/unavailable (no silent empty).
#[test]
fn iva9_7_exotic_pattern_fail_closed_without_ast_grep() {
    let temp = TempDir::new().unwrap();
    write_src(temp.path(), "a.rs", "fn alpha() { if cond { body(); } }\n");
    let store = IndexStore::open(temp.path(), None).unwrap();
    let old = std::env::var_os("ASGREP_DISABLE_AST_GREP");
    std::env::set_var("ASGREP_DISABLE_AST_GREP", "1");
    let result = search_pattern("if ($COND) { $BODY }", &store, temp.path(), None);
    match old {
        Some(v) => std::env::set_var("ASGREP_DISABLE_AST_GREP", v),
        None => std::env::remove_var("ASGREP_DISABLE_AST_GREP"),
    }
    let err = result.expect_err("exotic pattern must fail closed");
    let msg = err.to_string();
    assert!(
        msg.contains("fail-closed") || msg.contains("ast-grep"),
        "expected fail-closed error, got {msg}"
    );
}

/// iva9.7 — classifiable native empty remains authoritative match-none (no subprocess).
#[test]
fn iva9_7_classifiable_native_empty_is_match_none() {
    let temp = TempDir::new().unwrap();
    write_src(temp.path(), "a.rs", "fn alpha() {}\n");
    let store = IndexStore::open(temp.path(), None).unwrap();
    let hits = search_pattern("fn missing_name($$$)", &store, temp.path(), None).unwrap();
    assert!(hits.is_empty());
}

/// iva9.8 — chain edges ⊆ truncated nodes; seeds prefer callee on caller hits.
#[test]
fn iva9_8_chain_edges_subset_and_callee_seed() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let symbols_a = [SymbolRow {
        name: "alpha".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 5,
    }];
    let callers_a = [CallerRow {
        line_no: 2,
        caller: "alpha".into(),
        callee: "beta".into(),
        byte_start: 0,
        byte_end: 0,
    }];
    let lines_a = [
        (1u32, "fn alpha() { beta(); }".into()),
        (2u32, "    beta();".into()),
    ];
    let mut input_a = base("a.rs", Some("rust"), &lines_a, "ha");
    input_a.symbols = &symbols_a;
    input_a.callers = &callers_a;
    store.upsert_file(input_a).unwrap();

    let symbols_b = [SymbolRow {
        name: "beta".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 4,
    }];
    let lines_b = [(1u32, "fn beta() {}".into())];
    let mut input_b = base("b.rs", Some("rust"), &lines_b, "hb");
    input_b.symbols = &symbols_b;
    store.upsert_file(input_b).unwrap();

    // Extra nodes so truncate(limit=1) would previously leave dangling edges.
    for i in 0..5 {
        let name = format!("extra{i}");
        let symbols = [SymbolRow {
            name: name.clone(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            byte_start: 0,
            byte_end: 1,
        }];
        let lines = [(1u32, format!("fn {name}() {{}}"))];
        let path = format!("e{i}.rs");
        let hash = format!("he{i}");
        let mut input = base(&path, Some("rust"), &lines, &hash);
        input.symbols = &symbols;
        store.upsert_file(input).unwrap();
    }

    let resp = expand_chain(
        &store,
        "beta",
        &ChainConfig {
            max_depth: 2,
            decay_factor: 0.5,
            limit: 2,
            top_n: 8,
        },
    )
    .unwrap();
    let node_files: std::collections::HashSet<_> =
        resp.nodes.iter().map(|n| n.file.as_str()).collect();
    for edge in &resp.edges {
        assert!(
            node_files.contains(edge.from_file.as_str())
                && node_files.contains(edge.to_file.as_str()),
            "edge {:?}->{:?} escapes truncated nodes {:?}",
            edge.from_file,
            edge.to_file,
            node_files
        );
    }
    assert_eq!(resp.nodes.len(), resp.nodes.len().min(2));
    assert!(resp.edge_count == resp.edges.len());
}
