# EPIC evidence (PR #25 / `cursor/anti-bloat-cleanup-da21`)

Hard evidence for beads under epics `ast-sgrep-56w1`, `ast-sgrep-docs-honesty-v3pq`, and `ast-sgrep-qpee`.
Parent owns `.beads/issues.jsonl` updates; this file is the code/docs/test trail.

Verify with `export PATH="/usr/local/cargo/bin:$PATH"` from `/workspace/.worktrees/pr25`.

---

## `ast-sgrep-56w1.1` / `ast-sgrep-docs-honesty-v3pq.1` — QUERY_GRAMMAR matches `ParsedQuery::parse`

**Changed**
- `docs/QUERY_GRAMMAR.md:1-40` — normative single-prefix surface; demotes AND/`path:`/`lang:`/`sem:` fiction
- `crates/ast-sgrep-core/src/query.rs:183-220` — `parse_prefix_surface_matches_query_grammar_doc` locks prefixes to real modes and asserts fiction falls through to Hybrid
- Docs cleaned of composable-grammar / “pattern→ast-grep only” myths: `docs/getting-started.md`, `docs/how-it-works.md`, `docs/comparison.md`, `docs/use-cases.md`, `docs/mcp.md`, `README.md`

**Verify**
```bash
cargo test -p ast-sgrep-core --lib query::tests::parse_prefix_surface_matches_query_grammar_doc -j2
rg -n 'AND / `path:`|no composable' docs/QUERY_GRAMMAR.md
```

---

## `ast-sgrep-56w1.2` — `suggested_next` only executable asgrep commands

**Changed**
- `crates/ast-sgrep-plugins/src/lib.rs:88-108` — removed `pattern:` / `rg` myths; emit `asgrep …` commands only
- `crates/ast-sgrep-plugins/tests/capsule_format.rs:73-93` — `agent_suggested_next_is_executable_asgrep_only`
- `docs/use-cases.md` agent JSON example aligned

**Verify**
```bash
cargo test -p ast-sgrep-plugins --test capsule_format agent_suggested_next_is_executable_asgrep_only -j2
```

---

## `ast-sgrep-docs-honesty-v3pq.2` — Tantivy branded as secondary FTS5

**Changed**
- `docs/ARCHITECTURE.md:44`, `:53` — secondary SQLite FTS5 (`lexical.db`); no Tantivy crate
- `docs/how-it-works.md` sidecar table / lexical notes
- `docs/getting-started.md` `--tantivy` described as historical flag name
- `crates/ast-sgrep-cli/src/agent.rs` `feature_gated_flags["--tantivy"]` + capabilities golden

**Verify**
```bash
rg -n 'secondary SQLite FTS5|flag name is historical' docs/ARCHITECTURE.md docs/getting-started.md
cargo test -p ast-sgrep-cli --test machine_contracts capabilities_and_version_match_goldens -j2
```

---

## `ast-sgrep-docs-honesty-v3pq.3` — `--neural-embed` / `--rerank` fail closed

**Changed**
- `crates/ast-sgrep-cli/src/lib.rs:332-352` — `require_compiled_features`
- `crates/ast-sgrep-core/src/search/mod.rs:417` — `validate_search_feature_flags` in `Searcher::new`
- `crates/ast-sgrep-core/src/index.rs` — `Indexer::new` rejects `EmbedBackend::Neural` without feature
- `crates/ast-sgrep-cli/tests/machine_contracts.rs` — `neural_embed_and_rerank_fail_closed_without_features`
- `crates/ast-sgrep-core/tests/parity.rs` — wiring test no longer silently opens a searcher with unavailable features

**Verify**
```bash
cargo test -p ast-sgrep-cli --test machine_contracts neural_embed_and_rerank_fail_closed_without_features -j2
cargo test -p ast-sgrep-core --test parity parity_search_option_wiring -j2
# Manual: ./target/debug/asgrep --json --neural-embed status .  # ok:false exit 2 without feature
```

---

## `ast-sgrep-docs-honesty-v3pq.4` — bake-off / architecture knobs

**Changed**
- `benchmarks/results/bakeoff.md`, `head-to-head.md`, `losses.md` — removed phantom `ASGREP_RERANK_WEIGHT`; document real `ASGREP_RERANK_TOP_K` and `=1` env knobs
- `docs/ARCHITECTURE.md:71-73` — supervisor is SIGSTOP/CONT duty-cycle, not a diagnostic-only wrapper
- `docs/how-it-works.md` — C# uses tree-sitter (no false regex-fallback claim)

