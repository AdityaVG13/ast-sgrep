//! `read.v1` columnar read bundle.
//!
//! One mmap'd file beside the index DB carries the path-sorted line arena,
//! folded arena, line metadata, identifier postings and per-file trigram
//! filters. It is built from the committed SQLite rows at index finalize and
//! published by fsync + rename, so readers keep the old mapping until reopen.
//!
//! The format is versioned independently of the SQLite schema. A reader that
//! sees a different rev/schema or a generation mismatch must refuse the file
//! rather than serve mixed snapshots; `ASGREP_READPATH=bundle` makes that
//! refusal an error. `ASGREP_READPATH=sql` is the migration control arm.

use crate::store::line_corpus::{push_ascii_tokens, TokenCsr};
use crate::store::{IndexStore, INDEX_SCHEMA_VERSION};
use crate::Result;
use ast_sgrep_mmap::Mmap;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const BUNDLE_REV: u32 = 5;
pub const BUNDLE_REV_MIN: u32 = 3;
pub const MAGIC: &[u8; 8] = b"ASGRD001";
const HEADER_LEN: usize = 1024;
const SECTION_COUNT: usize = 24;
const SECTION_COUNT_V3: usize = 22;
const SECTION_BASE: usize = 128;
const WORKTREE_REVISION_OFF: usize = 768;
const SEMANTIC_PRESENT_OFF: usize = 776;
const SEMANTIC_EXPECTED_OFF: usize = 784;

const SEC_PATH_BYTES: usize = 0;
const SEC_PATH_OFFSETS: usize = 1;
const SEC_LANG_BYTES: usize = 2;
const SEC_LANG_OFFSETS: usize = 3;
const SEC_SOURCE: usize = 4;
const SEC_LINE_STARTS: usize = 5;
const SEC_FOLDED: usize = 6;
const SEC_FOLDED_STARTS: usize = 7;
const SEC_LINE_NOS: usize = 8;
const SEC_FILE_IDX: usize = 9;
const SEC_FILE_FIRST: usize = 10;
const SEC_TOKEN_BYTES: usize = 11;
const SEC_TOKEN_OFFSETS: usize = 12;
const SEC_POSTING_OFFSETS: usize = 13;
const SEC_POSTINGS: usize = 14;
const SEC_TRIGRAMS: usize = 15;

const FILTER_WORDS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPath {
    Sql,
    Bundle,
    /// Use the bundle when it exists, SQL when it does not. This is the
    /// migration default: explicit `bundle` remains fail-closed.
    Auto,
}

/// `auto` uses the bundle when present, SQL when not. Explicit `sql`/`bundle`
/// remain the control and fail-closed arms.
pub fn readpath() -> ReadPath {
    match std::env::var("ASGREP_READPATH").ok().as_deref() {
        Some("bundle") => ReadPath::Bundle,
        Some("sql") => ReadPath::Sql,
        _ => ReadPath::Auto,
    }
}

pub fn bundle_enabled() -> bool {
    readpath() != ReadPath::Sql
}

pub fn path_for(db_path: &Path) -> PathBuf {
    db_path.with_extension("read.v1")
}

/// Cheap file identity for a mapped bundle. Rename-publication changes the
/// inode/mtime, so a long-lived reader can reopen without trusting a stale
/// generation header.
pub(crate) fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some((metadata.len(), mtime))
}

fn trigram_bit(bytes: &[u8]) -> (usize, u64) {
    let key = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
    let bit = (key.wrapping_mul(0x9e37_79b1) >> 20) as usize;
    (bit / 64, 1u64 << (bit % 64))
}

#[derive(Debug, Clone, Copy)]
struct Section {
    off: u64,
    len: u64,
}

