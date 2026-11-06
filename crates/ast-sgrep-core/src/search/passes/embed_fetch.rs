use crate::intent::QueryIntent;
use crate::search::field_weight::field_weights;
use crate::semantic_chunk::{FieldVectorMask, SemanticFieldVectors};
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::SemanticChunkRow;

pub(crate) fn rows_in_id_order_with_vectors(
    store: &IndexStore,
    ids: &[i64],
) -> Result<Option<Vec<SemanticChunkRow>>> {
    assemble_rows_in_id_order(store.semantic_chunks_by_ids(ids)?, ids)
}

pub(crate) fn rows_in_id_order(store: &IndexStore, ids: &[i64]) -> Result<Option<Vec<SemanticChunkRow>>> {
    if let Some(chunks) = chunks_from_bundle(store, ids)? {
        return Ok(Some(chunks));
    }
    let _span = crate::perf_profile::Span::start(
        "semantic_hit_fetch",
        "semantic",
        "sqlite metadata for IVF survivors (no concat blob)",
    );
    assemble_rows_in_id_order(store.semantic_chunk_hits_by_ids(ids)?, ids)
}

pub(crate) fn chunks_from_bundle(
    store: &IndexStore,
    ids: &[i64],
) -> Result<Option<Vec<SemanticChunkRow>>> {
    let Some(bundle) = store.read_bundle(None)? else {
        return Ok(None);
    };
    let Some(view) = bundle.semantic_view() else {
        return Ok(None);
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(row) = view.row_by_id(&bundle, *id) else {
            return Ok(None);
        };
        out.push(row);
    }
    Ok(Some(out))
}

pub(crate) fn take_by_ids<T>(rows: Vec<(i64, T)>, ids: &[i64]) -> Option<Vec<T>> {
    let mut owned: Vec<(i64, Option<T>)> =
        rows.into_iter().map(|(id, row)| (id, Some(row))).collect();
    owned.sort_unstable_by_key(|(id, _)| *id);
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let slot = owned.binary_search_by_key(id, |(row_id, _)| *row_id).ok()?;
        out.push(owned[slot].1.take()?);
    }
    Some(out)
}

pub(crate) fn assemble_rows_in_id_order(
    fetched: Vec<(i64, SemanticChunkRow)>,
    ids: &[i64],
) -> Result<Option<Vec<SemanticChunkRow>>> {
    Ok(take_by_ids(fetched, ids))
}

pub(crate) fn rows_and_fields_in_id_order(
    store: &IndexStore,
    ids: &[i64],
    use_field_rescoring: bool,
    intent: QueryIntent,
) -> Result<Option<(Vec<SemanticChunkRow>, Vec<SemanticFieldVectors>)>> {
    let mask = field_weights(intent).mask();
    if ids.is_empty() {
        return Ok(Some((Vec::new(), Vec::new())));
    }
    if let Some(chunks) = chunks_from_bundle(store, ids)? {
        if !use_field_rescoring || !mask.any() {
            return Ok(Some((chunks, Vec::new())));
        }
        let fields = fields_for_ids(store, ids, true, intent)?;
        return Ok(Some((chunks, fields)));
    }
    if !use_field_rescoring || !mask.any() {
        let Some(chunks) = rows_in_id_order(store, ids)? else {
            return Ok(None);
        };
        return Ok(Some((chunks, Vec::new())));
    }
    let _span = crate::perf_profile::Span::start(
        "semantic_hit_fetch",
        "semantic",
        "sqlite metadata+fields for IVF survivors",
    );
    let fetched = store.semantic_hits_and_fields_by_ids(ids, mask)?;
    let mut owned: Vec<(i64, Option<(SemanticChunkRow, SemanticFieldVectors)>)> = fetched
        .into_iter()
        .map(|(id, row, fields)| (id, Some((row, fields))))
        .collect();
    owned.sort_unstable_by_key(|(id, _)| *id);
    let mut chunks = Vec::with_capacity(ids.len());
    let mut fields = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(slot) = owned.binary_search_by_key(id, |(row_id, _)| *row_id) else {
            return Ok(None);
        };
        let Some((row, field)) = owned[slot].1.take() else {
            return Ok(None);
        };
        chunks.push(row);
        fields.push(field);
    }
    Ok(Some((chunks, fields)))
}

pub(crate) fn fields_for_ids(
    store: &IndexStore,
    ids: &[i64],
    use_field_rescoring: bool,
    intent: QueryIntent,
) -> Result<Vec<SemanticFieldVectors>> {
    let mask = field_weights(intent).mask();
    if !use_field_rescoring || !mask.any() || ids.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(fields) = fields_from_bundle(store, ids, mask)? {
        return Ok(fields);
    }
    let _span = crate::perf_profile::Span::start(
        "semantic_field_fetch",
        "semantic",
        "sqlite field vectors for IVF survivors",
    );
    let fetched = store.semantic_field_vectors_by_ids(ids, mask)?;
    Ok(take_by_ids_or_default(fetched, ids))
}

pub(crate) fn fields_from_bundle(
    store: &IndexStore,
    ids: &[i64],
    mask: FieldVectorMask,
) -> Result<Option<Vec<SemanticFieldVectors>>> {
    let Some(bundle) = store.read_bundle(None)? else {
        return Ok(None);
    };
    let Some(view) = bundle.semantic_view() else {
        return Ok(None);
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(fields) = view.fields_by_id(&bundle, *id, mask) else {
            return Ok(None);
        };
        out.push(fields);
    }
    Ok(Some(out))
}

pub(crate) fn take_by_ids_or_default<T: Default>(rows: Vec<(i64, T)>, ids: &[i64]) -> Vec<T> {
    let mut owned: Vec<(i64, Option<T>)> =
        rows.into_iter().map(|(id, row)| (id, Some(row))).collect();
    owned.sort_unstable_by_key(|(id, _)| *id);
    ids.iter()
        .map(
            |id| match owned.binary_search_by_key(id, |(row_id, _)| *row_id) {
                Ok(slot) => owned[slot].1.take().unwrap_or_default(),
                Err(_) => T::default(),
            },
        )
        .collect()
}
