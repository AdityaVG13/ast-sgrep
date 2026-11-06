use super::embed::embed_hit_limit;
use super::embed_fetch::fields_for_ids;
use super::embed_query::embed_query_vector;
use super::super::fusion::NameIntern;
use crate::intent::QueryIntent;
use crate::rank::SCORE_EMBED;
use crate::search::field_weight::{
    field_rank_union_keys, field_rank_union_mode, rescore_similarity,
    EmbedFieldScores, FieldRankUnionMode,
};
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::semantic_chunk::SemanticFieldVectors;
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::SemanticChunkRow;

pub(crate) fn field_rescore_pool(hit_limit: usize) -> usize {
    // IVF mmap already ranked candidates. Fetching 8× hit_limit (400 at the
    // default embed cap of 50) from SQLite was the unique-query floor.
    // Field-rescore among the concat top-N; N must reach past the zone
    // where a diluted joint primary hides a chunk whose name/docs field
    // matches strongly (measured: a whole-body primary at rank ~108 of
    // 18.8k whose name field ranks ~29 pure). 192 keeps the masked
    // by-ids fetch bounded while covering that rescue zone.
    (hit_limit * 3).max(192)
}


// Stable rank-fanout shape: rank context is threaded, not bundled; bundling
// would churn every rescoring call for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn embed_hits_from_pre_rank(
    store: &IndexStore,
    chunk_at: impl Fn(usize) -> SemanticChunkRow,
    ids: &[i64],
    ranked: Vec<(usize, f32)>,
    query_vec: &[f32],
    intent: QueryIntent,
    hit_limit: usize,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    let pool = if use_field_rescoring {
        field_rescore_pool(hit_limit)
    } else {
        hit_limit
    };
    let taken: Vec<(usize, f32)> = ranked.into_iter().take(pool.max(1)).collect();
    let pool_chunks: Vec<SemanticChunkRow> =
        taken.iter().map(|(idx, _)| chunk_at(*idx)).collect();
    let pool_ids: Vec<i64> = taken.iter().map(|(idx, _)| ids[*idx]).collect();
    let remapped: Vec<(usize, f32)> = taken
        .into_iter()
        .enumerate()
        .map(|(i, (_, score))| (i, score))
        .collect();
    let fields = fields_for_ids(store, &pool_ids, use_field_rescoring, intent)?;
    Ok(embed_hits_rescored(
        &pool_chunks,
        remapped,
        query_vec,
        &fields,
        intent,
        hit_limit,
    ))
}

// Stable rank-fanout shape: rank context is threaded, not bundled; bundling
// would churn every rescoring call for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn embed_hits_from_concat_rank(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    ids: &[i64],
    query_vec: &[f32],
    intent: QueryIntent,
    hit_limit: usize,
    rescore_pool: usize,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    let ranked = ast_sgrep_embed::rank_chunk_indices_by_vector(query_vec, chunks, chunks.len());
    let taken: Vec<(usize, f32)> = ranked.into_iter().take(rescore_pool.max(1)).collect();
    let pool_chunks: Vec<SemanticChunkRow> =
        taken.iter().map(|(idx, _)| chunks[*idx].clone()).collect();
    let pool_ids: Vec<i64> = taken.iter().map(|(idx, _)| ids[*idx]).collect();
    let remapped: Vec<(usize, f32)> = taken
        .into_iter()
        .enumerate()
        .map(|(i, (_, score))| (i, score))
        .collect();
    let fields = fields_for_ids(store, &pool_ids, use_field_rescoring, intent)?;
    Ok(embed_hits_rescored(
        &pool_chunks,
        remapped,
        query_vec,
        &fields,
        intent,
        hit_limit,
    ))
}

pub(crate) fn embed_hits_rescored(
    chunks: &[SemanticChunkRow],
    ranked: Vec<(usize, f32)>,
    query_vec: &[f32],
    fields: &[SemanticFieldVectors],
    intent: QueryIntent,
    hit_limit: usize,
) -> Vec<SearchHit> {
    let mut notes = vec![None; chunks.len()];
    let ranked = ranked
        .into_iter()
        .map(|(idx, primary)| {
            if let Some(field) = fields.get(idx) {
                let (score, note) = rescore_similarity(primary, query_vec, field, intent);
                if idx < notes.len() {
                    notes[idx] = note;
                }
                (idx, score)
            } else {
                (idx, primary)
            }
        })
        .collect();
    embed_similarity_hits(chunks, ranked, &notes, hit_limit)
}

