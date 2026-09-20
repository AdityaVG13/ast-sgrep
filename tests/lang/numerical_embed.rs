//! Embedding contract suite: embed_text/bytes/similarity + expansion strings.
//!
//! Holds embed_contract (five merged clauses) plus the expand_concepts KEEP,
//! which is the sole test of its function and cannot live in the contract.
//! Tolerance table: BIT-EXACT (`assert_eq!` / `to_bits`) for byte layout,
//! token sets, order-sensitivity inequality, zero tables, degenerate zeros,
//! delegation lanes, byte edges, expansion strings; 1e-5 inline for 256-lane
//! embedding self-dots (f32 accumulation band); cross-sim Cauchy bound 1+1e-6.

use ast_sgrep_embed::{
    dot_similarity, embed_from_bytes, embed_to_bytes, expand_concepts, tokenize,
    SemanticLocalEmbedding, SEMANTIC_DIM,
};

/// INTENT: embedding total contract — little-endian byte layout, trigram-path
/// order sensitivity with order-invariant token set, unit-norm self-dots with
/// Cauchy-Schwarz cross bound, degenerate-input totality, provider similarity
/// delegation with byte edges.
/// KILLS: endianness/byte-layout, trigram-weight-removal (order-blind-embed),
/// normalize-division-skipped, degenerate-input panic-or-NaN, nondeterminism,
/// delegation-divergence, byte-edge mutants.
/// ABSORBS: embed_byte_layout_is_little_endian_exact (N1), embed_trigram_path_is_order_sensitive_while_tokens_static (N1),
/// embed_unit_norm_and_cauchy_schwarz_table (N1), embed_text_degenerate_inputs_are_total (N2),
/// similarity_delegation_and_byte_edges (N2).
/// LINE-DROPS: none.
#[test]
fn embed_contract() {
    // ── Clause byte_layout (N1): LE layout, not just roundtrip ──
    // 1.0 = 0x3F800000 -> LE 00 00 80 3F. BIT-EXACT.
    assert_eq!(embed_to_bytes(&[1.0]), vec![0x00, 0x00, 0x80, 0x3F]);
    // -2.5 = -(1.25 * 2^1): exp 128, mantissa .25 -> 0xC0200000
    // -> LE 00 00 20 C0. BIT-EXACT.
    assert_eq!(embed_to_bytes(&[-2.5]), vec![0x00, 0x00, 0x20, 0xC0]);
    assert_eq!(embed_to_bytes(&[0.0]), vec![0x00, 0x00, 0x00, 0x00]);

    // ── Clause trigram (N1): order-sensitive embeds, static tokens ──
    // Token features (weight 1.0) are a SET: order-invariant.
    assert_eq!(tokenize("ab cd"), tokenize("cd ab"));
    assert_eq!(tokenize("ab cd"), vec!["ab", "cd"]);
    // Trigram features (weight 0.35) run over the ordered compact string:
    // "ab cd ab cd" -> compact abcdabcd (windows abc,bcd,cda,dab,abc,bcd)
    // vs "cd ab ab cd" -> compact cdababcd (windows cda,dab,aba,bab,abc,bcd).
    // Different window multisets -> different blake3 signs -> the two
    // embeddings MUST differ, isolating the trigram path. Both unit-norm
    // (1e-5 band) and strictly below self-similarity.
    let emb = SemanticLocalEmbedding;
    let a = emb.embed_text("ab cd");
    let b = emb.embed_text("cd ab");
    assert_eq!(a.len(), SEMANTIC_DIM);
    assert_ne!(a, b);
    assert!((dot_similarity(&a, &a) - 1.0).abs() < 1e-5);
    assert!((dot_similarity(&b, &b) - 1.0).abs() < 1e-5);
    assert!(dot_similarity(&a, &b) < 1.0);

    // ── Clause unit_norm (N1): unit-norm + Cauchy-Schwarz table ──
    // Hand rule: embed_text divides by the L2 norm, so every NONZERO
    // embedding is unit up to f32 rounding (1e-5 over 256 lanes), and every
    // cross-similarity obeys Cauchy-Schwarz (|s| <= 1 + rounding).
    let queries = [
        "credential renewal",
        "sanitize user input",
        "hello world",
        "ab",
        "throttle inbound",
        "a1B2",
        "HTTPStatusCode",
        "x y z", // no 2+ char tokens, but compact "xyz" feeds one trigram
    ];
    let emb = SemanticLocalEmbedding;
    let vecs: Vec<Vec<f32>> = queries.iter().map(|q| emb.embed_text(q)).collect();
    for (q, v) in queries.iter().zip(vecs.iter()) {
        assert_eq!(v.len(), SEMANTIC_DIM, "{q:?}");
        assert!(v.iter().all(|x| x.is_finite()), "{q:?}");
        assert!(v.iter().any(|x| *x != 0.0), "{q:?} must be nonzero");
        assert!(
            (dot_similarity(v, v) - 1.0).abs() < 1e-5,
            "{q:?} self-dot = {}",
            dot_similarity(v, v)
        );
    }
    for (i, a) in vecs.iter().enumerate() {
        for (j, b) in vecs.iter().enumerate() {
            if i != j {
                let s = dot_similarity(a, b);
                assert!(s.abs() <= 1.0 + 1e-6, "{i},{j}: {s}");
            }
        }
    }
    // Zero table, BIT-EXACT: single alphanumerics yield no tokens
    // (len < 2 split) and a compact shorter than one trigram window.
    assert_eq!(emb.embed_text("a"), vec![0.0; SEMANTIC_DIM]);
    assert_eq!(emb.embed_text("I"), vec![0.0; SEMANTIC_DIM]);

    // ── Clause embed_degenerate (N2): degenerate inputs are total ──
    let emb = SemanticLocalEmbedding;
    // Whitespace-only and punctuation-only yield no tokens and a compact
    // shorter than one trigram window: exact zero vectors. BIT-EXACT.
    assert_eq!(emb.embed_text("   "), vec![0.0; SEMANTIC_DIM]);
    assert_eq!(emb.embed_text("!!!"), vec![0.0; SEMANTIC_DIM]);
    // Non-ASCII alphanumerics and very long inputs stay shaped, finite,
    // and deterministic (totality: no panic, no NaN poisoning).
    for text in ["日本語", &"credential renewal ".repeat(500)] {
        let v = emb.embed_text(text);
        assert_eq!(v.len(), SEMANTIC_DIM, "{text:?} shape");
        assert!(v.iter().all(|x| x.is_finite()), "{text:?} finite");
        assert_eq!(v, emb.embed_text(text), "{text:?} deterministic");
    }

    // ── Clause similarity_delegation (N2): delegation + byte edges ──
    let emb = SemanticLocalEmbedding;
    // similarity delegates to dot, including the degenerate lanes.
    assert_eq!(emb.similarity(&[], &[]), 0.0);
    assert_eq!(emb.similarity(&[1.0], &[1.0, 2.0]), 0.0);
    assert_eq!(emb.similarity(&[f32::NAN], &[1.0]), 0.0);
    assert_eq!(emb.similarity(&[f32::INFINITY], &[1.0]), 0.0);
    // Byte edges: empty encodes to empty; aligned NaN payloads decode
    // Ok with bits preserved (never Err on aligned length). BIT-EXACT.
    assert!(embed_to_bytes(&[]).is_empty());
    let nan = embed_from_bytes(&[0xFF, 0xFF, 0xFF, 0xFF]).expect("aligned ok");
    assert_eq!(nan.len(), 1);
    assert_eq!(nan[0].to_bits(), 0xFFFF_FFFF);
    let pair =
        embed_from_bytes(&[0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x00]).expect("aligned ok");
    assert_eq!(pair, vec![1.0, 0.0]);
}

