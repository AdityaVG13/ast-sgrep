//! Pass 2 (numerical N2): degenerate-input totality for codemode numerics.
//!
//! Owns ONLY totality over degenerate numeric inputs: non-numeric/NaN
//! thresholds, wrong-typed scores, negative/float/string limits, huge
//! budgets and batch configs, negative/huge read windows, empty refs.
//! Each test pins the real outcome (reject/default/clamp) via values or
//! discriminants — never panic, never silent garbage, never message text.
//!
//! Already covered elsewhere (do NOT re-assert): filter min inclusive,
//! missing-score default, negative min -1.0, limit 0/huge (numerical_pass1);
//! select limit 0 (numerical_pass1); budget constants + fresh 64
//! (numerical_pass1); read start/end 0, context, max_chars 0, wide line
//! (numerical_pass1); suggestion threshold (numerical_pass1); empty
//! batch/plan shape, 33-call ceiling, id/tool guards, max_calls 0,
//! BudgetExhausted stickiness (oracle_foundry_pass2/3, error_api_pass2/3);
//! empty hits/array Ok, min 1e9 impossible (oracle_foundry_pass3/4).
//!
//! Pure transforms and tempdir disk reads only — no index I/O except where
//! read/search math requires an indexed root (lexical only, deterministic).

use ast_sgrep_codemode::{
    BatchCall, BatchRequest, CallError, CodeModeSession, SessionConfig, parse_plan, run_batch,
    run_plan,
};
use ast_sgrep_plugins::OutputFormat;
use serde_json::json;

fn session_at(root: &std::path::Path) -> CodeModeSession {
    CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: None,
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    })
}

fn config_at(root: &std::path::Path) -> SessionConfig {
    SessionConfig {
        root: root.to_path_buf(),
        index_path: None,
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    }
}

fn indexed_session_at(root: &std::path::Path) -> (tempfile::TempDir, CodeModeSession) {
    let index_dir = tempfile::tempdir().expect("index dir");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(index_dir.path().join("index.db")),
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    });
    let indexed = session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");
    assert_eq!(indexed["ok"], json!(true));
    (index_dir, session)
}

fn hits_fixture() -> serde_json::Value {
    json!([
        {"kind": "def", "file": "src/a.rs", "score": 2.0},
        {"kind": "def", "file": "src/b.rs", "score": 1.999},
        {"kind": "def", "file": "src/c.rs", "score": 2.001},
    ])
}

fn batch_request(calls: Vec<BatchCall>, limit: Option<usize>) -> BatchRequest {
    BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit,
        parallel: None,
        parallel_mode: None,
        calls,
    }
}

fn catalog_call(id: &str) -> BatchCall {
    BatchCall {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    }
}

#[test]
fn filter_non_numeric_min_score_is_ignored() {
    // min_score maps via `as_f64()`: string/bool/null/object/array and NaN
    // (json! NaN -> Null) all become None, so no filtering applies.
    // Hand-computed: all 3 fixture hits survive every degenerate threshold.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let thresholds = vec![
        json!("2.0"),
        json!(true),
        json!(null),
        json!({"n": 1}),
        json!([1]),
        json!(f64::NAN),
    ];
    for min in thresholds {
        let out = session
            .call("filter_hits", json!({"hits": hits_fixture(), "min_score": min}))
            .expect("non-numeric min runs");
        assert_eq!(out["hit_count"], json!(3));
        assert_eq!(out["hits"].as_array().expect("hits").len(), 3);
    }
}

#[test]
fn filter_non_numeric_score_defaults_to_zero() {
    // Score maps via `as_f64().unwrap_or(0.0)`: string/bool/null/missing all
    // read as 0.0. Hand-computed: min 0.0 keeps all 5 (0.0<0.0 false);
    // min 0.5 cuts all 5 (0.0<0.5 true); min -1.0 keeps all 5.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let hits = json!([
        {"kind": "def", "file": "src/s.rs", "score": "high"},
        {"kind": "def", "file": "src/b.rs", "score": true},
        {"kind": "def", "file": "src/n.rs", "score": null},
        {"kind": "def", "file": "src/m.rs"},
        {"kind": "def", "file": "src/z.rs", "score": 0.0},
    ]);
    let at_zero = session
        .call("filter_hits", json!({"hits": hits, "min_score": 0.0}))
        .expect("min 0 runs");
    assert_eq!(at_zero["hit_count"], json!(5));
    let at_half = session
        .call("filter_hits", json!({"hits": hits, "min_score": 0.5}))
        .expect("min 0.5 runs");
    assert_eq!(at_half["hit_count"], json!(0));
    assert_eq!(at_half["hits"], json!([]));
    let negative = session
        .call("filter_hits", json!({"hits": hits, "min_score": -1.0}))
        .expect("negative min runs");
    assert_eq!(negative["hit_count"], json!(5));
}

