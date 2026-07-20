use ast_sgrep_core::IndexStore;
use ast_sgrep_core::store::{CallerRow, ImportRow, SymbolRow, UpsertFileInput};
use ast_sgrep_lang::PatternNode;
use tempfile::TempDir;

fn base_input<'a>(rel_path: &'a str, lines: &'a [(u32, String)], content_hash: &'a str) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path,
        language: Some("python"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash,
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

#[test]
fn semantic_mutation_removes_ivf_before_it_can_be_reloaded() {
    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");
    let sidecar = ast_sgrep_core::semantic_ivf::semantic_ivf_path(store.db_path());
    std::fs::write(&sidecar, b"stale sidecar").expect("seed stale sidecar");

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
    let mut input = base_input("semantic.py", &lines, "hash");
    input.semantic_chunks = &chunks;
    input.embed_semantic = true;

    store.upsert_file(input).expect("semantic upsert");

    assert!(
        !sidecar.exists(),
        "semantic mutation must invalidate the on-disk IVF before commit"
    );
}

#[test]
fn re_upsert_does_not_leave_stale_fts_rows() {
    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");

    let path = "stale_test.py";
    let first_lines = [(1, "alpha beta gamma".into()), (2, "delta epsilon".into())];
    let first = base_input(
        path,
        &first_lines,
        "hash1",
    );
    store.upsert_file(first).expect("first upsert");

    let fts_count: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_fts WHERE lines_fts MATCH 'alpha'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_count, 1, "lines_fts should find new content");
    let tri_count: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_trigram WHERE content MATCH 'alp'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tri_count, 1, "lines_trigram should find new content");

    let second_lines = [(1, "zeta eta theta".into()), (2, "iota kappa".into())];
    let second = base_input(
        path,
        &second_lines,
        "hash2",
    );
    store.upsert_file(second).expect("second upsert");

    let fts_stale: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_fts WHERE lines_fts MATCH 'alpha'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_stale, 0, "stale lines_fts rows must be deleted");
    let tri_stale: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_trigram WHERE content MATCH 'alp'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tri_stale, 0, "stale lines_trigram rows must be deleted");

    let fts_new: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_fts WHERE lines_fts MATCH 'zeta'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_new, 1, "lines_fts should find updated content");
    let tri_new: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_trigram WHERE content MATCH 'zet'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tri_new, 1, "lines_trigram should find updated content");
}

#[test]
fn remove_file_clears_all_per_file_tables() {
    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");

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
    let input = UpsertFileInput {
        rel_path: path,
        language: Some("python"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: "hash",
        lines: &lines,
        eol: "\n",
        symbols: &symbols,
        callers: &callers,
        imports: &imports,
        pattern_nodes: &pattern_nodes,
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
    };
    let file_id = store.upsert_file(input).expect("upsert");

    // embeddings and semantic_chunks are not populated by the API without a real embed backend,
    // so insert them directly to verify the delete path covers them.
    store.connection()
        .execute(
            "INSERT INTO embeddings (file_id, line_no, vector) VALUES (?1, ?2, ?3)",
            rusqlite::params![file_id, 1u32, vec![0u8; 8]],
        )
        .expect("insert embeddings");
    store.connection()
        .execute(
            "INSERT INTO semantic_chunks (file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector)
             VALUES (?1, NULL, 'file', 1, 2, '', 'text', ?2)",
            rusqlite::params![file_id, vec![0u8; 8]],
        )
        .expect("insert semantic_chunks");

    store.remove_file(path).expect("remove_file");

    let tables = [
        "lines", "lines_fts", "lines_trigram", "symbols", "callers", "imports",
        "pattern_nodes", "embeddings", "semantic_chunks",
    ];
    for table in tables {
        let count: i64 = store
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table} should be empty after remove_file");
    }
}

#[test]
fn re_upsert_preserves_other_files_fts_rows() {
    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");
    let first_lines = [(1, "first unique needle".into())];
    let second_lines = [(1, "second unique haystack".into())];

    store.upsert_file(base_input("first.py", &first_lines, "hash1")).expect("first upsert");
    store.upsert_file(base_input("second.py", &second_lines, "hash2")).expect("second upsert");

    let replacement = [(1, "replacement content".into())];
    store.upsert_file(base_input("first.py", &replacement, "hash3")).expect("replace first");

    for (table, query) in [("lines_fts", "second"), ("lines_trigram", "sec")] {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {table} MATCH ?1");
        let count: i64 = store.connection().query_row(&sql, [query], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "updating one file must preserve {table} rows for other files");
    }
}

