//! Generation-keyed in-memory line corpus for unique-query literal search.
//!
//! FFF's race is a warm process with files already in RAM. SQLite FTS/LIKE on
//! every unique `literal:` query paid a few milliseconds of parser/join tax on
//! this tree. This corpus loads indexed lines once and scans with `memchr`.

use crate::Result;
use ast_sgrep_embed::split_ident;
use rusqlite::Connection;
use std::collections::HashSet;
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

fn add_token_file(pairs: &mut Vec<(String, u32)>, token: String, fi: u32) {
    if pairs.last().is_some_and(|(t, f)| t == &token && *f == fi) {
        return;
    }
    pairs.push((token, fi));
}

pub fn push_ascii_tokens(pairs: &mut Vec<(String, u32)>, text: &str, fi: u32) {
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
        if raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            add_token_file(pairs, raw.to_owned(), fi);
            continue;
        }
        let token = raw.to_ascii_lowercase();
        for part in split_ident(raw) {
            if part.len() >= 3 {
                add_token_file(pairs, part, fi);
            }
        }
        add_token_file(pairs, token, fi);
    }
}

/// Sorted token names + concatenated file ids. Lookup is binary search.
#[derive(Debug)]
pub struct TokenCsr {
    tokens: Vec<String>,
    offs: Vec<u32>,
    files: Vec<u32>,
}

impl TokenCsr {
    pub fn empty() -> Self {
        Self {
            tokens: Vec::new(),
            offs: vec![0],
            files: Vec::new(),
        }
    }

