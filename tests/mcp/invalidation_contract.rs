//! MCP invalidation contract: status discriminants, stale-serve, and
//! session/process freshness agreement (pass1 KEEPs).
//!
//! Harness rides `ast-sgrep-testkit` (`rpc_session`, `LiveSession`,
//! `index_tree`, envelope builders/extractors); every live-session read uses
//! the testkit 15s `recv` bound and every process wait uses the 15s
//! `wait_clean` bound, so a regressed server fails the test instead of hanging
//! the suite. File-local helpers below carry only what testkit lacks.

use ast_sgrep_testkit::{
    assert_hit_envelope, assert_miss_envelope, assert_tool_error_shape, assert_tool_success,
    index_tree, rpc_at, search_call, tool_body, tool_call, tool_text, CallSession as Session,
};
use serde_json::json;

// Contract phases ride the shared MCP harness from testkit (`rpc_at`,
// `search_call`, the hit/miss envelope discriminants, and `CallSession`
// aliased to the file's `Session` vocabulary) with no file-local surface.

/// INTENT: index_status three-way discriminant: missing-root tool error,
/// unindexed zeros, fresh counts+nonzero epoch.
/// KILLS: status-discriminant mutant (missing→success, unindexed-nonzero,
/// fresh-zero-epoch).
/// ABSORBS: none (KEEP).
#[test]
fn index_status_discriminates_missing_unindexed_and_fresh() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn target_symbol() {}\n").unwrap();

    // Missing root: uniform tool error inside a live session.
    let missing_arg = temp.path().join("does_not_exist").display().to_string();
    let missing = rpc_at(
        tool_call(1, "index_status", json!({"root": missing_arg})),
        temp.path(),
    );
    assert_tool_error_shape(&missing);

    // Existing but never indexed: success with zero counts and epoch 0.
    let unindexed = rpc_at(tool_call(2, "index_status", json!({})), temp.path());
    assert_tool_success(&unindexed);
    let body = tool_body(&unindexed);
    assert_eq!(body["file_count"], 0, "{body:#}");
    assert_eq!(body["writer_generation"], 0, "{body:#}");

    // Fresh index: positive counts and a nonzero writer epoch.
    index_tree(temp.path());
    let fresh = rpc_at(tool_call(3, "index_status", json!({})), temp.path());
    assert_tool_success(&fresh);
    let body = tool_body(&fresh);
    assert_eq!(body["file_count"], 1, "{body:#}");
    assert!(body["symbol_count"].as_u64().unwrap_or(0) >= 1, "{body:#}");
    assert_ne!(body["writer_generation"], 0, "{body:#}");
}

/// INTENT: stale index serves pre-change hits byte-identically with success
/// shape, no staleness keys, frozen epoch.
/// KILLS: stale-visibility mutant (flag keys added, fresh served early, epoch
/// bumped on bare edit).
/// ABSORBS: none (KEEP).
#[test]
fn search_on_stale_index_serves_pre_change_hits_without_flag() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    let before = rpc_at(search_call(1, "alpha_marker"), temp.path());
    assert_tool_success(&before);
    let before_text = tool_text(&before).to_owned();
    assert_hit_envelope(&tool_body(&before));
    let status_before = tool_body(&rpc_at(
        tool_call(2, "index_status", json!({})),
        temp.path(),
    ));

    // Edit without reindexing: the file now names beta, the index still alpha.
    std::fs::write(temp.path().join("lib.rs"), "fn zebroid_quixotic() {}\n").unwrap();

    // Contract: search serves the stale index with success shape, byte-identical
    // to before the edit, and the envelope carries no staleness discriminant.
    let stale = rpc_at(search_call(3, "alpha_marker"), temp.path());
    assert_tool_success(&stale);
    assert_eq!(
        tool_text(&stale),
        before_text,
        "stale search must serve old rows"
    );
    let body = tool_body(&stale);
    assert_hit_envelope(&body);
    for key in ["stale", "fresh", "dirty", "generation", "writer_generation"] {
        assert!(body.get(key).is_none(), "no {key} discriminant: {body:#}");
    }
    // The new symbol is invisible until a reindex.
    let unseen = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
    assert_tool_success(&unseen);
    assert_miss_envelope(&tool_body(&unseen), "no_match");

    // A bare file edit moves no index discriminant.
    let status_after = tool_body(&rpc_at(
        tool_call(5, "index_status", json!({})),
        temp.path(),
    ));
    assert_eq!(status_after["file_count"], status_before["file_count"]);
    assert_eq!(
        status_after["writer_generation"], status_before["writer_generation"],
        "writer epoch moves only on index mutation"
    );
}

