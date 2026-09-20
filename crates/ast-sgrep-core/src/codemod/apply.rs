//! Transactional apply: staging, the all-or-nothing swap loop, rollback,
//! crash recovery, and the commit-race refusal surface.

use super::guard::{confined_relative_path, target_refuses_writes};
use super::{CodemodApplyResult, CodemodPlan};
use anyhow::{bail, Context};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Build the refusal for a concurrent write winning the capture->commit
/// gap. The error NAMES the capture sidecar (the only copy of the
/// pre-swap content) so the operator can recover or discard it
/// deliberately. The helper never touches the filesystem; the caller keeps
/// the sidecar on disk by simply not deleting it (`cleanup_staged` only
/// removes staged paths, never backups). `pub` so the
/// transaction-contract tests can pin the exact refusal wording.
pub fn concurrent_capture_error(
    root: &Path,
    relative: &str,
    backup: &Path,
    rollback: Option<std::io::Error>,
) -> anyhow::Error {
    let sidecar = root.join(backup);
    let base = format!(
        "source changed after codemod planning: {relative}; a concurrent write \
         recreated the file inside the apply window; its content is kept at the \
         source path and the pre-swap content remains in the capture sidecar {}",
        sidecar.display()
    );
    match rollback {
        Some(rb) => anyhow::anyhow!("{base}; rollback also failed: {rb}"),
        None => anyhow::anyhow!("{base} and all codemod changes are rolled back"),
    }
}

/// The commit's create-if-absent hard link lost the capture->commit gap
/// race — the source path exists again, holding a concurrent writer's
/// bytes. Invariants: the writer's content is the authoritative newest
/// state (restoring the capture would clobber it — the exact lost update
/// the hard-link commit exists to prevent), and the capture sidecar now
/// holds the ONLY copy of the pre-race content, so it stays on disk and is
/// named in the refusal. Never unlink the last reference. Rolls back every
/// earlier committed swap and removes the staged files; the backup sidecar
/// is deliberately untouched.
pub fn commit_race_writer_won(
    root_dir: &Dir,
    root: &Path,
    staged: &mut [StagedFile],
    index: usize,
    relative: &str,
    backup: &Path,
) -> anyhow::Error {
    let rollback = rollback_committed(root_dir, staged, index);
    cleanup_staged(root_dir, staged);
    concurrent_capture_error(root, relative, backup, rollback)
}

