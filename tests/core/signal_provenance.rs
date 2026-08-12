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
    assert_eq!(decoded.contributors, vec![HitKind::Embed]);
    assert_eq!(decoded.margin, 0.0);

    let spoofed = serde_json::json!({
        "kind": "embed",
        "signal": "exact",
        "contributors": ["asgrep", "def"],
        "margin": -4.0,
        "file": "src/lib.rs",
        "line_start": 1,
        "line_end": 2,
        "score": 0.9,
        "excerpt": "semantic body"
    });
    let decoded: SearchHit = serde_json::from_value(spoofed).unwrap();
    assert_eq!(decoded.signal, HitSignal::Semantic);
    assert_eq!(decoded.contributors, vec![HitKind::Embed]);
    assert_eq!(decoded.margin, 0.0);
}
