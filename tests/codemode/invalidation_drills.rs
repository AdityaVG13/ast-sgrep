//! Invalidation end-to-end drill suite for `ast-sgrep-codemode`.
//!
//! Consolidates the I4 drills (`invalidation_pass4.rs`) into the codemode-only
//! adapter plus the chained multi-change KEEP. Catalog:
//! `tests/catalog/invalidation-codemode.md` (M-drill row + the
//! `drill_chained_multi_change` KEEP).
//!
//! Discipline (inherited): each drill runs SERVE → CHANGE → DETECT → REFRESH
//! → SERVE through the session's own tools; discriminants via `matches!` /
//! counts / bytes of normalized hit sets only.
//!
//! Mirror rule: the add/modify/delete/rename change→detect→refresh→serve arc
//! matrix is owned by MCP `m7_full_arcs_per_delta_class`
//! (`tests/mcp/invalidation_drills.rs`, the wire-truth surface). This file
//! pins only what MCP cannot: the in-session `edit`-tool path and the session
//! force-rebuild path.

use ast_sgrep_codemode::CodeModeSession;
use ast_sgrep_testkit::{
    HitKey, find_limit8 as find, search_limit8 as search, seeded_py_repo as setup,
    status_file_count as file_count, status_writer_generation as writer_generation,
    targeted_refresh as refresh, write_py,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";
const GAMMA: &str = "snorkel_gamma_unique";
const DELTA: &str = "snorkel_delta_unique";

// Drill arcs ride the shared codemode-invalidation harness from testkit
// (seeded repo, limit-8 session/read wrappers, targeted refresh, status
// probes, hit projections — aliased to the file's vocabulary); the `defs`
// read wrapper below is the only file-local surface.

// WHY area-local: `defs` read wrapper pinning limit 8; only this suite reads
// through `defs` — single-suite helper.
fn defs(session: &mut CodeModeSession, symbol: &str) -> Value {
    session
        .call("defs", json!({"symbol": symbol, "limit": 8}))
        .expect("defs")
}

// WHY area-local: `HitKey`-backed basename projections for drill assertions;
// only this suite compares cross-tempdir hit sets by basename — single-suite helpers.
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

/// INTENT (adapter): codemode-only drill legs — the in-session `edit` inline
/// refresh loop and the `force` rebuild absorbing unrefreshed deltas — with
/// exact multi-surface serve and an epoch move per leg. The
/// add/modify/delete/rename change→detect→refresh→serve arc matrix is owned
/// by MCP `m7_full_arcs_per_delta_class` (wire truth) and is not re-pinned
/// here.
/// KILLS: edit-loop-break, rebuild-loop-break mutants.
/// ABSORBS: drill_edit_tool_change_refresh_serve,
/// drill_force_rebuild_change_detect_refresh_serve (codemode-only legs of the
/// former full-mirror m_drill; the add/modify/delete/rename legs mirror MCP
/// m7 and were deleted, not moved).
#[test]
fn m_drill_codemode_only_edit_rebuild_adapter() {
    // LEG 1 — session-edit loop (codemode-only `edit`-tool path): inline
    // refresh with an epoch move, exact all-surface swap.
    {
        let (_root, _index, mut session) = setup();
        assert_eq!(
            hit_name_set(&find(&mut session, ALPHA)),
            BTreeSet::from(["alpha.py".to_string()])
        );
        assert!(hit_name_set(&defs(&mut session, BETA)).is_empty());
        let gen_before = writer_generation(&mut session);

        let edited = session
            .call(
                "edit",
                json!({"path": "alpha.py", "oldText": ALPHA, "newText": BETA}),
            )
            .expect("edit");
        assert_eq!(edited["ok"], true);
        assert_eq!(edited["changed"], 1);
        assert_ne!(writer_generation(&mut session), gen_before);

        assert_eq!(hit_bytes(&find(&mut session, BETA)), b"alpha.py".as_slice());
        assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
        assert_eq!(
            hit_name_set(&search(&mut session, format!("word:{BETA}"))),
            BTreeSet::from(["alpha.py".to_string()])
        );
        assert!(hit_name_set(&search(&mut session, format!("word:{ALPHA}"))).is_empty());
        assert_eq!(
            hit_name_set(&defs(&mut session, BETA)),
            BTreeSet::from(["alpha.py".to_string()])
        );
        assert!(hit_name_set(&defs(&mut session, ALPHA)).is_empty());
        assert_eq!(file_count(&mut session), 1);
    }
    // LEG 2 — force rebuild (session force-rebuild path) absorbs unrefreshed
    // modify+add into the exact final multi-surface state with census 2.
    {
        let (root, _index, mut session) = setup();
        assert_eq!(
            hit_name_set(&find(&mut session, ALPHA)),
            BTreeSet::from(["alpha.py".to_string()])
        );
        let gen_before = writer_generation(&mut session);

        write_py(root.path(), "alpha.py", GAMMA);
        write_py(root.path(), "beta.py", BETA);

        assert_eq!(writer_generation(&mut session), gen_before);
        assert_eq!(
            hit_bytes(&find(&mut session, ALPHA)),
            b"alpha.py".as_slice()
        );
        assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
        assert!(hit_name_set(&find(&mut session, GAMMA)).is_empty());

        let rebuilt = session
            .call("index_repo", json!({"force": true}))
            .expect("force rebuild");
        assert_eq!(rebuilt["ok"], true);
        assert_eq!(rebuilt["force"], true);
        assert_ne!(writer_generation(&mut session), gen_before);

        assert_eq!(
            hit_bytes(&find(&mut session, GAMMA)),
            b"alpha.py".as_slice()
        );
        assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
        assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
        assert_eq!(
            hit_name_set(&search(&mut session, format!("word:{BETA}"))),
            BTreeSet::from(["beta.py".to_string()])
        );
        assert_eq!(
            hit_name_set(&defs(&mut session, GAMMA)),
            BTreeSet::from(["alpha.py".to_string()])
        );
        assert_eq!(
            hit_name_set(&defs(&mut session, BETA)),
            BTreeSet::from(["beta.py".to_string()])
        );
        assert_eq!(file_count(&mut session), 2);
    }
}

/// INTENT: four-link add→modify→delete→rename chain with stale-detect plus
/// epoch-move per link and exact final cross-tool serve.
/// KILLS: multi-link-interaction/sequence mutants.
/// ABSORBS: none (KEEP — no matrix siblings).
#[test]
fn drill_chained_multi_change_add_modify_delete_rename() {
    let (root, _index, mut session) = setup();

    // LINK 1 — ADD: baseline serves only alpha; add beta; refresh; serve both.
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    let gen0 = writer_generation(&mut session);
    write_py(root.path(), "beta.py", BETA);
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    let link1 = refresh(&mut session, &["beta.py"]);
    assert_eq!(link1["ok"], true);
    assert_ne!(writer_generation(&mut session), gen0);
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
    assert_eq!(
        hit_bytes(&find(&mut session, ALPHA)),
        b"alpha.py".as_slice()
    );
    assert_eq!(file_count(&mut session), 2);

    // LINK 2 — MODIFY: rewrite alpha.py; stale serves old; refresh; serve swap.
    let gen1 = writer_generation(&mut session);
    write_py(root.path(), "alpha.py", GAMMA);
    assert_eq!(
        hit_bytes(&find(&mut session, ALPHA)),
        b"alpha.py".as_slice()
    );
    let link2 = refresh(&mut session, &["alpha.py"]);
    assert_eq!(link2["ok"], true);
    assert_ne!(writer_generation(&mut session), gen1);
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"alpha.py".as_slice()
    );
    assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
    assert_eq!(file_count(&mut session), 2);

    // LINK 3 — DELETE: remove beta.py; stale serves deleted; refresh; evicted.
    let gen2 = writer_generation(&mut session);
    fs::remove_file(root.path().join("beta.py")).expect("delete beta");
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
    let link3 = refresh(&mut session, &["beta.py"]);
    assert_eq!(link3["stats"]["files_removed"], 1);
    assert_ne!(writer_generation(&mut session), gen2);
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"alpha.py".as_slice()
    );
    assert_eq!(file_count(&mut session), 1);

    // LINK 4 — RENAME: move alpha.py to delta.py; stale serves old path;
    // refresh; token moved exactly.
    let gen3 = writer_generation(&mut session);
    fs::rename(root.path().join("alpha.py"), root.path().join("delta.py")).expect("rename fixture");
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"alpha.py".as_slice()
    );
    let link4 = refresh(&mut session, &["alpha.py", "delta.py"]);
    assert_eq!(link4["ok"], true);
    assert_eq!(link4["stats"]["files_removed"], 1);
    assert_eq!(link4["stats"]["files_indexed"], 1);
    assert_ne!(writer_generation(&mut session), gen3);
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"delta.py".as_slice()
    );
    assert_eq!(
        hit_name_set(&find(&mut session, GAMMA)),
        BTreeSet::from(["delta.py".to_string()])
    );

    // FINAL SERVE: exact end state — only delta.py/GAMMA resolves, via every
    // read tool; all other tokens are gone.
    assert_eq!(
        hit_name_set(&search(&mut session, format!("word:{GAMMA}"))),
        BTreeSet::from(["delta.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&defs(&mut session, GAMMA)),
        BTreeSet::from(["delta.py".to_string()])
    );
    for token in [ALPHA, BETA, DELTA] {
        assert!(hit_name_set(&find(&mut session, token)).is_empty());
        assert!(hit_name_set(&defs(&mut session, token)).is_empty());
    }
    assert_eq!(file_count(&mut session), 1);
}
