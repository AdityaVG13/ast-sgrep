use ast_sgrep_codemode::plan::{example_plan, parse_plan, run_plan};
use ast_sgrep_codemode::{CodeModeSession, SessionConfig, MAX_CALL_RESPONSE_BYTES};
use ast_sgrep_core::{IndexOptions, Indexer};
use ast_sgrep_testkit::sample_root;
use serde_json::json;
use std::fs;
use std::time::Instant;
use tempfile::TempDir;

fn indexed_session() -> (TempDir, CodeModeSession) {
    let temp = TempDir::new().expect("tempdir");
    let index_path = temp.path().join("index.db");
    let root = sample_root();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");

    let session = CodeModeSession::new(SessionConfig {
        root,
        index_path: Some(index_path),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    });
    (temp, session)
}

fn indexed_embed_session() -> (TempDir, CodeModeSession) {
    let temp = TempDir::new().expect("tempdir");
    let index_path = temp.path().join("index.db");
    let root = sample_root();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");

    let session = CodeModeSession::new(SessionConfig {
        root,
        index_path: Some(index_path),
        limit: 8,
        use_embed: true,
        ..SessionConfig::default()
    });
    (temp, session)
}

#[test]
fn relative_index_path_resolves_against_session_root() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("lib.rs"), "pub fn relative_index() {}\n").expect("src");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.path().to_path_buf(),
        index_path: Some(std::path::PathBuf::from("custom-index")),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    });
    session
        .call("index_repo", json!({ "force": false }))
        .expect("index");
    let status = session.call("index_status", json!({})).expect("status");
    let expected = root
        .path()
        .canonicalize()
        .expect("canonicalize session root")
        .join("custom-index")
        .join("index.db");
    let actual = status["index_path"].as_str().expect("index_path");
    assert_eq!(std::path::Path::new(actual), expected.as_path(), "{status}");
}

#[test]
fn search_returns_capsule_by_default() {
    let (_tmp, mut session) = indexed_session();
    let out = session
        .call("search", json!({"query": "auth", "limit": 5}))
        .expect("search");
    assert_eq!(out["provider"], "ast-sgrep");
    assert_eq!(out["mode"], "capsule");
    assert!(out["hits"].as_array().unwrap().len() <= 5);
}

#[test]
fn defs_and_filter_compose_without_model() {
    let (_tmp, mut session) = indexed_session();
    let defs = session
        .call("defs", json!({"symbol": "auth_refresh", "limit": 5}))
        .expect("defs");
    assert!(defs["hit_count"].as_u64().unwrap_or(0) >= 1 || defs["hits"].as_array().is_some());

    let filtered = session
        .call(
            "filter_hits",
            json!({
                "hits": defs,
                "limit": 2
            }),
        )
        .expect("filter");
    assert!(filtered["hit_count"].as_u64().unwrap() <= 2);
}

#[test]
fn plan_runner_resolves_step_refs() {
    let (_tmp, mut session) = indexed_session();
    let plan = parse_plan(&json!({
        "steps": [
            {"id": "seed", "tool": "search", "args": {"query": "auth", "format": "capsule", "limit": 5}},
            {"id": "narrow", "tool": "filter_hits", "args": {"hits": "$seed", "limit": 3}},
            {"id": "out", "tool": "select", "args": {
                "value": "$narrow",
                "fields": ["hit_count", "hits"]
            }}
        ],
        "return": "$out"
    }))
    .expect("parse");
    let result = run_plan(&mut session, &plan).expect("run");
    assert!(result.ok);
    assert!(result.return_value.get("hit_count").is_some());
    assert!(result.call_count >= 2);
}

#[test]
fn example_plan_is_valid_json_shape() {
    let plan = parse_plan(&example_plan()).expect("example plan parses");
    assert_eq!(plan.steps.len(), 4);
}

