//! Graph tables packed into `read.v1` extra sections.
//!
//! Symbols, callers, imports, patterns, lexicon, and trigram df. Each table
//! is interned strings plus fixed-stride rows. Query time binary-searches
//! the keys; there is no RAM memo. Semantic concat lives in `bundle_semantic`.

use super::read_bundle::ReadBundle;
use super::IndexStore;
use crate::lexicon::Association;
use crate::rank::{max_normalize_idf, term_idf};
use crate::Result;
use std::collections::HashSet;

pub(crate) const SEC_DF: usize = 16;
pub(crate) const SEC_SYM: usize = 17;
pub(crate) const SEC_CALL: usize = 18;
pub(crate) const SEC_IMP: usize = 19;
pub(crate) const SEC_PAT: usize = 20;
pub(crate) const SEC_LEX: usize = 21;

const SYM_STRIDE: usize = 20;
const CALL_STRIDE: usize = 16;
const IMP_STRIDE: usize = 12;
const PAT_STRIDE: usize = 16;
const LEX_STRIDE: usize = 24;

#[derive(Default)]
pub(crate) struct GraphSections {
    pub df: Vec<u8>,
    pub symbols: Vec<u8>,
    pub callers: Vec<u8>,
    pub imports: Vec<u8>,
    pub patterns: Vec<u8>,
    pub lexicon: Vec<u8>,
}

pub(crate) struct Intern {
    bytes: Vec<u8>,
    offs: Vec<u32>,
    sorted: Vec<(String, u32)>,
}

impl Intern {
    pub(crate) fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offs: vec![0],
            sorted: Vec::new(),
        }
    }

    pub(crate) fn intern(&mut self, value: &str) -> Result<u32> {
        match self
            .sorted
            .binary_search_by(|(key, _)| key.as_str().cmp(value))
        {
            Ok(slot) => Ok(self.sorted[slot].1),
            Err(slot) => {
                let idx = (self.offs.len() - 1) as u32;
                self.bytes.extend_from_slice(value.as_bytes());
                let end = u32::try_from(self.bytes.len()).map_err(|_| {
                    crate::StoreError::Other("read.v1 intern arena exceeds u32".into())
                })?;
                self.offs.push(end);
                self.sorted.insert(slot, (value.to_owned(), idx));
                Ok(idx)
            }
        }
    }

    pub(crate) fn finish(self) -> (Vec<u8>, Vec<u32>) {
        (self.bytes, self.offs)
    }
}

fn pack_table(names: Intern, rows: Vec<u8>, stride: u32) -> Result<Vec<u8>> {
    let name_count = (names.offs.len() - 1) as u32;
    let row_count = if stride == 0 {
        0
    } else {
        (rows.len() as u32) / stride
    };
    let (mut name_bytes, name_offs) = names.finish();
    while name_bytes.len() % 4 != 0 {
        name_bytes.push(0);
    }
    let mut out = Vec::with_capacity(16 + name_bytes.len() + name_offs.len() * 4 + rows.len());
    out.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&name_count.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&stride.to_le_bytes());
    out.extend_from_slice(&name_bytes);
    out.extend_from_slice(bytemuck::cast_slice(&name_offs));
    out.extend_from_slice(&rows);
    Ok(out)
}

struct TableView<'a> {
    names: &'a [u8],
    offs_bytes: &'a [u8],
    rows: &'a [u8],
    stride: usize,
    name_count: usize,
}

fn view_table(bytes: &[u8]) -> Option<TableView<'_>> {
    if bytes.len() < 16 {
        return None;
    }
    let name_bytes_len = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
    let name_count = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    let row_count = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let stride = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    let names_at = 16;
    let offs_at = names_at + name_bytes_len;
    let offs_len = (name_count + 1).saturating_mul(4);
    let rows_at = offs_at + offs_len;
    let rows_len = row_count.saturating_mul(stride);
    if bytes.len() < rows_at + rows_len {
        return None;
    }
    Some(TableView {
        names: &bytes[names_at..offs_at],
        offs_bytes: &bytes[offs_at..offs_at + offs_len],
        rows: &bytes[rows_at..rows_at + rows_len],
        stride,
        name_count,
    })
}

