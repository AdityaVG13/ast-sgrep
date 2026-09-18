//! I1 invalidation-contract oracles for `ast-sgrep-codemode`.
//!
//! Pins the session freshness contract:
//!
//! * A warm session serves its pinned index until a writer stamps a new
//!   `writer_generation` epoch. Bare repo edits with no reindex are invisible.
//! * Any durable index write (own `index_repo`/`edit`, or an external
//!   `Indexer` on the same DB) invalidates the cached `Searcher` and the
//!   render cache; the next search reopens and serves fresh hits.
//! * Stale renders are never served: `peek_cached_search` answers `None`
//!   once the stamp moves, until a fresh search repopulates the cache.
//! * Pure tools (`catalog_*`, `filter_hits`, `select`) and no-op edits never
//!   bump the stamp or drop the warm cache.
//!
//! Discriminants are asserted with `matches!` / typed JSON accessors only;
//! no error-message text is matched.

use ast_sgrep_codemode::{CallError, CodeModeSession, SessionConfig};
use ast_sgrep_core::{read_writer_generation, IndexOptions, Indexer};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";
const GAMMA: &str = "snorkel_gamma_unique";

fn write_fixture(root: &Path, token: &str) {
    fs::write(root.join("alpha.py"), format!("def {token}():\n    return 1\n"))
        .expect("write fixture");
}

fn session_for(root: &Path, index_db: &Path) -> CodeModeSession {
    CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(index_db.to_path_buf()),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    })
}

/// Fresh root + index DB, one file indexed, session pointed at both.
fn setup(token: &str) -> (TempDir, TempDir, CodeModeSession) {
    let root = TempDir::new().expect("root");
    let index_dir = TempDir::new().expect("index dir");
    write_fixture(root.path(), token);
    let mut session = session_for(root.path(), &index_dir.path().join("index.db"));
    let indexed = session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");
    assert_eq!(indexed["ok"], true);
    (root, index_dir, session)
}

fn generation(session: &CodeModeSession) -> u64 {
    let config = session.config();
    read_writer_generation(&config.root, config.index_path.as_deref())
}

fn hit_files(value: &Value) -> Vec<String> {
    value["hits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|hit| {
            hit.get("file")
                .and_then(|file| file.as_str())
                .map(str::to_string)
        })
        .collect()
}

fn hits_file(value: &Value, name: &str) -> bool {
    hit_files(value).iter().any(|file| file.ends_with(name))
}