#[test]
fn filter_negative_and_non_numeric_limit_defaults_to_max() {
    // limit maps via `as_u64()`: -1/float/string/bool/null/object all become
    // None, so `unwrap_or(MAX_OUTPUT_RESULTS=1000)` keeps every hit.
    // Hand-computed: all 3 survive each degenerate limit, never an error.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let limits = vec![
        json!(-1),
        json!(1.5),
        json!("3"),
        json!(true),
        json!(null),
        json!({"n": 1}),
    ];
    for limit in limits {
        let out = session
            .call("filter_hits", json!({"hits": hits_fixture(), "limit": limit}))
            .expect("degenerate limit runs");
        assert_eq!(out["hit_count"], json!(3));
    }
}

#[test]
fn select_negative_huge_and_non_numeric_limit_keeps_all() {
    // select has NO clamp and NO default: `as_u64()` None means no truncate.
    // Hand-computed: -1/huge/float/string/null all keep all 3 rows.
    // Boundary opposite of limit 0, which truncates to [] (pass 1).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let value = json!([{"a": 1}, {"a": 2}, {"a": 3}]);
    let limits = vec![
        json!(-1),
        json!(u64::MAX),
        json!(1.5),
        json!("1"),
        json!(null),
    ];
    for limit in limits {
        let out = session
            .call(
                "select",
                json!({"value": value, "fields": ["a"], "limit": limit}),
            )
            .expect("degenerate select limit runs");
        assert_eq!(out.as_array().expect("array").len(), 3);
        assert_eq!(out, json!([{"a": 1}, {"a": 2}, {"a": 3}]));
    }
}

#[test]
fn session_huge_budget_never_exhausts_on_few_calls() {
    // max_calls=usize::MAX: exhausted() is false, one pure call succeeds,
    // counter moves to 1, still not exhausted. Totality: huge budgets never
    // overflow or trip early. Zero budget is already covered elsewhere.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = usize::MAX;
    assert!(!session.exhausted());
    assert_eq!(session.call_count(), 0);
    let out = session
        .call("filter_hits", json!({"hits": hits_fixture()}))
        .expect("huge budget runs");
    assert_eq!(out["hit_count"], json!(3));
    assert_eq!(session.call_count(), 1);
    assert!(!session.exhausted());
}

#[test]
fn batch_limit_zero_and_huge_clamp_without_panic() {
    // BatchRequest limit maps `.clamp(1, 500)`: 0 becomes 1, usize::MAX
    // becomes 500. Hand-computed: pure catalog batch stays Ok with all_ok,
    // call_count 1, serial mode under both degenerate configs.
    let temp = tempfile::tempdir().expect("tempdir");
    for limit in [Some(0usize), Some(usize::MAX)] {
        let req = batch_request(vec![catalog_call("solo")], limit);
        let resp = run_batch(config_at(temp.path()), &req).expect("degenerate batch limit runs");
        assert!(resp.all_ok);
        assert_eq!(resp.call_count, 1);
        assert_eq!(resp.mode, "serial");
        assert_eq!(resp.results.len(), 1);
        assert!(resp.results[0].ok);
    }
}

#[test]
fn search_limit_zero_huge_and_negative_totality() {
    // search limit maps `unwrap_or(config 5).clamp(1, 500)`: 0->1, huge->500,
    // -1/string->5. Fixture has exactly 3 files sharing one word, so the
    // hand-computed counts are 1, 3, 3, 3. Lexical find, deterministic.
    let temp = tempfile::tempdir().expect("tempdir");
    for (name, func) in [("a.rs", "a_one"), ("b.rs", "b_two"), ("c.rs", "c_three")] {
        std::fs::write(
            temp.path().join(name),
            format!("// n2totality marker\npub fn {func}() {{}}\n"),
        )
        .expect("write");
    }
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let floored = session
        .call("find", json!({"query": "n2totality", "limit": 0}))
        .expect("limit 0 runs");
    assert_eq!(floored["hits"].as_array().expect("hits").len(), 1);
    let capped = session
        .call("find", json!({"query": "n2totality", "limit": 1_000_000_000u64}))
        .expect("huge limit runs");
    assert_eq!(capped["hits"].as_array().expect("hits").len(), 3);
    let negative = session
        .call("find", json!({"query": "n2totality", "limit": -1}))
        .expect("negative limit runs");
    assert_eq!(negative["hits"].as_array().expect("hits").len(), 3);
    let stringy = session
        .call("find", json!({"query": "n2totality", "limit": "bad"}))
        .expect("string limit runs");
    assert_eq!(stringy["hits"].as_array().expect("hits").len(), 3);
}

