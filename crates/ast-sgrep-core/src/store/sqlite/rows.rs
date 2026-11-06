use ast_sgrep_lang::PatternNode;

#[derive(Debug, Clone)]
pub(crate) struct CallEvidenceRow {
    pub(crate) file: String,
    pub(crate) line: u32,
    pub(crate) caller: String,
    pub(crate) callee: String,
    pub(crate) scip_exact: bool,
}

/// INTEGER byte offsets must fit `i64` on write (pb2w). Wrapping `as i64`
/// would store a negative and later wrap to `usize::MAX` on a 64-bit host.
pub(crate) fn sql_i64_from_byte_offset(value: usize) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|_| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("byte offset {value} exceeds SQLite INTEGER storage"),
        )))
    })
}

/// Reject negative / out-of-range INTEGER byte offsets on read (pb2w).
pub(crate) fn sql_usize_from_byte_offset(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<usize> {
    let raw: i64 = row.get(idx)?;
    usize::try_from(raw).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(idx, raw))
}

pub struct PatternNodeRow {
    pub path: String,
    pub language: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticChunkStats {
    pub count: usize,
    pub max_id: i64,
    pub dim: usize,
}
#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct CallerRow {
    pub caller: String,
    pub callee: String,
    pub line_no: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct ImportRow {
    pub module_path: String,
    pub line_no: u32,
}
#[derive(Debug, Clone)]
pub struct SymbolLocationRow {
    pub path: String,
    pub name: String,
    pub language: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct FileIdentity {
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub content_hash: String,
}
pub struct UpsertFileInput<'a> {
    pub rel_path: &'a str,
    pub language: Option<&'a str>,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub content_hash: &'a str,
    pub lines: &'a [(u32, String)],
    pub eol: &'a str,
    pub symbols: &'a [SymbolRow],
    pub callers: &'a [CallerRow],
    pub imports: &'a [ImportRow],
    pub pattern_nodes: &'a [PatternNode],
    /// Extraction hit the depth budget; rows for this file are incomplete
    /// and the cached pattern lane must not serve them as authoritative.
    pub depth_truncated: bool,
    pub semantic_chunks: &'a [crate::semantic_chunk::SemanticChunkInput],
    pub embed_semantic: bool,
}
pub struct RefreshLinesInput<'a> {
    pub file_id: i64,
    pub language: Option<&'a str>,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub content_hash: &'a str,
    pub lines: &'a [(u32, String)],
    pub eol: &'a str,
    pub rel_path: &'a str,
}
