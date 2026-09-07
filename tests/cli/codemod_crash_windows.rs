//! Failure-first RED tests for the codemod apply/rollback crash windows
//! (br-i04, br-1xx, br-bci, br-hbd; audit:
//! docs/validation/audits/2026-08-23-codemod-edit-path.md).
//!
//! Every fixture is deterministic: the "crash window" races are realized by
//! mutating the tree between plan_codemod and apply_codemod (the window
//! verify-once/swap-later leaves unprotected) or by reproducing the exact
//! post-crash filesystem state of a mid-swap death. The concurrent-writer
//! test synchronizes on an observable apply artifact (file 0's backup
//! sidecar appearing = staging complete) instead of sleeping.
use ast_sgrep_core::codemod::{apply_codemod, plan_codemod};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

const SOURCE: &str = "fn run() { legacy(alpha); }\nfn keep() { modern(beta); }\n";
const PATTERN: &str = "legacy($ARG)";
const REWRITE: &str = "modern($ARG)";

struct Fixture {
    _temp: TempDir,
    root: std::path::PathBuf,
}

/// Build an indexed one-file fixture and a plan that rewrites `legacy(..)`.
fn fixture_with_plan() -> (Fixture, ast_sgrep_core::codemod::CodemodPlan) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lib.rs"), SOURCE).unwrap();
    let index_path = temp.path().join("index.db");

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            "--no-embed",
            root.to_str().unwrap(),
        ])
        .status()
        .expect("run asgrep index");
    assert!(status.success(), "indexing must succeed");

    let plan = plan_codemod(&root, Some(&index_path), None, PATTERN, REWRITE).unwrap();
    assert_eq!(plan.files.len(), 1, "one matching file in fixture");
    (Fixture { _temp: temp, root }, plan)
}

/// br-hbd / F4: between plan and apply, replace the target file with an
/// IN-ROOT RELATIVE symlink to an identical-content sibling. Plan-time reads
/// are O_NOFOLLOW but apply-time verification follows final-component
/// symlinks whose destination stays inside the root, so verification passes,
/// the rename moves the symlink into the backup slot, and success cleanup
/// deletes it. Contract: the leaf must remain a symlink after apply, and the
/// sibling target must be either edited or untouched — never lost.
#[test]
#[cfg(unix)]
fn apply_refuses_when_leaf_became_symlink_between_plan_and_apply() {
    let (fx, plan) = fixture_with_plan();
    let lib = fx.root.join("src/lib.rs");
    let sibling = fx.root.join("src/shared.rs");
    fs::write(&sibling, SOURCE).unwrap();
    fs::remove_file(&lib).unwrap();
    std::os::unix::fs::symlink("shared.rs", &lib).unwrap();

    // Pre-fix this returns Ok and destroys the symlink.
    let result = apply_codemod(&plan);

    match result {
        Err(error) => {
            let text = format!("{error:#}");
            assert!(
                text.contains("symlink") || text.contains("not a regular file"),
                "refusal must name the symlink problem: {text}"
            );
        }
        Ok(applied) => {
            // If apply claims success, the edit MUST have landed on the
            // symlink TARGET and the leaf must still be a symlink.
            assert!(applied.files_changed <= 1);
            let still_symlink = fs::symlink_metadata(&lib).unwrap().file_type().is_symlink();
            assert!(
                still_symlink,
                "apply must never destroy a symlink leaf it did not plan for"
            );
            let edited = fs::read_to_string(&sibling).unwrap();
            assert!(
                edited.contains("modern(alpha)") || edited == SOURCE,
                "target content must be either edited or untouched, never lost"
            );
        }
    }
}

