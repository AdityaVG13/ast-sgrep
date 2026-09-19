//! Invalidation rebuild-parity relation suite for `ast-sgrep-codemode`.
//!
//! Consolidates the I3 relation tests (`invalidation_pass3.rs`) into
//! per-relation matrices. Catalog: `tests/catalog/invalidation-codemode.md`
//! (M-parity / M-order / M-idem / M-conv rows).
//!
//! Discipline (inherited): hit files compared by basename so sessions on
//! distinct temp roots are comparable; bytes of normalized hit sets for exact
//! parity; no error-message text matched.

use ast_sgrep_testkit::{
    find_limit8 as find, hit_bytes, hit_count, hit_name_set, search_limit8 as search,
    seeded_py_repo as setup, session_at_limit8 as session_for, status_file_count as file_count,
    targeted_refresh as refresh, write_py,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use tempfile::TempDir;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";
const GAMMA: &str = "snorkel_gamma_unique";

// Parity phases ride the shared codemode-invalidation harness from testkit
// (seeded repo, limit-8 session/read wrappers, targeted refresh, census
// probe, hit projections — aliased to the file's vocabulary) with no
// file-local surface.

/// INTENT: refresh-path agreement — targeted refresh ≡ force rebuild ≡ fresh
/// index of the same final state agree on find/search/defs bytes, counts, and
/// census.
/// KILLS: targeted-vs-rebuild-divergence, incremental-divergence,
/// defs-or-census-divergence mutants.
/// ABSORBS: targeted_refresh_matches_force_rebuild_find,
/// incremental_vs_fresh_index_search_parity,
/// incremental_vs_fresh_index_defs_and_census_parity.
#[test]
fn m_parity_targeted_rebuild_fresh() {
    // LEG 1 — targeted refresh vs force rebuild of the same final state.
    {
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

        assert_eq!(
            hit_bytes(&find(&mut session_a, BETA)),
            hit_bytes(&find(&mut session_b, BETA))
        );
        assert_eq!(
            hit_count(&find(&mut session_a, BETA)),
            hit_count(&find(&mut session_b, BETA))
        );
        assert!(hit_name_set(&find(&mut session_a, ALPHA)).is_empty());
        assert!(hit_name_set(&find(&mut session_b, ALPHA)).is_empty());
        assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
    }
    // LEG 2 — incremental (add + modify + targeted refreshes) vs a fresh
    // index of the identical final tree: hybrid search + find agree exactly.
    {
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
    // LEG 3 — incremental vs fresh through defs + census.
    {
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
            assert_eq!(
                hit_bytes(&defs_a),
                hit_bytes(&defs_b),
                "defs parity for {symbol}"
            );
            assert_eq!(hit_count(&defs_a), hit_count(&defs_b));
        }
        assert_eq!(file_count(&mut session_a), 2);
        assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
    }
}

/// INTENT: order independence — multi-path refresh in opposite orders, delta
/// application in opposite orders, and rename old+new refresh in opposite
/// orders all converge on identical results and census.
/// KILLS: path-order-dependent, delta-order-dependent, rename-order-dependent
/// mutants.
/// ABSORBS: refresh_path_order_does_not_change_results,
/// delta_application_order_does_not_change_results,
/// rename_refresh_path_order_converges.
#[test]
fn m_order_independence() {
    // LEG 1 — multi-path refresh in opposite orders converges.
    {
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
    // LEG 2 — same final tree via different delta sequences
    // (modify-then-add vs add-then-modify) serves identical results.
    {
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
    // LEG 3 — rename old+new refresh in opposite orders moves the token
    // identically with no duplication.
    {
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
        assert_eq!(
            hit_name_set(&moved_a),
            BTreeSet::from(["beta.py".to_string()])
        );
        assert_eq!(
            hit_name_set(&moved_b),
            BTreeSet::from(["beta.py".to_string()])
        );
        assert_eq!(file_count(&mut session_a), 1);
        assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
    }
}

/// INTENT: repeat-refresh idempotence — repeating a multi-path refresh leaves
/// reads and census fixed, and refreshing an already-evicted path keeps reads
/// empty and the census at 0.
/// KILLS: repeat-mutates, repeat-delete-churn mutants.
/// ABSORBS: repeated_targeted_refresh_is_idempotent_for_reads,
/// repeated_delete_refresh_is_idempotent.
#[test]
fn m_idem_repeat_refresh() {
    // LEG 1 — repeating the same multi-path refresh fixes reads and census.
    {
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
    // LEG 2 — refreshing an already-evicted path keeps reads empty, census 0.
    {
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
}

/// INTENT: cycle convergence — a session that churned (transient add+delete)
/// converges to one that never saw the transient file, and an add/delete
/// cycle returns reads and census to baseline.
/// KILLS: transient-residue, cycle-residue mutants.
/// ABSORBS: transient_add_delete_cycle_is_unobservable_in_final_state,
/// add_then_delete_cycle_returns_to_baseline_reads.
#[test]
fn m_conv_cycle_convergence() {
    // LEG 1 — churned session converges to one that never saw the transient.
    {
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
    // LEG 2 — an add/delete cycle returns reads and census to baseline.
    {
        let (_root, _index, mut session) = setup();
        let baseline_find = hit_bytes(&find(&mut session, ALPHA));
        let baseline_search = hit_bytes(&search(&mut session, format!("word:{ALPHA}")));
        let baseline_census = file_count(&mut session);
        assert!(matches!(
            session.peek_cached_search(&json!({"query": format!("word:{ALPHA}"), "limit": 8})),
            Some(_)
        ));

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
}
