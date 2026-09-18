//! Deterministic edit-plan construction from indexed files. Read-only against
//! the store; refuses loudly where the plan would contradict search.

use super::apply::recover_orphans;
use super::guard::{confined_relative_path, target_refuses_writes};
use super::rewrite::{apply_edits, interpolate_rewrite, keep_outermost_matches};
use super::{CodemodEdit, CodemodFilePlan, CodemodPlan};
use crate::io_bounds::{RootDir, MAX_INDEX_FILE_BYTES};
use crate::IndexStore;
use anyhow::{bail, Context};
use ast_sgrep_lang::{
    classify_native, detect_language, match_pattern, required_pattern_literal, Language,
};
use std::path::Path;

/// Byte-order mark stripped from pattern ingress (U+FEFF), as a scalar.
const BOM_SCALAR: u32 = 0xFEFF;

/// Build a deterministic edit plan from files already present in the index.
/// Unsupported structural patterns are rejected rather than delegated to an
/// external `ast-grep` executable.
///
/// `lang_filter` honors `--lang` exactly like the search path: stored id,
/// file extension, or alias; `None` plans everywhere.
pub fn plan_codemod(
    root: &Path,
    index_path: Option<&Path>,
    lang_filter: Option<&str>,
    pattern: &str,
    rewrite: &str,
) -> anyhow::Result<CodemodPlan> {
    // The search lane's query grammar strips ONE optional leading
    // `pattern:` token (ParsedQuery::parse_mode, and the multi-pattern
    // ingress tolerates the same spelling per value); the
    // codemod lane never did, so `--pattern='pattern:echo "a b";'` handed
    // the raw prefixed text to the matcher as literal pattern code,
    // match-None'd, and silently planned zero edits on exactly the faces
    // search serves — the prefix even carried `$`-bearing spellings past the
    // classifier gate below and past the emptiness refusal (`pattern:`).
    // Normalize the ingress so both lanes see one pattern spelling; the
    // emptiness check and the `$`-gate keep judging the stripped spelling.
    // The strip ORDER must match the search lane's — search strips the
    // prefix FIRST (parse_mode) and the BOM later (search_pattern's BOM
    // strip). With the BOM strip running first,
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
    // The BOM strip runs BEFORE the empty check — the raw U+FEFF is not
    // whitespace, so a BOM-only pattern previously slipped the guard,
    // survived as an empty string, and planned a nonsense zero-edit
    // codemod instead of refusing loudly.
    let pattern = pattern
        .trim_start_matches(|c: char| c as u32 == BOM_SCALAR)
        .trim();
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
    // Planning only reads the store (root binding, indexed paths,
    // `pattern_nodes_matching_limited`), so open READ-ONLY — a writable
    // open would silently fire the row-discarding schema-15 migration as a
    // side effect of a planning read, demoting the index (pattern_nodes
    // wiped, hashes prefixed) with no re-extract and no marker. Read-only
    // gains the loud pre-current gate for free, symmetric with search: the
    // refusal names `asgrep reindex`, the honest remedy.
    let store = IndexStore::open_readonly(&root, index_path)?;
    // Read-side root binding, same contract as `Searcher::new` — a db
    // stamped for another root must never answer (let alone rewrite files)
    // for this tree. Cross-root REINDEX stays the designed prune-replace;
    // only the read is refused.
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
    // Heal a tree left inconsistent by an apply process that died mid-swap
    // (canonical path missing, orphaned `.name.asgrep-codemod-backup-*`
    // beside it) BEFORE reading the planned files, so a re-run recovers the
    // previous content instead of failing verification with ENOENT. Runs on
    // std::fs because planning has no Dir handle yet; `root` is canonical
    // and sidecar names are matched by exact marker, so confinement holds.
    recover_orphans(&root, &indexed_paths)?;
    if indexed_paths.is_empty() {
        bail!(
            "index is empty for {}; run: asgrep index {} --json",
            root.display(),
            root.display()
        );
    }

    // The prefilter literal must not be computed on raw bytes when the
    // pattern carries an expando char — `µNAME + 1` yielded the needle
    // "NAME", a byte the meta reading (`$NAME + 1` — what `match_pattern`
    // answers after normalizing) does NOT require, so µ-free files were
    // silently dropped from the plan while the search lane answered them
    // (the reference rewrites sites across files the plan dropped). Guard
    // with the same containment check the two search lanes use (index lane
    // + byte prefilter): over-broad matches only disable the fast
    // prefilter, never results.
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
        // The language filter is applied here, not only in search, so a
        // codemod plan can never cross the --lang boundary.
        if lang_filter
            .as_deref()
            .is_some_and(|want| language.as_str() != want)
        {
            continue;
        }
        let matches = match_pattern(language, &original, pattern)
            .with_context(|| format!("failed to match pattern in {rel_path}"))?;
        // Reference-compatible overlap resolution — structural matches
        // come from real tree nodes, so spans are identical, properly
        // nested, or disjoint; the OUTERMOST match wins and inner overlaps
        // are skipped per edit. The reference `run --rewrite` on the same
        // state rewrites the outer span
        // (`conn?.open()?.send(1)` → `log(conn?.open(), send)`), leaves the
        // inner row unrewritten, still applies every non-overlapping edit,
        // and exits 0. The historical whole-plan refusal
        // (`codemod matches overlap …`, exit 2) killed the unrelated edits
        // in the file where the reference answered.
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
        // Refuse a read-only target per file, reference-agreed. The apply
        // commit stages a temp sibling and swaps by rename/hard link, which
        // needs only the writable PARENT directory — the historical path
        // therefore rewrote a `chmod 444` file's content (keeping its
        // read-only mode, exit 0) where the reference's update-all refuses
        // the file (`Cannot rewrite file … Permission denied`, os error 13)
        // and leaves it byte+mode intact. Excluding the file here — and
        // naming it — keeps the dry-run preview equal to what apply will
        // do and honors the file's writability.
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
    // Honesty class: search serves exact ident and decl patterns from
    // the index (`cached_pattern_signatures` + `index_can_serve_pattern`),
    // but the native matcher that produces codemod edit spans answers
    // match-none for concrete decl templates (`fn old_name` is not a
    // literal node the literal lane can span). A silent ok:true zero-edit
    // plan while search answers hits is a fail-open; refuse loudly naming
    // the limitation instead. Patterns the index cannot serve (and
    // therefore search also answers empty) keep the quiet ok:true zero
    // plan.
    // The gate reads the CURRENT file bytes before refusing — a stale
    // index can still serve an identifier that a concurrent edit already
    // removed from the tree, and the refusal then fired on a hit that no
    // longer exists (false refuse). The same `required_pattern_literal`
    // prefilter the planning loop uses decides: when NO indexed file still
    // contains the pattern's required literal, the tree is authoritative
    // and the zero-edit plan is honest.
    // A zero-edit plan explained by named read-only refusals is NOT a
    // silent fail-open — the envelope names every refused file — so the
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
                // Depth-truncated files in scope make the index
                // non-authoritative — search answers via the native walk
                // while this planner saw no edit spans, so a quiet zero-edit
                // plan would contradict search. Refuse loudly. A re-index
                // does NOT clear the flag (the budget re-truncates the same
                // files), hence the verify-then-raise advice.
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