#[derive(Debug, Clone)]
pub struct BundleHeader {
    pub generation: i64,
    pub files: u64,
    pub lines: u64,
    pub tokens: u64,
    pub source_len: u64,
    pub folded_len: u64,
    pub fingerprint: u64,
    /// Highest indexed file mtime, captured at bundle build time.
    pub worktree_revision: i64,
    /// ANN fingerprint the semantic sidecar must carry for this generation.
    pub semantic_expected: Option<[u8; 32]>,
    pub rev: u32,
    sections: [Section; SECTION_COUNT],
}

impl BundleHeader {
    fn section(&self, index: usize) -> Section {
        self.sections[index]
    }
}

/// Read-only view over a published bundle. Holding the mapping keeps the
/// inode stable across concurrent reindex publishes (rename, never in-place).
pub struct ReadBundle {
    map: Mmap,
    header: BundleHeader,
    semantic: std::sync::OnceLock<Option<std::sync::Arc<crate::store::bundle_semantic::SemanticView>>>,
}

impl std::fmt::Debug for ReadBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadBundle")
            .field("generation", &self.header.generation)
            .field("files", &self.header.files)
            .field("lines", &self.header.lines)
            .field("tokens", &self.header.tokens)
            .finish()
    }
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("u32 span"))
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("u64 span"))
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn fingerprint(
    generation: i64,
    files: u64,
    lines: u64,
    tokens: u64,
    source_len: u64,
    folded_len: u64,
    worktree_revision: i64,
    semantic_expected: Option<[u8; 32]>,
    sections: &[Section],
) -> u64 {
    let mut bytes = Vec::with_capacity(80 + sections.len() * 16);
    put_u64(&mut bytes, generation as u64);
    put_u64(&mut bytes, files);
    put_u64(&mut bytes, lines);
    put_u64(&mut bytes, tokens);
    put_u64(&mut bytes, source_len);
    put_u64(&mut bytes, folded_len);
    put_u64(&mut bytes, worktree_revision as u64);
    match semantic_expected {
        Some(expected) => {
            put_u64(&mut bytes, 1);
            bytes.extend_from_slice(&expected);
        }
        None => put_u64(&mut bytes, 0),
    }
    for section in sections {
        put_u64(&mut bytes, section.off);
        put_u64(&mut bytes, section.len);
    }
    fnv1a(&bytes)
}

fn encode_header(header: &BundleHeader) -> [u8; HEADER_LEN] {
    let mut out = [0u8; HEADER_LEN];
    out[..8].copy_from_slice(MAGIC);
    out[8..12].copy_from_slice(&BUNDLE_REV.to_le_bytes());
    out[12..16].copy_from_slice(&(INDEX_SCHEMA_VERSION as u32).to_le_bytes());
    out[16..24].copy_from_slice(&header.generation.to_le_bytes());
    out[24..32].copy_from_slice(&header.files.to_le_bytes());
    out[32..40].copy_from_slice(&header.lines.to_le_bytes());
    out[40..48].copy_from_slice(&header.tokens.to_le_bytes());
    out[48..56].copy_from_slice(&header.source_len.to_le_bytes());
    out[56..64].copy_from_slice(&header.folded_len.to_le_bytes());
    out[64..72].copy_from_slice(&header.fingerprint.to_le_bytes());
    for (index, section) in header.sections.iter().enumerate() {
        let at = SECTION_BASE + index * 16;
        out[at..at + 8].copy_from_slice(&section.off.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&section.len.to_le_bytes());
    }
    out[WORKTREE_REVISION_OFF..WORKTREE_REVISION_OFF + 8]
        .copy_from_slice(&header.worktree_revision.to_le_bytes());
    if let Some(expected) = header.semantic_expected {
        out[SEMANTIC_PRESENT_OFF] = 1;
        out[SEMANTIC_EXPECTED_OFF..SEMANTIC_EXPECTED_OFF + 32].copy_from_slice(&expected);
    }
    out
}