/// INTENT: exact concept-expansion strings — single-group ("combine") and
/// multi-group ("auth refresh") expansions plus the exact 16-term token set.
/// KILLS: expansion-term/sort-order mutant.
/// ABSORBS: none — KEEP standalone (sole test of expand_concepts; N1 verbatim).
#[test]
fn expand_concepts_single_and_multi_group_strings_are_exact() {
    // "combine": tokens=[combine]; only the conjunction group fires.
    // parts = {combine} U {combine,conjunction,intersect,intersection},
    // sorted (combine < conjunction: 'm'<'n'; intersect < intersection:
    // prefix rule). BIT-EXACT whole string.
    assert_eq!(
        expand_concepts("combine"),
        "combine combine conjunction intersect intersection"
    );
    // "auth refresh": tokens=[auth,refresh]; auth group (9 terms) + refresh
    // group (7 terms), disjoint -> 16 parts, byte-sorted. BIT-EXACT.
    assert_eq!(
        expand_concepts("auth refresh"),
        "auth refresh auth authentication bearer credential identity login \
         oauth refresh reissue renew renewal revoke rotate session token update"
    );
    // The expanded token SET is exactly those 16 terms (query words add
    // nothing new). BIT-EXACT vec equality.
    assert_eq!(
        tokenize(&expand_concepts("auth refresh")),
        vec![
            "auth",
            "authentication",
            "bearer",
            "credential",
            "identity",
            "login",
            "oauth",
            "refresh",
            "reissue",
            "renew",
            "renewal",
            "revoke",
            "rotate",
            "session",
            "token",
            "update"
        ]
    );
}
