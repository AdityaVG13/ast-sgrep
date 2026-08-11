# RESULT — Pass 2 / Loop 2 (repository-census-and-scope)

```text
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 2
loop: 2
coverage_pending: architecture+trust+contracts rings
high_critical_without_loop27: 0
fresh_commands_loop28: n/a
residual_risk: dirty tree continues; HEAD 1 commit past freeze; ZeroStack engines still missing; coverage/mutation/formal unavailable
books: /Users/aditya/Developer/ast-sgrep/.rotational-code-analysis/iterations/02-census/
queue_action: none
braid_resolve: Continue
axes_changed: 4
axes: scale:repository→file | representation:filesystem | observer:maintainer | evidence:source
void_fixture_outcome: n/a mid-wave
north_star_probe_outcome: n/a
independent_loop27: n/a
baseline_cmd: git ls-tree -r --name-only fb932aac852f5496c0a7035cc5a0b508e05111cb
candidate_cmd: (same freeze; no re-baseline)
frozen_revision: fb932aac852f5496c0a7035cc5a0b508e05111cb
dirty: true
```

## Gate (loop 2)

- [x] Prior freeze loaded (not re-baselined)
- [x] Exclusion ledger with concrete reasons (tracked + disk)
- [x] In-scope product census by language/kind/module
- [x] Polyglot capability matrix (schema shape OK)
- [x] Stable module IDs + shard plan
- [x] Pass1 residual B-SNAPSHOT-NOISE addressed (target-pass* excluded with counts)
- [x] B-DIRTY-FREEZE re-confirmed (still dirty; HEAD drift noted)

## Census headline

| Metric | Value |
|---|---:|
| Tracked at freeze | 585 |
| In-scope | 523 |
| Tracked excluded | 62 |
| Disk-only excluded (approx) | ~91860 |
| Pass1 spin in_scope (superseded) | 6313 |

### In-scope by language (top)

| Language | Files |
|---|---:|
| Rust | 158 |
| Markdown | 133 |
| noext (seeds/scripts/LICENSE) | 64 |
| JSON | 52 |
| TypeScript | 29 |
| JavaScript | 20 |
| TOML | 14 |
| Shell | 8 |
| YAML | 7 |
| Python | 7 |
| fixture polyglot (Go/Java/…) | ≤3 each |

### Capability summary

| Ecosystem | Status | Parse/meta | Typecheck | Test | Static | Fuzz | Coverage | Mutation | Formal |
|---|---|---|---|---|---|---|---|---|---|
| Rust workspace | CONFIRMED | cargo metadata | cargo check/build | cargo test (+39 core itests) | clippy/fmt/forbid/audit | cargo-fuzz (8 targets) | UNAVAIL | UNAVAIL | UNAVAIL (optional miri) |
| TS/JS Pi packages | CONFIRMED | package.json | tsc | node --test + e2e gates | check:pi-contract/release | UNAVAIL | UNAVAIL | UNAVAIL | UNAVAIL |
| Python scripts | CONFIRMED | — | — | script tests | — | — | — | — | — |
| Fixture polyglot | CONFIRMED | tree-sitter via lang | n/a product | goldens/fuzz | — | lang_parse | — | — | — |

## Residual → Pass 3 (architecture-dependency-and-ownership-map)

Axes expected: scale file→module; representation graph/deps; observer architect; evidence source+metadata.

Must consume: shard plan module IDs; Cargo/npm edge facts; BND-* boundaries; unsafe islands M-mmap + M-codemode-napi.
