//! MCP invalidation drills: the M7 full-arc matrix target plus the four
//! pass4 KEEP drills (multi-edit, net-zero, rename-chain, chained-four).
//!
//! Harness rides `ast-sgrep-testkit` (`LiveSession`, `index_tree`, envelope
//! builders/extractors); every live-session read uses the testkit 15s `recv`
//! bound and every process wait uses the 15s `wait_clean` bound, so a
//! regressed server fails the test instead of hanging the suite. File-local
//! helpers below carry only what testkit lacks.

use ast_sgrep_testkit::{
    assert_hit_path_set, assert_miss_envelope, index_tree, CallSession as Session,
};
use serde_json::Value;

// Drill arcs ride the shared MCP harness from testkit (the hit/miss envelope
// discriminants, the compact-path projector, and `CallSession` aliased to the
// file's `Session` vocabulary); the tool-body JSON parse below is the only
// file-local surface.

// WHY area-local: parses tool body bytes back to JSON for drill
// detect/serve phases; only this suite re-parses served bytes — single-suite
// helper.
fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("tool body JSON")
}

/// INTENT (M7-drill): full change→detect→refresh→serve arc × {add, modify,
/// delete, rename}: each arc runs end to end in one live session —
/// miss/stale-bytes detect, byte-stable status, exact refresh stats, gen
/// advance, exact fresh sets.
/// KILLS: in-session-refresh-noop + arc-phase-skip (add/modify/delete/rename
/// column) mutants.
/// ABSORBS: in_session_reindex_then_search_reflects_changes (the modify flip
/// is the MODIFY arc's serve phase), drill_add_change_detect_refresh_serve,
/// drill_modify_change_detect_refresh_serve,
/// drill_delete_change_detect_refresh_serve,
/// drill_rename_change_detect_refresh_serve. Each arc holds its own live
/// session over its own tree; the all-four-chained-in-one-session variant is
/// the KEEP drill_chained_add_modify_delete_rename_in_one_session below.
#[test]
fn m7_full_arcs_per_delta_class() {
    // Arc ADD: miss-detect, byte-stable status, stats, gen advance, exact sets.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
        index_tree(temp.path());

        let mut session = Session::spawn(temp.path());
        let keep_before = session.search_text("alpha_marker");
        assert_hit_path_set(&parse(&keep_before), &["a.rs"]);
        let status_before = session.status_text();
        let gen_before = parse(&status_before)["writer_generation"].as_u64().unwrap();
        assert_ne!(gen_before, 0);
        assert_eq!(parse(&status_before)["file_count"], 1);

        std::fs::write(temp.path().join("b.rs"), "fn zebroid_quixotic() {}\n").unwrap();

        let unseen = session.search_text("zebroid_quixotic");
        assert_miss_envelope(&parse(&unseen), "no_match");
        assert_eq!(
            session.status_text(),
            status_before,
            "bare add must move no status byte"
        );

        let stats = session.refresh();
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_removed"], 0, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");
        let status_after = parse(&session.status_text());
        assert_eq!(status_after["file_count"], 2, "{status_after:#}");
        assert_ne!(
            status_after["writer_generation"].as_u64().unwrap(),
            gen_before,
            "refresh must advertise"
        );

        let found = session.search_text("zebroid_quixotic");
        assert_hit_path_set(&parse(&found), &["b.rs"]);
        assert_eq!(
            session.search_text("zebroid_quixotic"),
            found,
            "fresh serve must be byte-stable"
        );
        assert_eq!(
            session.search_text("alpha_marker"),
            keep_before,
            "untouched symbol serve must be unchanged"
        );
        session.finish();
    }

    // Arc MODIFY: stale-bytes detect, refresh, old-miss/new-hit serve.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
        index_tree(temp.path());

        let mut session = Session::spawn(temp.path());
        let old_before = session.search_text("quarry_sphinx");
        assert_hit_path_set(&parse(&old_before), &["a.rs"]);
        let status_before = session.status_text();
        let gen_before = parse(&status_before)["writer_generation"].as_u64().unwrap();

        std::fs::write(temp.path().join("a.rs"), "fn vortex_elm() {}\n").unwrap();

        assert_eq!(
            session.search_text("quarry_sphinx"),
            old_before,
            "stale modify must serve pre-change bytes"
        );
        let unseen = session.search_text("vortex_elm");
        assert_miss_envelope(&parse(&unseen), "no_match");
        assert_eq!(
            session.status_text(),
            status_before,
            "bare modify must move no status byte"
        );

        let stats = session.refresh();
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_removed"], 0, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");
        let status_after = parse(&session.status_text());
        assert_eq!(status_after["file_count"], 1, "{status_after:#}");
        assert_ne!(
            status_after["writer_generation"].as_u64().unwrap(),
            gen_before,
            "refresh must advertise"
        );

        let gone = session.search_text("quarry_sphinx");
        assert_miss_envelope(&parse(&gone), "no_match");
        let found = session.search_text("vortex_elm");
        assert_hit_path_set(&parse(&found), &["a.rs"]);
        assert_ne!(
            found, old_before,
            "fresh bytes must differ from stale bytes"
        );
        assert_eq!(
            session.search_text("vortex_elm"),
            found,
            "fresh serve must be byte-stable"
        );
        session.finish();
    }

    // Arc DELETE: stale-bytes detect, stats (0,1,0), kept byte-identical serve.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("keep.rs"), "fn blip_candle() {}\n").unwrap();
        std::fs::write(temp.path().join("drop.rs"), "fn froth_gazebo() {}\n").unwrap();
        index_tree(temp.path());

        let mut session = Session::spawn(temp.path());
        let gone_before = session.search_text("froth_gazebo");
        assert_hit_path_set(&parse(&gone_before), &["drop.rs"]);
        let keep_before = session.search_text("blip_candle");
        assert_hit_path_set(&parse(&keep_before), &["keep.rs"]);
        let status_before = session.status_text();
        assert_eq!(parse(&status_before)["file_count"], 2);
        let gen_before = parse(&status_before)["writer_generation"].as_u64().unwrap();

        std::fs::remove_file(temp.path().join("drop.rs")).unwrap();

        assert_eq!(
            session.search_text("froth_gazebo"),
            gone_before,
            "stale delete must serve pre-change bytes"
        );
        assert_eq!(
            session.status_text(),
            status_before,
            "bare delete must move no status byte"
        );

        let stats = session.refresh();
        assert_eq!(stats["files_indexed"], 0, "{stats:#}");
        assert_eq!(stats["files_removed"], 1, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");
        let status_after = parse(&session.status_text());
        assert_eq!(status_after["file_count"], 1, "{status_after:#}");
        assert_ne!(
            status_after["writer_generation"].as_u64().unwrap(),
            gen_before,
            "refresh must advertise"
        );

        let gone = session.search_text("froth_gazebo");
        assert_miss_envelope(&parse(&gone), "no_match");
        assert_eq!(
            session.search_text("blip_candle"),
            keep_before,
            "kept symbol serve must be unchanged"
        );
        session.finish();
    }

    // Arc RENAME: old-path stale detect, stats (1,1,0), new-path serve.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("old.rs"), "fn plumb_kiosk() {}\n").unwrap();
        index_tree(temp.path());

        let mut session = Session::spawn(temp.path());
        let stale_before = session.search_text("plumb_kiosk");
        assert_hit_path_set(&parse(&stale_before), &["old.rs"]);
        let status_before = session.status_text();
        let gen_before = parse(&status_before)["writer_generation"].as_u64().unwrap();

        std::fs::rename(temp.path().join("old.rs"), temp.path().join("new.rs")).unwrap();

        let stale = session.search_text("plumb_kiosk");
        assert_eq!(stale, stale_before, "stale rename must serve old bytes");
        assert_hit_path_set(&parse(&stale), &["old.rs"]);
        assert_eq!(
            session.status_text(),
            status_before,
            "bare rename must move no status byte"
        );

        let stats = session.refresh();
        assert_eq!(stats["files_indexed"], 1, "{stats:#}");
        assert_eq!(stats["files_removed"], 1, "{stats:#}");
        assert_eq!(stats["files_failed"], 0, "{stats:#}");
        let status_after = parse(&session.status_text());
        assert_eq!(status_after["file_count"], 1, "{status_after:#}");
        assert_ne!(
            status_after["writer_generation"].as_u64().unwrap(),
            gen_before,
            "refresh must advertise"
        );

        let moved = session.search_text("plumb_kiosk");
        assert_hit_path_set(&parse(&moved), &["new.rs"]);
        assert_ne!(
            moved, stale_before,
            "fresh bytes must differ from stale bytes"
        );
        assert_eq!(
            session.search_text("plumb_kiosk"),
            moved,
            "fresh serve must be byte-stable"
        );
        session.finish();
    }
}

