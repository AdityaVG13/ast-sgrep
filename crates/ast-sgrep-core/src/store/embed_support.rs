//! Private embed-cache + structure-fingerprint helpers used by `IndexStore`.
use std::cell::Cell;
use std::path::{Component, Path};
use ast_sgrep_lang::PatternNode;
use blake3::Hasher;
use rusqlite::{params, Connection};
use super::sql::optional_row;
use super::sqlite::{CallerRow, ImportRow, SymbolLocationRow, SymbolFileRow, SymbolRow};
use crate::Result;

pub(super) const DEFAULT_EMBED_CACHE_CAP: usize = 100_000;

pub(super) struct EmbeddedChunk {
    pub text: String, pub vector_bytes: Vec<u8>, pub dim: usize,
    pub backend: ast_sgrep_embed::EmbedBackendKind,
}
#[derive(Clone)]
pub(super) struct CacheRow {
    pub vector: Vec<u8>, pub backend: ast_sgrep_embed::EmbedBackendKind, pub dim: usize,
}
#[derive(Clone)]
pub(super) struct CacheEntry {
    pub chunk_hash: String, pub model_id: String, pub backend: ast_sgrep_embed::EmbedBackendKind,
    pub dim: usize, pub vector: Vec<u8>,
}
#[derive(Clone)]
pub(super) struct CacheHit { pub chunk_hash: String, pub model_id: String }
pub(super) struct EmbeddedChunks {
    pub chunks: Vec<EmbeddedChunk>, pub cache_entries: Vec<CacheEntry>, pub cache_hits: Vec<CacheHit>,
}
impl EmbeddedChunks {
    pub fn empty() -> Self {
        Self { chunks: Vec::new(), cache_entries: Vec::new(), cache_hits: Vec::new() }
    }
}

pub(super) fn hash_text(t: &str) -> String {
    let mut h = Hasher::new(); h.update(t.as_bytes()); h.finalize().to_hex().to_string()
}
pub(super) fn embed_cache_cap() -> usize {
    std::env::var("ASGREP_EMBED_CACHE_CAP").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_EMBED_CACHE_CAP)
}
fn semantic_mid() -> String {
    format!("semantic:hashed-v1:{}", ast_sgrep_embed::default_semantic_dim())
}
pub(super) fn cache_model_id_for_pref(p: ast_sgrep_embed::EmbedPreference) -> Option<String> {
    use ast_sgrep_embed::EmbedPreference::*;
    match p {
        Semantic => Some(semantic_mid()),
        Neural => Some(format!("neural:{}", ast_sgrep_embed::neural_configured_model_id())),
        Cloud | Ollama => None,
        Auto => {
            let skip = std::env::var_os("ASGREP_EMBED_API_KEY").is_some()
                || std::env::var_os("ASGREP_OLLAMA_EMBED").is_some()
                || std::env::var_os("ASGREP_OLLAMA_URL").is_some()
                || std::env::var("ASGREP_NEURAL_EMBED").is_ok_and(|v| v == "1");
            if skip { None } else { Some(semantic_mid()) }
        }
    }
}
fn cache_model_id_for_backend(b: ast_sgrep_embed::EmbedBackendKind) -> Option<String> {
    use ast_sgrep_embed::EmbedBackendKind::*;
    match b {
        Semantic => Some(semantic_mid()),
        Neural => Some(format!("neural:{}", ast_sgrep_embed::neural_configured_model_id())),
        Cloud => ast_sgrep_embed::CloudEmbeddingConfig::from_env().map(|c| format!("cloud:{}", c.model)),
        Ollama => ast_sgrep_embed::OllamaEmbeddingConfig::from_env().map(|c| format!("ollama:{}", c.model)),
    }
}

