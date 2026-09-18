//! I3 rebuild-parity metamorphic oracles for `ast-sgrep-codemode`.
//!
//! Where I1 pins the freshness *contract* and I2 proves each *delta class*,
//! I3 asserts *relations* between refresh paths through the session's own
//! tools (`index_repo`, `find`, `search`, `defs`, `index_status`, catalog):
//!
//! * REFRESH-VS-FRESH: incremental targeted refresh vs a fresh index of the
//!   same final repo state serve identical tool results.
//! * REFRESH-VS-REBUILD: targeted refresh vs `force` rebuild converge.
//! * ORDER-INDEPENDENCE: refresh path order and delta application order do
//!   not change observable results.
//! * IDEMPOTENCE: repeating a refresh leaves reads and census stable.
//! * CONVERGENCE: divergent histories ending at the same repo state (and
//!   add/delete cycles returning to baseline) converge observably.
//!
//! Discriminants are asserted with `matches!` / counts / bytes of normalized
//! hit sets only; no error-message text is matched. Hit files are compared by
//! basename so sessions on distinct temp roots are comparable.

use ast_sgrep_codemode::{CodeModeSession, SessionConfig};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";
const GAMMA: &str = "snorkel_gamma_unique";

fn write_py(root: &Path, name: &str, token: &str) {
    fs::write(
        root.join(name),
        format!("def {token}():\n    return 1\n"),
    )
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

/// Fresh root + index DB with `alpha.py` (`ALPHA`) indexed.
fn setup() -> (TempDir, TempDir, CodeModeSession) {
    let root = TempDir::new().expect("root");
    let index_dir = TempDir::new().expect("index dir");
    write_py(root.path(), "alpha.py", ALPHA);
    let mut session = session_for(root.path(), &index_dir.path().join("index.db"));
    let indexed = session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");
    assert_eq!(indexed["ok"], true);
    (root, index_dir, session)
}

/// Basename hit set: comparable across sessions rooted at distinct tempdirs.
fn hit_name_set(value: &Value) -> BTreeSet<String> {
    value["hits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|hit| {
            hit.get("file")
                .and_then(|file| file.as_str())
                .map(|file| file.rsplit('/').next().unwrap_or(file).to_string())
        })
        .collect()
}

/// Byte-stable encoding of a hit set for exact parity assertions.
fn hit_bytes(value: &Value) -> Vec<u8> {
    hit_name_set(value).into_iter().collect::<Vec<_>>().join("\n").into_bytes()
}

fn hit_count(value: &Value) -> usize {
    value["hits"].as_array().map(Vec::len).unwrap_or(0)
}

fn refresh(session: &mut CodeModeSession, paths: &[&str]) -> Value {
    session
        .call("index_repo", json!({"paths": paths}))
        .expect("targeted refresh")
}

fn find(session: &mut CodeModeSession, query: &str) -> Value {
    session
        .call("find", json!({"query": query, "limit": 8}))
        .expect("find")
}

fn search(session: &mut CodeModeSession, query: String) -> Value {
    session
        .call("search", json!({"query": query, "limit": 8}))
        .expect("search")
}

fn file_count(session: &mut CodeModeSession) -> u64 {
    session
        .call("index_status", json!({}))
        .expect("index status")["file_count"]
        .as_u64()
        .expect("file_count u64")
}

