//! Indexed read windows and unique-string edits for Code Mode.
//!
//! Amdahl: these stay in-process on the warm session. `find` is lexical
//! (`word:`) so unique queries stay on the trigram path. `read` pulls line
//! windows from SQLite when the file is indexed, else a bounded disk scan.
//! `edit` is a unique-string replace + targeted reindex — never a second
//! Searcher open.

use crate::session::CodeModeSession;
use anyhow::{anyhow, Context};
use ast_sgrep_core::{Indexer, IndexOptions, MAX_EXCERPT_LINES, MAX_INDEX_FILE_BYTES};
use serde_json::{json, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) const MAX_READ_REFS: usize = 32;
pub(crate) const MAX_READ_CHARS: usize = 100_000;
pub(crate) const MAX_EDITS: usize = 16;
const MAX_LINE_CHARS: usize = 2_000;

impl CodeModeSession {
    /// Lexical / identifier lookup. Unprefixed queries become `word:` so they
    /// skip hybrid fusion. Prefixed queries (`defs:`, `literal:`, …) pass through.
    pub(crate) fn find(&mut self, args: &Value) -> anyhow::Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("query is required")?;
        ast_sgrep_core::validate_query_len(query).map_err(|e| anyhow::anyhow!(e))?;
        let dispatched = dispatch_find_query(query);
        let mut forwarded = args.clone();
        if let Some(obj) = forwarded.as_object_mut() {
            obj.insert("query".into(), json!(dispatched));
            obj.insert("semantic_only".into(), json!(false));
        }
        self.search(&forwarded)
    }

    /// Batched line windows. One Searcher, many refs — SQLite seeks, not N opens.
    pub(crate) fn read_windows(&mut self, args: &Value) -> anyhow::Result<Value> {
        let root = self.jail_root(args)?;
        let context_lines = args
            .get("context_lines")
            .or_else(|| args.get("contextLines"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(0)
            .min(MAX_EXCERPT_LINES);
        let max_chars = args
            .get("max_chars")
            .or_else(|| args.get("maxChars"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(MAX_READ_CHARS)
            .clamp(1, MAX_READ_CHARS);
        let refs = collect_refs(args)?;
        if refs.is_empty() {
            return Err(anyhow!("read requires path, ref, or refs"));
        }
        if refs.len() > MAX_READ_REFS {
            return Err(anyhow!("read exceeds max {MAX_READ_REFS} windows"));
        }
        let windows = self.with_searcher(root.clone(), self.config().limit, |searcher| {
            let mut windows = Vec::with_capacity(refs.len());
            for spec in &refs {
                windows.push(read_one_window(
                    searcher.store(),
                    &root,
                    spec,
                    context_lines,
                    max_chars,
                )?);
            }
            Ok(windows)
        })?;
        Ok(json!({
            "ok": true,
            "count": windows.len(),
            "windows": windows,
        }))
    }

    /// Unique string replace, then targeted index update.
    pub(crate) fn edit_files(&mut self, args: &Value) -> anyhow::Result<Value> {
        let root = self.jail_root(args)?;
        let edits = collect_edits(args)?;
        if edits.is_empty() {
            return Err(anyhow!("edit requires path+oldText+newText or edits[]"));
        }
        if edits.len() > MAX_EDITS {
            return Err(anyhow!("edit exceeds max {MAX_EDITS} replacements"));
        }
        let mut applied = Vec::with_capacity(edits.len());
        let mut rel_paths = Vec::with_capacity(edits.len());
        for edit in &edits {
            let rel = jail_rel_path(&root, &edit.path)?;
            let abs = root.join(&rel);
            let original = fs::read_to_string(&abs)
                .with_context(|| format!("cannot read {}", rel.display()))?;
            if original.len() > MAX_INDEX_FILE_BYTES as usize {
                return Err(anyhow!(
                    "{} exceeds max {MAX_INDEX_FILE_BYTES} bytes",
                    rel.display()
                ));
            }
            let rewritten = unique_replace(&original, &edit.old_text, &edit.new_text)?;
            if rewritten == original {
                applied.push(json!({
                    "path": rel_display(&rel),
                    "changed": false,
                }));
                continue;
            }
            fs::write(&abs, rewritten.as_bytes())
                .with_context(|| format!("cannot write {}", rel.display()))?;
            applied.push(json!({
                "path": rel_display(&rel),
                "changed": true,
            }));
            rel_paths.push(rel_display(&rel));
        }
        if !rel_paths.is_empty() {
            let mut indexer = Indexer::new(IndexOptions {
                root: root.clone(),
                index_path: self.config().index_path.clone(),
                embed_semantic: self.config().use_embed,
                ..IndexOptions::default()
            })?;
            let paths: Vec<PathBuf> = rel_paths.iter().map(PathBuf::from).collect();
            indexer.update_paths(&paths)?;
            indexer.flush_deferred_rebuilds()?;
            self.invalidate_searcher_cache();
        }
        Ok(json!({
            "ok": true,
            "changed": applied.iter().filter(|row| row["changed"] == true).count(),
            "edits": applied,
        }))
    }
}

pub(crate) fn dispatch_find_query(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(target) = trimmed.strip_prefix("blast:") {
        let target = target.trim();
        if target.contains('/') || target.contains('\\') || target.contains('.') {
            return format!("imports:{target}");
        }
        return format!("callers:{target}");
    }
    let parsed = ast_sgrep_core::ParsedQuery::parse(trimmed);
    if parsed.mode != ast_sgrep_core::QueryMode::Hybrid {
        trimmed.to_string()
    } else {
        format!("word:{trimmed}")
    }
}

struct ReadSpec {
    path: String,
    start: u32,
    end: u32,
}

struct EditSpec {
    path: String,
    old_text: String,
    new_text: String,
}

fn collect_refs(args: &Value) -> anyhow::Result<Vec<ReadSpec>> {
    if let Some(refs) = args.get("refs").and_then(|v| v.as_array()) {
        return refs.iter().map(parse_ref_value).collect();
    }
    if let Some(r) = args.get("ref") {
        return Ok(vec![parse_ref_value(r)?]);
    }
    let path = args
        .get("path")
        .or_else(|| args.get("file"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("path is required"))?;
    let start = args
        .get("start")
        .or_else(|| args.get("line_start"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(1)
        .max(1);
    let end = args
        .get("end")
        .or_else(|| args.get("line_end"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(start)
        .max(start);
    Ok(vec![ReadSpec {
        path: path.to_string(),
        start,
        end,
    }])
}

fn parse_ref_value(value: &Value) -> anyhow::Result<ReadSpec> {
    if let Some(s) = value.as_str() {
        return parse_ref_str(s);
    }
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow!("ref must be a string or object"))?;
    if let Some(r) = obj.get("ref").and_then(|v| v.as_str()) {
        return parse_ref_str(r);
    }
    let path = obj
        .get("path")
        .or_else(|| obj.get("file"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("ref.path is required"))?;
    let start = obj
        .get("start")
        .or_else(|| obj.get("line_start"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(1)
        .max(1);
    let end = obj
        .get("end")
        .or_else(|| obj.get("line_end"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(start)
        .max(start);
    Ok(ReadSpec {
        path: path.to_string(),
        start,
        end,
    })
}

fn parse_ref_str(raw: &str) -> anyhow::Result<ReadSpec> {
    if let Some((path, rest)) = raw.rsplit_once("#L") {
        let rest = rest.trim();
        let (start_s, end_s) = rest.split_once("-L").unwrap_or((rest, rest));
        let start: u32 = start_s
            .parse()
            .map_err(|_| anyhow!("invalid ref start in {raw}"))?;
        let end: u32 = end_s
            .parse()
            .map_err(|_| anyhow!("invalid ref end in {raw}"))?;
        if start == 0 || end < start {
            return Err(anyhow!("invalid ref range in {raw}"));
        }
        return Ok(ReadSpec {
            path: path.to_string(),
            start,
            end,
        });
    }
    Ok(ReadSpec {
        path: raw.to_string(),
        start: 1,
        end: 40,
    })
}

fn collect_edits(args: &Value) -> anyhow::Result<Vec<EditSpec>> {
    if let Some(edits) = args.get("edits").and_then(|v| v.as_array()) {
        return edits.iter().map(parse_edit_value).collect();
    }
    if args.get("path").and_then(|v| v.as_str()).is_some() {
        return Ok(vec![parse_edit_value(args)?]);
    }
    Ok(Vec::new())
}

fn parse_edit_value(value: &Value) -> anyhow::Result<EditSpec> {
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow!("edit must be an object"))?;
    let path = obj
        .get("path")
        .or_else(|| obj.get("file"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("path is required"))?;
    let old_text = obj
        .get("oldText")
        .or_else(|| obj.get("old_string"))
        .or_else(|| obj.get("old"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("oldText is required"))?;
    let new_text = obj
        .get("newText")
        .or_else(|| obj.get("new_string"))
        .or_else(|| obj.get("new"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("newText is required"))?;
    if old_text.is_empty() {
        return Err(anyhow!("oldText must not be empty"));
    }
    Ok(EditSpec {
        path: path.to_string(),
        old_text: old_text.to_string(),
        new_text: new_text.to_string(),
    })
}

fn unique_replace(haystack: &str, old: &str, new: &str) -> anyhow::Result<String> {
    let count = haystack.matches(old).count();
    if count != 1 {
        return Err(anyhow!("oldText must match exactly once (found {count})"));
    }
    Ok(haystack.replacen(old, new, 1))
}

fn jail_rel_path(root: &Path, raw: &str) -> anyhow::Result<PathBuf> {
    let requested = Path::new(raw);
    if requested
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(anyhow!("path must not contain '..'"));
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let canon = candidate
        .canonicalize()
        .with_context(|| format!("cannot resolve path {raw}"))?;
    if !canon.starts_with(root) {
        return Err(anyhow!("path escapes session root: {raw}"));
    }
    Ok(canon
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or(canon))
}

fn rel_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn read_one_window(
    store: &ast_sgrep_core::IndexStore,
    root: &Path,
    spec: &ReadSpec,
    context_lines: usize,
    max_chars: usize,
) -> anyhow::Result<Value> {
    let rel = jail_rel_path(root, &spec.path)?;
    let rel_s = rel_display(&rel);
    let ctx = context_lines as u32;
    let start = spec.start.saturating_sub(ctx).max(1);
    let end = spec.end.saturating_add(ctx);
    let indexed = store.file_lines(&rel_s)?;
    let (text, actual_start, actual_end, truncated) = if indexed.is_empty() {
        read_disk_window(root, &rel, start, end, max_chars)?
    } else {
        slice_indexed(&indexed, start, end, max_chars)
    };
    Ok(json!({
        "path": rel_s,
        "ref": format!("{rel_s}#L{actual_start}-L{actual_end}"),
        "start": actual_start,
        "end": actual_end,
        "truncated": truncated,
        "text": text,
    }))
}

fn slice_indexed(
    lines: &[(u32, String)],
    start: u32,
    end: u32,
    max_chars: usize,
) -> (String, u32, u32, bool) {
    let mut out = String::new();
    let mut actual_start = start;
    let mut actual_end = start;
    let mut first = true;
    let mut truncated = false;
    let mut chars = 0usize;
    for (no, content) in lines {
        if *no < start {
            continue;
        }
        if *no > end {
            break;
        }
        let mut line = content.as_str();
        if line.chars().count() > MAX_LINE_CHARS {
            let end_idx = line
                .char_indices()
                .nth(MAX_LINE_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            line = &content[..end_idx];
            truncated = true;
        }
        let add = if first { 0 } else { 1 } + line.chars().count();
        if chars.saturating_add(add) > max_chars {
            truncated = true;
            break;
        }
        if first {
            actual_start = *no;
            first = false;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        actual_end = *no;
        chars += add;
    }
    if first {
        (String::new(), start, start, false)
    } else {
        (out, actual_start, actual_end, truncated)
    }
}

fn read_disk_window(
    root: &Path,
    rel: &Path,
    start: u32,
    end: u32,
    max_chars: usize,
) -> anyhow::Result<(String, u32, u32, bool)> {
    let text = fs::read_to_string(root.join(rel))
        .with_context(|| format!("cannot read {}", rel.display()))?;
    if text.len() > MAX_INDEX_FILE_BYTES as usize {
        return Err(anyhow!(
            "{} exceeds max {MAX_INDEX_FILE_BYTES} bytes",
            rel.display()
        ));
    }
    let numbered: Vec<(u32, String)> = text
        .lines()
        .enumerate()
        .map(|(i, line)| (i as u32 + 1, line.to_string()))
        .collect();
    Ok(slice_indexed(&numbered, start, end, max_chars))
}

#[cfg(test)]
mod find_dispatch {
    use super::dispatch_find_query;

    #[test]
    fn blast_symbol_becomes_callers() {
        assert_eq!(
            dispatch_find_query("blast:process_request"),
            "callers:process_request"
        );
    }

    #[test]
    fn blast_path_becomes_imports() {
        assert_eq!(dispatch_find_query("blast:src/auth.ts"), "imports:src/auth.ts");
    }

    #[test]
    fn unprefixed_is_word() {
        assert_eq!(dispatch_find_query("hello"), "word:hello");
    }
}
