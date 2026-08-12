use super::*;
#[test]
fn cloud_preference_excludes_ollama() {
    let kinds = chain_kinds(EmbedPreference::Cloud);
    assert_eq!(kinds, vec![EmbedBackendKind::Cloud]);
    assert!(!kinds.contains(&EmbedBackendKind::Ollama));
    assert!(!kinds.contains(&EmbedBackendKind::Semantic));
}
#[test]
fn auto_may_include_cloud_and_ollama() {
    let kinds = chain_kinds(EmbedPreference::Auto);
    assert!(kinds.contains(&EmbedBackendKind::Cloud));
    assert!(kinds.contains(&EmbedBackendKind::Ollama));
}