/// br-1xx / F2 recovery half + br-bci / F3 crash state: reproduce the exact
/// on-disk state of the OLD swap design dying between rename(source -> backup)
/// and rename(staged -> source): canonical path missing, backup present.
/// Contract: re-running `asgrep codemod` must HEAL the tree (restore some
/// complete content at the canonical path, consume the orphan backup) instead
/// of hard-failing with ENOENT while the file stays missing.
#[test]
fn rerun_after_mid_swap_crash_heals_instead_of_failing() {
    use std::process::Command;
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lib.rs"), SOURCE).unwrap();
    let index_path = temp.path().join("index.db");

    let status = Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            "--no-embed",
            root.to_str().unwrap(),
        ])
        .status()
        .expect("run asgrep index");
    assert!(status.success(), "indexing must succeed");

    // Post-crash state of a mid-swap death: canonical gone, orphan backup left.
    let lib = src.join("lib.rs");
    let backup = src.join(".lib.rs.asgrep-codemod-backup-test-1");
    fs::rename(&lib, &backup).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "--no-embed",
            "codemod",
            "--yes",
            "--pattern",
            PATTERN,
            "--rewrite",
            REWRITE,
            root.to_str().unwrap(),
        ])
        .output()
        .expect("run asgrep codemod");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("failed to verify"),
            "re-run after mid-swap crash must recover the orphaned backup, not \
             fail verification on the missing canonical file: {stderr}"
        );
    }
    let meta = fs::metadata(&lib).expect("canonical path must exist again");
    assert!(meta.is_file(), "healed path must be a regular file");
    assert!(
        !backup.exists(),
        "orphaned backup must be consumed by recovery"
    );
}

/// Multi-file fixture: `a.rs` matches (swapped first), `b.rs` and `c.rs`
/// also match so the swap loop has a real window between file 0's swap and
/// the last file's swap.
fn multi_fixture_with_plan() -> (
    TempDir,
    std::path::PathBuf,
    ast_sgrep_core::codemod::CodemodPlan,
) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        fs::write(src.join(name), SOURCE).unwrap();
    }
    let index_path = temp.path().join("index.db");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            "--no-embed",
            root.to_str().unwrap(),
        ])
        .status()
        .expect("run asgrep index");
    assert!(status.success(), "indexing must succeed");
    let plan = plan_codemod(&root, Some(&index_path), None, PATTERN, REWRITE).unwrap();
    assert_eq!(plan.files.len(), 3, "all three files must match");
    (temp, root, plan)
}

/// br-i04 / F1, H-SOAK-59-1 (pass 61): a concurrent writer lands inside the
/// verify-once/swap-later window. Each attempt fires ONE concurrent write to
/// the last planned file (c.rs) as soon as its predecessor (b.rs) is observed
/// swapped — the narrowest externally-visible point before c.rs's swap — so
/// the write races exactly the per-file verify->rename gap that pass-59 rep1
/// realized as a silent lost update.
///
/// Every attempt is classified by outcome and ALL 12 attempts run (no early
/// return on the first refusal: a write that lands during the verify-read is
/// refused by the swap-time re-verify, which is a different protection than
/// the post-swap backup-capture check under test):
/// - refusal face  (Err naming "source changed", writer content intact):
///   the contract exercised — required at least once;
/// - lost-update face (Ok while the completed concurrent write was replaced
///   by apply content): a silent data-integrity violation — immediate FAIL;
/// - late face (Ok, writer content intact: the shot landed after c.rs was
///   already swapped): harness precondition miss, never a product verdict.
///
/// The test passes only if the window was exercised (refusal face present)
/// and no lost-update face ever appeared, so scheduling starvation cannot
/// mask the contract arm (the pass-59 CI-flake face) and a reverted fix
/// cannot pass (the lost update returns as the lost-update face).
#[test]
fn concurrent_write_during_apply_is_refused_not_silently_overwritten() {
    const CONCURRENT: &str = "fn concurrent_edit() {}\n";
    let mut refusals = 0usize;
    let mut lates = 0usize;
    for _attempt in 0..12 {
        let (_temp, root, plan) = multi_fixture_with_plan();
        let c_path = root.join("src/c.rs");
        let wrote = Arc::new(AtomicBool::new(false));
        let watcher_wrote = wrote.clone();
        let watcher_root = root.clone();
        let watcher = std::thread::spawn(move || {
            let b_path = watcher_root.join("src/b.rs");
            // Deterministic in-window trigger: b.rs flips from `legacy(` to
            // rewritten content the instant its swap completes. Only c.rs's
            // stat/verify/rename remain — the narrowest observable point
            // before the target's swap.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let mut seen = false;
            while std::time::Instant::now() < deadline {
                match fs::read_to_string(&b_path) {
                    Ok(text) if text.contains("legacy(") => continue,
                    Ok(_) => {
                        seen = true; // b swapped: c's swap is the only one left
                        break;
                    }
                    Err(_) => continue,
                }
            }
            if seen {
                // The concurrent write: fresh content that does NOT match
                // the plan's expected original. If apply overwrites this
                // silently, it is a lost update.
                let _ = fs::write(&watcher_root.join("src/c.rs"), CONCURRENT);
                watcher_wrote.store(true, Ordering::SeqCst);
            }
        });

        let result = apply_codemod(&plan);
        watcher.join().unwrap();
        if !wrote.load(Ordering::SeqCst) {
            // Watcher never saw the window open (scheduling starvation) —
            // harness precondition miss for this attempt; retry.
            continue;
        }
        let c_now = fs::read_to_string(&c_path).unwrap();
        match result {
            Err(error) => {
                let text = format!("{error:#}");
                assert!(
                    text.contains("source changed"),
                    "refusal must name the stale-source problem: {text}"
                );
                assert!(
                    c_now.contains("concurrent_edit"),
                    "refusal must preserve the concurrent writer's content; \
                     c.rs now holds {c_now:?}"
                );
                refusals += 1;
            }
            Ok(applied) => {
                if !c_now.contains("concurrent_edit") {
                    // The completed concurrent write was replaced by apply
                    // content while apply reported success: silent lost
                    // update (H-SOAK-59-1, pass-59 rep1's face).
                    panic!(
                        "silent lost update: apply reported {applied:?} and \
                         c.rs now holds {c_now:?} — the completed concurrent \
                         writer was overwritten"
                    );
                }
                // The shot landed after c.rs was already swapped: the window
                // was never exercised. Harness precondition miss, not a
                // product verdict.
                lates += 1;
            }
        }
    }
    assert!(
        refusals > 0,
        "concurrent-write window never exercised in 12 attempts ({lates} \
         late shots) — the contract arm was not realized (watcher \
         precondition starvation)"
    );
}