#[test]
fn re_upsert_many_files_is_linear() {
    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");

    let n = 2000usize;
    let paths: Vec<String> = (0..n).map(|i| format!("file{i:04}.py")).collect();
    let lines = [(1, "hello world".into()), (2, "foo bar baz".into())];
    let lines2 = [(1, "goodbye world".into()), (2, "qux corge grault".into())];

    store.begin_bulk_tx().expect("begin bulk");
    let start = std::time::Instant::now();
    for (i, path) in paths.iter().enumerate() {
        let input = UpsertFileInput {
            rel_path: path,
            language: Some("python"),
            mtime_secs: i as i64,
            mtime_nanos: 0,
            content_hash: &format!("hash{i}"),
            lines: &lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
        };
        store.upsert_file(input).expect("upsert");
    }
    store.commit_bulk_tx().expect("commit bulk");
    let insert_elapsed = start.elapsed();

    store.begin_bulk_tx().expect("begin bulk 2");
    let start = std::time::Instant::now();
    for (i, path) in paths.iter().enumerate() {
        let input = UpsertFileInput {
            rel_path: path,
            language: Some("python"),
            mtime_secs: (i + n) as i64,
            mtime_nanos: 0,
            content_hash: &format!("hash2_{i}"),
            lines: &lines2,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
        };
        store.upsert_file(input).expect("re-upsert");
    }
    store.commit_bulk_tx().expect("commit bulk 2");
    let re_elapsed = start.elapsed();

    let total = insert_elapsed + re_elapsed;
    assert!(
        re_elapsed < std::time::Duration::from_secs(15),
        "re-upsert of {n} files took {:?}; expected < 15s", re_elapsed
    );
    assert!(
        total < std::time::Duration::from_secs(30),
        "insert + re-upsert of {n} files took {:?}; expected < 30s", total
    );
}

/// Same symbol/call/import spans but different function body text must refresh
/// semantic_chunks text and vectors (must not take the lines-only fast path).
#[test]
fn same_span_body_edit_refreshes_semantic_chunks() {
    use ast_sgrep_core::semantic_chunk::{build_semantic_chunks, SemanticChunkInput};

    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");
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

    let patterns_v1 = [PatternNode {
        signature: "fn compute".into(),
        line_start: 1,
        line_end: 3,
        excerpt: "return ALPHA_TOKEN_111".into(),
    }];
    let patterns_v2 = [PatternNode {
        signature: "fn compute".into(),
        line_start: 1,
        line_end: 3,
        excerpt: "return BETA_TOKEN_222".into(),
    }];

    let upsert = |lines: &[(u32, String)],
                  chunks: &[SemanticChunkInput],
                  patterns: &[PatternNode],
                  hash: &str| {
        store
            .upsert_file(UpsertFileInput {
                rel_path: path,
                language: Some("python"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: hash,
                lines,
                eol: "\n",
                symbols: &symbols,
                callers: &callers,
                imports: &imports,
                pattern_nodes: patterns,
                semantic_chunks: chunks,
                embed_semantic: true,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .expect("upsert");
    };

    upsert(&lines_v1, &chunks_v1, &patterns_v1, "hash_alpha");
    let rows_v1 = store.all_semantic_chunks(None).expect("chunks v1");
    assert_eq!(rows_v1.len(), 1);
    let text_v1 = rows_v1[0].4.clone();
    let vec_v1 = rows_v1[0].5.clone();
    assert!(text_v1.contains("ALPHA_TOKEN_111"), "v1 body missing: {text_v1}");

    upsert(&lines_v2, &chunks_v2, &patterns_v2, "hash_beta");
    let rows_v2 = store.all_semantic_chunks(None).expect("chunks v2");
    assert_eq!(rows_v2.len(), 1);
    let text_v2 = &rows_v2[0].4;
    let vec_v2 = &rows_v2[0].5;
    assert!(text_v2.contains("BETA_TOKEN_222"), "v2 body missing: {text_v2}");
    assert!(!text_v2.contains("ALPHA_TOKEN_111"), "stale v1 body: {text_v2}");
    assert_ne!(text_v1, *text_v2);
    assert_ne!(vec_v1, *vec_v2, "vectors must re-embed on same-span body edit");

    let pattern_excerpt: String = store
        .connection()
        .query_row("SELECT excerpt FROM pattern_nodes LIMIT 1", [], |r| r.get(0))
        .expect("pattern row");
    assert!(
        pattern_excerpt.contains("BETA_TOKEN_222"),
        "pattern excerpt must refresh: {pattern_excerpt}"
    );
}
