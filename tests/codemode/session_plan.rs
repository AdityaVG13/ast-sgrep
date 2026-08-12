use ast_sgrep_codemode::plan::{example_plan, parse_plan, run_plan};
use ast_sgrep_codemode::{CodeModeSession, SessionConfig, MAX_CALL_RESPONSE_BYTES};
use ast_sgrep_core::{IndexOptions, Indexer};
use ast_sgrep_testkit::sample_root;
use serde_json::json;
use std::fs;
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