pub(super) fn init_cache_seq(conn: &Connection, seq: &Cell<i64>) -> Result<()> {
    let max: i64 = conn.query_row("SELECT COALESCE(MAX(accessed_at), 0) FROM embed_cache", [], |r| r.get(0))?;
    seq.set(max); Ok(())
}
fn next_cache_seq(seq: &Cell<i64>) -> i64 {
    let n = seq.get().saturating_add(1); seq.set(n); n
}
fn drop_cache(conn: &Connection, h: &str, m: &str) {
    let _ = conn.execute("DELETE FROM embed_cache WHERE chunk_hash = ?1 AND model_id = ?2", params![h, m]);
}
pub(super) fn lookup_embed_cache(conn: &Connection, h: &str, m: &str) -> Result<Option<CacheRow>> {
    let raw: Option<(Vec<u8>, String, i64)> = optional_row(
        conn, "SELECT vector, backend, dim FROM embed_cache WHERE chunk_hash = ?1 AND model_id = ?2",
        &[&h, &m], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let Some((vector, backend_str, dim_i64)) = raw else { return Ok(None) };
    let Some(backend) = ast_sgrep_embed::EmbedBackendKind::parse(&backend_str) else {
        drop_cache(conn, h, m); return Ok(None);
    };
    let ok = usize::try_from(dim_i64).ok().and_then(|d| d.checked_mul(4))
        .is_some_and(|n| n > 0 && vector.len() == n);
    if !ok { drop_cache(conn, h, m); return Ok(None); }
    Ok(Some(CacheRow { vector, backend, dim: dim_i64 as usize }))
}
pub(super) fn insert_embed_cache_entries(
    conn: &Connection, seq: &Cell<i64>, entries: &[CacheEntry],
) -> Result<()> {
    if entries.is_empty() { return Ok(()); }
    let at = next_cache_seq(seq);
    let mut st = conn.prepare_cached(
        "INSERT INTO embed_cache(chunk_hash, model_id, backend, dim, vector, accessed_at)
         VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(chunk_hash, model_id) DO UPDATE SET
         vector=excluded.vector, backend=excluded.backend, dim=excluded.dim, accessed_at=excluded.accessed_at",
    )?;
    for e in entries {
        st.execute(params![&e.chunk_hash, &e.model_id, e.backend.as_meta_str(), e.dim as i64, &e.vector, at])?;
    }
    Ok(())
}
pub(super) fn touch_embed_cache_entries(
    conn: &Connection, seq: &Cell<i64>, keys: &[(String, String)],
) -> Result<()> {
    if keys.is_empty() { return Ok(()); }
    let at = next_cache_seq(seq);
    let mut st = conn.prepare_cached(
        "UPDATE embed_cache SET accessed_at = ?1 WHERE chunk_hash = ?2 AND model_id = ?3",
    )?;
    for (h, m) in keys { st.execute(params![at, h, m])?; }
    Ok(())
}
pub(super) fn evict_embed_cache(conn: &Connection, max_entries: usize) -> Result<()> {
    if max_entries == 0 { conn.execute("DELETE FROM embed_cache", [])?; return Ok(()); }
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM embed_cache", [], |r| r.get(0))?;
    let over = count.saturating_sub(max_entries as i64);
    if over <= 0 { return Ok(()); }
    conn.prepare_cached(
        "DELETE FROM embed_cache WHERE rowid IN (
           SELECT rowid FROM embed_cache ORDER BY accessed_at ASC, rowid ASC LIMIT ?1)",
    )?.execute(params![over])?;
    Ok(())
}

pub(super) fn embed_chunks(
    conn: &Connection, chunks: &[crate::semantic_chunk::SemanticChunkInput],
    do_embed: bool, backend: ast_sgrep_embed::EmbedPreference,
) -> Result<EmbeddedChunks> {
    if !do_embed || chunks.is_empty() { return Ok(EmbeddedChunks::empty()); }
    let mid = cache_model_id_for_pref(backend);
    let (chunks, cache_entries, cache_hits) = embed_parallel(conn, chunks, backend, &mid)?;
    Ok(EmbeddedChunks { chunks, cache_entries, cache_hits })
}

