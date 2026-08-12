use super::*;
fn chunk(vector: Vec<f32>) -> SemanticChunkRow {
    (String::new(), 0, 0, String::new(), String::new(), vector)
}
#[test]
fn semantic_backend_identity_includes_layout_and_dimension() {
    assert_eq!(
        configured_backend_model_id(EmbedBackendKind::Semantic, 256).as_deref(),
        Some("semantic:hashed-v2:256")
    );
    assert!(configured_backend_model_id(EmbedBackendKind::Neural, 256)
        .unwrap()
        .starts_with("neural:"));
}

#[test]
fn chunk_ranking_is_invariant_to_vector_magnitude() {
    let chunks = vec![chunk(vec![10.0, 1.0]), chunk(vec![1.0, 0.0])];
    let ranked = rank_chunk_indices_by_vector(&[1.0, 0.0], &chunks, 2);
    assert_eq!(
        ranked.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![1, 0]
    );
}
