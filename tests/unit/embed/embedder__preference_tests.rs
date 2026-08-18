use super::*;

#[test]
fn neural_preference_is_neural_only() {
    let kinds = chain_kinds(EmbedPreference::Neural);
    assert_eq!(kinds, vec![EmbedBackendKind::Neural]);
    assert!(!kinds.contains(&EmbedBackendKind::Semantic));
}

#[test]
fn auto_never_includes_hashed_in_the_try_chain() {
    let kinds = chain_kinds(EmbedPreference::Auto);
    assert!(
        kinds.is_empty() || kinds == vec![EmbedBackendKind::Neural],
        "Auto is neural-if-configured else empty hashed fallback, got {kinds:?}"
    );
    assert!(!kinds.contains(&EmbedBackendKind::Semantic));
}

#[test]
fn semantic_preference_skips_the_try_chain() {
    assert!(chain_kinds(EmbedPreference::Semantic).is_empty());
}