fn interned<'a>(table: &TableView<'a>, idx: u32) -> &'a str {
    let i = idx as usize;
    if i + 1 > table.name_count {
        return "";
    }
    let start = read_u32(table.offs_bytes, i * 4) as usize;
    let end = read_u32(table.offs_bytes, (i + 1) * 4) as usize;
    table.names.get(start..end).and_then(|b| std::str::from_utf8(b).ok()).unwrap_or("")
}

fn cmp_lower(a: &str, b: &str) -> std::cmp::Ordering {
    a.bytes()
        .map(|b| b.to_ascii_lowercase())
        .cmp(b.bytes().map(|b| b.to_ascii_lowercase()))
}

fn eq_lower(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn read_u32(rows: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(rows[at..at + 4].try_into().expect("u32"))
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn file_index(rows: &[crate::store::IndexedLineRow]) -> Vec<(String, u32)> {
    let mut owned = rows.to_vec();
    owned.sort_by(|left, right| {
        left.0
            .as_ref()
            .cmp(right.0.as_ref())
            .then(left.1.cmp(&right.1))
    });
    let mut files = Vec::new();
    let mut current: Option<&str> = None;
    for (path, _, _, _) in &owned {
        if current != Some(path.as_ref()) {
            files.push((path.to_string(), files.len() as u32));
            current = Some(path.as_ref());
        }
    }
    files
}

pub(crate) fn file_idx(files: &[(String, u32)], path: &str) -> Option<u32> {
    files
        .binary_search_by(|(name, _)| name.as_str().cmp(path))
        .ok()
        .map(|i| files[i].1)
}

pub(crate) fn pack_from_store(
    store: &IndexStore,
    rows: &[crate::store::IndexedLineRow],
) -> Result<GraphSections> {
    let files = file_index(rows);
    Ok(GraphSections {
        df: pack_df(store)?,
        symbols: pack_symbols(store, &files)?,
        callers: pack_callers(store, &files)?,
        imports: pack_imports(store, &files)?,
        patterns: pack_patterns(store, &files)?,
        lexicon: pack_lexicon(store)?,
    })
}

fn pack_df(store: &IndexStore) -> Result<Vec<u8>> {
    let conn = store.connection();
    conn.execute("DROP TABLE IF EXISTS temp.asgrep_trigram_vocab", [])
        .map_err(|e| crate::StoreError::Other(format!("fts5vocab drop: {e}")))?;
    match conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS temp.asgrep_trigram_vocab \
         USING fts5vocab('main', 'lines_trigram', 'row')",
    ) {
        Ok(()) => {}
        Err(_) => return Ok(Vec::new()),
    }
    let mut stmt = match conn.prepare("SELECT term, doc FROM temp.asgrep_trigram_vocab") {
        Ok(stmt) => stmt,
        Err(_) => return Ok(Vec::new()),
    };
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut query = stmt
        .query([])
        .map_err(|e| crate::StoreError::Other(format!("vocab query: {e}")))?;
    while let Some(row) = query
        .next()
        .map_err(|e| crate::StoreError::Other(format!("vocab row: {e}")))?
    {
        let term: String = row.get(0)?;
        let doc: i64 = row.get(1)?;
        if term.len() != 3 || !term.is_ascii() {
            continue;
        }
        let bytes = term.as_bytes();
        let key = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        pairs.push((key, doc.max(0) as u32));
    }
    pairs.sort_unstable_by_key(|pair| pair.0);
    pairs.dedup_by_key(|pair| pair.0);
    let mut out = Vec::with_capacity(pairs.len() * 8);
    for (key, df) in pairs {
        push_u32(&mut out, key);
        push_u32(&mut out, df);
    }
    Ok(out)
}

fn pack_symbols(store: &IndexStore, files: &[(String, u32)]) -> Result<Vec<u8>> {
    let conn = store.connection();
    let mut stmt = conn.prepare(
        "SELECT f.path, s.name, s.kind, s.line_start, s.line_end \
         FROM symbols s JOIN files f ON f.id = s.file_id",
    )?;
    let mut rows: Vec<(u32, String, String, u32, u32)> = Vec::new();
    let mut query = stmt.query([])?;
    while let Some(row) = query.next()? {
        let path: String = row.get(0)?;
        let Some(file_idx) = file_idx(files, &path) else {
            continue;
        };
        rows.push((
            file_idx,
            row.get(1)?,
            row.get(2)?,
            row.get::<_, i64>(3)? as u32,
            row.get::<_, i64>(4)? as u32,
        ));
    }
    rows.sort_by(|a, b| {
        cmp_lower(&a.1, &b.1)
            .then(a.0.cmp(&b.0))
            .then(a.3.cmp(&b.3))
            .then(a.1.cmp(&b.1))
    });
    let mut names = Intern::new();
    let mut packed = Vec::with_capacity(rows.len() * SYM_STRIDE);
    for (file_idx, name, kind, line_start, line_end) in &rows {
        let name_idx = names.intern(name)?;
        let kind_idx = names.intern(kind)?;
        push_u32(&mut packed, *file_idx);
        push_u32(&mut packed, *line_start);
        push_u32(&mut packed, *line_end);
        push_u32(&mut packed, name_idx);
        push_u32(&mut packed, kind_idx);
    }
    pack_table(names, packed, SYM_STRIDE as u32)
}

fn pack_callers(store: &IndexStore, files: &[(String, u32)]) -> Result<Vec<u8>> {
    let conn = store.connection();
    let mut stmt = conn.prepare(
        "SELECT f.path, c.caller, c.callee, c.line_no \
         FROM callers c JOIN files f ON f.id = c.file_id",
    )?;
    let mut rows: Vec<(u32, String, String, u32)> = Vec::new();
    let mut query = stmt.query([])?;
    while let Some(row) = query.next()? {
        let path: String = row.get(0)?;
        let Some(file_idx) = file_idx(files, &path) else {
            continue;
        };
        rows.push((
            file_idx,
            row.get(1)?,
            row.get(2)?,
            row.get::<_, i64>(3)? as u32,
        ));
    }
    rows.sort_by(|a, b| {
        cmp_lower(&a.2, &b.2)
            .then(a.0.cmp(&b.0))
            .then(a.3.cmp(&b.3))
            .then(a.1.cmp(&b.1))
    });
    let mut names = Intern::new();
    let mut packed = Vec::with_capacity(rows.len() * CALL_STRIDE);
    for (file_idx, caller, callee, line) in &rows {
        let caller_idx = names.intern(caller)?;
        let callee_idx = names.intern(callee)?;
        push_u32(&mut packed, *file_idx);
        push_u32(&mut packed, *line);
        push_u32(&mut packed, caller_idx);
        push_u32(&mut packed, callee_idx);
    }
    pack_table(names, packed, CALL_STRIDE as u32)
}

fn pack_imports(store: &IndexStore, files: &[(String, u32)]) -> Result<Vec<u8>> {
    let conn = store.connection();
    let mut stmt = conn.prepare(
        "SELECT f.path, i.module_path, i.line_no \
         FROM imports i JOIN files f ON f.id = i.file_id",
    )?;
    let mut rows: Vec<(u32, String, u32)> = Vec::new();
    let mut query = stmt.query([])?;
    while let Some(row) = query.next()? {
        let path: String = row.get(0)?;
        let Some(file_idx) = file_idx(files, &path) else {
            continue;
        };
        rows.push((file_idx, row.get(1)?, row.get::<_, i64>(2)? as u32));
    }
    rows.sort_by(|a, b| {
        cmp_lower(&a.1, &b.1)
            .then(a.0.cmp(&b.0))
            .then(a.2.cmp(&b.2))
    });
    let mut names = Intern::new();
    let mut packed = Vec::with_capacity(rows.len() * IMP_STRIDE);
    for (file_idx, module, line) in &rows {
        let module_idx = names.intern(module)?;
        push_u32(&mut packed, *file_idx);
        push_u32(&mut packed, *line);
        push_u32(&mut packed, module_idx);
    }
    pack_table(names, packed, IMP_STRIDE as u32)
}

fn pack_patterns(store: &IndexStore, files: &[(String, u32)]) -> Result<Vec<u8>> {
    let conn = store.connection();
    let mut stmt = conn.prepare(
        "SELECT f.path, n.signature, n.line_start, n.line_end \
         FROM pattern_nodes n JOIN files f ON f.id = n.file_id",
    )?;
    let mut rows: Vec<(u32, String, u32, u32)> = Vec::new();
    let mut query = stmt.query([])?;
    while let Some(row) = query.next()? {
        let path: String = row.get(0)?;
        let Some(file_idx) = file_idx(files, &path) else {
            continue;
        };
        rows.push((
            file_idx,
            row.get(1)?,
            row.get::<_, i64>(2)? as u32,
            row.get::<_, i64>(3)? as u32,
        ));
    }
    rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)).then(a.2.cmp(&b.2)));
    let mut names = Intern::new();
    let mut packed = Vec::with_capacity(rows.len() * PAT_STRIDE);
    for (file_idx, signature, line_start, line_end) in &rows {
        let sig_idx = names.intern(signature)?;
        push_u32(&mut packed, *file_idx);
        push_u32(&mut packed, *line_start);
        push_u32(&mut packed, *line_end);
        push_u32(&mut packed, sig_idx);
    }
    pack_table(names, packed, PAT_STRIDE as u32)
}