    pub fn from_pairs(mut pairs: Vec<(String, u32)>) -> Self {
        pairs.sort_unstable();
        pairs.dedup();
        let mut tokens = Vec::new();
        let mut offs = vec![0u32];
        let mut files = Vec::new();
        let mut i = 0;
        while i < pairs.len() {
            let token = std::mem::take(&mut pairs[i].0);
            files.push(pairs[i].1);
            i += 1;
            while i < pairs.len() && pairs[i].0 == token {
                files.push(pairs[i].1);
                i += 1;
            }
            tokens.push(token);
            offs.push(files.len() as u32);
        }
        Self {
            tokens,
            offs,
            files,
        }
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn get(&self, key: &str) -> &[u32] {
        match self.tokens.binary_search_by(|token| token.as_str().cmp(key)) {
            Ok(i) => {
                let start = self.offs[i] as usize;
                let end = self.offs[i + 1] as usize;
                &self.files[start..end]
            }
            Err(_) => &[],
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &[u32])> {
        self.tokens.iter().enumerate().map(|(i, token)| {
            let start = self.offs[i] as usize;
            let end = self.offs[i + 1] as usize;
            (token.as_str(), &self.files[start..end])
        })
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
    /// Owned backing for the SQL fallback; empty when `bundle` is present.
    bytes_owned: Vec<u8>,
    /// Owned backing for the SQL fallback; empty when `bundle` is present.
    folded_bytes_owned: Vec<u8>,
    /// Published read-bundle mapping (source/folded/offsets/postings).
    bundle: Option<std::sync::Arc<crate::store::read_bundle::ReadBundle>>,
    lines: usize,
    starts: Vec<u32>,
    folded_starts: Vec<u32>,
    line_nos: Vec<u32>,
    file_idx: Vec<u32>,
    files: Vec<CorpusFile>,
    /// First packed line of each file (cascade token lookup).
    file_first_line: Vec<u32>,
    /// Lowercased ASCII identifier -> file indexes. Unique-hybrid cascade
    /// looks up rare needles here instead of memchr-ing the whole blob.
    token_files: TokenCsr,
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
        let mut token_files = Vec::new();

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
            bytes_owned: bytes,
            folded_bytes_owned: folded_bytes,
            bundle: None,
            lines: starts.len(),
            starts,
            folded_starts,
            line_nos,
            file_idx,
            files,
            file_first_line,
            token_files: TokenCsr::from_pairs(token_files),
        })))
    }

    /// Open the columnar read-bundle materialization as a corpus view. Offsets
    /// and postings are read from the mapping; only the small per-line start
    /// tables and file metadata are decoded.
    pub fn from_bundle(
        bundle: std::sync::Arc<crate::store::read_bundle::ReadBundle>,
        index_data_version: i64,
        pragma_data_version: i64,
    ) -> Result<Arc<Self>> {
        let lines = bundle.line_count();
        let n_files = bundle.file_count();
        if lines > u32::MAX as usize || n_files > u32::MAX as usize {
            return Err(crate::StoreError::Other(
                "read bundle counts exceed u32 ranges".into(),
            ));
        }
        let file_first_line = (0..n_files).map(|i| bundle.file_first_line(i)).collect();
        let mut files = Vec::with_capacity(n_files);
        for fi in 0..n_files {
            let path = bundle.path(fi).to_string();
            let preferred = cascade_prefers_file(&path);
            files.push(CorpusFile {
                path,
                language: bundle.language(fi).map(str::to_string),
                preferred,
                trigrams: [0u64; FILTER_WORDS],
            });
        }
        Ok(Arc::new(Self {
            index_data_version,
            pragma_data_version,
            bytes_owned: Vec::new(),
            folded_bytes_owned: Vec::new(),
            bundle: Some(bundle),
            lines,
            starts: Vec::new(),
            folded_starts: Vec::new(),
            line_nos: Vec::new(),
            file_idx: Vec::new(),
            files,
            file_first_line,
            token_files: TokenCsr::empty(),
        }))
    }

    fn bytes(&self) -> &[u8] {
        match &self.bundle {
            Some(bundle) => bundle.source(),
            None => &self.bytes_owned,
        }
    }

    fn folded_bytes(&self) -> &[u8] {
        match &self.bundle {
            Some(bundle) => bundle.folded(),
            None => &self.folded_bytes_owned,
        }
    }

    fn token_postings(&self, key: &str) -> &[u32] {
        match &self.bundle {
            Some(bundle) => bundle.token_postings(key),
            None => self.token_files.get(key),
        }
    }

    pub fn len(&self) -> usize {
        self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines == 0
    }

    fn line_start(&self, i: usize) -> u32 {
        match &self.bundle {
            Some(bundle) => bundle.line_start(i),
            None => self.starts[i],
        }
    }

    fn folded_start(&self, i: usize) -> u32 {
        match &self.bundle {
            Some(bundle) => bundle.folded_start(i),
            None => self.folded_starts[i],
        }
    }

    fn line_no_at(&self, i: usize) -> u32 {
        match &self.bundle {
            Some(bundle) => bundle.line_no(i),
            None => self.line_nos[i],
        }
    }

    fn file_idx_at(&self, i: usize) -> u32 {
        match &self.bundle {
            Some(bundle) => bundle.file_idx(i),
            None => self.file_idx[i],
        }
    }

    fn file_trigram(&self, file: usize, word: usize) -> u64 {
        match &self.bundle {
            Some(bundle) => bundle.file_trigram(file, word),
            None => self.files[file].trigrams[word],
        }
    }

    /// Distinct indexed files packed in this corpus (cascade file-cap).
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Join indexed lines `[line_start, line_end]` for `path`. `None` when
    /// the path is not in this corpus (caller falls back to SQL).
    pub fn excerpt(&self, path: &str, line_start: u32, line_end: u32) -> Option<String> {
        let fi = self
            .files
            .binary_search_by(|file| file.path.as_str().cmp(path))
            .ok()?;
        let start = *self.file_first_line.get(fi)? as usize;
        let stop = self
            .file_first_line
            .get(fi + 1)
            .copied()
            .map(|n| n as usize)
            .unwrap_or(self.lines);
        let line_nos: &[u32] = match &self.bundle {
            Some(bundle) => bundle.line_nos(),
            None => &self.line_nos,
        };
        let mut lo = start;
        let mut hi = stop;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if line_nos.get(mid).copied().unwrap_or(u32::MAX) < line_start {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let max = crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES;
        let mut excerpt = String::new();
        for i in lo..stop {
            let no = line_nos.get(i).copied().unwrap_or(u32::MAX);
            if no > line_end {
                break;
            }
            let line = self.line_content(i);
            let separator = usize::from(!excerpt.is_empty());
            if excerpt
                .len()
                .saturating_add(separator)
                .saturating_add(line.len())
                <= max
            {
                if separator == 1 {
                    excerpt.push('\n');
                }
                excerpt.push_str(line);
                continue;
            }
            const MARKER: &str = "…";
            let content_limit = max.saturating_sub(MARKER.len());
            if excerpt.len() > content_limit {
                let mut end = content_limit;
                while end > 0 && !excerpt.is_char_boundary(end) {
                    end -= 1;
                }
                excerpt.truncate(end);
            }
            let mut allowance = content_limit.saturating_sub(excerpt.len());
            if separator == 1 && allowance > 0 {
                excerpt.push('\n');
                allowance -= 1;
            }
            let mut end = allowance.min(line.len());
            while end > 0 && !line.is_char_boundary(end) {
                end -= 1;
            }
            excerpt.push_str(&line[..end]);
            excerpt.push_str(MARKER);
            break;
        }
        Some(excerpt)
    }

    fn line_content_end(&self, i: usize) -> usize {
        if i + 1 < self.lines {
            (self.line_start(i + 1) as usize).saturating_sub(1)
        } else {
            self.bytes().len().saturating_sub(1)
        }
    }

    fn line_content(&self, i: usize) -> &str {
        let start = self.line_start(i) as usize;
        std::str::from_utf8(&self.bytes()[start..self.line_content_end(i)]).unwrap_or("")
    }

    fn hit(&self, i: usize) -> LineHit<'_> {
        let file = &self.files[self.file_idx_at(i) as usize];
        LineHit {
            path: &file.path,
            language: file.language.as_deref(),
            line_no: self.line_no_at(i),
            content: self.line_content(i),
        }
    }

    /// Locate a match only when its whole byte span belongs to one stored line.
    /// Packing separators are not evidence, even when the needle contains LF.
    fn line_index_for_match(&self, abs: usize, match_len: usize) -> Option<usize> {
        if self.lines == 0 {
            return None;
        }
        let target = abs.min(u32::MAX as usize) as u32;
        let (mut lo, mut hi) = (0usize, self.lines);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.line_start(mid) <= target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let i = lo.saturating_sub(1);
        let start = self.line_start(i) as usize;
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
        // Rare identifier/substring needles: the per-file trigram certificate
        // proves which files cannot contain the lowercased needle. Scan only
        // those files instead of the whole arena; exact byte verification
        // still owns every accepted line (trigram collisions only add scans).
        if needle.is_ascii() && needle.len() >= 3 {
            let lower = needle.to_ascii_lowercase();
            let mut candidates = Vec::new();
            for fi in 0..self.files.len() {
                let mut possible = true;
                for tri in lower.as_bytes().windows(3).take(32) {
                    let (word, bit) = trigram_bit(tri);
                    if self.file_trigram(fi, word) & bit == 0 {
                        possible = false;
                        break;
                    }
                }
                if possible {
                    candidates.push(fi);
                }
            }
            if candidates.is_empty() {
                return Vec::new();
            }
            let mut hits = Vec::new();
            let finder = memchr::memmem::Finder::new(needle.as_bytes());
            let mut last_emitted = usize::MAX;
            for fi in candidates {
                let first = self.file_first_line[fi] as usize;
                let end_line = self
                    .file_first_line
                    .get(fi + 1)
                    .copied()
                    .unwrap_or(self.len() as u32) as usize;
                if first >= end_line {
                    continue;
                }
                let start_byte = self.line_start(first) as usize;
                let end_byte = if end_line < self.len() {
                    self.line_start(end_line) as usize
                } else {
                    self.bytes().len()
                };
                let file = &self.files[fi];
                if !lang_ok(file.language.as_deref(), lang_filter) {
                    continue;
                }
                let mut pos = start_byte;
                while let Some(rel) = finder.find(&self.bytes()[pos..end_byte]) {
                    let abs = pos + rel;
                    pos = abs + 1;
                    let Some(i) = self.line_index_for_match(abs, needle.len()) else {
                        continue;
                    };
                    if i == last_emitted {
                        continue;
                    }
                    if word_mode {
                        let start = self.line_start(i) as usize;
                        let content = self.line_content(i);
                        let local = abs - start;
                        if !word_ok(content, local, needle.len()) {
                            continue;
                        }
                    }
                    last_emitted = i;
                    hits.push(self.hit(i));
                    if hits.len() >= cap {
                        return hits;
                    }
                }
            }
            return hits;
        }
        let mut hits = Vec::new();
        let finder = memchr::memmem::Finder::new(needle.as_bytes());
        let mut pos = 0usize;
        let mut last_emitted = usize::MAX;
        while let Some(rel) = finder.find(&self.bytes()[pos..]) {
            let abs = pos + rel;
            pos = abs + 1;
            let Some(i) = self.line_index_for_match(abs, needle.len()) else {
                continue;
            };
            if i == last_emitted {
                continue;
            }
            let file = &self.files[self.file_idx_at(i) as usize];
            if !lang_ok(file.language.as_deref(), lang_filter) {
                continue;
            }
            if word_mode {
                let start = self.line_start(i) as usize;
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
            let file = &self.files[self.file_idx_at(i) as usize];
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

    /// Files whose per-file trigram filter admits every trigram of `needle`.
    ///
    /// The filter is a bloom over the file's folded text: a file that fails
    /// cannot contain the needle, a file that passes may not. Callers use this
    /// as a superset prefilter and decide membership themselves. Indices are
    /// ascending, which is path order.
    pub fn files_may_contain(&self, needle: &str) -> Vec<usize> {
        let lower = needle.to_lowercase();
        let required: Vec<_> = lower
            .as_bytes()
            .windows(3)
            .take(32)
            .map(trigram_bit)
            .collect();
        if required.is_empty() {
            return (0..self.files.len()).collect();
        }
        (0..self.files.len())
            .filter(|&fi| {
                required
                    .iter()
                    .all(|&(word, bit)| self.file_trigram(fi, word) & bit != 0)
            })
            .collect()
    }

    /// Visit every line of `files` in (path, line_no) order.
    ///
    /// `files` must be ascending file indices, which is path order. `visit`
    /// owns admission, scoring, and budget checks; returning `Ok(false)` stops
    /// the scan. This is the regex pass's alternative to a trigram doclist
    /// walk: the caller prunes with [`Self::files_may_contain`] and decides
    /// each line itself, so no per-line index is needed.
    pub fn scan_files<'a>(
        &'a self,
        files: &[usize],
        mut visit: impl FnMut(LineHit<'a>) -> Result<bool>,
    ) -> Result<()> {
        for &fi in files {
            let Some(&start) = self.file_first_line.get(fi) else {
                continue;
            };
            let end = self
                .file_first_line
                .get(fi + 1)
                .copied()
                .unwrap_or(self.len() as u32);
            for i in start as usize..end as usize {
                if !visit(self.hit(i))? {
                    return Ok(());
                }
            }
        }
        Ok(())
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
        let postings = self.token_postings(&key);
        let mut seen = vec![false; self.files.len()];
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
                        .any(|&(word, bit)| self.file_trigram(fi, word) & bit == 0)
                    || already.contains(&file.path)
                    || !lang_ok(file.language.as_deref(), lang_filter)
                    || seen[fi]
                {
                    continue;
                }
                seen[fi] = true;
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
        let bytes_end = if end < self.len() {
            self.folded_start(end) as usize
        } else {
            self.folded_bytes().len()
        };
        let mut pos = self.folded_start(start) as usize;
        while let Some(offset) = finder.find(&self.folded_bytes()[pos..bytes_end]) {
            let abs = pos + offset;
            let target = abs.min(u32::MAX as usize) as u32;
            let (mut lo, mut hi) = (0usize, self.len());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if self.folded_start(mid) <= target {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            let line = lo.saturating_sub(1);
            // Reverify the original record: folds change byte offsets and packed
            // newlines are not evidence. One verification covers its occurrences.
            if matches(self.line_content(line)) {
                return Some(line);
            }
            pos = if line + 1 < self.len() {
                self.folded_start(line + 1) as usize
            } else {
                self.folded_bytes().len()
            };
        }
        None
    }
}
