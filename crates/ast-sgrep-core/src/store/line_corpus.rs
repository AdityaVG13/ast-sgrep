//! Generation-keyed in-memory line corpus for unique-query literal search.
//!
//! FFF's race is a warm process with files already in RAM. SQLite FTS/LIKE on
//! every unique `literal:` query paid a few milliseconds of parser/join tax on
//! this tree. This corpus loads indexed lines once and scans with `memchr`.

use crate::Result;
use ast_sgrep_embed::split_ident;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Skip the RAM corpus when packed source exceeds this. Callers fall back to
/// SQLite. 256 MiB covers typical agent worktrees; 100k-file monorepos that
/// overflow keep the FTS path instead of ballooning RSS.
pub const MAX_CORPUS_BYTES: usize = 256 * 1024 * 1024;

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn add_token_file(map: &mut HashMap<String, Vec<u32>>, token: String, fi: u32) {
    let entry = map.entry(token).or_default();
    if entry.last().copied() != Some(fi) {
        entry.push(fi);
    }
}

fn push_ascii_tokens(map: &mut HashMap<String, Vec<u32>>, text: &str, fi: u32) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !is_ident_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        if i - start < 3 {
            continue;
        }
        let raw = &text[start..i];
        let token = raw.to_ascii_lowercase();
        for part in split_ident(raw) {
            if part.len() >= 3 {
                add_token_file(map, part, fi);
            }
        }
        add_token_file(map, token, fi);
    }
}

fn file_first_lines(file_idx: &[u32], n_files: usize) -> Vec<u32> {
    let mut first = vec![u32::MAX; n_files];
    for (i, &fi) in file_idx.iter().enumerate() {
        if let Some(slot) = first.get_mut(fi as usize) {
            if *slot == u32::MAX {
                *slot = i as u32;
            }
        }
    }
    first
}

fn cascade_prefers_file(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    if file.ends_with(".md")
        || file.ends_with(".mdx")
        || file.ends_with(".rst")
        || file.ends_with(".txt")
        || file.starts_with("changelog")
        || file.starts_with("readme")
        || normalized.contains("/docs/")
    {
        return false;
    }
    if normalized
        .split('/')
        .any(|seg| seg == "tests" || seg == "test")
    {
        return false;
    }
    true
}

fn file_byte_ends(file_idx: &[u32], starts: &[u32], n_files: usize, bytes_len: usize) -> Vec<u32> {
    let mut ends = vec![bytes_len as u32; n_files];
    for (i, &fi) in file_idx.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(bytes_len as u32);
        if let Some(slot) = ends.get_mut(fi as usize) {
            *slot = end;
        }
    }
    ends
}

#[derive(Debug, Clone)]
struct CorpusFile {
    path: String,
    language: Option<String>,
}

/// Packed `lines` table: path-sorted, one record per indexed line.
#[derive(Debug)]
pub struct LineCorpus {
    pub index_data_version: i64,
    pub pragma_data_version: i64,
    bytes: Vec<u8>,
    starts: Vec<u32>,
    line_nos: Vec<u32>,
    file_idx: Vec<u32>,
    files: Vec<CorpusFile>,
    /// Exclusive packed-byte end of each file. Cascade discovery jumps here
    /// after the first hit so a common needle does not memchr the rest of
    /// that file.
    file_byte_ends: Vec<u32>,
    /// First packed line of each file (cascade token lookup).
    file_first_line: Vec<u32>,
    /// Lowercased ASCII identifier -> file indexes. Unique-hybrid cascade
    /// looks up rare needles here instead of memchr-ing the whole blob.
    token_files: HashMap<String, Vec<u32>>,
}

pub struct LineHit<'a> {
    pub path: &'a str,
    pub language: Option<&'a str>,
    pub line_no: u32,
    pub content: &'a str,
}

