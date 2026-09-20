//! Invalidation freshness matrix suite for `ast-sgrep-codemode`.
//!
//! Consolidates the I1 freshness-contract tests (`invalidation_pass1.rs`) and
//! absorbs the render-cache legs of I2/I4 into per-matrix tests. Catalog:
//! `tests/catalog/invalidation-codemode.md` (M-stale / M-render / M-writer /
//! M-pure rows plus the `index_repo_arg_conflicts` KEEP).
//!
//! Discipline (inherited): `CallError` discriminants via `matches!`, never
//! Display text; cache probes via `peek_cached_search`; epoch via
//! `read_writer_generation`; no timing asserts.

use ast_sgrep_codemode::{CallError, CodeModeSession};
use ast_sgrep_core::read_writer_generation;
use ast_sgrep_testkit::{
    external_reindex, hit_bytes, hit_file_set, hit_files, hit_name_set, hits_file,
    index_dir_db_path as index_db_path, seeded_py_repo as setup, targeted_refresh as refresh,
    write_py,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";

// Freshness phases ride the shared codemode-invalidation harness from testkit
// (seeded repo, limit-8 session, targeted refresh, hit projections, peer
// writer — aliased to the file's vocabulary); the direct on-disk epoch probe
// below is the only file-local surface.

// WHY area-local: direct on-disk epoch probe bypassing `index_status`; only
// this suite probes the epoch off-disk — single-suite helper.
fn generation(session: &CodeModeSession) -> u64 {
    let config = session.config();
    read_writer_generation(&config.root, config.index_path.as_deref())
}

/// INTENT: pinned-stale-until-stamp contract — bare disk edits are invisible
/// (epoch frozen, stale ALPHA served, BETA missing); an out-of-band Indexer
/// write stamps a new epoch; post-stamp search reopens and serves fresh hits;
/// `index_status` tracks the live epoch across the bump.
/// KILLS: auto-refresh-on-read, never-reopen-cached-Searcher,
/// no-stamp-on-external-write, cached-or-stale-status-epoch mutants.
/// ABSORBS: bare_file_change_without_reindex_serves_stale_hits,
/// external_indexer_write_bumps_writer_generation,
/// search_after_external_reindex_serves_fresh_hits,
/// index_status_reports_current_writer_generation.
#[test]
fn m_stale_pinned_until_stamp() {
    // LEG 1 — bare edit without a writer: epoch frozen, stale served, BETA
    // invisible (the stale-until-stamp contract).
    {
        let (root, _index, mut session) = setup();
        let before = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("find alpha");
        assert!(hits_file(&before, "alpha.py"), "{before}");

        let stamp = generation(&session);
        write_py(root.path(), "alpha.py", BETA);

        assert_eq!(generation(&session), stamp);
        let stale = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("stale find alpha");
        assert!(hits_file(&stale, "alpha.py"), "{stale}");
        let missing = session
            .call("find", json!({"query": BETA, "limit": 8}))
            .expect("unindexed find beta");
        assert!(hit_files(&missing).is_empty(), "{missing}");
    }
    // LEG 2 — durable out-of-band Indexer write stamps a new epoch.
    {
        let (root, index_dir, session) = setup();
        let before = generation(&session);
        write_py(root.path(), "alpha.py", BETA);
        external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");
        let after = generation(&session);
        assert_ne!(
            before, after,
            "durable external write must stamp a new epoch"
        );
    }
    // LEG 3 — post-stamp search reopens: BETA served, ALPHA evicted.
    {
        let (root, index_dir, mut session) = setup();
        let warm = session
            .call(
                "search",
                json!({"query": format!("word:{ALPHA}"), "limit": 8}),
            )
            .expect("warm search");
        assert!(hits_file(&warm, "alpha.py"), "{warm}");

        write_py(root.path(), "alpha.py", BETA);
        external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");

        let fresh = session
            .call(
                "search",
                json!({"query": format!("word:{BETA}"), "limit": 8}),
            )
            .expect("fresh search");
        assert!(hits_file(&fresh, "alpha.py"), "{fresh}");
        let gone = session
            .call(
                "search",
                json!({"query": format!("word:{ALPHA}"), "limit": 8}),
            )
            .expect("evicted search");
        assert!(hit_files(&gone).is_empty(), "{gone}");
    }
    // LEG 4 — `index_status` writer_generation tracks the live epoch.
    {
        let (root, index_dir, mut session) = setup();
        let status_before = session.call("index_status", json!({})).expect("status");
        assert_eq!(
            status_before["writer_generation"].as_u64(),
            Some(generation(&session)),
            "{status_before}"
        );

        write_py(root.path(), "alpha.py", BETA);
        external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");

        let status_after = session.call("index_status", json!({})).expect("status");
        assert_eq!(
            status_after["writer_generation"].as_u64(),
            Some(generation(&session)),
            "{status_after}"
        );
        assert_ne!(
            status_before["writer_generation"], status_after["writer_generation"],
            "status must track the live epoch"
        );
    }
}

/// INTENT: render-cache invalidation across triggers — a warm cache answers
/// None once any writer moves the epoch (external write, delete-refresh,
/// modify-refresh), is never served stale, and repopulates under the new
/// epoch on the next fresh search.
/// KILLS: serve-cache-regardless-of-epoch, render-survives-delete,
/// render-loop-break mutants.
/// ABSORBS: stale_render_cache_is_never_served_after_writer_change,
/// delta_delete_refresh_drops_stale_render_and_repopulates_fresh,
/// drill_render_cache_freshness_across_refresh.
#[test]
fn m_render_never_serve_stale() {
    // LEG 1 — external-write trigger: peek None after the stamp moves, Some
    // again once a fresh search repopulates.
    {
        let (root, index_dir, mut session) = setup();
        let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
        session.call("search", args.clone()).expect("warm search");
        assert!(session.peek_cached_search(&args).is_some());

        write_py(root.path(), "alpha.py", BETA);
        external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");

        assert!(session.peek_cached_search(&args).is_none());
        session.call("search", args.clone()).expect("reopen search");
        assert!(session.peek_cached_search(&args).is_some());
    }
    // LEG 2 — delete-refresh trigger: warm render drops, post-delete search
    // serves freshly computed emptiness, re-add repopulates.
    {
        let (root, _index, mut session) = setup();
        let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
        let warm = session.call("search", args.clone()).expect("warm search");
        assert!(hits_file(&warm, "alpha.py"), "{warm}");
        assert!(session.peek_cached_search(&args).is_some());

        fs::remove_file(root.path().join("alpha.py")).expect("delete fixture");
        refresh(&mut session, &["alpha.py"]);
        assert!(session.peek_cached_search(&args).is_none());
        let fresh = session
            .call("search", args.clone())
            .expect("post-delete search");
        assert!(hit_file_set(&fresh).is_empty(), "{fresh}");

        write_py(root.path(), "alpha.py", ALPHA);
        refresh(&mut session, &["alpha.py"]);
        assert!(session.peek_cached_search(&args).is_none());
        let revived = session
            .call("search", args.clone())
            .expect("post-add search");
        assert!(hits_file(&revived, "alpha.py"), "{revived}");
        assert!(session.peek_cached_search(&args).is_some());
    }
    // LEG 3 — modify-refresh trigger: cache answers stale pre-refresh, drops
    // on refresh, repopulates with freshly computed emptiness.
    {
        let (root, _index, mut session) = setup();
        let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
        let warm = session.call("search", args.clone()).expect("warm search");
        assert_eq!(
            hit_name_set(&warm),
            BTreeSet::from(["alpha.py".to_string()])
        );
        assert!(session.peek_cached_search(&args).is_some());

        write_py(root.path(), "alpha.py", BETA);
        assert!(session.peek_cached_search(&args).is_some());

        refresh(&mut session, &["alpha.py"]);
        assert!(session.peek_cached_search(&args).is_none());

        let fresh = session
            .call("search", args.clone())
            .expect("post-drill search");
        assert!(hit_name_set(&fresh).is_empty());
        assert_eq!(hit_bytes(&fresh), b"".as_slice());
        assert!(session.peek_cached_search(&args).is_some());

        let beta_args = json!({"query": format!("word:{BETA}"), "limit": 8});
        let beta = session
            .call("search", beta_args.clone())
            .expect("beta search");
        assert_eq!(hit_bytes(&beta), b"alpha.py".as_slice());
        assert!(session.peek_cached_search(&beta_args).is_some());
    }
}

/// INTENT: writer refresh paths — targeted `paths` refresh reports targeted
/// shape plus stats and serves the new token; force rebuild reports full
/// shape and swaps old for new; session `edit` mutates disk and reindexes so
/// reads swap inline.
/// KILLS: targeted-refresh-skipped, force-flag-ignored, edit-without-reindex
/// mutants.
/// ABSORBS: index_repo_targeted_refresh_returns_fresh_results,
/// index_repo_force_rebuild_refreshes_and_reports_full_shape,
/// edit_tool_reindexes_touched_paths.
#[test]
fn m_writer_refresh_paths() {
    // LEG 1 — targeted refresh: targeted shape + stats, new token served.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "alpha.py", BETA);
        let refreshed = session
            .call("index_repo", json!({"paths": ["alpha.py"]}))
            .expect("targeted refresh");
        assert_eq!(refreshed["ok"], true);
        assert_eq!(refreshed["targeted"], true);
        assert_eq!(refreshed["path_count"], 1);
        assert_eq!(refreshed["stats"]["files_indexed"], 1);

        let fresh = session
            .call("find", json!({"query": BETA, "limit": 8}))
            .expect("find beta");
        assert!(hits_file(&fresh, "alpha.py"), "{fresh}");
    }
    // LEG 2 — force rebuild: full shape, old token swapped for new.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "alpha.py", BETA);
        let rebuilt = session
            .call("index_repo", json!({"force": true}))
            .expect("force rebuild");
        assert_eq!(rebuilt["ok"], true);
        assert_eq!(rebuilt["force"], true);
        assert_eq!(rebuilt["targeted"], false);

        let fresh = session
            .call("find", json!({"query": BETA, "limit": 8}))
            .expect("find beta");
        assert!(hits_file(&fresh, "alpha.py"), "{fresh}");
        let gone = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("find alpha");
        assert!(hit_files(&gone).is_empty(), "{gone}");
    }
    // LEG 3 — session edit: disk mutated + touched paths reindexed inline.
    {
        let (_root, _index, mut session) = setup();
        let edited = session
            .call(
                "edit",
                json!({"path": "alpha.py", "oldText": ALPHA, "newText": BETA}),
            )
            .expect("edit");
        assert_eq!(edited["ok"], true);
        assert_eq!(edited["changed"], 1);

        let fresh = session
            .call("find", json!({"query": BETA, "limit": 8}))
            .expect("find beta");
        assert!(hits_file(&fresh, "alpha.py"), "{fresh}");
        let gone = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("find alpha");
        assert!(hit_files(&gone).is_empty(), "{gone}");
    }
}

