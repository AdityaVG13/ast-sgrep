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

pub(crate) fn read_semantic_chunk_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ast_sgrep_embed::SemanticChunkRow> {
    let file: String = row.get(0)?;
    let line_start: u32 = row.get(1)?;
    let line_end: u32 = row.get(2)?;
    let symbol: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
    let excerpt: String = row.get(4)?;
    let vector: Vec<u8> = row.get(5)?;
    Ok((
        file,
        line_start,
        line_end,
        symbol,
        excerpt,
        ast_sgrep_embed::embed_from_bytes(&vector).unwrap_or_default(),
    ))
}

pub(crate) fn read_legacy_embedding_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ast_sgrep_embed::SemanticChunkRow> {
    let file: String = row.get(0)?;
    let line_no: u32 = row.get(1)?;
    let content: String = row.get(2)?;
    let symbol: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
    let vector: Vec<u8> = row.get(4)?;
    Ok((
        file,
        line_no,
        line_no,
        symbol,
        content,
        ast_sgrep_embed::embed_from_bytes(&vector).unwrap_or_default(),
    ))
}
