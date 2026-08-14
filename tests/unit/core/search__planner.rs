use super::*;
use crate::search::critic::CriticNote;
use crate::search::types::{HitKind, SearchHit, SearchResponse, SnapshotStamp};

fn hit(kind: HitKind, file: &str, score: f64) -> SearchHit {
    SearchHit {
        kind,
        file: file.into(),
        line_start: 1,
        line_end: 10,
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

fn with_margin(mut hit: SearchHit, margin: f64) -> SearchHit {
    hit.margin = margin;
    hit
}

fn response(query: &str, hits: Vec<SearchHit>) -> SearchResponse {
    SearchResponse {
        query: query.into(),
        limit: 10,
        hits,
        counts: Vec::new(),
        read_bytes_estimate: 0,
        returned_excerpt_bytes: 0,
        prevented_read_bytes: 0,
        snapshot: SnapshotStamp::default(),
        query_expansions: Vec::new(),
    }
}

#[test]
fn weak_semantic_hit_gets_defs_and_callers_follow_ups() {
    // The handoff's canonical example: a semantic hit on auth_refresh with a
    // weak margin must produce the drill-down the engine itself would run.
    let hit = with_symbol(hit(HitKind::Embed, "src/auth.rs", 0.5), "auth_refresh");
    assert_eq!(
        follow_ups_for_hit("token renewal", &hit),
        vec!["defs:auth_refresh", "callers:auth_refresh"]
    );
}

#[test]
fn settled_hit_gets_no_follow_ups() {
    // Definition + usage evidence and a decisive margin: nothing left to ask.
    let hit = with_margin(
        with_contributors(
            with_symbol(hit(HitKind::Def, "src/auth.rs", 1.0), "auth_refresh"),
            &[HitKind::Def, HitKind::Caller, HitKind::Embed],
        ),
        0.5,
    );
    assert!(follow_ups_for_hit("auth_refresh", &hit).is_empty());
}

#[test]
fn complete_evidence_with_weak_margin_confirms_via_literal() {
    let hit = with_contributors(
        with_symbol(hit(HitKind::Def, "src/auth.rs", 1.0), "auth_refresh"),
        &[HitKind::Def, HitKind::Caller],
    );
    // margin 0.0: ordering is not decisive even though evidence is complete.
    assert_eq!(
        follow_ups_for_hit("auth_refresh", &hit),
        vec!["literal:auth_refresh"]
    );
}

#[test]
fn missing_usage_asks_for_callers_only() {
    let hit = with_margin(
        with_contributors(
            with_symbol(hit(HitKind::Def, "src/auth.rs", 1.0), "auth_refresh"),
            &[HitKind::Def, HitKind::Embed],
        ),
        0.5,
    );
    assert_eq!(
        follow_ups_for_hit("auth_refresh", &hit),
        vec!["callers:auth_refresh"]
    );
}

#[test]
fn missing_definition_asks_for_defs_only() {
    let hit = with_margin(
        with_contributors(
            with_symbol(hit(HitKind::Caller, "src/auth.rs", 1.0), "auth_refresh"),
            &[HitKind::Caller, HitKind::Embed],
        ),
        0.5,
    );
    assert_eq!(
        follow_ups_for_hit("auth_refresh", &hit),
        vec!["defs:auth_refresh"]
    );
}

#[test]
fn identifier_collision_drills_the_full_query_identifier() {
    let mut fragment = with_symbol(hit(HitKind::Def, "styles/site.css", 0.4), "refresh");
    fragment.critic.push(CriticNote::IdentifierCollision);
    assert_eq!(
        follow_ups_for_hit("auth_refresh flow", &fragment),
        vec!["defs:auth_refresh", "callers:auth_refresh"]
    );
}

#[test]
fn hit_without_symbol_has_no_follow_ups() {
    let hit = hit(HitKind::Asgrep, "src/main.rs", 0.9);
    assert!(follow_ups_for_hit("main", &hit).is_empty());
}

#[test]
fn margin_decisiveness_is_relative_to_score() {
    let strong = with_margin(hit(HitKind::Def, "a.rs", 1.0), 0.2);
    assert!(margin_is_decisive(&strong));
    let weak = with_margin(hit(HitKind::Def, "a.rs", 1.0), 0.01);
    assert!(!margin_is_decisive(&weak));
    let singleton = hit(HitKind::Def, "a.rs", 1.0);
    assert!(!margin_is_decisive(&singleton));
}

#[test]
fn empty_response_suggests_semantic_then_agent_rerun() {
    let plan = plan_suggested_next(&response("session cookie", Vec::new()));
    assert_eq!(
        plan,
        vec![
            "asgrep semantic \"session cookie\"",
            "asgrep --json --format agent \"session cookie\"",
        ]
    );
}

#[test]
fn suggested_next_follows_the_actual_top_hit() {
    let top = with_symbol(hit(HitKind::Embed, "src/auth.rs", 0.5), "auth_refresh");
    let plan = plan_suggested_next(&response("token renewal", vec![top]));
    assert_eq!(
        plan,
        vec![
            "asgrep \"defs:auth_refresh\"",
            "asgrep \"callers:auth_refresh\"",
            "asgrep --json --format agent \"token renewal\"",
        ]
    );
}

#[test]
fn semantic_rerun_is_suggested_only_without_semantic_evidence() {
    let structural = with_margin(
        with_symbol(hit(HitKind::Def, "src/auth.rs", 1.0), "auth_refresh"),
        0.5,
    );
    let plan = plan_suggested_next(&response("auth_refresh", vec![structural.clone()]));
    assert!(plan.contains(&"asgrep semantic \"auth_refresh\"".to_string()));

    let semantic = with_contributors(structural, &[HitKind::Def, HitKind::Embed]);
    let plan = plan_suggested_next(&response("auth_refresh", vec![semantic]));
    assert!(!plan.iter().any(|cmd| cmd.starts_with("asgrep semantic")));
}

#[test]
fn every_suggestion_is_an_executable_asgrep_command() {
    let top = with_symbol(hit(HitKind::Embed, "src/auth.rs", 0.5), "auth_refresh");
    let plan = plan_suggested_next(&response("token renewal", vec![top]));
    assert!(!plan.is_empty());
    for cmd in &plan {
        assert!(cmd.starts_with("asgrep "), "not executable: {cmd}");
    }
}
