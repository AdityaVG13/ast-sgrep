//! vh65: a location is one result carrying several channels of evidence, not
//! several near-identical results with opaque scores.
use ast_sgrep_core::search::{dedup_hits, hit_why, HitKind, HitSignal, SearchHit};

fn hit(kind: HitKind, score: f64, excerpt: &str) -> SearchHit {
    SearchHit {
        kind,
        file: "src/auth.rs".into(),
        line_start: 81,
        line_end: 109,
        symbol: Some("refresh_token".into()),
        caller: None,
        callee: None,
        language: Some("rust".into()),
        score,
        signal: kind.signal(),
        contributors: vec![kind],
        margin: 0.0,
        confidence: 0.0,
        excerpt: excerpt.into(),
    }
}

#[test]
fn one_location_found_by_three_channels_becomes_one_hit() {
    let merged = dedup_hits(vec![
        hit(HitKind::Def, 5.0, "fn refresh_token() {}"),
        hit(HitKind::Embed, 3.0, "fn refresh_token() {}"),
        hit(HitKind::Asgrep, 9.0, "fn refresh_token() {}"),
    ]);

    assert_eq!(merged.len(), 1, "same span must not survive three times");
    let hit = &merged[0];
    // Best score still wins ordering, exactly as before.
    assert_eq!(hit.score, 9.0);
    assert_eq!(hit.kind, HitKind::Asgrep);
    // Every channel is retained as evidence.
    for kind in [HitKind::Def, HitKind::Embed, HitKind::Asgrep] {
        assert!(
            hit.contributors.contains(&kind),
            "{kind:?} evidence was dropped: {:?}",
            hit.contributors
        );
    }
    // The strongest signal observed wins.
    assert_eq!(hit.signal, HitSignal::Exact);

    // The reasons are derived from the evidence, so they cannot drift from it.
    let why = hit_why(hit);
    assert!(why.contains(&"exact_symbol".to_owned()), "{why:?}");
    assert!(why.contains(&"semantic_similarity".to_owned()), "{why:?}");
    assert!(why.contains(&"exact_text".to_owned()), "{why:?}");
}

#[test]
fn confidence_is_separate_from_score_and_rises_with_agreement() {
    // A high score from one weak channel.
    let lonely = dedup_hits(vec![hit(HitKind::Embed, 99.0, "body")]);
    // A lower score confirmed by several channels.
    let corroborated = dedup_hits(vec![
        hit(HitKind::Def, 5.0, "body"),
        hit(HitKind::Embed, 4.0, "body"),
        hit(HitKind::Asgrep, 3.0, "body"),
    ]);

    assert!(
        lonely[0].score > corroborated[0].score,
        "fixture: the lonely hit must outrank on score"
    );
    assert!(
        corroborated[0].confidence > lonely[0].confidence,
        "confidence must reflect agreement, not score ({} vs {})",
        corroborated[0].confidence,
        lonely[0].confidence
    );
    assert!(
        (0.0..=0.99).contains(&corroborated[0].confidence),
        "confidence stays in range: {}",
        corroborated[0].confidence
    );
}

#[test]
fn distinct_locations_are_never_merged() {
    let mut second = hit(HitKind::Def, 4.0, "other");
    second.line_start = 200;
    second.line_end = 210;
    let mut third = hit(HitKind::Def, 4.0, "other file");
    third.file = "src/session.rs".into();

    let merged = dedup_hits(vec![hit(HitKind::Def, 5.0, "body"), second, third]);
    assert_eq!(merged.len(), 3, "different spans must stay separate");
}

#[test]
fn merge_backfills_non_identity_details_the_kept_row_lacked() {
    // symbol / caller / callee are part of the location identity, so rows that
    // differ in them are different locations by definition. `language` is
    // descriptive, so it is the field a merge can legitimately backfill.
    let mut kept = hit(HitKind::Asgrep, 9.0, "body");
    kept.language = None;
    let other = hit(HitKind::Def, 1.0, "body");

    let merged = dedup_hits(vec![kept, other]);
    assert_eq!(merged.len(), 1, "same location must merge");
    assert_eq!(
        merged[0].language.as_deref(),
        Some("rust"),
        "descriptive detail must be backfilled from the merged row"
    );
    assert_eq!(merged[0].score, 9.0, "best score still wins");
}

#[test]
fn rows_differing_in_identity_fields_stay_separate() {
    let mut other = hit(HitKind::Def, 1.0, "body");
    other.callee = Some("rotate".into());
    let merged = dedup_hits(vec![hit(HitKind::Asgrep, 9.0, "body"), other]);
    assert_eq!(
        merged.len(),
        2,
        "callee is part of identity, so these are different locations"
    );
}