/// br-hbd follow-up: the symlink refusal itself had a rollback defect — it
/// bails AFTER earlier files were already swapped, leaving them modernized,
/// staged sidecars leaked, and reporting an error without restoring the
/// pre-apply tree — breaking the all-or-nothing guarantee the check sits
/// inside. Contract: refusal must roll back committed swaps and clean staged
/// sidecars.
#[test]
fn symlink_refusal_mid_apply_rolls_back_committed_swaps() {
    let (_temp, root, plan) = multi_fixture_with_plan();
    // File B becomes a symlink after planning (in-root relative target).
    fs::write(root.join("src/shared_b.rs"), SOURCE).unwrap();
    fs::remove_file(root.join("src/b.rs")).unwrap();
    std::os::unix::fs::symlink("shared_b.rs", root.join("src/b.rs")).unwrap();
    // Sanity: the plan still names b.rs.
    assert!(plan.files.iter().any(|f| f.path == "src/b.rs"));

    let result = apply_codemod(&plan);

    if let Ok(applied) = &result {
        panic!(
            "apply must not succeed when a planned leaf became a symlink \
             mid-apply (got {applied:?})"
        );
    }
    let error_text = format!("{:#}", result.err().unwrap());
    assert!(
        error_text.contains("symlink"),
        "refusal must name the symlink problem: {error_text}"
    );
    // Rollback contract: file A (swapped before B's refusal) must hold its
    // ORIGINAL content again, not the rewritten one.
    let a_after = fs::read_to_string(root.join("src/a.rs")).unwrap();
    assert_eq!(
        a_after, SOURCE,
        "refusal at B must roll back A's committed swap (all-or-nothing)"
    );
    // No staged/backup sidecars may leak into the tree.
    for entry in fs::read_dir(root.join("src")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".asgrep-codemod-"),
            "sidecar leaked after refusal: {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// PASS 56 (r9 bug remediation) — planning honesty + recovery ordering.
// ---------------------------------------------------------------------------

/// Index one file whose content is `content` under `src/<name>` and return
/// (root, index_path) plus a closure-free handle on the temp dir.
fn indexed_single_file_fixture(
    name: &str,
    content: &str,
) -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join(name), content).unwrap();
    let index_path = temp.path().join("index.db");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            "--no-embed",
            root.to_str().unwrap(),
        ])
        .status()
        .expect("run asgrep index");
    assert!(status.success(), "indexing must succeed");
    (temp, root, index_path)
}

