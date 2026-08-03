# DOWNSTREAM_EVIDENCE — PR #21 (`test/quality-batch-e2hc-19-oxbj`)

Hard evidence for still-open beads closed on this branch tip.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr21`.
**Note:** `.beads/issues.jsonl` was not modified (per task instruction).

---

## Agent / CLI surface

| Bead | Evidence |
|------|----------|
| `ast-sgrep-p2v7` | `crates/ast-sgrep-cli/src/agent.rs` `capabilities_json` + `clap_catalog` derive commands/flags from `Cli::command()`. Golden `crates/ast-sgrep-cli/tests/fixtures/capabilities.json`. Test: `cargo test -p ast-sgrep-cli --test machine_contracts capabilities_lists_all_clap_subcommands_and_siblings` + `capabilities_and_version_match_goldens` |
| `ast-sgrep-vdqo` | Search-tuning moved to `SearchTuning` (non-global); flattened on search/index/bench. `capabilities --help` omits `--ann-probes`/`--rerank` (asserted in machine_contracts). Parent+subcommand merge via `Cli::active_tuning()` |
| `ast-sgrep-d7xh` | `sibling_binaries` + `integrations` in capabilities; root `after_help` mentions `asgrep-mcp`/`asgrep-lsp`. Asserted in `capabilities_lists_all_clap_subcommands_and_siblings` |
| `ast-sgrep-hceb` | robot-docs guide points agents to clap-derived `capabilities --json`; after_help triad aligned. `print_robot_guide` + capabilities catalog share command set |
| `ast-sgrep-xgzd` | `--format` implies machine JSON via `search_machine_output`; help strings on Commands/`SearchTuning`. Test: `format_alone_implies_json_machine_output` |
| `ast-sgrep-j0mj` | `README.md` Easy start (agents) leads with capabilities / robot-docs / doctor triad |
| `ast-sgrep-frkv` | `doctor_triage_json` `suggested_commands` interpolate `root.display()`. Test: `doctor_suggested_commands_echo_effective_root` |
| `ast-sgrep-cg6z` | Documented in capabilities `root_specification` + robot-docs; `ensure_unambiguous_root` still errors on `--root`+positional conflict; `effective_root` single resolver |
| `ast-sgrep-k5gq` | `IndexStore::open` / CLI `open_indexer`/`open_searcher` wrap errors with resolved index path + root |
| `ast-sgrep-0fg6` | `Searcher::new` canonicalize fail-closed (missing/non-dir → error); MCP `sandbox_root` already canonicalizes |
| `ast-sgrep-ei0i` | Shared `clamp_output_limit` / `clamp_agent_limit` remap 0→default and cap. Test: `limits::tests::clamps_to_hard_ceiling` |
| `ast-sgrep-arye` | Edit-distance ≤2 in `query_looks_like_subcommand_typo` (+ search-safe false-friend list). Test: `edit_distance_two_typos_are_rejected_before_search` |

## MCP

| Bead | Evidence |
|------|----------|
| `ast-sgrep-bix3` | `searcher_for` takes Searcher out of cache; mutex dropped before compute; `restore_searcher` after; poison clears cache. `cargo test -p ast-sgrep-mcp --test protocol` → 9 passed |
| `ast-sgrep-mwwu` | `tool_index_repo` sets `embed_semantic: self.use_embed` (honors `ASGREP_NO_EMBED`) |

## Search / ranking / cache / ops

| Bead | Evidence |
|------|----------|
| `ast-sgrep-50hx` | Hybrid Literal intent runs `literal_pass` on stripped quotes. Test: `pattern_routing::hybrid_quoted_literal_intent_hits_phrase_line` |
| `ast-sgrep-8mb8` | Pre-truncate keeps `keep*4` with coverage in sort key; multi-term coverage-first ranking. Test: `search::tests::pretruncate_keeps_high_coverage_lower_score` |
| `ast-sgrep-92nj` | `search_pattern` unions index signatures + native (no early-return). Tests: `pattern_routing::*`; honesty note in code (native-only, no default ast-grep spawn) |
| `ast-sgrep-hhca` | `excerpt_term_coverage` case policy. Test: `search::tests::excerpt_coverage_respects_term_casing` |
| `ast-sgrep-9gfx` | `chain_kinds(Cloud)=[Cloud]` only; loud `ASGREP_EMBED_FALLBACK` for hashed Semantic. Tests: `embedder::preference_tests::*` |
| `ast-sgrep-hdwh` | ResponseCache: PRAGMA fail disables cache; re-check gen after compute before insert |
| `ast-sgrep-nyui` | `SearchOptions::cache_identity` included in cache key. Test: `properties::cache_identity_changes_with_options` / `response_cache_isolates_option_identity` |
| `ast-sgrep-fj96` | LRU eviction via `order` VecDeque at `RESPONSE_CACHE_CAP=128` |
| `ast-sgrep-i5ef` | `cache_index_path` requires HOME/XDG_CACHE_HOME. Test: `cache_index_home::use_cache_without_home_fails_closed` |
| `ast-sgrep-rzzp` | Parent owns PG via `CommandExt::process_group(0)`; worker no longer `setpgid`. `cargo test -p ast-sgrep-cli --lib` childguard tests pass |
| `ast-sgrep-a639` | `route_hits` counts all non-empty terms; `init_schema` probes core tables. Test: `properties::single_char_route_hits_not_zeroed` |
| `ast-sgrep-6ulo` | `determinism_loop::fifty_identical_searches_are_byte_stable` → passed |
| `ast-sgrep-7ddb` | `index`/`reindex --dry-run`; stderr progress only when not `--json`; cancel semantics documented in robot-docs + dry-run payload. Test: `index_dry_run_does_not_mutate` |
| `ast-sgrep-e9qc` | Renamed `parity.rs` → `e2e_smoke.rs`; added `tests/pattern_routing.rs` |
| `ast-sgrep-ok49` | Restored `tests/properties.rs` (proptest parse/clamp + store/rank). `cargo test -p ast-sgrep-core --test properties` → 7 passed |

## Pi / CI / packaging docs

| Bead | Evidence |
|------|----------|
| `ast-sgrep-ktog` | Schema modes word/literal/regex/imports covered in `packages/pi/launcher/test/asgrep-search-mode-matrix.test.mjs` + skill/query-guide docs; `tools.test.ts` cases extended. `node --test packages/pi/launcher/test/asgrep-search-mode-matrix.test.mjs` → 3 passed |
| `ast-sgrep-snkc` | `packages/pi/release/targets.json`: both darwin targets use `macos-14` (drops `macos-15-intel`). Cross-smoke validates true single-runner dual-arch build |
| `ast-sgrep-lruy` | `.github/workflows/pi-cross-smoke.yml` reduced to macOS grouped build only (removed failing win32 cargo-xwin path) |
| `ast-sgrep-wldi` | `docs/npm-unscoped-deprecation.md` checklist (npm deprecate requires interactive auth) |
| `ast-sgrep-81pi` | `docs/pi-gallery-tracking.md` TRACKING note + in-repo publish correctness pointers |

---

## Focused commands run

```bash
cargo test -p ast-sgrep-cli --test machine_contracts
cargo test -p ast-sgrep-cli --test cli_smoke --test surface_equivalence
cargo test -p ast-sgrep-cli --lib
cargo test -p ast-sgrep-mcp --test protocol
cargo test -p ast-sgrep-core --lib search::tests
cargo test -p ast-sgrep-core --lib limits::
cargo test -p ast-sgrep-core --test properties --test pattern_routing --test determinism_loop --test cache_index_home --test e2e_smoke
cargo test -p ast-sgrep-embed --lib preference_tests
node --test packages/pi/launcher/test/asgrep-search-mode-matrix.test.mjs
node packages/pi/launcher/test/npm-native-packages.test.mjs
node packages/pi/scripts/check-contract.mjs
```

(`check-native-workflow.mjs` needs Ruby YAML on this image; contract check passed. Workflow YAML edits are validated by content review + targets.json runner change.)
