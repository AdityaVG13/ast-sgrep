# EPIC evidence — PR #21 (`test/quality-batch-e2hc-19-oxbj`)

Hard evidence for P0/P1 beads closed on this branch.
Commands use `PATH="/usr/local/cargo/bin:$PATH"` and focused package filters.
**Do not treat stretch `prl3` / `e2hc` mega-work as in-scope for this PR** (see DEFER).

Primary packaging-CI ownership for remaining release-train follow-ups:
**this PR / HANDOFF on `test/quality-batch-e2hc-19-oxbj` is the primary surface for `packaging-ci-oykd.4`.**

---

## P0 CRITICAL (prior pass — still green)

### `ast-sgrep-0pla` — Float trait / Scored NaN

| Item | Evidence |
|------|----------|
| Finite-domain `Scored::new` | `crates/ast-sgrep-embed/src/math.rs` — rejects NaN/±∞ |
| Eq/Ord coherent | Blank `Eq` + finite-only `Ord` |
| `top_by_similarity` filters non-finite | Same file |
| Unit + property tests | `cargo test -p ast-sgrep-embed --lib math::` → **9 passed** |

### `ast-sgrep-p7l3` / `p7l3.1` — Forbid-soundness

| Item | Evidence |
|------|----------|
| Workspace `unsafe_code = "forbid"` | Root `Cargo.toml` |
| Sealed mmap | `crates/ast-sgrep-mmap` |
| Gate | `bash scripts/verify-forbid-soundness` → **PASSED** |

### `ast-sgrep-s6ze` — Fail-closed agent honesty

Doctor `ok:false`/exit 2 when unhealthy; missing root / empty index fail-closed; boolish envs.
`cargo test -p ast-sgrep-cli --test machine_contracts` → **8 passed**

### `ast-sgrep-sxjc` — Panic/poison integrity

Regex join fail-closed; clear-on-poison caches; MCP/LSP poison recovery.
`lock_clear_on_poison_resets_state` **passed**

### `ast-sgrep-6j65` — MCP agent-surface safety

Sandbox roots; no panic expects; `index_repo` single-flight + 600s deadline (`es7u` / overlaps `k7l8.8`).
`cargo test -p ast-sgrep-mcp --test protocol` → **9 passed**

### `ast-sgrep-j0x4` — Env-trust

Embed URL allowlist; AST_GREP opt-in only; `docs/env-trust.md`.

### `ast-sgrep-eytx` / `kfhh` — Pattern cascade

Production never starts ast-grep by default; regex panic fail-closed (`1o0y` via sxjc).

---

## Must-finish batch (this pass)

### `ast-sgrep-dx4g` — Resource / DoS bounds

| Kid | Claim | Evidence |
|-----|-------|----------|
| `zbpc` | Cap `read_to_string` | `crates/ast-sgrep-core/src/io_bounds.rs` `read_text_capped` / `MAX_INDEX_FILE_BYTES` (64 MiB); used from `index.rs` prepare/index paths. Test: `io_bounds::tests::rejects_oversized_files` **passed** |
| `89er` | Bound META_CACHE + sanitize hit path joins | `search/mod.rs` `META_CACHE_CAP=4096` + eviction; reject absolute/`..`/prefix components before `root.join` |
| `5xf2` | Constrain `ASGREP_LEDGER_PATH` | Absolute, no `..`, must stay under cwd (canonicalize/prefix checks) |
| `gied` | Prevent `par_chunks_mut` panic on `dim=0` | `semantic_ann::flatten_vectors_for_search` errors when `dim==0` and chunks non-empty. Tests: `flatten_bounds_tests::*` **2 passed** |
| `aqyq` | Reduce format alloc | **DEFERRED** — hot-path `format!` cleanup is non-trivial without a measured alloc profile; no silent lie that it shipped. Track as follow-up. |
| `rouc`/`kfta`/`flhi` | If present as kids | Not separately implemented beyond the bounds above; covered by zbpc/89er/5xf2/gied |

### `ast-sgrep-r4rp` — Silent-fail honesty

| Kid | Claim | Evidence |
|-----|-------|----------|
| `3kvb` | Remove production `assert`/`expect` from `pipeline_parts` | `pipeline_parts.rs` is `Result`-valued (`time_loop` + store errors); `rg 'expect\(|assert!'` in that file → **no matches** |
| `2058` | Neural embedder silent fallback must not be silent | `embedder.rs`: loud message + requires `ASGREP_NEURAL_FALLBACK=1` acknowledgment before hashed swap; capabilities list includes the env |
| `1o0y` | Regex fail-closed | Verified: `search/passes/regex.rs` join maps panic → `StoreError` (no `unwrap_or_default`) |

### `ast-sgrep-cross-surface-0f7r` — Cross-surface consistency

