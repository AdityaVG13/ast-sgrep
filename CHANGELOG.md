# Changelog

All notable changes to **ast-sgrep** — hybrid code search that understands intent (lexical FTS + AST graph + offline semantic ranking).

Keep-a-Changelog conventions. Version policy: [`docs/RELEASING.md`](docs/RELEASING.md). Scope window: v1.0.0-alpha (2026-07-11) → v2.7.0 (2026-10-30). Measured claims trace to [`benchmarks/results/baselines.md`](benchmarks/results/baselines.md).

---

## [2.7.0] — 2026-10-30

The agent-native release. asgrep runs as first-class tools inside the Hermes agent harness, the CLI speaks the grep/rg habits coding models actually type, and the ranking engine banks its measured quality keeps: **CoSQA-500 MRR@10 .327 → .4230, RepoQA-600 .719 → .7319** on the frozen suites, with serve-path stalls measured out (8.3 s → 0.1–5.8 ms).

| Theme | What shipped |
|---|---|
| **Hermes-native** | Eager `asgrep_search` / `asgrep_read` / `asgrep_status` tools over one persistent `asgrep serve` worker — installed by `asgrep install --target hermes --yes`. |
| **The CLI speaks grep** | `-A/-B/-C/-m`, the short-flag cluster, `-p/--pattern`, `--count`, `asgrep read`, grep's `QUERY FILE` shape, bare metavariable patterns — every habit recovers to the canonical form with a note naming the difference. |
| **Pattern grammar parity** | Return-type sections, no-parens names, and body-less `fn` forms follow the reference per grammar — probed against ast-grep 0.45.2/0.51.0. |
| **Truthful envelopes** | `truncated: true` on every surface, zero-hit `why`/`next`, channel did-you-mean at the refusal boundary, quoted prefixed targets. |
| **Ranking keeps (measured)** | Semantic-leader + top-2 clamps, member-budget ANN probes, file-firsts hybrid pages. |
| **Performance (measured)** | Hybrid lane 2.35× median, −63% index CPU, ~−101 MB index, query RSS −40%, stale-root serve calls 8.3 s → <6 ms. |
| **Measurement hardened** | Frozen structural-intent suite (596 queries, ALL MRR@10 .9847), cross-surface parity gate, repin regression gate. |

### Hermes-native

- **Embedded plugin** — `asgrep install --target hermes --yes` writes `~/.hermes/plugins/asgrep`: self-contained, refuses dev symlinks, `--force` replaces, best-effort `hermes plugins enable`. One persistent `asgrep serve` worker answers eager tools — zero process spawn per call; tool errors pass through verbatim with their recovery commands. Hermes gets the plugin, not the MCP server.
- **Schemas teach the grammar** — tool schemas describe `defs:` / `callers:` / `imports:` / `word:` / `pattern:` channels, metavariable patterns, `excerpt_lines`, `semantic_only`, and LSP-style `path:line` refs, so models learn the query grammar from the tool list itself. Tool copy across surfaces is trigger-first and agent-accretive.
- **One grammar, every surface** — the MCP `query` schema, Pi tool schemas, and the Agent Plugins skill teach the same channel grammar and both ref spellings (`path#L12`, `path:12`), pinned by protocol contract tests; `code_read` accepts single-line and LSP-style refs everywhere.
- **Live-stress hardening** (found in the 2026-10-27 Hermes plugin stress): busy serve workers survive call timeouts and stay warm; a wedged-worker shutdown deadlock is fixed; the dormant `search_files` habit-lane takeover serves only provably-rg-identical answers and prewarms only repo-like roots; `limit` is clamped to the schema's 1..=200 (bridge maximums are unenforced — a live `limit=500` returned a 224 KB envelope).

### The CLI speaks grep

- `-A N` / `-B N` / `-C N` (attached `-A3` too) map to `--excerpt-lines N` — one window around the hit; `-m N` maps to `--limit N` (global hit cap; queries are fused, no per-file mode). Approximations are named in the note.
- The short-flag cluster (`-l -i -n -w -F -c -g -t -e`) recovers; `-p PATTERN` and repeatable `--pattern` batch onto `search`; `chain SYM --to TARGET` rewrites onto `call-path SYM TARGET`.
- Bare metavariable patterns (`asgrep 'fn parse($$$A)'`) route onto the pattern channel.
- `--count` / `-c`: `path:count` lines on the human path; `counts[]` in `--json` envelopes.
- A file as the search root walks ancestors nearest-first to the enclosing indexed root and scopes to that exact path (stale candidate indexes skipped).
- New `asgrep read` subcommand (shared reader: `path#L12-L40`, `path:12`, bare-path windows) matching the serve/MCP/Pi read surfaces.