fn decode_header(bytes: &[u8]) -> Result<BundleHeader> {
    if bytes.len() < HEADER_LEN {
        return Err(crate::StoreError::Other("read.v1 header truncated".into()));
    }
    if &bytes[..8] != MAGIC {
        return Err(crate::StoreError::Other("read.v1 magic mismatch".into()));
    }
    let rev = read_u32(bytes, 8);
    if rev < BUNDLE_REV_MIN || rev > BUNDLE_REV {
        return Err(crate::StoreError::Other(format!(
            "read.v1 bundle rev {rev} not in {BUNDLE_REV_MIN}..={BUNDLE_REV}; reindex required"
        )));
    }
    let schema = read_u32(bytes, 12);
    if i64::from(schema) != INDEX_SCHEMA_VERSION {
        return Err(crate::StoreError::Other(format!(
            "read.v1 schema {schema} != {INDEX_SCHEMA_VERSION}; reindex required"
        )));
    }
    let generation = i64::from_le_bytes(bytes[16..24].try_into().expect("generation span"));
    let files = read_u64(bytes, 24);
    let lines = read_u64(bytes, 32);
    let tokens = read_u64(bytes, 40);
    let source_len = read_u64(bytes, 48);
    let folded_len = read_u64(bytes, 56);
    let recorded_fingerprint = read_u64(bytes, 64);
    let worktree_revision = i64::from_le_bytes(
        bytes[WORKTREE_REVISION_OFF..WORKTREE_REVISION_OFF + 8]
            .try_into()
            .expect("worktree revision span"),
    );
    let semantic_expected = if bytes[SEMANTIC_PRESENT_OFF] != 0 {
        let mut expected = [0u8; 32];
        expected.copy_from_slice(&bytes[SEMANTIC_EXPECTED_OFF..SEMANTIC_EXPECTED_OFF + 32]);
        Some(expected)
    } else {
        None
    };
    let mut sections = [Section { off: 0, len: 0 }; SECTION_COUNT];
    let nsec = if rev == 3 {
        SECTION_COUNT_V3
    } else {
        SECTION_COUNT
    };
    for (index, section) in sections.iter_mut().enumerate().take(nsec) {
        let at = SECTION_BASE + index * 16;
        section.off = read_u64(bytes, at);
        section.len = read_u64(bytes, at + 8);
        let end = section
            .off
            .checked_add(section.len)
            .ok_or_else(|| crate::StoreError::Other("read.v1 section overflow".into()))?;
        if end > bytes.len() as u64 {
            return Err(crate::StoreError::Other(format!(
                "read.v1 section {index} outside file"
            )));
        }
    }
    let header = BundleHeader {
        generation,
        files,
        lines,
        tokens,
        source_len,
        folded_len,
        fingerprint: recorded_fingerprint,
        worktree_revision,
        semantic_expected,
        rev,
        sections,
    };
    let expected = fingerprint(
        generation,
        files,
        lines,
        tokens,
        source_len,
        folded_len,
        worktree_revision,
        semantic_expected,
        &sections[..nsec],
    );
    if expected != recorded_fingerprint {
        return Err(crate::StoreError::Other(
            "read.v1 header fingerprint mismatch".into(),
        ));
    }
    Ok(header)
}

impl ReadBundle {
    /// Open a bundle. `None` when the file is absent; malformed/version
    /// mismatches are errors so callers cannot silently fall back under
    /// `ASGREP_READPATH=bundle`.
    pub fn open(db_path: &Path) -> Result<Option<Self>> {
        let path = path_for(db_path);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let map = ast_sgrep_mmap::map_readonly(&file)?;
        let header = decode_header(&map)?;
        let bundle = Self {
            map,
            header,
            semantic: std::sync::OnceLock::new(),
        };
        bundle.validate()?;
        Ok(Some(bundle))
    }