/// INTENT: two successive modifies: one refresh serves the final revision
/// only, intermediate never visible.
/// KILLS: intermediate-revision-serve mutant.
/// ABSORBS: none (KEEP).
#[test]
fn drill_modify_twice_single_refresh_serves_final_only() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn snipe_tundra() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = Session::spawn(temp.path());
    let v1_before = session.search_text("snipe_tundra");
    assert_hit_path_set(&parse(&v1_before), &["a.rs"]);
    let status_before = session.status_text();

    // Two successive modifies, no refresh between them.
    std::fs::write(temp.path().join("a.rs"), "fn womble_frascati() {}\n").unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn joltik_nimbus() {}\n").unwrap();

    // DETECT: stale v1 bytes still serve, neither later revision visible,
    // status byte-stable.
    assert_eq!(session.search_text("snipe_tundra"), v1_before);
    assert_miss_envelope(&parse(&session.search_text("womble_frascati")), "no_match");
    assert_miss_envelope(&parse(&session.search_text("joltik_nimbus")), "no_match");
    assert_eq!(
        session.status_text(),
        status_before,
        "bare double modify must move no status byte"
    );

    // REFRESH once absorbs both edits: exactly one file touched.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    // SERVE (fresh): only the final revision hits; both older ones miss.
    let found = session.search_text("joltik_nimbus");
    assert_hit_path_set(&parse(&found), &["a.rs"]);
    assert_miss_envelope(&parse(&session.search_text("snipe_tundra")), "no_match");
    assert_miss_envelope(&parse(&session.search_text("womble_frascati")), "no_match");
    assert_eq!(
        session.search_text("joltik_nimbus"),
        found,
        "fresh serve must be byte-stable"
    );
    session.finish();
}

