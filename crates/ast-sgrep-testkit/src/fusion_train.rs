//! Offline fusion-weight training: pairwise softplus loss, sensitivity
//! analysis, and stiff-channel coordinate descent over channel weights.
//!
//! Measurement-only. No query-path caller uses this module — it exists for
//! the `numerical_fusion` contract suite and offline weight experiments.
//! Deliberately outside `ast-sgrep-core`'s shipped surface: the runtime
//! ranker is `apply_weighted_rrf` with fixed per-intent weights.

use ast_sgrep_core::fusion::{
    ChannelRanks, FusionCandidate, FusionChannel, set_weight, weight, weighted_rrf_score,
};
use ast_sgrep_core::intent::ChannelWeights;
use serde::{Deserialize, Serialize};

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
