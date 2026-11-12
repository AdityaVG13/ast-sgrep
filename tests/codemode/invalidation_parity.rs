//! Invalidation rebuild-parity adapter suite for `ast-sgrep-codemode`.
//!
//! The rebuild-parity relations (targeted≡rebuild≡fresh, order independence,
//! repeat-refresh idempotence, cycle convergence) are owned by MCP M4/M5/M6
//! (`tests/mcp/invalidation_matrix.rs`, the wire-truth surface). This file
//! keeps a single thin adapter asserting the session surface agrees — one
//! compact phase per relation. Catalog:
//! `tests/catalog/invalidation-codemode.md` (M-parity / M-order / M-idem /
//! M-conv rows, now adapter phases).
//!
//! Discipline (inherited): hit files compared by basename so sessions on
//! distinct temp roots are comparable; bytes of normalized hit sets for exact
//! parity; no error-message text matched.
//!
//! Surface-specific by design: the `peek_cached_search` cache-internal probe
//! in the converge phase stays here and is not part of any collapse.

use ast_sgrep_testkit::{
    HitKey, find_limit8 as find, search_limit8 as search, seeded_py_repo as setup,
    session_at_limit8 as session_for, status_file_count as file_count,
    targeted_refresh as refresh, write_py,
};
use serde_json::{json, Value};
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

// WHY area-local: `HitKey`-backed hit projections for parity assertions;
// only this suite compares exact find/search bytes and counts across
// sessions — single-suite helpers.
fn hit_name_set(value: &Value) -> BTreeSet<String> {
    HitKey::file_keys(value)
        .into_iter()
        .map(|file| HitKey::name_key_of(&file).to_owned())
        .collect()
}

fn hit_bytes(value: &Value) -> Vec<u8> {
    hit_name_set(value)
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn hit_count(value: &Value) -> usize {
    HitKey::hit_count(value)
}

/// INTENT (adapter): codemode agreement with the MCP-owned rebuild-parity
/// relations — one compact phase per relation (targeted≡rebuild≡fresh, order
/// independence, repeat-refresh idempotence, cycle convergence) asserting
/// find/search bytes and census. The full matrices live on MCP M4/M5/M6 (wire
/// truth) and are not re-pinned here.
/// KILLS: targeted-vs-rebuild-divergence, order-dependence, repeat-mutates,
/// transient-residue mutants (representative phases; full matrices on MCP).
/// ABSORBS: m_parity_targeted_rebuild_fresh, m_order_independence,
/// m_idem_repeat_refresh, m_conv_cycle_convergence.
#[test]
fn m_parity_adapter_agrees_with_mcp_relations() {
    // PHASE 1 (M4 — targeted≡rebuild≡fresh): targeted refresh plus a force
    // rebuild of one tree agrees with a fresh index of the same tree on find
    // bytes+counts and census.
    {
        let (root_a, _ia, mut session_a) = setup();
        write_py(root_a.path(), "beta.py", BETA);
        let targeted = refresh(&mut session_a, &["beta.py"]);
        assert_eq!(targeted["ok"], true);
        let rebuilt = session_a
            .call("index_repo", json!({"force": true}))
            .expect("force rebuild");
        assert_eq!(rebuilt["ok"], true);

        let root_b = TempDir::new().expect("root b");
        let index_b = TempDir::new().expect("index b");
        write_py(root_b.path(), "alpha.py", ALPHA);
        write_py(root_b.path(), "beta.py", BETA);
        let mut session_b = session_for(root_b.path(), &index_b.path().join("index.db"));
        session_b
            .call("index_repo", json!({"force": false}))
            .expect("fresh index");

        assert_eq!(
            hit_bytes(&find(&mut session_a, BETA)),
            hit_bytes(&find(&mut session_b, BETA))
        );
        assert_eq!(
            hit_count(&find(&mut session_a, BETA)),
            hit_count(&find(&mut session_b, BETA))
        );
        assert_eq!(
            hit_bytes(&find(&mut session_a, ALPHA)),
            hit_bytes(&find(&mut session_b, ALPHA)),
            "kept-symbol parity for ALPHA"
        );
        assert_eq!(file_count(&mut session_a), 2);
        assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
    }
    // PHASE 2 (M5 — order): modify-then-add vs add-then-modify serve
    // identical find/search bytes with the same census.
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
        assert_eq!(
            hit_name_set(&find(&mut session_a, BETA)),
            BTreeSet::from(["beta.py".to_string()])
        );
        assert_eq!(file_count(&mut session_a), file_count(&mut session_b));
    }
    // PHASE 3 (M6 — idempotent repeat): repeating a multi-path refresh fixes
    // reads and census.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "beta.py", BETA);
        write_py(root.path(), "alpha.py", GAMMA);

        let first = refresh(&mut session, &["alpha.py", "beta.py"]);
        assert_eq!(first["ok"], true);
        let baseline_beta = hit_bytes(&find(&mut session, BETA));
        let baseline_count = hit_count(&find(&mut session, BETA));
        let baseline_census = file_count(&mut session);

        let repeated = refresh(&mut session, &["alpha.py", "beta.py"]);
        assert_eq!(repeated["ok"], true);
        assert_eq!(hit_bytes(&find(&mut session, BETA)), baseline_beta);
        assert_eq!(hit_count(&find(&mut session, BETA)), baseline_count);
        assert_eq!(file_count(&mut session), baseline_census);
    }
    // PHASE 4 (M6-CONVERGE + cache-internal): an add/delete cycle returns
    // reads and census to baseline. The `peek_cached_search` probe below is
    // the codemode cache-internal pin: surface-specific by design, kept here
    // and never collapsed into the MCP matrix.
    {
        let (_root, _index, mut session) = setup();
        let baseline_find = hit_bytes(&find(&mut session, ALPHA));
        let baseline_search = hit_bytes(&search(&mut session, format!("word:{ALPHA}")));
        let baseline_census = file_count(&mut session);
        assert!(session
            .peek_cached_search(&json!({"query": format!("word:{ALPHA}"), "limit": 8}))
            .is_some());

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
