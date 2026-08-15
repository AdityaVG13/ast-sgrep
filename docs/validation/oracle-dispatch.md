# Oracle dispatch (WP4)

**Pass 1 Q1:** For each search channel, which oracle is authoritative, and which
comparators are *never* correctness?

This file is the router. Pattern×ast-grep Pattern-1 is
`tests/core/pattern_diff.rs` (env-gated). jell and MUST matrices (`ghiw.2`)
stay separate. DISC ids come from
[`DISCREPANCIES.md`](DISCREPANCIES.md). Machine copy:
[`docs/contracts/oracle_dispatch.toml`](../contracts/oracle_dispatch.toml).

`gate_class`:

| Class | Meaning |
|---|---|
| `correctness` | Fail = product contract broken |
| `local_correctness` | Explicit local dependency; Fail when configured, Not-run otherwise |
| `peer_parity` | Same process, two APIs; not an external tool |
| `latency_only` | Timing / keep-gate; **never** a hit-identity oracle |
| `never_correctness` | Explicitly not allowed as a Pass for answers |
| `deferred_excluded` | Not-run. Must not be reported as Pass |

Subject is always this tree (`asgrep` / `ast-sgrep-*`). Oracle IDs name the
*authority*, not a second binary unless stated.

## Dispatch table

| Channel | Scenario | authoritative_mode | subject_id | oracle_id | comparator | disc_ids | suite_path | gate_class |
|---|---|---|---|---|---|---|---|---|
| lexical | keyword / FTS hits | fixture | `asgrep` | `tests/core/parity.rs` + FTS contract | must_include / hit keys | `DISC-lexical-not-rg` | `tests/core/parity.rs` | `correctness` |
| lexical | vs ripgrep identity | excluded | `asgrep` | `rg` | hit-ID equality | `DISC-lexical-not-rg`, `DISC-no-jell-harness` | `docs/validation/jell-deferral.md` | `deferred_excluded` |
| graph | defs / callers / imports | fixture | `asgrep` | `tests/fixtures` graph cases | expected edges / symbols | | `tests/core/graph_oracle.rs` | `correctness` |
| structural-native | `pattern:` indexed subset | spec+fixture | `asgrep` | `docs/structural-patterns.md` | supported shapes hit; unsupported empty | `DISC-pattern-native-subset` | `crates/ast-sgrep-lang` pattern tests | `correctness` |
| structural-native | vs ast-grep CLI | pinned local Pattern-1 | `asgrep` | ast-grep 0.45.1 | match-set differential | `DISC-pattern-native-subset` | `tests/core/pattern_diff.rs` | `local_correctness`; Not-run until `ASGREP_DIFF_AST_GREP` |
| semantic/ANN | cosine / IVF adaptive | math+spec | `asgrep` | `ast-sgrep-embed` math + IVF docs | unit math; threshold honesty | `DISC-ivf-adaptive-threshold` | `ast-sgrep-embed` `math::` | `correctness` |
| semantic/ANN | published MRR | ledger | `asgrep` | `benchmarks/results/baselines.md` | provenance only | `DISC-baselines-unreproducible` | `benchmarks/results/baselines.md` | `never_correctness` |
| hybrid/NL | ranking must_include | fixture | `asgrep` | `tests/fixtures/ranking/cases.json` | must_include bag (not gold ranks) | `DISC-ranking-soft-oracle`, `DISC-casefold-ascii` | `tests/core/ranking_oracle.rs` | `correctness` |
| hybrid/NL | competitor bake-off scores | ledger | `asgrep` | UNREPRODUCIBLE results docs | none in-tree | `DISC-baselines-unreproducible` | `benchmarks/results/` | `never_correctness` |
| machine JSON | CLI envelopes | fixture+golden | `asgrep` | `tests/cli/machine_contracts.rs` + goldens | schema / golden JSON | `DISC-compact-drops-provenance` | `tests/cli/machine_contracts.rs` | `correctness` |
| machine JSON | MCP protocol | peer | `asgrep-mcp` | CLI/core contracts (no auto-fusion) | protocol shapes | `DISC-mcp-not-full-suite` | `tests/mcp` protocol | `peer_parity` |
| fail-closed | missing root / empty index / SSRF | spec | `asgrep` | `docs/validation/negative-ledgers.md` | must error, not empty hits | | product fail-closed table | `correctness` |
| keep-gate | search latency | history | `asgrep bench` | `.bench-history/*.latest.json` | −3%/−5% + cv quarantine | | `scripts/check-bench-output.py` | `latency_only` |
| forbid-soundness | first-party unsafe | policy | workspace | `scripts/verify-forbid-soundness` | exit 0 | | `scripts/verify-forbid-soundness` | `correctness` |
| jell | cross-engine hit IDs | excluded | `asgrep` | `rg` + `ast-grep` | identical hit IDs | `DISC-no-jell-harness` | `docs/validation/jell-deferral.md` | `deferred_excluded` |

## Proof-pack coverage

Every command in `docs/validation/proof-pack.md` maps here:

| Proof-pack command | Dispatch row |
|---|---|
| `scripts/verify-forbid-soundness` | forbid-soundness |
| `ranking_oracle` | hybrid/NL ranking must_include |
| `graph_oracle` | graph defs/callers/imports |
| `machine_contracts` | machine JSON CLI envelopes |
| `ast-sgrep-mcp --test protocol` | machine JSON MCP protocol |
| `ast-sgrep-embed --lib math::` | semantic/ANN math |

Keep-gate / speed.yml is **latency_only** and is not in the proof-pack command
list on purpose: it must not be cited as ranking correctness.

## Explicit non-ownership

Pattern×ast-grep match-set differential is **ghiw.3**. Its bounded equality
list is a pinned local gate; full ast-grep CLI parity remains outside the
native subset contract.
