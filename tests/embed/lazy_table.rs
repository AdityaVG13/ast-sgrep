//! Query-path embedding must not materialize the full f32 weight table.
//!
//! `embed` (single text: the query path) decodes only the token rows it
//! pools; `embed_batch` (indexing) materializes the whole 63,457 x 256 f32
//! table once. This test pins the laziness property in a dedicated test
//! process: one test, no other embed caller can pollute the global
//! materialized flag. Vector parity between the two paths is pinned by
//! `static_engine` (dual-path bit equality).

use ast_sgrep_embed::table_is_materialized;

/// KILLS: an `embed` that materializes the f32 table (65 MB decode + RSS on
/// every one-shot query process — the rg-lane latency loss).
#[test]
fn single_embed_never_materializes_the_f32_table() {
    assert!(
        !table_is_materialized(),
        "precondition: table starts unmaterialized in a fresh process"
    );
    let embedder = ast_sgrep_embed::StaticEmbedder::new();
    let v = embedder.embed("semantic_ivf search hybrid_lexical_prefilter");
    assert!(v.iter().any(|x| *x != 0.0), "query embed must be non-zero");
    assert!(
        !table_is_materialized(),
        "embed() materialized the full f32 table — query path must decode rows lazily"
    );
    // The batch path still materializes (indexing amortizes it).
    let _ = ast_sgrep_embed::embed_texts(&["asgrep index batch path"]);
    assert!(
        table_is_materialized(),
        "embed_batch must keep the one-shot full-table materialization"
    );
}
