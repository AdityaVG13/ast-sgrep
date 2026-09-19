//! MCP invalidation matrix targets M1–M6: per-surface delta-matrix intents
//! absorbing the 28 per-delta-class / parity MERGE verdicts.
//!
//! Harness rides `ast-sgrep-testkit` (`rpc_session`, `LiveSession`,
//! `index_tree`, envelope builders/extractors); every live-session read uses
//! the testkit 15s `recv` bound and every process wait uses the 15s
//! `wait_clean` bound, so a regressed server fails the test instead of hanging
//! the suite. File-local helpers below carry only what testkit lacks.
//!
//! `writer_generation` is asserted nonzero / advancing where relevant but never
//! compared for equality across refreshes: every `index_all` advertises a new
//! stamp even when no rows change, so the stamp is a liveness signal, not a
//! content digest. Content equality is carried by search bytes and counts.

use ast_sgrep_testkit::{
    assert_hit_envelope, assert_hit_path_set, assert_miss_envelope, assert_tool_success,
    index_tree, rpc_at, search_call, tool_body, tool_call, tool_text, CallSession as Session,
};
use serde_json::{json, Value};
use std::path::Path;

// Matrix phases ride the shared MCP harness from testkit (`rpc_at`,
// `search_call`, the hit/miss envelope discriminants, the compact-path
// projector, and `CallSession` aliased to the file's `Session` vocabulary);
// the fresh-process status/search/refresh probes below are the only
// file-local surface.

// WHY area-local: fresh-process index_status body for status-transition
// phases; only this suite probes one-shot status bodies — single-suite helper.
fn status_body(root: &Path) -> Value {
    let response = rpc_at(tool_call(1, "index_status", json!({})), root);
    assert_tool_success(&response);
    tool_body(&response)
}

// WHY area-local: fresh-process index_status raw bytes for byte-stability
// relations; single-suite helper (see `status_body`).
fn status_text(root: &Path) -> String {
    let response = rpc_at(tool_call(1, "index_status", json!({})), root);
    assert_tool_success(&response);
    tool_text(&response).to_owned()
}

// WHY area-local: fresh-process successful lexical search response body
// bytes for one query; single-suite helper (see `status_body`).
fn search_text(root: &Path, query: &str) -> String {
    let response = rpc_at(search_call(1, query), root);
    assert_tool_success(&response);
    tool_text(&response).to_owned()
}

// WHY area-local: incremental `index_repo` refresh returning the stats body;
// single-suite helper (see `status_body`).
fn refresh(root: &Path) -> Value {
    let response = rpc_at(tool_call(1, "index_repo", json!({})), root);
    assert_tool_success(&response);
    tool_body(&response)
}

// WHY area-local: full `index_repo` rebuild (`force: true`) returning the
// stats body; single-suite helper (see `status_body`).
fn force_refresh(root: &Path) -> Value {
    let response = rpc_at(tool_call(1, "index_repo", json!({"force": true})), root);
    assert_tool_success(&response);
    tool_body(&response)
}

// WHY area-local: content counts that must converge for the same final tree.
// `root` and `index_path` are absolute and excluded; `writer_generation` is a
// liveness stamp and compared only for nonzeroness. Single-suite helper.
fn assert_same_counts(a: &Value, b: &Value) {
    for key in ["file_count", "line_count", "symbol_count"] {
        assert_eq!(a[key], b[key], "status {key} must converge: {a:#} vs {b:#}");
    }
}

// WHY area-local: generation-is-liveness rule: nonzero, never cross-refresh
// equality. Single-suite helper.
fn assert_nonzero_generation(status: &Value) {
    assert_ne!(status["writer_generation"], 0, "{status:#}");
}

