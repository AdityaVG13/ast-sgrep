# ast-sgrep codemod edit-path audit — FINAL CONSOLIDATED FINDINGS

Repo `/Users/aditya/Developer/ast-sgrep`, branch `fix/bun-sqlite-and-auto-index`, clean tree,
read-only audit. Deliverable of record. Tests executed: `cargo test -p ast-sgrep-cli --test
cli_smoke codemod_` → 2 passed / 0 failed (happy-path apply + parent-symlink-swap refusal).
No files in the repo were modified.

## Numbered findings

**F1 · HIGH · TOCTOU between content verification and rename swap ⇒ silent lost update.**
`crates/ast-sgrep-core/src/codemod.rs:178-183` vs `:205-232`. Each file's `current ==
file.original` check runs inside the STAGING loop, but its rename swap runs in a second loop
that only begins after ALL files are staged (each stage does `sync_all`, `:356-358`). For
file i the unprotected window is (staging of i+1..N) + (swaps of 0..i-1) — sub-second for
small repos, seconds for many-file applies or slow disks. A concurrent writer inside that
window (IDE autosave, format-on-save daemon, `git checkout`, this product's own watch mode)
is overwritten by the stale rewrite with NO error: apply reports success. Trigger class:
concurrent modification of any target file during a multi-file apply.
RED fixture sketch: unit test builds a 2-file plan; helper thread rewrites file 1 once file
0's stage exists (widen deterministically by making file 0 multi-MB); assert file 1's
concurrent line survives apply — it currently will not. Fix shape: re-read/lstat immediately
before each source→backup rename, or fold verify+swap per-file.

**F2 · MEDIUM · Process death mid-swap leaves the user file MISSING from its path; no
crash recovery, no directory fsync, re-run does not heal.**
`codemod.rs:207-231`: swap = `rename(source → .name.asgrep-codemod-backup-*)` then
`rename(staged → source)`; between them the canonical path is EMPTY. SIGKILL/power loss here
(or non-atomic persistence of the pair — renames are never followed by a parent-dir fsync)
leaves only dotfiles behind. Grep confirms NO reference to `asgrep-codemod-{backup,stage}`
anywhere outside codemod.rs — no recovery sweep, no docs. Re-running the CLI then fails with
"failed to verify … before apply" (ENOENT) instead of restoring. `agent.rs:113`'s
"transactional" wording oversells this: the transaction guards in-process errors only.
Trigger class: kill -9 / crash / power loss during apply.
RED fixture sketch: spawn `asgrep codemod` on an N-file repo in a loop, kill -9 at random
offsets, assert every planned path always exists as a regular file — violations appear;
then re-run codemod and observe hard failure instead of recovery. Fix shape: fsync parent
dir after each rename pair; ship an orphan-recovery sweep (or rename staged→source directly
over the old inode on POSIX).

**F3 · MEDIUM · The ROLLBACK path itself can delete the file (remove-then-rename).**
`codemod.rs:396-415`: restore = `remove_file(new)` (`:406`) then `rename(backup → relative)`
(`:410`). Death/failure between the two (EIO/ENOSPC on the rename; only recorded in
`first_error`) leaves the path empty AND the edited content destroyed — strictly worse than
the failure being rolled back. POSIX allows renaming over an existing file, so the removal
is unnecessary on this platform. Trigger class: I/O fault or crash during rollback of any
commit error.
RED fixture sketch: fault-injecting FS returning EIO on the Nth rename during a forced
commit failure; assert the target path always holds either old or new content — currently
it can hold nothing. Fix shape: `rename(backup → relative)` directly over the new file;
fall back to remove-then-rename only where rename-over fails (Windows), then fsync.

**F4 · MEDIUM · Plan-time O_NOFOLLOW vs apply-time follow-enabled reads: a final-component
symlink swapped in mid-flight is silently DESTROYED and the file reported changed.**
Plan reads use `RootDir::read_text_capped` with O_NOFOLLOW on every component
(`io_bounds.rs:68-75`; Windows `FollowSymlinks::No` `:103`), so symlinks cannot exist at
plan time (indexer strips them anyway, `index.rs:779-784`). Apply-time verification uses
cap-std 4.0.2 `Dir::read_to_string`, which FOLLOWS final-component symlinks whose
destination stays inside the root (cap-primitives `manually/open.rs`
`maybe_last_component_symlink`; escapes rejected at `open.rs:426/473`; Linux openat2
RESOLVE_BENEATH likewise permits in-root links). Trigger: between plan and that file's
rename, `rm src/a.rs && ln -s ../lib/a.rs src/a.rs` with identical current content —
verification passes, `rename(src/a.rs → backup)` moves the SYMLINK, the staged regular file
takes its place, and success cleanup deletes the backup (`codemod.rs:234-238`): symlink
permanently gone, `lib/shared` target never edited, exit status success. Same window as F1,
lying-success outcome.
RED fixture sketch: same as F1 but the racing thread swaps in an in-root symlink to an
identical-content sibling; assert the leaf is still a symlink and the sibling was edited —
both fail today. Fix shape: `symlink_metadata`/O_NOFOLLOW leaf check immediately before
each rename.

**F5 · LOW · Index-derived file list makes codemods incomplete-by-stealth, and one stale
entry aborts everything.**
`codemod.rs:85-101`: the candidate set is `store.all_file_paths()` (`queries.rs:61`,
deterministic order). Files created after the last `asgrep index` are silently skipped and
the apply still reports full success (no freshness warning). Conversely ONE missing/
oversized/non-UTF8/symlinked indexed file hard-errors the ENTIRE plan ("failed to read
indexed file", no hint to reindex). Fail-closed, but brittle and quietly incomplete.
Trigger: touch a matching new file, or delete any indexed file, then run codemod.

**F6 · LOW · Rewrite-template edge cases.** `interpolate_rewrite` (`codemod.rs:285-304`):
`$$$$` bails "invalid metavariable" instead of emitting two literal `$`; `$$$name` binds
`name` while `$$name` emits literal text — undocumented and easy to trip. Capture values are
inserted verbatim with no re-expansion (verified safe).

**F7 · LOW · Content-fidelity nits.** A match spanning byte 0 folds the BOM into `before`,
so rewriting strips the BOM; rewrite templates containing `\n` insert LF into CRLF files
(mixed EOL). Untouched bytes are otherwise preserved exactly.

**F8 · LOW · Unbounded verify read + unfingerprinted dry-run output.**
Apply verification `root_dir.read_to_string` (`codemod.rs:178-180`) has no size cap — a
target grown huge between plan and apply is fully read before the mismatch bail (memory DoS,
local-only). Dry-run JSON (`codemod_cmd.rs:22-27`) exposes edits without per-file mtime/hash
fingerprints, so third parties replaying the printed plan have no staleness check.

## Ruled-out checklist (inspected, refuted)
- Overlapping/nested/duplicate match spans — `validate_non_overlapping` (`:257-270`) correct
  on sorted matches; touching spans correctly allowed.
- Offset drift across multiple edits in one file — `apply_edits` (`:316-336`) single pass
  against the original buffer; no sequential substitution.
- Empty-diff / lying counts — identity edits skipped (`:123`); `CodemodApplyResult` mirrors
  the atomic plan, unattainable on failure paths; index-refresh failure reported honestly
  (`codemod_cmd.rs:39-45`).
- Encoding corruption — strict UTF-8 everywhere (io_bounds `read_to_string` errors InvalidData;
  cap-std ditto); no truncation (over-cap errors); no lossy round-trip; non-UTF-8 fail-closed.
- Path traversal/injection — `confined_relative_path` (`:245-255`) rejects absolute/`..`/`.`
  components; sibling dotfile names pid+nanos+nonce with `create_new` retry; cap-std rejects
  symlink escapes (covered by passing test).
- Double-apply — second apply fails verification (content differs); CLI re-run re-plans.
- Plan-over-JSON hazard — `#[serde(skip)] original/rewritten` never cross a boundary:
  codemode tools/adapters/batch/napi/MCP expose no edit tool; CLI is the only writer.
- Intermediate-component symlink/junction swap at apply — covered by
  `codemod_apply_refuses_parent_symlink_swap` (passing) + cap-std RESOLVE_BENEATH /
  escape_attempt checks; residual risk is only F4's final component.
- Ordering nondeterminism — `ORDER BY path`.
- Permission loss — staged files inherit source permissions (`:184-189`), failure cleans up.

## Verification status (honesty note)
F1-F4 are established by code reading plus cap-primitives 4.0.2 source inspection; no live
race/crash reproduction was run (requires fault injection; repo untouched per audit rules).
RED fixtures above are sketches, deliberately not implemented. Test budget used:
1 command / 2 named suites of the allowed 5.

Checkpoint history: cp1 = core codemod.rs, cp2 = callers/wiring, cp3 = this consolidation.
