# Zero Tech Debt Evidence — `cursor/anti-bloat-cleanup-da21`

Hard evidence for deep zero-tech-debt cleanup on the CLI honesty/docs tip.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr25`.
**Note:** `.beads/` was not modified (per task instruction). No new PR opened; this branch only.

---

## Intended end state

CLI honesty/docs branch is lean:

| Concern | Surface |
|---------|---------|
| CLI not a god-file | `machine.rs` / `bench.rs` / `watch.rs` / `search_cmd.rs`; `lib.rs` thin clap dispatch |
| Dead wrappers gone | deleted unused `ast_sgrep_cli::run`; deleted `Searcher::search_regex` / `search_word` |
| Shared pattern/search helpers | lang `signature.rs` + `pattern_queries.rs`; core consumes `cached_pattern_signatures` / `structural_term_signatures`; `cmp_ranked_hits` / poison-tolerant locks |
| Scripts dens reduced | `release-acceptance.mjs` helpers; `check-native-workflow.mjs` shared parse/report |
| Docs honesty intact | QUERY_GRAMMAR, unreproducible-claim rules, MRR fingerprint docs **unchanged** |

---

## Batch A — CLI god-file split (biggest target)

### Before / after `crates/ast-sgrep-cli/src/lib.rs`

| Metric | Before | After |
|--------|--------|-------|
| Lines | **1165** | **462** |
| Decisions (`if`+`while`+`=>`) | 127 | 52 |
| Decisions (`if`+`match`+`while`+`=>`) | 140 | 57 |

### Modules extracted

| Module | Lines | Role |
|--------|-------|------|
| `machine.rs` | 98 | envelopes + `raw_command_name` / failure helpers |
| `bench.rs` | 495 | suite/batch + honesty `cv_pct` / history ratchet / vacuous ast-grep skip |
| `watch.rs` | 84 | incremental watch loop |
| `search_cmd.rs` | 92 | search/chain + `resolve_output_format` |

### Caller verification before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `ast_sgrep_cli::run` / `pub fn run()` | **zero** (entry is `main` → `run_process`) | deleted |
| `Searcher::search_regex` / `search_word` | **zero** (modes via `search("regex:…")` / `search("word:…")`) | deleted |
| `clear_semantic_ivf_session_cache` | only `mark_semantic_ivf_stale` | demoted private |
| `last_identifier_chain` thin wrapper | only self (lang) | deleted; call `last_identifier_in_chain` |

### CLI extracts / gaps

| Helper | Location | Purpose |
|--------|----------|---------|
| `resolve_output_format` | `search_cmd.rs` | one format parse path |
| `index_db_display` | `lib.rs` | shared DB path in open error messages |
| `raw_command_name` | `machine.rs` | added missing `"search"` token for pre-parse envelopes |

### Behavior invariants (honesty tip)

- Machine success envelopes still omit `exit_code` when `ok: true`; failure / `!ok` paths keep `exit_code` (incl. bench suite `print_machine_json_with_ok`).
- Bench still demotes vacuous hybrid/token `speedup_vs_ast_grep` claims; emits `cv_pct` + optional `.bench-history` ratchet.
- `require_compiled_features` / neural+rerank fail-closed preserved.
- Chain still uses `top_n: 1` (this tip’s contract).
- Regex wall-clock budget (`ASGREP_REGEX_BUDGET_MS`) preserved.

---

## Batch B — core / lang shared helpers

### End state

| Surface | Location |
|---------|----------|
| `FUNCTION_QUERY_TABLE` / `CLASS_QUERY_TABLE` / `queries_for` | `crates/ast-sgrep-lang/src/pattern_queries.rs` |
| `classify_native` / `DECL_PATTERN_PREFIXES` / `DECL_KIND_PREFIXES` | `crates/ast-sgrep-lang/src/pattern.rs` (exported) |
| `cached_pattern_signatures` / `required_pattern_literal` / `structural_term_signatures` | `crates/ast-sgrep-lang/src/signature.rs` |
| Shared kind consts | `extract.rs` (`pub(crate)`) |
| Core pattern search | imports lang `cached_pattern_signatures` |
| Hybrid `structural_index_pass` | consumes `structural_term_signatures` (byte-identical keys; score still `SCORE_PATTERN * 0.85`) |
| `cmp_ranked_hits` / `lock_response_cache` / `lock_poison_ok` | `search/mod.rs` |
| `hash_content` / `is_trailing_trivia_line` | `index.rs` |
| `wait_child_deadline` | `pattern.rs` |
| `budget_exhausted` early-return flatten | `search/passes/regex.rs` (budget semantics unchanged) |

### Decision counts

| File | Before (user / full) | After (user / full) |
|------|----------------------|---------------------|
| `ast-sgrep-lang/src/pattern.rs` | 81 / 113 (578L) | 56 / 63 (526L) |
| pattern + `pattern_queries` + `signature` | 113 full | 97 full (tables/signatures moved; control-flow thinner) |
| `ast-sgrep-core/src/pattern.rs` | 38 / 40 (348L) | 29 / 31 (299L) |
| `ast-sgrep-core/src/search/mod.rs` | 53 / 57 (812L) | 53 / 57 (785L) |
| `ast-sgrep-cli/src/lib.rs` | 127 / 140 (1165L) | 52 / 57 (462L) |

### Pi dens (if+else+&&+||+ternary-ish)

| File | Before | After | Δ dens ratio |
|------|--------|-------|--------------|
| `packages/pi/extension/src/runtime.ts` | 190 / 501 (0.379) | 184 / 511 (0.360) | −0.019 |
| `packages/pi/scripts/release-acceptance.mjs` | 124 / 273 (0.454) | 113 / 298 (0.379) | −0.075 |
| `packages/pi/scripts/check-native-workflow.mjs` | 88 / 139 (0.633) | 87 / 134 (0.649) | shared `YAML_PARSE` / `reportPush` |

### Runtime / scripts pinned

| Change | File |
|--------|------|
| `LEGACY_NUMBER_FIELDS`; `isContained`→`pathContained`; `assertVersionTriple` | `runtime.ts` (+ `dist/runtime.js`) |
| `packageSpec` / `requiredFilesFor` / `expectReject` / `COMMANDS` / … | `release-acceptance.mjs` |
| Fail codes / rejection labels | **unchanged** |

---

## Honesty docs preserved

`git diff 24657d8 HEAD` is empty for:

- `docs/QUERY_GRAMMAR.md`
- `README.md` MRR fingerprint claims
- `AGENTS.md` unreproducible-claim rules
- `docs/RELEASING.md` honesty checklist
- `benchmarks/results/baselines.md` canonical fingerprint rows

---

## Commands run

```bash
export PATH="/usr/local/cargo/bin:$PATH" CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER=

