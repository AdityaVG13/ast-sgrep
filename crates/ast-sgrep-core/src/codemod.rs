//! Indexed, in-process structural codemod planning and transactional apply.

use crate::io_bounds::{RootDir, MAX_INDEX_FILE_BYTES};
use crate::IndexStore;
use anyhow::{bail, Context};
use ast_sgrep_lang::{
    classify_native, detect_language, match_pattern, required_pattern_literal, PatternMatch,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
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
pub fn plan_codemod(
    root: &Path,
    index_path: Option<&Path>,
    pattern: &str,
    rewrite: &str,
) -> anyhow::Result<CodemodPlan> {
    if pattern.trim().is_empty() {
        bail!("codemod pattern must not be empty");
    }
    if pattern.contains('$') && classify_native(pattern).is_none() {
        bail!("pattern is not supported by the in-process structural matcher");
    }

    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root: {}", root.display()))?;
    let root_dir = RootDir::open(&root)?;
    let store = IndexStore::open(&root, index_path)?;
    let indexed_paths = store.all_file_paths()?;
    if indexed_paths.is_empty() {
        bail!(
            "index is empty for {}; run: asgrep index {} --json",
            root.display(),
            root.display()
        );
    }

    let required_literal = required_pattern_literal(pattern);
    let mut files = Vec::new();
    for rel_path in indexed_paths {
        let rel = confined_relative_path(&rel_path)?;
        let original = root_dir
            .read_text_capped(rel, MAX_INDEX_FILE_BYTES)
            .with_context(|| format!("failed to read indexed file {rel_path}"))?
            .text;
        if required_literal.as_ref().is_some_and(|literal| {
            memchr::memmem::find(original.as_bytes(), literal.as_bytes()).is_none()
        }) {
            continue;
        }
        let Some(language) = detect_language(rel, Some(&original)) else {
            continue;
        };
        let mut matches = match_pattern(language, &original, pattern)
            .with_context(|| format!("failed to match pattern in {rel_path}"))?;
        matches.sort_by_key(|matched| (matched.byte_start, matched.byte_end));
        validate_non_overlapping(&rel_path, &matches)?;

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
        let rewritten = apply_edits(&original, &edits);
        files.push(CodemodFilePlan {
            path: rel_path,
            edits,
            original,
            rewritten,
        });
    }
    let edit_count = files.iter().map(|file| file.edits.len()).sum();
    Ok(CodemodPlan {
        pattern: pattern.to_string(),
        rewrite: rewrite.to_string(),
        files_changed: files.len(),
        edit_count,
        files,
        root,
    })
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

    let mut staged = Vec::with_capacity(plan.files.len());
    for (index, file) in plan.files.iter().enumerate() {
        let prepared = (|| -> anyhow::Result<StagedFile> {
            let absolute = plan.root.join(confined_relative_path(&file.path)?);
            let current = fs::read_to_string(&absolute)
                .with_context(|| format!("failed to verify {} before apply", file.path))?;
            if current != file.original {
                bail!("source changed after codemod planning: {}", file.path);
            }
            let permissions = fs::metadata(&absolute)?.permissions();
            let staged_path = write_staged_file(&absolute, &file.rewritten, index)?;
            if let Err(error) = fs::set_permissions(&staged_path, permissions) {
                let _ = fs::remove_file(&staged_path);
                return Err(error).with_context(|| format!("failed to stage {}", file.path));
            }
            Ok(StagedFile {
                absolute,
                staged: staged_path,
                backup: None,
            })
        })();
        match prepared {
            Ok(prepared) => staged.push(prepared),
            Err(error) => {
                cleanup_staged(&staged);
                return Err(error);
            }
        }
    }

    for index in 0..staged.len() {
        let backup = unique_sibling_path(&staged[index].absolute, "backup", index)?;
        if let Err(error) = fs::rename(&staged[index].absolute, &backup) {
            let rollback = rollback_committed(&mut staged, index);
            cleanup_staged(&staged);
            return Err(transaction_error(error, rollback, &staged[index].absolute));
        }
        staged[index].backup = Some(backup.clone());
        if let Err(error) = fs::rename(&staged[index].staged, &staged[index].absolute) {
            let restore_current = fs::rename(&backup, &staged[index].absolute).err();
            staged[index].backup = None;
            let rollback = rollback_committed(&mut staged, index).or(restore_current);
            cleanup_staged(&staged);
            return Err(transaction_error(error, rollback, &staged[index].absolute));
        }
    }

    for file in &staged {
        if let Some(backup) = &file.backup {
            let _ = fs::remove_file(backup);
        }
    }
    Ok(CodemodApplyResult {
        files_changed: plan.files_changed,
        edits_applied: plan.edit_count,
    })
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

fn validate_non_overlapping(path: &str, matches: &[PatternMatch]) -> anyhow::Result<()> {
    for pair in matches.windows(2) {
        if pair[1].byte_start < pair[0].byte_end {
            bail!(
                "codemod matches overlap in {path} at byte ranges {}..{} and {}..{}",
                pair[0].byte_start,
                pair[0].byte_end,
                pair[1].byte_start,
                pair[1].byte_end
            );
        }
    }
    Ok(())
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
            output.push('$');
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
        let value = matched
            .captures
            .get(name)
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
    absolute: PathBuf,
    staged: PathBuf,
    backup: Option<PathBuf>,
}

fn write_staged_file(path: &Path, contents: &str, index: usize) -> anyhow::Result<PathBuf> {
    for attempt in 0..100 {
        let staged = unique_sibling_path(path, "stage", index * 100 + attempt)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
        {
            Ok(mut file) => {
                let write_result = file
                    .write_all(contents.as_bytes())
                    .and_then(|()| file.sync_all());
                match write_result {
                    Ok(()) => return Ok(staged),
                    Err(error) => {
                        drop(file);
                        let _ = fs::remove_file(staged);
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

fn rollback_committed(staged: &mut [StagedFile], count: usize) -> Option<std::io::Error> {
    let mut first_error = None;
    for file in staged[..count].iter_mut().rev() {
        let Some(backup) = file.backup.take() else {
            continue;
        };
        if let Err(error) = fs::remove_file(&file.absolute) {
            first_error.get_or_insert(error);
            continue;
        }
        if let Err(error) = fs::rename(backup, &file.absolute) {
            first_error.get_or_insert(error);
        }
    }
    first_error
}

fn cleanup_staged(staged: &[StagedFile]) {
    let mut paths = BTreeSet::new();
    for file in staged {
        paths.insert(file.staged.clone());
    }
    for path in paths {
        let _ = fs::remove_file(path);
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
