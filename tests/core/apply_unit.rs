//! The capture->commit gap the AlreadyExists arm guards is two adjacent
//! syscalls wide (rename moves the source away, hard_link recreates
//! it), so no other thread can observe the vacancy and write into it —
//! observation latency exceeds the window. The branch contract is
//! therefore pinned against a real on-disk staged state constructed
//! exactly as the swap loop leaves it when a concurrent writer wins the
//! gap, driven through the same entry point the arm calls.
use ast_sgrep_core::codemod::apply::*;
use ast_sgrep_core::codemod::guard::target_refuses_writes;
use ast_sgrep_core::codemod::{CodemodEdit, CodemodFilePlan, CodemodPlan};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use std::fs;
use std::path::PathBuf;

const SOURCE: &str = "fn run() { legacy(alpha); }\n";
const REWRITTEN: &str = "fn run() { modern(alpha); }\n";
const CONCURRENT: &str = "fn concurrent_edit() {}\n";

#[test]
fn orphan_recovery_keeps_each_missing_files_own_bytes() {
    use ast_sgrep_core::{IndexOptions, Indexer};
    use std::time::{Duration, SystemTime};

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let a = "fn a() { legacy(alpha); }\n";
    let b = "fn b() { legacy(beta); }\n";
    fs::write(root.join("a.rs"), a).unwrap();
    fs::write(root.join("b.rs"), b).unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        ..Default::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    drop(indexer);

    // Both source names were captured before a process died. The newer
    // b.rs backup must never be restored as a.rs or swept as its leftover.
    for (name, seconds) in [("a", 1), ("b", 2)] {
        let backup = root.join(format!(".{name}.rs.asgrep-codemod-backup-1-{seconds}-0"));
        fs::rename(root.join(format!("{name}.rs")), &backup).unwrap();
        fs::File::open(backup)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
            )
            .unwrap();
    }

    let unrelated_backup = root.join(".unindexed.rs.asgrep-codemod-backup-1-3-0");
    let unrelated_stage = root.join(".unindexed.rs.asgrep-codemod-stage-1-3-0");
    fs::write(&unrelated_backup, CONCURRENT).unwrap();
    fs::write(&unrelated_stage, REWRITTEN).unwrap();

    let plan =
        ast_sgrep_core::codemod::plan_codemod(root, None, None, "legacy($ARG)", "modern($ARG)");
    assert_eq!(fs::read_to_string(root.join("a.rs")).unwrap(), a);
    assert_eq!(fs::read_to_string(root.join("b.rs")).unwrap(), b);
    assert_eq!(plan.unwrap().files_changed, 2);
    assert_eq!(fs::read_to_string(unrelated_backup).unwrap(), CONCURRENT);
    assert_eq!(fs::read_to_string(unrelated_stage).unwrap(), REWRITTEN);
}

#[test]
#[cfg(unix)]
fn orphan_recovery_must_not_follow_an_escaping_parent_symlink() {
    use ast_sgrep_core::{IndexOptions, Indexer};
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(root.join("src/a.rs"), SOURCE).unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        embed_semantic: false,
        ..Default::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    drop(indexer);
    fs::rename(root.join("src"), root.join("original-src")).unwrap();
    symlink(&outside, root.join("src")).unwrap();
    let backup = outside.join(".a.rs.asgrep-codemod-backup-1-2-0");
    fs::write(&backup, CONCURRENT).unwrap();

    let result =
        ast_sgrep_core::codemod::plan_codemod(&root, None, None, "legacy($ARG)", "modern($ARG)");
    assert!(result.is_err(), "planning must refuse an escaped source");
    assert!(
        !outside.join("a.rs").exists(),
        "recovery wrote outside root"
    );
    assert_eq!(fs::read_to_string(backup).unwrap(), CONCURRENT);
}

