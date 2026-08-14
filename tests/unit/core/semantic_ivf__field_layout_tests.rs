use super::{compute_ann_fingerprint, fingerprint, SEMANTIC_IVF_FIELD_LAYOUT};

#[test]
fn field_layout_mismatch_changes_ann_fingerprint() {
    let base = fingerprint(
        3,
        9,
        8,
        Some("semantic"),
        1,
        SEMANTIC_IVF_FIELD_LAYOUT,
        None,
    );
    let other = fingerprint(
        3,
        9,
        8,
        Some("semantic"),
        1,
        SEMANTIC_IVF_FIELD_LAYOUT + 1,
        None,
    );
    assert_ne!(
        base, other,
        "a later multi-field layout must not match a concatenated sidecar"
    );
    assert_eq!(
        base,
        compute_ann_fingerprint(3, 9, 8, Some("semantic"), 1),
        "public fingerprint must hash the current field layout"
    );
}