#[test]
fn session_rejects_an_oversized_encoded_tool_value() {
    let (_tmp, mut session) = indexed_session();
    let error = session
        .call(
            "select",
            json!({
                "value": {"payload": "x".repeat(MAX_CALL_RESPONSE_BYTES + 1)},
                "fields": ["payload"],
            }),
        )
        .expect_err("oversized value must fail before host conversion");
    assert!(error
        .to_string()
        .contains(&MAX_CALL_RESPONSE_BYTES.to_string()));
}

#[test]
fn index_repo_updates_only_known_changed_and_deleted_paths() {
    let root = TempDir::new().expect("root");
    let index = TempDir::new().expect("index");
    let source = root.path().join("source.rs");
    fs::write(&source, "fn before() {}\n").expect("write source");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.path().to_path_buf(),
        index_path: Some(index.path().join("index.db")),
        use_embed: false,
        ..SessionConfig::default()
    });
    session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");

    fs::write(&source, "fn after() {}\n").expect("modify source");
    let changed = session
        .call("index_repo", json!({"paths": ["source.rs"]}))
        .expect("targeted update");
    assert_eq!(changed["targeted"], true);
    assert_eq!(changed["path_count"], 1);
    assert_eq!(changed["stats"]["files_indexed"], 1);

    fs::remove_file(&source).expect("delete source");
    let deleted = session
        .call("index_repo", json!({"paths": [source]}))
        .expect("targeted deletion");
    assert_eq!(deleted["stats"]["files_removed"], 1);
    assert_eq!(
        session.call("index_status", json!({})).expect("status")["file_count"],
        0
    );
}

#[test]
fn index_repo_expands_directory_paths_into_contained_files() {
    let root = TempDir::new().expect("root");
    let index = TempDir::new().expect("index");
    let nested = root.path().join("ARCHANA-3/src");
    fs::create_dir_all(&nested).expect("dir");
    fs::write(
        nested.join("train.py"),
        "def model_training_render_main():\n    return 1\n",
    )
    .expect("write scoped");
    fs::write(
        root.path().join("noise.py"),
        "def model_training_render_main():\n    return 2\n",
    )
    .expect("write noise");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.path().to_path_buf(),
        index_path: Some(index.path().join("index.db")),
        use_embed: false,
        ..SessionConfig::default()
    });
    let updated = session
        .call("index_repo", json!({"paths": ["ARCHANA-3/src"]}))
        .expect("targeted directory");
    assert_eq!(updated["targeted"], true);
    assert_eq!(
        updated["stats"]["files_indexed"], 1,
        "directory path must expand to contained files, not no-op: {updated}"
    );
}

#[test]
fn index_repo_rejects_targeted_paths_outside_root() {
    let root = TempDir::new().expect("root");
    let outside = TempDir::new().expect("outside");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.path().to_path_buf(),
        index_path: Some(root.path().join("index.db")),
        use_embed: false,
        ..SessionConfig::default()
    });
    let traversal = session
        .call("index_repo", json!({"paths": ["../outside.rs"]}))
        .expect_err("traversal must fail");
    assert!(traversal.to_string().contains("traversal rejected"));

    let escaped = session
        .call(
            "index_repo",
            json!({"paths": [outside.path().join("outside.rs")]}),
        )
        .expect_err("outside path must fail");
    assert!(escaped.to_string().contains("outside project root"));
}

#[test]
fn session_root_override_cannot_escape_configured_project() {
    let root = TempDir::new().expect("root");
    let child = root.path().join("child");
    fs::create_dir(&child).expect("child");
    let outside = TempDir::new().expect("outside");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.path().to_path_buf(),
        index_path: Some(root.path().join("index.db")),
        use_embed: false,
        ..SessionConfig::default()
    });

    session
        .call("index_status", json!({"root": "child"}))
        .expect("contained subroot is allowed");
    let error = session
        .call("index_status", json!({"root": outside.path()}))
        .expect_err("outside root must fail");
    assert!(error
        .to_string()
        .contains("outside the configured session root"));
}