pub(crate) fn embed_similarity_hits(
    chunks: &[SemanticChunkRow],
    ranked: Vec<(usize, f32)>,
    field_notes: &[Option<EmbedFieldScores>],
    hit_limit: usize,
) -> Vec<SearchHit> {
    #[derive(Debug)]
    struct ParentMatch {
        best_index: usize,
        best_similarity: f32,
        best_rank: (u32, f32),
        children: Vec<(f32, usize)>,
    }

    let union_mode = field_rank_union_mode();
    let files: Vec<&str> = chunks.iter().map(|c| c.path.as_str()).collect();
    let union_keys = if union_mode == FieldRankUnionMode::Off {
        (0..ranked.len() as u32)
            .zip(ranked.iter().map(|(_, score)| *score))
            .collect()
    } else {
        field_rank_union_keys(&ranked, &files, field_notes, union_mode)
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct ParentKey {
        file: u32,
        line_start: u32,
        line_end: u32,
        symbol: u32,
    }

    let mut intern = NameIntern::new(ranked.len());
    let mut parent_keys: Vec<ParentKey> = Vec::new();
    let mut parents: Vec<ParentMatch> = Vec::new();
    for (pos, (index, similarity)) in ranked.into_iter().enumerate() {
        let Some(chunk) = chunks.get(index) else {
            continue;
        };
        let key = union_keys
            .get(pos)
            .copied()
            .unwrap_or((pos as u32, similarity));
        let parent_key = ParentKey {
            file: intern.intern(&chunk.path),
            line_start: chunk.line_start,
            line_end: chunk.line_end,
            symbol: intern.intern(&chunk.symbol),
        };
        let parent = if let Some(slot) = parent_keys.iter().position(|seen| *seen == parent_key) {
            &mut parents[slot]
        } else {
            parent_keys.push(parent_key);
            parents.push(ParentMatch {
                best_index: index,
                best_similarity: similarity,
                best_rank: key,
                children: Vec::new(),
            });
            let slot = parents.len() - 1;
            &mut parents[slot]
        };
        if similarity > parent.best_similarity {
            parent.best_index = index;
            parent.best_similarity = similarity;
        }
        if key < parent.best_rank {
            parent.best_rank = key;
        }
        if !parent
            .children
            .iter()
            .any(|&(_, idx)| chunks.get(idx).is_some_and(|row| row.text == chunk.text))
        {
            parent.children.push((similarity, index));
        }
    }
    // Channel order = (best list rank, winning-list cosine, rescored
    // similarity), then the deterministic span tie-breaks. The carried
    // score stays the rescored similarity; only order changes.
    parents.sort_by(|left, right| {
        left.best_rank
            .0
            .cmp(&right.best_rank.0)
            .then_with(|| right.best_rank.1.total_cmp(&left.best_rank.1))
            .then_with(|| right.best_similarity.total_cmp(&left.best_similarity))
            .then_with(|| chunks[left.best_index].path.cmp(&chunks[right.best_index].path))
            .then_with(|| chunks[left.best_index].line_start.cmp(&chunks[right.best_index].line_start))
            .then_with(|| chunks[left.best_index].line_end.cmp(&chunks[right.best_index].line_end))
            .then_with(|| chunks[left.best_index].symbol.cmp(&chunks[right.best_index].symbol))
    });
    parents.truncate(hit_limit);
    parents
        .into_iter()
        .map(|mut parent| {
            parent.children.sort_by(|left, right| {
                right
                    .0
                    .total_cmp(&left.0)
                    .then_with(|| chunks[left.1].text.cmp(&chunks[right.1].text))
            });
            parent.children.truncate(3);
            let best = &chunks[parent.best_index];
            let mut hit = SearchHit::span(SpanHitInput {
                kind: HitKind::Embed,
                file: best.path.clone(),
                line_start: best.line_start,
                line_end: best.line_end,
                score: SCORE_EMBED * f64::from(parent.best_similarity),
                excerpt: parent
                    .children
                    .into_iter()
                    .map(|(_, idx)| chunks[idx].text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n...\n"),
                symbol: (!best.symbol.is_empty()).then_some(best.symbol.clone()),
                language: None,
                byte_span: None,
            });
            hit.embed_fields = field_notes.get(parent.best_index).cloned().flatten();
            hit
        })
        .collect()
}
pub(crate) fn embed_legacy_hits(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
) -> Result<Vec<SearchHit>> {
    let chunks = store.all_legacy_embeddings(options.lang_filter.as_deref())?;
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let query_vec = embed_query_vector(store, query, chunks.first().map(|c| c.embedding.len()))?;
    Ok(embed_similarity_hits(
        &chunks,
        ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &chunks, chunks.len()),
        &[],
        embed_hit_limit(options),
    ))
}
