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

/// Skip the RAM corpus when packed source, lowercase bytes and file filters
/// exceed this. Callers fall back to
/// SQLite. 256 MiB covers typical agent worktrees; 100k-file monorepos that
/// overflow keep the FTS path instead of ballooning RSS.
pub const MAX_CORPUS_BYTES: usize = 256 * 1024 * 1024;

// Conservative file-local byte-trigram certificate. Collisions only admit
// extra scans; exact record verification always owns positive answers.
const FILTER_WORDS: usize = 64;
fn trigram_bit(bytes: &[u8]) -> (usize, u64) {
    let key = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
    let bit = (key.wrapping_mul(0x9e37_79b1) >> 20) as usize;
    (bit / 64, 1u64 << (bit % 64))
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn add_token_file(map: &mut HashMap<String, Vec<u32>>, token: String, fi: u32) {
    let entry = map.entry(token).or_default();
    if entry.last().copied() != Some(fi) {
        entry.push(fi);
    }
}

pub fn push_ascii_tokens(map: &mut HashMap<String, Vec<u32>>, text: &str, fi: u32) {
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
        // These spellings have exactly one split_ident component, identical
        // to the full key. Reuse its posting directly; repeated occurrences
        // in this file need neither another owned key nor another split.
        if raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            if let Some(files) = map.get_mut(raw) {
                if files.last().copied() != Some(fi) {
                    files.push(fi);
                }
            } else {
                map.insert(raw.to_owned(), vec![fi]);
            }
            continue;
        }
        let token = raw.to_ascii_lowercase();
        for part in split_ident(raw) {
            if part.len() >= 3 {
                add_token_file(map, part, fi);
            }
        }
        add_token_file(map, token, fi);
    }
}