| Item | Evidence |
|------|----------|
| Shared `clamp_output_limit` | `crates/ast-sgrep-core/src/limits.rs`; CLI uses `MAX_OUTPUT_RESULTS` / `MAX_EXCERPT_LINES`; MCP uses `clamp_agent_limit` |
| Consistent bool env parse | Shared `env_flag` / `is_boolish_true` (`env_flag.rs`); CLI BoolishValueParser; capabilities `environment_bool_values` |
| Canonicalize search roots | `Searcher::new` canonicalizes like Indexer |
| Capabilities catalog | Expanded env + `machine_schema`; golden `crates/ast-sgrep-cli/tests/fixtures/capabilities.json` matches (`capabilities_and_version_match_goldens` **passed**) |
| Machine JSON envelopes | `docs/validation/machine-json-schema.md` + engine-identity FailureBundle |

### `ast-sgrep-f8qy` — Validation artifacts (real, minimal)

| Kid | Artifact |
|-----|----------|
| Ranking oracle | `tests/fixtures/ranking/cases.json` + `crates/ast-sgrep-core/tests/ranking_oracle.rs` → **passed** |
| `djo7` EngineIdentity / FailureBundle | `docs/validation/engine-identity.md` |
| `jell` external differential | Honest deferral: `docs/validation/jell-deferral.md` (oracles + parity exist; no bit-identical external claim) |
| `c1i2` proof-pack | `docs/validation/proof-pack.md` |
| `6lmt` negative ledgers | `docs/validation/negative-ledgers.md` (incl. empty-native) |
| `f8qy.3` FeatureUniverse | `docs/validation/feature-universe.md` |

### `ast-sgrep-l115` — Trust / unsafe surface documentation

| Item | Evidence |
|------|----------|
| Seal/doc `IndexStore::connection()` | Comment on `store/sqlite.rs::connection` — first-party / typed APIs preferred |
| cargo-geiger baseline | `docs/validation/cargo-geiger-baseline.txt` |
| Neural/ort trust | `docs/validation/neural-trust.md` + env-trust |
| IVF header alloc bounds | `docs/validation/ivf-alloc-bounds.md` |
| `Pid::from_raw` docs | Comment at supervisor spawn + `docs/validation/childguard.md` |
| `tree_sitter_language` re-export | Note in `ast-sgrep-lang/src/pattern.rs` (CSharp→Java stand-in) |

### `ast-sgrep-g799` — Property / fuzz-style numerics

| Item | Evidence |
|------|----------|
| Scored property micro-harness | `math::property_tests::scored_heap_never_admits_nan_across_seeded_inputs` |
| NaN embed property | `normalize_then_rank_rejects_nan_query_residuals` |
| Miri/TSan skipped docs | `docs/validation/scored-property.md` |

`cargo test -p ast-sgrep-embed --lib math::` → **9 passed** (7 contract + 2 property)

### `ast-sgrep-732x` — ChildGuard Drop / signal

| Item | Evidence |
|------|----------|
| Audit + harden | `ChildGuard` Drop always reaps when armed; clears armed before reap; TERM→deadline→KILL |
| Tests | `supervisor::childguard_tests` (duty cycle, clamp, `kill_and_reap` missing pid) → **3 passed** |
| Docs | `docs/validation/childguard.md` |

### `ast-sgrep-ziij` — Machine schema / semantic-only

| Kid | Evidence |
|-----|----------|
| Machine JSON schema notes | `docs/validation/machine-json-schema.md` + capabilities `machine_schema` |
| `tzl8` / semantic-only | CLI `do_search_with_cli` forces `search_semantic` when `--semantic-only` / `ASGREP_SEMANTIC_ONLY` (`lib.rs`); SearchOptions carries `use_semantic_only` via `env_flag` |

### `ast-sgrep-packaging-ci-oykd` — Packaging honesty

| Kid | Evidence |
|-----|----------|
| Empty natives fail even if checksum = empty SHA256 | `packages/pi/launcher/src/index.js` `ASGREP_EXECUTABLE_EMPTY`; test in `npm-native-packages.test.mjs` → **9/9 pass** including empty-executable case |
| `.4` primary ownership | **This PR / HANDOFF (`test/quality-batch-e2hc-19-oxbj`) is primary** for packaging-ci-oykd.4 follow-through |
| cargo-check on PR | Already wired (`ci.yml`) |

### `ast-sgrep-k7l8` — Agent surface polish (implementable subset)

