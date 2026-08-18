use super::*;
use crate::search::dedup_hits;
use crate::search::field_weight::EmbedFieldScores;

fn hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
    SearchHit {
        kind,
        file: file.into(),
        line_start: line,
        line_end: line,
        symbol: None,
        caller: None,
        callee: None,
        language: None,
        score,
        signal: kind.signal(),
        contributors: vec![kind],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: String::new(),
    }
}

#[test]
fn confidence_uses_strongest_contributor_not_display_signal() {
    // Higher-scoring Embed wins kind/score; lower-scoring Asgrep still contributes
    // exact evidence. After margins rewrite display signal to Semantic, confidence
    // must keep Exact base + one agreement step (0.75 + 0.08).
    let mut merged = dedup_hits(vec![
        hit(HitKind::Embed, "a.rs", 1, 0.9),
        hit(HitKind::Asgrep, "a.rs", 1, 0.4),
    ]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].kind, HitKind::Embed);
    assert!(merged[0].contributors.contains(&HitKind::Asgrep));
    assert!(merged[0].contributors.contains(&HitKind::Embed));

    assign_signal_margins(&mut merged);
    assert_eq!(merged[0].signal, HitSignal::Semantic);
    // Re-assign as finish_response does after margins (pass5).
    assign_hit_confidence(&mut merged);
    let expected = 0.75 + 0.08;
    assert!(
        (merged[0].confidence - expected).abs() < 1e-12,
        "confidence={} expected {expected}",
        merged[0].confidence
    );
}

#[test]
fn semantic_only_confidence_is_nonzero_without_dedup() {
    // search_semantic uses dedup=false; confidence must still be populated.
    let mut hits = vec![hit(HitKind::Embed, "sem.rs", 3, 2.5)];
    assign_signal_margins(&mut hits);
    assign_hit_confidence(&mut hits);
    assert!((hits[0].confidence - 0.35).abs() < 1e-12);
    assert!(hits[0].confidence > 0.0);
}

#[test]
fn evidence_merge_preserves_semantic_field_scores() {
    let exact = hit(HitKind::Def, "a.rs", 1, 1.0);
    let mut semantic = hit(HitKind::Embed, "a.rs", 1, 0.5);
    semantic.embed_fields = Some(EmbedFieldScores {
        name: Some(0.8),
        docs: None,
        body: Some(0.4),
        graph: None,
        tests_examples: None,
    });
    let expected = semantic.embed_fields.clone();

    let merged = dedup_hits(vec![exact, semantic]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].embed_fields, expected);
}

#[test]
fn empty_hits_confidence_assign_is_noop() {
    let mut hits: Vec<SearchHit> = vec![];
    assign_hit_confidence(&mut hits);
    assert!(hits.is_empty());
}

#[test]
fn search_hit_json_round_trip_preserves_confidence() {
    // d2a1.8: custom Deserialize used SearchHitWire without confidence, so
    // round-trip always forced 0.0 even when finish_response had assigned it.
    let mut original = hit(HitKind::Asgrep, "lib.rs", 10, 1.0);
    original.confidence = 0.83;
    original.excerpt = "fn foo() {}".into();
    original.symbol = Some("foo".into());

    let json = serde_json::to_string(&original).expect("serialize");
    assert!(
        json.contains("\"confidence\""),
        "serialized JSON must emit confidence: {json}"
    );
    let back: SearchHit = serde_json::from_str(&json).expect("deserialize");
    assert!(
        (back.confidence - 0.83).abs() < 1e-12,
        "round-trip confidence={} expected 0.83",
        back.confidence
    );
    assert_eq!(back.file, "lib.rs");
    assert_eq!(back.kind, HitKind::Asgrep);
    assert_eq!(back.symbol.as_deref(), Some("foo"));
}

#[test]
fn search_hit_json_missing_confidence_defaults_zero() {
    let json = r#"{
            "kind": "embed",
            "file": "a.rs",
            "line_start": 1,
            "line_end": 1,
            "score": 0.5,
            "excerpt": "x"
        }"#;
    let hit: SearchHit = serde_json::from_str(json).expect("deserialize without confidence");
    assert_eq!(hit.confidence, 0.0);
    assert_eq!(hit.kind, HitKind::Embed);
}

#[test]
fn constructed_and_deserialized_excerpts_are_utf8_safely_bounded() {
    let oversized = "🦀".repeat(crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES);
    let hit = SearchHit::span(SpanHitInput {
        kind: HitKind::Asgrep,
        file: "large.rs".into(),
        line_start: 1,
        line_end: 1,
        score: 1.0,
        excerpt: oversized.clone(),
        symbol: None,
        language: Some("rust".into()),
    });
    assert!(hit.excerpt.len() <= crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES);
    assert!(hit.excerpt.ends_with("\n…"));

    let wire = serde_json::json!({
        "kind": "asgrep",
        "file": "large.rs",
        "line_start": 1,
        "line_end": 1,
        "score": 1.0,
        "excerpt": oversized,
    });
    let decoded: SearchHit = serde_json::from_value(wire).expect("bounded hit");
    assert!(decoded.excerpt.len() <= crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES);
    assert!(decoded.excerpt.ends_with("\n…"));

    let mut externally_mutated = hit;
    externally_mutated.excerpt = "🦀".repeat(crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES);
    let encoded = serde_json::to_value(externally_mutated).expect("bounded serialization");
    let excerpt = encoded["excerpt"].as_str().expect("serialized excerpt");
    assert!(excerpt.len() <= crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES);
    assert!(excerpt.ends_with("\n…"));
}

#[test]
fn embed_backend_roundtrips_through_use_star_flags() {
    use crate::EmbedBackend;
    let mut options = SearchOptions::default();
    for backend in [
        EmbedBackend::Auto,
        EmbedBackend::Neural,
        EmbedBackend::Semantic,
    ] {
        options.set_embed_backend(backend);
        assert_eq!(options.embed_backend(), backend);
        assert_eq!(options.embed_preference(), backend.to_preference());
        let (neural, semantic) = backend.to_flags();
        assert_eq!(options.use_neural_embed, neural);
        assert_eq!(options.use_semantic_only, semantic);
    }
}

#[test]
fn embed_backend_from_flags_prefers_neural_over_semantic() {
    let options = SearchOptions {
        use_neural_embed: true,
        use_semantic_only: true,
        ..SearchOptions::default()
    };
    assert_eq!(options.embed_backend(), crate::EmbedBackend::Neural);
}
