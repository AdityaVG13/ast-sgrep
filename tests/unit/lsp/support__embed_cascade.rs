use super::*;

fn settings(cloud: Option<bool>, ollama: Option<bool>, semantic: Option<bool>) -> AsgrepSettings {
    AsgrepSettings {
        cloud_embed: cloud,
        ollama_embed: ollama,
        semantic_only: semantic,
        ..AsgrepSettings::default()
    }
}

fn exclusive_search(settings: &AsgrepSettings) -> SearchOptions {
    let mut opts = SearchOptions::default();
    opts.use_cloud_embed = false;
    opts.use_ollama_embed = false;
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
fn search_options_collapses_cloud_over_ollama_and_semantic() {
    let opts = exclusive_search(&settings(Some(true), Some(true), Some(true)));
    assert_eq!(opts.embed_backend(), EmbedBackend::Cloud);
    assert!(opts.use_cloud_embed);
    assert!(!opts.use_ollama_embed);
    assert!(!opts.use_semantic_only);
}

#[test]
fn search_options_collapses_ollama_over_semantic() {
    let opts = exclusive_search(&settings(Some(false), Some(true), Some(true)));
    assert_eq!(opts.embed_backend(), EmbedBackend::Ollama);
    assert!(!opts.use_cloud_embed);
    assert!(opts.use_ollama_embed);
    assert!(!opts.use_semantic_only);
}

#[test]
fn search_options_string_backend_then_bool_overlay_prefers_cloud() {
    let settings = AsgrepSettings {
        embed_backend: Some("ollama".into()),
        cloud_embed: Some(true),
        ..AsgrepSettings::default()
    };
    let opts = exclusive_search(&settings);
    assert_eq!(opts.embed_backend(), EmbedBackend::Cloud);
}

#[test]
fn search_options_cloud_string_is_not_overwritten_by_semantic_only() {
    let settings = AsgrepSettings {
        embed_backend: Some("cloud".into()),
        semantic_only: Some(true),
        ..AsgrepSettings::default()
    };
    let opts = exclusive_search(&settings);
    assert_eq!(opts.embed_backend(), EmbedBackend::Cloud);
}

#[test]
fn index_options_use_the_same_exclusive_cascade() {
    let opts = exclusive_index(&settings(Some(true), Some(true), Some(true)));
    assert_eq!(opts.embed_backend, EmbedBackend::Cloud);
}
