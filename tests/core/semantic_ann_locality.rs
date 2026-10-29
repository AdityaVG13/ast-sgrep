use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
#[test]
fn probed_members_are_returned_in_flat_vector_order() {
    let mut bytes = Vec::new();
    push_u32(&mut bytes, 2);
    for value in [1.0, 0.0, 0.0, 1.0] {
        push_f32(&mut bytes, value);
    }
    push_u32(&mut bytes, 2);
    push_u32(&mut bytes, 2);
    push_u32(&mut bytes, 3);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, 2);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 2);
    let index = SemanticAnnIndex::read_clusters_bounded(&bytes, 2, 2, 4).unwrap();
    assert_eq!(
        index.candidate_indices(&[1.0, 0.0], Some(2)),
        vec![0, 1, 2, 3]
    );
}

/// Default probe depth on n>10_000 corpora is a member budget (~4k
/// candidates), not a flat probe count: admission recall saturated at
/// ~4k members on the 19k-chunk CoSQA fixture, while flat 32 flooded
/// near-duplicate corpora (date-fns rank-1 -> >100). 100 clusters x 105
/// members = 10_500 members: budget = 4096/105 = 39 probes, so the walk
/// must reach past cluster 9 (a flat-8 cap stops at cluster 7) without
/// taking the <=10k 90% path (which reaches cluster 89).
#[test]
fn large_corpus_default_probes_follow_member_budget() {
    const CLUSTERS: usize = 100;
    const MEMBERS_PER: usize = 105;
    let mut bytes = Vec::new();
    push_u32(&mut bytes, CLUSTERS as u32);
    // Centroids on one semicircle: cos(angle) is then strictly decreasing
    // in i, so the score order is deterministically 0, 1, 2, ... with no
    // mirror near-ties from the far side of the circle.
    for i in 0..CLUSTERS {
        let theta = std::f32::consts::PI * (i as f32 + 0.5) / CLUSTERS as f32;
        push_f32(&mut bytes, theta.cos());
        push_f32(&mut bytes, theta.sin());
    }
    // The blob repeats k after the centroids as a parse check.
    push_u32(&mut bytes, CLUSTERS as u32);
    for i in 0..CLUSTERS {
        push_u32(&mut bytes, MEMBERS_PER as u32);
        for m in 0..MEMBERS_PER {
            push_u32(&mut bytes, (i * MEMBERS_PER + m) as u32);
        }
    }
    let index =
        SemanticAnnIndex::read_clusters_bounded(&bytes, CLUSTERS, 2, CLUSTERS * MEMBERS_PER)
            .unwrap();
    let got = index.candidate_indices(&[1.0, 0.0], None);
    let last = *got.last().expect("members");
    assert!(
        last >= MEMBERS_PER * 9,
        "flat-8 cap reached only cluster {}: budget must probe past cluster 9",
        last / MEMBERS_PER
    );
    assert!(
        last < MEMBERS_PER * 45,
        "90% path reached cluster {}: budget must stay well under it",
        last / MEMBERS_PER
    );
}
