use ast_sgrep_core::intent::QueryIntent;
use ast_sgrep_core::search::field_weight::*;
use ast_sgrep_core::semantic_chunk::SemanticFieldVectors;

fn blob(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn populated_fields() -> SemanticFieldVectors {
    SemanticFieldVectors {
        name: Some(blob(&[1.0, 0.0])),
        docs: Some(blob(&[0.0, 1.0])),
        body: Some(blob(&[1.0, 1.0])),
        graph: Some(blob(&[0.5, 0.5])),
        tests_examples: Some(blob(&[0.0, 0.0])),
    }
}

#[test]
fn literal_intent_skips_zero_weight_why_terms() {
    let query = [1.0, 0.0];
    let (score, notes) =
        rescore_similarity(0.42, &query, &populated_fields(), QueryIntent::Literal);
    assert_eq!(score, 0.42);
    assert!(
        notes.is_none(),
        "literal why must not emit unweighted embed_field terms: {notes:?}"
    );
    assert!(!field_weights(QueryIntent::Literal).mask().any());
}

#[test]
fn symbol_intent_scores_only_name() {
    let query = [1.0, 0.0];
    let (score, notes) = rescore_similarity(0.1, &query, &populated_fields(), QueryIntent::Symbol);
    let notes = notes.expect("symbol queries expose the name field");
    assert!(notes.name.is_some());
    assert!(notes.docs.is_none());
    assert!(notes.body.is_none());
    assert!(notes.graph.is_none());
    assert!(notes.tests_examples.is_none());
    assert!(
        score > 0.9,
        "name-only mix should keep the name cosine, got {score}"
    );
    let why = notes.why_terms();
    assert!(
        why.iter().any(|t| t.starts_with("embed_field:name=")),
        "{why:?}"
    );
    assert!(
        why.iter().all(|t| t.starts_with("embed_field:name=")),
        "{why:?}"
    );
}

#[test]
fn conceptual_intent_includes_name() {
    let weights = field_weights(QueryIntent::Conceptual);
    assert!(
        weights.name > 0.0,
        "conceptual name weight must participate"
    );
    assert!(weights.body > 0.0 && weights.docs > 0.0);
    let query = [1.0, 0.0];
    let (score, notes) =
        rescore_similarity(0.1, &query, &populated_fields(), QueryIntent::Conceptual);
    let notes = notes.expect("conceptual queries expose weighted fields");
    assert!(notes.name.is_some(), "{notes:?}");
    assert!(
        score > 0.1,
        "name-inclusive mix should beat the dummy primary, got {score}"
    );
}

#[test]
fn weak_fields_never_veto_a_strong_primary() {
    // INTENT: the primary vector is the trained joint embedding of the
    // chunk; the per-field mix is an auxiliary estimate. Fields may only
    // lift a score — when every field disagrees with the query but the
    // joint estimate is strong, the score must stay at the primary, not
    // collapse to the mix. (Replace semantics let one weak field veto a
    // strong joint match; that was a hashed-backend holdover.)
    let query = [1.0, 0.0];
    let orthogonal = SemanticFieldVectors {
        name: Some(blob(&[0.0, 1.0])),
        docs: Some(blob(&[0.0, 1.0])),
        body: Some(blob(&[0.0, 1.0])),
        graph: Some(blob(&[0.0, 1.0])),
        tests_examples: Some(blob(&[0.0, 1.0])),
    };
    let (score, _) = rescore_similarity(0.9, &query, &orthogonal, QueryIntent::Conceptual);
    assert_eq!(
        score, 0.9,
        "fields must only lift; a zero mix must not veto the primary"
    );
}

/// INTENT: field rank union ordering (field_rank_union_keys). A row's order
/// key is (best rank across lists, cosine of the list that produced it).
/// A docs-list top hit must outrank a name-list top hit at equal rank when
/// its winning cosine is higher — the joint/concat similarity must NOT be
/// the tie-break, or body-diluted rows keep burying docs evidence.
#[test]
fn field_rank_union_orders_by_winning_list_cosine() {
    // ranked is (chunk idx, similarity) in concat/similarity order: the
    // distractor (idx 1) leads the primary list.
    let ranked: Vec<(usize, f32)> = vec![(1, 0.90), (0, 0.50), (2, 0.10)];
    let notes = vec![
        // idx 0: gold — exact docs evidence, weak elsewhere.
        Some(EmbedFieldScores {
            name: Some(0.10),
            docs: Some(0.95),
            body: Some(0.20),
            graph: None,
            tests_examples: None,
        }),
        // idx 1: distractor — strong name+body, generic docs.
        Some(EmbedFieldScores {
            name: Some(0.90),
            docs: Some(0.15),
            body: Some(0.85),
            graph: None,
            tests_examples: None,
        }),
        // idx 2: weak everywhere.
        Some(EmbedFieldScores {
            name: Some(0.05),
            docs: Some(0.05),
            body: Some(0.05),
            graph: None,
            tests_examples: None,
        }),
    ];
    let keys = field_rank_union_keys(
        &ranked,
        &["a.rs", "b.rs", "c.rs"],
        &notes,
        FieldRankUnionMode::All,
    );
    // keys are indexed by primary position: pos 0 = distractor (idx 1),
    // pos 1 = gold (idx 0), pos 2 = weak (idx 2).
    // Docs list: gold 0.95 > distractor 0.15 > weak 0.05 → gold rank 0.
    // Name list: distractor 0.90 > gold 0.10 → distractor rank 0.
    // Both leaders tie at rank 0; the winning cosine (0.95 > 0.90) must
    // put gold first when positions are ordered by (rank, -cos).
    assert_eq!(
        keys[0],
        (0, 0.90),
        "distractor keys = name-list rank with its cosine"
    );
    assert_eq!(
        keys[1],
        (0, 0.95),
        "gold keys = docs-list rank with its cosine"
    );
    // The weak row's best rank (2) is available in every list; its winning
    // cosine is its primary similarity (0.10), which beats its field
    // cosines (0.05) at the same rank.
    assert_eq!(keys[2], (2, 0.10), "weak row keeps its per-list rank");

    // The order the channel must emit: gold (docs winner) before the
    // distractor (name winner) despite the distractor's higher joint
    // similarity.
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|&a, &b| {
        keys[a]
            .0
            .cmp(&keys[b].0)
            .then_with(|| keys[b].1.total_cmp(&keys[a].1))
    });
    assert_eq!(
        order,
        vec![1, 0, 2],
        "winning-list cosine, not joint similarity, breaks rank ties"
    );

    // Monotonicity: a row never falls below its primary position. The weak
    // row (primary position 2) stays at rank 2 even though it appears in
    // every field list.
    assert!(
        keys[2].0 >= 2,
        "row at primary position 2 must not improve below 2"
    );

    // Kill switch: `ASGREP_FIELD_RANK_UNION=off` disables the union at the
    // call site (embed_similarity_hits), which then sorts by similarity —
    // that arm is exercised by the paired corpus measurement, not here.
}

