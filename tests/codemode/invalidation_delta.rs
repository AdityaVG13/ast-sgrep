//! Invalidation per-surface delta-matrix suite for `ast-sgrep-codemode`.
//!
//! Consolidates the I2 delta-class tests (`invalidation_pass2.rs`) into a
//! thin session adapter plus the catalog stability/convergence matrix (I1+I3
//! legs). Catalog: `tests/catalog/invalidation-codemode.md` (M-find /
//! M-search / M-census rows, now adapter phases, plus M-defs / M-catalog
//! rows).
//!
//! Mirror rule: the add/modify/delete/rename change→detect→refresh→serve arc
//! matrix is owned by MCP `m1_search_hit_path_sets_across_delta_classes`
//! (`tests/mcp/invalidation_matrix.rs`) and
//! `m7_full_arcs_per_delta_class` (`tests/mcp/invalidation_drills.rs`), with
//! rename chains on `drill_rename_chain_single_refresh_serves_final_path`
//! (wire-truth surfaces). This file pins only session agreement — one compact
//! phase per surface — plus the `defs` delta matrix (MCP-unpinned) and the
//! catalog matrix (session-surface flags and discriminants, never collapsed).
//!
//! Discipline (inherited): discriminants via `matches!` / typed JSON accessors
//! only; the shared DELETE-leg source (`delta_delete_find_and_search_...`)
//! contributes its find leg to M-find and its search leg to M-search; the
//! trailing catalog leg pins static-registry vs live-session-serve name
//! agreement (two genuinely different paths) and appears only inside M-catalog.

