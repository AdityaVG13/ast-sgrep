# Fusion ranking

Hybrid queries rank cascade survivors with weighted reciprocal-rank fusion (RRF). Raw lexical, definition, caller, graph, anchor, semantic, pattern, and import scores are converted to within-channel ranks. Evidence at the same file and start line shares the sum

```text
score(result) = Σ channel_weight / (60 + channel_rank + 1)
```

This prevents incomparable channel score scales from dominating fusion. Each file/start-line identity emits one canonical hit; its `contributors` array records every positive evidence kind that entered fusion. Suppressed evidence is excluded. Ties are deterministic by file and span. Dedicated query modes keep their direct ranking path.

## Post-fusion critic

After weighted RRF, a deterministic critic pass reviews the fused shortlist
(`search/critic.rs`). It is the in-process replacement for "have a second model
check the results": no model, no network, pure evidence rules.

- **Corroboration annotation.** An embed-only hit whose parent span has no local
  same-file span or symbol corroboration is retained with the
  `semantic_uncorroborated` note. Unrelated structural evidence elsewhere never
  deletes it.
- **Agreement boost.** Semantic plus structural agreement on the same span
  multiplies the fused score by 1.15 (`channel_agreement`); definition plus
  usage plus semantic agreement multiplies it by 1.25 (`full_agreement`).
  Signal provenance is untouched: a boosted semantic hit stays `semantic`.
- **Identifier-collision penalty.** When the query names a compound identifier
  (`auth_refresh`) and a hit's symbol is only a fragment of it (`refresh`)
  without evidencing the full identifier, the score is multiplied by 0.85
  (`identifier_collision`). The inverse also applies: `Searcher` demotes
  `bench_searcher`, and a partial token match (`refresh` inside a longer test
  name) loses to the exact spelling.
- **Code over docs / entrypoints.** Markdown lexical hits lose when real code
  exists. Conceptual queries boost symbols that share concept tokens with the
  query and demote generic `main`/`start` callers.

Critic notes surface as `critic:<note>` entries in each hit's `why` array on
the agent envelope. The critic runs before margins and confidence are
assigned, so honesty fields reflect the critiqued ordering. Boost and penalty
constants are engine defaults, not certified weights.

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