### Pattern grammar (reference parity)

- **Return-type sections**: `fn $A($$$) -> u32`, `-> $R` (binds the declared text), go bare results, ts/php `: T`, py `-> T:`; `;`-completed forms follow each grammar's strict reading. Concrete returns are path identifiers plus one generic layer (`Option<u32>`); anything louder stays fail-closed.
- **No-parens names**: `fn $A` / `fn plain_ret` answer every fn-shaped node — rust trait signatures AND function items, generics wildcarded; ts/php free functions only. Any section on a no-parens form (`fn $A -> u32`) answers empty, except go's bare metavar result.
- **Body-less `fn $NAME($$$)`** follows the reference's per-grammar parse recovery: rust answers trait method signatures only, go answers declarations, ts/php answer empty. Spell `{ $$$B }` when you mean bodies.
- Rust trait method signatures are indexed (`decl:fn:` rows), so `fn` patterns admit sig-only files. **Upgrade note:** run `asgrep reindex <root>` once to gain signature rows on unchanged files.

### Envelopes that tell the truth

- `truncated: true` whenever the answer set exceeded the limit, on every model-facing render — `agent`, `agent-capsule`, `compact`, `github`'s `incomplete_results` — and warm-cache re-cuts set it instead of silently clearing it. The `limit` echo is always the call's limit. Previously the 17th hit of a 17-row answer silently vanished at the default limit 16.
- Zero hits carry `why` + a next step (`pattern_shape_contract` teaches that bodyless patterns match signatures only) — a bare `0` a model reads as "nowhere" is a bug.
- Channel near-misses — single-token aliases (`def:`, `import:`, `sem:`) or ≤2-edit misspellings — refuse with a did-you-mean plus the `literal:'…'` escape, instead of degrading to semantic free-text noise. Ordinary colon text (URLs, `key:value`, rust `mod::path`, `in:` scopes) is never hijacked.
- Quoted prefixed targets parse: `pattern:'fn name($$$A)'`, `word:'auth refresh'` — one balanced quote layer; nested or unbalanced quotes fail closed.

### Ranking keeps (measured on the frozen suites)

| Frozen-suite arm — MRR@10 (CoSQA-500 / RepoQA-600) | CoSQA | RepoQA |
|---|---:|---:|
| 2.6.0 published (2026-10-19 pairing) | .3271 | .7194 |
| + grammar keeps: scope prior for test/example trees, PEP 257 docstrings, C/C++ prototype declarations | .3319 | .7224 |
| + semantic-leader clamp (boost-manufactured overrides capped) | .3920 | .7302 |
| + top-2 semantic protection (rescues sem-rank-1/2 gold) | .4023 | .7296 |
| + member-budget ANN probes (constant ~4k admission budget replaces the flat 8-probe cap) | .4230 | .7319 |
| **2.7.0 shipped** — file-firsts pages land MRR-identical (file-diversity@10 .612 → .992) | **.4230** | **.7319** |

Shipped control detail (2026-10-30 HEAD run): CoSQA hit@1 .316 / hit@10 .662; RepoQA hit@1 .637 / hit@10 .935; fresh-process p50 13.2 ms / 18.0 ms; ≥50%@8k page coverage .815. Structural-intent suite: callers .9846 (hit@10 1.000), defs 1.0000, imports .9617, ALL .9847.

- **File-firsts hybrid pages** — an N-row page covers up to N distinct files instead of ~0.6 N; the page you read stops spending ~40% of its slots on files it already showed. MRR bit-identical, and ≥50%@8k page coverage rises .797 → .815 (more distinct files early covers more needle lines within budget); structural modes skip the cap.
- Closed with retry predicates in [`docs/progress/`](docs/progress/README.md): path-words embedding (re-rejected under the transformed engine), query-side observable gates (degenerate oracle), index-side centrality priors (inverted — the noise outranking gold is *more* central), offline fusion replay (infeasible: the shipped score is not globally linear in RRF terms; weight work now requires in-engine paired arms), directory-level page caps (gold legitimately clusters in its home dir).
- Wall decomposition (QVEC dump + dump rows): all 173 remaining misses are in-pool scoring losses, not table depth — the wall is ranking, not recall.