fn pack_lexicon(store: &IndexStore) -> Result<Vec<u8>> {
    let conn = store.connection();
    let mut stmt = conn.prepare("SELECT term, related, ppmi, support FROM lexicon")?;
    let mut rows: Vec<(String, String, f64, u32)> = Vec::new();
    let mut query = stmt.query([])?;
    while let Some(row) = query.next()? {
        rows.push((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get::<_, i64>(3)? as u32,
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut names = Intern::new();
    let mut packed = Vec::with_capacity(rows.len() * LEX_STRIDE);
    for (term, related, ppmi, support) in &rows {
        let term_idx = names.intern(term)?;
        let related_idx = names.intern(related)?;
        push_u32(&mut packed, term_idx);
        push_u32(&mut packed, related_idx);
        push_u32(&mut packed, *support);
        packed.extend_from_slice(&0u32.to_le_bytes());
        packed.extend_from_slice(&ppmi.to_le_bytes());
    }
    pack_table(names, packed, LEX_STRIDE as u32)
}

fn bundle_table(bundle: &ReadBundle, section: usize) -> Option<TableView<'_>> {
    let bytes = bundle.section(section);
    if bytes.is_empty() {
        return None;
    }
    view_table(bytes)
}

fn lang_ok(bundle: &ReadBundle, file_idx: u32, lang: Option<&str>) -> bool {
    lang.is_none_or(|want| {
        bundle.language(file_idx as usize).is_some_and(|have| {
            ast_sgrep_lang::Language::normalize_id(have)
                == ast_sgrep_lang::Language::normalize_id(want)
        })
    })
}

pub fn trigram_df(bundle: &ReadBundle, tri: &str) -> Option<i64> {
    if tri.len() != 3 || !tri.is_ascii() {
        return None;
    }
    let bytes = bundle.section(SEC_DF);
    if bytes.len() < 8 || bytes.len() % 8 != 0 {
        return None;
    }
    let b = tri.as_bytes();
    let key = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
    let mut lo = 0usize;
    let mut hi = bytes.len() / 8;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let at = mid * 8;
        let found = u32::from_le_bytes(bytes[at..at + 4].try_into().ok()?);
        match found.cmp(&key) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => {
                return Some(u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as i64);
            }
        }
    }
    Some(0)
}