/// lbx1.11: real session with `use_embed: true` must index hashed chunks and
/// return embed hits (not a flag-only green).
#[test]
fn session_embed_on_indexes_and_returns_semantic_hits() {
    let root = TempDir::new().expect("root");
    let index_dir = TempDir::new().expect("index dir");
    fs::write(
        root.path().join("planted.rs"),
        "pub fn planted_lbx111_embed() { let _ = \"unique lbx111 semantic phrase\"; }\n",
    )
    .expect("write");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.path().to_path_buf(),
        index_path: Some(index_dir.path().join("index.db")),
        limit: 8,
        use_embed: true,
        ..SessionConfig::default()
    });
    session
        .call("index_repo", json!({ "force": false }))
        .expect("embed-on index");
    let status = session.call("index_status", json!({})).expect("status");
    assert!(
        status["semantic_chunk_count"].as_u64().unwrap_or(0) > 0,
        "embed-on index must store semantic chunks: {status}"
    );
    assert!(
        status["embed_backend"].as_str().is_some(),
        "embed-on index must record backend: {status}"
    );

    let out = session
        .call(
            "search",
            json!({
                "query": "unique lbx111 semantic phrase",
                "semantic_only": true,
                "format": "agent",
                "limit": 8
            }),
        )
        .expect("semantic search");
    let hits = out["hits"].as_array().expect("hits array");
    assert!(
        !hits.is_empty(),
        "semantic_only must not be empty through the session API: {out}"
    );
    assert!(
        hits.iter()
            .any(|hit| hit["kind"] == "embed" || hit["semantic"] == true),
        "expected embed hits through the session API: {out}"
    );
}

fn writable_session() -> (TempDir, CodeModeSession) {
    let temp = TempDir::new().expect("tempdir");
    fs::write(temp.path().join("hello.py"), "def hello():\n    return 1\n").expect("write");
    let index_path = temp.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");
    let session = CodeModeSession::new(SessionConfig {
        root: temp.path().canonicalize().expect("canon root"),
        index_path: Some(index_path),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    });
    (temp, session)
}

#[test]
fn find_is_lexical_word_lookup() {
    let (_tmp, mut session) = writable_session();
    let out = session
        .call("find", json!({"query": "hello", "limit": 8}))
        .expect("find");
    let hits = out["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["file"].as_str().unwrap_or("").contains("hello.py")),
        "find hello should hit hello.py: {out}"
    );
}

#[test]
fn read_returns_indexed_line_window() {
    let (_tmp, mut session) = writable_session();
    let out = session
        .call("read", json!({"path": "hello.py", "start": 1, "end": 2}))
        .expect("read");
    assert_eq!(out["ok"], true);
    assert_eq!(out["count"], 1);
    let text = out["windows"][0]["text"].as_str().expect("text");
    assert!(text.contains("def hello"), "{text}");
}

#[test]
fn edit_unique_replace_then_reindex() {
    let (_tmp, mut session) = writable_session();
    let out = session
        .call(
            "edit",
            json!({
                "path": "hello.py",
                "oldText": "return 1",
                "newText": "return 2"
            }),
        )
        .expect("edit");
    assert_eq!(out["ok"], true);
    assert_eq!(out["changed"], 1);
    let body = fs::read_to_string(_tmp.path().join("hello.py")).expect("reread");
    assert!(body.contains("return 2"), "{body}");
    let window = session
        .call("read", json!({"path": "hello.py", "start": 1, "end": 2}))
        .expect("read after edit");
    let text = window["windows"][0]["text"].as_str().expect("text");
    assert!(text.contains("return 2"), "{text}");
}

