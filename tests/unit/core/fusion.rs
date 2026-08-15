use super::*;

fn candidate(
    id: &str,
    relevance: f64,
    lexical: Option<usize>,
    semantic: Option<usize>,
) -> FusionCandidate {
    FusionCandidate {
        id: id.into(),
        relevance,
        ranks: ChannelRanks {
            lexical,
            semantic,
            ..ChannelRanks::default()
        },
    }
}

#[test]
fn learner_improves_stiff_channel_without_tuning_sloppy_channels() {
    let examples = vec![FusionExample {
        query: "renew credentials".into(),
        candidates: vec![
            candidate("relevant", 2.0, Some(8), Some(0)),
            candidate("distractor", 0.0, Some(0), Some(8)),
        ],
    }];
    let initial = ChannelWeights::default();
    let model = learn_fusion_weights(&examples, initial.clone());
    assert!(model.loss_after < model.loss_before);
    assert!(model.weights.embed > model.weights.lexical);
    assert_eq!(model.weights.graph, initial.graph);
    let graph = model
        .sensitivity
        .iter()
        .find(|row| row.channel == FusionChannel::Graph)
        .unwrap();
    assert!(!graph.stiff);
    assert_eq!(graph.curvature, 0.0);
    assert_eq!(graph.rank_churn, 0.0);
    for row in model.sensitivity.iter().filter(|row| row.stiff) {
        for delta in [-1e-3, 1e-3] {
            let mut neighbor = model.weights.clone();
            let center = weight(&neighbor, row.channel);
            set_weight(&mut neighbor, row.channel, center + delta);
            assert!(pairwise_loss(&examples, &neighbor) + 1e-10 >= model.loss_after);
        }
    }
}

#[test]
fn boundary_sensitivity_uses_one_sided_stencils() {
    let examples = vec![FusionExample {
        query: "renew credentials".into(),
        candidates: vec![
            candidate("relevant", 2.0, None, Some(0)),
            candidate("distractor", 0.0, Some(0), None),
        ],
    }];
    let weights = ChannelWeights {
        embed: 0.25,
        lexical: 2.0,
        ..ChannelWeights::default()
    };
    let rows = analyze_weight_sensitivity(&examples, &weights, 0.1);
    for channel in [FusionChannel::Semantic, FusionChannel::Lexical] {
        let row = rows.iter().find(|row| row.channel == channel).unwrap();
        assert!(row.gradient.is_finite());
        assert!(row.curvature.is_finite());
        assert_ne!(row.gradient, 0.0);
        assert!(row.stiff);
    }
}

#[test]
fn weighted_rrf_aggregates_channels_by_result_location() {
    fn hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
        SearchHit {
            kind,
            file: file.into(),
            line_start: line,
            line_end: line,
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
    let mut hits = vec![
        hit(HitKind::Asgrep, "both.rs", 1, 0.8),
        hit(HitKind::Embed, "both.rs", 1, 0.8),
        hit(HitKind::Asgrep, "lexical.rs", 1, 1.0),
    ];
    apply_weighted_rrf(&mut hits, &ChannelWeights::default());
    assert_eq!(hits.len(), 2);
    let both = hits.iter().find(|hit| hit.file == "both.rs").unwrap();
    let lexical = hits.iter().find(|hit| hit.file == "lexical.rs").unwrap();
    assert!(both.score > lexical.score);
    assert_eq!(both.kind, HitKind::Asgrep);
    assert_eq!(both.contributors, vec![HitKind::Asgrep, HitKind::Embed]);

    let mut suppressed = vec![
        hit(HitKind::Asgrep, "shared.rs", 1, 1.0),
        hit(HitKind::Embed, "shared.rs", 1, 0.0),
    ];
    apply_weighted_rrf(&mut suppressed, &ChannelWeights::default());
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0].contributors, vec![HitKind::Asgrep]);

    let mut zero = vec![hit(HitKind::Asgrep, "zero.rs", 1, 0.0)];
    apply_weighted_rrf(&mut zero, &ChannelWeights::default());
    assert!(zero.is_empty());
}

#[test]
fn same_channel_duplicates_do_not_consume_rrf_positions() {
    fn lexical(file: &str, score: f64, symbol: Option<&str>) -> SearchHit {
        SearchHit {
            kind: HitKind::Asgrep,
            file: file.into(),
            line_start: 1,
            line_end: 1,
            symbol: symbol.map(str::to_string),
            caller: None,
            callee: None,
            language: None,
            score,
            signal: HitKind::Asgrep.signal(),
            contributors: vec![HitKind::Asgrep],
            margin: 0.0,
            confidence: 0.0,
            resolution: None,
            embed_fields: None,
            critic: Vec::new(),
            excerpt: symbol.unwrap_or_default().into(),
        }
    }
    let mut hits = vec![
        lexical("duplicate.rs", 1.0, Some("zeta")),
        lexical("duplicate.rs", 1.0, Some("alpha")),
        lexical("later.rs", 0.8, None),
    ];
    apply_weighted_rrf(&mut hits, &ChannelWeights::default());
    assert_eq!(hits.len(), 2);
    let duplicate = hits.iter().find(|hit| hit.file == "duplicate.rs").unwrap();
    let later = hits.iter().find(|hit| hit.file == "later.rs").unwrap();
    assert_eq!(duplicate.symbol.as_deref(), Some("alpha"));
    assert!((later.score - rrf_score(1, RRF_K)).abs() < 1e-12);
}

#[test]
fn nonfinite_input_weights_are_sanitized_for_training_and_runtime() {
    let examples = vec![FusionExample {
        query: "query".into(),
        candidates: vec![
            candidate("relevant", 1.0, Some(0), None),
            candidate("other", 0.0, Some(1), None),
        ],
    }];
    let weights = ChannelWeights {
        lexical: f64::NAN,
        graph: f64::INFINITY,
        ..ChannelWeights::default()
    };
    let model = learn_fusion_weights(&examples, weights);
    assert!(model.weights.lexical.is_finite());
    assert!(model.weights.graph.is_finite());
    assert!(model.loss_before.is_finite());
    assert!(model.loss_after.is_finite());
    assert!(model.intent_weight_spec("symbol").contains("import="));
}
