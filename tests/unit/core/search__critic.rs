use super::*;
use crate::intent::QueryIntent;
use crate::query::ParsedQuery;
use crate::search::types::{HitKind, SearchHit};

fn hit(kind: HitKind, file: &str, lines: (u32, u32), score: f64) -> SearchHit {
    SearchHit {
        kind,
        file: file.into(),
        line_start: lines.0,
        line_end: lines.1,
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

fn with_symbol(mut hit: SearchHit, symbol: &str) -> SearchHit {
    hit.symbol = Some(symbol.into());
    hit
}

fn with_contributors(mut hit: SearchHit, contributors: &[HitKind]) -> SearchHit {
    hit.contributors = contributors.to_vec();
    hit
}

#[test]
fn unrelated_structural_hit_does_not_delete_embed_hit_for_symbol_queries() {
    let parsed = ParsedQuery::parse("auth_refresh");
    // Embed hit in a file with no other evidence; a structural hit elsewhere
    // proves the structural stage was not empty.
    let mut hits = vec![
        with_symbol(
            hit(HitKind::Def, "src/auth.rs", (10, 20), 0.9),
            "auth_refresh",
        ),
        with_symbol(
            hit(HitKind::Embed, "styles/site.css", (1, 5), 0.8),
            "refresh_css",
        ),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    assert_eq!(hits.len(), 2);
    let embed = hits.iter().find(|hit| hit.kind == HitKind::Embed).unwrap();
    assert!(embed.critic.contains(&CriticNote::SemanticUncorroborated));
}

#[test]
fn embed_hit_corroborated_by_overlapping_span_survives() {
    let parsed = ParsedQuery::parse("auth_refresh");
    let mut hits = vec![
        with_symbol(
            hit(HitKind::Def, "src/auth.rs", (10, 20), 0.9),
            "auth_refresh",
        ),
        with_symbol(
            hit(HitKind::Embed, "src/auth.rs", (12, 18), 0.5),
            "auth_refresh",
        ),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    assert_eq!(hits.len(), 2);
}

#[test]
fn embed_hit_corroborated_by_symbol_match_survives() {
    let parsed = ParsedQuery::parse("auth_refresh");
    // Non-overlapping spans, but a caller edge names the same parent symbol.
    let mut caller = hit(HitKind::Caller, "src/session.rs", (7, 7), 0.6);
    caller.callee = Some("auth_refresh".into());
    let mut hits = vec![
        caller,
        with_symbol(
            hit(HitKind::Embed, "src/session.rs", (100, 120), 0.5),
            "auth_refresh",
        ),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    assert_eq!(hits.len(), 2);
}

#[test]
fn conceptual_query_with_empty_structural_keeps_embed_hits_labeled() {
    let parsed = ParsedQuery::parse("where do we renew expired sessions");
    let mut hits = vec![
        with_symbol(
            hit(HitKind::Embed, "src/auth.rs", (10, 20), 0.9),
            "auth_refresh",
        ),
        hit(HitKind::Asgrep, "src/other.rs", (1, 1), 0.2),
    ];
    apply_critic(&parsed, QueryIntent::Conceptual, &mut hits);
    assert_eq!(hits.len(), 2);
    let embed = hits.iter().find(|h| h.kind == HitKind::Embed).unwrap();
    assert!(embed.critic.contains(&CriticNote::SemanticUncorroborated));
}

#[test]
fn conceptual_query_with_unrelated_structural_evidence_keeps_embed_labeled() {
    let parsed = ParsedQuery::parse("where do we renew expired sessions");
    let mut hits = vec![
        with_symbol(
            hit(HitKind::Def, "src/auth.rs", (10, 20), 0.9),
            "renew_session",
        ),
        with_symbol(
            hit(HitKind::Embed, "styles/site.css", (1, 5), 0.8),
            "refresh_css",
        ),
    ];
    apply_critic(&parsed, QueryIntent::Conceptual, &mut hits);
    assert_eq!(hits.len(), 2);
    let embed = hits.iter().find(|hit| hit.kind == HitKind::Embed).unwrap();
    assert!(embed.critic.contains(&CriticNote::SemanticUncorroborated));
}

#[test]
fn structural_plus_semantic_agreement_boosts_score() {
    let parsed = ParsedQuery::parse("auth_refresh");
    let base = 0.5;
    let mut hits = vec![
        with_contributors(
            with_symbol(
                hit(HitKind::Def, "src/auth.rs", (10, 20), base),
                "auth_refresh",
            ),
            &[HitKind::Def, HitKind::Embed],
        ),
        with_symbol(
            hit(HitKind::Def, "src/other.rs", (1, 5), base),
            "auth_refresh",
        ),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    let agreed = &hits[0];
    let lone = &hits[1];
    assert!(agreed.critic.contains(&CriticNote::ChannelAgreement));
    assert!((agreed.score - base * AGREEMENT_BOOST).abs() < 1e-12);
    assert!((lone.score - base).abs() < 1e-12);
}

#[test]
fn def_usage_and_semantic_full_agreement_boosts_more() {
    let parsed = ParsedQuery::parse("auth_refresh");
    let base = 0.5;
    let mut hits = vec![with_contributors(
        with_symbol(
            hit(HitKind::Def, "src/auth.rs", (10, 20), base),
            "auth_refresh",
        ),
        &[HitKind::Def, HitKind::Caller, HitKind::Embed],
    )];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    assert!(hits[0].critic.contains(&CriticNote::FullAgreement));
    assert!((hits[0].score - base * FULL_AGREEMENT_BOOST).abs() < 1e-12);
}

#[test]
fn fragment_symbol_of_query_identifier_is_penalized() {
    // Query names auth_refresh; a bare `refresh` symbol (the CSS collision)
    // is penalized while the full identifier is not.
    let parsed = ParsedQuery::parse("auth_refresh");
    let base = 0.5;
    let mut hits = vec![
        with_symbol(
            hit(HitKind::Def, "src/auth.rs", (10, 20), base),
            "auth_refresh",
        ),
        with_symbol(
            hit(HitKind::Def, "styles/site.css", (3, 3), base),
            "refresh",
        ),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    let full = hits.iter().find(|h| h.file == "src/auth.rs").unwrap();
    let fragment = hits.iter().find(|h| h.file == "styles/site.css").unwrap();
    assert!(full.critic.is_empty());
    assert!(fragment.critic.contains(&CriticNote::IdentifierCollision));
    assert!((full.score - base).abs() < 1e-12);
    assert!((fragment.score - base * COLLISION_PENALTY).abs() < 1e-12);
}

#[test]
fn fragment_symbol_whose_excerpt_shows_full_identifier_is_not_penalized() {
    let parsed = ParsedQuery::parse("auth_refresh");
    let base = 0.5;
    let mut fragment = with_symbol(hit(HitKind::Def, "src/wrap.rs", (3, 5), base), "refresh");
    fragment.excerpt = "fn refresh() { auth_refresh() }".into();
    let mut hits = vec![fragment];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    assert!(hits[0].critic.is_empty());
    assert!((hits[0].score - base).abs() < 1e-12);
}

#[test]
fn critic_notes_render_in_hit_why() {
    let parsed = ParsedQuery::parse("auth_refresh");
    let mut hits = vec![with_contributors(
        with_symbol(
            hit(HitKind::Def, "src/auth.rs", (10, 20), 0.5),
            "auth_refresh",
        ),
        &[HitKind::Def, HitKind::Embed],
    )];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
    let why = crate::search::hit_why(&hits[0]);
    assert!(
        why.iter().any(|w| w == "critic:channel_agreement"),
        "{why:?}"
    );
}

#[test]
fn empty_shortlist_is_a_no_op() {
    let parsed = ParsedQuery::parse("anything");
    let mut hits: Vec<SearchHit> = Vec::new();
    apply_critic(&parsed, QueryIntent::Conceptual, &mut hits);
    assert!(hits.is_empty());
}
