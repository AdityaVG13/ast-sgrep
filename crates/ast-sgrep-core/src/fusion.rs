use crate::intent::ChannelWeights;
use crate::rank::{rrf_score, RRF_K};
use crate::search::{HitKind, SearchHit};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FusionChannel {
    Lexical,
    Definition,
    Caller,
    Graph,
    Anchor,
    Semantic,
    Pattern,
    Import,
}

impl FusionChannel {
    pub const ALL: [Self; 8] = [
        Self::Lexical,
        Self::Definition,
        Self::Caller,
        Self::Graph,
        Self::Anchor,
        Self::Semantic,
        Self::Pattern,
        Self::Import,
    ];

    fn index(self) -> usize {
        match self {
            Self::Lexical => 0,
            Self::Definition => 1,
            Self::Caller => 2,
            Self::Graph => 3,
            Self::Anchor => 4,
            Self::Semantic => 5,
            Self::Pattern => 6,
            Self::Import => 7,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRanks {
    pub lexical: Option<usize>,
    pub definition: Option<usize>,
    pub caller: Option<usize>,
    pub graph: Option<usize>,
    pub anchor: Option<usize>,
    pub semantic: Option<usize>,
    pub pattern: Option<usize>,
    pub import: Option<usize>,
}

impl ChannelRanks {
    pub fn get(&self, channel: FusionChannel) -> Option<usize> {
        match channel {
            FusionChannel::Lexical => self.lexical,
            FusionChannel::Definition => self.definition,
            FusionChannel::Caller => self.caller,
            FusionChannel::Graph => self.graph,
            FusionChannel::Anchor => self.anchor,
            FusionChannel::Semantic => self.semantic,
            FusionChannel::Pattern => self.pattern,
            FusionChannel::Import => self.import,
        }
    }

    fn set_best(&mut self, channel: FusionChannel, rank: usize) {
        let slot = match channel {
            FusionChannel::Lexical => &mut self.lexical,
            FusionChannel::Definition => &mut self.definition,
            FusionChannel::Caller => &mut self.caller,
            FusionChannel::Graph => &mut self.graph,
            FusionChannel::Anchor => &mut self.anchor,
            FusionChannel::Semantic => &mut self.semantic,
            FusionChannel::Pattern => &mut self.pattern,
            FusionChannel::Import => &mut self.import,
        };
        if slot.is_none_or(|current| rank < current) {
            *slot = Some(rank);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionCandidate {
    pub id: String,
    pub relevance: f64,
    pub ranks: ChannelRanks,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FusionExample {
    pub query: String,
    pub candidates: Vec<FusionCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightSensitivity {
    pub channel: FusionChannel,
    pub gradient: f64,
    pub curvature: f64,
    pub rank_churn: f64,
    pub stiff: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedFusionModel {
    pub weights: ChannelWeights,
    pub loss_before: f64,
    pub loss_after: f64,
    pub sensitivity: Vec<WeightSensitivity>,
}

impl LearnedFusionModel {
    pub fn intent_weight_spec(&self, intent: &str) -> String {
        format!(
            "{intent}:lexical={:.6},def={:.6},caller={:.6},graph={:.6},anchor={:.6},embed={:.6},pattern={:.6},import={:.6}",
            self.weights.lexical,
            self.weights.def,
            self.weights.caller,
            self.weights.graph,
            self.weights.anchor,
            self.weights.embed,
            self.weights.pattern,
            self.weights.import,
        )
    }
}

fn channel_for_kind(kind: HitKind) -> FusionChannel {
    match kind {
        HitKind::Asgrep => FusionChannel::Lexical,
        HitKind::Def => FusionChannel::Definition,
        HitKind::Caller => FusionChannel::Caller,
        HitKind::Graph => FusionChannel::Graph,
        HitKind::Anchor => FusionChannel::Anchor,
        HitKind::Embed => FusionChannel::Semantic,
        HitKind::Pattern => FusionChannel::Pattern,
        HitKind::Import => FusionChannel::Import,
    }
}

fn clamp_channel_weight(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.25, 2.0)
    } else {
        1.0
    }
}

fn weight(weights: &ChannelWeights, channel: FusionChannel) -> f64 {
    clamp_channel_weight(match channel {
        FusionChannel::Lexical => weights.lexical,
        FusionChannel::Definition => weights.def,
        FusionChannel::Caller => weights.caller,
        FusionChannel::Graph => weights.graph,
        FusionChannel::Anchor => weights.anchor,
        FusionChannel::Semantic => weights.embed,
        FusionChannel::Pattern => weights.pattern,
        FusionChannel::Import => weights.import,
    })
}

fn set_weight(weights: &mut ChannelWeights, channel: FusionChannel, value: f64) {
    let slot = match channel {
        FusionChannel::Lexical => &mut weights.lexical,
        FusionChannel::Definition => &mut weights.def,
        FusionChannel::Caller => &mut weights.caller,
        FusionChannel::Graph => &mut weights.graph,
        FusionChannel::Anchor => &mut weights.anchor,
        FusionChannel::Semantic => &mut weights.embed,
        FusionChannel::Pattern => &mut weights.pattern,
        FusionChannel::Import => &mut weights.import,
    };
    *slot = clamp_channel_weight(value);
}

pub fn weighted_rrf_score(ranks: &ChannelRanks, weights: &ChannelWeights) -> f64 {
    FusionChannel::ALL
        .into_iter()
        .filter_map(|channel| {
            ranks
                .get(channel)
                .map(|rank| weight(weights, channel) * rrf_score(rank, RRF_K))
        })
        .sum()
}

fn canonical_priority(kind: HitKind) -> usize {
    match kind {
        HitKind::Def => 0,
        HitKind::Caller => 1,
        HitKind::Graph => 2,
        HitKind::Pattern => 3,
        HitKind::Import => 4,
        HitKind::Anchor => 5,
        HitKind::Asgrep => 6,
        HitKind::Embed => 7,
    }
}

pub fn apply_weighted_rrf(hits: &mut Vec<SearchHit>, weights: &ChannelWeights) {
    if hits.is_empty() {
        return;
    }
    let mut channels: [Vec<usize>; 8] = std::array::from_fn(|_| Vec::new());
    let mut members_by_result = BTreeMap::<(String, u32), Vec<usize>>::new();
    for (index, hit) in hits.iter().enumerate() {
        if hit.score.is_finite() && hit.score > 0.0 {
            channels[channel_for_kind(hit.kind).index()].push(index);
            members_by_result
                .entry((hit.file.clone(), hit.line_start))
                .or_default()
                .push(index);
        }
    }
    let mut ranks_by_result = HashMap::<(String, u32), ChannelRanks>::new();
    for channel in FusionChannel::ALL {
        let members = &mut channels[channel.index()];
        members.sort_by(|left, right| {
            hits[*right]
                .score
                .total_cmp(&hits[*left].score)
                .then_with(|| hits[*left].file.cmp(&hits[*right].file))
                .then_with(|| hits[*left].line_start.cmp(&hits[*right].line_start))
                .then_with(|| hits[*left].line_end.cmp(&hits[*right].line_end))
        });
        let mut seen_results = std::collections::HashSet::new();
        let mut rank = 0usize;
        for index in members.iter().copied() {
            let hit = &hits[index];
            let key = (hit.file.clone(), hit.line_start);
            if seen_results.insert(key.clone()) {
                ranks_by_result
                    .entry(key)
                    .or_default()
                    .set_best(channel, rank);
                rank += 1;
            }
        }
    }

    let mut fused = Vec::with_capacity(members_by_result.len());
    for (key, mut members) in members_by_result {
        members.sort_by(|left, right| {
            canonical_priority(hits[*left].kind)
                .cmp(&canonical_priority(hits[*right].kind))
                .then_with(|| hits[*right].score.total_cmp(&hits[*left].score))
                .then_with(|| hits[*left].line_end.cmp(&hits[*right].line_end))
                .then_with(|| hits[*left].symbol.cmp(&hits[*right].symbol))
                .then_with(|| hits[*left].caller.cmp(&hits[*right].caller))
                .then_with(|| hits[*left].callee.cmp(&hits[*right].callee))
                .then_with(|| hits[*left].language.cmp(&hits[*right].language))
                .then_with(|| hits[*left].excerpt.cmp(&hits[*right].excerpt))
        });
        let mut result = hits[members[0]].clone();
        let mut contributors = members
            .iter()
            .map(|index| hits[*index].kind)
            .collect::<Vec<_>>();
        contributors.sort_by_key(|kind| channel_for_kind(*kind).index());
        contributors.dedup();
        result.contributors = contributors;
        result.score = weighted_rrf_score(&ranks_by_result[&key], weights);
        fused.push(result);
    }
    *hits = fused;
}

fn pairwise_loss(examples: &[FusionExample], weights: &ChannelWeights) -> f64 {
    let mut loss = 0.0;
    let mut pairs = 0usize;
    for example in examples {
        for (index, better) in example.candidates.iter().enumerate() {
            for worse in example.candidates.iter().skip(index + 1) {
                let (better, worse) = if better.relevance > worse.relevance {
                    (better, worse)
                } else if worse.relevance > better.relevance {
                    (worse, better)
                } else {
                    continue;
                };
                let delta = (weighted_rrf_score(&better.ranks, weights)
                    - weighted_rrf_score(&worse.ranks, weights))
                    * 100.0;
                loss += if delta >= 0.0 {
                    (-delta).exp().ln_1p()
                } else {
                    -delta + delta.exp().ln_1p()
                };
                pairs += 1;
            }
        }
    }
    if pairs == 0 {
        0.0
    } else {
        loss / pairs as f64
    }
}

fn ranking_churn(
    examples: &[FusionExample],
    baseline: &ChannelWeights,
    perturbed: &ChannelWeights,
) -> f64 {
    let mut changed = 0usize;
    let mut pairs = 0usize;
    for example in examples {
        for (index, left) in example.candidates.iter().enumerate() {
            for right in example.candidates.iter().skip(index + 1) {
                let baseline_order = weighted_rrf_score(&left.ranks, baseline)
                    .total_cmp(&weighted_rrf_score(&right.ranks, baseline));
                let perturbed_order = weighted_rrf_score(&left.ranks, perturbed)
                    .total_cmp(&weighted_rrf_score(&right.ranks, perturbed));
                changed += usize::from(baseline_order != perturbed_order);
                pairs += 1;
            }
        }
    }
    if pairs == 0 {
        0.0
    } else {
        changed as f64 / pairs as f64
    }
}

pub fn analyze_weight_sensitivity(
    examples: &[FusionExample],
    weights: &ChannelWeights,
    step: f64,
) -> Vec<WeightSensitivity> {
    let step = if step.is_finite() && step > 0.0 {
        step.min(0.5)
    } else {
        0.1
    };
    let base_loss = pairwise_loss(examples, weights);
    let mut rows = Vec::with_capacity(FusionChannel::ALL.len());
    for channel in FusionChannel::ALL {
        let center = weight(weights, channel);
        let left_room = center - 0.25;
        let right_room = 2.0 - center;
        let (gradient, curvature, rank_churn) = if left_room >= step && right_room >= step {
            let mut plus = weights.clone();
            let mut minus = weights.clone();
            set_weight(&mut plus, channel, center + step);
            set_weight(&mut minus, channel, center - step);
            let plus_loss = pairwise_loss(examples, &plus);
            let minus_loss = pairwise_loss(examples, &minus);
            (
                (plus_loss - minus_loss) / (2.0 * step),
                ((plus_loss - 2.0 * base_loss + minus_loss) / step.powi(2)).max(0.0),
                ranking_churn(examples, weights, &plus)
                    .max(ranking_churn(examples, weights, &minus)),
            )
        } else if right_room >= left_room {
            let h = step.min(right_room / 2.0);
            let mut first = weights.clone();
            let mut second = weights.clone();
            set_weight(&mut first, channel, center + h);
            set_weight(&mut second, channel, center + 2.0 * h);
            let first_loss = pairwise_loss(examples, &first);
            let second_loss = pairwise_loss(examples, &second);
            (
                (-3.0 * base_loss + 4.0 * first_loss - second_loss) / (2.0 * h),
                ((base_loss - 2.0 * first_loss + second_loss) / h.powi(2)).max(0.0),
                ranking_churn(examples, weights, &first)
                    .max(ranking_churn(examples, weights, &second)),
            )
        } else {
            let h = step.min(left_room / 2.0);
            let mut first = weights.clone();
            let mut second = weights.clone();
            set_weight(&mut first, channel, center - h);
            set_weight(&mut second, channel, center - 2.0 * h);
            let first_loss = pairwise_loss(examples, &first);
            let second_loss = pairwise_loss(examples, &second);
            (
                (3.0 * base_loss - 4.0 * first_loss + second_loss) / (2.0 * h),
                ((base_loss - 2.0 * first_loss + second_loss) / h.powi(2)).max(0.0),
                ranking_churn(examples, weights, &first)
                    .max(ranking_churn(examples, weights, &second)),
            )
        };
        rows.push(WeightSensitivity {
            channel,
            gradient,
            curvature,
            rank_churn,
            stiff: false,
        });
    }
    let max_curvature = rows.iter().map(|row| row.curvature).fold(0.0, f64::max);
    for row in &mut rows {
        row.stiff =
            (max_curvature > 0.0 && row.curvature >= max_curvature * 0.1) || row.rank_churn >= 0.05;
    }
    rows
}

pub fn learn_fusion_weights(
    examples: &[FusionExample],
    mut initial: ChannelWeights,
) -> LearnedFusionModel {
    for channel in FusionChannel::ALL {
        let value = weight(&initial, channel);
        set_weight(&mut initial, channel, value);
    }
    let loss_before = pairwise_loss(examples, &initial);
    let sensitivity = analyze_weight_sensitivity(examples, &initial, 0.1);
    let stiff = sensitivity
        .iter()
        .filter(|row| row.stiff)
        .map(|row| row.channel)
        .collect::<Vec<_>>();
    let mut weights = initial;
    let mut best_loss = loss_before;
    let mut step = 0.25;
    for _ in 0..64 {
        let mut improved = false;
        for channel in &stiff {
            let center = weight(&weights, *channel);
            for candidate_value in [center - step, center + step] {
                let mut candidate = weights.clone();
                set_weight(&mut candidate, *channel, candidate_value);
                let candidate_loss = pairwise_loss(examples, &candidate);
                if candidate_loss + 1e-12 < best_loss {
                    weights = candidate;
                    best_loss = candidate_loss;
                    improved = true;
                }
            }
        }
        step *= if improved { 0.9 } else { 0.5 };
        if step < 1e-3 {
            break;
        }
    }
    LearnedFusionModel {
        weights,
        loss_before,
        loss_after: best_loss,
        sensitivity,
    }
}

#[cfg(test)]
mod tests {
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
}