use ast_sgrep_codemode::CodeModeSession;
use ast_sgrep_testkit as testkit;
use ast_sgrep_testkit::{
    HitKey, external_reindex, index_dir_db_path as index_db_path, seeded_py_repo as setup,
    session_at_limit8 as session_for, status_file_count as file_count,
    status_writer_generation as writer_generation, targeted_refresh as refresh, write_py,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use tempfile::TempDir;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";
const GAMMA: &str = "snorkel_gamma_unique";
const NEWDEF: &str = "snorkel_newdef_unique";
const OLDDEF: &str = "snorkel_olddef_unique";

// Delta phases ride the shared codemode-invalidation harness from testkit
// (seeded repo, limit-8 session, targeted refresh, status probes, hit
// projections, peer writer — aliased to the file's vocabulary); the catalog
// projection below is the only file-local surface.
// (The census probe now returns `u64` with no narrowing cast: strictly
// stronger than the former `usize` form, same assertion shape.)

// WHY area-local: catalog tool-name projection for convergence assertions;
// only this suite reads the tool catalog — single-suite helper.
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

// WHY area-local: `HitKey`-backed file projections for delta assertions;
// only this suite reads find/defs file sets — single-suite helpers.
fn hit_file_set(value: &Value) -> BTreeSet<String> {
    HitKey::file_keys(value).into_iter().collect()
}

fn hits_file(value: &Value, name: &str) -> bool {
    HitKey::has_file(value, name)
}

/// INTENT (adapter): session agreement with the MCP-owned delta matrix — one
/// compact phase per surface (find modify-swap, find+search delete-evict,
/// search add, census move with epoch bump). The full add/modify/delete/
/// rename matrices live on MCP M1 + m7 (wire truth); the rename arc
/// additionally lives in the session chained drill KEEP
/// (`drill_chained_multi_change_add_modify_delete_rename`, LINK 4) and is not
/// re-pinned here.
/// KILLS: add-invisible, spillover, stale-linger, new-missing, delete-linger,
/// census-not-tracked mutants (representative phases; full matrices on MCP).
/// ABSORBS: delta_add_find_surfaces_new_file_token_only,
/// delta_modify_find_swaps_token_exactly,
/// delta_delete_find_and_search_evict_token (both legs),
/// delta_rename_refresh_moves_hits_to_new_path,
/// delta_add_search_word_query_surfaces_new_file,
/// delta_catalog_file_count_tracks_add_and_delete.
#[test]
fn m_delta_adapter_agrees_with_mcp_matrix() {
    // PHASE 1 (MODIFY swap on find): new token present AND old token gone —
    // exactly the swap, with index stats shape.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "alpha.py", BETA);
        let refreshed = refresh(&mut session, &["alpha.py"]);
        assert_eq!(refreshed["ok"], true);
        assert_eq!(refreshed["stats"]["files_indexed"], 1);
        assert_eq!(refreshed["stats"]["files_removed"], 0);

        let fresh = session
            .call("find", json!({"query": BETA, "limit": 8}))
            .expect("find beta");
        assert!(hits_file(&fresh, "alpha.py"), "{fresh}");
        let gone = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("find alpha");
        assert!(hit_file_set(&gone).is_empty(), "{gone}");
    }
    // PHASE 2 (DELETE evict on find+search): no surface may serve the removed
    // token, with removal stats shape.
    {
        let (root, _index, mut session) = setup();
        fs::remove_file(root.path().join("alpha.py")).expect("delete fixture");
        let refreshed = refresh(&mut session, &["alpha.py"]);
        assert_eq!(refreshed["ok"], true);
        assert_eq!(refreshed["stats"]["files_removed"], 1);
        assert_eq!(refreshed["stats"]["files_indexed"], 0);

        let via_find = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("find alpha");
        assert!(hit_file_set(&via_find).is_empty(), "{via_find}");
        let via_search = session
            .call(
                "search",
                json!({"query": format!("word:{ALPHA}"), "limit": 8}),
            )
            .expect("search alpha");
        assert!(hit_file_set(&via_search).is_empty(), "{via_search}");
    }
    // PHASE 3 (ADD on search + sibling intact on find): the new token
    // resolves to exactly the new file with no spillover.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "beta.py", BETA);
        refresh(&mut session, &["beta.py"]);

        let found = session
            .call(
                "search",
                json!({"query": format!("word:{BETA}"), "limit": 8}),
            )
            .expect("search beta");
        assert!(hits_file(&found, "beta.py"), "{found}");
        assert_eq!(hit_file_set(&found).len(), 1, "{found}");
        let sibling = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("find alpha");
        assert!(hits_file(&sibling, "alpha.py"), "{sibling}");
    }
    // PHASE 4 (census move): `index_status` file_count moves 1→2→1 with an
    // epoch bump per delta, and reads agree with the census (only the
    // survivor resolves).
    {
        let (root, _index, mut session) = setup();
        assert_eq!(file_count(&mut session), 1);
        let gen_before = writer_generation(&mut session);

        write_py(root.path(), "beta.py", BETA);
        refresh(&mut session, &["beta.py"]);
        assert_eq!(file_count(&mut session), 2);
        assert_ne!(writer_generation(&mut session), gen_before);

        fs::remove_file(root.path().join("beta.py")).expect("delete fixture");
        refresh(&mut session, &["beta.py"]);
        assert_eq!(file_count(&mut session), 1);

        let survivor = session
            .call("find", json!({"query": ALPHA, "limit": 8}))
            .expect("find alpha");
        assert_eq!(hit_file_set(&survivor).len(), 1, "{survivor}");
        let evicted = session
            .call("find", json!({"query": BETA, "limit": 8}))
            .expect("find beta");
        assert!(hit_file_set(&evicted).is_empty(), "{evicted}");
    }
}

