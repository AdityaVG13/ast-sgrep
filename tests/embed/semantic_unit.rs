use ast_sgrep_embed::semantic::*;

fn hash_feature_old(feature: &str, vec: &mut [f32], weight: f32) {
    let mut hasher = blake3::Hasher::new();
    hasher.update(feature.as_bytes());
    let mut bytes = vec![0u8; vec.len()];
    hasher.finalize_xof().fill(&mut bytes);
    for (slot, &b) in vec.iter_mut().zip(bytes.iter()) {
        *slot += if b & 1 == 0 { weight } else { -weight };
    }
}

fn embed_text_old(text: &str) -> Vec<f32> {
    let expanded = expand_concepts(text);
    let mut vec = vec![0.0_f32; SEMANTIC_DIM];
    for token in tokenize(&expanded) {
        hash_feature_old(&format!("tok:{token}"), &mut vec, 1.0);
    }
    let compact: String = expanded
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    if compact.len() >= 3 {
        for window in compact.as_bytes().windows(3) {
            hash_feature_old(
                &format!("tri:{}", String::from_utf8_lossy(window)),
                &mut vec,
                0.35,
            );
        }
    }
    normalize(&mut vec);
    vec
}

#[test]
fn alloc_free_hash_matches_format_concat_identity() {
    let embedder = SemanticLocalEmbedding;
    for q in [
        "credential renewal",
        "sanitize user input",
        "FooBar_baz",
        "a",
        "ab",
        "abc",
    ] {
        let fresh = embedder.embed_text(q);
        let old = embed_text_old(q);
        assert_eq!(fresh, old, "identity drift on {q:?}");
    }
}

#[test]
fn embed_text_short_query_timing() {
    let embedder = SemanticLocalEmbedding;
    let q = "credential renewal variant 42";
    for _ in 0..20 {
        let _ = embedder.embed_text(q);
    }
    let start = std::time::Instant::now();
    const N: u32 = 200;
    for _ in 0..N {
        let _ = embedder.embed_text(q);
    }
    let us = start.elapsed().as_secs_f64() * 1.0e6 / f64::from(N);
    eprintln!("embed_text mean {us:.1} us over {N} runs of {q:?}");
}

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
