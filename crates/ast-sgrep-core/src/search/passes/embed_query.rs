use crate::store::IndexStore;
use crate::Result;

pub(crate) fn embed_store_meta(store: &IndexStore) -> Result<(Option<String>, Option<String>)> {
    Ok((
        store.get_meta("embed_backend")?,
        store.get_meta("embed_model")?,
    ))
}

pub(crate) fn embed_query_vector(
    store: &IndexStore,
    query: &str,
    stored_dim: Option<usize>,
) -> Result<Vec<f32>> {
    let (stored_backend, stored_model) = embed_store_meta(store)?;
    embed_query_vector_from_meta(
        query,
        stored_dim,
        stored_backend.as_deref(),
        stored_model.as_deref(),
    )
}

pub(crate) fn embed_query_vector_from_meta(
    query: &str,
    stored_dim: Option<usize>,
    stored_backend: Option<&str>,
    stored_model: Option<&str>,
) -> Result<Vec<f32>> {
    let t_encode = crate::env_flag::profile_clock();
    let vector = embed_query_vector_from_meta_inner(query, stored_dim, stored_backend, stored_model)?;
    crate::env_flag::emit_profile_stage_since("search_embed_encode", t_encode);
    Ok(vector)
}

pub(crate) fn embed_query_vector_from_meta_inner(
    query: &str,
    stored_dim: Option<usize>,
    stored_backend: Option<&str>,
    stored_model: Option<&str>,
) -> Result<Vec<f32>> {
    if let Some(msg) = crate::env_flag::retired_embed_request() {
        return Err(crate::StoreError::Other(msg));
    }
    let dim = stored_dim.unwrap_or(ast_sgrep_embed::STATIC_DIM);
    // Never serve vectors embedded by a retired engine or an older model:
    // `asgrep index` performs the migration rewrite. The unversioned
    // "semantic" layout gets the same refusal (only a full rewrite may
    // promote it).
    if let Some(backend_name) = stored_backend {
        let active = crate::store::active_model_id();
        if backend_name != "static" || stored_model != Some(active) {
            return Err(crate::StoreError::Other(format!(
                "index was embedded with model {:?} (backend {backend_name:?}); active embedding model is {active:?} — run `asgrep index` to migrate (one-time full re-embed)",
                stored_model.unwrap_or("unknown")
            )));
        }
    }
    if dim != ast_sgrep_embed::STATIC_DIM {
        return Err(crate::StoreError::Other(format!(
            "stored embedding dimension {dim} does not match active dimension {}; run `asgrep index` to migrate",
            ast_sgrep_embed::STATIC_DIM
        )));
    }
    let _span = crate::perf_profile::Span::start(
        "embed_query",
        "semantic",
        "static query embed",
    );
    let vector = ast_sgrep_embed::embed_text(query);
    qvec_dump_if_set(query, &vector);
    Ok(vector)
}

/// Offline-analysis hook (default off): append the query embedding as one
/// JSON line per process, mirroring the ASGREP_FUSION_DUMP contract
/// (best-effort, errors swallowed). One CLI run = one query = one line, so
/// line order joins to the driving harness's query order exactly.
pub(crate) fn qvec_dump_if_set(query: &str, vector: &[f32]) {
    use std::io::Write;
    let Ok(path) = std::env::var("ASGREP_QVEC_DUMP") else {
        return;
    };
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let body = serde_json::json!({"q": query, "v": vector});
    let _ = writeln!(f, "{body}");
}
