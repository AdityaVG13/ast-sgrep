# Conformance coverage skeleton

Legend: **covered** | **partial** | **gap** | **disc** (see DISCREPANCIES.md) | **deferred**.

This is a living index, not a score. Empty cells are unknown until a child
bead fills them. Do not treat blanks as Pass.

## Surfaces

| ID | Surface | Status | Notes |
|---|---|---|---|
| S1 | Hybrid / NL search | partial | Ranking must_include oracle only (`DISC-ranking-soft-oracle`) |
| S2 | Lexical / keyword | partial | FTS, not rg (`DISC-lexical-not-rg`) |
| S3 | Graph (defs/callers/imports) | partial | `tests/core/graph_oracle.rs` |
| S4 | Native `pattern:` | disc | `DISC-pattern-native-subset` |
| S5 | Semantic / ANN | partial | Adaptive IVF (`DISC-ivf-adaptive-threshold`) |
| S6 | Machine JSON / CLI envelopes | partial | `tests/cli/machine_contracts.rs` + nz7i goldens |
| S7 | Compact / agent formats | disc | `DISC-compact-drops-provenance` |
| S8 | MCP tools | disc | `DISC-mcp-not-full-suite` |
| S9 | LSP | partial | Navigation surface; not full CLI |
| S10 | Extraction dumps | disc | `DISC-extraction-presence-only` until nz7i.4 |

## Deferred external differentials

| Oracle | Status | Pointer |
|---|---|---|
| ast-grep CLI identity | deferred | `DISC-pattern-native-subset`, `DISC-no-jell-harness` |
| ripgrep identity | deferred | `DISC-lexical-not-rg` |
| jell harness | deferred | `docs/validation/jell-deferral.md` |

## How to regenerate

1. Do not invent coverage. Edit this table when a test or DISC row lands.
2. Child **ghiw.2** fills MUST matrices. Child **ghiw.3** owns pattern vs
   ast-grep differential. Child **ghiw.5** emits a report from these files.
3. Proof pack commands stay in `docs/validation/proof-pack.md`.
4. Verdict rules: `docs/validation/conformance-verdicts.md`.
