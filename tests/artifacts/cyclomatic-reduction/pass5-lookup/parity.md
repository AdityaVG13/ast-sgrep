# Parity report — Pass 5 (lookup_table)

## Commands run

| Check | Command | Result |
|---|---|---|
| Core compile | `cargo check -p ast-sgrep-core` | **green** |
| Literal integration | `cargo test -p ast-sgrep-core --test literal_glob --test pattern_prefilter --test chain_case` | **7 passed** |
| Extension suite | `cd packages/pi/extension && npm test` | **88 passed** |
| argvFor differential | node one-shot: 12 tool/arg cases vs golden argv | **PASS** |

## Pre-existing (not pass-5)

`cargo test -p ast-sgrep-core --lib` fails on `SearchHit.resolution` / `SearchResponse` fixture drift (fusion.rs, search/mod.rs tests) — same class noted after pass 3/4. Unrelated to SQL template table.

## Behavior preserved

- argvFor: default/unknown → capsule query; catalog → status placeholder; force true/false → reindex/index.
- searchToolCall: all SearchMode keys; prefix encoding `mode: query`; chain top_n: 20; capsule format.
- literal_sql: LIKE ESCAPE vs GLOB pattern escaping; lang `?3` bind when present; word_mode postfilter.
