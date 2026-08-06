use ast_sgrep_codemode::plan::{example_plan, parse_plan, run_plan};
use ast_sgrep_codemode::{CodeModeSession, SessionConfig};
use ast_sgrep_core::{IndexOptions, Indexer};
use ast_sgrep_testkit::sample_root;
use serde_json::json;
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
        .call(
            "search",
            json!({"query": "auth", "limit": 5}),
        )
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
