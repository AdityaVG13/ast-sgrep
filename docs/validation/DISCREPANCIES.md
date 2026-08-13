# Intentional discrepancies (DISC)

Registered divergences from a naive "we match ast-grep / rg / a full MCP
suite" reading. Green tests do **not** claim these surfaces. XFAIL / ignore
is allowed only with a DISC id (see `conformance-verdicts.md`).

Claim classes (do not mix):

| Class | Meaning |
|---|---|
| Product contract | What this tree ships and tests |
| Peer parity | Same process, two APIs (CLI vs MCP vs LSP) |
| External oracle | ast-grep CLI, ripgrep, jell -- **not** claimed here |

## Seed register

| ID | Surface | Intentional divergence | Evidence | Test / XFAIL posture |
|---|---|---|---|---|
| `DISC-pattern-native-subset` | `pattern:` | Native tree-sitter + indexed signatures only. Nested templates, YAML rules, rewrites, and relational metavars return no hits or fail-closed. **No silent ast-grep subprocess** (search does not walk PATH; `ASGREP_ALLOW_AST_GREP` is bench-only). | `docs/structural-patterns.md`, `tests/core/pattern_diff.rs`, `crates/ast-sgrep-core/src/pattern.rs` `find_ast_grep_binary` | Fail if a test assumes ast-grep CLI identity without `ASGREP_DIFF_AST_GREP`. Ignore Pattern-1 equality only with this DISC. |
| `DISC-no-jell-harness` | External differential | Cross-engine hit-ID bake-off (asgrep vs rg vs ast-grep) is deferred. | `docs/validation/jell-deferral.md` | Not-run. Never Pass. |
| `DISC-lexical-not-rg` | Keyword / FTS | Lexical modes are FTS-backed, not ripgrep-compatible result sets. | `docs/validation/jell-deferral.md` | Fail if a test asserts rg identity. |
| `DISC-compact-drops-provenance` | `--format compact` | Compact rows keep path, span, kind, signal, symbol. They drop duplicate paths and nonessential prose / full provenance blobs. | `docs/validation/compact-output.md` | Assert identity of ranked task keys, not native JSON equality. |
| `DISC-casefold-ascii` | Ranking / search | ASCII case-fold only; not Unicode casemapping. | `docs/validation/issue-12-senpi.md` | Fail on ASCII mismatch. Unicode fold is out of contract. |
| `DISC-ranking-soft-oracle` | Ranking fixture | `tests/fixtures/ranking/cases.json` is a must_include bag, not a gold rank vector or MRR. | `tests/core/ranking_oracle.rs` | Panic on missing must_include. Do not treat as external bake-off. |
| `DISC-extraction-presence-only` | Lang extraction | Presence tuples / kind checks, not full dump goldens (those are nz7i.4). | `crates/ast-sgrep-testkit/src/lang.rs` | Fail on missing expected symbol. Not a dump freeze. |
| `DISC-mcp-not-full-suite` | MCP | MCP does not auto-fuse hybrid channels; not a full CLI clone. | `docs/validation/surface-parity.md` | Peer-parity tests only. |
| `DISC-ivf-adaptive-threshold` | ANN | IVF/ANN only above `chunk_count` threshold; small corpora stay brute cosine. | `docs/validation/semantic-ivf-mmap.md` | Do not claim ANN on sample fixtures. |
| `DISC-baselines-unreproducible` | Published benches | Quality fingerprints stay UNREPRODUCIBLE until gold+eval is in-tree. Latency 2026-08-05 self-corpus rows are `reproducible-in-tree`. File-level banners must not override section tags. | `benchmarks/README.md`, `benchmarks/results/baselines.md` | Not-run ≠ Pass. Never invent replacement numbers. |

## Verdict conventions

See `docs/validation/conformance-verdicts.md`.