/// P1-1 (H-AUDIT-52-1, pass 56): a concrete decl pattern (`fn old_name`) is
/// served from the INDEX by search (`pattern_nodes` decl rows), but the native
/// matcher codemod plans with answers match-none for that shape. The plan must
/// therefore FAIL LOUDLY (usage-class error naming the limitation) instead of
/// returning ok:true with zero edits while search answers hits
/// (H-CONF-023 honesty class). Pre-fix this returned Ok with zero edits.
#[test]
fn codemod_loudly_refuses_index_served_decl_pattern_that_plans_zero_edits() {
    let content = "fn old_name() { legacy(alpha); }\n";
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.rs", content);

    // Parity witness: search answers the same pattern from the same index.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "--no-embed",
            "search",
            "--json",
            "pattern:fn old_name",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("run asgrep search");
    assert!(output.status.success(), "search must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"kind\""),
        "search must serve hits for the decl pattern (index lane): {stdout}"
    );

    let result = plan_codemod(&root, Some(&index_path), None, "fn old_name", "fn new_name");
    match result {
        Err(error) => {
            let text = format!("{error:#}");
            assert!(
                text.contains("fn old_name"),
                "refusal must name the unsupported pattern: {text}"
            );
            assert!(
                text.contains("index") || text.contains("structural matcher"),
                "refusal must name the limitation (index-served vs native matcher): {text}"
            );
        }
        Ok(plan) => panic!(
            "codemod must not silently plan zero edits for a pattern search \
             answers: got files_changed={} edit_count={}",
            plan.files_changed, plan.edit_count
        ),
    }
}

/// P1-1 flip side: when the index holds NO rows for the decl pattern (the
/// declaration genuinely does not exist), zero edits stay ok:true — identical
/// to search returning an empty envelope. No false loud refusal.
#[test]
fn codemod_zero_edits_stay_ok_when_index_serves_nothing() {
    let content = "fn unrelated() { keep(beta); }\n";
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.rs", content);
    let plan =
        plan_codemod(&root, Some(&index_path), None, "fn old_name", "fn new_name")
            .expect("absent declaration must plan without error");
    assert_eq!(plan.files_changed, 0, "nothing to change");
    assert_eq!(plan.edit_count, 0);
}

/// P1-2 (H-AUDIT-52-2, pass 56): a bare-ident pattern must match the IDENT
/// node, never the enclosing fn item. Pre-fix the matcher emitted BOTH the
/// whole `fn old_name() { ... }` item and the identifier, so the plan either
/// hard-failed the overlap validator or (validator weakened) would rewrite
/// the entire function body with the replacement text.
#[test]
fn codemod_bare_ident_matches_the_ident_node_not_the_enclosing_item() {
    let content = "fn old_name() { legacy(alpha); }\n";
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.rs", content);

    let plan = plan_codemod(&root, Some(&index_path), None, "old_name", "new_name")
        .expect("bare-ident rename must plan cleanly");
    assert_eq!(plan.files.len(), 1, "one file holds the identifier");
    for file in &plan.files {
        for edit in &file.edits {
            assert_eq!(
                edit.before, "old_name",
                "edit span must be the identifier itself, not an enclosing item"
            );
            assert_eq!(edit.after, "new_name");
        }
    }
    assert_eq!(plan.edit_count, 1, "exactly the definition site matches");

    // Destructive-rewrite guard: applying the plan must leave the body intact.
    let applied = ast_sgrep_core::codemod::apply_codemod(&plan).unwrap();
    assert_eq!(applied.edits_applied, 1);
    let rewritten = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert_eq!(
        rewritten, "fn new_name() { legacy(alpha); }\n",
        "only the identifier may change; the function body must survive"
    );
}

