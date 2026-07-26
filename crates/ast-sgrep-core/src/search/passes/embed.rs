use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::semantic_ann::rank_chunk_indices_flat;
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::{embed_query, SemanticChunkRow};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
const EMBED_HIT_LIMIT: usize = 50;
pub struct EmbedContext {
    pub chunks: Arc<Vec<SemanticChunkRow>>,
    pub flat_vectors: Arc<Vec<f32>>,
}
pub fn embed_pass_lazy_ivf(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Option<Vec<SearchHit>>> {
    if parsed.terms.is_empty() || !options.use_embed {
        return Ok(Some(Vec::new()));
    }
    if options.lang_filter.is_some() {
        return Ok(None);
    }
    let stats = store.semantic_chunk_stats(None)?;
    if !crate::semantic_ann::should_use_ann(stats.count, options.ann_threshold) || stats.dim == 0 {
        return Ok(None);
    }
    let backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    let fingerprint = crate::semantic_ivf::compute_ann_fingerprint(
        stats.count,
        stats.max_id,
        stats.dim,
        Some(&backend),
        store.index_data_version()?,
    );
    let path = crate::semantic_ivf::semantic_ivf_path(store.db_path());
    let Some(ivf) = crate::semantic_ivf::load_semantic_ivf_index(&path, fingerprint)? else {
        return Ok(None);
    };
    if ivf.chunk_count() != stats.count || ivf.dim != stats.dim {
        return Ok(None);
    }
    let query = parsed.terms.join(" ");
    let query_vec = embed_query_vector(store, options, &query, Some(stats.dim))?;
    let candidate_indices = ivf.candidate_indices(&query_vec, options.ann_probes);
    if candidate_indices.is_empty() {
        return Ok(None);
    }
    let ids = store.semantic_chunk_ids(None)?;
    if ids.len() != stats.count {
        return Ok(None);
    }
    let candidate_ids: Vec<i64> = candidate_indices
        .iter()
        .filter_map(|&idx| ids.get(idx).copied())
        .collect();
    if candidate_ids.len() != candidate_indices.len() {
        return Ok(None);
    }
    let mut rows: HashMap<i64, SemanticChunkRow> = store
        .semantic_chunks_by_ids(&candidate_ids)?
        .into_iter()
        .collect();
    let mut chunks = Vec::with_capacity(candidate_ids.len());
    for id in candidate_ids {
        let Some(row) = rows.remove(&id) else {
            return Ok(None);
        };
        chunks.push(row);
    }
    let ranked = ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &chunks, chunks.len());
    Ok(Some(embed_similarity_hits(&chunks, ranked)))
}
pub fn embed_pass_for_files(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || !options.use_embed || allowed_files.is_empty() {
        return Ok(Vec::new());
    }
    let query = parsed.terms.join(" ");
    let mut survivors =
        store.semantic_chunks_for_files(allowed_files, options.lang_filter.as_deref())?;
    let modern_files = survivors
        .iter()
        .map(|chunk| chunk.0.clone())
        .collect::<HashSet<_>>();
    let legacy_only_files = allowed_files
        .difference(&modern_files)
        .cloned()
        .collect::<HashSet<_>>();
    survivors.extend(
        store.legacy_embeddings_for_files(&legacy_only_files, options.lang_filter.as_deref())?,
    );
    if survivors.is_empty() {
        return Ok(Vec::new());
    }
    let query_vec = embed_query_vector(
        store,
        options,
        &query,
        survivors.first().map(|chunk| chunk.5.len()),
    )?;
    let ranked =
        ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &survivors, survivors.len());
    Ok(embed_similarity_hits(&survivors, ranked))
}

