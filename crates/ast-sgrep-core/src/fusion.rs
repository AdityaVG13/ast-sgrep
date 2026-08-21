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
        self as usize
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

const RANK_GETTERS: [fn(&ChannelRanks) -> Option<usize>; 8] = [
    |r| r.lexical,
    |r| r.definition,
    |r| r.caller,
    |r| r.graph,
    |r| r.anchor,
    |r| r.semantic,
    |r| r.pattern,
    |r| r.import,
];

const RANK_SETTERS: [fn(&mut ChannelRanks, Option<usize>); 8] = [
    |r, v| r.lexical = v,
    |r, v| r.definition = v,
    |r, v| r.caller = v,
    |r, v| r.graph = v,
    |r, v| r.anchor = v,
    |r, v| r.semantic = v,
    |r, v| r.pattern = v,
    |r, v| r.import = v,
];

const WEIGHT_GETTERS: [fn(&ChannelWeights) -> f64; 8] = [
    |w| w.lexical,
    |w| w.def,
    |w| w.caller,
    |w| w.graph,
    |w| w.anchor,
    |w| w.embed,
    |w| w.pattern,
    |w| w.import,
];

const WEIGHT_SETTERS: [fn(&mut ChannelWeights, f64); 8] = [
    |w, v| w.lexical = v,
    |w, v| w.def = v,
    |w, v| w.caller = v,
    |w, v| w.graph = v,
    |w, v| w.anchor = v,
    |w, v| w.embed = v,
    |w, v| w.pattern = v,
    |w, v| w.import = v,
];

/// HitKind discriminant order → FusionChannel::ALL index.
const HIT_KIND_CHANNEL_IDX: [usize; 8] = [0, 1, 2, 3, 4, 7, 6, 5];

/// HitKind discriminant order → canonical merge priority (lower wins).
const CANONICAL_PRIORITY: [usize; 8] = [6, 0, 1, 2, 5, 4, 3, 7];

impl ChannelRanks {
    pub fn get(&self, channel: FusionChannel) -> Option<usize> {
        RANK_GETTERS[channel.index()](self)
    }

    fn set_best(&mut self, channel: FusionChannel, rank: usize) {
        let idx = channel.index();
        let current = RANK_GETTERS[idx](self);
        if current.is_none_or(|value| rank < value) {
            RANK_SETTERS[idx](self, Some(rank));
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

fn hit_kind_idx(kind: HitKind) -> usize {
    match kind {
        HitKind::Asgrep => 0,
        HitKind::Def => 1,
        HitKind::Caller => 2,
        HitKind::Graph => 3,
        HitKind::Anchor => 4,
        HitKind::Import => 5,
        HitKind::Pattern => 6,
        HitKind::Embed => 7,
    }
}

fn channel_for_kind(kind: HitKind) -> FusionChannel {
    FusionChannel::ALL[HIT_KIND_CHANNEL_IDX[hit_kind_idx(kind)]]
}

fn clamp_channel_weight(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.25, 2.0)
    } else {
        1.0
    }
}

fn weight(weights: &ChannelWeights, channel: FusionChannel) -> f64 {
    clamp_channel_weight(WEIGHT_GETTERS[channel.index()](weights))
}

fn set_weight(weights: &mut ChannelWeights, channel: FusionChannel, value: f64) {
    WEIGHT_SETTERS[channel.index()](weights, clamp_channel_weight(value));
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
    CANONICAL_PRIORITY[hit_kind_idx(kind)]
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
        // Enrich identifying fields from members: a merged same-line result
        // keeps the canonical kind/priority but surfaces the symbol a semantic
        // chunk carries for that line (semantic_cache_version contract).
        if result.symbol.is_none() {
            if let Some(sym) = members
                .iter()
                .map(|index| hits[*index].symbol.as_deref())
                .find(|s| s.is_some())
                .flatten()
            {
                result.symbol = Some(sym.to_string());
            }
        }
        if result.embed_fields.is_none() {
            result.embed_fields = members
                .iter()
                .find_map(|index| hits[*index].embed_fields.clone());
        }
        result.contributors = contributors;
        result.score = weighted_rrf_score(&ranks_by_result[&key], weights);
        fused.push(result);
    }
    *hits = fused;
}

fn pairwise_loss(examples: &[FusionExample], weights: &ChannelWeights) -> f64 {
    if examples.is_empty() {
        return 0.0;
    }
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

fn channel_sensitivity(
    examples: &[FusionExample],
    weights: &ChannelWeights,
    channel: FusionChannel,
    center: f64,
    step: f64,
    base_loss: f64,
) -> (f64, f64, f64) {
    let left_room = center - 0.25;
    let right_room = 2.0 - center;
    if left_room >= step && right_room >= step {
        let mut plus = weights.clone();
        let mut minus = weights.clone();
        set_weight(&mut plus, channel, center + step);
        set_weight(&mut minus, channel, center - step);
        let plus_loss = pairwise_loss(examples, &plus);
        let minus_loss = pairwise_loss(examples, &minus);
        return (
            (plus_loss - minus_loss) / (2.0 * step),
            ((plus_loss - 2.0 * base_loss + minus_loss) / step.powi(2)).max(0.0),
            ranking_churn(examples, weights, &plus).max(ranking_churn(examples, weights, &minus)),
        );
    }
    if right_room >= left_room {
        let h = step.min(right_room / 2.0);
        let mut first = weights.clone();
        let mut second = weights.clone();
        set_weight(&mut first, channel, center + h);
        set_weight(&mut second, channel, center + 2.0 * h);
        let first_loss = pairwise_loss(examples, &first);
        let second_loss = pairwise_loss(examples, &second);
        return (
            (-3.0 * base_loss + 4.0 * first_loss - second_loss) / (2.0 * h),
            ((base_loss - 2.0 * first_loss + second_loss) / h.powi(2)).max(0.0),
            ranking_churn(examples, weights, &first).max(ranking_churn(examples, weights, &second)),
        );
    }
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
        ranking_churn(examples, weights, &first).max(ranking_churn(examples, weights, &second)),
    )
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
        let (gradient, curvature, rank_churn) =
            channel_sensitivity(examples, weights, channel, center, step, base_loss);
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
