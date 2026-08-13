# Compact agent output validation

## Contract

`asgrep --json --format compact` preserves every ranked result's path, line
span, kind, signal, and primary symbol while removing duplicate paths and
nonessential prose. Stable IDs are derived from a deterministic FNV-1a path
hash plus the exact line range. Hash collisions are detected and assigned a
deterministic suffix before rows are emitted.

Snippet ceilings use conservative token units: one UTF-8 byte per unit. A
byte-fallback tokenizer can always represent a byte with at most one token, so
this ceiling cannot understate its token count. UTF-8 truncation moves to the
previous character boundary and never emits invalid text. Rank metadata stays
available after the aggregate snippet budget is exhausted.

## Fixed-query measurement

The targeted `ast-sgrep-plugins` format harness uses three fixed queries and
six ranked results with realistic 40-line function excerpts. It decodes each
compact ID through the path dictionary and asserts ordered identity equality
against the native response before measuring minified serialized payloads.

| Format | Conservative token units per result | Relative reduction |
|--------|------------------------------------:|-------------------:|
| Native JSON | 1,934.0 | baseline |
| Compact JSON | 213.5 | 89.0% |

The measured reduction exceeds the 50% acceptance threshold while retaining
100% of ranked task identities. The metric is deliberately labeled
conservative token units, not vendor-specific tokenizer output.

## MCP surface adoption (kxmc)

The MCP server previously rendered `OutputFormat::AgentCapsule` through
`serde_json::to_string_pretty`, so the compact reduction above did not reach
agents at all. Agent search now emits the compact envelope minified.

Fixture: ten hits across three files with realistic Rust excerpts, in
`compact_minified_is_much_smaller_than_pretty_capsule`. The same test asserts
that minified compact output is smaller than the previous pretty capsule and
that each distinct path appears exactly once in the payload. The previous
capsule repeated paths in both `file` and `ref`.

```bash
cargo test -p ast-sgrep-plugins --test capsule_format -- --nocapture compact_minified
```

## Key ordering (9q0l)

`serde_json` sorts object keys alphabetically, so key names determine wire
order. Per-call accounting is therefore named `zb` (budget), `zn` (hit count),
and `zt` (truncation count) so it sorts after the content keys `h`, `p`, `q`,
and `v`. Envelopes have a stable head and a volatile tail, and repeated
identical searches over an unchanged index are byte-stable
(`search_envelope_is_byte_stable_with_volatile_accounting_last`).

MCP `tools/list` is byte-identical across calls and across processes
(`tools_list_is_byte_identical_across_calls_and_processes`). Tool definitions
enter the prompt on every request, so this is the largest cacheable region the
server controls.

## Reproduction

All Rust compilation and tests run through the remote compilation helper:

```bash
RCH_VISIBILITY=summary RCH_PRIORITY=high rch exec -- \
  env RUSTC_WRAPPER= CARGO_TARGET_DIR=/tmp/rch_target_ast_sgrep_compact \
  cargo test --locked -p ast-sgrep-plugins --test capsule_format -- --nocapture

RCH_VISIBILITY=summary RCH_PRIORITY=high rch exec -- \
  env RUSTC_WRAPPER= CARGO_TARGET_DIR=/tmp/rch_target_ast_sgrep_compact_cli \
  cargo test --locked -p ast-sgrep-cli --test cli_smoke --test machine_contracts
```

The CLI contract target also verifies compact output is a single minified JSON
line and that user-supplied per-result and aggregate limits are enforced.