#[test]
fn read_negative_start_end_default_to_one() {
    // start maps `as_u64 -> unwrap_or(1).max(1)`; end maps
    // `unwrap_or(start).max(start)`. Hand-computed on l1..l5: start -1 ->
    // 1..1 ("l1"); start 3 end -1 -> 3..3 ("l3"); string bounds -> 1..1.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5").expect("write");
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let neg = session
        .call("read", json!({"path": "f.txt", "start": -1}))
        .expect("negative start runs");
    assert_eq!(neg["windows"][0]["start"], json!(1));
    assert_eq!(neg["windows"][0]["end"], json!(1));
    assert_eq!(neg["windows"][0]["text"], json!("l1"));
    let pinned = session
        .call("read", json!({"path": "f.txt", "start": 3, "end": -1}))
        .expect("negative end runs");
    assert_eq!(pinned["windows"][0]["start"], json!(3));
    assert_eq!(pinned["windows"][0]["end"], json!(3));
    assert_eq!(pinned["windows"][0]["text"], json!("l3"));
    let stringy = session
        .call("read", json!({"path": "f.txt", "start": "bad", "end": "bad"}))
        .expect("string bounds run");
    assert_eq!(stringy["windows"][0]["start"], json!(1));
    assert_eq!(stringy["windows"][0]["end"], json!(1));
    assert_eq!(stringy["windows"][0]["text"], json!("l1"));
}

#[test]
fn read_beyond_eof_and_huge_end_are_total() {
    // Beyond EOF yields empty text with start==end==requested start and
    // truncated=false. Huge end widens to the real last line. Huge max_chars
    // pins to .clamp(1, 100000), so the whole 5-line file survives.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5").expect("write");
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let past = session
        .call("read", json!({"path": "f.txt", "start": 9999}))
        .expect("beyond EOF runs");
    assert_eq!(past["windows"][0]["start"], json!(9999));
    assert_eq!(past["windows"][0]["end"], json!(9999));
    assert_eq!(past["windows"][0]["text"], json!(""));
    assert_eq!(past["windows"][0]["truncated"], json!(false));
    let widened = session
        .call("read", json!({"path": "f.txt", "start": 1, "end": 1_000_000_000u64}))
        .expect("huge end runs");
    assert_eq!(widened["windows"][0]["start"], json!(1));
    assert_eq!(widened["windows"][0]["end"], json!(5));
    assert_eq!(widened["windows"][0]["text"], json!("l1\nl2\nl3\nl4\nl5"));
    assert_eq!(widened["windows"][0]["truncated"], json!(false));
    let chars = session
        .call(
            "read",
            json!({"path": "f.txt", "start": 1, "end": 5, "max_chars": u64::MAX}),
        )
        .expect("huge max_chars runs");
    assert_eq!(chars["windows"][0]["text"], json!("l1\nl2\nl3\nl4\nl5"));
    assert_eq!(chars["windows"][0]["truncated"], json!(false));
}

#[test]
fn empty_batch_and_empty_plan_reject_under_degenerate_budgets() {
    // Zero-size totality plus validation order: empty batch rejects with
    // InvalidArgs even when the batch limit is 0 or usize::MAX (envelope
    // check precedes config clamp); empty plan rejects with InvalidArgs even
    // when the session budget is 0 or usize::MAX (steps check precedes calls).
    let temp = tempfile::tempdir().expect("tempdir");
    for limit in [Some(0usize), Some(usize::MAX)] {
        let err = run_batch(config_at(temp.path()), &batch_request(vec![], limit))
            .expect_err("empty batch never runs");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    }
    let empty = parse_plan(&json!({"steps": []})).expect("empty parses");
    for max in [0usize, usize::MAX] {
        let mut session = session_at(temp.path());
        session.max_calls = max;
        let err = run_plan(&mut session, &empty).expect_err("empty plan never runs");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    }
}

#[test]
fn read_empty_refs_rejects_without_panic() {
    // Empty refs array and missing path/ref/refs both fail closed as Other
    // (read_windows anyhow), never Ok with silent empty windows, never panic.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("f.txt"), "l1\nl2\n").expect("write");
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let err = session
        .call("read", json!({"refs": []}))
        .expect_err("empty refs must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session.call("read", json!({})).expect_err("missing ref must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
}