### Performance (measured)

**Search path** — the c1 campaign, closed at 2.35× median hybrid-lane speedup (2× target met; 1.69× under load):

- Embed+fanout channel forked onto a sibling read-only store: −19.9% under-load wall.
- IDF document frequencies counted in Rust over one vocabulary scan, then served from a `names.v1` sidecar without SQL row streaming.
- Query path pools straight from f16 bytes, touching only the rows its tokens need: default needle 40.9 → 30.9 ms, `-l` 41.0 → 27.5 ms, RSS 234.5 → 141.7 MiB — vectors bit-identical.

**Index path**:

- IVF seeding O(n·k²) → O(n·k) incremental nearest-max: **−63% index CPU**; Lloyd assignment over a flat centroid buffer: −4% index wall — both bit-identical.
- v3 IVF sidecar carries chunk ids; the query path mmaps them: −27% fresh-CLI.
- Schema 18 slimming: FTS line analyzers bound to external content (−32 MB), redundant `idx_pattern_nodes_file` dropped (−6.1 MB), static-embedder `embed_cache` dropped (−63 MB reclaimed on reindex).
- Embedding hoisted into the parallel prepare phase — the serial write loop only binds rows (7.0 s → 5.3–6.2 s span on the self corpus); the f16 weight table decodes to f32 once. Digests bit-identical.

**Serve path**:

- WAL close fold bounded to 250 ms — a serve session's pinned WAL no longer sleeps out the 5 s busy timeout per ambient drop: stale-root calls 8.3 s → 1.05–1.46 s.
- Stat-fingerprint ambient memo skips the full probe on unchanged trees (~3 ms stat walk vs 1.0–1.5 s probe): steady-state 0.1–5.8 ms; `index_status` surfaces `ambient_refreshes`.
- Rejected with retry predicate: deferred FTS/trigram wholesale rebuild for index waves (+0.25 s wall / +1.04 s CPU median — the rebuild repeats the tokenization volume).

### Measurement

- Frozen structural-intent suite (`benchmarks/frozen_suite/struct_gen.py`, sha-pinned corpora, 596 deterministic queries across callers/defs/imports).
- `ASGREP_QVEC_DUMP` query-vector dump hook (default off, `ASGREP_FUSION_DUMP` precedent) for offline wall decomposition.
- Cross-surface parity gate: CLI vs serve vs MCP on shared fixtures.
- Repin regression gate (`npm run test:repin-gate`), mutation-proven, pinning the lock-repin predicate against the 2.5.4 incomplete-lock class.
- Release engineering: Linux preflight e2e + starved-Actions fallback publisher; the public reference-tool comparison retired (suite harness renamed `frozen_suite`; internal rows stay gitignored).

### Fixed

- **In-memory VFS regression** from the 2.6.0 WAL-persist change: every `IndexStore::open_in_memory` (foreign-root searches, `--index-path`) failed with a raw `SQLITE_NOTFOUND` — the persist now gates on WAL journal mode.
- **Mixed-fleet IVF compatibility**: the semantic IVF sidecar stays v2-shaped with chunk ids in a fingerprint-bound `semantic.ivf.ids` sidecar — 2.6.x readers accept the main file instead of silently retraining k-means per invocation; crash windows fail to the SQL id fallback, never wrong ids.
- The persistent Code Mode front door is `asgrep serve` (`codemode-serve` remains an alias); missing-index errors name the enclosing indexed ancestor and the `--auto-index` recovery.
- Index-format contract layer catches up to core schema 18; test suite reconciled (shared `.asgrep` indexes for generation harnesses, umask-proof apply fixtures).

### Upgrade notes

- Run `asgrep reindex <root>` once to gain trait-signature rows on unchanged files.
- The MCP serve layer moved rmcp → **fastmcp-rust 0.13** (nightly toolchain pinned): single-era serve (2024-11-05 revision), framework-owned argument admission (`-32602` naming the offending path), 22 wire-level suites (142 tests) reconciled against live probes.
- Dependencies: clap 4.6, tree-sitter 0.27, notify 8, cap-std 4.0.3, toolchain pin.
- The public Rust `ChainConfig`/`ChainResponse` literal break from 2.6.0 stands; 2.7.0 adds no new required fields.
- Hermes plugin tool schemas freeze per agent session — a plugin upgrade needs a new session.