/// INTENT: non-writer stability — a zero-change edit reports `changed: 0` and
/// pure tools (`catalog_*`, `filter_hits`, `select`) leave the epoch and the
/// warm render cache untouched.
/// KILLS: noop-bumps-epoch, noop-drops-cache, pure-tool-invalidates mutants.
/// ABSORBS: noop_edit_leaves_generation_and_cache_untouched,
/// pure_tools_neither_bump_generation_nor_drop_cache.
#[test]
fn m_pure_leaves_epoch_and_cache() {
    // LEG 1 — no-op edit: zero writes, no reindex, warm state survives.
    {
        let (_root, _index, mut session) = setup();
        let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
        session.call("search", args.clone()).expect("warm search");
        assert!(session.peek_cached_search(&args).is_some());
        let stamp = generation(&session);

        let edited = session
            .call(
                "edit",
                json!({"path": "alpha.py", "oldText": "return 1", "newText": "return 1"}),
            )
            .expect("noop edit");
        assert_eq!(edited["ok"], true);
        assert_eq!(edited["changed"], 0);
        assert_eq!(generation(&session), stamp);
        assert!(session.peek_cached_search(&args).is_some());
    }
    // LEG 2 — pure tools never touch the index or the warm cache.
    {
        let (_root, _index, mut session) = setup();
        let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
        let first = session.call("search", args.clone()).expect("warm search");
        assert!(session.peek_cached_search(&args).is_some());
        let stamp = generation(&session);

        let found = session
            .call("catalog_search", json!({"query": "index"}))
            .expect("catalog_search");
        assert!(found["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()));
        session
            .call("catalog_describe", json!({"name": "search"}))
            .expect("catalog_describe");
        session
            .call("filter_hits", json!({"hits": first, "limit": 4}))
            .expect("filter_hits");
        session
            .call(
                "select",
                json!({"value": {"a": 1, "b": 2}, "fields": ["a"]}),
            )
            .expect("select");

        assert_eq!(generation(&session), stamp);
        assert!(session.peek_cached_search(&args).is_some());
    }
}

/// INTENT: force+paths, empty paths, and traversal fail as CallError::Other
/// with epoch and warm cache untouched.
/// KILLS: conflict-silent-ok, wrong-discriminant, failure-drops-cache mutants.
/// ABSORBS: none (KEEP — no matrix siblings).
#[test]
fn index_repo_arg_conflicts_fail_with_other_discriminant() {
    let (_root, _index, mut session) = setup();
    let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
    session.call("search", args.clone()).expect("warm search");

    // Argument validation failures surface as `Other`, never as a
    // silent no-op, and leave the warm session untouched (no stamp bump,
    // no cache drop) since no writer ran.
    let stamp = generation(&session);
    let conflict = session
        .call("index_repo", json!({"force": true, "paths": ["alpha.py"]}))
        .expect_err("force+paths must conflict");
    assert!(matches!(conflict, CallError::Other(_)), "{conflict:?}");
    let empty = session
        .call("index_repo", json!({"paths": []}))
        .expect_err("empty paths must fail");
    assert!(matches!(empty, CallError::Other(_)), "{empty:?}");
    let traversal = session
        .call("index_repo", json!({"paths": ["../escape.py"]}))
        .expect_err("traversal must fail");
    assert!(matches!(traversal, CallError::Other(_)), "{traversal:?}");

    assert_eq!(generation(&session), stamp);
    assert!(session.peek_cached_search(&args).is_some());
}
