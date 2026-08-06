# External differential harness (`jell`) — honest deferral

A full cross-engine differential harness (asgrep vs ripgrep vs ast-grep CLI on
shared corpora with identical hit IDs) is **deferred**. This tree ships:

- Ranking oracle: `crates/ast-sgrep-core/tests/ranking_oracle.rs` + `tests/fixtures/ranking/cases.json`
- Graph oracle: `crates/ast-sgrep-core/tests/graph_oracle.rs`
- Parity suite: `crates/ast-sgrep-core/tests/parity.rs`

What is intentionally **not** claimed: bit-identical result sets versus
external tools. Structural patterns are a native subset (see
`docs/structural-patterns.md`); lexical modes are FTS-backed, not rg-compatible.

Proof pack entry: `docs/validation/proof-pack.md`.