/// P2-1 (H-AUDIT-52-3, pass 56): orphan recovery must pick the newest backup
/// by TIMESTAMP, not by sidecar name. The sidecar name is
/// `.{file}.asgrep-codemod-backup-{pid}-{clock}-{nonce}`: pid precedes clock,
/// so lexicographic "newest" is temporal garbage across processes. Here the
/// lexicographically-largest name carries the OLDER content; recovery must
/// still restore the newer backup (and only then sweep leftovers).
/// Pre-fix this restored the stale backup and DELETED the newest.
#[test]
fn recover_orphans_restores_newest_backup_by_mtime_not_name_order() {
    let stale_content = "fn stale_backup() { legacy(alpha); }\n";
    let newest_content = "fn newest_backup() { legacy(alpha); }\n";
    let (temp, root, index_path) = indexed_single_file_fixture("lib.rs", newest_content);
    let src = root.join("src");
    let lib = src.join("lib.rs");

    // Post-crash state: canonical gone, two orphaned backups beside it.
    fs::remove_file(&lib).unwrap();
    let stale = src.join(".lib.rs.asgrep-codemod-backup-99999-1000-0");
    let newest = src.join(".lib.rs.asgrep-codemod-backup-100-9999999999-0");
    fs::write(&stale, stale_content).unwrap();
    fs::write(&newest, newest_content).unwrap();
    // lexicographic order: "…backup-100-…" < "…backup-99999-…", so the STALE
    // name sorts LAST and wins `sort(); pop()`. Timestamps say the opposite.
    let epoch = std::time::SystemTime::UNIX_EPOCH;
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(epoch + std::time::Duration::from_secs(1_000))
        .unwrap();
    std::fs::File::options()
        .write(true)
        .open(&newest)
        .unwrap()
        .set_modified(epoch + std::time::Duration::from_secs(9_999_999))
        .unwrap();

    // Any plan run over the still-indexed path heals the tree first.
    let plan = plan_codemod(&root, Some(&index_path), None, "legacy($ARG)", "modern($ARG)");
    let healed = fs::read_to_string(&lib).expect("canonical path must be restored");
    assert_eq!(
        healed, newest_content,
        "recovery must restore the temporally NEWEST backup, not the \
         lexicographically-largest name"
    );
    assert!(
        !stale.exists() && !newest.exists(),
        "both sidecars are swept after the restore"
    );
    let _ = plan.expect("planning continues over the healed tree");
    drop(temp);
}

// ---------------------------------------------------------------------------
// PASS 65 — pattern-ingress honesty (LOW a) and stale-index honesty (LOW b).
// ---------------------------------------------------------------------------

/// PASS 65 (LOW a): a BOM-only pattern degrades to `""` after the H-CONF-032
/// strip, so it must hit the SAME loud emptiness refusal as `""` — pre-fix
/// the strip ran after the guard and a raw U+FEFF slipped through as a
/// silent ok:true zero-edit plan with `pattern: ""`. Control: a BOM-LED real
/// pattern is NOT empty; the strip must keep working and the plan must
/// succeed (kills an always-err mutant of the guard).
#[test]
fn codemod_bom_only_pattern_refuses_loud_like_the_empty_pattern() {
    let content = "fn old_name() { legacy(alpha); }\n";
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.rs", content);

    for pattern in ["", "\u{feff}", " \u{feff} ", "\u{feff}\u{feff}"] {
        let error = plan_codemod(&root, Some(&index_path), None, pattern, "zzz")
            .expect_err("empty/BOM-only pattern must refuse loudly");
        let text = format!("{error:#}");
        assert!(
            text.contains("must not be empty"),
            "BOM-only pattern must hit the loud emptiness refusal, got: {text}"
        );
    }

    let led = plan_codemod(&root, Some(&index_path), None, "\u{feff}old_name", "new_name")
        .expect("a BOM-led real pattern must strip and plan, not refuse as empty");
    assert_eq!(led.edit_count, 1, "the stripped pattern still plans its edit");
}

/// PASS 65 (LOW b): a stale index must not trigger the H-CONF-023 honesty
/// refusal. The declaration left the tree after indexing; the index still
/// serves the old row, but the CURRENT tree is authoritative and the
/// zero-edit plan is the honest answer. Pre-65 this refused loudly blaming
/// the structural matcher — a false refusal with a wrong-cause message.
/// Control: the sibling true-positive test
/// (`codemod_loudly_refuses_index_served_decl_pattern_that_plans_zero_edits`)
/// pins that a fresh index with the declaration still present refuses.
#[test]
fn codemod_stale_index_with_removed_decl_plans_quiet_zero_not_false_refusal() {
    let content = "fn old_name() { legacy(alpha); }\n";
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.rs", content);

    // Stale the index: the declaration leaves the tree AFTER indexing.
    fs::write(root.join("src/lib.rs"), "fn new_name() { legacy(alpha); }\n").unwrap();

    let plan = plan_codemod(&root, Some(&index_path), None, "fn old_name", "fn new_name")
        .expect("stale index with the declaration gone must plan quietly, not refuse");
    assert_eq!(plan.files_changed, 0, "the tree holds no match");
    assert_eq!(plan.edit_count, 0);
}