/// INTENT: add-then-delete before refresh: zero-mutation stats, serve
/// identical to baseline.
/// KILLS: cancel-delta mutation mutant (nonzero stats, changed serve).
/// ABSORBS: none (KEEP).
#[test]
fn drill_add_then_delete_before_refresh_is_net_zero() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn gazebo_froth() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = Session::spawn(temp.path());
    let keep_before = session.search_text("gazebo_froth");
    assert_hit_path_set(&parse(&keep_before), &["a.rs"]);
    let status_before = parse(&session.status_text());
    assert_eq!(status_before["file_count"], 1, "{status_before:#}");

    // CHANGE then anti-change: add a file, then delete it before refresh.
    std::fs::write(temp.path().join("tmp.rs"), "fn quixotic_zebroid() {}\n").unwrap();
    std::fs::remove_file(temp.path().join("tmp.rs")).unwrap();

    // DETECT: the never-indexed symbol misses and counts are untouched.
    assert_miss_envelope(&parse(&session.search_text("quixotic_zebroid")), "no_match");
    assert_eq!(parse(&session.status_text())["file_count"], 1);

    // REFRESH: zero-mutation stats for a cancelled delta.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 0, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    // SERVE (fresh): the tree serves exactly as at baseline.
    assert_eq!(
        session.search_text("gazebo_froth"),
        keep_before,
        "cancelled delta must leave serve bytes unchanged"
    );
    assert_miss_envelope(&parse(&session.search_text("quixotic_zebroid")), "no_match");
    let status_after = parse(&session.status_text());
    assert_eq!(status_after["file_count"], 1, "{status_after:#}");
    assert_eq!(
        status_after["symbol_count"], status_before["symbol_count"],
        "cancelled delta must leave counts unchanged"
    );
    session.finish();
}

/// INTENT: rename chain (a→b→c): one refresh collapses to single remove+add
/// serving the final path.
/// KILLS: chain-collapse mutant (intermediate path served, wrong stats).
/// ABSORBS: none (KEEP).
#[test]
fn drill_rename_chain_single_refresh_serves_final_path() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn tundra_snipe() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = Session::spawn(temp.path());
    let first_before = session.search_text("tundra_snipe");
    assert_hit_path_set(&parse(&first_before), &["a.rs"]);
    let status_before = session.status_text();

    // CHANGE: rename twice (a -> b -> c) with no refresh between.
    std::fs::rename(temp.path().join("a.rs"), temp.path().join("b.rs")).unwrap();
    std::fs::rename(temp.path().join("b.rs"), temp.path().join("c.rs")).unwrap();

    // DETECT: stale serve still carries the original path, status byte-stable.
    let stale = session.search_text("tundra_snipe");
    assert_eq!(stale, first_before, "stale chain must serve original bytes");
    assert_hit_path_set(&parse(&stale), &["a.rs"]);
    assert_eq!(
        session.status_text(),
        status_before,
        "bare rename chain must move no status byte"
    );

    // REFRESH once absorbs the chain: remove + add, count stable.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    assert_eq!(parse(&session.status_text())["file_count"], 1);

    // SERVE (fresh): exactly the final path; both older paths are gone.
    let moved = session.search_text("tundra_snipe");
    assert_hit_path_set(&parse(&moved), &["c.rs"]);
    assert_eq!(
        session.search_text("tundra_snipe"),
        moved,
        "fresh serve must be byte-stable"
    );
    session.finish();
}