impl LineCorpus {
    pub fn load(
        conn: &Connection,
        index_data_version: i64,
        pragma_data_version: i64,
    ) -> Result<Option<Arc<Self>>> {
        let mut stmt = conn.prepare_cached(
            "SELECT f.path, f.language, l.line_no, l.content \
             FROM lines l JOIN files f ON f.id = l.file_id \
             ORDER BY f.path, l.line_no",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut bytes = Vec::new();
        let mut starts = Vec::new();
        let mut line_nos = Vec::new();
        let mut file_idx = Vec::new();
        let mut files = Vec::new();
        let mut current_path: Option<String> = None;
        let mut token_files: HashMap<String, Vec<u32>> = HashMap::new();

        for row in rows {
            let (path, language, line_no, content) = row?;
            if bytes.len().saturating_add(content.len()).saturating_add(1) > MAX_CORPUS_BYTES {
                return Ok(None);
            }
            if current_path.as_deref() != Some(path.as_str()) {
                current_path = Some(path.clone());
                files.push(CorpusFile { path, language });
            }
            let idx = (files.len() - 1) as u32;
            push_ascii_tokens(&mut token_files, &content, idx);
            starts.push(bytes.len() as u32);
            bytes.extend_from_slice(content.as_bytes());
            bytes.push(b'\n');
            line_nos.push(line_no);
            file_idx.push(idx);
        }

        let n_files = files.len();
        let file_byte_ends = file_byte_ends(&file_idx, &starts, n_files, bytes.len());
        let file_first_line = file_first_lines(&file_idx, n_files);
        Ok(Some(Arc::new(Self {
            index_data_version,
            pragma_data_version,
            bytes,
            starts,
            line_nos,
            file_idx,
            files,
            file_byte_ends,
            file_first_line,
            token_files,
        })))
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    /// Distinct indexed files packed in this corpus (cascade file-cap).
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    fn line_content(&self, i: usize) -> &str {
        let start = self.starts[i] as usize;
        let end = self
            .starts
            .get(i + 1)
            .copied()
            .map(|s| s as usize)
            .unwrap_or(self.bytes.len())
            .saturating_sub(1);
        std::str::from_utf8(&self.bytes[start..end]).unwrap_or("")
    }

    fn hit(&self, i: usize) -> LineHit<'_> {
        let file = &self.files[self.file_idx[i] as usize];
        LineHit {
            path: &file.path,
            language: file.language.as_deref(),
            line_no: self.line_nos[i],
            content: self.line_content(i),
        }
    }

    fn line_index_at_byte(&self, abs: usize) -> Option<usize> {
        if self.starts.is_empty() {
            return None;
        }
        let i = match self
            .starts
            .binary_search(&(abs.min(u32::MAX as usize) as u32))
        {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let start = self.starts[i] as usize;
        let content_end = self
            .starts
            .get(i + 1)
            .copied()
            .map(|s| s as usize)
            .unwrap_or(self.bytes.len())
            .saturating_sub(1);
        if abs >= start && abs < content_end {
            Some(i)
        } else {
            None
        }
    }

    /// Path-sorted matches. `cap` is the keep-set (same role as SQL LIMIT).
    pub fn scan_cs<'a>(
        &'a self,
        needle: &str,
        word_mode: bool,
        lang_filter: Option<&str>,
        cap: usize,
        lang_ok: impl Fn(Option<&str>, Option<&str>) -> bool,
        word_ok: impl Fn(&str, usize, usize) -> bool,
    ) -> Vec<LineHit<'a>> {
        if needle.is_empty() || cap == 0 {
            return Vec::new();
        }
        let mut hits = Vec::new();
        let finder = memchr::memmem::Finder::new(needle.as_bytes());
        let mut pos = 0usize;
        let mut last_emitted = usize::MAX;
        while let Some(rel) = finder.find(&self.bytes[pos..]) {
            let abs = pos + rel;
            pos = abs + 1;
            let Some(i) = self.line_index_at_byte(abs) else {
                continue;
            };
            if i == last_emitted {
                continue;
            }
            let file = &self.files[self.file_idx[i] as usize];
            if !lang_ok(file.language.as_deref(), lang_filter) {
                continue;
            }
            if word_mode {
                let start = self.starts[i] as usize;
                let content = self.line_content(i);
                let local = abs - start;
                if !word_ok(content, local, needle.len()) {
                    continue;
                }
            }
            last_emitted = i;
            hits.push(self.hit(i));
            if hits.len() >= cap {
                break;
            }
        }
        hits
    }