/// INTENT: per-surface `defs` delta matrix — ADD resolves the new symbol with
/// no phantom for unknown symbols; MODIFY renaming a definition moves lookup
/// to the new name only; DELETE of the defining file empties lookup.
/// KILLS: defs-misses-add, phantom-def, defs-stale-name, defs-delete-linger
/// mutants.
/// ABSORBS: delta_add_defs_lookup_finds_new_symbol,
/// delta_modify_defs_lookup_tracks_renamed_symbol,
/// delta_delete_defs_lookup_empties.
#[test]
fn m_defs_delta_matrix() {
    // LEG 1 — ADD: new definition resolves; unknown symbol resolves empty.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "newdef.py", NEWDEF);
        refresh(&mut session, &["newdef.py"]);

        let found = session
            .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
            .expect("defs newdef");
        assert!(hits_file(&found, "newdef.py"), "{found}");
        let phantom = session
            .call(
                "defs",
                json!({"symbol": "snorkel_never_defined", "limit": 8}),
            )
            .expect("defs phantom");
        assert!(hit_file_set(&phantom).is_empty(), "{phantom}");
    }
    // LEG 2 — MODIFY: renamed definition moves lookup to the new name only.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "defs.py", OLDDEF);
        refresh(&mut session, &["defs.py"]);
        let before = session
            .call("defs", json!({"symbol": OLDDEF, "limit": 8}))
            .expect("defs olddef");
        assert!(hits_file(&before, "defs.py"), "{before}");

        write_py(root.path(), "defs.py", NEWDEF);
        refresh(&mut session, &["defs.py"]);
        let fresh = session
            .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
            .expect("defs newdef");
        assert!(hits_file(&fresh, "defs.py"), "{fresh}");
        let gone = session
            .call("defs", json!({"symbol": OLDDEF, "limit": 8}))
            .expect("defs olddef");
        assert!(hit_file_set(&gone).is_empty(), "{gone}");
    }
    // LEG 3 — DELETE: removing the defining file empties lookup.
    {
        let (root, _index, mut session) = setup();
        write_py(root.path(), "doomed.py", NEWDEF);
        refresh(&mut session, &["doomed.py"]);
        let before = session
            .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
            .expect("defs newdef");
        assert!(hits_file(&before, "doomed.py"), "{before}");

        fs::remove_file(root.path().join("doomed.py")).expect("delete fixture");
        let refreshed = refresh(&mut session, &["doomed.py"]);
        assert_eq!(refreshed["stats"]["files_removed"], 1);
        let gone = session
            .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
            .expect("defs newdef");
        assert!(hit_file_set(&gone).is_empty(), "{gone}");
    }
}

/// INTENT: catalog stability across writes and convergence across histories —
/// `read_only` flags survive every writer path, unknown names fail
/// InvalidArgs, and divergent refresh histories over the same tree expose
/// identical catalogs and census.
/// KILLS: read_only-flag-flip, unknown-not-InvalidArgs,
/// session-catalog-wiring (static-vs-serve name agreement),
/// history-dependent-catalog mutants.
/// ABSORBS: catalog_is_stable_across_index_writes (static-vs-serve
/// name-agreement leg kept only here),
/// catalog_converges_across_divergent_refresh_histories.
#[test]
fn m_catalog_stable_and_convergent() {
    // LEG 1 — stability: drive every writer path (external peer, targeted
    // refresh, full rebuild, in-session edit); the static tool catalog flags
    // must be identical afterwards and unknown names must fail InvalidArgs.
    {
        let (root, index_dir, mut session) = setup();
        let names_before: Vec<&str> = {
            let mut names: Vec<&str> = ast_sgrep_codemode::catalog_search("")
                .iter()
                .map(|def| def.name)
                .collect();
            names.sort_unstable();
            names
        };
        assert!(names_before.contains(&"index_repo"));

        write_py(root.path(), "alpha.py", BETA);
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

        let repo = ast_sgrep_codemode::catalog_describe("index_repo");
        assert!(repo.is_some());
        assert_eq!(repo.map(|def| def.read_only), Some(false));
        let search = ast_sgrep_codemode::catalog_describe("search");
        assert!(search.is_some());
        assert_eq!(search.map(|def| def.read_only), Some(true));

        let unknown = session
            .call("catalog_describe", json!({"name": "no_such_tool"}))
            .expect_err("unknown catalog entry must fail");
        assert_eq!(
            testkit::call_error_discriminant(&unknown),
            "invalid_args",
            "{unknown:?}"
        );

        // TRAILING leg (static snapshot vs live session serve): the same
        // "index" query answered two genuinely different ways — direct static
        // registry call vs session tool dispatch + JSON round-trip — so a
        // broken session catalog wiring fails it. (The former
        // static-vs-static form killed nothing: both sides read the same fn
        // in the same binary.)
        let live_names = catalog_tool_names(&mut session, "index");
        let static_names: BTreeSet<String> =
            ast_sgrep_codemode::catalog_search("index")
                .into_iter()
                .map(|def| def.name.to_string())
                .collect();
        assert_eq!(live_names, static_names);
        assert!(live_names.contains("index_repo"));
    }
    // LEG 2 — convergence: wildly different refresh histories over the same
    // final tree expose identical catalogs and census.
    {
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
}
