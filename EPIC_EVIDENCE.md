# EPIC evidence — PR #21 (`test/quality-batch-e2hc-19-oxbj`)

Hard evidence for P0 (and selected P1) beads addressed on this branch.
Commands use `PATH="/usr/local/cargo/bin:$PATH"` and focused package filters.

---

## P0 CRITICAL

### `ast-sgrep-0pla` — Float trait / Scored NaN

| Item | Evidence |
|------|----------|
| Finite-domain `Scored::new` | `crates/ast-sgrep-embed/src/math.rs` — rejects NaN/±∞ |
| Eq/Ord coherent | Blank `Eq` + finite-only `Ord`; no NaN==NaN special case on `Scored` |
| `top_by_similarity` filters non-finite | Same file; retain requires `is_finite` + ULP min gate |
| `normalize_vec` NaN residuals | Shared `normalize_vec` / `normalize_vec_in_place`; ANN uses embed helpers |
| RerankConfig Hash via Display | Comment in `rerank.rs` documenting stable model identity |
| Unit tests | `cargo test -p ast-sgrep-embed --lib math::` → **7 passed** |

Commit: `fix(embed): finite-domain Scored so Eq/Ord stay coherent (0pla)`

---

### `ast-sgrep-p7l3` / `p7l3.1` — Forbid-soundness

| Item | Evidence |
|------|----------|
| Workspace `unsafe_code = "forbid"` | Root `Cargo.toml` |
| Core no longer `deny`+`allow` | `crates/ast-sgrep-core/Cargo.toml` inherits `[lints] workspace = true` |
| Sealed mmap wrapper | `crates/ast-sgrep-mmap` — sole intentional `unsafe { MmapOptions::map }` |
| Product crate roots | `#![forbid(unsafe_code)]` on lib/bin roots |
| Gate script | `bash scripts/verify-forbid-soundness` → **PASSED** |
| CI on `pull_request` | `.github/workflows/ci.yml` jobs `forbid-soundness` + `cargo-check` |
| Policy docs | `SECURITY.md`, `CONTRIBUTING.md`; fuzz/ exclusion noted |
| cargo-audit ≠ soundness | Explicit table in `SECURITY.md` + script footer |
| IVF still mapped | `cargo test -p ast-sgrep-core --test semantic_ivf_roundtrip` → **8 passed** |

Commit: `fix(security): restore forbid(unsafe_code) via sealed mmap (p7l3)`

---

### `ast-sgrep-s6ze` — Fail-closed agent honesty

| Bead | Claim | Evidence |
|------|-------|----------|
| `768z` | index/reindex/search fail when ROOT missing | `ensure_existing_root`; `machine_contracts::format_aliases_typos_and_root_failures_are_unambiguous` exit 2 |
| `9ilf` | empty/missing index not silent zero success | `open_searcher` / `run_chain` bail `index is empty`; contract test asserts message |
| `s6ze.1` | doctor never ok:true/exit 0 when healthy:false | `agent::run_doctor` → `print_machine_json_status(..., false, 2)` + `exit(2)`; `assert_doctor_unhealthy` |
| `s6ze.2` | agent envelope never ok:true on hard Root/Index faults | operational failures golden `ok:false` exit 2 |
| `s6ze.3` | triage codes empty-index / missing-root | `doctor_triage_json` kinds `missing_root`, `empty_index`; contract asserts `missing_root` |
| `xunv` | boolish ASGREP_* | CLI BoolishValueParser; `env_flag` in core/embed/MCP; capabilities lists spellings; contract loops all bool envs |

Tests: `cargo test -p ast-sgrep-cli --test machine_contracts` → **8 passed**  
Commit: `fix(cli): doctor envelope fails closed when unhealthy (s6ze)`

---

### `ast-sgrep-sxjc` — Panic/poison integrity

| Item | Evidence |
|------|----------|
| Regex fail-open join fixed | `regex.rs` maps worker panic → `StoreError` (no `unwrap_or_default`) |
| Clear-on-poison caches | `lock_clear_on_poison` for response/semantic/META caches; IVF `SESSION_CACHE` |
| MCP Searcher poison invalidate | `searcher_for` / `invalidate_searcher_cache` clear slot on poison |
| LSP `index_ready` after poison | `with_index_lock` sets `index_ready=false`, `clear_poison` |
| Panic-injection test | `search::tests::lock_clear_on_poison_resets_state` **passed** |
| Docs matrix | `docs/panic-poison.md` |

LSP regression: `cargo test -p ast-sgrep-lsp --test lsp` → **4 passed**

---

### `ast-sgrep-6j65` — MCP agent-surface safety