fn embed_parallel(
    conn: &Connection, chunks: &[crate::semantic_chunk::SemanticChunkInput],
    backend: ast_sgrep_embed::EmbedPreference, expected_mid: &Option<String>,
) -> Result<(Vec<EmbeddedChunk>, Vec<CacheEntry>, Vec<CacheHit>)> {
    let texts: Vec<String> = chunks.iter().map(crate::semantic_chunk::render_chunk_text).collect();
    let mut cached: Vec<Option<CacheRow>> = vec![None; texts.len()];
    let mut hits = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        let h = hash_text(t);
        if let Some(mid) = expected_mid {
            if let Some(row) = lookup_embed_cache(conn, &h, mid)? {
                cached[i] = Some(row);
                hits.push(CacheHit { chunk_hash: h, model_id: mid.clone() });
            }
        }
    }
    if hits.len() == texts.len() {
        let out = texts.into_iter().zip(cached).map(|(text, row)| {
            let row = row.expect("hit");
            EmbeddedChunk { text, vector_bytes: row.vector, dim: row.dim, backend: row.backend }
        }).collect();
        return Ok((out, Vec::new(), hits));
    }
    let miss_idx: Vec<usize> = (0..texts.len()).filter(|&i| cached[i].is_none()).collect();
    // One chain attempt for the whole miss batch (avoids per-chunk backend probing).
    let miss_refs: Vec<&str> = miss_idx.iter().map(|&i| texts[i].as_str()).collect();
    let miss_res = ast_sgrep_embed::embed_batch_with_chain(&miss_refs, backend);
    if miss_res.len() != miss_idx.len() {
        return Err(crate::StoreError::Other("embedding result length mismatch".into()));
    }
    let mut out = Vec::with_capacity(texts.len());
    let mut entries = Vec::with_capacity(miss_res.len());
    let mut miss_it = miss_res.into_iter();
    for (i, text) in texts.into_iter().enumerate() {
        if let Some(row) = cached[i].take() {
            out.push(EmbeddedChunk { text, vector_bytes: row.vector, dim: row.dim, backend: row.backend });
            continue;
        }
        let r = miss_it.next()
            .ok_or_else(|| crate::StoreError::Other("embedding result length mismatch".into()))?;
        let vb = ast_sgrep_embed::embed_to_bytes(&r.vector);
        let dim = r.vector.len();
        if let Some(mid) = cache_model_id_for_backend(r.backend) {
            entries.push(CacheEntry {
                chunk_hash: hash_text(&text), model_id: mid, backend: r.backend, dim, vector: vb.clone(),
            });
        }
        out.push(EmbeddedChunk { text, vector_bytes: vb, dim, backend: r.backend });
    }
    Ok((out, entries, hits))
}

pub(super) fn read_sym_loc(r: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolLocationRow> {
    Ok(SymbolLocationRow {
        path: r.get(0)?, name: r.get(1)?, language: r.get(2)?, line_start: r.get(3)?, line_end: r.get(4)?,
    })
}
pub(super) fn read_sym_file(r: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolFileRow> {
    Ok(SymbolFileRow {
        path: r.get(0)?, language: r.get(1)?, name: r.get(2)?, kind: r.get(3)?,
        line_start: r.get(4)?, line_end: r.get(5)?,
    })
}
pub(super) fn normalize_rel(path: &Path) -> String {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => { parts.pop(); }
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}

/// Graph rows + body-dependent semantic/pattern content. Equal fingerprints ⇒ lines-only upsert safe.
pub(super) fn structure_fingerprint(
    symbols: &[SymbolRow], callers: &[CallerRow], imports: &[ImportRow],
    pattern_nodes: &[PatternNode], semantic_chunks: &[crate::semantic_chunk::SemanticChunkInput],
) -> String {
    let mut h = Hasher::new();
    for s in symbols {
        h.update(s.name.as_bytes()); h.update(b"\0"); h.update(s.kind.as_bytes());
        h.update(&s.line_start.to_le_bytes()); h.update(&s.line_end.to_le_bytes());
        h.update(&s.byte_start.to_le_bytes()); h.update(&s.byte_end.to_le_bytes());
    }
    h.update(b"|c|");
    for c in callers {
        h.update(c.caller.as_bytes()); h.update(b"\0"); h.update(c.callee.as_bytes());
        h.update(&c.line_no.to_le_bytes());
        h.update(&c.byte_start.to_le_bytes()); h.update(&c.byte_end.to_le_bytes());
    }
    h.update(b"|i|");
    for i in imports { h.update(i.module_path.as_bytes()); h.update(&i.line_no.to_le_bytes()); }
    h.update(b"|p|");
    for n in pattern_nodes {
        h.update(n.signature.as_bytes());
        h.update(&n.line_start.to_le_bytes()); h.update(&n.line_end.to_le_bytes());
        h.update(n.excerpt.as_bytes()); h.update(b"\0");
    }
    h.update(b"|s|");
    for chunk in semantic_chunks {
        // Hash raw chunk fields (not expand_concepts) — same equality for structure-stable edits.
        h.update(chunk.symbol_name.as_bytes());
        h.update(&chunk.line_start.to_le_bytes()); h.update(&chunk.line_end.to_le_bytes());
        h.update(chunk.excerpt.as_bytes()); h.update(chunk.doc.as_bytes()); h.update(b"\0");
    }
    h.finalize().to_hex().to_string()
}
