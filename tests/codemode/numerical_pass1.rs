//! Pass 1 (numerical N1): hand-computed numeric oracles for codemode.
//!
//! Owns ONLY previously uncovered numeric surface: `filter_hits` float
//! threshold/limit math, `select` limit truncation, batch byte-budget
//! constant arithmetic, fresh-session call budget, `read` window/char
//! clamp math, and the unknown-tool suggestion distance threshold.
//!
//! Already covered elsewhere (do NOT re-assert): BudgetExhausted/exhausted
//! boundaries (oracle_foundry_pass2, error_api_pass2/3), batch/plan
//! call_count (batch, durable_recovery), MAX_BATCH_CALLS/ID/TOOL ceilings
//! (oracle_foundry_pass2, error_api_pass2), MAX_CALL_RESPONSE_BYTES value
//! (oracle_foundry_pass4), >32 read windows (error_api_pass1), one
//! suggestion happy-path (session_plan).
//!
//! Every expectation below is hand-computed from the source expression.
//! Pure transforms and tempdir disk reads only — no index I/O.

use ast_sgrep_codemode::{
    CallError, CodeModeSession, SessionConfig, MAX_BATCH_ERROR_BYTES,
    MAX_BATCH_RESPONSE_BYTES, MAX_BATCH_VALUE_BYTES,
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

/// Session over an indexed root: `read` fails closed on an empty index, so
/// window/char math tests index first (lexical only, deterministic).
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

#[test]
fn filter_min_score_boundary_is_inclusive() {
    // Reject rule is `score < min` (tools.rs): score == min survives.
    // Hand-computed: 2.0 and 2.001 pass min 2.0; 1.999 is cut.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let out = session
        .call("filter_hits", json!({"hits": hits_fixture(), "min_score": 2.0}))
        .expect("filter runs");
    assert_eq!(out["hit_count"], json!(2));
    assert_eq!(out["hits"][0]["file"], json!("src/a.rs"));
    assert_eq!(out["hits"][1]["file"], json!("src/c.rs"));
}

#[test]
fn filter_missing_score_defaults_to_zero() {
    // Missing score is `unwrap_or(0.0)`: a scoreless hit passes min 0.0
    // and min -1.0, but fails min 0.5 alongside a true 0.0 hit.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let hits = json!([
        {"kind": "def", "file": "src/noscore.rs"},
        {"kind": "def", "file": "src/zero.rs", "score": 0.0},
    ]);
    let at_zero = session
        .call("filter_hits", json!({"hits": hits, "min_score": 0.0}))
        .expect("min 0 runs");
    assert_eq!(at_zero["hit_count"], json!(2));
    let negative = session
        .call("filter_hits", json!({"hits": hits_fixture(), "min_score": -1.0}))
        .expect("negative min runs");
    assert_eq!(negative["hit_count"], json!(3));
    let positive = session
        .call("filter_hits", json!({"hits": hits, "min_score": 0.5}))
        .expect("min 0.5 runs");
    assert_eq!(positive["hit_count"], json!(0));
    assert_eq!(positive["hits"], json!([]));
}

#[test]
fn filter_limit_zero_clamps_to_one() {
    // limit maps through `.clamp(1, MAX_OUTPUT_RESULTS=1000)`: 0 becomes 1
    // (one hit survives), and 99999 becomes 1000 (all 3 survive, no error).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let floored = session
        .call("filter_hits", json!({"hits": hits_fixture(), "limit": 0}))
        .expect("limit 0 runs");
    assert_eq!(floored["hit_count"], json!(1));
    assert_eq!(floored["hits"][0]["file"], json!("src/a.rs"));
    let capped = session
        .call("filter_hits", json!({"hits": hits_fixture(), "limit": 99999}))
        .expect("huge limit runs");
    assert_eq!(capped["hit_count"], json!(3));
}

#[test]
fn select_limit_zero_empties_array() {
    // select has NO clamp: `truncate(0)` yields []. Boundary opposite of
    // filter_hits, where limit 0 clamps up to 1.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let out = session
        .call(
            "select",
            json!({"value": [{"a": 1}, {"a": 2}], "fields": ["a"], "limit": 0}),
        )
        .expect("select runs");
    assert_eq!(out, json!([]));
}