pub fn df_section_present(bundle: &ReadBundle) -> bool {
    bundle.section(SEC_DF).len() >= 8
}

pub fn symbols_section_present(bundle: &ReadBundle) -> bool {
    bundle_table(bundle, SEC_SYM).is_some()
}

pub fn callers_section_present(bundle: &ReadBundle) -> bool {
    bundle_table(bundle, SEC_CALL).is_some()
}

pub fn pattern_row_count(bundle: &ReadBundle) -> Option<usize> {
    let table = bundle_table(bundle, SEC_PAT)?;
    if table.stride == 0 {
        return Some(0);
    }
    Some(table.rows.len() / table.stride)
}

pub fn symbol_name_counts(bundle: &ReadBundle, name: &str) -> (Vec<(String, usize)>, usize) {
    let Some(table) = bundle_table(bundle, SEC_SYM) else {
        return (Vec::new(), 0);
    };
    let Some((start, end)) = name_row_range(&table, name, 12) else {
        return (Vec::new(), 0);
    };
    let mut files = Vec::new();
    for i in start..end {
        let Some(row) = symbol_row(bundle, &table, i) else {
            continue;
        };
        if !eq_lower(&row.2, name) {
            continue;
        }
        files.push(row.0);
    }
    let total = files.len();
    files.sort_unstable();
    (run_length_counts(files), total)
}