/// INTENT: four chained arcs in one session with per-link gen advance and
/// exact final serve.
/// KILLS: chained-link state-bleed mutant.
/// ABSORBS: none (KEEP).
#[test]
fn drill_chained_add_modify_delete_rename_in_one_session() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn marble_alpha() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn candle_blip() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = Session::spawn(temp.path());

    // Baseline serve: both symbols hit under their own paths.
    assert_hit_path_set(&parse(&session.search_text("marble_alpha")), &["a.rs"]);
    assert_hit_path_set(&parse(&session.search_text("candle_blip")), &["b.rs"]);
    assert_eq!(parse(&session.status_text())["file_count"], 2);
    let mut gen = parse(&session.status_text())["writer_generation"]
        .as_u64()
        .unwrap();

    // Link 1 -- ADD c.rs: detect (miss), refresh, serve (exact new path).
    std::fs::write(temp.path().join("c.rs"), "fn vortex_nimbus() {}\n").unwrap();
    assert_miss_envelope(&parse(&session.search_text("vortex_nimbus")), "no_match");
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 3, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    gen = status["writer_generation"].as_u64().unwrap();
    assert_hit_path_set(&parse(&session.search_text("vortex_nimbus")), &["c.rs"]);

    // Link 2 -- MODIFY a.rs (marble_alpha -> quixotic_elm): detect (stale old
    // bytes, new misses), refresh, serve (old misses, new hits a.rs).
    let a_before = session.search_text("marble_alpha");
    assert_hit_path_set(&parse(&a_before), &["a.rs"]);
    std::fs::write(temp.path().join("a.rs"), "fn quixotic_elm() {}\n").unwrap();
    assert_eq!(session.search_text("marble_alpha"), a_before);
    assert_miss_envelope(&parse(&session.search_text("quixotic_elm")), "no_match");
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 3, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    gen = status["writer_generation"].as_u64().unwrap();
    assert_miss_envelope(&parse(&session.search_text("marble_alpha")), "no_match");
    assert_hit_path_set(&parse(&session.search_text("quixotic_elm")), &["a.rs"]);

    // Link 3 -- DELETE b.rs: detect (stale bytes), refresh, serve (miss).
    let b_before = session.search_text("candle_blip");
    assert_hit_path_set(&parse(&b_before), &["b.rs"]);
    std::fs::remove_file(temp.path().join("b.rs")).unwrap();
    assert_eq!(session.search_text("candle_blip"), b_before);
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 0, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 2, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    gen = status["writer_generation"].as_u64().unwrap();
    assert_miss_envelope(&parse(&session.search_text("candle_blip")), "no_match");

    // Link 4 -- RENAME c.rs -> d.rs: detect (stale old path), refresh,
    // serve (exact new path).
    let c_before = session.search_text("vortex_nimbus");
    assert_hit_path_set(&parse(&c_before), &["c.rs"]);
    std::fs::rename(temp.path().join("c.rs"), temp.path().join("d.rs")).unwrap();
    let stale = session.search_text("vortex_nimbus");
    assert_eq!(stale, c_before);
    assert_hit_path_set(&parse(&stale), &["c.rs"]);
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 2, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    assert_hit_path_set(&parse(&session.search_text("vortex_nimbus")), &["d.rs"]);

    // Final serve: the whole chained tree answers exactly, byte-stable.
    let a2 = session.search_text("quixotic_elm");
    assert_hit_path_set(&parse(&a2), &["a.rs"]);
    assert_eq!(session.search_text("quixotic_elm"), a2);
    let c = session.search_text("vortex_nimbus");
    assert_hit_path_set(&parse(&c), &["d.rs"]);
    assert_eq!(session.search_text("vortex_nimbus"), c);
    assert_miss_envelope(&parse(&session.search_text("marble_alpha")), "no_match");
    assert_miss_envelope(&parse(&session.search_text("candle_blip")), "no_match");
    session.finish();
}