    pub fn open_for_generation(db_path: &Path, generation: i64) -> Result<Option<Self>> {
        match Self::open(db_path)? {
            Some(bundle) if bundle.header.generation == generation => Ok(Some(bundle)),
            Some(bundle) => Err(crate::StoreError::Other(format!(
                "read.v1 generation {} != store {generation}; reindex required",
                bundle.header.generation
            ))
            .into()),
            None => Ok(None),
        }
    }

    fn validate(&self) -> Result<()> {
        let files = self.header.files as usize;
        let lines = self.header.lines as usize;
        let tokens = self.header.tokens as usize;
        let checks: &[(usize, usize)] = &[
            (SEC_PATH_OFFSETS, files + 1),
            (SEC_LANG_OFFSETS, files + 1),
            (SEC_LINE_STARTS, lines + 1),
            (SEC_FOLDED_STARTS, lines + 1),
            (SEC_LINE_NOS, lines),
            (SEC_FILE_IDX, lines),
            (SEC_FILE_FIRST, files),
            (SEC_TOKEN_OFFSETS, tokens + 1),
            (SEC_POSTING_OFFSETS, tokens + 1),
        ];
        for &(index, count) in checks {
            let section = self.header.section(index);
            if section.len as usize != count.saturating_mul(std::mem::size_of::<u32>()) {
                return Err(crate::StoreError::Other(format!(
                    "read.v1 section {index} length mismatch"
                )));
            }
        }
        if self.section(SEC_SOURCE).len() != self.header.source_len as usize
            || self.section(SEC_FOLDED).len() != self.header.folded_len as usize
        {
            return Err(crate::StoreError::Other(
                "read.v1 arena length mismatch".into(),
            ));
        }
        if self.section(SEC_POSTINGS).len() % 4 != 0 {
            return Err(crate::StoreError::Other(
                "read.v1 postings length misaligned".into(),
            ));
        }
        if self.section(SEC_TRIGRAMS).len() != files.saturating_mul(FILTER_WORDS).saturating_mul(8)
        {
            return Err(crate::StoreError::Other(
                "read.v1 trigram section length mismatch".into(),
            ));
        }
        Ok(())
    }

    pub fn generation(&self) -> i64 {
        self.header.generation
    }

    pub fn rev(&self) -> u32 {
        self.header.rev
    }

    pub fn file_count(&self) -> usize {
        self.header.files as usize
    }

    pub fn line_count(&self) -> usize {
        self.header.lines as usize
    }

    pub fn token_count(&self) -> usize {
        self.header.tokens as usize
    }

    pub fn worktree_revision(&self) -> i64 {
        self.header.worktree_revision
    }

    pub fn semantic_expected(&self) -> Option<[u8; 32]> {
        self.header.semantic_expected
    }

    pub fn semantic_view(
        &self,
    ) -> Option<std::sync::Arc<crate::store::bundle_semantic::SemanticView>> {
        self.semantic
            .get_or_init(|| {
                crate::store::bundle_semantic::decode_view(self).map(std::sync::Arc::new)
            })
            .clone()
    }

    pub fn source(&self) -> &[u8] {
        self.section(SEC_SOURCE)
    }

    pub fn folded(&self) -> &[u8] {
        self.section(SEC_FOLDED)
    }

    pub fn section(&self, index: usize) -> &[u8] {
        let section = self.header.section(index);
        &self.map[section.off as usize..(section.off + section.len) as usize]
    }

    fn u32s(&self, index: usize) -> &[u32] {
        let bytes = self.section(index);
        bytemuck::cast_slice(bytes)
    }

    pub fn line_nos(&self) -> &[u32] {
        self.u32s(SEC_LINE_NOS)
    }

    pub fn line_start(&self, index: usize) -> u32 {
        self.u32s(SEC_LINE_STARTS)[index]
    }

    pub fn folded_start(&self, index: usize) -> u32 {
        self.u32s(SEC_FOLDED_STARTS)[index]
    }

    pub fn line_no(&self, index: usize) -> u32 {
        self.u32s(SEC_LINE_NOS)[index]
    }

