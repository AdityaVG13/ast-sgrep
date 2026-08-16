# Changelog

All notable changes to **ast-sgrep** — hybrid code search that understands intent (lexical FTS + AST graph + offline semantic ranking).

This changelog follows [Keep a Changelog](https://keepachangelog.com/) conventions. Version numbering follows the project release policy in [`docs/RELEASING.md`](docs/RELEASING.md): additive, backward-compatible functionality increments the minor version after 1.0.

**Scope window:** v1.0.0-alpha (2026-07-11) → v2.0.0 (2026-08-15). The v1.4.0 section covers seven earlier PRs plus direct-to-main commits since v1.3.2; research evidence is logged in [`CHANGELOG_RESEARCH.md`](CHANGELOG_RESEARCH.md).

## Unreleased

## Version Timeline

| Version | Date | Summary |
|---------|------|---------|
| [v2.0.2](#v202-2026-08-16) | 2026-08-16 | Pi package: first search no longer full-walks a ready index; cancel stops in-flight index |
| [v2.0.1](#v201-2026-08-16) | 2026-08-16 | Pi package: truncate asgrep TUI chrome so long queries no longer crash Pi |
| [v2.0.0](#v200-2026-08-15) | 2026-08-15 | Local-first major: five PRs (#27, #29–#32). Remote embed APIs removed; critic, conjunction, SCIP, Pi results |
| [v1.4.0](#v140-2026-08-06) | 2026-08-06 | 7-PR release: Code Mode (PTC), 13-language pattern surface, search/ranking correctness, LSP symbol fixes, watch freshness, durability hardening, quality gates + anti-bloat |
| [v1.3.2](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v1.3.2) | 2026-07-23 | **The Pi Package Update** — "Out of the Alpha and into the Light" |
| [v1.2.0-alpha](#v120-alpha-draft-superseded) | 2026-07-21 | *The Fast Update* — draft release, superseded by 1.3.2 |
| [v1.1.0-alpha.1](https://github.com/AdityaVG13/ast-sgrep/tree/v1.1.0-alpha.1) | 2026-07-17 | Pi npm bootstrap, SSH-signed tag verification |
| [v1.1.0-alpha](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v1.1.0-alpha) | 2026-07-12 | FTS per-file delete hardening |
| [v1.0.0-alpha](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v1.0.0-alpha) | 2026-07-11 | First alpha |

---

## v2.0.2 (2026-08-16)

`pi-ast-sgrep` 2.0.2. Native CLI, launcher, and platform packages stay at 2.0.0.

### Fixed

- Pi first search no longer walks a ready, clean index. The refresh interval re-checks status instead of hashing the tree.
- Last cancelled search waiter aborts the shared in-flight index so workers cannot keep running after Pi moves on.
- Incremental `index_all` skips unchanged files by stored mtime before read/hash. Code Mode indexing uses host parallelism by default (`ASGREP_INDEX_THREADS` still caps). Native mtime skip and cancel polling land in the next family rebuild; this patch ships the Pi freshness coordinator immediately.

## v2.0.1 (2026-08-16)

`pi-ast-sgrep` 2.0.1. Native CLI, launcher, and platform packages stay at 2.0.0.

### Fixed

- Pi TUI no longer exits when asgrep renders a long search query. `AsgrepText.render()` now truncates to the terminal width.

---

## v2.0.0 (2026-08-15)

2.0 is a direct, stable major release. It makes ast-sgrep local-first, fixes the Pi result path, and lands five merged PRs on top of v1.4.0: [#27](https://github.com/AdityaVG13/ast-sgrep/pull/27), [#29](https://github.com/AdityaVG13/ast-sgrep/pull/29), [#30](https://github.com/AdityaVG13/ast-sgrep/pull/30), [#31](https://github.com/AdityaVG13/ast-sgrep/pull/31), and [#32](https://github.com/AdityaVG13/ast-sgrep/pull/32), plus stacked and follow-on commits.

### Breaking changes

Cloud (`--cloud-embed`, `ASGREP_EMBED_API_KEY`, OpenAI-compatible HTTP) and Ollama (`--ollama-embed`, `ASGREP_OLLAMA_URL`) embedding clients are gone. Embeddings are in-process only: hashed semantic (default) and optional ONNX neural (`--features neural-embed`). Indexes that still store `embed_backend=cloud|ollama` fail closed until `asgrep reindex`. The Cloudflare Code Mode adapter is unrelated and stays.

The associated CLI flags, environment settings, configuration variants, and public Rust APIs were removed. Pi users can update the package normally, but this API removal and the index-format update make 2.0 a breaking semver release.

### Capability map

| Track | What landed | Evidence |
|-------|-------------|----------|
| [#27](https://github.com/AdityaVG13/ast-sgrep/pull/27) Index / retrieval / agents | Atomic index generations and durability profiles; separate code vs prose FTS; repository-learned PPMI expansions; graph resolution tiers; staged planner; IVF k-means; MCP `structuredContent` / `outputSchema`; Agent Plugins package | `00c430ba` and the #27 merge |
| [#29](https://github.com/AdityaVG13/ast-sgrep/pull/29) Maintainability + Pi | Isomorphic store/index/search/MCP splits behind façades; native hybrid search off the Node event loop; writer-generation advertised after partial watch-batch errors | `778caec5` |
| [#30](https://github.com/AdityaVG13/ast-sgrep/pull/30) Honesty + local embed | Golden asserts; default-on keep-gates vs committed benches; per-field semantic vectors + intent weighting; SCIP JSON overlay (`index\|reindex --scip`); HTTP embed clients removed | `38960f02` |
| [#31](https://github.com/AdityaVG13/ast-sgrep/pull/31) Critic / planner / conjunction | Deterministic post-fusion critic; causal `follow_up_queries`; two-channel `AND` / `AND NOT`; native nested structural templates | `80c8f3f2` |
| [#32](https://github.com/AdityaVG13/ast-sgrep/pull/32) Gates / freshness / joins | Pattern-1 vs pinned ast-grep and `literal:` vs pinned ripgrep keep-gates (Not-run unless provisioned); watch freshness bound under sustained writes; `pattern:`+`callers:` span joins; `call-path` and indexed `codemod` on the stacked branch | `9a3b4cd6` |

### Fixed and improved

- **Pi results reach the model:** one-shot tools now serialize bounded hits into `content`, and Code Mode places its rendered final result in `content` instead of leaving useful output only in display-only `details`.
- **Clean, user-controlled indexing:** `.git` and `.asgrep` are the only unconditional directory skips. Repository ignore rules remain authoritative; dotfiles and user-specific directories are not silently hardcoded. Binary source-looking files are skipped without noisy failures, and stale rows are removed.
- **Index compatibility:** Pi and the native engine now agree on index schema 12, with controlled rebuilds for older formats.
- **Retrieval and graph quality:** semantic field vectors, SCIP facts, critic/planner routing, graph joins, keep-gates, span handling, and blank-line excerpt safety are integrated.
- **Storage maintainability:** the SQLite store is split into focused modules without changing its public ownership boundary.

---

## v1.4.0 (2026-08-06)

The next release ships **seven pull requests** plus direct-to-main hardening. Highlights in one line: a new in-process **Code Mode (PTC)** API, a **13-language** pattern/extraction surface with native C# and Swift grammars, **search and ranking correctness** (fusion normalization, coverage-aware ranking), **LSP symbol navigation** that finally handles case-mismatched identifiers, **bounded watch freshness**, **durability/cache correctness**, and a large **quality + anti-bloat** wave with measured release gates.

### Capability map

| PR | Theme | Files changed |
|----|-------|---------------|
| [#14](https://github.com/AdityaVG13/ast-sgrep/pull/14) | LSP symbol correctness & compatibility | 36 |
| [#20](https://github.com/AdityaVG13/ast-sgrep/pull/20) | P1 store & search correctness | 43 |
| [#21](https://github.com/AdityaVG13/ast-sgrep/pull/21) | Quality & compatibility batch (measured gates) | 159 |
| [#22](https://github.com/AdityaVG13/ast-sgrep/pull/22) | Fusion normalization & ranking correctness | 47 |
| [#23](https://github.com/AdityaVG13/ast-sgrep/pull/23) | C# + 13-language pattern correctness | 54 |
| [#25](https://github.com/AdityaVG13/ast-sgrep/pull/25) | Anti-bloat cleanup & compatibility hardening | 62 |
| [#26](https://github.com/AdityaVG13/ast-sgrep/pull/26) | **ast-sgrep-codemode** scaffold (Code Mode / PTC) | 976* |

\* #26's file count is dominated by ~867 fuzz corpus fixtures; the feature surface is ~90 source files.

---

### PR #26 — Code Mode (PTC): in-process programmatic search

**Delivered capability:** a new in-process `ast-sgrep-codemode` NAPI addon that turns ast-sgrep into a programmatic tool-calling surface for coding agents — warm sessions, typed tool catalog, zero CLI spawn.

Ships a new **`ast-sgrep-codemode`** crate and its NAPI addon (`ast-sgrep-codemode.node`) inside the existing five `@ast-sgrep/<platform>` npm packages — same install path as the CLI binary, so `pi install` gets **zero-spawn Code Mode** out of the box.

- `CodeModeSession`: warm, stateful search session over `ast-sgrep-core` with a sticky `Searcher` cache, per-call limits (clamped 1–500), and a soft call budget (default 64) that fails closed.
- Typed, stringly-dispatched tool catalog: `search`, `semantic`, `chain`, `defs`, `callers`, `imports`, `index_status`, `index_repo`, `filter_hits`, `select`, `catalog_search`, `catalog_describe`.
- In-plan transforms (`filter_hits`, `select`) run as pure JSON projections — no shell, no code execution outside the sandbox.
- Pi extension integration: Code Mode JS sandbox as primary agent execution, warm parallel batching, session-scoped sticky pool, hardened execution paths.

Representative commits: [`4873c0e`](https://github.com/AdityaVG13/ast-sgrep/commit/4873c0e), [`47d595c`](https://github.com/AdityaVG13/ast-sgrep/commit/47d595c), [`5aab31d`](https://github.com/AdityaVG13/ast-sgrep/commit/5aab31d).

### PR #23 — C# correctness and the 13-language pattern surface

**Delivered capability:** native C# and Swift grammar support plus a shared nine-language conformance contract, delivered through a table-driven 13-language pattern/extraction surface.

- **Native C# grammar**: structural patterns and calls now use real `tree-sitter-c-sharp` instead of a Java stand-in, covering declarations, properties, local functions, constructors, and invocation expressions ([difu.5](https://github.com/AdityaVG13/ast-sgrep/commit/6c3151f)).
- **Complete Swift support**: grammar registration, symbol and import extraction, call ownership, structural patterns, source discovery, module resolution, editor activation ([difu.2](https://github.com/AdityaVG13/ast-sgrep/commit/c4cddcc)).
- **More grammars**: C/C++/Kotlin/PHP grammars and Ruby `singleton_method` coverage ([difu.3/4/6](https://github.com/AdityaVG13/ast-sgrep/commit/2ded187)).
- **One shared conformance contract** across all nine languages: parse fidelity, symbols, imports, callers, patterns, spans, and false-positive suppression ([difu.1](https://github.com/AdityaVG13/ast-sgrep/commit/59ac840)), plus a table-driven 13-language pattern/extract surface.
- Post-review hardening (pushed during this session): literal `LIKE`/`GLOB` metacharacter escaping already landed on main ([c2j5](https://github.com/AdityaVG13/ast-sgrep/commit/23ce658)); single-character hybrid terms stay substantive and embedding switched to **full-rank XOF feature hashing** ([`4e9c981`](https://github.com/AdityaVG13/ast-sgrep/commit/4e9c981)); P0 durability/agent/LSP crash paths hardened ([`fb2cc6b`](https://github.com/AdityaVG13/ast-sgrep/commit/fb2cc6b)).

### PR #22 — Fusion normalization and ranking correctness

**Delivered capability:** hybrid scores that respect each producer's real scoring contract — no more dilution by unrelated query terms, with coverage-aware, threshold-safe ranking.

- **Lexical fusion normalization**: hybrid scores are normalized against the producer's actual rank-zero RRF ceiling instead of being diluted by total query terms ([e2hc.14](https://github.com/AdityaVG13/ast-sgrep/commit/d7f3ea9)).
- **Single-character queries stay searchable**: every non-empty query term is treated as substantive ([`945bec3`](https://github.com/AdityaVG13/ast-sgrep/commit/945bec3)).
- **Def/Caller ceilings** derive from the terms that actually match each hit's symbol/callee, removing unmatched-term dilution ([u9fj]).
- **Coverage-aware ranking**: pre-truncation keeps coverage in the sort key with a `keep*4` pool ([8mb8]), rerank writes consistent scores back into hits ([iva9.8]), zero/non-finite scores can no longer fill the limit ([iva9.4]), and invalid `file_filter` globs error instead of silently skipping the filter ([iva9.2]).
- Quoted hybrid queries route to a literal pass; structural-index fused scores are bounded at a calibrated fraction of the pattern channel ([noik]).

Representative commits: [`d7f3ea9`](https://github.com/AdityaVG13/ast-sgrep/commit/d7f3ea9), [`a9860de`](https://github.com/AdityaVG13/ast-sgrep/commit/a9860de), [`b470c6e`](https://github.com/AdityaVG13/ast-sgrep/commit/b470c6e).

### PR #20 — P1 store & search correctness

**Delivered capability:** monotonic generation counters that kill stale cache/IVF identities, full 256-bit semantic projections, and a bounded max-latency watch pipeline.

- **Monotonic generations**: `semantic_data_version` and searchable-index generations defeat stale semantic cache and IVF identities after delete/re-add, across connections ([44a4](https://github.com/AdityaVG13/ast-sgrep/commit/2c6d700)).
- **All 256 BLAKE3 sign bits** consumed in semantic projection instead of tiling the first 32 ([e2hc.13](https://github.com/AdityaVG13/ast-sgrep/commit/36212e3)).
- **Bounded watch freshness**: a max-latency debounce state machine (quiet-gap coalescing + `3×` max-latency bound + `.asgrep`/sidecar self-event filtering) replaces the unbounded-sustained-stream stall ([jsfn](https://github.com/AdityaVG13/ast-sgrep/commit/01cdaad)).
- Nested-file-transaction depth tracking with poisoned rollback and `synchronous=NORMAL` restore on end; meta preserved across clears; UTF-8 path handling.

Representative commits: [`100424a`](https://github.com/AdityaVG13/ast-sgrep/commit/100424a), [`01cdaad`](https://github.com/AdityaVG13/ast-sgrep/commit/01cdaad), [`fe0e655`](https://github.com/AdityaVG13/ast-sgrep/commit/fe0e655).

### PR #14 — LSP symbol correctness & compatibility

**Delivered capability:** reliable definition/reference navigation for uppercase and case-mismatched symbols, with hardened UTF-16 spans and multi-root handling.

- **Case-insensitive symbol navigation**: definition/reference lookup routes through case-insensitive indexed resolution, so uppercase and mixed-case symbols resolve reliably ([nuli](https://github.com/AdityaVG13/ast-sgrep/commit/b3f6236), [z47q](https://github.com/AdityaVG13/ast-sgrep/commit/35207af)).
- **Call-chain nodes** with source spelling differing from stored symbol case resolve through the real chain expansion path.
- **UTF-16 span fixes**: `utf16_span_end` no longer eats the next character on pure insertion with a zero-length range ([c9os](https://github.com/AdityaVG13/ast-sgrep/commit/e61b2a8)).
- Multi-root folder binding, readiness, dirty-buffer and sync-error hardening ([zblv/x46g](https://github.com/AdityaVG13/ast-sgrep/commit/bd882e0), [ei0i](https://github.com/AdityaVG13/ast-sgrep/commit/bc019ae)).

### PR #21 — Quality & compatibility batch

**Delivered capability:** measured quality gates, SIMD-accelerated literal search, weighted RRF fusion with learned weights, mmap-backed IVF, and a typed TypeScript Code Mode API — all backed by hard test evidence.

- **Measured quality gates replace vacuous gates**: intended-hit and rank contracts, repaired shared-subset rank correlation, ANN quality exercised on the indexed path ([e2hc.19]).
- **Performance**: SIMD literal prefiltering + Rayon work stealing with measured work-span profiling ([e2hc.1]); ~60% faster pipeline and ~60% fewer crates LOC.
- **Ranking honesty**: immutable signal provenance and within-signal score margins on every result/JSON surface ([e2hc.2]); weighted RRF runtime fusion with learned weights and Fisher-style sensitivity ([e2hc.4]); strict literal → AST → semantic constraint cascade for unprefixed queries ([e2hc.3]).
- **Retrieval**: bounded AST-child embeddings with nearest function/file parent mapping ([e2hc.6]); nonfused hierarchical keyword/AST/semantic agent retrieval with stable node refs ([k7l8.4]); pinned caller/import normalization contract ([7uz6]).
- **Freshness & memory**: monotonic freshness identity across caches, sidecars, models, bulk/watch indexing ([e2hc.15]); aligned read-only mmap IVF layout with measured cold/fresh/warm open p99 ([e2hc.9]); minified compact output with deduped paths and hard snippet budgets ([k7l8.7]).
- **Security hardening**: MCP sandbox/env-trust and poison fail-closed patterns ([436d5c3](https://github.com/AdityaVG13/ast-sgrep/commit/436d5c3)); `forbid(unsafe_code)` restored via sealed mmap ([96e26af](https://github.com/AdityaVG13/ast-sgrep/commit/96e26af)); doctor envelope fails closed when unhealthy ([eb5577e](https://github.com/AdityaVG13/ast-sgrep/commit/eb5577e)).
- **Delivery**: independent verification of native npm delivery across macOS arm64/x64, Linux arm64/x64, Windows x64 ([ls6.1]); graph retrieval oracle across four languages and four naming styles ([55hl]); case-equivalent retrieval verified against the real senpi monorepo ([oxbj]).

### PR #25 — Anti-bloat cleanup & compatibility hardening

**Delivered capability:** a Zero Tech Debt sweep that deletes dead surfaces, documents honest performance/grammar facts, and hardens compatibility — while preserving every public API.

- **Zero Tech Debt wave**: dead surfaces deleted (orphan `passes/` tooling, dead re-export shims, `ast_grep_pattern_for_query` with zero callers), `module_resolve` split, CLI/search/store surfaces table-driven.
- **Honesty infrastructure**: accurate `QUERY_GRAMMAR.md`, `PERF_INVENTORY.md` + docs index, benchmark honesty rules, EPIC evidence records.
- **Pi workflow checker** moves to `python3` YAML (no Ruby): `check:pi-contract`, `check:pi-release`, `test:pi-release-gate` all green.
- Public APIs preserved during cleanup ([8a96bd5](https://github.com/AdityaVG13/ast-sgrep/commit/8a96bd5)).

### Also landing on main since v1.3.2 (ships in v1.4.0)

- `fix(store+search)`: monotonic `semantic_data_version` defeats cache+IVF collision ([44a4](https://github.com/AdityaVG13/ast-sgrep/commit/2c6d700))
- `fix(store)`: `symbols_named` case-insensitive + functional index ([z47q](https://github.com/AdityaVG13/ast-sgrep/commit/35207af))
- `fix(store)`: language-aware `resolve_module_path` ([5wkz](https://github.com/AdityaVG13/ast-sgrep/commit/a79c35f))
- `fix(embed)`: probe and cache Ollama/Cloud embedding dim ([tmy6](https://github.com/AdityaVG13/ast-sgrep/commit/e3abc9a)); language-aware doc comment markers ([pwfm](https://github.com/AdityaVG13/ast-sgrep/commit/d30b4c6))
- `fix(literal)`: escape GLOB/LIKE metacharacters in needles ([c2j5](https://github.com/AdityaVG13/ast-sgrep/commit/23ce658))
- Tests: graph query oracle ([55hl](https://github.com/AdityaVG13/ast-sgrep/commit/41ccd6b)), imports mixed-case parity ([oxbj](https://github.com/AdityaVG13/ast-sgrep/commit/0870cba)), uppercase LSP navigation pins ([nuli](https://github.com/AdityaVG13/ast-sgrep/commit/b3f6236))
- CI: durable release assets + cross-compile smoke test, idempotent publish + local preflight

---

## v1.3.2 — The Pi Package Update

Released 2026-07-23 — *"Out of the Alpha and into the Light."*

- **ast-sgrep is now a pi package**: the `pi-ast-sgrep` extension and `ast-sgrep` launcher, published as one atomic npm family at `1.3.2` with five host-constrained native packages (`@ast-sgrep/darwin-arm64`, `darwin-x64`, `linux-arm64-gnu`, `linux-x64-gnu`, `win32-x64-msvc`).
- **Performance & LOC**: ≥60% faster pipeline and ≥60% fewer crates LOC ([55c2eb8](https://github.com/AdityaVG13/ast-sgrep/commit/55c2eb8)); sub-1ms core pipeline gate on warm sample fixture ([6d3eb0b](https://github.com/AdityaVG13/ast-sgrep/commit/6d3eb0b)).
- Watcher paths normalized against canonical roots ([5480cf7](https://github.com/AdityaVG13/ast-sgrep/commit/5480cf7)); full LSP/MCP/eval/embed surfaces restored with densify-only LOC cuts ([857cd43](https://github.com/AdityaVG13/ast-sgrep/commit/857cd43)).
- Release train hardening: pinned publish npm, debug CLI for packaged e2e, partial-publish recovery (1.3.0 → 1.3.1 → 1.3.2).

## v1.2.0-alpha — (draft, superseded)

The "Fast Update" release exists only as a **draft GitHub release** (2026-07-21); no tag was published and it was superseded by v1.3.2. It is listed here for history only.

## v1.1.0-alpha.1

- Pi npm bootstrap: first npm publication, `pi-ast-sgrep` package workspace and release train ([008ff1a](https://github.com/AdityaVG13/ast-sgrep/commit/008ff1a)).
- Verify SSH-signed release tags ([e6b6a27](https://github.com/AdityaVG13/ast-sgrep/commit/e6b6a27)); release workflows made manual-only.
- Fused scores preserved through rerank ([22781f5](https://github.com/AdityaVG13/ast-sgrep/commit/22781f5)); release/machine contract hardening.

## v1.1.0-alpha — FTS per-file delete hardening

- **Rowid-based FTS deletes**: replace O(N²) deletes with rowids collected from `lines`, then chunked deletes on `lines_trigram`, plus missing `file_id` indexes ([37f6920](https://github.com/AdityaVG13/ast-sgrep/commit/37f6920), [4817889](https://github.com/AdityaVG13/ast-sgrep/commit/4817889)).

## v1.0.0-alpha

- First alpha release: hybrid code search — lexical FTS + AST graph + offline semantic ranking. Alpha quality; APIs subject to change.

---

## Workstreams

Durable workstream anchors live in the project tracker (`.beads/issues.jsonl`, managed via `br`). The v1.4.0 window closes these workstream groups:

- **Ranking & retrieval correctness**: `ast-sgrep-e2hc.14`, `ast-sgrep-u9fj`, `ast-sgrep-s7jw`, `ast-sgrep-8mb8`, `ast-sgrep-iva9`, `ast-sgrep-noik`, `ast-sgrep-hhca`
- **Store & cache correctness**: `ast-sgrep-44a4`, `ast-sgrep-e2hc.13`, `ast-sgrep-jsfn`, `ast-sgrep-naiv`, `ast-sgrep-c2j5`, `ast-sgrep-z47q`, `ast-sgrep-5wkz`, `ast-sgrep-tmy6`, `ast-sgrep-pwfm`
- **LSP**: `ast-sgrep-nuli`, `ast-sgrep-zblv`, `ast-sgrep-x46g`, `ast-sgrep-c9os`, `ast-sgrep-ei0i`
- **Language surface**: `ast-sgrep-difu.1` – `ast-sgrep-difu.6`
- **Quality & delivery**: `ast-sgrep-e2hc.1` – `ast-sgrep-e2hc.22`, `ast-sgrep-k7l8.*`, `ast-sgrep-7uz6`, `ast-sgrep-oxbj`, `ast-sgrep-55hl`, `ast-sgrep-ls6.1`, `ast-sgrep-tk4c`, `ast-sgrep-7m36`, `ast-sgrep-kp3e`, `ast-sgrep-56w1.3`
- **Code Mode (PTC)**: `ast-sgrep-k7l8.1`, `ast-sgrep-k7l8.4`, `ast-sgrep-k7l8.7`, `ast-sgrep-codemode-9228`

## Notes for agents

- PR bodies cite **bead ids** (`ast-sgrep-<slug>`) that map to records in `.beads/issues.jsonl`; the bead ids above are the durable workstream anchors.
- The seven v1.4.0 PRs are open at the time of writing and reference the pre-merge branch state; representative commits are from each PR's head branch.
- Research memo: [`CHANGELOG_RESEARCH.md`](CHANGELOG_RESEARCH.md).
