use ast_sgrep_embed::{embed_from_bytes, rank_by_similarity};
#[cfg(feature = "cloud-embed")]
use ast_sgrep_embed::{rank_by_vector, CloudEmbeddingConfig};

use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::store::IndexStore;
use crate::Result;
use crate::search::types::{HitKind, SearchHit, SearchOptions};

const EMBED_SQL_LIMIT: usize = 10_000;

pub fn embed_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || !options.use_embed {
        return Ok(Vec::new());
    }

    let query = parsed.terms.join(" ");
    let conn = store.connection();
    let lang_clause = if options.lang_filter.is_some() {
        " AND f.language = ?1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT f.path, l.line_no, l.content, f.language, e.vector
         FROM embeddings e
         JOIN lines l ON l.file_id = e.file_id AND l.line_no = e.line_no
         JOIN files f ON f.id = e.file_id
         WHERE 1=1{lang_clause}
         LIMIT {EMBED_SQL_LIMIT}"
    );

    let mut lines = Vec::new();
    if let Some(ref lang) = options.lang_filter {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![lang])?;
        while let Some(row) = rows.next()? {
            push_embed_row(&mut lines, row)?;
        }
    } else {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            push_embed_row(&mut lines, row)?;
        }
    }

    if lines.is_empty() {
        return Ok(Vec::new());
    }

    let stored_dim = lines.first().map(|l| l.3.len()).unwrap_or(0);
    let embed_backend = store.get_meta("embed_backend").unwrap_or(None);
    let use_cloud = options.use_cloud_embed && embed_backend.as_deref() == Some("cloud");

    let ranked = if use_cloud {
        rank_with_cloud(&query, &lines, stored_dim)
    } else {
        rank_by_similarity(&query, &lines, 50)
    };

    Ok(ranked
        .into_iter()
        .map(|(sim, file, line_no, content)| SearchHit {
            kind: HitKind::Embed,
            file,
            line_start: line_no,
            line_end: line_no,
            symbol: None,
            caller: None,
            callee: None,
            language: None,
            score: SCORE_EMBED * f64::from(sim),
            excerpt: content,
        })
        .collect())
}

fn push_embed_row(
    lines: &mut Vec<(String, u32, String, Vec<f32>)>,
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<()> {
    let file: String = row.get(0)?;
    let line_no: u32 = row.get(1)?;
    let content: String = row.get(2)?;
    let vector: Vec<u8> = row.get(4)?;
    lines.push((file, line_no, content, embed_from_bytes(&vector)));
    Ok(())
}

fn rank_with_cloud(
    query: &str,
    lines: &[(String, u32, String, Vec<f32>)],
    stored_dim: usize,
) -> Vec<(f32, String, u32, String)> {
    #[cfg(feature = "cloud-embed")]
    {
        if let Some(config) = CloudEmbeddingConfig::from_env() {
            if let Ok(query_vec) = ast_sgrep_embed::embed_via_api(query, &config) {
                if stored_dim > 0 && stored_dim == query_vec.len() {
                    return rank_by_vector(&query_vec, lines, 50);
                }
            }
        }
    }
    rank_by_similarity(query, lines, 50)
}