**Verify**
```bash
rg -n 'ASGREP_RERANK_WEIGHT' benchmarks/results/ | head
# Mentions should only be "does not exist" clarifications
rg -n 'SIGSTOP|duty cycle' docs/ARCHITECTURE.md
```

---

## `ast-sgrep-docs-honesty-v3pq.5` — truthful surface-equivalence gate name

**Changed**
- Renamed `crates/ast-sgrep-cli/tests/surface_equivalence.rs` → `tests/no_embed_hit_key_parity.rs`
- Test: `no_embed_hit_key_order_parity_across_cli_core_lsp` documents `--no-embed` hit-key order only
- `crates/ast-sgrep-testkit/src/index.rs` HitKey comment updated

**Verify**
```bash
cargo test -p ast-sgrep-cli --test no_embed_hit_key_parity -j2
```

---

## `ast-sgrep-qpee.1` — demote vacuous `speedup_vs_ast_grep`

**Changed**
- `crates/ast-sgrep-cli/src/lib.rs:658-688` — `ast_grep_comparison`: only `pattern:` queries; otherwise `compared:false` + `skipped_reason`
- Bench JSON no longer emits top-level `speedup_vs_ast_grep` for hybrid/token queries
- `machine_contracts` test `bench_json_emits_cv_pct_and_skips_vacuous_ast_grep_speedup`

**Verify**
```bash
cargo test -p ast-sgrep-cli --test machine_contracts bench_json_emits_cv_pct_and_skips_vacuous_ast_grep_speedup -j2
```

---

## `ast-sgrep-qpee.2` — `.bench-history` ratchet + `cv_pct`

**Changed**
- `crates/ast-sgrep-cli/src/lib.rs:638-750` — `cv_pct`, `update_bench_history`, optional `ASGREP_BENCH_RATCHET=1` (50% regression tripwire)
- Default history path `.bench-history.json` (gitignored); override `ASGREP_BENCH_HISTORY_PATH`; disable `ASGREP_BENCH_HISTORY=0`
- `docs/benchmarks.md` documents the contract

**Verify**
```bash
cargo test -p ast-sgrep-cli --test machine_contracts bench_json_emits_cv_pct_and_skips_vacuous_ast_grep_speedup -j2
# Emits cv_pct + writes history under ASGREP_BENCH_HISTORY_PATH
```

---

## `ast-sgrep-qpee.3` — `bench --suite --json` single envelope

**Changed**
- `crates/ast-sgrep-cli/src/lib.rs:307-318`, `:876-880` — `print_machine_json_with_ok`; suite prints one object with `ok`/`suite_ok`, then `exit(2)` on failure (no success-then-failure dual JSON)
- Test: `bench_suite_json_is_single_envelope_even_on_failure` (stdout must parse as one JSON value)

**Verify**
```bash
cargo test -p ast-sgrep-cli --test machine_contracts bench_suite_json_is_single_envelope_even_on_failure -j2
```

---

## `ast-sgrep-56w1.3` — regex wall-clock budget

**Changed**
- `crates/ast-sgrep-core/src/search/passes/regex.rs:14-92` — default 2000ms budget (`ASGREP_REGEX_BUDGET_MS`); between-line deadline; error discards partials
- `crates/ast-sgrep-core/tests/regex_budget.rs` — zero-budget fail-closed integration test

**Verify**
```bash
cargo test -p ast-sgrep-core --test regex_budget -j2 -- --test-threads=1
cargo test -p ast-sgrep-core --lib search::passes::regex -j2
```

---

## Focused test batch (ran)

```bash
export PATH="/usr/local/cargo/bin:$PATH"
cargo test -p ast-sgrep-core --lib query::tests -j2 -- --test-threads=1
cargo test -p ast-sgrep-core --test parity -j2 -- --test-threads=1
cargo test -p ast-sgrep-core --test regex_budget -j2 -- --test-threads=1
cargo test -p ast-sgrep-plugins --test capsule_format -j2 -- --test-threads=1
cargo test -p ast-sgrep-cli --test machine_contracts --test no_embed_hit_key_parity -j2 -- --test-threads=1
```