---

## [2.6.0] — 2026-10-19

The static-engine release: offline semantic ranking in every default search, embedding table compiled into the binary. Frozen-suite pairing against published 2.5.4: RepoQA-600 MRR@10 .297 → .719, CoSQA-500 .104 → .327, fully offline (canonical rows: [`benchmarks/results/baselines.md`](benchmarks/results/baselines.md)).

### Added

- Native static embed engine: deterministic 256-d static (Model2Vec-style) embeddings compiled into the binary from a vendored MIT table — no model download, no ONNX runtime, no API key; the embed channel runs ungated in hybrid fusion, so offline semantic ranking is part of every default search.
- Model-identity index migration: indexes record their embedding model identity and re-embed automatically on the next index run when it changes; retired engines (`hashed`, `neural`) fail closed.
- Hybrid retrieval quality pass, measured per lever on the frozen suite with a held-out set (short NL questions drop boilerplate before embedding — kept at CoSQA +.0102); rejected levers and retry conditions live in the [`docs/progress/`](docs/progress/README.md) negative ledgers.
- Frozen-suite measurement harness under `benchmarks/frozen_suite/` (fusion capture, paired arm drivers) so quality claims are reproducible from the tree.
- Bound graph-chain exploration with `max_work`, with work/truncation diagnostics.

### Fixed

