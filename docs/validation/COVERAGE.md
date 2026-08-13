# Conformance coverage skeleton

Legend: **covered** | **partial** | **gap** | **disc** (see DISCREPANCIES.md) | **deferred**.

This is a living index, not a score. Empty cells are unknown until a child
bead fills them. Do not treat blanks as Pass.

## Surfaces

| ID | Surface | Status | Notes |
|---|---|---|---|
| S1 | Hybrid / NL search | partial | Ranking must_include oracle only (`DISC-ranking-soft-oracle`) |
| S2 | Lexical / keyword | partial | FTS, not rg (`DISC-lexical-not-rg`). Query prefix MUST matrix: `docs/QUERY_GRAMMAR.md` QG-001…026 (parse covered; search identity still FTS). |
| S3 | Graph (defs/callers/imports) | partial | `tests/core/graph_oracle.rs` |
| S4 | Native `pattern:` | partial | Supported native hits + unsupported fail-closed in `tests/core/pattern_diff.rs`. Full ast-grep identity is **disc** (`DISC-pattern-native-subset`); Pattern-1 equality is `#[ignore]` / `ASGREP_DIFF_AST_GREP`. |
| S5 | Semantic / ANN | partial | Adaptive IVF (`DISC-ivf-adaptive-threshold`) |
| S6 | Machine JSON / CLI envelopes | partial | MJ-001…010, MJ-013 covered in `machine_contracts.rs`. **MJ-011** hit-array freeze = gap (nz7i.2). **MJ-012** MCP envelope = `DISC-mcp-not-full-suite`. |
| S7 | Compact / agent formats | disc | `DISC-compact-drops-provenance` (NL-008 asserts compact ≠ native hit array) |
| S8 | MCP tools | disc | `DISC-mcp-not-full-suite` |
| S9 | LSP | partial | Navigation surface; not full CLI |
| S10 | Extraction dumps | disc | `DISC-extraction-presence-only` until nz7i.4 |

## MUST clause matrices (ghiw.2)

Clause IDs landed. **Score TBD** after a full run (ghiw.5). Do not claim ≥0.95 MUST%.

| Family | Status | SSoT | Tests |
|---|---|---|---|
| QG | covered (parse) | `docs/QUERY_GRAMMAR.md` | `query::tests::qg_must_matrix`, `parse_never_panics` |
| MJ | partial | `docs/validation/machine-json-schema.md` | `tests/cli/machine_contracts.rs` (MJ-011/012 not Pass) |
| NL | partial | `docs/validation/negative-ledgers.md` | CLI fail-closed + NL-008 compact; NL-005/007/009 gap |

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