// ---------------------------------------------------------------------------
// PASS 67b (r17 remediation) — sg-parity semantics: overlap resolution
// (F66a-7), read-only target refusal (F66a-9), and the dry-run/apply
// preview contract on a stale index (F66a-10).
// ---------------------------------------------------------------------------

/// F66a-7 (pass 66a): the optional-chain pattern matches BOTH the inner
/// `conn?.open()` and the outer `conn?.open()?.send(1)` on one line. sg's
/// `run --rewrite` resolves this per edit with OUTERMOST-WINS — it rewrites
/// the outer span (`log(conn?.open(), send)`), leaves the inner row
/// unrewritten, still applies every non-overlapping edit, and exits 0. The
/// historical subject refused the WHOLE plan (`codemod matches overlap …`,
/// exit 2), killing the unrelated edits too. The expected bytes below are
/// the probed sg-0.45.2 oracle output for exactly this fixture.
#[test]
fn plan_resolves_nested_overlaps_outermost_wins_like_sg() {
    const CHAIN_SOURCE: &str = "\
const a = user?.profile?.name;
const b = conn?.open()?.send(1);
const c = plain.call(1);
const d = maybe()?.run(2);
";
    const SG_ORACLE: &str = "\
const a = user?.profile?.name;
const b = log(conn?.open(), send);
const c = plain.call(1);
const d = log(maybe(), run);
";
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.ts", CHAIN_SOURCE);

    let plan = plan_codemod(&root, Some(&index_path), None, "$O?.$M($$$A)", "log($O, $M)")
        .expect("nested overlaps must resolve outermost-wins, not refuse the whole plan");
    assert_eq!(plan.edit_count, 2, "one OUTER edit per matched line");
    assert_eq!(plan.files.len(), 1);
    let edits = &plan.files[0].edits;

    let outer = edits
        .iter()
        .find(|edit| edit.before.contains("conn"))
        .expect("the overlapping line must keep its outer edit");
    assert_eq!(
        outer.before, "conn?.open()?.send(1)",
        "the OUTER span must win over the nested inner match"
    );
    assert_eq!(
        outer.after, "log(conn?.open(), send)",
        "the rewrite must interpolate the outer captures (sg byte-exact)"
    );
    let sibling = edits
        .iter()
        .find(|edit| edit.before.contains("maybe"))
        .expect("the non-overlapping optional-chain edit must survive the overlap resolution");
    assert_eq!(sibling.before, "maybe()?.run(2)");
    assert_eq!(sibling.after, "log(maybe(), run)");

    apply_codemod(&plan).expect("the resolved plan must apply cleanly");
    let after = fs::read_to_string(root.join("src/lib.ts")).unwrap();
    assert_eq!(
        after, SG_ORACLE,
        "applied bytes must equal the sg --rewrite oracle (inner unrewritten, no glue)"
    );
}

/// F66a-9 (plan face): a target whose owner-write bit is clear must be
/// REFUSED per file, sg-agreed — sg's update-all skips it (`Cannot rewrite
/// file … Permission denied`, os error 13) and leaves it byte+mode intact.
/// The plan must exclude the file and NAME it in `read_only_refused`
/// (preview contract: the dry-run envelope shows exactly what apply would
/// do), never plan edits it would then smuggle in through the staged
/// temp+rename commit (which only needs the writable parent directory).
#[test]
#[cfg(unix)]
fn plan_excludes_read_only_target_and_names_it_sg_agreed() {
    use std::os::unix::fs::PermissionsExt;
    let content = "fn run() { legacy(alpha); }\n";
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.rs", content);
    let lib = root.join("src/lib.rs");
    fs::set_permissions(&lib, fs::Permissions::from_mode(0o444)).unwrap();

    let plan = plan_codemod(&root, Some(&index_path), None, PATTERN, REWRITE)
        .expect("a read-only target is refused per file, not a whole-plan error");
    assert!(
        plan.files.is_empty(),
        "no edits may be planned for a read-only target"
    );
    assert_eq!(
        plan.read_only_refused,
        vec!["src/lib.rs".to_string()],
        "the refused file must be named for the operator"
    );
    assert_eq!(plan.edit_count, 0);
}