- Past-EOF string line refs rejected with the file's real line bound (`path#L999999`, `path:9999`) — serve `read`, CLI `read`, and codemode now match MCP `code_read`'s pinned fail-closed contract; in-range empty lines stay legitimate empty windows.
- Agent-ergonomics: the Hermes plugin dedupes repeated `asgrep_read` refs; zero-hit `agent-capsule` envelopes drop the pointless read-more hint.
- Zero-hit misses classify their reason on every machine surface (`MissReason::pattern_shape_contract` + a teaching next step, instead of a bare `0`).
- Pi transports classify rejected query scopes as `INVALID_ARGUMENTS`; stale CLI binary overrides are named and explained; literal/word/regex answers stay identical across warm/cold search, Unicode and embedded-NUL boundaries, and language filtering; multilingual structural matching repaired (repeated metavariable binding, qualified C# imports, fail-closed unsupported patterns); codemod apply rereads bounded; concurrent writes preserve their capture-sidecar evidence on refusal; LSP protocol errors and edits repair against exact open-buffer text; Pi startup warmups anchor to the authoritative checkout and cancel on retirement; large integers preserved in native results; annotated release tags compared by peeled commit; delayed npm metadata retried, versionless native lock entries rejected.
- SQLite WAL sidecars stay alive: writer connections set `SQLITE_FCNTL_PERSIST_WAL`, so one-shot writer CLIs (reindex, targeted updates) can no longer trigger SQLite's last-close deletion of the shared `-wal`/`-shm` underneath live in-process sessions — the Linux "database disk image is malformed" release-gate failure, strace-confirmed.

### Changed

- Public Rust `ChainConfig` and `ChainResponse` gained required fields — the planned Rust source-compatibility break; exhaustive struct literals need updating. 2.6.0 is not fully Rust source-compatible with 2.5.x.
- The Pi extension (`pi-ast-sgrep`) rides lockstep at 2.6.0; `launcherRange` stays `>=2.5.4 <3` by judgment (runtime schema guards degrade gracefully on 2.5.4 launchers).

---

## [2.5.4] - 2026-09-22

Native-inclusive release: all five platform packages, the launcher, and the Pi extension ship together. Pi now requires launcher `>=2.5.4 <3` so updates cannot retain native packages that predate these fixes. Reindex affected projects to restore any terminal carriage returns discarded by an older indexer.

### Fixed

- Preserve explicitly returned Code Mode arrays, read windows, and nested data in model-visible output, with an 8,000-character bound and an explicit truncation notice. Arbitrary `hits` arrays and caller-owned `nodes` no longer crash rendering; custom hit fields, excerpts, and deliberate fields such as `refs` and `ok` remain visible.
- Keep directory/glob search scopes as retrieval filters instead of passing them to the file-only incremental indexing API.
- Reject pre-aborted native calls and batches before executing any tools; preserve session reuse afterward. This fix requires rebuilding and releasing the native packages, not only the Pi extension.
- Make explicitly requested native test addons fail on load errors or fallback substitution, and derive their expected version from the binding contract.
- Apply the Code Mode deadline to freshness work, not only guest execution, and distinguish deadline expiry from caller cancellation.
- Preserve leading blank lines and truncation flags in native and filesystem read windows, plus lone carriage returns in filesystem reads; disclose presentation truncation and label only the displayed line range, including at the final character cap.
- Preserve literal POSIX backslashes in read/edit paths and indexed lookups; normalize zero read ranges consistently across native and filesystem paths.
- Reject invalid UTF-8 in filesystem reads/edits rather than silently rewriting bytes, while preserving valid source BOMs.
- Retain environment-free Rust call/batch construction while checking pre-aborted native signals.
- Correct the documented guest Worker and opt-in CLI supervisor execution models.
- Preserve path identity through ignore matching, pattern candidates/results, outline, LSP URIs, SCIP overlays, and code/test classification; reject invalid UTF-8 file URIs and canonical filesystem paths rather than opening a replacement-character alias.
- Keep indexed EOF cursor rows out of source windows, and preserve final carriage returns during indexing. Reindex existing affected files to restore previously discarded carriage returns.
- Repair the exported typed reader's BOM, Unicode-line-limit, empty-file, truncation-ref, and resume-offset handling.
- Serialize filesystem fallback edits across connectors, reject unpaired-surrogate edits/paths, and never replay ambiguous native mutations after a transport failure. Edit validation is grouped; filesystem writes are not a multi-file transaction.
- Keep checkout-relative refs stable in anchored sessions, scope symbol lookups on every transport, propagate connector cancellation to doctor, and preserve stale-index qualifications within output budgets.
- Preserve complete explicitly returned hit objects instead of inferring redundant fields; keep lossy summaries on the TUI/one-shot search surfaces only.
- Probe the actual workspace release directory for development addons, and align the Pi edit schema with the native 16-replacement limit.
- Prepare lockstep family releases after an independently versioned Pi patch without failing halfway through the version bump.

## [pi-ast-sgrep 2.5.3] - 2026-09-22

Extension-only patch; the CLI, launcher, and native packages remain at 2.5.2.

### Fixed

- Recover missing environment-only developer binary overrides through the checksum-verified bundled binary, with a visible warning. Explicit binary settings, permissions, empty artifacts, and checksum errors still fail closed.
- Preserve the binary-resolution cause in backend errors and report extension, launcher, and native versions in `/asgrep-doctor`, even when index health fails.
- Keep `asgrep_read` available without a native backend or index, preserving bounded windows, cancellation, and path confinement.
- Require launcher `>=2.5.2 <3` so an extension update cannot retain the obsolete 2.0.0 native engine.

## [2.5.2] - 2026-09-21

2.5.1 was tagged but never published (release-automation defects below); 2.5.2 supersedes it with identical product content plus the fixes.

### Fixed

- npm OIDC lane: publish jobs no longer pass `registry-url` to setup-node, which exports a placeholder `NODE_AUTH_TOKEN` that forced every publish into the token lane. Bootstrap auth is wired explicitly instead; verified end to end with a CI probe.

## [2.5.1] - 2026-09-21

### Added

- MoonBit scripting mode (`.mbtx`) files are detected, indexed, and searchable as MoonBit (gh-35).

### Fixed

- npm release bootstrap lane: token-authed publishes no longer attempt the GitHub Actions OIDC exchange (npm prefers OIDC whenever it is available and failed the PUT on a trusted-publisher mismatch instead of falling back to the token). The publish job fails fast when the bootstrap token is empty or rejected.
- Inter-crate path dependencies now pin the release version (`2.5.1`) per the release policy instead of the stale `2.0.0` floor.

## [2.5.0] - 2026-09-20

### Added

- Dart language support: `tree-sitter-dart` parser, `.dart` indexing, symbol/call/import extraction (classes, enums, mixins, extensions, methods, getters/setters, constructors, top-level functions), and native structural patterns (`fn`-decls via `function`-shaped queries, `class`/`interface`/`type` decls, calls, if-templates).
- MoonBit language support: `tree-sitter-moonbit` parser, `.mbt` indexing, extraction of functions (`fn`), trait impl methods (`impl ... with fn`), structs, tuple structs, enums, external enums, types, error types, and traits, plus `import`-statement paths and `apply`/dot-apply call edges. `Type::method` definitions resolve to the method name.
- `type $NAME` structural patterns now also match C `typedef`s (`type_definition` shares its node kind with MoonBit `type` declarations).

### Fixed

- `pi-ast-sgrep` no longer imports `node:sqlite` at load time, so Pi/OMP/ZMP can boot under Bun via `bun:sqlite`.
- Search opens the index read-only and no longer auto-indexes. `--no-auto-index` is the search default; pass `--auto-index` to opt in. `asgrep index` / `reindex` / `watch` remain the write path.
- Unprefixed identifier search ranks the exact-case definition first (`Searcher` before `bench_searcher`). Conceptual queries prefer code over markdown that repeats the query, and still run defs/callers (not pattern matching) so `how does hybrid search work` hits `search_hybrid`.
- Conceptual fanout seeds defs of concept-related symbols instead of flooding `callers:main`. `credential renewal` ranks `auth_refresh` over generic entrypoint callers; partial identifier matches (`refresh` in a longer test name) lose to the exact spelling. Identifier queries pull exact defs from the whole index so a 100-file lexical cascade cannot hide them.
- SIGTERM/SIGHUP on the supervisor kills the worker within 100 ms (CONT then TERM then KILL) instead of waiting 5 s while holding a write lock.
- Writers checkpoint WAL (`TRUNCATE`) on clean close and cap `journal_size_limit` at 64 MiB.
- `asgrep version` prints `index_schema`. Doctor reports on-disk vs binary schema and the recovery command (`asgrep reindex` vs install a newer binary).
- `--path` on search is a hidden alias of `--file-filter` so agents do not hard-fail.
- Pattern-node excerpts are reconstructed from `lines` at search time instead of duplicating source text in `pattern_nodes`.
- `--lang` aliases include every indexed source extension (`ts`, `h`, `hpp`, `py`, `rs`, …) so SQL filters match stored language ids.
- `pi-ast-sgrep` installs and runs against official launchers behind the canonical version: CLI envelopes accept the same binary major, and `read`/`edit`/`find` fall back to extension-side implementations when the launcher predates the native tools. The `>=2.1.0` floor that made `pi-ast-sgrep@2.2.0` uninstallable (npm ETARGET / bun "failed to resolve") is now `>=2.0.0`.
- One-shot `literal:`/`word:` search no longer loads the full RAM line corpus (full `lines` scan plus token indexing on every cold query). Cold queries take the trigram/SQL path; warmed sessions (`codemode-serve`) keep the memchr scan. One-shot `literal:SearchHit` drops from 171 ms to 7.9 ms p95 on the self corpus (beats ripgrep 13.0 ms); under-cap queries are hit-identical across paths.

### Changed

- `asgrep-mcp` speaks stdio through official `rmcp` instead of a hand-rolled JSON-RPC loop. Tool catalogs and compact envelopes are unchanged. `notifications/cancelled` aborts an in-flight `index_repo` walk; `ping` is served while index/search run on blocking threads.
- GitHub Actions `CI` no longer runs on `pull_request`; dispatch it from the Actions tab. Other workflows were already `workflow_dispatch` only.
- Repository hygiene: drop campaign scripts and process docs; keep only clone-required `scripts/` (`rustc-capped`, `cpu-limit-exec.py`, `verify-forbid-soundness`).
- Keep search, index, and Pi behavior tests under `tests/`. Drop campaign fuzz, benches, keep-gates, process suites, and crate-source `#[cfg(test)]` stubs.
- Public `benchmarks/results/` drops campaign process docs (`FLOOR_PROMOTION_PROTOCOL`, fusion scorecard) and the 600-line missing-harness speed dump. Live CLI latency is the 2026-08-28 self-corpus row in `speed.md`. CI no longer installs `cargo-fuzz` against a crate that is not shipped.
- Extension publish lanes (local dogfood + signed `pi-v` tag) refuse to publish when `launcherRange` resolves to no published launcher version.
- Release gates follow the severed version model: the e2e asserts per-layer versions, `check-contract` verifies the registered Pi tool surface against the contract, the inventory drops the removed `asgrep_status` tool, and the retired two-version upgrade path is deleted from the e2e harness.

---

## Earlier releases

| Version | Date | Summary |
|---------|------|---------|
| [v2.0.2](https://github.com/AdityaVG13/ast-sgrep/releases) | 2026-08-16 | `pi-ast-sgrep` patch: first search no longer full-walks a ready index; cancelled search aborts the shared in-flight index; incremental `index_all` skips unchanged files by stored mtime |
| [v2.0.1](https://github.com/AdityaVG13/ast-sgrep/releases) | 2026-08-16 | `pi-ast-sgrep` patch: TUI query rendering truncates to terminal width so long queries no longer crash Pi |
| [v2.0.0](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v2.0.0) | 2026-08-15 | **Local-first major** — five PRs (#27, #29–#32). Breaking: cloud/Ollama embed clients removed (embeddings in-process only; `cloud|ollama` indexes fail closed until reindex). Landed: atomic index generations + durability profiles, separate code/prose FTS, PPMI expansions, graph resolution tiers, staged planner, IVF k-means, MCP `structuredContent`/`outputSchema`, Agent Plugins package, deterministic critic, causal `follow_up_queries`, two-channel `AND`/`AND NOT`, SCIP JSON overlay, per-field semantic vectors, keep-gates vs pinned ast-grep/ripgrep, watch freshness bound, `pattern:`+`callers:` span joins, `call-path`, indexed `codemod`, Pi results reaching the model via `content` |
| [v1.4.0](https://github.com/AdityaVG13/ast-sgrep/releases) | 2026-08-06 | Seven-PR release — see below |
| [v1.3.2](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v1.3.2) | 2026-07-23 | **The Pi Package Update** — atomic npm family with five host-constrained native packages; ≥60% faster pipeline and ≥60% fewer crates LOC; sub-1 ms warm core gate; release-train hardening (partial-publish recovery 1.3.0 → 1.3.1 → 1.3.2) |
| v1.2.0-alpha | 2026-07-21 | "Fast Update" — draft release only, superseded by v1.3.2 |
| [v1.1.0-alpha.1](https://github.com/AdityaVG13/ast-sgrep/tree/v1.1.0-alpha.1) | 2026-07-17 | Pi npm bootstrap, SSH-signed tag verification, fused scores preserved through rerank |
| [v1.1.0-alpha](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v1.1.0-alpha) | 2026-07-12 | FTS per-file delete hardening (rowid-based chunked deletes replace O(N²)) |
| [v1.0.0-alpha](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v1.0.0-alpha) | 2026-07-11 | First alpha: hybrid code search — lexical FTS + AST graph + offline semantic ranking |

### v1.4.0 in one paragraph

Seven PRs plus direct-to-main hardening: in-process **Code Mode (PTC)** API (`ast-sgrep-codemode`, warm sessions, typed tool catalog, pure-JSON in-plan transforms); a **13-language** pattern/extraction surface with native C# and Swift grammars; fusion normalization and coverage-aware ranking correctness; LSP symbol navigation fixed for case-mismatched identifiers; bounded watch freshness; monotonic store generations defeating stale cache/IVF identities; measured quality gates replacing vacuous ones; and an anti-bloat sweep preserving every public API.

| PR | Theme |
|----|-------|
| [#14](https://github.com/AdityaVG13/ast-sgrep/pull/14) | LSP symbol correctness & compatibility |
| [#20](https://github.com/AdityaVG13/ast-sgrep/pull/20) | P1 store & search correctness |
| [#21](https://github.com/AdityaVG13/ast-sgrep/pull/21) | Quality & compatibility batch (measured gates) |
| [#22](https://github.com/AdityaVG13/ast-sgrep/pull/22) | Fusion normalization & ranking correctness |
| [#23](https://github.com/AdityaVG13/ast-sgrep/pull/23) | C# + 13-language pattern correctness |
| [#25](https://github.com/AdityaVG13/ast-sgrep/pull/25) | Anti-bloat cleanup & compatibility hardening |
| [#26](https://github.com/AdityaVG13/ast-sgrep/pull/26) | **ast-sgrep-codemode** scaffold (Code Mode / PTC) |

### Workstream anchors

PR bodies cite bead ids (`ast-sgrep-<slug>`) mapping to records in `.beads/issues.jsonl` — the durable workstream anchors for the v1.4.0 window (ranking `e2hc.*`/`u9fj`/`iva9.*`, store `44a4`/`jsfn`/`c2j5`, LSP `nuli`/`zblv`, languages `difu.1–6`, Code Mode `k7l8.*`). Representative commits are live-linked in each PR body.