/// Apply every file in a prepared plan as one source transaction. All output
/// is staged before the first source path changes; any commit error restores
/// every source path already replaced.
pub fn apply_codemod(plan: &CodemodPlan) -> anyhow::Result<CodemodApplyResult> {
    if plan.files.is_empty() {
        return Ok(CodemodApplyResult {
            files_changed: 0,
            edits_applied: 0,
        });
    }

    let root_dir = Dir::open_ambient_dir(&plan.root, ambient_authority())
        .with_context(|| format!("failed to open project root: {}", plan.root.display()))?;
    let mut staged = Vec::with_capacity(plan.files.len());
    for (index, file) in plan.files.iter().enumerate() {
        let prepared = (|| -> anyhow::Result<StagedFile> {
            let relative = confined_relative_path(&file.path)?.to_path_buf();
            let current = root_dir
                .read_to_string(&relative)
                .with_context(|| format!("failed to verify {} before apply", file.path))?;
            if current != file.original {
                bail!("source changed after codemod planning: {}", file.path);
            }
            let permissions = root_dir.metadata(&relative)?.permissions();
            // Plan→apply TOCTOU on writability. A target chmod'd read-only
            // after planning must fail closed here: the staged temp+rename
            // commit only needs the writable parent and would silently
            // bypass the file's permission (exit 0, content swapped, 0444
            // mode preserved — the reference refuses the same state).
            // Staging precedes every swap, so this refusal leaves the tree
            // byte-identical; cleanup_staged removes any partially staged
            // files. This staging-time read alone does NOT close the race
            // — a chmod can still land between it and the swap — so the
            // swap loop re-verifies the mode immediately before each rename
            // (see `swap_staged_files`). Writability is decided only from
            // fresh reads at each of the three checkpoints; plan time reads
            // `std` metadata captured at file open, staging and swap read
            // fresh cap-std fstatat results — both fold to the same
            // `mode & 0o200` predicate with no cached mode anywhere.
            if target_refuses_writes(&permissions) {
                bail!(
                    "refusing to rewrite read-only target file {}: the \
                     owner-write bit is clear and the staged-rename commit \
                     would bypass the file's write permission (sg refuses the \
                     same state with `Cannot rewrite file: Permission denied`); \
                     chmod u+w and re-plan",
                    file.path
                );
            }
            let staged_path = write_staged_file(&root_dir, &relative, &file.rewritten, index)?;
            if let Err(error) = root_dir.set_permissions(&staged_path, permissions) {
                let _ = root_dir.remove_file(&staged_path);
                return Err(error).with_context(|| format!("failed to stage {}", file.path));
            }
            Ok(StagedFile {
                relative,
                staged: staged_path,
                backup: None,
            })
        })();
        match prepared {
            Ok(prepared) => staged.push(prepared),
            Err(error) => {
                cleanup_staged(&root_dir, &staged);
                return Err(error);
            }
        }
    }

    swap_staged_files(&root_dir, plan, &mut staged)?;

    for file in &staged {
        if let Some(backup) = &file.backup {
            let _ = root_dir.remove_file(backup);
        }
    }
    // The hard-link commit leaves each staged name beside its source (both
    // names referenced the fsynced inode); unlink the extra names now.
    cleanup_staged(&root_dir, &staged);
    Ok(CodemodApplyResult {
        files_changed: plan.files_changed,
        edits_applied: plan.edit_count,
    })
}