| Bead | Evidence |
|------|----------|
| `v0mg` sandbox roots | `McpServer::sandbox_root`; test `tool_roots_are_sandboxed_under_configured_workspace` **passed** |
| `2hgl` no panic expects | `searcher_for` populate uses `ok_or_else` error, not `expect` |
| `es7u` deadline + single-flight | `index_lock` + `INDEX_REPO_DEADLINE` (600s) in `tool_index_repo` |

Tests: `cargo test -p ast-sgrep-mcp --test protocol` → **9 passed**

---

### `ast-sgrep-j0x4` — Env-trust

| Bead | Evidence |
|------|----------|
| `2lbz` embed URL allowlist | `embed_url_is_allowed`; blocks metadata/evil hosts; `embed_url_allowlist_blocks_ssrf_targets` **passed** |
| `y2hc` AST_GREP hardening | PATH search removed; requires `ASGREP_ALLOW_AST_GREP=1` + absolute `ASGREP_AST_GREP`; timed version probe + kill/reap |
| `73um` docs | `docs/env-trust.md` (+ SECURITY / docs index) |

Test: `pattern::tests::external_ast_grep_is_disabled_without_explicit_allow` **passed**

---

### `ast-sgrep-eytx` / `kfhh` — Pattern cascade / ast-grep lifecycle

| Item | Status / evidence |
|------|-------------------|
| Production never starts ast-grep | `docs/structural-patterns.md` + code: `search_pattern` is native/index only |
| No PATH/`sg` exec by default | `find_ast_grep_binary` opt-in only (j0x4) |
| Dead exotic subprocess lies | Fixed suite asserts `!needs_ast_grep_fallback`; bench path optional |
| `lv0x`/`0pgq`/`l0xb`/`lgef`/`rwnm` | Subprocess path not used in production; remaining bench path uses kill+wait (reap) and timeout |
| `kfhh.3` / `18bf` / `9drb` / `kfhh.1` | Pre-existing native cascade on branch (`e2hc.22` removed production subprocess); bakeoff suite pinned native |
| `kfhh.2` / `1o0y` regex panic | Regex join fail-closed (sxjc) |

---

### `ast-sgrep-agent-security-rl1p`

| Kid | Evidence |
|-----|----------|
| `.4` Preflight roots/empty indexes | Same as s6ze `768z`/`9ilf` |
| `.5` Do not exec untrusted PATH/ASGREP_AST_GREP by default | j0x4 `y2hc` |
| `.6` Gate ASGREP_BIN / binaryPath | Launcher `validateExecutable` already fail-closed; docs in `env-trust.md` |
| `.7` Block SSRF via embed URL env | Overlaps `2lbz` allowlist |

---

## P1 (time permitting)

| Bead | Status |
|------|--------|
| `packaging-ci-oykd` cargo check on PR | Wired in `ci.yml` `cargo-check` job |
| Homebrew version note | `packaging/homebrew/ast-sgrep.rb` comment clarified |
| Empty native fail | Existing launcher `ASGREP_PLATFORM_PACKAGE_MISSING` / unsupported host (verified by package tests) |
| `ziij` semantic-only → `search_semantic` | CLI `do_search_with_cli` forces semantic when `--semantic-only` / env |
| `f8qy` cases.json | Present at `tests/fixtures/ranking/cases.json`; wired on branch (`e2hc.19e`) |
| `dx4g` META_CACHE poison | Clear-on-poison applied in `estimate_prevented_reads` |
| `e2hc` stretch (HNSW/GPU/daemon) | **Not implemented** — deferred for parent |
| `prl3` stretch | Left open unless already satisfied elsewhere |

---

## Validation commands (focused)

```bash
export PATH="/usr/local/cargo/bin:$PATH"
bash scripts/verify-forbid-soundness
cargo test -p ast-sgrep-embed --lib math:: -j1 -- --test-threads=1
cargo test -p ast-sgrep-embed --lib embed_url -j1 -- --test-threads=1
cargo test -p ast-sgrep-core --lib -j1 -- lock_clear_on_poison external_ast_grep --test-threads=1
cargo test -p ast-sgrep-core --test semantic_ivf_roundtrip -j1 -- --test-threads=1
cargo test -p ast-sgrep-cli --test machine_contracts -j1 -- --test-threads=1
cargo test -p ast-sgrep-mcp --test protocol -j1 -- --test-threads=1
cargo test -p ast-sgrep-lsp --test lsp -j1 -- --test-threads=1
```

---

## Commits on this agent pass

1. `fix(embed): finite-domain Scored so Eq/Ord stay coherent (0pla)`
2. `fix(security): restore forbid(unsafe_code) via sealed mmap (p7l3)`
3. `fix(cli): doctor envelope fails closed when unhealthy (s6ze)`
4. *(pending)* panic/poison + MCP sandbox + env-trust + pattern allow + ziij + evidence