/// Pre-condition: when a concurrent writer wins the capture->commit gap,
/// the refusal must (1) keep the writer's content at the source path,
/// (2) keep the capture sidecar — the ONLY copy of the pre-race content
/// — on disk and name it in the error, (3) roll back earlier committed
/// swaps, and (4) remove staged files. The historical behavior unlinked
/// the sidecar, destroying the last pre-race copy.
#[test]
fn commit_race_refusal_keeps_the_only_pre_race_copy_and_names_it() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();

    // Post-race state for a two-file plan (a.rs committed, b.rs raced):
    // a.rs holds applied content; its backup holds the original and is
    // the rollback source; both staged files still exist (hard_link adds
    // a name, it does not consume the staged one).
    fs::write(src.join("a.rs"), REWRITTEN).unwrap();
    fs::write(src.join(".a.rs.asgrep-codemod-stage-111-222-0"), REWRITTEN).unwrap();
    fs::write(src.join(".a.rs.asgrep-codemod-backup-111-222-0"), SOURCE).unwrap();
    // b.rs holds the concurrent writer's content; its capture sidecar
    // holds the pre-race content and is now the only copy of it.
    fs::write(src.join("b.rs"), CONCURRENT).unwrap();
    fs::write(src.join(".b.rs.asgrep-codemod-stage-111-222-1"), REWRITTEN).unwrap();
    let capture_name = ".b.rs.asgrep-codemod-backup-111-222-1";
    fs::write(src.join(capture_name), SOURCE).unwrap();

    let root_dir = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();
    let mut staged = vec![
        StagedFile {
            relative: PathBuf::from("src/a.rs"),
            staged: PathBuf::from("src/.a.rs.asgrep-codemod-stage-111-222-0"),
            backup: Some(PathBuf::from("src/.a.rs.asgrep-codemod-backup-111-222-0")),
        },
        StagedFile {
            relative: PathBuf::from("src/b.rs"),
            staged: PathBuf::from("src/.b.rs.asgrep-codemod-stage-111-222-1"),
            backup: Some(PathBuf::from("src").join(capture_name)),
        },
    ];

    let error = commit_race_writer_won(
        &root_dir,
        &root,
        &mut staged,
        1,
        "src/b.rs",
        &PathBuf::from("src").join(capture_name),
    );

    // (2) the capture sidecar survives ON DISK with the pre-race content
    // and the error names it. (Mutant: the historical unlink makes this
    // the failing assertion — the only pre-race copy was destroyed.)
    let text = format!("{error:#}");
    assert!(
        text.contains("capture sidecar") && text.contains(capture_name),
        "refusal must name the capture sidecar: {text}"
    );
    let kept = fs::read_to_string(src.join(capture_name))
        .unwrap_or_else(|error| panic!("capture sidecar must survive the refusal: {error}"));
    assert_eq!(
        kept, SOURCE,
        "the surviving sidecar must hold the pre-race content"
    );
    // (1) the concurrent writer's content stays authoritative.
    assert_eq!(
        fs::read_to_string(src.join("b.rs")).unwrap(),
        CONCURRENT,
        "the writer's newest content must stay at the source path"
    );
    // (3) earlier committed swaps are rolled back to their originals.
    assert_eq!(
        fs::read_to_string(src.join("a.rs")).unwrap(),
        SOURCE,
        "a.rs must be rolled back (all-or-nothing)"
    );
    assert!(
        !src.join(".a.rs.asgrep-codemod-backup-111-222-0").exists(),
        "a.rs's backup is consumed by the rollback"
    );
    // (4) staged files are cleaned up on both sides of the refusal.
    assert!(!src.join(".a.rs.asgrep-codemod-stage-111-222-0").exists());
    assert!(!src.join(".b.rs.asgrep-codemod-stage-111-222-1").exists());
}

const TWO_LINE_REWRITTEN: &str = "fn run() { modern(alpha); }\nfn keep() { modern(beta); }\n";

