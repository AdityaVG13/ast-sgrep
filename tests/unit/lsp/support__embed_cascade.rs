use super::*;

fn settings(neural: Option<bool>, semantic: Option<bool>) -> AsgrepSettings {
    AsgrepSettings {
        neural_embed: neural,
        semantic_only: semantic,
        ..AsgrepSettings::default()
    }
}

fn exclusive_search(settings: &AsgrepSettings) -> SearchOptions {
    let mut opts = SearchOptions::default();
    opts.use_neural_embed = false;
    opts.use_semantic_only = false;
    settings.apply_to_search_options(&mut opts);
    opts
}

fn exclusive_index(settings: &AsgrepSettings) -> IndexOptions {
    let mut opts = IndexOptions {
        embed_backend: EmbedBackend::Auto,
        ..IndexOptions::default()
    };
    settings.apply_to_index_options(&mut opts);
    opts
}

#[test]
fn search_options_collapses_neural_over_semantic() {
    let opts = exclusive_search(&settings(Some(true), Some(true)));
    assert_eq!(opts.embed_backend(), EmbedBackend::Neural);
    assert!(opts.use_neural_embed);
    assert!(!opts.use_semantic_only);
}

#[test]
fn search_options_semantic_only_is_exclusive() {
    let opts = exclusive_search(&settings(Some(false), Some(true)));
    assert_eq!(opts.embed_backend(), EmbedBackend::Semantic);
    assert!(!opts.use_neural_embed);
    assert!(opts.use_semantic_only);
}

#[test]
fn search_options_string_backend_then_bool_overlay_prefers_neural() {
    let settings = AsgrepSettings {
        embed_backend: Some("semantic".into()),
        neural_embed: Some(true),
        ..AsgrepSettings::default()
    };
    let opts = exclusive_search(&settings);
    assert_eq!(opts.embed_backend(), EmbedBackend::Neural);
}

#[test]
fn search_options_neural_string_is_not_overwritten_by_semantic_only() {
    let settings = AsgrepSettings {
        embed_backend: Some("neural".into()),
        semantic_only: Some(true),
        ..AsgrepSettings::default()
    };
    let opts = exclusive_search(&settings);
    assert_eq!(opts.embed_backend(), EmbedBackend::Neural);
}

#[test]
fn index_options_use_the_same_exclusive_cascade() {
    let opts = exclusive_index(&settings(Some(true), Some(true)));
    assert_eq!(opts.embed_backend, EmbedBackend::Neural);
}
