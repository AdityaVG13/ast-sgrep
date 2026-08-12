use super::{score_members, write_usize_u32, SemanticAnnIndex, DEFAULT_ANN_THRESHOLD};
use ast_sgrep_embed::{top_k_flat_similarity, top_k_similarity, MIN_SIMILARITY};

#[cfg(target_pointer_width = "64")]
#[test]
fn ivf_writer_rejects_values_larger_than_its_u32_format() {
    let mut bytes = Vec::new();
    let error = write_usize_u32(&mut bytes, u32::MAX as usize + 1)
        .expect_err("oversized IVF offsets must not truncate");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(bytes.is_empty());
}

/// IVF member scoring and flat top-k must share the ULP-stable exclusive gate.
#[test]
fn score_members_rejects_one_ulp_above_min_like_flat() {
    let min = MIN_SIMILARITY;
    let one = f32::from_bits(min.to_bits() + 1);
    let two = f32::from_bits(min.to_bits() + 2);
    // Direct top_k path (same predicate score_members now uses).
    assert!(
        top_k_similarity([(0, one)], 1, Some(min)).is_empty(),
        "1 ULP above min must be excluded"
    );
    assert_eq!(top_k_similarity([(0, two)], 1, Some(min)), vec![(0, two)]);
    // score_members on a 1-d "flat" of constant rows: cosine(query,row)=row[0]
    // when query=[1] and rows are length-1 (cosine degenerates to sign-aware
    // product / norms). Use dim=2 unit rows for true cosine.
    let dim = 2usize;
    let q = [1.0_f32, 0.0];
    let y_one = (1.0 - one * one).sqrt();
    let y_two = (1.0 - two * two).sqrt();
    let flat = vec![one, y_one, two, y_two];
    let members = vec![0usize, 1usize];
    let hits = score_members(&q, &flat, dim, 2, &members, 2);
    let idxs: Vec<usize> = hits.iter().map(|(i, _)| *i).collect();
    assert!(
        !idxs.contains(&0),
        "score_members must exclude sim=1ulp above MIN, got {hits:?}"
    );
    assert!(
        idxs.contains(&1),
        "score_members must keep sim=2ulp above MIN, got {hits:?}"
    );
    let flat_hits = top_k_flat_similarity(&q, &flat, dim, 2, Some(MIN_SIMILARITY));
    let flat_idxs: Vec<usize> = flat_hits.iter().map(|(i, _)| *i).collect();
    assert_eq!(idxs, flat_idxs);
}

#[test]
fn mid_size_ivf_uses_score_members_not_default_threshold_gate() {
    // Override-class corpus: n well below DEFAULT_ANN_THRESHOLD but IVF
    // was built (as load_or_build would under a lowered ann_threshold).
    // Query path must score via clusters (all probes) not silent brute-only.
    let dim = 4usize;
    let n = 128usize;
    assert!(n < DEFAULT_ANN_THRESHOLD);
    let mut flat = Vec::with_capacity(n * dim);
    let mut state = 0xA11_u64;
    for _ in 0..n {
        let start = flat.len();
        for _ in 0..dim {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            flat.push((((state >> 32) as u32) as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        ast_sgrep_embed::normalize_vec_in_place(&mut flat[start..start + dim]);
    }
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let q = &flat[..dim];
    assert!(
        !index.candidate_indices(q, Some(usize::MAX)).is_empty(),
        "built IVF must expose cluster members"
    );
    let ivf = index.search_flat_with_probes(&flat, dim, q, 10, Some(usize::MAX));
    let brute = top_k_flat_similarity(
        &ast_sgrep_embed::normalize_vec(q),
        &flat,
        dim,
        10,
        Some(MIN_SIMILARITY),
    );
    let ivf_idx: Vec<usize> = ivf.iter().map(|(i, _)| *i).collect();
    let brute_idx: Vec<usize> = brute.iter().map(|(i, _)| *i).collect();
    assert_eq!(
            ivf_idx, brute_idx,
            "mid-size IVF (all probes) must match flat; was query still gated on DEFAULT_ANN_THRESHOLD?"
        );
}

#[test]
fn ivf_route_above_threshold_matches_flat_on_ulp_boundary_fixture() {
    // Boundary fixture at default ANN size (production build gate).
    let dim = 2usize;
    let n = DEFAULT_ANN_THRESHOLD;
    let min = MIN_SIMILARITY;
    let one = f32::from_bits(min.to_bits() + 1);
    let two = f32::from_bits(min.to_bits() + 2);
    let y_one = (1.0 - one * one).sqrt();
    let y_two = (1.0 - two * two).sqrt();
    // Fill with low-similarity noise, then plant boundary rows at 0 and 1.
    let mut flat = Vec::with_capacity(n * dim);
    for i in 0..n {
        if i == 0 {
            flat.extend_from_slice(&[one, y_one]);
        } else if i == 1 {
            flat.extend_from_slice(&[two, y_two]);
        } else {
            // Nearly orthogonal to [1,0]
            flat.extend_from_slice(&[0.0, 1.0]);
        }
    }
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let q = [1.0_f32, 0.0];
    let ivf: Vec<usize> = index
        .search_flat_with_probes(&flat, dim, &q, 8, Some(usize::MAX))
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    let brute: Vec<usize> = top_k_flat_similarity(&q, &flat, dim, 8, Some(MIN_SIMILARITY))
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    assert!(
        !ivf.contains(&0) && !brute.contains(&0),
        "1ulp row must be gated out on both paths: ivf={ivf:?} brute={brute:?}"
    );
    assert!(
        ivf.contains(&1) && brute.contains(&1),
        "2ulp row must pass both paths: ivf={ivf:?} brute={brute:?}"
    );
    assert_eq!(ivf, brute);
}
