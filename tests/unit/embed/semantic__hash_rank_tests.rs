use super::{hash_feature, SemanticLocalEmbedding, SEMANTIC_DIM};

#[test]
fn hash_feature_is_not_period_32() {
    let mut vec = vec![0.0_f32; SEMANTIC_DIM];
    hash_feature("tok:example_feature", &mut vec, 1.0);
    // Period-32 tiling would force sign(vec[i]) == sign(vec[i+32]) for all i.
    let mismatches = (0..32)
        .filter(|&i| vec[i].signum() != vec[i + 32].signum() || vec[i] != vec[i + 32])
        .count();
    assert!(
        mismatches > 0,
        "expected independent dims; period-32 tiling still present"
    );
    // Across a few blocks, not all identical
    let block0: Vec<_> = vec[0..32].to_vec();
    let block1: Vec<_> = vec[32..64].to_vec();
    let block2: Vec<_> = vec[64..96].to_vec();
    assert_ne!(block0, block1);
    assert_ne!(block1, block2);
}

#[test]
fn embed_text_has_full_dim() {
    let emb = SemanticLocalEmbedding.embed_text("refresh_token authentication");
    assert_eq!(emb.len(), SEMANTIC_DIM);
    assert!(emb.iter().any(|x| *x != 0.0));
}