/// The swap loop re-verified CONTENT immediately before each swap but
/// NOT the target's MODE. A target chmod'd 0444 after the staging guard
/// read a writable mode therefore went through the staged rename anyway
/// — exit 0, content swapped, the exact silent-rewrite face the
/// writability guards closed, realized intermittently by a soak
/// repetition on an unchanged binary (guard read 0444 as writable).
/// Deterministic seam reproduction: hand-construct the exact post-staging
/// state (both targets staged with their captured WRITABLE perms, sources
/// untouched), then chmod ONE target read-only BEFORE the swap loop runs
/// — the staging→swap interleaving the staging-time guard cannot see, as
/// a state instead of a race. Contract: the swap refuses the transaction,
/// rolls back the already-committed file, leaves every byte+mode intact,
/// and leaks no sidecars.
#[test]
#[cfg(unix)]
fn swap_loop_refuses_target_turned_read_only_after_staging() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.rs"), SOURCE).unwrap();
    fs::write(src.join("b.rs"), SOURCE).unwrap();
    let root_dir = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();

    // Plan made while both targets were writable; both files staged
    // exactly as the staging closure leaves them (staged sibling written,
    // writable perms copied).
    let plan = CodemodPlan {
        pattern: "legacy($ARG)".to_string(),
        rewrite: "modern($ARG)".to_string(),
        files_changed: 2,
        edit_count: 2,
        files: vec![
            CodemodFilePlan {
                path: "src/a.rs".to_string(),
                edits: vec![CodemodEdit {
                    path: "src/a.rs".to_string(),
                    byte_start: 10,
                    byte_end: 23,
                    line_start: 1,
                    line_end: 1,
                    before: "legacy(alpha)".to_string(),
                    after: "modern(alpha)".to_string(),
                }],
                original: SOURCE.to_string(),
                rewritten: TWO_LINE_REWRITTEN.to_string(),
            },
            CodemodFilePlan {
                path: "src/b.rs".to_string(),
                edits: vec![CodemodEdit {
                    path: "src/b.rs".to_string(),
                    byte_start: 10,
                    byte_end: 23,
                    line_start: 1,
                    line_end: 1,
                    before: "legacy(alpha)".to_string(),
                    after: "modern(alpha)".to_string(),
                }],
                original: SOURCE.to_string(),
                rewritten: TWO_LINE_REWRITTEN.to_string(),
            },
        ],
        read_only_refused: Vec::new(),
        root: root.clone(),
    };
    let mut staged = Vec::new();
    for (index, name) in ["a.rs", "b.rs"].iter().enumerate() {
        let relative = PathBuf::from("src").join(name);
        let permissions = root_dir.metadata(&relative).unwrap().permissions();
        assert!(
            !target_refuses_writes(&permissions),
            "precondition: {name} is writable at staging time"
        );
        let staged_path =
            write_staged_file(&root_dir, &relative, TWO_LINE_REWRITTEN, index).unwrap();
        root_dir.set_permissions(&staged_path, permissions).unwrap();
        staged.push(StagedFile {
            relative,
            staged: staged_path,
            backup: None,
        });
    }

    // The chmod lands AFTER the staging guard read a writable mode and
    // BEFORE the swap loop reaches b.rs — the window the soak realized
    // intermittently, constructed here as a state, not a race.
    fs::set_permissions(src.join("b.rs"), fs::Permissions::from_mode(0o444)).unwrap();

    let result = swap_staged_files(&root_dir, &plan, &mut staged);

    let error = result.expect_err(
        "a target turned read-only after staging must be refused, never \
             swapped through the staged rename",
    );
    let text = format!("{error:#}");
    assert!(
        text.contains("read-only") && text.contains("src/b.rs"),
        "refusal must name the read-only target: {text}"
    );
    // b.rs: byte+mode intact — never swapped, never restaged.
    let b_meta = fs::metadata(src.join("b.rs")).unwrap();
    assert_eq!(
        b_meta.permissions().mode() & 0o777,
        0o444,
        "the refused file's mode must be untouched"
    );
    assert_eq!(
        fs::read_to_string(src.join("b.rs")).unwrap(),
        SOURCE,
        "the refused file's bytes must be untouched"
    );
    // a.rs: its swap committed before b.rs's refusal, so the rollback
    // must restore its original bytes (all-or-nothing).
    assert_eq!(
        fs::read_to_string(src.join("a.rs")).unwrap(),
        SOURCE,
        "a.rs's committed swap must be rolled back"
    );
    let a_meta = fs::metadata(src.join("a.rs")).unwrap();
    assert_eq!(
        a_meta.permissions().mode() & 0o777,
        0o644,
        "the rolled-back file must carry its original mode again"
    );
    // No backup or staged sidecars may survive the refusal.
    for entry in fs::read_dir(&src).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".asgrep-codemod-"),
            "sidecar leaked after the swap-time read-only refusal: {name}"
        );
    }
}