/// INTENT (M1-search): keyword_search hit-path sets × {add, modify,
/// modify-move, delete, rename}: each delta class serves stale rows until
/// refresh, then exact fresh path sets.
/// KILLS: unindexed-serve/old-path-drop + modify-swap (stale old hit, new
/// miss, wrong path) + modify-move path-set + delete-row-leak + rename-path
/// (old lingers, new missing) mutants.
/// ABSORBS: add_delta_new_file_hits_appear_only_after_refresh,
/// modify_delta_symbol_swap_exact_path_sets,
/// modify_delta_symbol_move_stale_path_set_then_fresh,
/// delete_delta_pruned_hits_exact_paths, rename_delta_old_path_gone_new_path_hit.
#[test]
fn m1_search_hit_path_sets_across_delta_classes() {
    // Phase ADD: a new file is invisible until refresh, then exact new+old sets.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(temp.path());

        let mut session = Session::spawn(temp.path());
        let keep = session.call(
            "keyword_search",
            json!({"query": "alpha_marker", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&keep);
        assert_hit_path_set(&tool_body(&keep), &["a.rs"]);

        std::fs::write(temp.path().join("b.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        let unseen = session.call(
            "keyword_search",
            json!({"query": "zebroid_quixotic", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&unseen);
        assert_miss_envelope(&tool_body(&unseen), "no_match");

        let reindex = session.call("index_repo", json!({}));
        assert_tool_success(&reindex);

        let found = session.call(
            "keyword_search",
            json!({"query": "zebroid_quixotic", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&found);
        assert_hit_path_set(&tool_body(&found), &["b.rs"]);
        let still = session.call(
            "keyword_search",
            json!({"query": "alpha_marker", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&still);
        assert_hit_path_set(&tool_body(&still), &["a.rs"]);
        session.finish();
    }

    // Phase MODIFY: old symbol misses, new hits the edited path, untouched exact.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn blip_candle() {}\n").unwrap();
        index_tree(temp.path());

        let mut session = Session::spawn(temp.path());
        let warm = session.call(
            "keyword_search",
            json!({"query": "quarry_sphinx", "limit": 8, "resend_seen": true}),
        );
        assert_hit_path_set(&tool_body(&warm), &["a.rs"]);

        std::fs::write(temp.path().join("a.rs"), "fn vortex_elm() {}\n").unwrap();
        let reindex = session.call("index_repo", json!({}));
        assert_tool_success(&reindex);
        let stats = tool_body(&reindex);
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_removed"], 0, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");

        let gone = session.call(
            "keyword_search",
            json!({"query": "quarry_sphinx", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&gone);
        assert_miss_envelope(&tool_body(&gone), "no_match");
        let found = session.call(
            "keyword_search",
            json!({"query": "vortex_elm", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&found);
        assert_hit_path_set(&tool_body(&found), &["a.rs"]);
        let kept = session.call(
            "keyword_search",
            json!({"query": "blip_candle", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&kept);
        assert_hit_path_set(&tool_body(&kept), &["b.rs"]);
        session.finish();
    }

    // Phase MODIFY-MOVE: stale serves the pre-move path with frozen
    // discriminants, fresh serves the post-move path only.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn froth_gazebo() {}\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn plumb_kiosk() {}\n").unwrap();
        index_tree(temp.path());

        let fresh = status_body(temp.path());
        let gen_fresh = fresh["writer_generation"].as_u64().unwrap();

        std::fs::write(temp.path().join("a.rs"), "fn snipe_tundra() {}\n").unwrap();
        std::fs::write(
            temp.path().join("b.rs"),
            "fn plumb_kiosk() {}\nfn froth_gazebo() {}\n",
        )
        .unwrap();

        let stale = status_body(temp.path());
        assert_eq!(stale["file_count"], fresh["file_count"]);
        assert_eq!(
            stale["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "bare modify moves no epoch"
        );
        let served = rpc_at(search_call(1, "froth_gazebo"), temp.path());
        assert_tool_success(&served);
        assert_hit_path_set(&tool_body(&served), &["a.rs"]);

        let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);

        let moved = rpc_at(search_call(3, "froth_gazebo"), temp.path());
        assert_tool_success(&moved);
        assert_hit_path_set(&tool_body(&moved), &["b.rs"]);
        let after = status_body(temp.path());
        assert_eq!(after["file_count"], 2, "{after:#}");
        assert_ne!(
            after["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "refresh must advertise"
        );
    }

    // Phase DELETE: stale row serves the old path until refresh, then misses;
    // kept symbol exact.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("keep.rs"), "fn alpha_marker() {}\n").unwrap();
        std::fs::write(temp.path().join("drop.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        index_tree(temp.path());

        let before = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
        assert_hit_path_set(&tool_body(&before), &["drop.rs"]);

        std::fs::remove_file(temp.path().join("drop.rs")).unwrap();
        let stale = rpc_at(search_call(2, "zebroid_quixotic"), temp.path());
        assert_tool_success(&stale);
        assert_hit_path_set(&tool_body(&stale), &["drop.rs"]);

        let reindex = rpc_at(tool_call(3, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);

        let gone = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
        assert_tool_success(&gone);
        assert_miss_envelope(&tool_body(&gone), "no_match");
        let kept = rpc_at(search_call(5, "alpha_marker"), temp.path());
        assert_tool_success(&kept);
        assert_hit_path_set(&tool_body(&kept), &["keep.rs"]);
        assert_eq!(status_body(temp.path())["file_count"], 1);
    }

    // Phase RENAME: stale old path until refresh; fresh hits the new path only
    // and the old path leaves the `p` table (pinned by helper).
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("old.rs"), "fn i2_rename_sym() {}\n").unwrap();
        index_tree(temp.path());

        let before = rpc_at(search_call(1, "i2_rename_sym"), temp.path());
        assert_hit_path_set(&tool_body(&before), &["old.rs"]);

        std::fs::rename(temp.path().join("old.rs"), temp.path().join("new.rs")).unwrap();
        let stale = rpc_at(search_call(2, "i2_rename_sym"), temp.path());
        assert_tool_success(&stale);
        assert_hit_path_set(&tool_body(&stale), &["old.rs"]);

        let reindex = rpc_at(tool_call(3, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);

        let moved = rpc_at(search_call(4, "i2_rename_sym"), temp.path());
        assert_tool_success(&moved);
        assert_hit_path_set(&tool_body(&moved), &["new.rs"]);
    }
}

/// INTENT (M2-status): index_status transitions + index_repo refresh stats ×
/// {add, modify, delete, rename}: a bare delta moves no discriminant; the
/// refresh advertises exact stats, exact counts, and a new epoch.
/// KILLS: add-stats/count + delete-stats/count + rename-stats (count/symbol
/// drift) + delete-row-leak (prune) mutants.
/// ABSORBS: reindex_after_deletion_prunes_counts_and_hits,
/// add_delta_status_counts_and_refresh_stats,
/// delete_delta_status_transition_and_refresh_stats,
/// rename_delta_status_file_count_stable. The MODIFY column has no dedicated
/// absorbed test; the target's × {add, modify, delete, rename} definition
/// requires it (modify stats shape (1,0,0) is shared with M1's MODIFY phase).
#[test]
fn m2_status_transitions_and_refresh_stats() {
    // Phase ADD: bare add moves no discriminant; refresh stats (1,0,0),
    // count+1, epoch advances.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn womble_frascati() {}\n").unwrap();
        index_tree(temp.path());

        let fresh = status_body(temp.path());
        assert_eq!(fresh["file_count"], 1, "{fresh:#}");
        let gen_fresh = fresh["writer_generation"].as_u64().unwrap();
        assert_ne!(gen_fresh, 0);

        std::fs::write(temp.path().join("b.rs"), "fn joltik_nimbus() {}\n").unwrap();
        let stale = status_body(temp.path());
        assert_eq!(stale["file_count"], 1, "{stale:#}");
        assert_eq!(
            stale["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "bare add moves no epoch"
        );

        let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);
        let stats = tool_body(&reindex);
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_removed"], 0, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");

        let after = status_body(temp.path());
        assert_eq!(after["file_count"], 2, "{after:#}");
        let gen_after = after["writer_generation"].as_u64().unwrap();
        assert_ne!(gen_after, 0);
        assert_ne!(gen_after, gen_fresh, "refresh must advertise");

        let found = rpc_at(search_call(3, "joltik_nimbus"), temp.path());
        assert_hit_path_set(&tool_body(&found), &["b.rs"]);
    }

    // Phase MODIFY: bare modify moves no discriminant; refresh stats (1,0,0),
    // count stable, epoch advances.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn womble_frascati() {}\n").unwrap();
        index_tree(temp.path());

        let fresh = status_body(temp.path());
        assert_eq!(fresh["file_count"], 1, "{fresh:#}");
        let gen_fresh = fresh["writer_generation"].as_u64().unwrap();
        assert_ne!(gen_fresh, 0);

        std::fs::write(temp.path().join("a.rs"), "fn joltik_nimbus() {}\n").unwrap();
        let stale = status_body(temp.path());
        assert_eq!(stale["file_count"], 1, "{stale:#}");
        assert_eq!(
            stale["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "bare modify moves no epoch"
        );

        let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);
        let stats = tool_body(&reindex);
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_removed"], 0, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");

        let after = status_body(temp.path());
        assert_eq!(after["file_count"], 1, "{after:#}");
        assert_ne!(
            after["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "refresh must advertise"
        );
        let gone = rpc_at(search_call(3, "womble_frascati"), temp.path());
        assert_miss_envelope(&tool_body(&gone), "no_match");
        let found = rpc_at(search_call(4, "joltik_nimbus"), temp.path());
        assert_hit_path_set(&tool_body(&found), &["a.rs"]);
    }

    // Phase DELETE: bare delete moves no discriminant; refresh stats (0,1,0),
    // count-1, epoch advances; deleted symbol misses, kept symbol hits.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("keep.rs"), "fn alpha_marker() {}\n").unwrap();
        std::fs::write(temp.path().join("drop.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        index_tree(temp.path());

        let fresh = status_body(temp.path());
        assert_eq!(fresh["file_count"], 2, "{fresh:#}");
        let gen_fresh = fresh["writer_generation"].as_u64().unwrap();
        assert_ne!(gen_fresh, 0);
        let before = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
        assert_hit_path_set(&tool_body(&before), &["drop.rs"]);

        std::fs::remove_file(temp.path().join("drop.rs")).unwrap();
        let stale = status_body(temp.path());
        assert_eq!(stale["file_count"], 2, "{stale:#}");
        assert_eq!(
            stale["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "bare delete moves no epoch"
        );

        let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);
        let stats = tool_body(&reindex);
        assert_eq!(stats["files_removed"], 1, "{stats:#}");
        assert_eq!(stats["files_indexed"], 0, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");

        let after = status_body(temp.path());
        assert_eq!(after["file_count"], 1, "{after:#}");
        let gen_after = after["writer_generation"].as_u64().unwrap();
        assert_ne!(gen_after, 0);
        assert_ne!(gen_after, gen_fresh, "refresh must advertise");
        let gone = rpc_at(search_call(3, "zebroid_quixotic"), temp.path());
        assert_tool_success(&gone);
        assert_miss_envelope(&tool_body(&gone), "no_match");
        let kept = rpc_at(search_call(4, "alpha_marker"), temp.path());
        assert_tool_success(&kept);
        assert_hit_envelope(&tool_body(&kept));
    }

    // Phase RENAME: bare rename moves no discriminant; refresh stats (1,1,0),
    // count+symbols stable, epoch advances.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("old.rs"), "fn i2_renstat_sym() {}\n").unwrap();
        index_tree(temp.path());

        let fresh = status_body(temp.path());
        assert_eq!(fresh["file_count"], 1, "{fresh:#}");
        let symbols = fresh["symbol_count"].as_u64().unwrap();
        assert!(symbols >= 1, "{fresh:#}");
        let gen_fresh = fresh["writer_generation"].as_u64().unwrap();
        assert_ne!(gen_fresh, 0);

        std::fs::rename(temp.path().join("old.rs"), temp.path().join("new.rs")).unwrap();
        let stale = status_body(temp.path());
        assert_eq!(stale["file_count"], 1, "{stale:#}");
        assert_eq!(
            stale["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "bare rename moves no epoch"
        );

        let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);
        let stats = tool_body(&reindex);
        assert_eq!(stats["files_removed"], 1, "{stats:#}");
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");

        let after = status_body(temp.path());
        assert_eq!(after["file_count"], 1, "{after:#}");
        assert_eq!(after["symbol_count"], symbols, "{after:#}");
        assert_ne!(
            after["writer_generation"].as_u64().unwrap(),
            gen_fresh,
            "refresh must advertise"
        );
    }
}

/// INTENT (M3-multi): multi/mixed single-refresh exact sets + stats: one
/// refresh absorbs several deltas with exact per-path hit sets and exact
/// refresh stats.
/// KILLS: multi-add cross-contamination + mixed-delta misattribution mutants.
/// ABSORBS: add_multiple_delta_single_refresh_exact_hit_sets,
/// mixed_add_delete_delta_single_refresh_exact_hit_sets.
#[test]
fn m3_multi_and_mixed_single_refresh() {
    // Phase MULTI-ADD: one refresh absorbs two adds with exact per-path sets,
    // stats (2,0,0).
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("base.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(temp.path());

        std::fs::write(temp.path().join("c.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        std::fs::write(temp.path().join("d.rs"), "fn womble_frascati() {}\n").unwrap();
        let unseen_c = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
        assert_miss_envelope(&tool_body(&unseen_c), "no_match");
        let unseen_d = rpc_at(search_call(2, "womble_frascati"), temp.path());
        assert_miss_envelope(&tool_body(&unseen_d), "no_match");

        let reindex = rpc_at(tool_call(3, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);
        let stats = tool_body(&reindex);
        assert_eq!(stats["files_indexed"], 2, "{stats:#}");
        assert_eq!(stats["files_removed"], 0, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");
        assert_eq!(status_body(temp.path())["file_count"], 3);

        let hit_c = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
        assert_hit_path_set(&tool_body(&hit_c), &["c.rs"]);
        let hit_d = rpc_at(search_call(5, "womble_frascati"), temp.path());
        assert_hit_path_set(&tool_body(&hit_d), &["d.rs"]);
        let hit_base = rpc_at(search_call(6, "alpha_marker"), temp.path());
        assert_hit_path_set(&tool_body(&hit_base), &["base.rs"]);
    }

    // Phase MIXED add+delete: one refresh, stats (1,1,0), exact
    // miss/hit/kept sets.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn joltik_nimbus() {}\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn quarry_sphinx() {}\n").unwrap();
        index_tree(temp.path());
        assert_eq!(status_body(temp.path())["file_count"], 2);

        std::fs::remove_file(temp.path().join("a.rs")).unwrap();
        std::fs::write(temp.path().join("c.rs"), "fn vortex_elm() {}\n").unwrap();

        let reindex = rpc_at(tool_call(1, "index_repo", json!({})), temp.path());
        assert_tool_success(&reindex);
        let stats = tool_body(&reindex);
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_removed"], 1, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");
        assert_eq!(status_body(temp.path())["file_count"], 2);

        let gone = rpc_at(search_call(2, "joltik_nimbus"), temp.path());
        assert_tool_success(&gone);
        assert_miss_envelope(&tool_body(&gone), "no_match");
        let added = rpc_at(search_call(3, "vortex_elm"), temp.path());
        assert_tool_success(&added);
        assert_hit_path_set(&tool_body(&added), &["c.rs"]);
        let kept = rpc_at(search_call(4, "quarry_sphinx"), temp.path());
        assert_tool_success(&kept);
        assert_hit_path_set(&tool_body(&kept), &["b.rs"]);
    }
}

/// INTENT (M4-parity): incremental ≡ fresh ≡ force byte/count agreement:
/// post-reindex in-session answers agree with a fresh process, incremental
/// refresh matches a fresh build, and force rebuilds preserve answers.
/// KILLS: session-vs-process divergence + delta-history-leak (incremental≠fresh
/// rows/counts) + force-rebuild content + force-vs-incremental divergence mutants.
/// ABSORBS: session_and_fresh_process_agree_after_reindex,
/// incremental_refresh_matches_fresh_build_after_modify,
/// incremental_refresh_matches_fresh_build_after_mixed_deltas,
/// force_rebuild_preserves_search_bytes_and_counts,
/// force_refresh_from_stale_matches_incremental_refresh.
#[test]
fn m4_incremental_fresh_force_parity() {
    // Phase SESSION-VS-PROCESS: post-reindex in-session answers agree
    // byte-identically with a fresh process.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(temp.path());

        let mut session = Session::spawn(temp.path());
        let warm = session.call(
            "keyword_search",
            json!({"query": "alpha_marker", "limit": 8, "resend_seen": true}),
        );
        assert_hit_envelope(&tool_body(&warm));
        std::fs::write(
            temp.path().join("lib.rs"),
            "fn zebroid_quixotic() {}\n",
        )
        .unwrap();
        let reindex = session.call("index_repo", json!({}));
        assert_tool_success(&reindex);
        let in_session = session.call(
            "keyword_search",
            json!({"query": "zebroid_quixotic", "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&in_session);
        let in_session_text = tool_text(&in_session).to_owned();
        session.finish();

        let fresh = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
        assert_tool_success(&fresh);
        assert_eq!(tool_text(&fresh), in_session_text);
    }

    // Phase INCREMENTAL-VS-FRESH after modify: byte-identical hit+miss with
    // equal counts.
    {
        let inc = tempfile::tempdir().unwrap();
        std::fs::write(inc.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(inc.path());
        std::fs::write(inc.path().join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        refresh(inc.path());

        let fresh = tempfile::tempdir().unwrap();
        std::fs::write(fresh.path().join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        index_tree(fresh.path());

        let hit_inc = search_text(inc.path(), "zebroid_quixotic");
        let hit_fresh = search_text(fresh.path(), "zebroid_quixotic");
        assert_hit_envelope(&serde_json::from_str(&hit_inc).unwrap());
        assert_eq!(hit_inc, hit_fresh, "incremental vs fresh hit bytes");

        let miss_inc = rpc_at(search_call(1, "alpha_marker"), inc.path());
        let miss_fresh = rpc_at(search_call(1, "alpha_marker"), fresh.path());
        assert_miss_envelope(&tool_body(&miss_inc), "no_match");
        assert_eq!(
            tool_text(&miss_inc),
            tool_text(&miss_fresh),
            "incremental vs fresh miss bytes"
        );
        assert_same_counts(&status_body(inc.path()), &status_body(fresh.path()));
    }

    // Phase INCREMENTAL-VS-FRESH after mixed modify+delete+add.
    {
        let inc = tempfile::tempdir().unwrap();
        std::fs::write(inc.path().join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
        std::fs::write(inc.path().join("b.rs"), "fn joltik_nimbus() {}\n").unwrap();
        index_tree(inc.path());
        std::fs::write(
            inc.path().join("a.rs"),
            "fn quarry_sphinx() {}\nfn vortex_elm() {}\n",
        )
        .unwrap();
        std::fs::remove_file(inc.path().join("b.rs")).unwrap();
        std::fs::write(inc.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
        refresh(inc.path());

        let fresh = tempfile::tempdir().unwrap();
        std::fs::write(
            fresh.path().join("a.rs"),
            "fn quarry_sphinx() {}\nfn vortex_elm() {}\n",
        )
        .unwrap();
        std::fs::write(fresh.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
        index_tree(fresh.path());

        assert_eq!(
            search_text(inc.path(), "vortex_elm"),
            search_text(fresh.path(), "vortex_elm"),
            "modified-symbol hit bytes"
        );
        assert_eq!(
            search_text(inc.path(), "blip_candle"),
            search_text(fresh.path(), "blip_candle"),
            "added-symbol hit bytes"
        );
        let miss_inc = rpc_at(search_call(1, "joltik_nimbus"), inc.path());
        let miss_fresh = rpc_at(search_call(1, "joltik_nimbus"), fresh.path());
        assert_miss_envelope(&tool_body(&miss_inc), "no_match");
        assert_eq!(tool_text(&miss_inc), tool_text(&miss_fresh));
        let status_inc = status_body(inc.path());
        let status_fresh = status_body(fresh.path());
        assert_eq!(status_inc["file_count"], 2, "{status_inc:#}");
        assert_same_counts(&status_inc, &status_fresh);
    }

    // Phase FORCE-PRESERVES: a full rebuild over the same tree preserves
    // search bytes and counts exactly.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn froth_gazebo() {}\n").unwrap();
        index_tree(temp.path());
        std::fs::write(temp.path().join("a.rs"), "fn plumb_kiosk() {}\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn snipe_tundra() {}\n").unwrap();

        refresh(temp.path());
        let hit_before = search_text(temp.path(), "plumb_kiosk");
        assert_hit_envelope(&serde_json::from_str(&hit_before).unwrap());
        let counts_before = status_body(temp.path());
        assert_nonzero_generation(&counts_before);

        let stats = force_refresh(temp.path());
        assert_eq!(stats["files_failed"], 0, "{stats:#}");
        let hit_after = search_text(temp.path(), "plumb_kiosk");
        assert_eq!(hit_before, hit_after, "force rebuild must preserve hit bytes");
        let miss_after = rpc_at(search_call(1, "froth_gazebo"), temp.path());
        assert_miss_envelope(&tool_body(&miss_after), "no_match");
        let counts_after = status_body(temp.path());
        assert_nonzero_generation(&counts_after);
        assert_same_counts(&counts_before, &counts_after);
    }

    // Phase FORCE-VS-INCREMENTAL from identical stale trees.
    {
        let make_v1 = |dir: &Path| {
            std::fs::write(dir.join("a.rs"), "fn alpha_marker() {}\n").unwrap();
            std::fs::write(dir.join("b.rs"), "fn womble_frascati() {}\n").unwrap();
            index_tree(dir);
            std::fs::write(dir.join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
            std::fs::remove_file(dir.join("b.rs")).unwrap();
            std::fs::write(dir.join("c.rs"), "fn joltik_nimbus() {}\n").unwrap();
        };
        let inc = tempfile::tempdir().unwrap();
        make_v1(inc.path());
        let forced = tempfile::tempdir().unwrap();
        make_v1(forced.path());

        refresh(inc.path());
        force_refresh(forced.path());

        assert_eq!(
            search_text(inc.path(), "zebroid_quixotic"),
            search_text(forced.path(), "zebroid_quixotic"),
            "incremental vs force hit bytes"
        );
        assert_eq!(
            search_text(inc.path(), "joltik_nimbus"),
            search_text(forced.path(), "joltik_nimbus"),
            "incremental vs force added-symbol hit bytes"
        );
        let miss_inc = rpc_at(search_call(1, "womble_frascati"), inc.path());
        assert_miss_envelope(&tool_body(&miss_inc), "no_match");
        let miss_forced = rpc_at(search_call(1, "womble_frascati"), forced.path());
        assert_eq!(tool_text(&miss_inc), tool_text(&miss_forced));
        assert_same_counts(&status_body(inc.path()), &status_body(forced.path()));
    }
}

/// INTENT (M5-order): delta-order/interleave independence: the same final tree
/// reached via different delta orders converges to byte-identical search
/// responses and equal counts after one refresh each.
/// KILLS: order-dependence mutant (add order, mixed add/delete order,
/// modify+add interleave).
/// ABSORBS: add_order_independent_under_single_refresh,
/// mixed_add_delete_order_independent, modify_add_interleave_order_independent.
#[test]
fn m5_delta_order_independence() {
    // Phase ADD-ORDER: same two adds in opposite order.
    {
        let first = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("base.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(first.path());
        let second = tempfile::tempdir().unwrap();
        std::fs::write(second.path().join("base.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(second.path());

        std::fs::write(first.path().join("c.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        std::fs::write(first.path().join("d.rs"), "fn womble_frascati() {}\n").unwrap();
        std::fs::write(second.path().join("d.rs"), "fn womble_frascati() {}\n").unwrap();
        std::fs::write(second.path().join("c.rs"), "fn zebroid_quixotic() {}\n").unwrap();

        refresh(first.path());
        refresh(second.path());

        assert_eq!(
            search_text(first.path(), "zebroid_quixotic"),
            search_text(second.path(), "zebroid_quixotic"),
            "add order must not affect hit bytes"
        );
        assert_eq!(
            search_text(first.path(), "womble_frascati"),
            search_text(second.path(), "womble_frascati"),
            "add order must not affect hit bytes"
        );
        let a = status_body(first.path());
        let b = status_body(second.path());
        assert_eq!(a["file_count"], 3, "{a:#}");
        assert_same_counts(&a, &b);
    }

    // Phase MIXED-ORDER: same delete+add in opposite order.
    {
        let setup = |dir: &Path| {
            std::fs::write(dir.join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
            std::fs::write(dir.join("b.rs"), "fn vortex_elm() {}\n").unwrap();
            index_tree(dir);
        };
        let first = tempfile::tempdir().unwrap();
        setup(first.path());
        let second = tempfile::tempdir().unwrap();
        setup(second.path());

        std::fs::remove_file(first.path().join("a.rs")).unwrap();
        std::fs::write(first.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
        std::fs::write(second.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
        std::fs::remove_file(second.path().join("a.rs")).unwrap();

        refresh(first.path());
        refresh(second.path());

        assert_eq!(
            search_text(first.path(), "blip_candle"),
            search_text(second.path(), "blip_candle"),
            "mixed order must not affect hit bytes"
        );
        assert_eq!(
            search_text(first.path(), "vortex_elm"),
            search_text(second.path(), "vortex_elm"),
            "kept symbol must be order-independent"
        );
        let miss_first = rpc_at(search_call(1, "quarry_sphinx"), first.path());
        assert_miss_envelope(&tool_body(&miss_first), "no_match");
        let miss_second = rpc_at(search_call(1, "quarry_sphinx"), second.path());
        assert_eq!(tool_text(&miss_first), tool_text(&miss_second));
        let a = status_body(first.path());
        let b = status_body(second.path());
        assert_eq!(a["file_count"], 2, "{a:#}");
        assert_same_counts(&a, &b);
    }

    // Phase INTERLEAVE-ORDER: same modify+add in opposite interleave order.
    {
        let setup = |dir: &Path| {
            std::fs::write(dir.join("a.rs"), "fn froth_gazebo() {}\n").unwrap();
            index_tree(dir);
        };
        let first = tempfile::tempdir().unwrap();
        setup(first.path());
        let second = tempfile::tempdir().unwrap();
        setup(second.path());

        std::fs::write(first.path().join("a.rs"), "fn plumb_kiosk() {}\n").unwrap();
        std::fs::write(first.path().join("b.rs"), "fn snipe_tundra() {}\n").unwrap();
        std::fs::write(second.path().join("b.rs"), "fn snipe_tundra() {}\n").unwrap();
        std::fs::write(second.path().join("a.rs"), "fn plumb_kiosk() {}\n").unwrap();

        refresh(first.path());
        refresh(second.path());

        assert_eq!(
            search_text(first.path(), "plumb_kiosk"),
            search_text(second.path(), "plumb_kiosk"),
            "interleave order must not affect hit bytes"
        );
        assert_eq!(
            search_text(first.path(), "snipe_tundra"),
            search_text(second.path(), "snipe_tundra"),
            "interleave order must not affect hit bytes"
        );
        let miss_first = rpc_at(search_call(1, "froth_gazebo"), first.path());
        assert_miss_envelope(&tool_body(&miss_first), "no_match");
        let miss_second = rpc_at(search_call(1, "froth_gazebo"), second.path());
        assert_eq!(tool_text(&miss_first), tool_text(&miss_second));
        assert_same_counts(&status_body(first.path()), &status_body(second.path()));
    }
}

/// INTENT (M6-idempotent): redundant-refresh noop + status convergence: a
/// second refresh with no tree change leaves bytes/counts identical with
/// zero-mutation stats; repeated status is byte-stable; counts converge.
/// KILLS: redundant-refresh-content + redundant-refresh-stats (nonzero stats on
/// noop) + nondeterministic-status-serialization + non-convergence mutants.
/// ABSORBS: refresh_twice_search_bytes_identical (TAUTOLOGY-RISK tail assert —
/// a self-compare of search_text — DROPPED; the hit+miss-bytes-identical
/// intent still merges), second_refresh_without_changes_is_content_noop,
/// index_status_byte_stable_across_repeated_calls,
/// index_status_counts_converge_across_repeated_refresh.
#[test]
fn m6_redundant_refresh_idempotent() {
    // Phase REDUNDANT-BYTES: second refresh with no tree change leaves hit+miss
    // bytes identical. (The absorbed test's tail assert compared search_text to
    // itself — a tautology — and is dropped here; the intent still merges.)
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(temp.path());
        std::fs::write(temp.path().join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn womble_frascati() {}\n").unwrap();

        refresh(temp.path());
        let hit_once = search_text(temp.path(), "zebroid_quixotic");
        assert_hit_envelope(&serde_json::from_str(&hit_once).unwrap());
        let miss_once = search_text(temp.path(), "alpha_marker");
        assert_miss_envelope(&serde_json::from_str(&miss_once).unwrap(), "no_match");

        refresh(temp.path());
        assert_eq!(
            search_text(temp.path(), "zebroid_quixotic"),
            hit_once,
            "refresh twice must equal refresh once"
        );
        assert_eq!(
            search_text(temp.path(), "alpha_marker"),
            miss_once,
            "miss bytes must survive a redundant refresh"
        );
    }

    // Phase REDUNDANT-STATS: first refresh absorbs the delta (exactly the two
    // touched files); second refresh mutates nothing and counts converge.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn i3_noop_old() {}\n").unwrap();
        index_tree(temp.path());
        std::fs::write(temp.path().join("a.rs"), "fn i3_noop_new() {}\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn i3_noop_add() {}\n").unwrap();

        let first = refresh(temp.path());
        assert_eq!(first["files_indexed"], 2, "{first:#}");
        assert_eq!(first["files_removed"], 0, "{first:#}");
        assert_eq!(first["files_failed"], 0, "{first:#}");
        let counts_before = status_body(temp.path());
        assert_nonzero_generation(&counts_before);

        let second = refresh(temp.path());
        assert_eq!(second["files_indexed"], 0, "{second:#}");
        assert_eq!(second["files_removed"], 0, "{second:#}");
        assert_eq!(second["files_failed"], 0, "{second:#}");
        let counts_after = status_body(temp.path());
        assert_nonzero_generation(&counts_after);
        assert_same_counts(&counts_before, &counts_after);
    }

    // Phase STATUS-BYTE-STABLE: no refresh between calls, status bytes identical.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn i3_stable_one() {}\n").unwrap();
        std::fs::write(temp.path().join("b.rs"), "fn i3_stable_two() {}\n").unwrap();
        index_tree(temp.path());

        let first = status_text(temp.path());
        let second = status_text(temp.path());
        let third = status_text(temp.path());
        assert_eq!(first, second, "repeated status must be byte-stable");
        assert_eq!(first, third, "repeated status must be byte-stable");
        let body: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(body["file_count"], 2, "{body:#}");
        assert_nonzero_generation(&body);
    }

    // Phase CONVERGE: three refreshes over one delta converge counts+bytes at
    // once and stay converged; only the liveness stamp may advance.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn i3_conv_old() {}\n").unwrap();
        index_tree(temp.path());
        std::fs::write(temp.path().join("a.rs"), "fn i3_conv_new() {}\n").unwrap();

        refresh(temp.path());
        let counts_first = status_body(temp.path());
        let hit_first = search_text(temp.path(), "i3_conv_new");
        refresh(temp.path());
        let counts_second = status_body(temp.path());
        let hit_second = search_text(temp.path(), "i3_conv_new");
        refresh(temp.path());
        let counts_third = status_body(temp.path());
        let hit_third = search_text(temp.path(), "i3_conv_new");

        assert_nonzero_generation(&counts_first);
        assert_nonzero_generation(&counts_second);
        assert_nonzero_generation(&counts_third);
        assert_same_counts(&counts_first, &counts_second);
        assert_same_counts(&counts_first, &counts_third);
        assert_eq!(hit_first, hit_second, "hits must converge across refreshes");
        assert_eq!(hit_first, hit_third, "hits must converge across refreshes");
        assert_hit_envelope(&serde_json::from_str(&hit_first).unwrap());
    }
}
