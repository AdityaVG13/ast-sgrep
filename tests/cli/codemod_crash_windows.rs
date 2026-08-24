//! Failure-first RED tests for the codemod apply/rollback crash windows
//! (br-i04, br-1xx, br-bci, br-hbd; audit:
//! docs/validation/audits/2026-08-23-codemod-edit-path.md).
//!
//! Every fixture is deterministic: the "crash window" races are realized by
//! mutating the tree between plan_codemod and apply_codemod (the window
//! verify-once/swap-later leaves unprotected) or by reproducing the exact
//! post-crash filesystem state of a mid-swap death.
use ast_sgrep_core::codemod::{apply_codemod, plan_codemod};
use std::fs;
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