/// Hand-construct the exact post-staging state for a two-file plan,
/// mirroring `swap_loop_refuses_target_turned_read_only_after_staging`:
/// sources byte-identical to the plan, staged siblings written with the
/// sources' writable perms, no backups yet (the swap loop creates
/// those).
fn two_file_swappable_state() -> (tempfile::TempDir, CodemodPlan, Vec<StagedFile>) {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.rs"), SOURCE).unwrap();
    fs::write(src.join("b.rs"), SOURCE).unwrap();
    let root_dir = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();

    let file_plan = |name: &str| CodemodFilePlan {
        path: format!("src/{name}"),
        edits: vec![CodemodEdit {
            path: format!("src/{name}"),
            byte_start: 10,
            byte_end: 23,
            line_start: 1,
            line_end: 1,
            before: "legacy(alpha)".to_string(),
            after: "modern(alpha)".to_string(),
        }],
        original: SOURCE.to_string(),
        rewritten: TWO_LINE_REWRITTEN.to_string(),
    };
    let plan = CodemodPlan {
        pattern: "legacy($ARG)".to_string(),
        rewrite: "modern($ARG)".to_string(),
        files_changed: 2,
        edit_count: 2,
        files: vec![file_plan("a.rs"), file_plan("b.rs")],
        read_only_refused: Vec::new(),
        root: root.clone(),
    };
    let mut staged = Vec::new();
    for (index, name) in ["a.rs", "b.rs"].iter().enumerate() {
        let relative = PathBuf::from("src").join(name);
        let permissions = root_dir.metadata(&relative).unwrap().permissions();
        assert!(
            !target_refuses_writes(&permissions),
            "precondition: {name} is writable at staging time"
        );
        let staged_path =
            write_staged_file(&root_dir, &relative, TWO_LINE_REWRITTEN, index).unwrap();
        root_dir.set_permissions(&staged_path, permissions).unwrap();
        staged.push(StagedFile {
            relative,
            staged: staged_path,
            backup: None,
        });
    }
    assert_eq!(
        fs::metadata(src.join("a.rs")).unwrap().permissions().mode() & 0o777,
        0o644,
        "precondition: sources are plain writable files"
    );
    (temp, plan, staged)
}

/// A file enlarged after planning must be rejected before allocating/decoding
/// its contents, both during staging and after an earlier file was committed.
#[test]
#[cfg(unix)]
fn apply_and_swap_reject_oversized_sources_before_utf8_decode() {
    use ast_sgrep_core::MAX_INDEX_FILE_BYTES;
    use std::collections::BTreeSet;
    use std::io::{Read, Write};
    use std::os::unix::fs::MetadataExt;

    let mut missing_cap = Vec::new();
    for during_swap in [false, true] {
        let (_temp, plan, mut staged) = two_file_swappable_state();
        let src = plan.root.join("src");
        let original_inode = fs::metadata(src.join("a.rs")).unwrap().ino();
        let expected_sidecars: BTreeSet<_> = if during_swap {
            BTreeSet::new()
        } else {
            // apply_codemod owns its new staging files, not these pre-existing ones.
            staged
                .iter()
                .map(|file| plan.root.join(&file.staged))
                .collect()
        };
        let mut enlarged = fs::File::create(src.join("b.rs")).unwrap();
        enlarged.write_all(&[0xff]).unwrap();
        enlarged.set_len(MAX_INDEX_FILE_BYTES + 1).unwrap();
        drop(enlarged);

        let result = if during_swap {
            let dir = Dir::open_ambient_dir(&plan.root, ambient_authority()).unwrap();
            swap_staged_files(&dir, &plan, &mut staged)
        } else {
            apply_codemod(&plan).map(|_| ())
        };
        let error = format!(
            "{:#}",
            result.expect_err("oversized changed source must refuse")
        );
        if !error.contains("byte index cap") || !error.contains("src/b.rs") {
            missing_cap.push(format!("during_swap={during_swap}: {error}"));
        }
        assert_eq!(fs::read_to_string(src.join("a.rs")).unwrap(), SOURCE);
        assert_eq!(
            fs::metadata(src.join("a.rs")).unwrap().ino(),
            original_inode
        );
        assert_eq!(
            fs::metadata(src.join("b.rs")).unwrap().len(),
            MAX_INDEX_FILE_BYTES + 1
        );
        let mut first_byte = [0];
        fs::File::open(src.join("b.rs"))
            .unwrap()
            .read_exact(&mut first_byte)
            .unwrap();
        assert_eq!(first_byte, [0xff], "refusal must preserve the changed file");
        let remaining_sidecars: BTreeSet<_> = fs::read_dir(&src)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".asgrep-codemod-")
            })
            .collect();
        assert_eq!(
            remaining_sidecars, expected_sidecars,
            "refusal must clean only owned staging"
        );
    }
    assert!(
        missing_cap.is_empty(),
        "size admission must precede UTF-8 decoding: {missing_cap:?}"
    );
}