    pub fn file_idx(&self, index: usize) -> u32 {
        self.u32s(SEC_FILE_IDX)[index]
    }

    pub fn file_first_line(&self, index: usize) -> u32 {
        self.u32s(SEC_FILE_FIRST)[index]
    }

    pub fn path(&self, file: usize) -> &str {
        let offsets = self.u32s(SEC_PATH_OFFSETS);
        let bytes = self.section(SEC_PATH_BYTES);
        let start = offsets[file] as usize;
        let end = offsets[file + 1] as usize;
        std::str::from_utf8(&bytes[start..end]).unwrap_or("")
    }

    pub fn language(&self, file: usize) -> Option<&str> {
        let offsets = self.u32s(SEC_LANG_OFFSETS);
        let bytes = self.section(SEC_LANG_BYTES);
        let start = offsets[file] as usize;
        let end = offsets[file + 1] as usize;
        if start == end {
            None
        } else {
            Some(std::str::from_utf8(&bytes[start..end]).unwrap_or(""))
        }
    }

    pub fn file_trigram(&self, file: usize, word: usize) -> u64 {
        let bytes = self.section(SEC_TRIGRAMS);
        let at = (file * FILTER_WORDS + word) * 8;
        u64::from_le_bytes(bytes[at..at + 8].try_into().expect("trigram span"))
    }

    /// Posting list for one lowercased identifier. Sorted token table +
    /// binary search keeps this out of process heap state.
    pub fn token_postings(&self, key: &str) -> &[u32] {
        let count = self.header.tokens as usize;
        if count == 0 {
            return &[];
        }
        let offsets = self.u32s(SEC_TOKEN_OFFSETS);
        let bytes = self.section(SEC_TOKEN_BYTES);
        let (mut lo, mut hi) = (0usize, count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let token = &bytes[offsets[mid] as usize..offsets[mid + 1] as usize];
            match token.cmp(key.as_bytes()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    let posting_offsets = self.u32s(SEC_POSTING_OFFSETS);
                    let postings = self.u32s(SEC_POSTINGS);
                    let start = posting_offsets[mid] as usize;
                    let end = posting_offsets[mid + 1] as usize;
                    return &postings[start..end];
                }
            }
        }
        &[]
    }
}

struct BuildFile {
    trigrams: [u64; FILTER_WORDS],
}

/// Build and atomically publish a bundle from already-materialized rows.
/// Public for the line-corpus round-trip test; the index path uses
/// [`write_for_store`].
pub fn write_from_rows(
    db_path: &Path,
    rows: &[crate::store::IndexedLineRow],
    generation: i64,
) -> Result<()> {
    write_from_rows_with_metadata(
        db_path,
        rows,
        generation,
        0,
        None,
        crate::store::bundle_graph::GraphSections::default(),
        crate::store::bundle_semantic::SemanticSections::default(),
    )
}

