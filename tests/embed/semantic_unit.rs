use ast_sgrep_embed::text::*;

// The hashed-embedder identity/timing tests that used to live here
// (alloc_free_hash_matches_format_concat_identity,
// embed_text_short_query_timing) tested `SemanticLocalEmbedding`, deleted
// in Phase 2C when the static engine became the only backend. Their
// subject no longer exists; static-engine identity is pinned in
// tests/embed/static_engine.rs. What remains below tests the concept
// utilities that survived the rename into `text.rs`.

#[test]
fn eviction_expands_to_prune() {
    let expanded = expand_concepts("eviction");
    for token in ["prune", "cache", "stale"] {
        assert!(expanded.contains(token), "missing {token} in {expanded:?}");
    }
}

#[test]
fn redaction_expands_to_scrub() {
    let expanded = expand_concepts("redaction");
    for token in ["scrub", "secret", "sanitize"] {
        assert!(expanded.contains(token), "missing {token} in {expanded:?}");
    }
}

#[test]
fn hybrid_expands_to_search() {
    let expanded = expand_concepts("how does hybrid search work");
    assert!(expanded.contains("cascade") || expanded.contains("lexical"));
}

#[test]
fn follow_up_expands_to_planner() {
    let expanded = expand_concepts("derive the next command to run from the best result");
    assert!(
        expanded.contains("planner") || expanded.contains("follow_up"),
        "{expanded}"
    );
}

#[test]
fn remember_embeddings_expands_to_cache() {
    let expanded = expand_concepts("remember query embeddings between searches");
    assert!(expanded.contains("cache"), "{expanded}");
}

#[test]
fn combine_channels_expands_to_conjunction_not_rrf() {
    let query = "combine two search channels in a single query";
    let expanded = expand_concepts(query);
    let token_vec = tokenize(&expanded);
    let tokens: std::collections::HashSet<&str> = token_vec.iter().map(String::as_str).collect();
    if !tokens.contains("conjunction") {
        panic!(
            "\n\n===== DEAD PROGRAM: NO CONJUNCTION EXPANSION =====\n\
                 Query: {query:?}\n\
                 Expanded: {expanded:?}\n\
                 Tokens: {tokens:?}\n\
                 Mutant: drop the combine→conjunction group.\n\
                 ===== END AUTOPSY =====\n"
        );
    }
    if tokens.contains("rrf") || tokens.contains("fusion") {
        panic!(
            "\n\n===== DEAD PROGRAM: CHANNELS MAP TO RRF =====\n\
                 Query: {query:?}\n\
                 Expanded: {expanded:?}\n\
                 Tokens: {tokens:?}\n\
                 'channel'/'channels' must not expand to fusion/rrf. That steals \
                 the two-channel AND query for ranking.\n\
                 Mutant: restore reciprocal/channel/channels/evidence → rrf/fusion.\n\
                 ===== END AUTOPSY =====\n"
        );
    }
}

#[test]
fn reciprocal_rank_fusion_still_expands_to_rrf() {
    let query = "reciprocal rank fusion across evidence channels";
    let expanded = expand_concepts(query);
    if !expanded.contains("rrf") && !expanded.contains("fusion") {
        panic!(
            "\n\n===== DEAD PROGRAM: RRF QUERY LOST FUSION =====\n\
                 Query: {query:?}\n\
                 Expanded: {expanded:?}\n\
                 Splitting conjunction must leave rank-fusion expansion intact.\n\
                 ===== END AUTOPSY =====\n"
        );
    }
}

#[test]
fn throttle_expands_to_rate_limit() {
    let expanded = expand_concepts("throttle inbound clients");
    for token in ["rate", "limit", "quota"] {
        assert!(expanded.contains(token), "missing {token} in {expanded:?}");
    }
}

#[test]
fn debounce_expands_to_coalesce() {
    let expanded = expand_concepts("debounce noisy updates");
    for token in ["coalesce", "watch", "events"] {
        assert!(expanded.contains(token), "missing {token} in {expanded:?}");
    }
}

#[test]
fn retry_expands_to_backoff() {
    let expanded = expand_concepts("retry after transient failure");
    for token in ["backoff", "attempt", "transient"] {
        assert!(expanded.contains(token), "missing {token} in {expanded:?}");
    }
}

#[test]
fn split_ident_keeps_acronym_runs() {
    assert_eq!(
        split_ident("HTTPStatusCode"),
        vec!["httpstatus".to_string(), "code".to_string()]
    );
    assert_eq!(
        split_ident("refreshToken"),
        vec!["refresh".to_string(), "token".to_string()]
    );
}