| Kid | Status |
|-----|--------|
| `.8` MCP deadline | Done via `INDEX_REPO_DEADLINE` + single-flight (`es7u` overlap) |
| `.9` Surface parity table | `docs/validation/surface-parity.md` |
| `.6` Tool description improvements | MCP tool descriptions clarified (no auto-fusion; sandbox; deadline) |
| `.2` ACP spike | **DEFERRED** — out of scope for this quality batch; no ACP client in-tree |
| `.3` fastmcp eval | **DEFERRED** — would need external FastMCP harness + pinned host; not claimed |
| `.5` session tracker | **DEFERRED** — durable multi-session state is a product feature, not a P0 honesty fix |
| `.10` / `.11` | **DEFERRED** — parent should schedule separately; not required for core epic acceptance |

### `ast-sgrep-ls6.2` / `ls6.3`

| Kid | Evidence |
|-----|----------|
| `ls6.2` version-triple conjunction | `packages/pi/extension/src/runtime.ts` + matching `dist/runtime.js`: when either of `version` / `machine_schema_version` is present, both must be present and match |
| `ls6.3` cloud embed feature through core | Core feature `cloud-embed` → `ast-sgrep-embed/cloud`; `crates/ast-sgrep-core/tests/cloud_feature_gate.rs` **passed** |

---

## DEFER — stretch (do **not** implement on this PR)

Core epic acceptance for fusion / sub-1ms paths is already met by closed children on this branch (pipeline_parts Result path, ranking oracle, semantic IVF roundtrip suite from prior commits, forbid-soundness). The following remain intentionally open:

| Bead | Justification for deferral |
|------|----------------------------|
| `ast-sgrep-prl3.1` | Amdahl index write/prep pipeline — large perf rewrite; needs measured before/after on fixed corpus |
| `ast-sgrep-prl3.2` | Parallel IVF kmeans — perf-only; recall determinism harness not in this batch |
| `ast-sgrep-prl3.3` | Continuous search cost-model replacing 128-file cliff — behavior change needing microbench matrix |
| `ast-sgrep-prl3.4` | `select_nth` for IVF probes — micro-opt; no honesty/safety gap |
| `ast-sgrep-prl3.5` | Batch chain BFS SQL — perf rewrite + identical node/edge proof |
| `ast-sgrep-prl3.6` | Index↔query invertibility property — frontier; separate CI harness |
| `ast-sgrep-e2hc` HNSW / GPU / daemon / Amdahl mega | Explicitly out of scope; fusion + sub-1ms bench path already exercised via closed `pipeline_parts` / ranking / IVF kids |
| `ast-sgrep-dx4g` / `aqyq` | Format-alloc reduction deferred pending profile |
| `ast-sgrep-k7l8.2/.3/.5/.10/.11` | See k7l8 table — written justification above |
| `ast-sgrep-f8qy` / `jell` full external differential | Stubbed as honest deferral doc; oracles cover in-tree ranking |

Closed kids that prove fusion / sub-1ms path exists without stretch work:

- `pipeline_parts` sub-1ms bench Result path (`3kvb` cleanup)
- `ranking_oracle` + `cases.json` (`f8qy`)
- Prior `semantic_ivf_roundtrip` / forbid-soundness / machine_contracts green on this branch

---

## Validation commands (focused)

```bash
export PATH="/usr/local/cargo/bin:$PATH"
bash scripts/verify-forbid-soundness
cargo test -p ast-sgrep-embed --lib math:: -j1 -- --test-threads=1
cargo test -p ast-sgrep-core --lib -j1 -- flatten_bounds_tests limits io_bounds env_flag lock_clear_on_poison --test-threads=1
cargo test -p ast-sgrep-core --test ranking_oracle -j1 -- --test-threads=1
cargo test -p ast-sgrep-core --test cloud_feature_gate -j1 -- --test-threads=1
cargo test -p ast-sgrep-cli --lib -j1 -- childguard_tests --test-threads=1
cargo test -p ast-sgrep-cli --test machine_contracts -j1 -- --test-threads=1
cargo test -p ast-sgrep-mcp --test protocol -j1 -- --test-threads=1
node --test packages/pi/launcher/test/npm-native-packages.test.mjs
```

Observed this pass: forbid-soundness **PASSED**; math **9**; flatten/limits/io/env **5**; ranking_oracle **1**; cloud_feature_gate **1**; childguard **3**; machine_contracts **8**; MCP protocol **9**; launcher **9**.

---

## Commits on this agent lineage

1. `6d9e3d5` `fix(embed): finite-domain Scored so Eq/Ord stay coherent (0pla)`
2. `96e26af` `fix(security): restore forbid(unsafe_code) via sealed mmap (p7l3)`
3. `eb5577e` `fix(cli): doctor envelope fails closed when unhealthy (s6ze)`
4. `436d5c3` `fix(security): poison/MCP sandbox/env-trust and fail-closed patterns`
5. `d6f22c7` / `16467d4` EPIC_EVIDENCE docs
6. *(this commit)* Resource bounds, silent-fail honesty, cross-surface clamps, validation docs, ChildGuard, packaging empty-native, ls6 version triple
