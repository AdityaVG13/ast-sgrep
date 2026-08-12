use super::*;

#[test]
fn learning_storage_is_hard_bounded() {
    let mut builder = LexiconBuilder::new();
    for index in 0..4_100 {
        builder.observe(&Observation {
            identifier_terms: vec![format!("identifier{index}")],
            prose_terms: (0..MAX_PROSE_TERMS)
                .map(|term| format!("prose{index}_{term}"))
                .collect(),
        });
    }
    assert!(builder.pair_counts.len() <= MAX_PAIRS);
    assert!(builder.observations <= MAX_OBSERVATIONS);
    // With one identifier and N prose terms, there is one more retained
    // term than pairs per observation; MAX_OBSERVATIONS covers that gap.
    assert!(builder.term_counts.len() <= MAX_PAIRS + MAX_OBSERVATIONS as usize);
}