/// The swap-time mode re-check propagated a stat FAILURE with `?` —
/// early-returning from `swap_staged_files` after file 1's swap already
/// committed, WITHOUT `rollback_committed`/`cleanup_staged`, against the
/// contract that ANY error rolls back every swap committed so far and
/// removes the staged siblings. Deterministic seam: the injected fstatat
/// failure fires for b.rs AFTER its content re-read succeeded — the
/// exact read→stat window (delete race / EACCES / EIO between the
/// re-read at the loop head and the stat), which no portable on-disk
/// state can produce. Required behavior: loud error naming the file,
/// a.rs restored bytes+mode+INODE (the rollback renames the captured
/// original back — the same hygiene path the read-only refusal uses),
/// zero sidecar leftovers.
#[test]
#[cfg(unix)]
fn swap_loop_stat_failure_after_a_committed_swap_rolls_back_and_sweeps() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let (temp, plan, mut staged) = two_file_swappable_state();
    let src = temp.path().join("fixture/src");
    let root_dir = Dir::open_ambient_dir(temp.path().join("fixture"), ambient_authority()).unwrap();

    let a_before = fs::metadata(src.join("a.rs")).unwrap();
    let a_ino_before = a_before.ino();

    arm_injected_swap_stat_failure(
        "b.rs",
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected swap-time fstatat failure",
        ),
    );

    let error = swap_staged_files(&root_dir, &plan, &mut staged).expect_err(
        "a failed swap-time stat must refuse the transaction loudly, never \
             proceed with a partial swap",
    );
    let text = format!("{error:#}");
    assert!(
        text.contains("failed to stat") && text.contains("src/b.rs"),
        "refusal must name the stat failure and the file: {text}"
    );
    assert!(
        text.contains("all changes rolled back"),
        "error must carry the rollback rider: {text}"
    );

    // a.rs: its swap committed before the stat failed, so the rollback
    // must restore the original bytes, mode AND inode (the original file
    // is renamed back from its backup slot, not re-created).
    assert_eq!(
        fs::read_to_string(src.join("a.rs")).unwrap(),
        SOURCE,
        "a.rs's committed swap must be rolled back (all-or-nothing)"
    );
    let a_after = fs::metadata(src.join("a.rs")).unwrap();
    assert_eq!(
        a_after.permissions().mode() & 0o777,
        0o644,
        "the rolled-back file must carry its original mode again"
    );
    assert_eq!(
        a_after.ino(),
        a_ino_before,
        "rollback must rename the original inode back, not leave the \
             staged inode at the source path"
    );
    // b.rs: never reached the swap; untouched.
    assert_eq!(
        fs::read_to_string(src.join("b.rs")).unwrap(),
        SOURCE,
        "the file whose stat failed must be byte-identical"
    );
    // No staged or backup sidecars may survive the refusal.
    for entry in fs::read_dir(&src).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".asgrep-codemod-"),
            "sidecar leaked after the swap-time stat-failure rollback: {name}"
        );
    }
}

/// The PRE-EXISTING sibling hole on the content re-read
/// (`read_to_string(...)?`) — the same leak class on an older arm. A
/// source removed between staging and its swap (concurrent `rm`, the
/// delete-race face the re-read exists to catch) must refuse with the
/// same rollback+cleanup+rider hygiene as the mismatch refusal two
/// lines below it. Driven by a real on-disk state, no injection needed.
#[test]
#[cfg(unix)]
fn swap_loop_source_vanished_after_a_committed_swap_rolls_back_and_sweeps() {
    use std::os::unix::fs::MetadataExt;
    let (temp, plan, mut staged) = two_file_swappable_state();
    let src = temp.path().join("fixture/src");
    let root_dir = Dir::open_ambient_dir(temp.path().join("fixture"), ambient_authority()).unwrap();

    let a_ino_before = fs::metadata(src.join("a.rs")).unwrap().ino();
    // The concurrent removal lands after staging, before b.rs's swap.
    fs::remove_file(src.join("b.rs")).unwrap();

    let error = swap_staged_files(&root_dir, &plan, &mut staged).expect_err(
        "a vanished source must refuse the transaction, never be silently \
             skipped or swapped",
    );
    let text = format!("{error:#}");
    assert!(
        text.contains("failed to re-read") && text.contains("src/b.rs"),
        "refusal must name the re-read failure and the file: {text}"
    );
    assert!(
        text.contains("all changes rolled back"),
        "error must carry the rollback rider: {text}"
    );

    assert_eq!(
        fs::read_to_string(src.join("a.rs")).unwrap(),
        SOURCE,
        "a.rs's committed swap must be rolled back (all-or-nothing)"
    );
    assert_eq!(
        fs::metadata(src.join("a.rs")).unwrap().ino(),
        a_ino_before,
        "rollback must restore the original inode"
    );
    for entry in fs::read_dir(&src).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(".asgrep-codemod-"),
            "sidecar leaked after the vanished-source rollback: {name}"
        );
    }
}