/// INTENT: python-gated docs union (`docs-python`). Docstrings are
/// python-specific prose (PEP 257, first statement below the def); in rust
/// the docs field carries `///` comments that echo the signature, and
/// unioning those armed identifier distractors (round D measured ripgrep
/// .582 -> .568). The docs list must therefore form from `.py` rows only:
/// a non-py row keeps its concat key even when its docs cosine would top
/// the list, so non-python corpora are bit-identical to `off` by
/// construction (no `.py` docs rows -> list never reaches the 2-member
/// guard).
#[test]
fn docs_python_union_gates_list_to_python_rows() {
    // ranked is (chunk idx, similarity) in concat order: the rust row leads.
    let ranked: Vec<(usize, f32)> = vec![(0, 0.90), (1, 0.50), (2, 0.10)];
    let files = vec!["src/a.rs", "src/b.py", "src/c.py"];
    let note = |docs: f32| {
        Some(EmbedFieldScores {
            name: None,
            docs: Some(docs),
            body: None,
            graph: None,
            tests_examples: None,
        })
    };
    let notes = vec![note(0.95), note(0.60), note(0.20)];
    let keys = field_rank_union_keys(
        &ranked,
        &files,
        &notes,
        FieldRankUnionMode::DocsPython,
    );
    // Rust row excluded from the docs list: keeps its concat key.
    assert_eq!(keys[0], (0, 0.90), "non-py row keeps its concat key");
    // Python rows re-ranked among themselves: b.py lifted to rank 0.
    assert_eq!(keys[1], (0, 0.60), "b.py wins the python-only docs list");
    assert_eq!(keys[2], (1, 0.20), "c.py second in the python-only list");
    // Contrast: ungated `docs` hands the rust row the docs-list lead —
    // exactly the round-D regression shape.
    let ungated = field_rank_union_keys(
        &ranked,
        &files,
        &notes,
        FieldRankUnionMode::Docs,
    );
    assert_eq!(
        ungated[0],
        (0, 0.95),
        "ungated docs hands the rust row the docs-list lead"
    );
}