    /// Case-insensitive / non-ASCII path: per-line verify, still RAM-only.
    pub fn scan_lines<'a>(
        &'a self,
        lang_filter: Option<&str>,
        cap: usize,
        lang_ok: impl Fn(Option<&str>, Option<&str>) -> bool,
        matches: impl Fn(&str) -> bool,
    ) -> Vec<LineHit<'a>> {
        let mut hits = Vec::new();
        for i in 0..self.len() {
            let file = &self.files[self.file_idx[i] as usize];
            if !lang_ok(file.language.as_deref(), lang_filter) {
                continue;
            }
            let content = self.line_content(i);
            if !matches(content) {
                continue;
            }
            hits.push(self.hit(i));
            if hits.len() >= cap {
                break;
            }
        }
        hits
    }

    /// Cascade discovery: identifier needles use the warmed token→file map
    /// (case-folded). Other needles fall back to memchr and jump to the next
    /// file after a hit so a common substring does not walk every remaining
    /// line of that file.
    pub fn scan_distinct_files_cs<'a>(
        &'a self,
        needle: &str,
        file_cap: usize,
        already: &HashSet<String>,
    ) -> Vec<LineHit<'a>> {
        if needle.is_empty() || file_cap == 0 {
            return Vec::new();
        }
        if needle.len() >= 3
            && needle
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            let key = needle.to_ascii_lowercase();
            if let Some(file_ids) = self.token_files.get(&key) {
                return self.hits_from_file_ids(file_ids, file_cap, already);
            }
        }
        let finder = memchr::memmem::Finder::new(needle.as_bytes());
        let mut hits = Vec::new();
        let mut seen = HashSet::new();
        let mut pos = 0usize;
        while let Some(rel) = finder.find(&self.bytes[pos..]) {
            let abs = pos + rel;
            let Some(i) = self.line_index_at_byte(abs) else {
                pos = abs + 1;
                continue;
            };
            let fi = self.file_idx[i];
            let end = self
                .file_byte_ends
                .get(fi as usize)
                .copied()
                .map(|e| e as usize)
                .unwrap_or(self.bytes.len())
                .max(abs + 1);
            pos = end;
            if !seen.insert(fi) {
                continue;
            }
            let hit = self.hit(i);
            if already.contains(hit.path) {
                continue;
            }
            hits.push(hit);
            if hits.len() >= file_cap {
                break;
            }
        }
        hits
    }

    fn hits_from_file_ids<'a>(
        &'a self,
        file_ids: &[u32],
        file_cap: usize,
        already: &HashSet<String>,
    ) -> Vec<LineHit<'a>> {
        let mut preferred = Vec::new();
        let mut rest = Vec::new();
        for &fi in file_ids {
            let Some(file) = self.files.get(fi as usize) else {
                continue;
            };
            if already.contains(&file.path) {
                continue;
            }
            if cascade_prefers_file(&file.path) {
                preferred.push(fi);
            } else {
                rest.push(fi);
            }
        }
        let mut hits = Vec::new();
        for &fi in preferred.iter().chain(rest.iter()) {
            let Some(&line_i) = self.file_first_line.get(fi as usize) else {
                continue;
            };
            if line_i == u32::MAX {
                continue;
            }
            hits.push(self.hit(line_i as usize));
            if hits.len() >= file_cap {
                break;
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(lines: &[(&str, Option<&str>, u32, &str)]) -> LineCorpus {
        let mut bytes = Vec::new();
        let mut starts = Vec::new();
        let mut line_nos = Vec::new();
        let mut file_idx = Vec::new();
        let mut files = Vec::new();
        let mut current: Option<&str> = None;
        for &(path, language, line_no, content) in lines {
            if current != Some(path) {
                current = Some(path);
                files.push(CorpusFile {
                    path: path.to_string(),
                    language: language.map(str::to_string),
                });
            }
            starts.push(bytes.len() as u32);
            bytes.extend_from_slice(content.as_bytes());
            bytes.push(b'\n');
            line_nos.push(line_no);
            file_idx.push((files.len() - 1) as u32);
        }
        let n_files = files.len();
        let mut token_files = HashMap::new();
        for (i, &fi) in file_idx.iter().enumerate() {
            let start = starts[i] as usize;
            let end = starts
                .get(i + 1)
                .copied()
                .map(|s| s as usize)
                .unwrap_or(bytes.len())
                .saturating_sub(1);
            if let Ok(text) = std::str::from_utf8(&bytes[start..end]) {
                push_ascii_tokens(&mut token_files, text, fi);
            }
        }
        LineCorpus {
            index_data_version: 1,
            pragma_data_version: 1,
            bytes,
            starts,
            line_nos,
            file_idx: file_idx.clone(),
            files,
            file_byte_ends: file_byte_ends(&file_idx, &starts, n_files, bytes.len()),
            file_first_line: file_first_lines(&file_idx, n_files),
            token_files,
        }
    }

    #[test]
    fn ident_token_map_finds_camel_case_pieces() {
        let corpus = pack(&[
            ("types.rs", Some("rust"), 10, "pub struct SnapshotStamp {"),
            ("other.rs", Some("rust"), 1, "fn unrelated() {}"),
        ]);
        let already = HashSet::new();
        let hits = corpus.scan_distinct_files_cs("snapshot", 8, &already);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "types.rs");
        assert_eq!(hits[0].line_no, 10);
    }

    #[test]
    fn ident_token_map_prefers_code_over_markdown() {
        let corpus = pack(&[
            ("README.md", None, 1, "snapshot generation notes"),
            ("search/types.rs", Some("rust"), 10, "pub struct SnapshotStamp {"),
        ]);
        let already = HashSet::new();
        let hits = corpus.scan_distinct_files_cs("snapshot", 1, &already);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "search/types.rs");
    }

    #[test]
    fn packed_scan_is_path_sorted_and_respects_cap() {
        let corpus = pack(&[
            ("a.rs", Some("rust"), 1, "alpha SearchHit"),
            ("a.rs", Some("rust"), 2, "nope"),
            ("b.rs", Some("rust"), 10, "SearchHit again"),
            ("c.rs", Some("rust"), 3, "SearchHit third"),
        ]);
        let hits = corpus.scan_cs("SearchHit", false, None, 2, |_, _| true, |_, _, _| true);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, "a.rs");
        assert_eq!(hits[0].line_no, 1);
        assert_eq!(hits[1].path, "b.rs");
        assert_eq!(hits[1].line_no, 10);
    }

    #[test]
    fn packed_scan_does_not_cross_newlines() {
        let corpus = pack(&[("a.rs", None, 1, "Search"), ("a.rs", None, 2, "Hit")]);
        let hits = corpus.scan_cs("SearchHit", false, None, 8, |_, _| true, |_, _, _| true);
        assert!(hits.is_empty());
    }

    #[test]
    fn packed_scan_emits_line_once() {
        let corpus = pack(&[("a.rs", None, 1, "SearchHit and SearchHit again")]);
        let hits = corpus.scan_cs("SearchHit", false, None, 8, |_, _| true, |_, _, _| true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_no, 1);
    }
}
