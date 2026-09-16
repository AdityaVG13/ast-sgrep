//! Indexed, in-process structural codemod planning and transactional apply.

use crate::io_bounds::{RootDir, MAX_INDEX_FILE_BYTES};
use crate::IndexStore;
use anyhow::{bail, Context};
use ast_sgrep_lang::{
    classify_native, detect_language, match_pattern, required_pattern_literal, Language,
    PatternMatch,
};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct CodemodEdit {
    pub path: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: u32,
    pub line_end: u32,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodemodFilePlan {
    pub path: String,
    pub edits: Vec<CodemodEdit>,
    #[serde(skip)]
    original: String,
    #[serde(skip)]
    rewritten: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodemodPlan {
    pub pattern: String,
    pub rewrite: String,
    pub files_changed: usize,
    pub edit_count: usize,
    pub files: Vec<CodemodFilePlan>,
    /// F66a-9 (pass 67b): planned-for files whose owner-write bit is clear,
    /// refused sg-agreed (sg's update-all skips them with `Cannot rewrite
    /// file … Permission denied` and exits 6) instead of being rewritten
    /// through the staged temp+rename commit, which only needs the writable
    /// PARENT directory and would otherwise silently swap a `chmod 444`
    /// file's content while keeping its read-only mode. Serialized so the
    /// dry-run preview shows exactly what apply will do (F66a-10 contract).
    pub read_only_refused: Vec<String>,
    #[serde(skip)]
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodemodApplyResult {
    pub files_changed: usize,
    pub edits_applied: usize,
}

impl CodemodPlan {
    pub fn changed_paths(&self) -> Vec<PathBuf> {
        self.files
            .iter()
            .map(|file| self.root.join(&file.path))
            .collect()
    }
}

/// Build a deterministic edit plan from files already present in the index.
/// Unsupported structural patterns are rejected rather than delegated to an
/// external `ast-grep` executable.
///
/// `lang_filter` honors `--lang` exactly like the search path (EXP-001,
/// H-CONF-011): stored id, file extension, or alias; `None` plans everywhere.
pub fn plan_codemod(
    root: &Path,
    index_path: Option<&Path>,
    lang_filter: Option<&str>,
    pattern: &str,
    rewrite: &str,
) -> anyhow::Result<CodemodPlan> {
    // FB-80a-04 (pass 80a, r31): the search lane's query grammar strips ONE
    // optional leading `pattern:` token (ParsedQuery::parse_mode, and the
    // multi-pattern ingress tolerates the same spelling per value); the
    // codemod lane never did, so `--pattern='pattern:echo "a b";'` handed
    // the raw prefixed text to the matcher as literal pattern code,
    // match-None'd, and silently planned zero edits on exactly the faces
    // search serves — the prefix even carried `$`-bearing spellings past the
    // classifier gate below and past the emptiness refusal (`pattern:`).
    // Normalize the ingress so both lanes see one pattern spelling; the
    // emptiness check and the `$`-gate keep judging the stripped spelling.
    // 82c-2 (pass 82c, r33): the strip ORDER must match the search lane's —
    // search strips the prefix FIRST (parse_mode) and the BOM later
    // (search_pattern's H-CONF-032 strip). With the BOM strip running first,
    // a BOM AFTER the prefix token (`pattern:<BOM>`) was neither BOM-stripped
    // (no longer leading) nor caught by the emptiness check (U+FEFF is not
    // `str` whitespace): codemod silently planned a zero-edit codemod on the
    // exact string where search refuses loudly. Prefix first, then BOM, then
    // trim, so every ingress spelling lands in the same loud/silent class in
    // both lanes.
    let pattern = pattern.trim();
    let pattern = pattern
        .strip_prefix("pattern:")
        .map(str::trim)
        .unwrap_or(pattern);
    // H-CONF-032 (pass 63) + PASS 65 (LOW a): the BOM strip runs BEFORE the
    // empty check — the raw U+FEFF is not whitespace, so a BOM-only pattern
    // previously slipped the guard, survived as an empty string, and
    // planned a nonsense zero-edit codemod instead of refusing loudly.
    let pattern = pattern.trim_start_matches('\u{feff}').trim();
    if pattern.is_empty() {
        bail!("codemod pattern must not be empty");
    }
    if pattern.contains('$') && classify_native(pattern).is_none() {
        bail!("pattern is not supported by the in-process structural matcher");
    }

    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root: {}", root.display()))?;
    let root_dir = RootDir::open(&root)?;
    // F76c-2 (r26 pass 77a): planning only reads the store (root binding,
    // indexed paths, `pattern_nodes_matching_limited`), so open READ-ONLY —
    // a writable open would silently fire the row-discarding schema-15
    // migration as a side effect of a planning read, demoting the index
    // (pattern_nodes wiped, hashes prefixed) with no re-extract and no
    // marker. Read-only gains the loud pre-current gate for free, symmetric
    // with search: the refusal names `asgrep reindex`, the honest remedy.
    let store = IndexStore::open_readonly(&root, index_path)?;
    // H-CONF-033 (pass 63): read-side root binding, same contract as
    // `Searcher::new` — a db stamped for another root must never answer
    // (let alone rewrite files) for this tree. Cross-root REINDEX stays the
    // designed prune-replace (GA-12); only the read is refused.
    if let Some(bound) = store.get_meta("root")? {
        let bound = bound.trim_end_matches('/');
        let here = root.display().to_string();
        let here = here.trim_end_matches('/');
        if bound != here {
            let index_desc = index_path
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "the default index path".to_string());
            bail!(
                "index is bound to a different project root: index at {index_desc} was built \
                 for root {bound}, but this codemod is rooted at {here}; re-index {here} \
                 (asgrep index) or run the codemod against {bound}"
            );
        }
    }
    let indexed_paths = store.all_file_paths()?;
    // br-1xx: heal a tree left inconsistent by an apply process that died
    // mid-swap (canonical path missing, orphaned `.name.asgrep-codemod-backup-*`
    // beside it) BEFORE reading the planned files, so a re-run recovers the
    // previous content instead of failing verification with ENOENT. Runs on
    // std::fs because planning has no Dir handle yet; `root` is canonical and
    // sidecar names are matched by exact marker, so confinement holds.
    recover_orphans(&root, &indexed_paths)?;
    if indexed_paths.is_empty() {
        bail!(
            "index is empty for {}; run: asgrep index {} --json",
            root.display(),
            root.display()
        );
    }

    // FB-97B-2 (pass 98): the prefilter literal must not be computed on raw
    // bytes when the pattern carries an expando char — `µNAME + 1` yielded
    // the needle "NAME", a byte the meta reading (`$NAME + 1` — what
    // `match_pattern` answers after normalizing) does NOT require, so
    // µ-free files were silently dropped from the plan while the search lane
    // answered them (subject codemod planned b.rs only; sg rewrites 6 sites
    // across a/b/c.rs — matrix m3d). Guard with the same containment check
    // the two search lanes use (index lane + byte prefilter): over-broad
    // matches only disable the fast prefilter, never results.
    let required_literal = if crate::pattern::pattern_may_carry_expando_meta(pattern) {
        None
    } else {
        required_pattern_literal(pattern)
    };
    let lang_filter = Language::canonical_filter(lang_filter);
    let mut files = Vec::new();
    let mut read_only_refused = Vec::new();
    for rel_path in &indexed_paths {
        let rel = confined_relative_path(&rel_path)?;
        let capped = root_dir
            .read_text_capped(rel, MAX_INDEX_FILE_BYTES)
            .with_context(|| format!("failed to read indexed file {rel_path}"))?;
        let original = capped.text;
        if required_literal.as_ref().is_some_and(|literal| {
            memchr::memmem::find(original.as_bytes(), literal.as_bytes()).is_none()
        }) {
            continue;
        }
        let Some(language) = detect_language(rel, Some(&original)) else {
            continue;
        };
        // EXP-001 (H-CONF-011): the language filter is applied here, not only
        // in search, so a codemod plan can never cross the --lang boundary.
        if lang_filter
            .as_deref()
            .is_some_and(|want| language.as_str() != want)
        {
            continue;
        }
        let matches = match_pattern(language, &original, pattern)
            .with_context(|| format!("failed to match pattern in {rel_path}"))?;
        // F66a-7 (pass 66a): sg-compatible overlap resolution — structural
        // matches come from real tree nodes, so spans are identical, properly
        // nested, or disjoint; the OUTERMOST match wins and inner overlaps
        // are skipped per edit. sg `run --rewrite` on the same state rewrites
        // the outer span (`conn?.open()?.send(1)` → `log(conn?.open(), send)`),
        // leaves the inner row unrewritten, still applies every
        // non-overlapping edit, and exits 0. The historical whole-plan refusal
        // (`codemod matches overlap …`, exit 2) killed the unrelated edits in
        // the file where sg answered.
        let matches = keep_outermost_matches(matches);

        let mut edits = Vec::new();
        for matched in matches {
            let before = original
                .get(matched.byte_start..matched.byte_end)
                .with_context(|| format!("matcher returned an invalid byte span for {rel_path}"))?
                .to_string();
            let after = interpolate_rewrite(rewrite, &matched)
                .with_context(|| format!("invalid rewrite for {rel_path}"))?;
            if before == after {
                continue;
            }
            edits.push(CodemodEdit {
                path: rel_path.clone(),
                byte_start: matched.byte_start,
                byte_end: matched.byte_end,
                line_start: matched.line_start,
                line_end: matched.line_end,
                before,
                after,
            });
        }
        if edits.is_empty() {
            continue;
        }
        // F66a-9 (pass 66a): refuse a read-only target per file, sg-agreed.
        // The apply commit stages a temp sibling and swaps by rename/hard
        // link, which needs only the writable PARENT directory — the
        // historical path therefore rewrote a `chmod 444` file's content
        // (keeping its read-only mode, exit 0) where sg's update-all refuses
        // the file (`Cannot rewrite file … Permission denied`, os error 13)
        // and leaves it byte+mode intact. Excluding the file here — and
        // naming it — keeps the dry-run preview equal to what apply will do
        // (F66a-10 preview contract) and honors the file's writability.
        if target_refuses_writes(&capped.metadata.permissions()) {
            read_only_refused.push(rel_path.clone());
            continue;
        }
        let rewritten = apply_edits(&original, &edits);
        files.push(CodemodFilePlan {
            path: rel_path.clone(),
            edits,
            original,
            rewritten,
        });
    }
    let edit_count = files.iter().map(|file| file.edits.len()).sum();
    // H-CONF-023 honesty class (pass 56, H-AUDIT-52-1): search serves exact
    // ident and decl patterns from the index (`cached_pattern_signatures` +
    // `index_can_serve_pattern`), but the native matcher that produces codemod
    // edit spans answers match-none for concrete decl templates (`fn old_name`
    // is not a literal node the literal lane can span). A silent ok:true
    // zero-edit plan while search answers hits is a fail-open; refuse loudly
    // naming the limitation instead. Patterns the index cannot serve (and
    // therefore search also answers empty) keep the quiet ok:true zero plan.
    // PASS 65 (LOW b): the gate reads the CURRENT file bytes before
    // refusing — a stale index can still serve an identifier that a
    // concurrent edit already removed from the tree, and the refusal then
    // fired on a hit that no longer exists (false refuse). The same
    // `required_pattern_literal` prefilter the planning loop uses decides:
    // when NO indexed file still contains the pattern's required literal,
    // the tree is authoritative and the zero-edit plan is honest.
    // F66a-9: a zero-edit plan explained by named read-only refusals is NOT
    // a silent fail-open — the envelope names every refused file — so the
    // honesty gate below (which would misattribute the zero to the matcher)
    // only runs when nothing was refused.
    if edit_count == 0 && read_only_refused.is_empty() {
        if let Some(signatures) = ast_sgrep_lang::cached_pattern_signatures(pattern) {
            if ast_sgrep_lang::index_can_serve_pattern(pattern, &signatures) {
                let mut index_serves_any = false;
                for signature in &signatures {
                    let served =
                        store.pattern_nodes_matching_limited(signature, lang_filter.as_deref(), 1)?;
                    if !served.is_empty() {
                        index_serves_any = true;
                        break;
                    }
                }
                let literal_still_in_tree = match required_literal.as_ref() {
                    Some(literal) => {
                        let needle = literal.as_bytes();
                        indexed_paths.iter().any(|rel_path| {
                            let confined = confined_relative_path(rel_path);
                            let confined = match confined {
                                Ok(path) => path,
                                // Unprojectable path: assume the literal may
                                // still be present (conservative → refuse).
                                Err(_) => return true,
                            };
                            root_dir
                                .read_text_capped(confined, MAX_INDEX_FILE_BYTES)
                                .map(|capped| {
                                    memchr::memmem::find(capped.text.as_bytes(), needle).is_some()
                                })
                                // Read failure: assume present (same direction).
                                .unwrap_or(true)
                        })
                    }
                    // No prefilter literal: the matcher's match-none cannot be
                    // reconciled against bytes, so keep the loud refusal.
                    None => true,
                };
                // F4 (Sept 14 wave, H-CONF-024): depth-truncated files in scope
                // make the index non-authoritative — search answers via the
                // native walk while this planner saw no edit spans, so a quiet
                // zero-edit plan would contradict search. Refuse loudly. A
                // re-index does NOT clear the flag (the budget re-truncates the
                // same files), hence the verify-then-raise advice.
                if store.has_depth_truncated_files(lang_filter.as_deref())?
                    && literal_still_in_tree
                {
                    bail!(
                        "codemod cannot plan edits for pattern {pattern:?}: the index \
                         holds depth-truncated file(s) in scope, so the zero-edit \
                         plan is not trustworthy; run `search` to confirm hits, then \
                         rewrite with a literal or metavariable template, or raise \
                         the extraction depth budget and re-index"
                    );
                }
                if index_serves_any && literal_still_in_tree {
                    bail!(
                        "codemod cannot plan edits for pattern {pattern:?}: search \
                         serves it from the index but the in-process structural \
                         matcher produces no edit span; rewrite the identifier \
                         itself (e.g. the bare name) or use a metavariable \
                         template; if the tree changed since indexing, re-index \
                         and re-run"
                    );
                }
            }
        }
    }
    Ok(CodemodPlan {
        pattern: pattern.to_string(),
        rewrite: rewrite.to_string(),
        files_changed: files.len(),
        edit_count,
        files,
        read_only_refused,
        root,
    })
}

/// PASS 65 (P9c): build the refusal for a concurrent write winning the
/// capture->commit gap. The error NAMES the capture sidecar (the only copy
/// of the pre-swap content) so the operator can recover or discard it
/// deliberately. The helper never touches the filesystem; the caller keeps
/// the sidecar on disk by simply not deleting it (`cleanup_staged` only
/// removes staged paths, never backups). `pub` so the transaction-contract
/// tests can pin the exact refusal wording (`f64_race_capture_sidecar_is_
/// preserved_and_named`).
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

/// PASS 65 (LOW c): the commit's create-if-absent hard link lost the
/// capture->commit gap race — the source path exists again, holding a
/// concurrent writer's bytes. Invariants: the writer's content is the
/// authoritative newest state (restoring the capture would clobber it — the
/// exact lost update the hard-link commit exists to prevent), and the
/// capture sidecar now holds the ONLY copy of the pre-race content, so it
/// stays on disk and is named in the refusal. Never unlink the last
/// reference. Rolls back every earlier committed swap and removes the staged
/// files; the backup sidecar is deliberately untouched.
fn commit_race_writer_won(
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
            // F66a-9 (pass 67b): plan→apply TOCTOU on writability. A target
            // chmod'd read-only after planning must fail closed here: the
            // staged temp+rename commit only needs the writable parent and
            // would silently bypass the file's permission (the registered
            // 66a face: exit 0, content swapped, 0444 mode preserved — sg
            // refuses the same state). Staging precedes every swap, so this
            // refusal leaves the tree byte-identical; cleanup_staged removes
            // any partially staged files. PASS 69b (SOAK-68B-CW-1): this
            // staging-time read alone does NOT close the race — a chmod can
            // still land between it and the swap — so the swap loop
            // re-verifies the mode immediately before each rename (see
            // `swap_staged_files`). Writability is decided only from fresh
            // reads at each of the three checkpoints; plan time reads `std`
            // metadata captured at file open, staging and swap read fresh
            // cap-std fstatat results — both fold to the same `mode & 0o200`
            // predicate with no cached mode anywhere.
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
/// loop (PASS 69b, SOAK-68B-CW-1) so the swap-time contract is unit-drivable
/// on a hand-constructed post-staging state — the same seam pattern the
/// `commit_race_tests` module uses for the capture->commit gap.
fn swap_staged_files(
    root_dir: &Dir,
    plan: &CodemodPlan,
    staged: &mut [StagedFile],
) -> anyhow::Result<()> {
    for index in 0..staged.len() {
        // br-hbd: plan-time reads are O_NOFOLLOW but apply-time verification
        // follows final-component symlinks whose destination stays inside the
        // root. A file swapped for an in-root symlink between plan and apply
        // would pass verification, get renamed into the backup slot, and be
        // deleted by success cleanup. Fail closed instead.
        let is_symlink = root_dir
            .symlink_metadata(&staged[index].relative)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_symlink {
            // br-hbd follow-up: this refusal sits INSIDE the swap loop, so it
            // must restore the pre-apply tree like any other commit failure.
            let rollback = rollback_committed(&root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(match rollback {
                Some(rb) => anyhow::anyhow!(
                    "source changed after codemod planning: {} is now a symlink; \
                     rollback also failed: {rb}",
                    staged[index].relative.display()
                ),
                None => anyhow::anyhow!(
                    "source changed after codemod planning: {} is now a symlink; \
                     all changes rolled back",
                    staged[index].relative.display()
                ),
            });
        }
        // br-i04: verification happened once per file during staging, but the
        // swap loop runs afterwards — a concurrent writer can land in between
        // with no error (silent lost update). Re-read each source immediately
        // before its swap; anything other than the planned original refuses
        // the whole transaction. F2 (pass 71b): the re-read itself can fail
        // (concurrent removal / EACCES / EIO); that failure is an error arm
        // like any other and must roll back every swap committed so far — a
        // bare `?` here left the tree mid-transaction with sidecar litter.
        let current = match root_dir.read_to_string(&staged[index].relative) {
            Ok(text) => text,
            Err(error) => {
                let rollback = rollback_committed(&root_dir, staged, index);
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
            let rollback = rollback_committed(&root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(match rollback {
                Some(rb) => anyhow::anyhow!(
                    "source changed after codemod planning: {}; rollback also \
                     failed: {rb}",
                    plan.files[index].path
                ),
                None => anyhow::anyhow!(
                    "source changed after codemod planning: {}; all changes \
                     rolled back",
                    plan.files[index].path
                ),
            });
        }
        // PASS 69b (SOAK-68B-CW-1 / F-68c-3): the staging guard re-verifies
        // the target's writability, but a chmod landing between that read and
        // this point — all of staging plus every earlier file's swap — went
        // through the rename anyway (soak rep1: 0444 target swapped, exit 0,
        // on an unchanged binary). The swap-time re-verify re-read CONTENT
        // but not MODE. Re-read the mode from live metadata immediately
        // before the capture rename, mirroring the content re-verify: refuse
        // the whole transaction when the owner-write bit is clear. This check
        // is the last one before the swap, so the true invariant is: mode is
        // re-verified at THREE points over fresh reads (plan open, staging,
        // swap), never from a cached value; the residual window is only the
        // kernel-level chmod between this fstatat and the adjacent rename
        // syscalls — the same race class as sg's in-place write, bounded to
        // adjacent syscalls and not removable without file locking.
        // F2 (pass 71b, finding 70c-F2): the re-check must never propagate a
        // stat FAILURE with `?` — that early-returned after earlier files'
        // swaps had committed, violating this loop's all-or-nothing contract
        // (the refusal arm below was verified-correct; this error arm leaked).
        // Fold it into the same rollback+cleanup+rider hygiene.
        let swap_permissions = match swap_time_metadata(root_dir, &staged[index].relative) {
            Ok(metadata) => metadata.permissions(),
            Err(error) => {
                let rollback = rollback_committed(&root_dir, staged, index);
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
            let rollback = rollback_committed(&root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(match rollback {
                Some(rb) => anyhow::anyhow!(
                    "refusing to rewrite read-only target file {}: the \
                     owner-write bit is clear and the staged-rename commit \
                     would bypass the file's write permission (sg refuses the \
                     same state with `Cannot rewrite file: Permission \
                     denied`); chmod u+w and re-plan; rollback also failed: \
                     {rb}",
                    plan.files[index].path
                ),
                None => anyhow::anyhow!(
                    "refusing to rewrite read-only target file {}: the \
                     owner-write bit is clear and the staged-rename commit \
                     would bypass the file's write permission (sg refuses the \
                     same state with `Cannot rewrite file: Permission \
                     denied`); chmod u+w and re-plan; all changes rolled back",
                    plan.files[index].path
                ),
            });
        }
        // F2 audit rider (pass 71b): this `?` was the last unpaired early
        // exit in the loop. Unreachable in practice for confined UTF-8
        // relative paths (parent and file name always exist, the clock read
        // is unwrap_or_default), but the contract is structural: no `?` may
        // bypass rollback inside the swap loop.
        let backup = match unique_sibling_path(&staged[index].relative, "backup", index) {
            Ok(backup) => backup,
            Err(error) => {
                let rollback = rollback_committed(&root_dir, staged, index);
                cleanup_staged(root_dir, staged);
                return Err(swap_loop_error(error, rollback));
            }
        };
        if let Err(error) = root_dir.rename(&staged[index].relative, &root_dir, &backup) {
            let rollback = rollback_committed(&root_dir, staged, index);
            cleanup_staged(root_dir, staged);
            return Err(transaction_error(
                error,
                rollback,
                &plan.root.join(&staged[index].relative),
            ));
        }
        staged[index].backup = Some(backup.clone());
        // H-SOAK-59-1 (pass 61): commit via create-if-absent hard link, not
        // an atomic REPLACE. rename(staged -> relative) silently replaces a
        // file a concurrent writer created inside the between-renames gap
        // (realized on the pass-61 flake bound: rename(source -> backup)
        // leaves the path vacant for an instant; the writer's fs::write
        // re-created it there; the replace rename destroyed the fresh write
        // with no capture anywhere — a lost update reported as Ok).
        // hard_link fails with AlreadyExists when the path exists, so a
        // writer landing in the gap is detected and its content preserved.
        match root_dir.hard_link(&staged[index].staged, &root_dir, &staged[index].relative) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A concurrent writer recreated the path after the capture
                // rename moved it away. PASS 65 (LOW c): the refusal keeps
                // the writer's content at the source path AND the capture
                // sidecar (the only pre-race copy) on disk, naming it —
                // the historical behavior deleted the sidecar outright.
                return Err(commit_race_writer_won(
                    &root_dir,
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
                if let Err(error) = root_dir.rename(
                    &staged[index].staged,
                    &root_dir,
                    &staged[index].relative,
                ) {
                    let restore_current = root_dir
                        .rename(&backup, &root_dir, &staged[index].relative)
                        .err();
                    staged[index].backup = None;
                    let rollback = rollback_committed(&root_dir, staged, index).or(restore_current);
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
        // H-SOAK-59-1 (pass 61): rename(source -> backup) above is an atomic
        // capture of whatever the path held at swap time, so the verify-read
        // earlier in this iteration still leaves a read->capture gap a
        // concurrent writer can win: its content lands in the backup slot
        // and success cleanup would delete the only copy — a silent lost
        // update reported as Ok (pass-59 rep1 realized exactly this
        // interleaving). Compare the captured backup against the planned
        // original: any difference is a concurrent write inside the window.
        // Restore that content at the source path (the backup slot holds the
        // only copy), roll back every earlier committed swap, and refuse
        // loudly. (The capture->commit gap is closed by the create-if-absent
        // hard link commit above; after commit, a racing writer merely
        // supersedes the applied content through its own write — no apply
        // destruction remains on hardlink-capable filesystems.)
        let captured = match root_dir.read_to_string(&backup) {
            Ok(text) => text,
            Err(error) => {
                let mut rollback = rollback_committed(&root_dir, staged, index);
                match root_dir.rename(&backup, &root_dir, &staged[index].relative) {
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
            match root_dir.rename(&backup, &root_dir, &staged[index].relative) {
                Ok(()) => staged[index].backup = None,
                // Restore failed: the concurrent content now survives only in
                // the backup sidecar. Keep it on disk and name it in the
                // error rather than destroying it for tidiness.
                Err(error) => rollback = Some(error),
            }
            if let Some(error) = rollback_committed(&root_dir, staged, index) {
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

/// F2 (pass 71b): the swap-time mode re-check's fstatat. No portable on-disk
/// state makes this stat fail AFTER the content re-read immediately before it
/// succeeded — anything that defeats stat defeats open first — so the
/// stat-error arm cannot be driven by a real fixture the way the read-only
/// refusal is. cfg(test) builds get a one-shot injection seam; release
/// builds are a bare fstatat.
fn swap_time_metadata(
    root_dir: &Dir,
    relative: &Path,
) -> std::io::Result<cap_std::fs::Metadata> {
    #[cfg(test)]
    {
        if let Some(error) = take_injected_swap_stat_failure(relative) {
            return Err(error);
        }
    }
    let _ = relative;
    root_dir.metadata(relative)
}

#[cfg(test)]
thread_local! {
    /// Armed one-shot stat failures keyed by the source file's final
    /// component. thread_local because libtest runs each test on its own
    /// thread; armed entries never leak across tests.
    static INJECTED_SWAP_STAT_FAILURES:
        std::cell::RefCell<Vec<(String, std::io::Error)>> =
        std::cell::RefCell::new(Vec::new());
}

#[cfg(test)]
fn take_injected_swap_stat_failure(relative: &Path) -> Option<std::io::Error> {
    let name = relative.file_name()?.to_string_lossy().into_owned();
    INJECTED_SWAP_STAT_FAILURES.with(|armed| {
        let mut queue = armed.borrow_mut();
        let position = queue.iter().position(|(target, _)| *target == name)?;
        Some(queue.remove(position).1)
    })
}

#[cfg(test)]
fn arm_injected_swap_stat_failure(file_name: &str, error: std::io::Error) {
    INJECTED_SWAP_STAT_FAILURES.with(|armed| {
        armed
            .borrow_mut()
            .push((file_name.to_string(), error));
    });
}

fn confined_relative_path(path: &str) -> anyhow::Result<&Path> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("index contains a non-relative path: {}", path.display());
    }
    Ok(path)
}

/// F66a-7 (pass 66a): reduce matches to a disjoint, OUTERMOST-wins set.
/// Structural matches come from real tree nodes, so spans are identical,
/// properly nested, or disjoint. Sorted by (start asc, end DESC) the outer
/// span of any nested pair is visited first, and a candidate that starts
/// inside the last kept span is contained in (or partially overlaps) it —
/// dropped, because sg keeps the outer rewrite and skips the inner row.
/// Kept matches stay sorted by start, the order `apply_edits` requires.
fn keep_outermost_matches(mut matches: Vec<PatternMatch>) -> Vec<PatternMatch> {
    matches.sort_by(|left, right| {
        left.byte_start
            .cmp(&right.byte_start)
            .then_with(|| right.byte_end.cmp(&left.byte_end))
    });
    let mut kept: Vec<PatternMatch> = Vec::with_capacity(matches.len());
    for matched in matches {
        if kept
            .last()
            .is_some_and(|outer| matched.byte_start < outer.byte_end)
        {
            continue;
        }
        kept.push(matched);
    }
    kept
}

/// F66a-9 (pass 66a): a planned target the owner cannot write must be
/// refused, not rewritten. sg's update-all refuses the same state per file
/// (`Cannot rewrite file …` / `Permission denied`, os error 13) and leaves
/// it byte+mode intact. On unix the owner-write bit is the permission that
/// matters (the process runs as the owner; the staged-rename commit would
/// otherwise bypass a clear write bit); other platforms fall back to the
/// read-only predicate.
///
/// The codemod path touches two permission representations — `std` for the
/// plan-time capped reads (`CappedText::metadata`, captured at file open)
/// and cap-std for the apply-time staging and swap-time re-verification
/// (fresh fstatat each call) — so the predicate is expressed once over both.
/// Neither representation caches the mode; plan/apply never share a metadata
/// value (PASS 69b, SOAK-68B-CW-1 mechanism note).
trait TargetWritability {
    fn owner_cannot_write(&self) -> bool;
}

impl TargetWritability for fs::Permissions {
    fn owner_cannot_write(&self) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            self.mode() & 0o200 == 0
        }
        #[cfg(not(unix))]
        {
            self.readonly()
        }
    }
}

impl TargetWritability for cap_std::fs::Permissions {
    fn owner_cannot_write(&self) -> bool {
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            PermissionsExt::mode(self) & 0o200 == 0
        }
        #[cfg(not(unix))]
        {
            self.readonly()
        }
    }
}

fn target_refuses_writes<P: TargetWritability>(permissions: &P) -> bool {
    permissions.owner_cannot_write()
}

fn interpolate_rewrite(template: &str, matched: &PatternMatch) -> anyhow::Result<String> {
    let bytes = template.as_bytes();
    let mut output = String::with_capacity(template.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            let next = template[index..]
                .find('$')
                .map_or(bytes.len(), |offset| index + offset);
            output.push_str(&template[index..next]);
            index = next;
            continue;
        }
        if template[index..].starts_with("$$") && !template[index..].starts_with("$$$") {
            // F76-3 (pass 77E): `$$NAME` in a rewrite template is a capture
            // reference whose bound text itself begins with a literal `$`
            // (sg's rewrite output substitutes the capture whenever a name
            // follows the `$$`). Keys are the stripped names (single
            // namespace, F74a-3), so `$$A` reads the same key the single-`$A`
            // template reference reads — emitting the name as literal text
            // corrupted sources on apply.
            // F78-3 (pass 79): when NO name follows, sg keeps the `$$`
            // verbatim — there is no `$$`→`$` escape reduction (probed
            // spellings: end of template, space, punctuation).
            let mut name_end = index + 2;
            while name_end < bytes.len()
                && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
            {
                name_end += 1;
            }
            if name_end > index + 2 {
                let name = &template[index + 2..name_end];
                let value = matched
                    .captures
                    .get(name)
                    .with_context(|| format!("rewrite references unbound metavariable $${name}"))?;
                output.push_str(value);
                index = name_end;
                continue;
            }
            output.push_str("$$");
            index += 2;
            continue;
        }
        let prefix_len = if template[index..].starts_with("$$$") {
            3
        } else {
            1
        };
        let name_start = index + prefix_len;
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
        {
            name_end += 1;
        }
        if name_end == name_start {
            bail!("rewrite contains an invalid metavariable at byte {index}");
        }
        let name = &template[name_start..name_end];
        // PASS 60 (H-CONF-026): `$$$NAME` template references read the multi
        // namespace key; there is deliberately no flat fallback so a template
        // that asks for an unbound multi capture fails loudly instead of
        // silently reusing a same-named single capture (sg keeps the two
        // namespaces distinct in its own rewrite output).
        let key = if prefix_len == 3 {
            format!("$$${name}")
        } else {
            name.to_string()
        };
        let value = matched
            .captures
            .get(&key)
            .with_context(|| format!("rewrite references unbound metavariable ${name}"))?;
        output.push_str(value);
        index = name_end;
    }
    Ok(output)
}

fn apply_edits(original: &str, edits: &[CodemodEdit]) -> String {
    let replaced_bytes: usize = edits
        .iter()
        .map(|edit| edit.byte_end - edit.byte_start)
        .sum();
    let replacement_bytes: usize = edits.iter().map(|edit| edit.after.len()).sum();
    let mut rewritten = String::with_capacity(
        original
            .len()
            .saturating_sub(replaced_bytes)
            .saturating_add(replacement_bytes),
    );
    let mut cursor = 0;
    for edit in edits {
        rewritten.push_str(&original[cursor..edit.byte_start]);
        rewritten.push_str(&edit.after);
        cursor = edit.byte_end;
    }
    rewritten.push_str(&original[cursor..]);
    rewritten
}

struct StagedFile {
    relative: PathBuf,
    staged: PathBuf,
    backup: Option<PathBuf>,
}

fn write_staged_file(
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
        // br-bci: rename(backup -> path) replaces any existing file atomically
        // on POSIX. The previous remove_file-then-rename sequence had a crash
        // window that left the path missing AND the edited content destroyed.
        if let Err(error) = root_dir.rename(backup, root_dir, &file.relative) {
            first_error.get_or_insert(error);
        }
    }
    first_error
}

/// br-1xx: heal a tree left inconsistent by an apply process that died
/// mid-swap. For every planned path, restore the newest orphaned backup when
/// the canonical file is gone, then delete stale stage/backup leftovers so
/// re-runs recover instead of failing verification with ENOENT.
fn recover_orphans(root: &Path, planned_paths: &[String]) -> anyhow::Result<()> {
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
        // PASS 56 (H-AUDIT-52-3): "newest" is a TIMESTAMP question. The
        // sidecar name is `.{file}.asgrep-codemod-backup-{pid}-{clock}-{nonce}`
        // and pid precedes clock, so lexicographic order is temporal garbage
        // across processes (a large pid buries a recent clock). Sort by the
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

/// F2 (pass 71b): `transaction_error`'s rollback rider applied to an
/// already-contextualized error. Every mid-loop early exit in
/// `swap_staged_files` must pair `rollback_committed` + `cleanup_staged`
/// with this rider — the function's doc contract ("any error rolls back
/// every swap committed so far and removes the staged siblings") holds only
/// if no `?` can leak past them.
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
    match rollback {
        Some(rollback) => anyhow::anyhow!(
            "failed to apply {}: {commit}; rollback also failed: {rollback}",
            path.display()
        ),
        None => anyhow::anyhow!(
            "failed to apply {}: {commit}; all changes rolled back",
            path.display()
        ),
    }
}

#[cfg(test)]
mod commit_race_tests {
    //! PASS 65 (LOW c): the capture->commit gap the AlreadyExists arm guards
    //! is two adjacent syscalls wide (rename moves the source away,
    //! hard_link recreates it), so no other thread can observe the vacancy
    //! and write into it — observation latency exceeds the window. The
    //! branch contract is therefore pinned against a real on-disk staged
    //! state constructed exactly as the swap loop leaves it when a
    //! concurrent writer wins the gap, driven through the same entry point
    //! the arm calls.
    use super::*;
    use std::fs;

    const SOURCE: &str = "fn run() { legacy(alpha); }\n";
    const REWRITTEN: &str = "fn run() { modern(alpha); }\n";
    const CONCURRENT: &str = "fn concurrent_edit() {}\n";

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
        let kept = fs::read_to_string(src.join(capture_name)).unwrap_or_else(|error| {
            panic!("capture sidecar must survive the refusal: {error}")
        });
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

    const TWO_LINE_REWRITTEN: &str =
        "fn run() { modern(alpha); }\nfn keep() { modern(beta); }\n";

    /// PASS 69b (SOAK-68B-CW-1 / F-68c-3): the swap loop re-verified CONTENT
    /// immediately before each swap but NOT the target's MODE. A target
    /// chmod'd 0444 after the staging guard read a writable mode therefore
    /// went through the staged rename anyway — exit 0, content swapped, the
    /// exact silent-rewrite face F66a-9/67b closed, realized intermittently
    /// by soak rep1 on an unchanged binary (guard read 0444 as writable).
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
        // BEFORE the swap loop reaches b.rs — the window SOAK-68B-CW-1
        // realized intermittently, constructed here as a state, not a race.
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

    /// F2 (pass 71b): hand-construct the exact post-staging state for a
    /// two-file plan, mirroring
    /// `swap_loop_refuses_target_turned_read_only_after_staging`: sources
    /// byte-identical to the plan, staged siblings written with the sources'
    /// writable perms, no backups yet (the swap loop creates those).
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
            root_dir
                .set_permissions(&staged_path, permissions)
                .unwrap();
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

    /// THE F2 RED (pass 71b, finding 70c-F2): the swap-time mode re-check
    /// propagated a stat FAILURE with `?` — early-returning from
    /// `swap_staged_files` after file 1's swap already committed, WITHOUT
    /// `rollback_committed`/`cleanup_staged`. Contract (L440-442): ANY error
    /// rolls back every swap committed so far and removes the staged
    /// siblings. Deterministic seam: the injected fstatat failure fires for
    /// b.rs AFTER its content re-read succeeded — the exact pass-69b window
    /// (delete race / EACCES / EIO between the re-read at the loop head and
    /// the stat), which no portable on-disk state can produce. Required
    /// behavior: loud error naming the file, a.rs restored bytes+mode+INODE
    /// (the rollback renames the captured original back — the same hygiene
    /// path the read-only refusal uses), zero sidecar leftovers.
    #[test]
    #[cfg(unix)]
    fn swap_loop_stat_failure_after_a_committed_swap_rolls_back_and_sweeps() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let (temp, plan, mut staged) = two_file_swappable_state();
        let src = temp.path().join("fixture/src");
        let root_dir =
            Dir::open_ambient_dir(temp.path().join("fixture"), ambient_authority()).unwrap();

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

    /// F2 audit rider (pass 71b): the PRE-EXISTING sibling hole the 70c
    /// report names on the content re-read (`read_to_string(...)?`) — the
    /// same leak class on an older arm. A source removed between staging and
    /// its swap (concurrent `rm`, the delete-race face the re-read exists to
    /// catch) must refuse with the same rollback+cleanup+rider hygiene as
    /// the mismatch refusal two lines below it. Driven by a real on-disk
    /// state, no injection needed.
    #[test]
    #[cfg(unix)]
    fn swap_loop_source_vanished_after_a_committed_swap_rolls_back_and_sweeps() {
        use std::os::unix::fs::MetadataExt;
        let (temp, plan, mut staged) = two_file_swappable_state();
        let src = temp.path().join("fixture/src");
        let root_dir =
            Dir::open_ambient_dir(temp.path().join("fixture"), ambient_authority()).unwrap();

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
}
