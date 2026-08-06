# Fusion ranking

Hybrid queries rank cascade survivors with weighted reciprocal-rank fusion (RRF). Raw lexical, definition, caller, graph, anchor, semantic, pattern, and import scores are converted to within-channel ranks. Evidence at the same file and start line shares the sum

```text
score(result) = Σ channel_weight / (60 + channel_rank + 1)
```

This prevents incomparable channel score scales from dominating fusion. Each file/start-line identity emits one canonical hit; its `contributors` array records every positive evidence kind that entered fusion. Suppressed evidence is excluded. Ties are deterministic by file and span. Dedicated query modes keep their direct ranking path.

## Learned weights

`ast_sgrep_core::learn_fusion_weights` accepts judged `FusionExample` values. Each candidate supplies relevance and optional per-channel ranks. Training minimizes deterministic pairwise logistic loss with weights clamped to `[0.25, 2.0]`.

```rust
use ast_sgrep_core::{learn_fusion_weights, FusionExample};
use ast_sgrep_core::intent::ChannelWeights;

let model = learn_fusion_weights(&examples, ChannelWeights::default());
assert!(model.loss_after <= model.loss_before);
println!("{}", model.intent_weight_spec("conceptual"));
```

Deploy the emitted value through `ASGREP_INTENT_WEIGHTS`. Multiple intent classes remain semicolon-separated:

```bash
export ASGREP_INTENT_WEIGHTS='conceptual:lexical=0.8,embed=1.4;symbol:def=1.8,graph=1.1'
```

Unknown, nonfinite, or out-of-range values cannot escape the runtime clamp.

## Fisher-style sensitivity

`analyze_weight_sensitivity` perturbs each channel around the supplied weights and reports:

- `gradient`: first-order pairwise-loss movement;
- `curvature`: nonnegative finite-difference curvature, the Fisher-style stiffness proxy;
- `rank_churn`: fraction of candidate pairs whose order changes;
- `stiff`: curvature at least 10% as large as the strongest channel or rank churn of at least 5%.

The learner tunes only channels marked stiff. Channels absent from the judged examples remain unchanged, avoiding unsupported optimization of sloppy parameters. The learned model retains the full sensitivity report for review before deployment.