/// INTENT: out-of-band reindex invalidates a warm same-session Searcher
/// (serves fresh rows).
/// KILLS: warm-Searcher-snapshot mutant (generation advance ignored).
/// ABSORBS: none (KEEP).
#[test]
fn external_reindex_invalidates_warm_session_searcher() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    // Warm the session Searcher, then mutate the index out of band.
    let mut session = Session::spawn(temp.path());
    let first = session.call(
        "keyword_search",
        json!({"query": "alpha_marker", "limit": 8}),
    );
    assert_tool_success(&first);
    assert_hit_envelope(&tool_body(&first));

    std::fs::write(temp.path().join("lib.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    index_tree(temp.path());

    // The same session must serve fresh rows, not its warm snapshot.
    let gone = session.call(
        "keyword_search",
        json!({"query": "alpha_marker", "limit": 8}),
    );
    assert_tool_success(&gone);
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let found = session.call(
        "keyword_search",
        json!({"query": "zebroid_quixotic", "limit": 8}),
    );
    assert_tool_success(&found);
    assert_hit_envelope(&tool_body(&found));
    session.finish();
}

/// INTENT: restarted process observes post-mutation epoch+rows, deterministic
/// across restarts.
/// KILLS: restart-stale-generation mutant.
/// ABSORBS: none (KEEP).
#[test]
fn restart_picks_up_fresh_state() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    let first = rpc_at(search_call(1, "alpha_marker"), temp.path());
    assert_hit_envelope(&tool_body(&first));
    let gen_before = tool_body(&rpc_at(
        tool_call(2, "index_status", json!({})),
        temp.path(),
    ))["writer_generation"]
        .as_u64()
        .unwrap();

    std::fs::write(temp.path().join("lib.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    index_tree(temp.path());

    // A restarted process observes the new epoch and the new rows.
    let gone = rpc_at(search_call(3, "alpha_marker"), temp.path());
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let found = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
    assert_tool_success(&found);
    assert_hit_envelope(&tool_body(&found));
    let status = tool_body(&rpc_at(
        tool_call(5, "index_status", json!({})),
        temp.path(),
    ));
    assert_eq!(status["file_count"], 1);
    assert_ne!(status["writer_generation"].as_u64().unwrap(), 0);
    assert_ne!(
        status["writer_generation"].as_u64().unwrap(),
        gen_before,
        "restart must observe the post-mutation epoch"
    );

    // Fresh state is deterministic across restarts.
    let again = rpc_at(search_call(6, "zebroid_quixotic"), temp.path());
    assert_eq!(tool_text(&again), tool_text(&found));
}

/// INTENT: in-session index_repo heals an empty_index miss
/// (miss→stats→hit→count=1).
/// KILLS: empty-heal mutant (index_repo noop on empty, wrong miss code).
/// ABSORBS: none (KEEP).
#[test]
fn index_repo_heals_empty_index_miss_within_session() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn target_symbol() {}\n").unwrap();

    let mut session = Session::spawn(temp.path());
    let miss = session.call(
        "keyword_search",
        json!({"query": "target_symbol", "limit": 8}),
    );
    assert_tool_success(&miss);
    assert_miss_envelope(&tool_body(&miss), "empty_index");

    let reindex = session.call("index_repo", json!({}));
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert!(
        stats["files_indexed"].as_u64().unwrap_or(0) >= 1,
        "{stats:#}"
    );
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    let found = session.call(
        "keyword_search",
        json!({"query": "target_symbol", "limit": 8}),
    );
    assert_tool_success(&found);
    assert_hit_envelope(&tool_body(&found));
    let status = session.call("index_status", json!({}));
    assert_tool_success(&status);
    assert_eq!(tool_body(&status)["file_count"], 1);
    session.finish();
}