fn catalog_tool_names(session: &mut CodeModeSession, query: &str) -> BTreeSet<String> {
    session
        .call("catalog_search", json!({"query": query}))
        .expect("catalog_search")["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tool| {
            tool.get("name")
                .and_then(|name| name.as_str())
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn targeted_refresh_matches_force_rebuild_find() {
    // REFRESH-VS-REBUILD: same final repo, one side targeted, one side full.
    let (root_a, _ia, mut session_a) = setup();
    let (root_b, _ib, mut session_b) = setup();
    write_py(root_a.path(), "alpha.py", BETA);
    write_py(root_b.path(), "alpha.py", BETA);

    let targeted = refresh(&mut session_a, &["alpha.py"]);
    assert_eq!(targeted["ok"], true);
    let rebuilt = session_b
        .call("index_repo", json!({"force": true}))
        .expect("force rebuild");
    assert_eq!(rebuilt["ok"], true);

    assert_eq!(hit_bytes(&find(&mut session_a, BETA)), hit_bytes(&find(&mut session_b, BETA)));
    assert_eq!(hit_count(&find(&mut session_a, BETA)), hit_count(&find(&mut session_b, BETA)));
    assert!(hit_name_set(&find(&mut session_a, ALPHA)).is_empty());
    assert!(hit_name_set(&find(&mut session_b, ALPHA)).is_empty());
    assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
}

#[test]
fn incremental_vs_fresh_index_search_parity() {
    // REFRESH-VS-FRESH: session A reaches the final state incrementally
    // (add + modify + targeted refreshes); session B indexes the identical
    // final tree from scratch. Hybrid search must agree exactly.
    let (root_a, _ia, mut session_a) = setup();
    write_py(root_a.path(), "beta.py", BETA);
    refresh(&mut session_a, &["beta.py"]);
    write_py(root_a.path(), "alpha.py", GAMMA);
    refresh(&mut session_a, &["alpha.py"]);

    let root_b = TempDir::new().expect("root b");
    let index_b = TempDir::new().expect("index b");
    write_py(root_b.path(), "alpha.py", GAMMA);
    write_py(root_b.path(), "beta.py", BETA);
    let mut session_b = session_for(root_b.path(), &index_b.path().join("index.db"));
    let indexed = session_b
        .call("index_repo", json!({"force": false}))
        .expect("fresh index");
    assert_eq!(indexed["ok"], true);

    for token in [BETA, GAMMA] {
        let query = format!("word:{token}");
        assert_eq!(
            hit_bytes(&search(&mut session_a, query.clone())),
            hit_bytes(&search(&mut session_b, query)),
            "search parity for {token}"
        );
    }
    assert_eq!(
        hit_bytes(&find(&mut session_a, BETA)),
        hit_bytes(&find(&mut session_b, BETA))
    );
    assert!(hit_name_set(&find(&mut session_a, ALPHA)).is_empty());
    assert!(hit_name_set(&find(&mut session_b, ALPHA)).is_empty());
}

#[test]
fn incremental_vs_fresh_index_defs_and_census_parity() {
    // REFRESH-VS-FRESH through defs + census: same final tree, one side
    // incremental, one side fresh.
    let (root_a, _ia, mut session_a) = setup();
    write_py(root_a.path(), "beta.py", BETA);
    refresh(&mut session_a, &["beta.py"]);

    let root_b = TempDir::new().expect("root b");
    let index_b = TempDir::new().expect("index b");
    write_py(root_b.path(), "alpha.py", ALPHA);
    write_py(root_b.path(), "beta.py", BETA);
    let mut session_b = session_for(root_b.path(), &index_b.path().join("index.db"));
    session_b
        .call("index_repo", json!({"force": false}))
        .expect("fresh index");

    for symbol in [ALPHA, BETA] {
        let defs_a = session_a
            .call("defs", json!({"symbol": symbol, "limit": 8}))
            .expect("defs a");
        let defs_b = session_b
            .call("defs", json!({"symbol": symbol, "limit": 8}))
            .expect("defs b");
        assert_eq!(hit_bytes(&defs_a), hit_bytes(&defs_b), "defs parity for {symbol}");
        assert_eq!(hit_count(&defs_a), hit_count(&defs_b));
    }
    assert_eq!(file_count(&mut session_a), 2);
    assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
}

#[test]
fn refresh_path_order_does_not_change_results() {
    // ORDER-INDEPENDENCE: multi-path refresh in opposite orders converges.
    let (root_a, _ia, mut session_a) = setup();
    let (root_b, _ib, mut session_b) = setup();
    for (root, session, order) in [
        (&root_a, &mut session_a, ["alpha.py", "beta.py"]),
        (&root_b, &mut session_b, ["beta.py", "alpha.py"]),
    ] {
        write_py(root.path(), "alpha.py", GAMMA);
        write_py(root.path(), "beta.py", BETA);
        let refreshed = refresh(session, &order);
        assert_eq!(refreshed["ok"], true);
    }

    for token in [BETA, GAMMA] {
        assert_eq!(
            hit_bytes(&find(&mut session_a, token)),
            hit_bytes(&find(&mut session_b, token)),
            "find parity for {token}"
        );
    }
    assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
}

#[test]
fn delta_application_order_does_not_change_results() {
    // ORDER-INDEPENDENCE: same final tree via different delta sequences
    // (modify-then-add vs add-then-modify) serves identical results.
    let (root_a, _ia, mut session_a) = setup();
    write_py(root_a.path(), "alpha.py", GAMMA);
    refresh(&mut session_a, &["alpha.py"]);
    write_py(root_a.path(), "beta.py", BETA);
    refresh(&mut session_a, &["beta.py"]);

    let (root_b, _ib, mut session_b) = setup();
    write_py(root_b.path(), "beta.py", BETA);
    refresh(&mut session_b, &["beta.py"]);
    write_py(root_b.path(), "alpha.py", GAMMA);
    refresh(&mut session_b, &["alpha.py"]);

    for token in [BETA, GAMMA] {
        let query = format!("word:{token}");
        assert_eq!(
            hit_bytes(&search(&mut session_a, query.clone())),
            hit_bytes(&search(&mut session_b, query)),
            "search parity for {token}"
        );
        assert_eq!(
            hit_bytes(&find(&mut session_a, token)),
            hit_bytes(&find(&mut session_b, token)),
            "find parity for {token}"
        );
    }
    assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
}

#[test]
fn transient_add_delete_cycle_is_unobservable_in_final_state() {
    // CONVERGENCE: a session that churned (add + delete of a transient file)
    // converges to one that never saw the transient file.
    let (root_a, _ia, mut session_a) = setup();
    write_py(root_a.path(), "beta.py", BETA);
    refresh(&mut session_a, &["beta.py"]);
    write_py(root_a.path(), "alpha.py", GAMMA);
    refresh(&mut session_a, &["alpha.py"]);
    fs::remove_file(root_a.path().join("beta.py")).expect("delete transient");
    let evicted = refresh(&mut session_a, &["beta.py"]);
    assert_eq!(evicted["ok"], true);

    let (root_b, _ib, mut session_b) = setup();
    write_py(root_b.path(), "alpha.py", GAMMA);
    refresh(&mut session_b, &["alpha.py"]);

    assert_eq!(
        hit_bytes(&find(&mut session_a, GAMMA)),
        hit_bytes(&find(&mut session_b, GAMMA))
    );
    assert!(hit_name_set(&find(&mut session_a, BETA)).is_empty());
    assert!(hit_name_set(&find(&mut session_b, BETA)).is_empty());
    assert_eq!(file_count(&mut session_a), 1);
    assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
}

#[test]
fn repeated_targeted_refresh_is_idempotent_for_reads() {
    // IDEMPOTENCE: repeating the same refresh leaves reads and census fixed.
    let (root, _index, mut session) = setup();
    write_py(root.path(), "beta.py", BETA);
    write_py(root.path(), "alpha.py", GAMMA);

    let first = refresh(&mut session, &["alpha.py", "beta.py"]);
    assert_eq!(first["ok"], true);
    let baseline_beta = hit_bytes(&find(&mut session, BETA));
    let baseline_count = hit_count(&find(&mut session, BETA));
    let baseline_gamma = hit_bytes(&find(&mut session, GAMMA));
    let baseline_census = file_count(&mut session);

    for _ in 0..2 {
        let repeated = refresh(&mut session, &["alpha.py", "beta.py"]);
        assert_eq!(repeated["ok"], true);
        assert_eq!(hit_bytes(&find(&mut session, BETA)), baseline_beta);
        assert_eq!(hit_count(&find(&mut session, BETA)), baseline_count);
        assert_eq!(hit_bytes(&find(&mut session, GAMMA)), baseline_gamma);
        assert_eq!(file_count(&mut session), baseline_census);
    }
}

#[test]
fn repeated_delete_refresh_is_idempotent() {
    // IDEMPOTENCE: refreshing an already-evicted path keeps reads empty and
    // the census fixed.
    let (root, _index, mut session) = setup();
    fs::remove_file(root.path().join("alpha.py")).expect("delete fixture");
    let first = refresh(&mut session, &["alpha.py"]);
    assert_eq!(first["ok"], true);
    assert_eq!(first["stats"]["files_removed"], 1);
    assert_eq!(file_count(&mut session), 0);

    let repeated = refresh(&mut session, &["alpha.py"]);
    assert_eq!(repeated["ok"], true);
    assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
    let via_search = search(&mut session, format!("word:{ALPHA}"));
    assert!(hit_name_set(&via_search).is_empty());
    assert_eq!(file_count(&mut session), 0);
}

#[test]
fn rename_refresh_path_order_converges() {
    // ORDER-INDEPENDENCE (rename): old+new refresh in opposite orders moves
    // the token identically with no duplication.
    let (root_a, _ia, mut session_a) = setup();
    let (root_b, _ib, mut session_b) = setup();
    fs::rename(root_a.path().join("alpha.py"), root_a.path().join("beta.py"))
        .expect("rename a");
    fs::rename(root_b.path().join("alpha.py"), root_b.path().join("beta.py"))
        .expect("rename b");
    let forward = refresh(&mut session_a, &["alpha.py", "beta.py"]);
    assert_eq!(forward["ok"], true);
    let reverse = refresh(&mut session_b, &["beta.py", "alpha.py"]);
    assert_eq!(reverse["ok"], true);

    let moved_a = find(&mut session_a, ALPHA);
    let moved_b = find(&mut session_b, ALPHA);
    assert_eq!(hit_bytes(&moved_a), hit_bytes(&moved_b));
    assert_eq!(hit_name_set(&moved_a), BTreeSet::from(["beta.py".to_string()]));
    assert_eq!(hit_name_set(&moved_b), BTreeSet::from(["beta.py".to_string()]));
    assert_eq!(file_count(&mut session_a), 1);
    assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
}

#[test]
fn catalog_converges_across_divergent_refresh_histories() {
    // CONVERGENCE: two sessions with wildly different refresh histories over
    // the same final tree expose identical catalogs and census.
    let (root_a, _ia, mut session_a) = setup();
    write_py(root_a.path(), "beta.py", BETA);
    refresh(&mut session_a, &["beta.py"]);
    write_py(root_a.path(), "alpha.py", GAMMA);
    refresh(&mut session_a, &["alpha.py"]);
    refresh(&mut session_a, &["alpha.py", "beta.py"]);
    session_a
        .call("index_repo", json!({"force": true}))
        .expect("rebuild a");

    let root_b = TempDir::new().expect("root b");
    let index_b = TempDir::new().expect("index b");
    write_py(root_b.path(), "alpha.py", GAMMA);
    write_py(root_b.path(), "beta.py", BETA);
    let mut session_b = session_for(root_b.path(), &index_b.path().join("index.db"));
    session_b
        .call("index_repo", json!({"force": false}))
        .expect("fresh index b");

    assert_eq!(
        catalog_tool_names(&mut session_a, "index"),
        catalog_tool_names(&mut session_b, "index")
    );
    assert!(!catalog_tool_names(&mut session_a, "index").is_empty());
    let describe_a = session_a
        .call("catalog_describe", json!({"name": "search"}))
        .expect("describe a");
    let describe_b = session_b
        .call("catalog_describe", json!({"name": "search"}))
        .expect("describe b");
    assert_eq!(describe_a["read_only"], true);
    assert_eq!(describe_a["read_only"], describe_b["read_only"]);
    assert_eq!(describe_a["name"], describe_b["name"]);
    assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
}

#[test]
fn add_then_delete_cycle_returns_to_baseline_reads() {
    // CONVERGENCE: an add/delete cycle returns reads and census to baseline.
    let (_root, _index, mut session) = setup();
    let baseline_find = hit_bytes(&find(&mut session, ALPHA));
    let baseline_search = hit_bytes(&search(&mut session, format!("word:{ALPHA}")));
    let baseline_census = file_count(&mut session);
    assert!(matches!(session.peek_cached_search(&json!({"query": format!("word:{ALPHA}"), "limit": 8})), Some(_)));

    // Borrow the root path via config to add the transient file.
    let root = session.config().root.clone();
    write_py(&root, "beta.py", BETA);
    refresh(&mut session, &["beta.py"]);
    assert_eq!(file_count(&mut session), baseline_census + 1);
    fs::remove_file(root.join("beta.py")).expect("delete transient");
    refresh(&mut session, &["beta.py"]);

    assert_eq!(hit_bytes(&find(&mut session, ALPHA)), baseline_find);
    assert_eq!(
        hit_bytes(&search(&mut session, format!("word:{ALPHA}"))),
        baseline_search
    );
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    assert_eq!(file_count(&mut session), baseline_census);
}