pub fn embed_pass_with_context(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    ctx: Option<EmbedContext>,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || !options.use_embed {
        return Ok(Vec::new());
    }
    let query = parsed.terms.join(" ");
    let owned;
    let chunks: &[SemanticChunkRow] = match &ctx {
        Some(ctx) => &ctx.chunks,
        None => {
            owned = store.all_semantic_chunks(options.lang_filter.as_deref())?;
            &owned
        }
    };
    if chunks.is_empty() {
        return embed_legacy_hits(store, options, &query);
    }
    let flat = ctx.as_ref().map(|c| c.flat_vectors.as_slice());
    let query_vec = embed_query_vector(store, options, &query, chunks.first().map(|c| c.5.len()))?;
    let ann_threshold = if options.lang_filter.is_some() {
        Some(usize::MAX)
    } else {
        options.ann_threshold
    };
    let indices =
        rank_chunk_indices_flat(store, &query_vec, chunks, flat, chunks.len(), ann_threshold)?;
    Ok(embed_similarity_hits(chunks, indices))
}
fn embed_query_vector(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
    stored_dim: Option<usize>,
) -> Result<Vec<f32>> {
    use std::sync::{Mutex, OnceLock};
    static QCACHE: OnceLock<Mutex<HashMap<String, Vec<f32>>>> = OnceLock::new();
    let stored_backend = store.get_meta("embed_backend")?;
    let stored_model = store.get_meta("embed_model")?;
    let dim = stored_dim.unwrap_or(ast_sgrep_embed::default_semantic_dim());
    if let Some(backend_name) = stored_backend.as_deref() {
        let backend = ast_sgrep_embed::EmbedBackendKind::parse(backend_name).ok_or_else(|| {
            crate::StoreError::Other(format!("unknown stored embedding backend {backend_name:?}"))
        })?;
        let active_model = ast_sgrep_embed::configured_backend_model_id(backend, dim);
        if stored_model != active_model {
            return Err(crate::StoreError::Other(format!(
                "stored embedding model {:?} does not match active model {:?}; reindex with: asgrep reindex",
                stored_model.as_deref().unwrap_or("unknown"),
                active_model.as_deref().unwrap_or("unavailable")
            )));
        }
    }
    let cache_key = format!(
        "{}|{}|{}|{}|{:?}",
        query,
        stored_backend.as_deref().unwrap_or(""),
        stored_model.as_deref().unwrap_or(""),
        dim,
        options.embed_preference()
    );
    {
        let cache = QCACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(guard) = cache.lock() {
            if let Some(v) = guard.get(&cache_key) {
                return Ok(v.clone());
            }
        }
    }
    let vector = embed_query(
        query,
        stored_backend.as_deref(),
        dim,
        options.embed_preference(),
    )
    .map_err(crate::StoreError::Other)?
    .vector;
    if let Ok(mut guard) = QCACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        if guard.len() < 64 {
            guard.insert(cache_key, vector.clone());
        }
    }
    Ok(vector)
}
fn embed_similarity_hits(chunks: &[SemanticChunkRow], ranked: Vec<(usize, f32)>) -> Vec<SearchHit> {
    #[derive(Debug)]
    struct ParentMatch {
        best_index: usize,
        best_similarity: f32,
        children: Vec<(f32, String)>,
    }

    let mut parents = HashMap::<(String, u32, u32, String), ParentMatch>::new();
    for (index, similarity) in ranked {
        let Some((file, line_start, line_end, symbol, excerpt, _)) = chunks.get(index) else {
            continue;
        };
        let parent = parents
            .entry((file.clone(), *line_start, *line_end, symbol.clone()))
            .or_insert_with(|| ParentMatch {
                best_index: index,
                best_similarity: similarity,
                children: Vec::new(),
            });
        if similarity > parent.best_similarity {
            parent.best_index = index;
            parent.best_similarity = similarity;
        }
        if !parent.children.iter().any(|(_, child)| child == excerpt) {
            parent.children.push((similarity, excerpt.clone()));
        }
    }
    let mut parents = parents.into_values().collect::<Vec<_>>();
    parents.sort_by(|left, right| {
        right
            .best_similarity
            .total_cmp(&left.best_similarity)
            .then_with(|| chunks[left.best_index].0.cmp(&chunks[right.best_index].0))
            .then_with(|| chunks[left.best_index].1.cmp(&chunks[right.best_index].1))
            .then_with(|| chunks[left.best_index].2.cmp(&chunks[right.best_index].2))
            .then_with(|| chunks[left.best_index].3.cmp(&chunks[right.best_index].3))
    });
    parents.truncate(EMBED_HIT_LIMIT);
    parents
        .into_iter()
        .map(|mut parent| {
            parent.children.sort_by(|left, right| {
                right
                    .0
                    .total_cmp(&left.0)
                    .then_with(|| left.1.cmp(&right.1))
            });
            parent.children.truncate(3);
            let (file, line_start, line_end, symbol, _, _) = &chunks[parent.best_index];
            SearchHit::span(SpanHitInput {
                kind: HitKind::Embed,
                file: file.clone(),
                line_start: *line_start,
                line_end: *line_end,
                score: SCORE_EMBED * f64::from(parent.best_similarity),
                excerpt: parent
                    .children
                    .into_iter()
                    .map(|(_, excerpt)| excerpt)
                    .collect::<Vec<_>>()
                    .join("\n...\n"),
                symbol: (!symbol.is_empty()).then_some(symbol.clone()),
                language: None,
            })
        })
        .collect()
}
fn embed_legacy_hits(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
) -> Result<Vec<SearchHit>> {
    let chunks = store.all_legacy_embeddings(options.lang_filter.as_deref())?;
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let query_vec = embed_query_vector(store, options, query, chunks.first().map(|c| c.5.len()))?;
    Ok(embed_similarity_hits(
        &chunks,
        ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &chunks, chunks.len()),
    ))
}

#[cfg(test)]
mod cascade_tests {
    use super::{embed_pass_for_files, embed_pass_with_context, embed_similarity_hits};
    use crate::query::ParsedQuery;
    use crate::search::SearchOptions;
    use crate::semantic_chunk::SemanticChunkInput;
    use crate::store::{IndexStore, UpsertFileInput};
    use std::collections::HashSet;
    use tempfile::TempDir;