#[test]
fn budget_constants_and_fresh_session_are_hand_computed() {
    // 4 MiB response cap minus 64 KiB envelope reserve:
    // 4*1024*1024 = 4194304; 4194304 - 64*1024 = 4128768.
    assert_eq!(MAX_BATCH_RESPONSE_BYTES, 4_194_304);
    assert_eq!(MAX_BATCH_VALUE_BYTES, 4_128_768);
    assert_eq!(MAX_BATCH_VALUE_BYTES + 64 * 1024, MAX_BATCH_RESPONSE_BYTES);
    assert_eq!(MAX_BATCH_ERROR_BYTES, 8 * 1024);
    // Fresh session: 64-call budget, zero consumed, not exhausted.
    let temp = tempfile::tempdir().expect("tempdir");
    let session = session_at(temp.path());
    assert_eq!(session.max_calls, 64);
    assert_eq!(session.call_count(), 0);
    assert!(!session.exhausted());
}

#[test]
fn read_start_end_clamp_to_valid_range() {
    // start maps `.max(1)`; end maps `.max(start)`. File l1..l5:
    // start=0 -> line 1; start=3,end=0 -> line 3 only.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").expect("write");
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let floored = session
        .call("read", json!({"path": "f.txt", "start": 0}))
        .expect("start 0 runs");
    assert_eq!(floored["windows"][0]["start"], json!(1));
    assert_eq!(floored["windows"][0]["end"], json!(1));
    assert_eq!(floored["windows"][0]["text"], json!("l1"));
    let end_pinned = session
        .call("read", json!({"path": "f.txt", "start": 3, "end": 0}))
        .expect("end 0 runs");
    assert_eq!(end_pinned["windows"][0]["start"], json!(3));
    assert_eq!(end_pinned["windows"][0]["end"], json!(3));
    assert_eq!(end_pinned["windows"][0]["text"], json!("l3"));
}

#[test]
fn read_context_lines_widens_symmetrically() {
    // context_lines=1 on line 3 of l1..l5 -> lines 2..4, joined with \n.
    // context_lines huge pins to .min(MAX_EXCERPT_LINES=100) -> whole file.
    let temp = tempfile::tempdir().expect("tempdir");
    // No trailing newline: the index stores split('\n'), so a trailing \n
    // would add an empty 6th line and this oracle is about context math.
    std::fs::write(temp.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5").expect("write");
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let widened = session
        .call(
            "read",
            json!({"path": "f.txt", "start": 3, "end": 3, "context_lines": 1}),
        )
        .expect("context 1 runs");
    assert_eq!(widened["windows"][0]["start"], json!(2));
    assert_eq!(widened["windows"][0]["end"], json!(4));
    assert_eq!(widened["windows"][0]["text"], json!("l2\nl3\nl4"));
    let saturated = session
        .call(
            "read",
            json!({"path": "f.txt", "start": 3, "end": 3, "context_lines": 1_000_000_000u64}),
        )
        .expect("huge context runs");
    assert_eq!(saturated["windows"][0]["start"], json!(1));
    assert_eq!(saturated["windows"][0]["end"], json!(5));
    assert_eq!(
        saturated["windows"][0]["text"],
        json!("l1\nl2\nl3\nl4\nl5")
    );
}

#[test]
fn read_char_budgets_truncate_with_flag() {
    // max_chars maps `.clamp(1, 100000)`: 0 -> 1. Single-char lines a/b/c:
    // line 1 costs 0+1=1 char (fits), line 2 costs 1+1=2 (1+2>1, cut).
    // A 2500-char line pins to MAX_LINE_CHARS=2000 with truncated=true.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("abc.txt"), "a\nb\nc\n").expect("write");
    std::fs::write(temp.path().join("wide.txt"), format!("{}\n", "x".repeat(2500)))
        .expect("write");
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let one_char = session
        .call("read", json!({"path": "abc.txt", "start": 1, "end": 3, "max_chars": 0}))
        .expect("max_chars 0 runs");
    assert_eq!(one_char["windows"][0]["text"], json!("a"));
    assert_eq!(one_char["windows"][0]["start"], json!(1));
    assert_eq!(one_char["windows"][0]["end"], json!(1));
    assert_eq!(one_char["windows"][0]["truncated"], json!(true));
    let wide = session
        .call("read", json!({"path": "wide.txt", "start": 1, "end": 1}))
        .expect("wide line runs");
    assert_eq!(wide["windows"][0]["text"], json!("x".repeat(2000)));
    assert_eq!(wide["windows"][0]["truncated"], json!(true));
}

#[test]
fn unknown_tool_suggestion_threshold() {
    // Threshold is (len/3).max(1). "serach" (len 6 -> dist budget 2,
    // Levenshtein 2 from "search" via the a/r swap) suggests; "xyzq"
    // (len 4 -> budget 1, distance >1 from every tool) does not.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let err = session
        .call("serach", json!({}))
        .expect_err("typo is unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert!(err.to_string().contains("Did you mean search"), "{err}");
    let err = session.call("xyzq", json!({})).expect_err("far is unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert!(!err.to_string().contains("Did you mean"), "{err}");
}