/// Simulate an out-of-band writer: a second `Indexer` handle mutating the
/// same DB, as a watcher/CLI peer would. Roots are canonicalized so the
/// incremental path survives the `strip_prefix` gate in `update_paths`.
fn external_reindex(root: &Path, index_db: &Path, rel: &str) {
    let canon = root.canonicalize().expect("canonical root");
    let mut indexer = Indexer::new(IndexOptions {
        root: canon.clone(),
        index_path: Some(index_db.to_path_buf()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("external indexer");
    indexer
        .update_paths(&[canon.join(rel)])
        .expect("external update");
    indexer.flush_deferred_rebuilds().expect("external flush");
}

fn index_db_path(index_dir: &TempDir) -> std::path::PathBuf {
    index_dir.path().join("index.db")
}

#[test]
fn bare_file_change_without_reindex_serves_stale_hits() {
    let (root, _index, mut session) = setup(ALPHA);
    let before = session
        .call("find", json!({"query": ALPHA, "limit": 8}))
        .expect("find alpha");
    assert!(hits_file(&before, "alpha.py"), "{before}");

    let stamp = generation(&session);
    write_fixture(root.path(), BETA);

    // No writer ran, so the epoch is unchanged and the pinned index still
    // answers: stale reads are the contract until a reindex stamps.
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

#[test]
fn external_indexer_write_bumps_writer_generation() {
    let (root, index_dir, session) = setup(ALPHA);
    let before = generation(&session);
    write_fixture(root.path(), BETA);
    external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");
    let after = generation(&session);
    assert_ne!(before, after, "durable external write must stamp a new epoch");
}

#[test]
fn search_after_external_reindex_serves_fresh_hits() {
    let (root, index_dir, mut session) = setup(ALPHA);
    let warm = session
        .call("search", json!({"query": format!("word:{ALPHA}"), "limit": 8}))
        .expect("warm search");
    assert!(hits_file(&warm, "alpha.py"), "{warm}");

    write_fixture(root.path(), BETA);
    external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");

    // The stamp moved, so the session must reopen and serve post-write hits.
    let fresh = session
        .call("search", json!({"query": format!("word:{BETA}"), "limit": 8}))
        .expect("fresh search");
    assert!(hits_file(&fresh, "alpha.py"), "{fresh}");
    let gone = session
        .call("search", json!({"query": format!("word:{ALPHA}"), "limit": 8}))
        .expect("evicted search");
    assert!(hit_files(&gone).is_empty(), "{gone}");
}

#[test]
fn stale_render_cache_is_never_served_after_writer_change() {
    let (root, index_dir, mut session) = setup(ALPHA);
    let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
    session.call("search", args.clone()).expect("warm search");
    assert!(matches!(session.peek_cached_search(&args), Some(_)));

    write_fixture(root.path(), BETA);
    external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");

    // Stale epoch: the cached render must not be served.
    assert!(matches!(session.peek_cached_search(&args), None));
    // A fresh search repopulates the cache under the new epoch.
    session.call("search", args.clone()).expect("reopen search");
    assert!(matches!(session.peek_cached_search(&args), Some(_)));
}

#[test]
fn index_repo_targeted_refresh_returns_fresh_results() {
    let (root, _index, mut session) = setup(ALPHA);
    write_fixture(root.path(), BETA);
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

#[test]
fn index_repo_force_rebuild_refreshes_and_reports_full_shape() {
    let (root, _index, mut session) = setup(ALPHA);
    write_fixture(root.path(), BETA);
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

#[test]
fn index_repo_arg_conflicts_fail_with_other_discriminant() {
    let (_root, _index, mut session) = setup(ALPHA);
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
    assert!(matches!(session.peek_cached_search(&args), Some(_)));
}

#[test]
fn edit_tool_reindexes_touched_paths() {
    let (_root, _index, mut session) = setup(ALPHA);
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

#[test]
fn noop_edit_leaves_generation_and_cache_untouched() {
    let (_root, _index, mut session) = setup(ALPHA);
    let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
    session.call("search", args.clone()).expect("warm search");
    assert!(matches!(session.peek_cached_search(&args), Some(_)));
    let stamp = generation(&session);

    // Identical old/new text: zero writes, so no targeted reindex and no
    // invalidation of the warm Searcher or render cache.
    let edited = session
        .call(
            "edit",
            json!({"path": "alpha.py", "oldText": "return 1", "newText": "return 1"}),
        )
        .expect("noop edit");
    assert_eq!(edited["ok"], true);
    assert_eq!(edited["changed"], 0);
    assert_eq!(generation(&session), stamp);
    assert!(matches!(session.peek_cached_search(&args), Some(_)));
}

#[test]
fn catalog_is_stable_across_index_writes() {
    let (root, index_dir, mut session) = setup(ALPHA);
    let names_before: Vec<&str> = {
        let mut names: Vec<&str> = ast_sgrep_codemode::catalog_search("")
            .iter()
            .map(|def| def.name)
            .collect();
        names.sort_unstable();
        names
    };
    assert!(names_before.contains(&"index_repo"));

    // Drive every writer path: external peer, targeted refresh, full
    // rebuild, and in-session edit. The tool catalog is static and must
    // be identical afterwards.
    write_fixture(root.path(), BETA);
    external_reindex(root.path(), &index_db_path(&index_dir), "alpha.py");
    session
        .call("index_repo", json!({"paths": ["alpha.py"]}))
        .expect("targeted");
    session
        .call("index_repo", json!({"force": true}))
        .expect("rebuild");
    session
        .call(
            "edit",
            json!({"path": "alpha.py", "oldText": BETA, "newText": GAMMA}),
        )
        .expect("edit");

    let names_after: Vec<&str> = {
        let mut names: Vec<&str> = ast_sgrep_codemode::catalog_search("")
            .iter()
            .map(|def| def.name)
            .collect();
        names.sort_unstable();
        names
    };
    assert_eq!(names_before, names_after);

    let repo = ast_sgrep_codemode::catalog_describe("index_repo");
    assert!(matches!(repo, Some(_)));
    assert_eq!(repo.map(|def| def.read_only), Some(false));
    let search = ast_sgrep_codemode::catalog_describe("search");
    assert!(matches!(search, Some(_)));
    assert_eq!(search.map(|def| def.read_only), Some(true));

    let unknown = session
        .call("catalog_describe", json!({"name": "no_such_tool"}))
        .expect_err("unknown catalog entry must fail");
    assert!(matches!(unknown, CallError::InvalidArgs(_)), "{unknown:?}");
}

#[test]
fn pure_tools_neither_bump_generation_nor_drop_cache() {
    let (_root, _index, mut session) = setup(ALPHA);
    let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
    let first = session.call("search", args.clone()).expect("warm search");
    assert!(matches!(session.peek_cached_search(&args), Some(_)));
    let stamp = generation(&session);

    // Catalog and transform tools never touch the index: the stamp must
    // not move and the warm render cache must survive them.
    let found = session
        .call("catalog_search", json!({"query": "index"}))
        .expect("catalog_search");
    assert!(found["tools"].as_array().is_some_and(|tools| !tools.is_empty()));
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
    assert!(matches!(session.peek_cached_search(&args), Some(_)));
}

#[test]
fn index_status_reports_current_writer_generation() {
    let (root, index_dir, mut session) = setup(ALPHA);
    let status_before = session.call("index_status", json!({})).expect("status");
    assert_eq!(
        status_before["writer_generation"].as_u64(),
        Some(generation(&session)),
        "{status_before}"
    );

    write_fixture(root.path(), BETA);
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