/// The second half of `apply_codemod`'s transaction: every file is already
/// staged (source bytes verified, staged sibling written and fsynced); this
/// loop swaps each staged file into place. Any error rolls back every swap
/// committed so far and removes the staged siblings, so the tree is left
/// byte-identical to the pre-apply state. Extracted verbatim from the inline
/// loop so the swap-time contract is unit-drivable on a hand-constructed
/// post-staging state — the same seam pattern the `commit_race_tests`
/// module uses for the capture->commit gap.
pub fn swap_staged_files(
    root_dir: &Dir,
    plan: &CodemodPlan,
    staged: &mut [StagedFile],
) -> anyhow::Result<()> {
    for index in 0..staged.len() {
        // Plan-time reads are O_NOFOLLOW but apply-time verification
        // follows final-component symlinks whose destination stays inside
        // the root. A file swapped for an in-root symlink between plan and
        // apply would pass verification, get renamed into the backup slot,
        // and be deleted by success cleanup. Fail closed instead.
        let is_symlink = root_dir
            .symlink_metadata(&staged[index].relative)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_symlink {
            // This refusal sits INSIDE the swap loop, so it must restore the
            // pre-apply tree like any other commit failure.
            let rollback = rollback_committed(root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(swap_loop_error(
                anyhow::anyhow!(
                    "source changed after codemod planning: {} is now a symlink",
                    staged[index].relative.display()
                ),
                rollback,
            ));
        }
        // Verification happened once per file during staging, but the swap
        // loop runs afterwards — a concurrent writer can land in between
        // with no error (silent lost update). Re-read each source
        // immediately before its swap; anything other than the planned
        // original refuses the whole transaction. The re-read itself can
        // fail (concurrent removal / EACCES / EIO); that failure is an
        // error arm like any other and must roll back every swap committed
        // so far — a bare `?` here left the tree mid-transaction with
        // sidecar litter.
        let current = match root_dir.read_to_string(&staged[index].relative) {
            Ok(text) => text,
            Err(error) => {
                let rollback = rollback_committed(root_dir, staged, index);
                cleanup_staged(root_dir, staged);
                return Err(swap_loop_error(
                    anyhow::Error::new(error).context(format!(
                        "failed to re-read {} before swap",
                        plan.files[index].path
                    )),
                    rollback,
                ));
            }
        };
        if current != plan.files[index].original {
            let rollback = rollback_committed(root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(swap_loop_error(
                anyhow::anyhow!(
                    "source changed after codemod planning: {}",
                    plan.files[index].path
                ),
                rollback,
            ));
        }
        // The staging guard re-verifies the target's writability, but a
        // chmod landing between that read and this point — all of staging
        // plus every earlier file's swap — went through the rename anyway
        // (0444 target swapped, exit 0). The swap-time re-verify re-read
        // CONTENT but not MODE. Re-read the mode from live metadata
        // immediately before the capture rename, mirroring the content
        // re-verify: refuse the whole transaction when the owner-write bit
        // is clear. This check is the last one before the swap, so the true
        // invariant is: mode is re-verified at THREE points over fresh
        // reads (plan open, staging, swap), never from a cached value; the
        // residual window is only the kernel-level chmod between this
        // fstatat and the adjacent rename syscalls — the same race class as
        // the reference's in-place write, bounded to adjacent syscalls and
        // not removable without file locking.
        // The re-check must never propagate a stat FAILURE with `?` — that
        // early-returned after earlier files' swaps had committed,
        // violating this loop's all-or-nothing contract (the refusal arm
        // below was verified-correct; this error arm leaked). Fold it into
        // the same rollback+cleanup+rider hygiene.
        let swap_permissions = match swap_time_metadata(root_dir, &staged[index].relative) {
            Ok(metadata) => metadata.permissions(),
            Err(error) => {
                let rollback = rollback_committed(root_dir, staged, index);
                cleanup_staged(root_dir, staged);
                return Err(swap_loop_error(
                    anyhow::Error::new(error).context(format!(
                        "failed to stat {} before swap",
                        plan.files[index].path
                    )),
                    rollback,
                ));
            }
        };
        if target_refuses_writes(&swap_permissions) {
            let rollback = rollback_committed(root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(swap_loop_error(
                anyhow::anyhow!(
                    "refusing to rewrite read-only target file {}: the \
                     owner-write bit is clear and the staged-rename commit \
                     would bypass the file's write permission (sg refuses the \
                     same state with `Cannot rewrite file: Permission \
                     denied`); chmod u+w and re-plan",
                    plan.files[index].path
                ),
                rollback,
            ));
        }
        // Audit rider: this `?` was the last unpaired early exit in the
        // loop. Unreachable in practice for confined UTF-8 relative paths
        // (parent and file name always exist, the clock read is
        // unwrap_or_default), but the contract is structural: no `?` may
        // bypass rollback inside the swap loop.
        let backup = match unique_sibling_path(&staged[index].relative, "backup", index) {
            Ok(backup) => backup,
            Err(error) => {
                let rollback = rollback_committed(root_dir, staged, index);
                cleanup_staged(root_dir, staged);
                return Err(swap_loop_error(error, rollback));
            }
        };
        if let Err(error) = root_dir.rename(&staged[index].relative, root_dir, &backup) {
            let rollback = rollback_committed(root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(transaction_error(
                error,
                rollback,
                &plan.root.join(&staged[index].relative),
            ));
        }
        staged[index].backup = Some(backup.clone());
        // Commit via create-if-absent hard link, not an atomic REPLACE.
        // rename(staged -> relative) silently replaces a file a concurrent
        // writer created inside the between-renames gap (rename(source ->
        // backup) leaves the path vacant for an instant; the writer's
        // fs::write re-created it there; the replace rename destroyed the
        // fresh write with no capture anywhere — a lost update reported as
        // Ok). hard_link fails with AlreadyExists when the path exists, so
        // a writer landing in the gap is detected and its content
        // preserved.
        match root_dir.hard_link(&staged[index].staged, root_dir, &staged[index].relative) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A concurrent writer recreated the path after the capture
                // rename moved it away. The refusal keeps the writer's
                // content at the source path AND the capture sidecar (the
                // only pre-race copy) on disk, naming it — the historical
                // behavior deleted the sidecar outright.
                return Err(commit_race_writer_won(
                    root_dir,
                    &plan.root,
                    staged,
                    index,
                    &plan.files[index].path,
                    &backup,
                ));
            }
            Err(link_error) => {
                // This filesystem does not support hard links (or failed
                // some other way): degrade to the historical atomic-replace
                // rename. Its between-renames gap is then uncaptured — the
                // documented degradation, never silently data-losing on a
                // hardlink-capable filesystem.
                if let Err(error) =
                    root_dir.rename(&staged[index].staged, root_dir, &staged[index].relative)
                {
                    let restore_current = root_dir
                        .rename(&backup, root_dir, &staged[index].relative)
                        .err();
                    staged[index].backup = None;
                    let rollback = rollback_committed(root_dir, staged, index).or(restore_current);
                    cleanup_staged(root_dir, staged);
                    return Err(transaction_error(
                        error,
                        rollback,
                        &plan.root.join(&staged[index].relative),
                    ));
                }
                let _ = link_error;
            }
        }
        // rename(source -> backup) above is an atomic capture of whatever
        // the path held at swap time, so the verify-read earlier in this
        // iteration still leaves a read->capture gap a concurrent writer
        // can win: its content lands in the backup slot and success cleanup
        // would delete the only copy — a silent lost update reported as Ok.
        // Compare the captured backup against the planned original: any
        // difference is a concurrent write inside the window.
        // Restore that content at the source path (the backup slot holds the
        // only copy), roll back every earlier committed swap, and refuse
        // loudly. (The capture->commit gap is closed by the create-if-absent
        // hard link commit above; after commit, a racing writer merely
        // supersedes the applied content through its own write — no apply
        // destruction remains on hardlink-capable filesystems.)
        let captured = match root_dir.read_to_string(&backup) {
            Ok(text) => text,
            Err(error) => {
                let mut rollback = rollback_committed(root_dir, staged, index);
                match root_dir.rename(&backup, root_dir, &staged[index].relative) {
                    Ok(()) => staged[index].backup = None,
                    Err(restore_error) => {
                        rollback.get_or_insert(restore_error);
                    }
                }
                cleanup_staged(root_dir, staged);
                return Err(transaction_error(
                    error,
                    rollback,
                    &plan.root.join(&staged[index].relative),
                ));
            }
        };
        if captured != plan.files[index].original {
            let mut rollback = None;
            match root_dir.rename(&backup, root_dir, &staged[index].relative) {
                Ok(()) => staged[index].backup = None,
                // Restore failed: the concurrent content now survives only in
                // the backup sidecar. Keep it on disk and name it in the
                // error rather than destroying it for tidiness.
                Err(error) => rollback = Some(error),
            }
            if let Some(error) = rollback_committed(root_dir, staged, index) {
                rollback.get_or_insert(error);
            }
            cleanup_staged(root_dir, staged);
            let sidecar_note = match staged[index].backup {
                // Restore failed: the concurrent content survives only in the
                // backup sidecar now.
                Some(ref leftover) => format!(
                    " (the concurrent content remains in the backup sidecar {})",
                    plan.root.join(leftover).display()
                ),
                None => String::new(),
            };
            return Err(match rollback {
                Some(rb) => anyhow::anyhow!(
                    "source changed after codemod planning: {}; a concurrent \
                     write landed inside the apply window; rollback also \
                     failed: {rb}{sidecar_note}",
                    plan.files[index].path
                ),
                None => anyhow::anyhow!(
                    "source changed after codemod planning: {}; a concurrent \
                     write landed inside the apply window; its content is \
                     restored at the source path and all codemod changes are \
                     rolled back{sidecar_note}",
                    plan.files[index].path
                ),
            });
        }
    }
    Ok(())
}

/// The swap-time mode re-check's fstatat. No portable on-disk state makes
/// this stat fail AFTER the content re-read immediately before it succeeded
/// — anything that defeats stat defeats open first — so the stat-error arm
/// cannot be driven by a real fixture the way the read-only refusal is.
/// A one-shot injection seam covers it; the queue is always empty outside
/// tests, so production pays one thread-local lookup per swap.
fn swap_time_metadata(root_dir: &Dir, relative: &Path) -> std::io::Result<cap_std::fs::Metadata> {
    if let Some(error) = take_injected_swap_stat_failure(relative) {
        return Err(error);
    }
    let _ = relative;
    root_dir.metadata(relative)
}

thread_local! {
    /// Armed one-shot stat failures keyed by the source file's final
    /// component. thread_local because libtest runs each test on its own
    /// thread; armed entries never leak across tests.
    static INJECTED_SWAP_STAT_FAILURES:
        std::cell::RefCell<Vec<(String, std::io::Error)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn take_injected_swap_stat_failure(relative: &Path) -> Option<std::io::Error> {
    let name = relative.file_name()?.to_string_lossy().into_owned();
    INJECTED_SWAP_STAT_FAILURES.with(|armed| {
        let mut queue = armed.borrow_mut();
        let position = queue.iter().position(|(target, _)| *target == name)?;
        Some(queue.remove(position).1)
    })
}

/// Test seam for the stat-error arm of [`swap_time_metadata`]: arms a one-shot
/// failure returned for the next swap of the named file. Public because the
/// swap-race suite lives in integration tests; production never arms it.
pub fn arm_injected_swap_stat_failure(file_name: &str, error: std::io::Error) {
    INJECTED_SWAP_STAT_FAILURES.with(|armed| {
        armed.borrow_mut().push((file_name.to_string(), error));
    });
}

pub struct StagedFile {
    pub relative: PathBuf,
    pub staged: PathBuf,
    pub backup: Option<PathBuf>,
}

pub fn write_staged_file(
    root_dir: &Dir,
    path: &Path,
    contents: &str,
    index: usize,
) -> anyhow::Result<PathBuf> {
    for attempt in 0..100 {
        let staged = unique_sibling_path(path, "stage", index * 100 + attempt)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        match root_dir.open_with(&staged, &options) {
            Ok(mut file) => {
                let write_result = file
                    .write_all(contents.as_bytes())
                    .and_then(|()| file.sync_all());
                match write_result {
                    Ok(()) => return Ok(staged),
                    Err(error) => {
                        drop(file);
                        let _ = root_dir.remove_file(staged);
                        return Err(error.into());
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!(
        "could not allocate a staging file beside {}",
        path.display()
    )
}

fn unique_sibling_path(path: &Path, role: &str, nonce: usize) -> anyhow::Result<PathBuf> {
    let parent = path
        .parent()
        .with_context(|| format!("source path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("source path is not UTF-8: {}", path.display()))?;
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(parent.join(format!(
        ".{file_name}.asgrep-codemod-{role}-{}-{clock}-{nonce}",
        std::process::id()
    )))
}

fn rollback_committed(
    root_dir: &Dir,
    staged: &mut [StagedFile],
    count: usize,
) -> Option<std::io::Error> {
    let mut first_error = None;
    for file in staged[..count].iter_mut().rev() {
        let Some(backup) = file.backup.take() else {
            continue;
        };
        // rename(backup -> path) replaces any existing file atomically on
        // POSIX. The previous remove_file-then-rename sequence had a crash
        // window that left the path missing AND the edited content destroyed.
        if let Err(error) = root_dir.rename(backup, root_dir, &file.relative) {
            first_error.get_or_insert(error);
        }
    }
    first_error
}

/// Heal a tree left inconsistent by an apply process that died mid-swap.
/// For every planned path, restore the newest orphaned backup when the
/// canonical file is gone, then delete stale stage/backup leftovers so
/// re-runs recover instead of failing verification with ENOENT.
pub(crate) fn recover_orphans(root: &Path, planned_paths: &[String]) -> anyhow::Result<()> {
    for path in planned_paths {
        let relative = confined_relative_path(path)?;
        let full = root.join(relative);
        if full.symlink_metadata().is_ok() {
            // Canonical file present: nothing to heal at this path. Stale
            // backups beside a live file are left alone here — they are
            // removed by normal success cleanup of their own apply.
            continue;
        }
        let Some(parent) = relative.parent() else {
            continue;
        };
        let parent_full = root.join(parent);
        let mut orphans: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(&parent_full)
            .with_context(|| format!("failed to scan {}", parent_full.display()))?
            .filter_map(|e| e.ok())
        {
            let file_name = entry.file_name();
            if is_codemod_sidecar(file_name.to_string_lossy().as_ref(), "backup") {
                orphans.push(file_name.into());
            }
        }
        // "Newest" is a TIMESTAMP question. The sidecar name is
        // `.{file}.asgrep-codemod-backup-{pid}-{clock}-{nonce}` and pid
        // precedes clock, so lexicographic order is temporal garbage across
        // processes (a large pid buries a recent clock). Sort by the
        // sidecar's mtime (name as the deterministic tiebreak) so the newest
        // backup is the one restored; the rest are swept as leftovers.
        orphans.sort_by(|left, right| {
            let modified = |path: &PathBuf| {
                fs::metadata(parent_full.join(path))
                    .and_then(|meta| meta.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH)
            };
            modified(left)
                .cmp(&modified(right))
                .then_with(|| left.cmp(right))
        });
        if let Some(newest) = orphans.pop() {
            let candidate = parent_full.join(newest);
            // Restore only if the sidecar is a regular file holding complete
            // content (it was fsynced before the swap that died).
            if candidate.symlink_metadata()?.is_file() {
                fs::rename(candidate, &full)?;
            }
        }
        cleanup_leftovers(&parent_full);
    }
    Ok(())
}

/// Remove stale `.name.asgrep-codemod-{stage,backup}-*` sidecars beside `path`
/// whose canonical file exists (or after its backup has been restored).
fn cleanup_leftovers(parent_full: &Path) {
    let Ok(entries) = fs::read_dir(parent_full) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().as_ref().to_owned();
        if !is_codemod_sidecar(&name, "stage") && !is_codemod_sidecar(&name, "backup") {
            continue;
        }
        let _ = fs::remove_file(parent_full.join(&file_name));
    }
}

/// Match `.name.asgrep-codemod-{role}-*` sidecar names (any pid/clock/nonce tail).
fn is_codemod_sidecar(file_name: &str, role: &str) -> bool {
    let Some(rest) = file_name.strip_prefix('.') else {
        return false;
    };
    let marker = ".asgrep-codemod-";
    let Some(marker_pos) = rest.find(marker) else {
        return false;
    };
    let after_marker = &rest[marker_pos + marker.len()..];
    match after_marker.split_once('-') {
        Some((found_role, tail)) => found_role == role && !tail.is_empty(),
        None => false,
    }
}

fn cleanup_staged(root_dir: &Dir, staged: &[StagedFile]) {
    let mut paths = BTreeSet::new();
    for file in staged {
        paths.insert(file.staged.clone());
    }
    for path in paths {
        let _ = root_dir.remove_file(path);
    }
}

/// The transaction rollback rider applied to an already-contextualized
/// error. Every mid-loop early exit in `swap_staged_files` must pair
/// `rollback_committed` + `cleanup_staged` with this rider — the function's
/// doc contract ("any error rolls back every swap committed so far and
/// removes the staged siblings") holds only if no `?` can leak past them.
fn swap_loop_error(commit: anyhow::Error, rollback: Option<std::io::Error>) -> anyhow::Error {
    match rollback {
        Some(rb) => anyhow::anyhow!("{commit}; rollback also failed: {rb}"),
        None => anyhow::anyhow!("{commit}; all changes rolled back"),
    }
}

fn transaction_error(
    commit: std::io::Error,
    rollback: Option<std::io::Error>,
    path: &Path,
) -> anyhow::Error {
    let base = anyhow::anyhow!("failed to apply {}: {commit}", path.display());
    swap_loop_error(base, rollback)
}