pub fn file_first_lines(file_idx: &[u32], n_files: usize) -> Vec<u32> {
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

pub(crate) fn cascade_prefers_file(path: &str) -> bool {
    // DB keys already normalize host separators; a Unix backslash is content.
    let normalized = path.to_ascii_lowercase();
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

#[derive(Debug, Clone)]
pub struct CorpusFile {
    pub path: String,
    pub language: Option<String>,
    preferred: bool,
    trigrams: [u64; FILTER_WORDS],
}

/// Packed `lines` table: path-sorted, one record per indexed line.
#[derive(Debug)]
pub struct LineCorpus {
    pub index_data_version: i64,
    pub pragma_data_version: i64,
    bytes: Vec<u8>,
    starts: Vec<u32>,
    /// Record-local Unicode lowercase, cached under the same generation as source.
    folded_bytes: Vec<u8>,
    folded_starts: Vec<u32>,
    line_nos: Vec<u32>,
    file_idx: Vec<u32>,
    files: Vec<CorpusFile>,
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
        let mut folded_bytes = Vec::new();
        let mut folded_starts = Vec::new();
        let mut line_nos = Vec::new();
        let mut file_idx = Vec::new();
        let mut files = Vec::new();
        let mut current_path: Option<String> = None;
        let mut token_files: HashMap<String, Vec<u32>> = HashMap::new();

        for row in rows {
            let (path, language, line_no, content) = row?;
            let folded = content.to_lowercase();
            if bytes
                .len()
                .saturating_add(folded_bytes.len())
                .saturating_add(content.len())
                .saturating_add(folded.len())
                .saturating_add(2)
                .saturating_add(
                    files
                        .len()
                        .saturating_add(1)
                        .saturating_mul(FILTER_WORDS * 8),
                )
                > MAX_CORPUS_BYTES
            {
                return Ok(None);
            }
            if current_path.as_deref() != Some(path.as_str()) {
                current_path = Some(path.clone());
                let preferred = cascade_prefers_file(&path);
                files.push(CorpusFile {
                    path,
                    language,
                    preferred,
                    trigrams: [0; FILTER_WORDS],
                });
            }
            let idx = (files.len() - 1) as u32;
            for tri in folded.as_bytes().windows(3) {
                let (word, bit) = trigram_bit(tri);
                files[idx as usize].trigrams[word] |= bit;
            }
            push_ascii_tokens(&mut token_files, &content, idx);
            starts.push(bytes.len() as u32);
            folded_starts.push(folded_bytes.len() as u32);
            folded_bytes.extend_from_slice(folded.as_bytes());
            folded_bytes.push(b'\n');
            bytes.extend_from_slice(content.as_bytes());
            bytes.push(b'\n');
            line_nos.push(line_no);
            file_idx.push(idx);
        }

        let n_files = files.len();
        let file_first_line = file_first_lines(&file_idx, n_files);
        Ok(Some(Arc::new(Self {
            index_data_version,
            pragma_data_version,
            bytes,
            starts,
            folded_bytes,
            folded_starts,
            line_nos,
            file_idx,
            files,
            file_first_line,
            token_files,
        })))
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// Distinct indexed files packed in this corpus (cascade file-cap).
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    fn line_content_end(&self, i: usize) -> usize {
        self.starts
            .get(i + 1)
            .copied()
            .map(|s| s as usize)
            .unwrap_or(self.bytes.len())
            .saturating_sub(1)
    }

    fn line_content(&self, i: usize) -> &str {
        let start = self.starts[i] as usize;
        std::str::from_utf8(&self.bytes[start..self.line_content_end(i)]).unwrap_or("")
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

    /// Locate a match only when its whole byte span belongs to one stored line.
    /// Packing separators are not evidence, even when the needle contains LF.
    fn line_index_for_match(&self, abs: usize, match_len: usize) -> Option<usize> {
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
        let content_end = self.line_content_end(i);
        if abs >= start && abs < content_end && match_len <= content_end - abs {
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
            let Some(i) = self.line_index_for_match(abs, needle.len()) else {
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

    /// Bounded distinct-file discovery. Token postings seed each preference
    /// group, but unused slots must be filled by residual matches. The caller
    /// supplies the same per-record verifier as cold search, so Unicode folds
    /// and packed separators cannot change membership or representative lines.
    pub fn scan_distinct_files<'a>(
        &'a self,
        needle: &str,
        file_cap: usize,
        already: &HashSet<String>,
        lang_filter: Option<&str>,
        lang_ok: impl Fn(Option<&str>, Option<&str>) -> bool,
        matches: impl Fn(&str) -> bool,
    ) -> Vec<LineHit<'a>> {
        if needle.is_empty() || file_cap == 0 {
            return Vec::new();
        }
        let key = needle.to_ascii_lowercase();
        let lower = needle.to_lowercase();
        let finder = memchr::memmem::Finder::new(lower.as_bytes());
        // Any absent gram proves there is no lowercase substring in this file.
        // Short needles have no certificate and use the exact path unchanged.
        let required: Vec<_> = lower
            .as_bytes()
            .windows(3)
            .take(32)
            .map(trigram_bit)
            .collect();
        let postings = self
            .token_files
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for preferred in [true, false] {
            for fi in postings
                .iter()
                .map(|&fi| fi as usize)
                .chain(0..self.files.len())
            {
                let file = &self.files[fi];
                if file.preferred != preferred
                    || required
                        .iter()
                        .any(|&(word, bit)| file.trigrams[word] & bit == 0)
                    || already.contains(&file.path)
                    || !lang_ok(file.language.as_deref(), lang_filter)
                    || !seen.insert(fi)
                {
                    continue;
                }
                let Some(line) = self.first_matching_line(fi, &finder, &matches) else {
                    continue;
                };
                hits.push(self.hit(line));
                if hits.len() >= file_cap {
                    return hits;
                }
            }
        }
        hits
    }

    fn first_matching_line(
        &self,
        fi: usize,
        finder: &memchr::memmem::Finder<'_>,
        matches: &impl Fn(&str) -> bool,
    ) -> Option<usize> {
        let start = *self.file_first_line.get(fi)? as usize;
        let end = self
            .file_first_line
            .get(fi + 1)
            .copied()
            .unwrap_or(self.len() as u32) as usize;
        let bytes_end = self
            .folded_starts
            .get(end)
            .copied()
            .unwrap_or(self.folded_bytes.len() as u32) as usize;
        let mut pos = self.folded_starts[start] as usize;
        while let Some(offset) = finder.find(&self.folded_bytes[pos..bytes_end]) {
            let abs = pos + offset;
            let line = self.folded_starts.partition_point(|&s| s as usize <= abs) - 1;
            // Reverify the original record: folds change byte offsets and packed
            // newlines are not evidence. One verification covers its occurrences.
            if matches(self.line_content(line)) {
                return Some(line);
            }
            pos = self
                .folded_starts
                .get(line + 1)
                .copied()
                .unwrap_or(self.folded_bytes.len() as u32) as usize;
        }
        None
    }
}