/// Build a bundle with the generation-derived stamp metadata carried in the
/// header, so readers never scan `files` or `semantic_chunks` for it.
pub(crate) fn write_from_rows_with_metadata(
    db_path: &Path,
    rows: &[crate::store::IndexedLineRow],
    generation: i64,
    worktree_revision: i64,
    semantic_expected: Option<[u8; 32]>,
    graph: crate::store::bundle_graph::GraphSections,
    semantic: crate::store::bundle_semantic::SemanticSections,
) -> Result<()> {
    let mut owned = rows.to_vec();
    owned.sort_by(|left, right| {
        left.0
            .as_ref()
            .cmp(right.0.as_ref())
            .then(left.1.cmp(&right.1))
    });
    let rows = owned.as_slice();
    let mut files: Vec<BuildFile> = Vec::new();
    let mut path_bytes = Vec::new();
    let mut path_offsets = Vec::with_capacity(8);
    let mut lang_bytes = Vec::new();
    let mut lang_offsets = Vec::with_capacity(8);
    let mut source = Vec::new();
    let mut folded = Vec::new();
    let mut line_starts = Vec::with_capacity(rows.len() + 1);
    let mut folded_starts = Vec::with_capacity(rows.len() + 1);
    let mut line_nos = Vec::with_capacity(rows.len());
    let mut file_idx = Vec::with_capacity(rows.len());
    let mut token_files = Vec::new();
    let mut current_path: Option<&str> = None;

    for (path, line_no, content, language) in rows {
        if current_path != Some(path.as_ref()) {
            current_path = Some(path.as_ref());
            path_offsets.push(
                u32::try_from(path_bytes.len()).map_err(|_| {
                    crate::StoreError::Other("read.v1 path arena exceeds u32".into())
                })?,
            );
            path_bytes.extend_from_slice(path.as_bytes());
            lang_offsets.push(u32::try_from(lang_bytes.len()).map_err(|_| {
                crate::StoreError::Other("read.v1 language arena exceeds u32".into())
            })?);
            if let Some(language) = language {
                lang_bytes.extend_from_slice(language.as_bytes());
            }
            files.push(BuildFile {
                trigrams: [0; FILTER_WORDS],
            });
        }
        let fi = (files.len() - 1) as u32;
        let folded_line = content.to_lowercase();
        for tri in folded_line.as_bytes().windows(3) {
            let (word, bit) = trigram_bit(tri);
            files[fi as usize].trigrams[word] |= bit;
        }
        push_ascii_tokens(&mut token_files, content, fi);
        line_starts
            .push(u32::try_from(source.len()).map_err(|_| {
                crate::StoreError::Other("read.v1 source arena exceeds u32".into())
            })?);
        folded_starts
            .push(u32::try_from(folded.len()).map_err(|_| {
                crate::StoreError::Other("read.v1 folded arena exceeds u32".into())
            })?);
        source.extend_from_slice(content.as_bytes());
        source.push(b'\n');
        folded.extend_from_slice(folded_line.as_bytes());
        folded.push(b'\n');
        line_nos.push(*line_no);
        file_idx.push(fi);
    }
    path_offsets.push(
        u32::try_from(path_bytes.len())
            .map_err(|_| crate::StoreError::Other("read.v1 path arena exceeds u32".into()))?,
    );
    lang_offsets.push(
        u32::try_from(lang_bytes.len())
            .map_err(|_| crate::StoreError::Other("read.v1 language arena exceeds u32".into()))?,
    );
    line_starts.push(
        u32::try_from(source.len())
            .map_err(|_| crate::StoreError::Other("read.v1 source arena exceeds u32".into()))?,
    );
    folded_starts.push(
        u32::try_from(folded.len())
            .map_err(|_| crate::StoreError::Other("read.v1 folded arena exceeds u32".into()))?,
    );
    let mut file_first = vec![0u32; files.len()];
    for (line, &fi) in file_idx.iter().enumerate() {
        let slot = &mut file_first[fi as usize];
        if *slot == 0 && line != 0 {
            *slot = line as u32;
        }
    }
    if let Some(first) = file_first.first_mut() {
        *first = 0;
    }

    let tokens = TokenCsr::from_pairs(token_files);
    let mut token_bytes = Vec::new();
    let mut token_offsets = Vec::with_capacity(tokens.entries().count() + 1);
    let mut posting_offsets = Vec::with_capacity(tokens.entries().count() + 1);
    let mut postings = Vec::new();
    token_offsets.push(0);
    posting_offsets.push(0);
    for (token, files) in tokens.entries() {
        token_bytes.extend_from_slice(token.as_bytes());
        token_offsets.push(token_bytes.len() as u32);
        postings.extend_from_slice(files);
        posting_offsets.push(postings.len() as u32);
    }

    let sections_data: [Vec<u8>; SECTION_COUNT] = [
        path_bytes,
        bytemuck::cast_slice(&path_offsets).to_vec(),
        lang_bytes,
        bytemuck::cast_slice(&lang_offsets).to_vec(),
        source,
        bytemuck::cast_slice(&line_starts).to_vec(),
        folded,
        bytemuck::cast_slice(&folded_starts).to_vec(),
        bytemuck::cast_slice(&line_nos).to_vec(),
        bytemuck::cast_slice(&file_idx).to_vec(),
        bytemuck::cast_slice(&file_first).to_vec(),
        token_bytes,
        bytemuck::cast_slice(&token_offsets).to_vec(),
        bytemuck::cast_slice(&posting_offsets).to_vec(),
        bytemuck::cast_slice(&postings).to_vec(),
        {
            let mut bytes = Vec::with_capacity(files.len() * FILTER_WORDS * 8);
            for file in &files {
                bytes.extend_from_slice(bytemuck::cast_slice(&file.trigrams));
            }
            bytes
        },
        graph.df,
        graph.symbols,
        graph.callers,
        graph.imports,
        graph.patterns,
        graph.lexicon,
        semantic.meta,
        semantic.vecs,
    ];
    let source_len = sections_data[SEC_SOURCE].len() as u64;
    let folded_len = sections_data[SEC_FOLDED].len() as u64;

    let path = path_for(db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!("read.v1.tmp.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)?;
    file.write_all(&[0u8; HEADER_LEN])?;
    let mut sections = [Section { off: 0, len: 0 }; SECTION_COUNT];
    for (index, data) in sections_data.iter().enumerate() {
        let padding = (8 - (file.stream_position()? % 8)) % 8;
        if padding > 0 {
            file.write_all(&[0u8; 8][..padding as usize])?;
        }
        let offset = file.stream_position()?;
        file.write_all(data)?;
        sections[index] = Section {
            off: offset,
            len: data.len() as u64,
        };
    }
    let fingerprint = fingerprint(
        generation,
        files.len() as u64,
        rows.len() as u64,
        tokens.len() as u64,
        source_len,
        folded_len,
        worktree_revision,
        semantic_expected,
        &sections,
    );
    let header = BundleHeader {
        generation,
        files: files.len() as u64,
        lines: rows.len() as u64,
        tokens: tokens.len() as u64,
        source_len,
        folded_len,
        fingerprint,
        worktree_revision,
        semantic_expected,
        rev: BUNDLE_REV,
        sections,
    };
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&encode_header(&header))?;
    file.sync_all()?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn write_for_store(store: &IndexStore, generation: i64) -> Result<()> {
    if crate::semantic_ivf::is_in_memory_db(store.db_path()) {
        return Ok(());
    }
    let rows = store.all_indexed_lines()?;
    let worktree_revision = store.worktree_revision()?;
    let stats = store.semantic_chunk_stats(None)?;
    let semantic_expected = if stats.count == 0 || stats.dim == 0 {
        None
    } else {
        let backend = store.get_meta("embed_backend")?;
        Some(crate::semantic_ivf::compute_ann_fingerprint(
            stats.count,
            stats.max_id,
            stats.dim,
            backend.as_deref(),
            generation,
        ))
    };
    let graph = crate::store::bundle_graph::pack_from_store(store, &rows)?;
    let semantic = crate::store::bundle_semantic::pack_from_store(store, &rows)?;
    write_from_rows_with_metadata(
        store.db_path(),
        &rows,
        generation,
        worktree_revision,
        semantic_expected,
        graph,
        semantic,
    )
}

/// True when the on-disk bundle already matches `generation` and its rev.
pub fn is_fresh(db_path: &Path, generation: i64) -> bool {
    match ReadBundle::open(db_path) {
        Ok(Some(bundle)) => bundle.generation() == generation && bundle.rev() == BUNDLE_REV,
        _ => false,
    }
}
