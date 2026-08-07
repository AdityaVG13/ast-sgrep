# ast-sgrep cargo-fuzz program

Workspace-excluded (`Cargo.toml` `exclude = ["fuzz"]`) so product crates never
pull `libfuzzer-sys` / fuzz-only deps into normal builds.

## Targets

| Bin | Surface | Oracle |
|-----|---------|--------|
| `query_grammar` | `ParsedQuery::parse` | structural mode/target/raw + reparse |
| `rank` | `score_symbol` / `fuse_rrf` | finite/range + reverse-RRF |
| `lang_parse` | `ParserRegistry::parse` | no panic; OnceLock registry |
| `classify_native` | `classify_native` + fallback consistency | no panic + consistency |
| `ann_clusters` | `SemanticAnnIndex::read_clusters_bounded` | crash + write/read RT |
| `embed_roundtrip` | `embed_from_bytes` / `embed_to_bytes` | round-trip |
| `lsp_frame` | `read_message` over `Cursor` | panic-free framing (≤64 KiB) |
| `codemode_serve` | `ServeRequest` / `BatchRequest` serde | panic-free JSON parse |

Security motivation: tree-sitter C + dual pattern×source (native targets);
binary OOB/magic/length (ANN/embed); URI escape + framing DoS (wire).

## Quick start

```bash
cargo install cargo-fuzz --locked   # once
cd fuzz
bash scripts/sync_seeds.sh
cargo +nightly fuzz run query_grammar -- -max_total_time=30 -timeout=5 \
  -dict=dictionaries/query_grammar.dict
cargo +nightly fuzz run rank -- -max_total_time=30 -timeout=5
```

List bins: `cargo +nightly fuzz list`

## L1 seeds vs evolved corpus

- **Committed L1 seeds:** `seed_corpus/<target>/` (≥5 files where required).
- **Evolved corpus:** `corpus/<target>/` (gitignored). Sync with
  `scripts/sync_seeds.sh` before CI/local runs (`cp -n` so evolved inputs stay).

## Dictionaries

- `dictionaries/query_grammar.dict` — mode prefixes (`callers:`, `defs:`, …).
- `dictionaries/lang_source.dict` — common syntax tokens for native parse.
- `dictionaries/lsp_frame.dict` — `Content-Length` framing tokens.

Pass via libFuzzer: `-dict=dictionaries/<name>.dict`.

## Sanitizer smoke (ASan + UBSan)

cargo-fuzz enables ASan by default. For ASan+UBSan local/nightly smoke:

```bash
cd fuzz
bash scripts/sync_seeds.sh
# Default ASan campaign (baseline):
cargo +nightly fuzz run query_grammar -- -max_total_time=30 -timeout=5
# Optional UBSan-focused rebuild when investigating integer/UB issues
# (separate campaign; do not invent exec/s numbers — see PASS2 for floors):
# RUSTFLAGS="-Zsanitizer=undefined" cargo +nightly fuzz run query_grammar -- ...
```

MSan is for tree-sitter C / mmap-adjacent targets later (full dep rebuild
required). TSan is program-level P3 (unit bit-identical oracles already cover
kmeans thread parity).

## Coverage plateau ladder (PASS5 §5)

When edge discovery flattens for 30–120 minutes on a baseline bin:

1. Expand L1 seeds + keep size guards.
2. Expand dict + run with `-use_value_profile=1`.
3. Optional offline CMPLOG/AFL++ (docs-only; not required in CI).
4. Structure-aware / Arbitrary upgrade for multi-field inputs.
5. Accept saturation and invest in **breadth** (new targets) over longer runs.

## Crash triage → regression

1. **Minimize:** `cargo +nightly fuzz tmin <target> artifacts/<target>/crash-*`
2. **Reproduce** minimized input 10× (must be deterministic).
3. **Dedup** by top-5 stack frames (not by crash filename).
4. **Regression fixture:** commit minimized bytes under
   `tests/fuzz_regressions/<target>/crash_<short_hash>.bin` (or `.txt`)
   and a unit/integration test that feeds the bytes into the **same pure API**
   the harness calls (must not panic after the fix).
5. **Re-fuzz** the target so deeper bugs surface.

Example regression skeleton (product test, not in this package):

```rust
#[test]
fn regression_fuzz_query_grammar_abc123() {
    let input = include_str!("../fuzz_regressions/query_grammar/crash_abc123.txt");
    let _ = ast_sgrep_core::ParsedQuery::parse(input);
}
```

## Corpus minimize / regen

```bash
cd fuzz
bash scripts/cmin_all.sh          # cargo fuzz cmin per target
bash scripts/sync_seeds.sh       # re-seed L1 after wiping corpus
```

Regenerate tiny valid binary seeds for ANN/embed by re-running unit builders
or extending `scripts/gen_seed_corpus.sh` if present.

## Prod dependency isolation

After any `fuzz/Cargo.toml` or product feature change:

```bash
cargo tree -p ast-sgrep-core --no-dev | grep -E 'libfuzzer|arbitrary|bolero' || true
# must print nothing
```

Fuzz stays in the excluded `fuzz/` package; never add libfuzzer to product
crates' normal dependencies.

## CI / release gate

- `.github/workflows/ci.yml` `bounded-fuzz` job (workflow_dispatch): real bins
  only (`query_grammar`, `rank`), seeds synced first.
- `scripts/local-release-gate.sh`: both baseline bins, 30s each.

PR-tier continuous fuzz is optional/short; deep campaigns stay dispatch/nightly.