pub(crate) fn file_tally(by_file: &[(String, usize)], file: &str) -> usize {
    by_file
        .binary_search_by(|(path, _)| path.as_str().cmp(file))
        .map(|i| by_file[i].1)
        .unwrap_or(0)
}

fn run_length_counts(files: Vec<String>) -> Vec<(String, usize)> {
    let mut counts = Vec::new();
    let mut iter = files.into_iter();
    let Some(mut current) = iter.next() else {
        return counts;
    };
    let mut n = 1usize;
    for file in iter {
        if file == current {
            n += 1;
        } else {
            counts.push((current, n));
            current = file;
            n = 1;
        }
    }
    counts.push((current, n));
    counts
}

pub fn callee_idf_weights(bundle: &ReadBundle, terms: &[String]) -> Vec<f64> {
    let Some(table) = bundle_table(bundle, SEC_CALL) else {
        return vec![0.0; terms.len()];
    };
    if table.stride == 0 {
        return vec![0.0; terms.len()];
    }
    let total = (table.rows.len() / table.stride) as u64;
    let mut weights = Vec::with_capacity(terms.len());
    for term in terms {
        let df = name_row_range(&table, term, 12)
            .map(|(start, end)| (end - start) as u64)
            .unwrap_or(0);
        weights.push(term_idf(df, total));
    }
    max_normalize_idf(&mut weights);
    weights
}

pub type SymbolSpanRow = (String, Option<String>, String, String, u32, u32);
pub type CallerQueryRow = (String, Option<String>, String, String, u32);

fn symbol_row(bundle: &ReadBundle, table: &TableView<'_>, i: usize) -> Option<SymbolSpanRow> {
    let at = i * table.stride;
    if at + SYM_STRIDE > table.rows.len() {
        return None;
    }
    let file_idx = read_u32(table.rows, at);
    let line_start = read_u32(table.rows, at + 4);
    let line_end = read_u32(table.rows, at + 8);
    let name_idx = read_u32(table.rows, at + 12);
    let kind_idx = read_u32(table.rows, at + 16);
    Some((
        bundle.path(file_idx as usize).to_owned(),
        bundle.language(file_idx as usize).map(str::to_owned),
        interned(table, name_idx).to_owned(),
        interned(table, kind_idx).to_owned(),
        line_start,
        line_end,
    ))
}

/// Inclusive range of rows whose interned name matches `needle` case-insensitively.
/// Rows are sorted by lowercase name, so this is a binary search over row names.
fn name_row_range(table: &TableView<'_>, needle: &str, name_at: usize) -> Option<(usize, usize)> {
    let n = table.rows.len() / table.stride;
    if n == 0 {
        return None;
    }
    let mut lo = 0usize;
    let mut hi = n;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let idx = read_u32(table.rows, mid * table.stride + name_at);
        match cmp_lower(interned(table, idx), needle) {
            std::cmp::Ordering::Less => lo = mid + 1,
            _ => hi = mid,
        }
    }
    let start = lo;
    hi = n;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let idx = read_u32(table.rows, mid * table.stride + name_at);
        match cmp_lower(interned(table, idx), needle) {
            std::cmp::Ordering::Greater => hi = mid,
            _ => lo = mid + 1,
        }
    }
    if start < hi {
        Some((start, hi))
    } else {
        None
    }
}

