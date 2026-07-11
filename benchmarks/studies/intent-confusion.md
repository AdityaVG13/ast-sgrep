# Intent confusion study

> **Published record** of measured results. No runnable harnesses ship in this tree.

Produced by `intent-calibrate.py` on 2026-07-10 (asgrep release-perf,
gold sets: self 18q, ripgrep 14q, flask 15q; aggregate = mean of the three
corpus MRRs from `asgrep eval`).

## Routing result

| config | aggregate MRR | symbol MRR | conceptual MRR |
|--------|--------------:|-----------:|---------------:|
| fixed fusion (all weights 1.0) | 0.3980 | 1.000 | 0.3368 |
| **routed: conceptual graph=0.9 (baked default)** | **0.4100** | **1.000** | **0.3498** |
| conceptual graph=0.8 | 0.3983 | 1.000 | 0.3377 |
| conceptual graph=0.7 | 0.4043 | 1.000 | 0.3445 |

- Routing beats fixed fusion by +0.012 aggregate MRR (+3.0% relative).
- No class regresses (misroute floor: every class must stay within 2% of
  fixed fusion; symbol is byte-identical, conceptual improves).
- Embed and anchor multipliers in {0.9..1.25} had no measurable effect and
  stay at 1.0.
- Non-monotonic graph weight (0.9 > 0.7 > 0.8) reflects flat-scored graph
  edges crossing different def-hit score bands per corpus; 0.9 is the only
  setting that improves all three corpora simultaneously.

Reproduce:

```bash
cargo build --profile release-perf -p ast-sgrep-cli
cd benchmarks && python3 intent-calibrate.py --bin ../target/release-perf/asgrep --full-grid
```

## Classifier confusion matrix (rows = gold label, cols = predicted)

Labels derive from gold query-name prefixes (defs_/callers_/literal_ ->
symbol; nl_/synonym_/rg_/fl_ -> conceptual). 47 queries total.

| gold \ predicted | literal | symbol | structural | conceptual |
|------------------|--------:|-------:|-----------:|-----------:|
| literal | 0 | 0 | 0 | 0 |
| symbol | 0 | **6** | 0 | 0 |
| structural | 0 | 0 | 0 | 0 |
| conceptual | 0 | 0 | 0 | **41** |

Zero misclassifications on the gold sets. The literal and structural rows
are empty because no committed gold query exercises those classes yet
(quoted-string and code-snippet shapes are covered by unit tests in
`crates/ast-sgrep-core/src/intent.rs`); expanding gold coverage for them is
part of ast-sgrep-cc0.

Runtime override for experiments:

```bash
ASGREP_INTENT_WEIGHTS="conceptual:graph=0.85,embed=1.1" asgrep "where is auth" .
```

## Addendum (2026-07-10): expanded-suite confusion matrix (ast-sgrep-cc0)

Gold-set expansion added two foreign corpora with class-bearing query-name
prefixes so the literal/symbol classifier rows get real coverage (they were
previously empty): `benchmarks/gold/tokio.json` (tokio 1.38.0, Rust, 30
queries: 22 `tk_nl_` conceptual, 5 `tk_sym_` bare-identifier, 3 `tk_lit_`
quoted-string) and `benchmarks/gold/express.json` (express 4.19.2,
JavaScript, 25 queries: 18 `ex_nl_` conceptual, 4 `ex_sym_` bare-identifier,
3 `ex_lit_` quoted-string). `intent-calibrate.py`'s `CORPORA` and
`LABEL_PREFIXES` were extended accordingly (`tk_nl_`/`tk_sym_`/`tk_lit_` and
`ex_nl_`/`ex_sym_`/`ex_lit_`). The existing calibration result section above
is left as-is; it documents the 3-corpus 2026-07-10 run. This addendum
documents the same script re-run once, unchanged otherwise, over all five
corpora (self 18q, ripgrep 14q, flask 15q, tokio 30q, express 25q = 102
queries total).

Reproduce:

```bash
cd benchmarks && python3 intent-calibrate.py --bin ../target/release-perf/asgrep
```

Result (default single-config mode, not the full grid):

```
fixed fusion: aggregate MRR 0.3388  per-class {'symbol': 0.7, 'conceptual': 0.2403, 'literal': 0.3612}
conceptual:graph=0.8,embed=1.0,anchor=1.0: aggregate 0.3660  per-class {'symbol': 0.7, 'conceptual': 0.2845, 'literal': 0.3612}
BEST: conceptual:graph=0.8,embed=1.0,anchor=1.0  aggregate MRR 0.3660 (fixed 0.3388, delta +0.0272)
```

Confusion matrix (rows = gold label, cols = predicted), 102 queries:

| gold \ predicted | literal | symbol | structural | conceptual |
|------------------|--------:|-------:|-----------:|-----------:|
| literal | **6** | 0 | 0 | 0 |
| symbol | 0 | **10** | 0 | 5 |
| structural | 0 | 0 | 0 | 0 |
| conceptual | 0 | 0 | 0 | **81** |

Literal is now populated and clean: all 6 `*_lit_` gold queries (quoted
error-message / string-literal searches) classify as `literal`. Structural
is still empty -- no committed gold query exercises code-snippet-shaped
queries yet; that remains open follow-up work, not something this expansion
covers.

**Misclassification found (honest, not relabeled):** 5 of 15 symbol-labeled
queries are classified `conceptual` instead of `symbol`: `tk_sym_Notify`,
`ex_sym_Router`, `ex_sym_Layer`, `ex_sym_Route`, `ex_sym_View`. All five are
bare, single-word PascalCase type names (`Notify`, `Router`, `Layer`,
`Route`, `View`) with no internal lowercase-to-uppercase transition after
the leading capital letter. The classifier's `ident_like()` heuristic
(`crates/ast-sgrep-core/src/intent.rs`, `classify_hybrid`) treats a token as
identifier-shaped only if it contains `::`, `_`, ends with `()`, or has an
internal lowercase-to-uppercase transition (e.g. `JoinHandle`,
`TcpListener`, `AsyncRead` -- all of which *did* classify correctly as
`symbol` in this same gold set). A single capitalized word with no
internal case transition falls through every check and lands on
`Conceptual` by default. This is a real, reproducible classifier gap for
one-word type-name lookups, not a labeling error in the gold set (`Notify`,
`Router`, `Layer`, `Route`, `View` are unambiguous bare-identifier queries
by the same convention as the pre-existing `literal_*` self-corpus
queries). Left as signal for follow-up on `ident_like()`; gold labels were
not adjusted to make the classifier look right.

### Follow-up (2026-07-10, later): PascalCase gap fixed

`classify_hybrid` now also treats a single TitleCase word ("Router",
"Notify") as identifier-shaped in short (<=2 token) queries
(`title_case()` in `crates/ast-sgrep-core/src/intent.rs`). Rerun of the
same 5-corpus, 102-query suite:

| gold \ predicted | literal | symbol | structural | conceptual |
|------------------|--------:|-------:|-----------:|-----------:|
| literal | **6** | 0 | 0 | 0 |
| symbol | 0 | **15** | 0 | 0 |
| structural | 0 | 0 | 0 | 0 |
| conceptual | 0 | 0 | 0 | **81** |

102/102 correct. Routing on the expanded suite: fixed fusion aggregate MRR
0.3332 vs routed 0.3660 (+0.0328); symbol and literal classes unchanged by
routing. Structural gold coverage remains open.
