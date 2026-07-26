use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::{finish_response, HitKind, HitSignal, SpanHitInput};
use ast_sgrep_core::{SearchHit, SearchOptions};

fn hit(kind: HitKind, file: &str, score: f64) -> SearchHit {
    SearchHit::span(SpanHitInput {
        kind,
        file: file.to_string(),
        line_start: 1,
        line_end: 1,
        score,
        excerpt: file.to_string(),
        symbol: Some(file.to_string()),
        language: Some("rust".to_string()),
    })
}

#[test]
fn fusion_preserves_signal_tiers_and_computes_margins_within_each_tier() {
    let parsed = ParsedQuery::literal("needle");
    let options = SearchOptions {
        limit: 16,
        use_embed: false,
        ..SearchOptions::default()
    };
    let mut spoofed_semantic = hit(HitKind::Embed, "semantic-high", 99.0);
    spoofed_semantic.signal = HitSignal::Exact;
    let response = finish_response(
        &parsed,
        &options,
        vec![
            hit(HitKind::Asgrep, "exact-high", 1.0),
            hit(HitKind::Asgrep, "exact-low", 0.75),
            hit(HitKind::Pattern, "structural-high", 5.0),
            hit(HitKind::Pattern, "structural-low", 3.0),
            spoofed_semantic,
            hit(HitKind::Embed, "semantic-low", 98.5),
        ],
        true,
    );

    let find = |file: &str| response.hits.iter().find(|hit| hit.file == file).unwrap();
    assert_eq!(find("exact-high").signal, HitSignal::Exact);
    assert_eq!(find("exact-high").margin, 0.25);
    assert_eq!(find("exact-low").margin, 0.0);
    assert_eq!(find("structural-high").signal, HitSignal::Structural);
    assert_eq!(find("structural-high").margin, 2.0);
    assert_eq!(find("structural-low").margin, 0.0);
    assert_eq!(find("semantic-high").signal, HitSignal::Semantic);
    assert_eq!(find("semantic-high").margin, 0.5);
    assert_eq!(find("semantic-low").margin, 0.0);

    let json = serde_json::to_value(&response).unwrap();
    for hit in json["hits"].as_array().unwrap() {
        assert!(matches!(
            hit["signal"].as_str(),
            Some("exact" | "structural" | "semantic")
        ));
        assert!(hit["score"].is_number());
        assert!(hit["margin"].is_number());
    }
}

#[test]
fn every_hit_kind_has_a_stable_signal_tier() {
    assert_eq!(HitKind::Asgrep.signal(), HitSignal::Exact);
    assert_eq!(HitKind::Embed.signal(), HitSignal::Semantic);
    for kind in [
        HitKind::Def,
        HitKind::Caller,
        HitKind::Graph,
        HitKind::Anchor,
        HitKind::Import,
        HitKind::Pattern,
    ] {
        assert_eq!(kind.signal(), HitSignal::Structural, "{kind:?}");
    }
}

#[test]
fn legacy_and_spoofed_json_decode_to_kind_derived_signal() {
    let legacy = serde_json::json!({
        "kind": "embed",
        "file": "src/lib.rs",
        "line_start": 1,
        "line_end": 2,
        "score": 0.9,
        "excerpt": "semantic body"
    });
    let decoded: SearchHit = serde_json::from_value(legacy).unwrap();
    assert_eq!(decoded.signal, HitSignal::Semantic);
    assert_eq!(decoded.margin, 0.0);

    let spoofed = serde_json::json!({
        "kind": "embed",
        "signal": "exact",
        "margin": -4.0,
        "file": "src/lib.rs",
        "line_start": 1,
        "line_end": 2,
        "score": 0.9,
        "excerpt": "semantic body"
    });
    let decoded: SearchHit = serde_json::from_value(spoofed).unwrap();
    assert_eq!(decoded.signal, HitSignal::Semantic);
    assert_eq!(decoded.margin, 0.0);
}

#[test]
fn extreme_finite_scores_saturate_to_a_finite_margin() {
    let parsed = ParsedQuery::literal("needle");
    let options = SearchOptions {
        limit: 8,
        use_embed: false,
        ..SearchOptions::default()
    };
    let response = finish_response(
        &parsed,
        &options,
        vec![
            hit(HitKind::Asgrep, "maximum", f64::MAX),
            hit(HitKind::Asgrep, "minimum", -f64::MAX),
        ],
        true,
    );
    let maximum = response
        .hits
        .iter()
        .find(|hit| hit.file == "maximum")
        .unwrap();
    assert_eq!(maximum.margin, f64::MAX);
    assert!(response
        .hits
        .iter()
        .all(|hit| hit.margin.is_finite() && hit.margin >= 0.0));
}

#[test]
fn tied_scores_have_zero_margin_instead_of_false_confidence() {
    let parsed = ParsedQuery::literal("needle");
    let options = SearchOptions {
        limit: 8,
        use_embed: false,
        ..SearchOptions::default()
    };
    let response = finish_response(
        &parsed,
        &options,
        vec![
            hit(HitKind::Embed, "semantic-a", 0.8),
            hit(HitKind::Embed, "semantic-b", 0.8),
            hit(HitKind::Embed, "semantic-c", 0.2),
        ],
        true,
    );
    assert!(response.hits.iter().all(|hit| hit.margin == 0.0));
}