pub fn defs_exact(
    bundle: &ReadBundle,
    name: &str,
    lang: Option<&str>,
    limit: usize,
) -> Vec<SymbolSpanRow> {
    let Some(table) = bundle_table(bundle, SEC_SYM) else {
        return Vec::new();
    };
    let Some((start, end)) = name_row_range(&table, name, 12) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in start..end {
        if out.len() >= limit {
            break;
        }
        let Some(row) = symbol_row(bundle, &table, i) else {
            continue;
        };
        if !eq_lower(&row.2, name) {
            continue;
        }
        let lang_ok = lang.is_none_or(|want| {
            row.1.as_deref().is_some_and(|have| {
                ast_sgrep_lang::Language::normalize_id(have)
                    == ast_sgrep_lang::Language::normalize_id(want)
            })
        });
        if !lang_ok {
            continue;
        }
        out.push(row);
    }
    out
}

pub fn symbol_matches_for_files(
    bundle: &ReadBundle,
    terms: &[String],
    allowed_files: &std::collections::HashSet<String>,
    lang: Option<&str>,
    limit: usize,
) -> Vec<SymbolSpanRow> {
    let Some(table) = bundle_table(bundle, SEC_SYM) else {
        return Vec::new();
    };
    let terms_lower: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let n = table.rows.len() / table.stride;
    let mut matched = Vec::new();
    for i in 0..n {
        let at = i * table.stride;
        let file_idx = read_u32(table.rows, at);
        if !lang_ok(bundle, file_idx, lang) {
            continue;
        }
        let path = bundle.path(file_idx as usize);
        if !allowed_files.contains(path) {
            continue;
        }
        let name = interned(&table, read_u32(table.rows, at + 12));
        let lower = name.to_ascii_lowercase();
        if terms_lower.iter().any(|term| lower.contains(term)) {
            if let Some(row) = symbol_row(bundle, &table, i) {
                matched.push(row);
            }
        }
    }
    let _ = limit;
    matched
}

pub fn callers_exact(
    bundle: &ReadBundle,
    callee: &str,
    lang: Option<&str>,
    limit: usize,
) -> Vec<CallerQueryRow> {
    let Some(table) = bundle_table(bundle, SEC_CALL) else {
        return Vec::new();
    };
    let Some((start, end)) = name_row_range(&table, callee, 12) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in start..end {
        if out.len() >= limit {
            break;
        }
        let at = i * table.stride;
        let file_idx = read_u32(table.rows, at);
        if !lang_ok(bundle, file_idx, lang) {
            continue;
        }
        let line = read_u32(table.rows, at + 4);
        let caller = interned(&table, read_u32(table.rows, at + 8)).to_owned();
        let got = interned(&table, read_u32(table.rows, at + 12));
        if !eq_lower(got, callee) {
            continue;
        }
        out.push((
            bundle.path(file_idx as usize).to_owned(),
            bundle.language(file_idx as usize).map(str::to_owned),
            caller,
            got.to_owned(),
            line,
        ));
    }
    out
}

/// File-restricted hybrid callers: substring on caller or callee, same
/// predicate as SQL `caller_terms_filter`. Cap at `limit` in table order
/// (callee-sorted); SQL LIMIT has no ORDER BY, so this is not rowid-identical.
pub fn callers_matches_for_files(
    bundle: &ReadBundle,
    terms: &[String],
    allowed_files: &std::collections::HashSet<String>,
    lang: Option<&str>,
    limit: usize,
) -> Vec<CallerQueryRow> {
    let Some(table) = bundle_table(bundle, SEC_CALL) else {
        return Vec::new();
    };
    if limit == 0 {
        return Vec::new();
    }
    let terms_lower: Vec<String> = terms.iter().map(|t| t.to_ascii_lowercase()).collect();
    let n = table.rows.len() / table.stride;
    let mut out = Vec::new();
    for i in 0..n {
        if out.len() >= limit {
            break;
        }
        let at = i * table.stride;
        let file_idx = read_u32(table.rows, at);
        if !lang_ok(bundle, file_idx, lang) {
            continue;
        }
        let path = bundle.path(file_idx as usize);
        if !allowed_files.contains(path) {
            continue;
        }
        let caller = interned(&table, read_u32(table.rows, at + 8));
        let callee = interned(&table, read_u32(table.rows, at + 12));
        let caller_l = caller.to_ascii_lowercase();
        let callee_l = callee.to_ascii_lowercase();
        if !terms_lower
            .iter()
            .any(|term| caller_l.contains(term) || callee_l.contains(term))
        {
            continue;
        }
        out.push((
            path.to_owned(),
            bundle.language(file_idx as usize).map(str::to_owned),
            caller.to_owned(),
            callee.to_owned(),
            read_u32(table.rows, at + 4),
        ));
    }
    out
}

