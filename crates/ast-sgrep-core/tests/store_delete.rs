use ast_sgrep_core::store::{CallerRow, ImportRow, SymbolRow, UpsertFileInput};
use ast_sgrep_core::IndexStore;
use ast_sgrep_lang::PatternNode;
use tempfile::TempDir;
fn base<'a>(path: &'a str, lines: &'a [(u32, String)], hash: &'a str) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("python"),
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
fn count(store: &IndexStore, sql: &str) -> i64 {
    store.connection().query_row(sql, [], |r| r.get(0)).unwrap()
}
fn count_match(store: &IndexStore, table: &str, q: &str) -> i64 {
    store
        .connection()
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE {table} MATCH ?1"),
            [q],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
fn semantic_mutation_removes_ivf_before_it_can_be_reloaded() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let sidecar = ast_sgrep_core::semantic_ivf::semantic_ivf_path(store.db_path());
    std::fs::write(&sidecar, b"stale sidecar").unwrap();
    let lines = [(1, "semantic content".into())];
    let chunks = [ast_sgrep_core::semantic_chunk::SemanticChunkInput {
        symbol_name: "example".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        excerpt: "semantic content".into(),
        callers: vec![],
        callees: vec![],
        doc: String::new(),
        scope: String::new(),
    }];
    let mut input = base("semantic.py", &lines, "hash");
    input.semantic_chunks = &chunks;
    input.embed_semantic = true;
    store.upsert_file(input).unwrap();
    assert!(
        !sidecar.exists(),
        "semantic mutation must invalidate the on-disk IVF before commit"
    );
}
#[test]
fn re_upsert_does_not_leave_stale_fts_rows() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let path = "stale_test.py";
    let first = [(1, "alpha beta gamma".into()), (2, "delta epsilon".into())];
    store.upsert_file(base(path, &first, "hash1")).unwrap();
    assert_eq!(count_match(&store, "lines_fts", "alpha"), 1);
    assert_eq!(count_match(&store, "lines_trigram", "alp"), 1);
    let second = [(1, "zeta eta theta".into()), (2, "iota kappa".into())];
    store.upsert_file(base(path, &second, "hash2")).unwrap();
    assert_eq!(count_match(&store, "lines_fts", "alpha"), 0);
    assert_eq!(count_match(&store, "lines_trigram", "alp"), 0);
    assert_eq!(count_match(&store, "lines_fts", "zeta"), 1);
    assert_eq!(count_match(&store, "lines_trigram", "zet"), 1);
}
#[test]
fn remove_file_clears_all_per_file_tables() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let path = "delete_all.py";
    let symbols = [SymbolRow {
        name: "foo".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 2,
        byte_start: 0,
        byte_end: 10,
    }];
    let callers = [CallerRow {
        caller: "foo".into(),
        callee: "bar".into(),
        line_no: 1,
        byte_start: 0,
        byte_end: 3,
    }];
    let imports = [ImportRow {
        module_path: "os".into(),
        line_no: 1,
    }];
    let pattern_nodes = [PatternNode {
        signature: "sig".into(),
        line_start: 1,
        line_end: 1,
        excerpt: "ex".into(),
    }];
    let lines = [(1, "import os".into()), (2, "foo(bar)".into())];
    let mut input = base(path, &lines, "hash");
    input.symbols = &symbols;
    input.callers = &callers;
    input.imports = &imports;
    input.pattern_nodes = &pattern_nodes;
    let file_id = store.upsert_file(input).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO embeddings (file_id, line_no, vector) VALUES (?1, ?2, ?3)",
            rusqlite::params![file_id, 1u32, vec![0u8; 8]],
        )
        .unwrap();
    store.connection().execute(
        "INSERT INTO semantic_chunks (file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector) VALUES (?1, NULL, 'file', 1, 2, '', 'text', ?2)", rusqlite::params![file_id, vec![0u8; 8]],
    ).unwrap();
    store.remove_file(path).unwrap();
    for table in [
        "lines",
        "lines_fts",
        "lines_trigram",
        "symbols",
        "callers",
        "imports",
        "pattern_nodes",
        "embeddings",
        "semantic_chunks",
    ] {
        assert_eq!(
            count(&store, &format!("SELECT COUNT(*) FROM {table}")),
            0,
            "{table} should be empty"
        );
    }
}
#[test]
fn re_upsert_preserves_other_files_fts_rows() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let a = [(1, "first unique needle".into())];
    let b = [(1, "second unique haystack".into())];
    store.upsert_file(base("first.py", &a, "hash1")).unwrap();
    store.upsert_file(base("second.py", &b, "hash2")).unwrap();
    let rep = [(1, "replacement content".into())];
    store.upsert_file(base("first.py", &rep, "hash3")).unwrap();
    assert_eq!(count_match(&store, "lines_fts", "second"), 1);
    assert_eq!(count_match(&store, "lines_trigram", "sec"), 1);
}
#[test]
fn re_upsert_many_files_is_linear() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let n = 2000usize;
    let paths: Vec<String> = (0..n).map(|i| format!("file{i:04}.py")).collect();
    let lines = [(1, "hello world".into()), (2, "foo bar baz".into())];
    let lines2 = [(1, "goodbye world".into()), (2, "qux corge grault".into())];
    let run = |lines: &[(u32, String)], prefix: &str, offset: usize| {
        store.begin_bulk_tx().unwrap();
        let t0 = std::time::Instant::now();
        for (i, path) in paths.iter().enumerate() {
            let hash = format!("{prefix}{i}");
            let mut input = base(path, lines, &hash);
            input.mtime_secs = (i + offset) as i64;
            store.upsert_file(input).unwrap();
        }
        store.commit_bulk_tx().unwrap();
        t0.elapsed()
    };
    let insert = run(&lines, "hash", 0);
    let re = run(&lines2, "hash2_", n);
    assert!(
        re < std::time::Duration::from_secs(15),
        "re-upsert of {n} took {re:?}"
    );
    assert!(
        insert + re < std::time::Duration::from_secs(30),
        "total took {:?}",
        insert + re
    );
}
/// Body-hash / structure-stable append must keep lines_trigram searchable so the
/// literal BMH path (≥1000 lines) still finds newly appended trailing tokens.
#[test]
fn structure_stable_append_keeps_trigram_and_search_literal() {
    use ast_sgrep_core::{SearchOptions, Searcher};
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let path = "big_pad.py";
    // ≥1000 lines forces literal_pass onto the trigram path (BMH_LINE_THRESHOLD).
    let mut lines: Vec<(u32, String)> = (1u32..=1000)
        .map(|i| (i, format!("pad content line number {i} filler")))
        .collect();
    store
        .upsert_file(base(path, &lines, "hash_pad_v1"))
        .unwrap();
    assert!(
        store.indexed_line_count().unwrap() >= 1000,
        "fixture must reach BMH threshold"
    );
    lines.push((
        1001,
        "// UNIQUE_TRAILING_TOKEN_xyzzy_body_hash_append".into(),
    ));
    // Empty graph structure matches first upsert → refresh_lines_only append path.
    store
        .upsert_file(base(path, &lines, "hash_pad_v2"))
        .unwrap();
    assert_eq!(
        count_match(&store, "lines_trigram", "xyzzy"),
        1,
        "append must insert lines_trigram rows for new trailing content"
    );
    assert_eq!(
        count_match(
            &store,
            "lines_fts",
            "UNIQUE_TRAILING_TOKEN_xyzzy_body_hash_append"
        ),
        1,
        "append must insert lines_fts rows"
    );
    let searcher = Searcher::with_store(
        store,
        SearchOptions {
            root: temp.path().to_path_buf(),
            limit: 16,
            use_embed: false,
            ..SearchOptions::default()
        },
    );
    let resp = searcher
        .search_literal("UNIQUE_TRAILING_TOKEN_xyzzy_body_hash_append")
        .expect("search_literal");
    assert!(
        resp.hits.iter().any(|h| h.excerpt.contains("xyzzy")),
        "search_literal must hit appended trailing token via trigram path; hits={:?}",
        resp.hits
            .iter()
            .map(|h| (h.file.as_str(), h.line_start, h.excerpt.as_str()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn structure_stable_truncate_drops_trigram_rows() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let path = "trim.py";
    let long = [
        (1, "keep alpha".into()),
        (2, "drop UNIQUE_TRIM_TOKEN_qqq".into()),
    ];
    store.upsert_file(base(path, &long, "h1")).unwrap();
    assert_eq!(count_match(&store, "lines_trigram", "qqq"), 1);
    let short = [(1, "keep alpha".into())];
    store.upsert_file(base(path, &short, "h2")).unwrap();
    assert_eq!(
        count_match(&store, "lines_trigram", "qqq"),
        0,
        "truncate must delete lines_trigram rowids for dropped lines"
    );
    assert_eq!(count_match(&store, "lines_fts", "UNIQUE_TRIM_TOKEN_qqq"), 0);
}

#[test]
fn same_span_body_edit_refreshes_semantic_chunks() {
    use ast_sgrep_core::semantic_chunk::build_semantic_chunks;
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let path = "body_edit.py";
    let symbols = [SymbolRow {
        name: "compute".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 3,
        byte_start: 0,
        byte_end: 40,
    }];
    let callers: [CallerRow; 0] = [];
    let imports: [ImportRow; 0] = [];
    let lines_v1 = [
        (1, "def compute():".into()),
        (2, "    return ALPHA_TOKEN_111".into()),
        (3, "".into()),
    ];
    let lines_v2 = [
        (1, "def compute():".into()),
        (2, "    return BETA_TOKEN_222".into()),
        (3, "".into()),
    ];
    let chunks_v1 = build_semantic_chunks(&symbols, &callers, &lines_v1);
    let chunks_v2 = build_semantic_chunks(&symbols, &callers, &lines_v2);
    assert!(!chunks_v1.is_empty() && !chunks_v2.is_empty());
    assert_ne!(chunks_v1[0].excerpt, chunks_v2[0].excerpt);
    let pat_v1 = [PatternNode {
        signature: "fn compute".into(),
        line_start: 1,
        line_end: 3,
        excerpt: "return ALPHA_TOKEN_111".into(),
    }];
    let pat_v2 = [PatternNode {
        signature: "fn compute".into(),
        line_start: 1,
        line_end: 3,
        excerpt: "return BETA_TOKEN_222".into(),
    }];
    let upsert = |lines: &[(u32, String)],
                  chunks: &[ast_sgrep_core::semantic_chunk::SemanticChunkInput],
                  pats: &[PatternNode],
                  hash: &str| {
        let mut input = base(path, lines, hash);
        input.symbols = &symbols;
        input.callers = &callers;
        input.imports = &imports;
        input.pattern_nodes = pats;
        input.semantic_chunks = chunks;
        input.embed_semantic = true;
        input.embed_backend = ast_sgrep_embed::EmbedPreference::Semantic;
        store.upsert_file(input).unwrap();
    };
    upsert(&lines_v1, &chunks_v1, &pat_v1, "hash_alpha");
    let rows_v1 = store.all_semantic_chunks(None).unwrap();
    assert_eq!(rows_v1.len(), 1);
    let (text_v1, vec_v1) = (rows_v1[0].4.clone(), rows_v1[0].5.clone());
    assert!(text_v1.contains("ALPHA_TOKEN_111"));
    upsert(&lines_v2, &chunks_v2, &pat_v2, "hash_beta");
    let rows_v2 = store.all_semantic_chunks(None).unwrap();
    assert_eq!(rows_v2.len(), 1);
    assert!(rows_v2[0].4.contains("BETA_TOKEN_222"));
    assert!(!rows_v2[0].4.contains("ALPHA_TOKEN_111"));
    assert_ne!(text_v1, rows_v2[0].4);
    assert_ne!(vec_v1, rows_v2[0].5);
    let excerpt: String = store
        .connection()
        .query_row("SELECT excerpt FROM pattern_nodes LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        excerpt.contains("BETA_TOKEN_222"),
        "pattern excerpt must refresh: {excerpt}"
    );
}