cargo test -p ast-sgrep-cli --test machine_contracts
# → 9 passed (incl. bench honesty + neural/rerank fail-closed)

cargo test -p ast-sgrep-lang --lib --test pattern
# → lib: 6 passed; pattern: 5 passed

cargo test -p ast-sgrep-core --lib
# → 21 passed (incl. query grammar surface + regex budget unit)

cargo test -p ast-sgrep-core --test regex_budget
# → 1 passed

cargo test -p ast-sgrep-core --lib search::
# → 4 passed

cargo test -p ast-sgrep-cli --test no_embed_hit_key_parity --test watch_incremental
# → 1 + 1 passed

cargo clippy -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-lang -- -D warnings
# → ok

node packages/pi/scripts/release-acceptance.mjs self-test
# → gate self-test accepted; rejection codes unchanged

node packages/pi/scripts/check-native-workflow.mjs
# → structurally consistent; 19 negative mutations rejected
```

### Observed results

| Suite | Result |
|-------|--------|
| `machine_contracts` | **9 passed** |
| `ast-sgrep-lang --lib` | **6 passed** |
| `ast-sgrep-lang --test pattern` | **5 passed** |
| `ast-sgrep-core --lib` | **21 passed** |
| `regex_budget` | **1 passed** |
| `search::` lib filter | **4 passed** |
| `no_embed_hit_key_parity` | **1 passed** |
| `watch_incremental` | **1 passed** |
| clippy `-D warnings` (libs) | **ok** |
| pi release-acceptance self-test | **accepted** |
| check-native-workflow | **ok** |

---

## Thin-wrapper audit (final)

| Symbol | Callers | Action |
|--------|---------|--------|
| `search_regex` / `search_word` | 0 | deleted |
| `ast_sgrep_cli::run` | 0 | deleted |
| `clear_semantic_ivf_session_cache` | 1 | private |
| `last_identifier_chain` | 1 | deleted |
| Core local signature classifiers | duplicated | deleted; use lang exports |
| CLI clap `parse_*` value parsers | 1 each | **kept** (clap requires named fn) |

---

## Out of scope / preserved on this tip

- QUERY_GRAMMAR honesty, unreproducible-claim rules, MRR fingerprint docs.
- Regex budget fail-closed path.
- Bench single-envelope + vacuous speedup demotion.
- Neural/rerank feature fail-closed.
- `.beads/` untouched.
- No new PR; branch `cursor/anti-bloat-cleanup-da21` pushed only.