    #[test]
    fn child_scores_use_parent_max_and_return_one_parent_hit() {
        let chunks = vec![
            (
                "parent.rs".into(),
                10,
                20,
                "parent".into(),
                "weaker child".into(),
                vec![0.0],
            ),
            (
                "parent.rs".into(),
                10,
                20,
                "parent".into(),
                "best child".into(),
                vec![0.0],
            ),
            (
                "other.rs".into(),
                1,
                3,
                "other".into(),
                "other child".into(),
                vec![0.0],
            ),
        ];
        let hits = embed_similarity_hits(&chunks, vec![(0, 0.2), (2, 0.8), (1, 0.9)]);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].file, "parent.rs");
        assert_eq!((hits[0].line_start, hits[0].line_end), (10, 20));
        assert_eq!(hits[0].score, super::SCORE_EMBED * f64::from(0.9_f32));
        assert_eq!(hits[0].excerpt, "best child\n...\nweaker child");
    }

    #[test]
    fn language_filtered_semantic_search_does_not_publish_global_sidecar() {
        let temp = TempDir::new().unwrap();
        let store = IndexStore::open(temp.path(), None).unwrap();
        let lines = [(1, "fn filtered_handler() {}".to_string())];
        let chunks = [SemanticChunkInput {
            symbol_name: "filtered_handler".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            excerpt: "filtered semantic handler".into(),
            callers: Vec::new(),
            callees: Vec::new(),
            doc: String::new(),
            scope: String::new(),
        }];
        store
            .upsert_file(UpsertFileInput {
                rel_path: "filtered.rs",
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: "filtered",
                lines: &lines,
                eol: "\n",
                symbols: &[],
                callers: &[],
                imports: &[],
                pattern_nodes: &[],
                semantic_chunks: &chunks,
                embed_semantic: true,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();
        let hits = embed_pass_with_context(
            &store,
            &SearchOptions {
                root: temp.path().to_path_buf(),
                use_embed: true,
                lang_filter: Some("rust".into()),
                ann_threshold: Some(1),
                ..SearchOptions::default()
            },
            &ParsedQuery::parse("filtered semantic"),
            None,
        )
        .unwrap();
        assert!(!hits.is_empty());
        assert!(!crate::semantic_ivf::semantic_ivf_path(store.db_path()).exists());
    }

    #[test]
    fn cascade_ranks_modern_and_legacy_vectors_in_allowed_files() {
        let temp = TempDir::new().unwrap();
        let store = IndexStore::open(temp.path(), None).unwrap();
        let lines = [(1, "fn renewal_handler() {}".to_string())];
        store
            .upsert_file(UpsertFileInput {
                rel_path: "allowed.rs",
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: "legacy",
                lines: &lines,
                eol: "\n",
                symbols: &[],
                callers: &[],
                imports: &[],
                pattern_nodes: &[],
                semantic_chunks: &[],
                embed_semantic: false,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();
        let file_id = store.file_id("allowed.rs").unwrap().unwrap();
        let vector = ast_sgrep_embed::embed_query(
            "renewal handler",
            None,
            0,
            ast_sgrep_embed::EmbedPreference::Semantic,
        )
        .unwrap()
        .vector;
        store
            .connection()
            .execute(
                "INSERT INTO embeddings(file_id, line_no, vector) VALUES(?1, ?2, ?3)",
                rusqlite::params![file_id, 1, ast_sgrep_embed::embed_to_bytes(&vector)],
            )
            .unwrap();

        let modern_lines = [(1, "fn payment_renewal() {}".to_string())];
        let modern_chunks = [SemanticChunkInput {
            symbol_name: "payment_renewal".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            excerpt: "payment renewal modern handler".into(),
            callers: Vec::new(),
            callees: Vec::new(),
            doc: String::new(),
            scope: String::new(),
        }];
        store
            .upsert_file(UpsertFileInput {
                rel_path: "modern.rs",
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: "modern",
                lines: &modern_lines,
                eol: "\n",
                symbols: &[],
                callers: &[],
                imports: &[],
                pattern_nodes: &[],
                semantic_chunks: &modern_chunks,
                embed_semantic: true,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();

        let allowed = HashSet::from(["allowed.rs".to_string(), "modern.rs".to_string()]);
        let stored = store.semantic_chunks_for_files(&allowed, None).unwrap();
        assert!(stored.iter().any(|chunk| {
            chunk.0 == "modern.rs" && chunk.4 == "payment renewal modern handler"
        }));
        assert!(stored.iter().all(|chunk| !chunk.4.starts_with("symbol:")));
        let hits = embed_pass_for_files(
            &store,
            &SearchOptions {
                root: temp.path().to_path_buf(),
                use_embed: true,
                ..SearchOptions::default()
            },
            &ParsedQuery::parse("renewal handler"),
            &allowed,
        )
        .unwrap();
        let hit_files = hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(hit_files, HashSet::from(["allowed.rs", "modern.rs"]));

        store.set_meta("embed_model", "stale-model").unwrap();
        let error = embed_pass_for_files(
            &store,
            &SearchOptions {
                root: temp.path().to_path_buf(),
                use_embed: true,
                ..SearchOptions::default()
            },
            &ParsedQuery::parse("renewal handler"),
            &allowed,
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not match active model"));
    }
}
