use ast_sgrep_core::bench_suite::measure_semantic_ivf_open_p99;
use ast_sgrep_core::semantic_ann::{SemanticAnnIndex, DEFAULT_ANN_THRESHOLD};
use ast_sgrep_core::semantic_ivf::{
    compute_ann_fingerprint, invalidate_semantic_ivf, load_semantic_ivf, load_semantic_ivf_index,
    load_semantic_ivf_unchecked, save_semantic_ivf, save_semantic_ivf_with_publication,
};
use ast_sgrep_embed::{top_k_flat_similarity, MIN_SIMILARITY};
use std::collections::HashSet;
#[test]
fn invalidating_a_missing_sidecar_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("index.db");
    invalidate_semantic_ivf(&database).unwrap();
    invalidate_semantic_ivf(&database).unwrap();
}

#[test]
fn semantic_ivf_roundtrip_and_fingerprint_gate() {
    let dim = 4usize;
    let vectors: Vec<f32> = (0..24).map(|i| i as f32 * 0.1).collect();
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let fingerprint = compute_ann_fingerprint(6, 6, dim, Some("test"), 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("semantic.ivf");
    save_semantic_ivf(&path, fingerprint, dim, &vectors, &index).unwrap();
    let loaded = load_semantic_ivf(&path, fingerprint)
        .unwrap()
        .expect("valid sidecar");
    assert_eq!(loaded.dim, dim);
    assert!(loaded.is_mapped());
    assert_eq!(loaded.vectors(), vectors);
    assert_eq!(loaded.fingerprint, fingerprint);
    let lazy = load_semantic_ivf_index(&path, fingerprint)
        .unwrap()
        .expect("valid lazy sidecar");
    assert_eq!(lazy.dim, dim);
    assert_eq!(lazy.chunk_count(), 6);
    assert_eq!(
        lazy.candidate_indices(&[0.1; 4], Some(usize::MAX))
            .into_iter()
            .collect::<HashSet<_>>(),
        (0..6).collect()
    );
    let wrong_fp = compute_ann_fingerprint(6, 5, dim, Some("test"), 0);
    assert!(load_semantic_ivf(&path, wrong_fp).unwrap().is_none());
    assert!(load_semantic_ivf_index(&path, wrong_fp).unwrap().is_none());
    let wrong_generation = compute_ann_fingerprint(6, 6, dim, Some("test"), 1);
    assert!(load_semantic_ivf(&path, wrong_generation)
        .unwrap()
        .is_none());
    let unchecked = load_semantic_ivf_unchecked(&path)
        .unwrap()
        .expect("unchecked load");
    assert!(unchecked.is_mapped());
    assert_eq!(unchecked.vectors(), vectors);
    let query = vec![0.1f32; dim];
    assert_eq!(
        index.search_flat(&vectors, dim, &query, 3),
        loaded.index.search_flat(loaded.vectors(), dim, &query, 3)
    );
}

#[test]
fn save_rejects_an_index_for_a_different_vector_population() {
    let dim = 4;
    let indexed = vec![0.5_f32; 32];
    let supplied = vec![0.5_f32; 16];
    let index = SemanticAnnIndex::build_from_flat(&indexed, dim);
    let fingerprint = compute_ann_fingerprint(4, 4, dim, Some("mismatch"), 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("semantic.ivf");
    assert!(save_semantic_ivf(&path, fingerprint, dim, &supplied, &index).is_err());
    assert!(!path.exists());
}

#[test]
fn mapped_reader_rejects_corrupt_or_truncated_frames_without_panicking() {
    let dim = 4;
    let vectors = vec![0.5_f32; 32];
    let fingerprint = compute_ann_fingerprint(8, 8, dim, Some("corruption"), 0);
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("semantic.ivf");
    save_semantic_ivf(&path, fingerprint, dim, &vectors, &index).unwrap();
    let valid = std::fs::read(&path).unwrap();

    let mut cases = Vec::new();
    let mut bad_magic = valid.clone();
    bad_magic[0] ^= 0xff;
    cases.push(("magic", bad_magic));
    let mut old_version = valid.clone();
    old_version[6..10].copy_from_slice(&1_u32.to_le_bytes());
    cases.push(("version", old_version));
    let mut bad_header = valid.clone();
    bad_header[10..12].copy_from_slice(&79_u16.to_le_bytes());
    cases.push(("header", bad_header));
    let mut zero_clusters = valid.clone();
    zero_clusters[56..60].copy_from_slice(&0_u32.to_le_bytes());
    cases.push(("clusters", zero_clusters));
    let mut reserved = valid.clone();
    reserved[76] = 1;
    cases.push(("reserved", reserved));
    let mut trailing = valid.clone();
    trailing.push(0);
    cases.push(("trailing", trailing));
    cases.push(("truncated", valid[..valid.len() - 4].to_vec()));

    for (name, bytes) in cases {
        std::fs::write(&path, bytes).unwrap();
        assert!(
            load_semantic_ivf(&path, fingerprint).unwrap().is_none(),
            "accepted corrupt {name} frame"
        );
    }
}

#[test]
fn mapped_reader_survives_atomic_sidecar_replacement() {
    let dim = 4;
    let first = vec![0.25_f32; 32];
    let second = vec![0.75_f32; 32];
    let fingerprint = compute_ann_fingerprint(8, 8, dim, Some("mapped"), 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("semantic.ivf");
    save_semantic_ivf(
        &path,
        fingerprint,
        dim,
        &first,
        &SemanticAnnIndex::build_from_flat(&first, dim),
    )
    .unwrap();
    let old = load_semantic_ivf(&path, fingerprint).unwrap().unwrap();
    let published = save_semantic_ivf_with_publication(
        &path,
        fingerprint,
        dim,
        &second,
        &SemanticAnnIndex::build_from_flat(&second, dim),
    )
    .unwrap();
    let current = load_semantic_ivf(&path, fingerprint).unwrap().unwrap();
    assert_eq!(old.vectors(), first);
    if published {
        assert_eq!(current.vectors(), second);
    } else {
        assert_eq!(current.vectors(), first);
    }
}

#[test]
fn medium_mapped_sidecar_reports_open_p99() {
    let dim = 8;
    let count = 10_000;
    let vectors = normalized_flat_vectors(count, dim, 0x0F3_0009);
    let fingerprint = compute_ann_fingerprint(count, count as i64, dim, Some("open-bench"), 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("semantic.ivf");
    save_semantic_ivf(
        &path,
        fingerprint,
        dim,
        &vectors,
        &SemanticAnnIndex::build_from_flat(&vectors, dim),
    )
    .unwrap();
    let latency = measure_semantic_ivf_open_p99(&path, fingerprint, 100).unwrap();
    eprintln!(
        "semantic_ivf mmap open samples={} fresh_inode_p99_ns={} warm_p99_ns={} sidecar_bytes={} mapped_vector_bytes={} resident_index_bytes={}",
        latency.samples,
        latency.fresh_inode_p99_ns,
        latency.warm_p99_ns,
        latency.sidecar_bytes,
        latency.mapped_vector_bytes,
        latency.resident_index_bytes
    );
    assert_eq!(latency.samples, 100);
    assert_eq!(latency.mapped_vector_bytes, count * dim * 4);
    assert!(latency.mapped_vector_bytes > latency.resident_index_bytes);
    if std::env::var("ASGREP_PERF_ASSERTS").as_deref() == Ok("1") {
        assert!(
            latency.warm_p99_ns < 1_000_000,
            "warm mmap open p99 must remain below 1ms: {latency:?}"
        );
    }
}

/// Deterministic LCG unit vectors for IVF regression (CE-003).
fn normalized_flat_vectors(count: usize, dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    let mut flat = Vec::with_capacity(count * dim);
    for _ in 0..count {
        let start = flat.len();
        for _ in 0..dim {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            flat.push((((state >> 32) as u32) as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        normalize_row_in_place(&mut flat[start..start + dim]);
    }
    flat
}
fn normalize_row_in_place(row: &mut [f32]) {
    let norm: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in row.iter_mut() {
            *x /= norm;
        }
    }
}
fn normalize_query(query: &[f32]) -> Vec<f32> {
    let mut out = query.to_vec();
    normalize_row_in_place(&mut out);
    out
}
fn brute_force_top_k_indices(
    flat: &[f32],
    dim: usize,
    query: &[f32],
    limit: usize,
) -> HashSet<usize> {
    top_k_flat_similarity(
        &normalize_query(query),
        flat,
        dim,
        limit,
        Some(MIN_SIMILARITY),
    )
    .into_iter()
    .map(|(idx, _)| idx)
    .collect()
}
/// CE-003: IVF search with all-cluster probing must return the same top-k indices as brute force.
///
/// e2hc.19(a): vector_count must exceed DEFAULT_ANN_THRESHOLD (2000) so that
/// `search_flat_with_probes` actually routes through the IVF cluster path
/// (`candidate_indices` → `score_members`) instead of the `n < threshold`
/// brute-force early return. At n=512 the test was vacuous: both arms ran
/// `brute_force_flat`, so the cluster machinery was never exercised.
#[test]
fn ivf_search_matches_brute_force_top_k_indices_ce003() {
    let dim = 32usize;
    let vector_count = 2048usize;
    assert!(
        vector_count >= DEFAULT_ANN_THRESHOLD,
        "vector_count must exceed DEFAULT_ANN_THRESHOLD so the IVF cluster path is exercised, not brute-force"
    );
    let limit = 24usize;
    let flat = normalized_flat_vectors(vector_count, dim, 0xCE_003_u64);
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    assert!(index.validate_partition(vector_count));
    for &qi in &[0usize, 17, 137, 299, 400, 511] {
        let query = &flat[qi * dim..(qi + 1) * dim];
        let brute = brute_force_top_k_indices(&flat, dim, query, limit);
        let ivf: HashSet<usize> = index
            .search_flat_with_probes(&flat, dim, query, limit, Some(usize::MAX))
            .into_iter()
            .map(|(idx, _)| idx)
            .collect();
        assert_eq!(
            ivf, brute,
            "IVF top-k index set must match brute-force top_k_flat_similarity (query chunk {qi})"
        );
    }
}
/// Adaptive IVF recall@10 must stay within the measured quality budget.
///
/// e2hc.19(a): The original 0.99 SLO was vacuous — vector_count=512 <
/// DEFAULT_ANN_THRESHOLD (2000) meant both arms hit the `n < threshold →
/// brute_force_flat` early return, so recall=1.0 by construction and the ANN
/// cluster path was never exercised.
///
/// With vector_count=2048 (> threshold), the adaptive arm probes at most 90%
/// of populated clusters. It must preserve recall@10 >= 0.99 while examining
/// no more than 95% of the exact all-cluster candidates.
#[test]
fn adaptive_ivf_recall_at_10_stays_within_quality_error_budget() {
    const RECALL_SLO: f64 = 0.99;
    let dim = 32usize;
    let vector_count = 2048usize;
    assert!(
        vector_count >= DEFAULT_ANN_THRESHOLD,
        "vector_count must exceed DEFAULT_ANN_THRESHOLD so adaptive IVF is measured, not brute-force"
    );
    let limit = 10usize;
    let flat = normalized_flat_vectors(vector_count, dim, 0x5D0_036_u64);
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let mut matches = 0usize;
    let mut expected = 0usize;
    for qi in (0..vector_count).step_by(8) {
        let query = &flat[qi * dim..(qi + 1) * dim];
        let exact: HashSet<_> = index
            .search_flat_with_probes(&flat, dim, query, limit, Some(usize::MAX))
            .into_iter()
            .map(|(idx, _)| idx)
            .collect();
        let candidates = index.candidate_indices(query, None);
        let candidate_ceiling = (vector_count * 95).div_ceil(100);
        assert!(
            candidates.len() <= candidate_ceiling,
            "adaptive probing scanned {} of {vector_count} candidates, above the 95% ceiling",
            candidates.len()
        );
        let adaptive: HashSet<_> = index
            .search_flat(&flat, dim, query, limit)
            .into_iter()
            .map(|(idx, _)| idx)
            .collect();
        matches += exact.intersection(&adaptive).count();
        expected += exact.len();
    }
    let recall = matches as f64 / expected as f64;
    let miss_rate = 1.0 - recall;
    let burn_rate = miss_rate / (1.0 - RECALL_SLO);
    eprintln!(
        "adaptive IVF recall@10={recall:.6}, miss_rate={miss_rate:.6}, burn_rate={burn_rate:.3}"
    );
    assert!(burn_rate <= 1.0 + f64::EPSILON, "adaptive IVF quality error budget exceeded: recall@10={recall:.6}, burn_rate={burn_rate:.3}");
}

#[test]
#[ignore = "release-mode ANN recall/latency tradeoff"]
fn adaptive_ivf_tradeoff_at_2048_and_10000_vectors() {
    let dim = 32usize;
    let limit = 10usize;
    for &(vector_count, seed) in &[(2_048usize, 0x5D0_036_u64), (10_000, 0x07A1_0000_u64)] {
        let flat = normalized_flat_vectors(vector_count, dim, seed);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let cluster_count = ((vector_count as f64).sqrt() as usize).clamp(16, 256);
        let query_indices = (0..64)
            .map(|index| index * (vector_count / 64))
            .collect::<Vec<_>>();
        let exact = query_indices
            .iter()
            .map(|query_index| {
                let query = &flat[query_index * dim..(query_index + 1) * dim];
                index
                    .search_flat_with_probes(&flat, dim, query, limit, Some(usize::MAX))
                    .into_iter()
                    .map(|(index, _)| index)
                    .collect::<HashSet<_>>()
            })
            .collect::<Vec<_>>();
        for percent in [50usize, 75, 90, 100] {
            let probes = (cluster_count * percent).div_ceil(100);
            let selected_probes = (percent != 90).then_some(probes);
            let started = std::time::Instant::now();
            let mut matches = 0usize;
            let mut expected = 0usize;
            let mut candidates = 0usize;
            for (slot, query_index) in query_indices.iter().enumerate() {
                let query = &flat[query_index * dim..(query_index + 1) * dim];
                candidates += index.candidate_indices(query, selected_probes).len();
                let actual = index
                    .search_flat_with_probes(&flat, dim, query, limit, selected_probes)
                    .into_iter()
                    .map(|(index, _)| index)
                    .collect::<HashSet<_>>();
                matches += exact[slot].intersection(&actual).count();
                expected += exact[slot].len();
            }
            let recall = matches as f64 / expected as f64;
            let average_us =
                started.elapsed().as_secs_f64() * 1_000_000.0 / query_indices.len() as f64;
            let candidate_fraction =
                candidates as f64 / (vector_count * query_indices.len()) as f64;
            eprintln!(
                "ivf_tradeoff n={vector_count} probes={percent}% recall_at_10={recall:.6} avg_us={average_us:.3} candidate_fraction={candidate_fraction:.6}"
            );
            if percent == 90 {
                assert!(recall >= 0.99, "n={vector_count} recall={recall}");
                assert!(candidate_fraction <= 0.95);
            }
        }
    }
}