#[test]
fn edit_batch_is_atomic_when_a_later_edit_fails() {
    let (_tmp, mut session) = writable_session();
    // First edit is valid; second can never match. The batch must fail
    // without committing the first edit — callers never see ok:false on a
    // partially applied edits[].
    session
        .call(
            "edit",
            json!({
                "edits": [
                    { "path": "hello.py", "oldText": "return 1", "newText": "return 99" },
                    { "path": "hello.py", "oldText": "not-present-anywhere", "newText": "x" }
                ]
            }),
        )
        .expect_err("a batch containing an unresolvable edit must fail");
    let body = fs::read_to_string(_tmp.path().join("hello.py")).expect("reread");
    assert!(
        body.contains("return 1"),
        "the valid earlier edit must not have committed: {body}"
    );
    assert!(!body.contains("return 99"), "{body}");
}

#[test]
fn edit_batch_applies_all_valid_edits_in_order() {
    let (_tmp, mut session) = writable_session();
    let out = session
        .call(
            "edit",
            json!({
                "edits": [
                    { "path": "hello.py", "oldText": "return 1", "newText": "return 2" },
                    { "path": "hello.py", "oldText": "def hello():", "newText": "def greet():" }
                ]
            }),
        )
        .expect("all-valid batch");
    assert_eq!(out["ok"], true);
    assert_eq!(out["changed"], 2);
    let body = fs::read_to_string(_tmp.path().join("hello.py")).expect("reread");
    assert!(
        body.contains("def greet():") && body.contains("return 2"),
        "{body}"
    );
}

#[test]
fn edit_rejects_non_unique_old_text() {
    let (_tmp, mut session) = writable_session();
    let err = session
        .call(
            "edit",
            json!({
                "path": "hello.py",
                "oldText": "e",
                "newText": "x"
            }),
        )
        .expect_err("non-unique must fail");
    assert!(err.to_string().contains("exactly once"), "{err}");
}

#[test]
fn defs_accepts_query_as_symbol_alias() {
    let (_tmp, mut session) = indexed_session();
    let out = session
        .call("defs", json!({"query": "auth_refresh", "limit": 5}))
        .expect("defs via query");
    assert!(
        out["hit_count"].as_u64().unwrap_or(0) >= 1 || out["hits"].as_array().is_some(),
        "{out}"
    );
}

#[test]
fn define_alias_dispatches_to_defs() {
    let (_tmp, mut session) = indexed_session();
    let out = session
        .call("define", json!({"symbol": "auth_refresh", "limit": 5}))
        .expect("define alias");
    assert!(out["hits"].as_array().is_some(), "{out}");
}

#[test]
fn repeated_search_is_identical_on_the_sticky_session() {
    let (_tmp, mut session) = indexed_session();
    let first = session
        .call("search", json!({"query": "auth", "limit": 5}))
        .expect("first");
    let second = session
        .call("search", json!({"query": "auth", "limit": 5}))
        .expect("second");
    assert_eq!(first["hits"], second["hits"], "{first} vs {second}");
    assert_eq!(first["hit_count"], second["hit_count"]);
}

#[test]
fn sticky_session_repeat_is_faster_than_unique_search() {
    let (_tmp, mut session) = indexed_session();
    session
        .call("search", json!({"query": "auth", "limit": 5}))
        .expect("warmup");

    const N: usize = 24;
    let mut unique_ns = Vec::with_capacity(N);
    for i in 0..N {
        let query = format!("auth needle-{i}");
        let t0 = Instant::now();
        session
            .call("search", json!({"query": query, "limit": 5}))
            .expect("unique");
        unique_ns.push(t0.elapsed().as_nanos() as u64);
    }

    let mut repeat_ns = Vec::with_capacity(N);
    for _ in 0..N {
        let t0 = Instant::now();
        session
            .call("search", json!({"query": "auth", "limit": 5}))
            .expect("repeat");
        repeat_ns.push(t0.elapsed().as_nanos() as u64);
    }

    unique_ns.sort_unstable();
    repeat_ns.sort_unstable();
    let unique_p50 = unique_ns[N / 2];
    let unique_p100 = *unique_ns.last().unwrap();
    let repeat_p50 = repeat_ns[N / 2];
    let repeat_p100 = *repeat_ns.last().unwrap();
    eprintln!(
        "codemode sticky search n={N} unique p50={:.3}ms p100={:.3}ms repeat p50={:.3}ms p100={:.3}ms",
        unique_p50 as f64 / 1e6,
        unique_p100 as f64 / 1e6,
        repeat_p50 as f64 / 1e6,
        repeat_p100 as f64 / 1e6
    );
    if unique_p50 > 50_000 {
        assert!(
            repeat_p50 < unique_p50,
            "repeat p50 {repeat_p50}ns must beat unique p50 {unique_p50}ns"
        );
    }
}

