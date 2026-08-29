//! Generation-keyed in-memory line corpus for unique-query literal search.
//!
//! FFF's race is a warm process with files already in RAM. SQLite FTS/LIKE on
//! every unique `literal:` query paid a few milliseconds of parser/join tax on
//! this tree. This corpus loads indexed lines once and scans with `memchr`.

use crate::Result;
use rusqlite::Connection;
use std::sync::Arc;

/// Skip the RAM corpus when packed source exceeds this. Callers fall back to
/// SQLite. 256 MiB covers typical agent worktrees; 100k-file monorepos that
/// overflow keep the FTS path instead of ballooning RSS.
pub const MAX_CORPUS_BYTES: usize = 256 * 1024 * 1024;

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
            starts.push(bytes.len() as u32);
            bytes.extend_from_slice(content.as_bytes());
            bytes.push(b'\n');
            line_nos.push(line_no);
            file_idx.push(idx);
        }

        Ok(Some(Arc::new(Self {
            index_data_version,
            pragma_data_version,
            bytes,
            starts,
            line_nos,
            file_idx,
            files,
        })))
    }

    pub fn len(&self) -> usize {
        self.starts.len()
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
        LineCorpus {
            index_data_version: 1,
            pragma_data_version: 1,
            bytes,
            starts,
            line_nos,
            file_idx,
            files,
        }
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