/// F66a-9 (apply face, plan→apply TOCTOU): a target chmod'd 0444 AFTER
/// planning must refuse the transaction — the staged temp+rename commit
/// would otherwise bypass the file's read-only permission and silently
/// replace its content (the registered 66a face: exit 0, content swapped,
/// 0444 mode preserved). Fail closed like sg, leave the file byte+mode
/// intact, and leak no sidecars.
#[test]
#[cfg(unix)]
fn apply_refuses_when_target_turned_read_only_after_planning() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, root, index_path) = indexed_single_file_fixture("lib.rs", SOURCE);
    let lib = root.join("src/lib.rs");

    let plan = plan_codemod(&root, Some(&index_path), None, PATTERN, REWRITE)
        .expect("plan while the target is still writable");
    fs::set_permissions(&lib, fs::Permissions::from_mode(0o444)).unwrap();

    let error = apply_codemod(&plan)
        .expect_err("a read-only target must be refused, never rewritten via rename");
    let text = format!("{error:#}");
    assert!(
        text.contains("read-only") && text.contains("src/lib.rs"),
        "refusal must name the read-only target: {text}"
    );
    let meta = fs::metadata(&lib).unwrap();
    assert_eq!(
        meta.permissions().mode() & 0o777,
        0o444,
        "the refused file's mode must be untouched"
    );
    assert_eq!(
        fs::read_to_string(&lib).unwrap(),
        SOURCE,
        "the refused file's bytes must be untouched"
    );
    for entry in fs::read_dir(root.join("src")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".asgrep-codemod-"),
            "sidecar leaked after the read-only refusal: {name}"
        );
    }
}

/// F66a-9 (CLI face, sg exit class): with one read-only and one writable
/// target, sg's update-all applies the writable file, skips the read-only
/// one (`Skip to next file`), and exits 6. The subject must mirror that
/// surface: the writable edit IS applied, the read-only file stays
/// byte+mode intact, and the run fails loudly (the subject's operational
/// exit class) naming the refused file — never a silent ok:true.
#[test]
#[cfg(unix)]
fn codemod_apply_skips_read_only_file_applies_writable_and_exits_nonzero() {
    use std::os::unix::fs::PermissionsExt;
    const RO_SOURCE: &str = "fn ro_target() { legacy(ro); }\n";
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("ro.rs"), RO_SOURCE).unwrap();
    fs::write(src.join("rw.rs"), SOURCE).unwrap();
    let index_path = temp.path().join("index.db");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            "--no-embed",
            root.to_str().unwrap(),
        ])
        .status()
        .expect("run asgrep index");
    assert!(status.success(), "indexing must succeed");
    let ro = src.join("ro.rs");
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o444)).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "--no-embed",
            "codemod",
            "--yes",
            "--pattern",
            PATTERN,
            "--rewrite",
            REWRITE,
            root.to_str().unwrap(),
        ])
        .output()
        .expect("run asgrep codemod");

    assert!(
        !output.status.success(),
        "a refused read-only target must fail the run (sg exits 6), not exit 0"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("read-only") && stderr.contains("ro.rs"),
        "the refusal must name the read-only file: {stderr}"
    );
    let meta = fs::metadata(&ro).unwrap();
    assert_eq!(
        meta.permissions().mode() & 0o777,
        0o444,
        "the refused file's mode must be untouched"
    );
    assert_eq!(
        fs::read_to_string(&ro).unwrap(),
        RO_SOURCE,
        "the refused file's bytes must be untouched"
    );
    assert_eq!(
        fs::read_to_string(src.join("rw.rs")).unwrap(),
        "fn run() { modern(alpha); }\nfn keep() { modern(beta); }\n",
        "the writable sibling's edits must still be applied (sg per-file skip)"
    );
    for entry in fs::read_dir(&src).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".asgrep-codemod-"),
            "sidecar leaked after the read-only skip: {name}"
        );
    }
}