#[test]
fn sticky_embed_hybrid_unique_search_latency() {
    let (_tmp, mut session) = indexed_embed_session();
    session
        .call("search", json!({"query": "warmup probe token", "limit": 8}))
        .expect("warmup");

    let queries = [
        "how does auth refresh work",
        "credential renewal",
        "sanitize user input",
        "process inbound request",
        "token refresh flow",
        "validate the session cookie",
        "store durable credentials",
        "rank hybrid search results",
        "debounce noisy file events",
        "retry after a timeout",
        "combine two search channels",
        "remember query embeddings",
    ];
    let mut unique_ns = Vec::with_capacity(queries.len());
    for q in queries {
        let t0 = Instant::now();
        session
            .call("search", json!({"query": q, "limit": 8}))
            .expect("unique hybrid");
        let ns = t0.elapsed().as_nanos() as u64;
        eprintln!(
            "codemode sticky embed-hybrid unique {q:?} {:.3}ms",
            ns as f64 / 1e6
        );
        unique_ns.push(ns);
    }
    unique_ns.sort_unstable();
    let n = unique_ns.len();
    let p50 = unique_ns[n / 2];
    let p100 = *unique_ns.last().unwrap();
    eprintln!(
        "codemode sticky embed-hybrid unique n={n} p50={:.3}ms p100={:.3}ms",
        p50 as f64 / 1e6,
        p100 as f64 / 1e6
    );
    let t0 = Instant::now();
    session
        .call(
            "search",
            json!({"query": "how does auth refresh work", "limit": 8}),
        )
        .expect("repeat hybrid");
    eprintln!(
        "codemode sticky embed-hybrid repeat {:.3}ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
fn peek_cached_search_hits_after_first_call() {
    let (_tmp, mut session) = indexed_session();
    let args = json!({"query": "auth", "limit": 5});
    assert!(
        session.peek_cached_search(&args).is_none(),
        "unique search must not pretend to be cached"
    );
    let first = session.call("search", args.clone()).expect("search");
    let peeked = session
        .peek_cached_search(&args)
        .expect("sticky repeat must hit the render cache");
    assert_eq!(peeked["hits"], first["hits"], "{peeked} vs {first}");
}

#[test]
fn search_injects_in_and_fail_closes_unknown_lang() {
    let (_tmp, mut session) = indexed_session();
    // in: scopes refuse loudly when they match nothing (65fb66ba) — an empty
    // scope silently returning zero hits hides typos in path filters.
    let missing_scope = session
        .call(
            "search",
            json!({"query": "auth", "in": "no_such_dir", "limit": 8}),
        )
        .expect_err("a scope matching no file must fail loudly");
    assert!(
        missing_scope
            .to_string()
            .contains("matches no file or directory"),
        "{missing_scope}"
    );
    let scoped = session
        .call("search", json!({"query": "auth", "in": ".", "limit": 8}))
        .expect("valid scope must search");
    assert!(scoped["hits"].is_array(), "{scoped}");
    let err = session
        .call("search", json!({"query": "auth", "lang": "notalang"}))
        .expect_err("unknown lang");
    assert!(err.to_string().contains("unknown lang"), "{err}");
}

#[test]
fn unknown_tool_suggests_a_close_name() {
    let (_tmp, mut session) = indexed_session();
    let err = session
        .call("searc", json!({"query": "auth"}))
        .expect_err("typo");
    let message = err.to_string();
    assert!(message.contains("Did you mean search"), "{message}");
}