pub fn imports_like(
    bundle: &ReadBundle,
    module: Option<&str>,
    lang: Option<&str>,
    limit: usize,
) -> Vec<(String, Option<String>, String, u32)> {
    let Some(table) = bundle_table(bundle, SEC_IMP) else {
        return Vec::new();
    };
    let needle = module.unwrap_or("").to_ascii_lowercase();
    let n = table.rows.len() / table.stride;
    let mut out = Vec::new();
    for i in 0..n {
        if out.len() >= limit {
            break;
        }
        let at = i * table.stride;
        let file_idx = read_u32(table.rows, at);
        if !lang_ok(bundle, file_idx, lang) {
            continue;
        }
        let line = read_u32(table.rows, at + 4);
        let path_mod = interned(&table, read_u32(table.rows, at + 8));
        if !needle.is_empty() && !path_mod.to_ascii_lowercase().contains(&needle) {
            continue;
        }
        out.push((
            bundle.path(file_idx as usize).to_owned(),
            bundle.language(file_idx as usize).map(str::to_owned),
            path_mod.to_owned(),
            line,
        ));
    }
    out
}

pub fn patterns_exact(
    bundle: &ReadBundle,
    signature: &str,
    lang: Option<&str>,
    limit: Option<usize>,
) -> Vec<(String, Option<String>, u32, u32)> {
    let Some(table) = bundle_table(bundle, SEC_PAT) else {
        return Vec::new();
    };
    let n = table.rows.len() / table.stride;
    let mut lo = 0usize;
    let mut hi = n;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let idx = read_u32(table.rows, mid * table.stride + 12);
        match interned(&table, idx).cmp(signature) {
            std::cmp::Ordering::Less => lo = mid + 1,
            _ => hi = mid,
        }
    }
    let start = lo;
    hi = n;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let idx = read_u32(table.rows, mid * table.stride + 12);
        match interned(&table, idx).cmp(signature) {
            std::cmp::Ordering::Greater => hi = mid,
            _ => lo = mid + 1,
        }
    }
    let mut out = Vec::new();
    for i in start..hi {
        if limit.is_some_and(|cap| out.len() >= cap) {
            break;
        }
        let at = i * table.stride;
        let file_idx = read_u32(table.rows, at);
        if !lang_ok(bundle, file_idx, lang) {
            continue;
        }
        if interned(&table, read_u32(table.rows, at + 12)) != signature {
            continue;
        }
        out.push((
            bundle.path(file_idx as usize).to_owned(),
            bundle.language(file_idx as usize).map(str::to_owned),
            read_u32(table.rows, at + 4),
            read_u32(table.rows, at + 8),
        ));
    }
    out
}

pub fn lexicon_row_count(bundle: &ReadBundle) -> usize {
    let Some(table) = bundle_table(bundle, SEC_LEX) else {
        return 0;
    };
    if table.stride == 0 {
        return 0;
    }
    table.rows.len() / table.stride
}

pub fn lexicon_for_terms(bundle: &ReadBundle, terms: &[String]) -> Vec<Association> {
    let Some(table) = bundle_table(bundle, SEC_LEX) else {
        return Vec::new();
    };
    let want: HashSet<&str> = terms.iter().map(String::as_str).collect();
    let n = table.rows.len() / table.stride;
    let mut out = Vec::new();
    for i in 0..n {
        let at = i * table.stride;
        let term = interned(&table, read_u32(table.rows, at));
        let related = interned(&table, read_u32(table.rows, at + 4));
        if !want.is_empty() && !want.contains(term) && !want.contains(related) {
            continue;
        }
        let support = read_u32(table.rows, at + 8);
        let ppmi = f64::from_le_bytes(table.rows[at + 16..at + 24].try_into().unwrap_or([0; 8]));
        out.push(Association {
            term: term.to_owned(),
            related: related.to_owned(),
            ppmi,
            support,
        });
    }
    out
}