/// F66a-10 (pass 66a): the preview contract — on a STALE index (file
/// shifted after indexing, no re-index) the dry-run preview and the `--yes`
/// apply must agree on the SAME plan, because both paths plan from the
/// current tree bytes: correctly literal-anchored edits are shown AND
/// applied. The registered r16 face observed dry-run `edit_count: 0` while
/// apply performed the edits; whichever path grew a freshness short-circuit,
/// this pin fails if the two surfaces ever diverge again.
#[test]
fn stale_index_dry_run_and_apply_agree_on_edit_count() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.py"), "def old_name(x):\n    return old_name(x)\n").unwrap();
    let index_path = temp.path().join("index.db");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            "--no-embed",
            root.to_str().unwrap(),
        ])
        .status()
        .expect("run asgrep index");
    assert!(status.success(), "indexing must succeed");
    // Stale the index: shift every match down two lines, do NOT re-index.
    let shifted = "# shifted\n# shifted2\ndef old_name(x):\n    return old_name(x)\n";
    fs::write(src.join("a.py"), shifted).unwrap();

    let dry = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "--no-embed",
            "codemod",
            "--dry-run",
            "--json",
            "--pattern",
            "old_name",
            "--rewrite",
            "new_name",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("run asgrep codemod --dry-run");
    assert!(dry.status.success(), "dry-run must succeed on a stale index");
    let dry: serde_json::Value = serde_json::from_slice(&dry.stdout).unwrap();
    let dry_edits = dry["plan"]["edit_count"].as_u64().unwrap();

    let apply = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "--no-embed",
            "codemod",
            "--yes",
            "--json",
            "--pattern",
            "old_name",
            "--rewrite",
            "new_name",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("run asgrep codemod --yes");
    assert!(
        apply.status.success(),
        "apply must succeed on the same stale state (edits are literal-anchored)"
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).unwrap();
    let applied = apply["edits_applied"].as_u64().unwrap();

    assert_eq!(
        dry_edits, applied,
        "preview contract: dry-run and apply must agree on the same stale state"
    );
    assert_eq!(dry_edits, 2, "both sites are literal-anchored after the shift");
    assert_eq!(
        fs::read_to_string(src.join("a.py")).unwrap(),
        "# shifted\n# shifted2\ndef new_name(x):\n    return new_name(x)\n",
        "apply must anchor the edits to the shifted bytes"
    );
}

/// F76-3 (pass 77E): a `$$NAME` reference in a rewrite template must
/// substitute the capture named NAME — sg's rewrite output treats `$$A` as a
/// capture reference whenever a name follows the `$$`, and only falls back to
/// the `$$`→`$` escape when no name follows. Pre-fix the escape arm fired
/// unconditionally, so pattern `g($$A)` → rewrite `wrap($$A)` planned
/// `wrap($A)` (literal text), which would have corrupted sources on apply.
/// Faces are sg text-parity: ts `g($$A)`→`wrap($$A)` plans `wrap(1)`/`wrap(5)`
/// like sg's applied diff, and php `$svc->run($$A)`→`ZZ($$A)` replays the FULL
/// bound text (sg binds `$$weird` verbatim as the capture, single namespace
/// F74a-3 stripped keys).
#[test]
fn rewrite_template_two_dollar_name_substitutes_the_capture_like_sg() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.ts"), "const x = g(1);\nconst y = g(5);\n").unwrap();
    fs::write(
        root.join("b.php"),
        "<?php\n$svc->run($u);\n$svc->run($$weird);\n",
    )
    .unwrap();

    // plan_codemod fail-closes on an empty index (H-AUDIT missing/empty-index
    // law), so arm a real store before planning.
    let index_path = temp.path().join("index.db");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            "--no-embed",
            root.to_str().unwrap(),
        ])
        .status()
        .expect("run asgrep index");
    assert!(status.success(), "indexing must succeed");

    let plan = plan_codemod(
        &root,
        Some(&index_path),
        None,
        "g($$A)",
        "wrap($$A)",
    )
    .unwrap();
    let mut planned: Vec<String> = plan
        .files
        .iter()
        .flat_map(|file| file.edits.iter().map(|edit| edit.after.clone()))
        .collect();
    planned.sort();
    assert_eq!(
        planned,
        vec!["wrap(1)".to_string(), "wrap(5)".to_string()],
        "$$A in the rewrite template must substitute capture A, not emit literal $A"
    );

    let plan = plan_codemod(&root, Some(&index_path), None, "$svc->run($$A)", "ZZ($$A)").unwrap();
    let mut planned: Vec<String> = plan
        .files
        .iter()
        .flat_map(|file| file.edits.iter().map(|edit| edit.after.clone()))
        .collect();
    planned.sort();
    assert_eq!(
        planned,
        vec!["ZZ($$weird)".to_string(), "ZZ($u)".to_string()],
        "sg binds the FULL $$weird text as the capture; $$A template replays it verbatim"
    );
}
