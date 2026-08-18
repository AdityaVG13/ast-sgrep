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
