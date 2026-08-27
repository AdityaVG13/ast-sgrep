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

    let plan = plan_codemod(&root, Some(&index_path), PATTERN, REWRITE).unwrap();
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
    let plan = plan_codemod(&root, Some(&index_path), PATTERN, REWRITE).unwrap();
    assert_eq!(plan.files.len(), 3, "all three files must match");
    (temp, root, plan)
}

/// br-i04 / F1: a concurrent writer lands inside the verify-once/swap-later
/// window. Deterministic realization: a watcher thread polls for file 0's
/// BACKUP sidecar to appear (= staging finished, every file already verified,
/// swap loop entered) and mutates the LAST planned file at that moment.
///
/// Contract: apply must fail loudly naming "source changed", never report
/// success; the writer's content must survive; and any earlier committed
/// swap of this apply must be rolled back (the transaction is all-or-nothing).
#[test]
fn concurrent_write_during_apply_is_refused_not_silently_overwritten() {
    let (_temp, root, plan) = multi_fixture_with_plan();
    let c_path = root.join("src/c.rs");
    let backup_seen = Arc::new(AtomicBool::new(false));
    let watcher_flag = backup_seen.clone();
    let watcher_root = root.clone();
    let watcher = std::thread::spawn(move || {
        let a_path = watcher_root.join("src/a.rs");
        // Deterministic in-window signal: file A's canonical content changes
        // from `legacy(` to `modern(` the instant its swap completes. That
        // is proof staging finished (every file verified) and file A's swap
        // is done — exactly inside the verify-once/swap-later window for
        // files B and C.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut seen = false;
        while std::time::Instant::now() < deadline {
            match fs::read_to_string(&a_path) {
                Ok(text) if text.contains("legacy(") => {
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
                Ok(_) => {
                    seen = true; // A now holds rewritten content: window open
                    break;
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
            }
        }
        if seen {
            // The concurrent write: fresh content that does NOT match the
            // plan's expected original. If apply overwrites this silently,
            // it is a lost update.
            fs::write(&watcher_root.join("src/c.rs"), "fn concurrent_edit() {}\n").unwrap();
            backup_seen.store(true, Ordering::SeqCst);
        }
        seen
    });

    let result = apply_codemod(&plan);
    let raced = watcher_flag.load(Ordering::SeqCst);
    let watcher_hit = watcher.join().unwrap();

    // The race MUST have been realized: if the watcher never saw the swap
    // window open, the fixture failed its own precondition (CI flake guard).
    assert!(watcher_hit && raced, "watcher must observe the swap window");

    match result {
        Err(error) => {
            let text = format!("{error:#}");
            assert!(
                text.contains("source changed"),
                "refusal must name the stale-source problem: {text}"
            );
        }
        Ok(applied) => {
            // Pre-fix behavior: Ok with the concurrent content destroyed.
            let c_now = fs::read_to_string(&c_path).unwrap();
            assert!(
                applied.files_changed == 0 && c_now.contains("concurrent_edit"),
                "silent lost update: apply reported {applied:?} and c.rs now \
                 holds {c_now:?} — the concurrent writer was overwritten"
            );
        }
    }
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
