<p align="center">
  <img src=".github/assets/banner.svg" width="100%" alt="ast-sgrep">
</p>

# ast-sgrep

**Hybrid code search that understands intent** -- not only text or syntax.

**v2.5.2** · 15 languages · local-first semantic · critic + two-channel `AND` · Code Mode (on by default, no API key)

> **One search tool.** Identifiers, natural language, defs/callers, semantic, and patterns — ranked. You do not need a second grep.

---

## Install

For Pi, install the native package directly:

```bash
pi install npm:pi-ast-sgrep
```

It immediately adds **`asgrep`** (Code Mode), `asgrep_search`, `asgrep_index`, `asgrep_status`, four `/asgrep-*` commands, and the `ast-sgrep` skill. The first search lazily creates `.asgrep/`; no Rust toolchain, PATH setup, MCP adapter, credential, or runtime download is required. See the [complete Pi package guide](docs/pi-package.md) and [Code Mode](docs/codemode.md).

**Upgrading to 2.0:** this is a breaking semver release. Cloud (`--cloud-embed`, `ASGREP_EMBED_API_KEY`) and Ollama (`--ollama-embed`, `ASGREP_OLLAMA_URL`) embedding clients are gone. Local hashed semantic search remains the default, optional neural embeddings remain in-process, and indexes that still store `embed_backend=cloud|ollama` fail closed until `asgrep reindex`. Pi users can update the package normally.

Standalone CLI binaries are on the [v2.5.2 GitHub Release](https://github.com/AdityaVG13/ast-sgrep/releases/tag/v2.5.2) (`asgrep`, `asgrep_darwin_x64`, `asgrep_linux_arm64`, `asgrep_linux_x64`, `asgrep_windows_amd64.exe`). This release is GitHub + npm only; it is not published to crates.io.

To build from source:

```bash
git clone https://github.com/AdityaVG13/ast-sgrep
cd ast-sgrep
cargo build --release -p ast-sgrep-cli
./target/release/asgrep --help
```

Standalone binaries: `asgrep` and `ast-sgrep` (aliases).

On Unix, the CLI runs commands through the process supervisor. `ASGREP_CPU_LIMIT_PERCENT`
sets the worker process runnable wall-time fraction in each 10 ms SIGSTOP/CONT cycle; it is not
a machine-wide or one-core CPU percentage, and multi-threaded work may consume several cores
while runnable. On Windows, commands run directly: search, indexing, cancellation, and path
handling are supported, but the duty cycle is not enforced.

---


### Agent Plugins (portable skills + MCP)

For non-Pi clients, use the [Agent Plugins](https://agent-plugins.org/) package at [`packages/agent-plugin`](packages/agent-plugin): `plugin.json` + `skills/ast-sgrep` + `mcp.json` (stdio `asgrep-mcp`).

**Code Mode XOR MCP:** Pi → `pi install npm:pi-ast-sgrep` (Code Mode). MCP hosts → `asgrep-mcp` / agent-plugin. Do not register both in the same agent.


## Easy start (agents)

Paste into your agent:

```text
Clone https://github.com/AdityaVG13/ast-sgrep, cd into it, run `cargo build --release -p ast-sgrep-cli`.
Register target/release/asgrep-mcp as a stdio MCP server named "ast-sgrep" (build with: cargo build --release -p ast-sgrep-mcp).
Verify: run ./target/release/asgrep index . then search for defs: of a symbol in this repo.
```

---

## What's new in 2.5.0

2.5.0 is a speed + correctness release: two new languages, a 20× one-shot
literal fix, read-only search by default, and a hardened MCP/Pi surface.
Full notes: [CHANGELOG](CHANGELOG.md#250---2026-09-20).

| You can now... | How |
|----------------|-----|
| Search Dart and MoonBit | `.dart` / `.mbt` indexing with symbol/call/import extraction and native structural patterns. 15 languages total. |
| Get one-shot literal answers in ~8 ms | Cold queries take the trigram/SQL path instead of loading the full RAM line corpus: `literal:SearchHit` dropped from 171 ms to 8.5 ms p95 (beats ripgrep 13.1 ms on the same tree). |
| Search without touching the index | Search opens the index read-only and no longer auto-indexes. `--auto-index` opts back in; `index` / `reindex` / `watch` remain the write path. |
| Run MCP on official rmcp | `asgrep-mcp` speaks stdio through `rmcp` instead of a hand-rolled loop. `notifications/cancelled` aborts in-flight indexing; `ping` stays live. |
| Trust the ranking more | Exact-case definitions rank first, conceptual queries prefer code over docs that repeat them, and fanout seeds concept-related defs instead of flooding `callers:main`. |
| Recover from schema drift | `asgrep version` prints `index_schema`; doctor reports on-disk vs binary schema and the exact recovery command. |

Also in this release: SIGTERM/SIGHUP kills the worker in <100 ms;
writers checkpoint WAL with a 64 MiB journal cap; pattern excerpts are
reconstructed from `lines` (no duplicated source text); `--lang` accepts
every indexed extension; C `typedef`s match `type:` patterns; Pi boots
under Bun and degrades gracefully against older launchers.

## What's new in 2.0

2.0 is the local-first major release. It lands five merged PRs on top of v1.4.0 -- [#27](https://github.com/AdityaVG13/ast-sgrep/pull/27), [#29](https://github.com/AdityaVG13/ast-sgrep/pull/29), [#30](https://github.com/AdityaVG13/ast-sgrep/pull/30), [#31](https://github.com/AdityaVG13/ast-sgrep/pull/31), [#32](https://github.com/AdityaVG13/ast-sgrep/pull/32) -- plus stacked and follow-on commits. Full notes: [CHANGELOG](CHANGELOG.md#v200-2026-08-15).

| You can now... | How |
|----------------|-----|
| Search without a remote embed API | Cloud and Ollama clients are removed. Hashed semantic is default; optional ONNX MiniLM stays in-process (`--features neural-embed`). |
| Compose two indexed channels | `callers:process_request AND pattern:fn $NAME($$$)` joins by overlapping span. Other pairs join by file. `AND NOT` subtracts. Plain English `and` is still hybrid search. |
| See why a hit ranked | A deterministic post-fusion **critic** boosts multi-channel agreement, penalizes identifier-fragment collisions, and writes `critic:` notes into agent JSON `why`. |
| Drill without guessing prefixes | `follow_up_queries` / `suggested_next` are derived from the actual top hit (kind, symbol, missing evidence, margin). Settled hits get an empty list. |
| Overlay SCIP facts | `asgrep index . --scip path/to/index.json` (JSON SCIP only). Missing or malformed input degrades; it never fails the index. Matching graph edges upgrade to `ScipExact`. |
| Trace a directed call path | `asgrep call-path SOURCE SINK .` -- call graph only, not value flow, with resolution-tier evidence. Reports at most one shortest path (a deterministic function of the index: SCIP-exact evidence first, then file path, line, callee name); it is an edge sampler, not an enumerator -- use `callers:` / `callees:` search for full edge evidence. A hop is `precise` only when it resolves to a unique same-file definition (or compiler/import evidence), so same-named symbols in different languages never serialize as exact edges. |
| Dry-run an indexed rewrite | `asgrep codemod --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' --dry-run .` then omit `--dry-run` to apply transactionally. |
| Keep Pi results on the model path | One-shot tools and Code Mode put bounded hits in `content`, not only display-only `details`. Native search runs off the Node event loop. |

Also in this release, without changing the day-to-day query prefixes:

- **Index schema 12** with atomic generations, durability profiles, separate code vs prose FTS, and controlled rebuilds for older formats.
- **Ignore rules stay yours.** `.git` and `.asgrep` are the only unconditional directory skips. Dotfiles and user-specific directories are not silently hardcoded.
- **Multi-field semantic vectors** persist beside each chunk; query intent weights those fields. Large repos still use `.asgrep/semantic.ivf`.
- **Repository-learned vocabulary** can widen conceptual candidate discovery (PPMI); final lexical/structural scoring still uses the original query.
- **Watch** bounds freshness under sustained same-path writes and ignores `.asgrep` artifacts before they enter the queue.
- **Native `pattern:`** covers nested structural templates in-process. Optional keep-gates compare `literal:` presence to pinned ripgrep and Pattern-1 to pinned ast-grep when those binaries are provisioned; they do not claim full tool identity.

---

## Why this exists

For **search**, one tool is enough. You do not need ripgrep to find a token, ast-grep to find a shape, or Semgrep to find a concept. Index the repo once; unprefixed `asgrep` ranks the definition, the callers, the structural match, and the semantic near-miss in one list.

Most other code search is either **fast text** (ripgrep) or **pattern matching** (ast-grep / Semgrep). Neither answers *"where does credential renewal happen?"* when those words never appear in the code, and neither ranks `Searcher` above `bench_searcher`.

**ast-sgrep** builds a **persistent index**: symbols, caller/callee edges, imports, lexical FTS, and **symbol-level semantic vectors** enriched with call-graph context. Query in natural language or with graph prefixes; get ranked hits with excerpts for humans or agents.

**No API key required.** Offline hashed semantic search works out of the box. Optional in-process neural embeddings (ONNX / MiniLM) are a local upgrade, never a network call.

| You need... | ast-sgrep gives you... |
|-------------|------------------------|
| Where is X defined? | `defs:` + ranked hybrid hits |
| Who calls this? | `callers:` + call hierarchy (LSP) |
| How does auth refresh work? | NL → symbols + anchors + semantic similarity |
| "credential renewal" (no token overlap) | Semantic hit on `auth_refresh` |
| Callers of X that match a shape | `callers:X AND pattern:fn $NAME($$$)` |
| Skip test callers | `defs:handle AND NOT callers:test_` |
| Structured JSON for an agent | `--json --format agent` (`why`, `follow_up_queries`) |
| Structural rewrite / codemod | `asgrep codemod` (indexed native patterns) |
| Agent needs search as a tool (not a subprocess) | `asgrep` -- in-process, stateful session (Code Mode) |

[Full comparison →](docs/comparison.md)

---

## Where it fits

On an indexed tree, **asgrep is the search function.** Do not spawn ripgrep, ast-grep, or Semgrep to find code the index already covers.

| Job | Tool |
|------|------|
| Find a name, a shape, a caller, or an idea | **ast-sgrep** (hybrid / `defs:` / `callers:` / `pattern:` / `literal:` / semantic) |
| Logs, generated files, or a tree you have not indexed | [ripgrep](https://github.com/BurntSushi/ripgrep) |
| Full-rule rewrites and exotic ast-grep YAML | [ast-grep](https://github.com/ast-grep/ast-grep) (asgrep `codemod` covers indexed native patterns) |
| SAST rule packs | [Semgrep](https://github.com/semgrep/semgrep) |

Search quality is the product bar: exact identifiers rank the definition first, conceptual NL prefers code over docs that repeat the query, and vocabulary expansion is a precision tool. CLI process start on a tiny tree can still lose a raw `rg` race; that is not the contest. See [comparison.md](docs/comparison.md).

---

## Quick start

Index is incremental and lives under the project root at `.asgrep/`.

```bash
cargo build --release -p ast-sgrep-cli
./target/release/asgrep index .
./target/release/asgrep 'defs:auth_refresh' . --limit 3
./target/release/asgrep semantic 'credential renewal' . --limit 3
./target/release/asgrep chain 'auth_refresh' . --limit 3
./target/release/asgrep 'callers:process_request AND pattern:fn $NAME($$$)' .
./target/release/asgrep 'defs:handle AND NOT callers:test_' .
./target/release/asgrep call-path main validate_input .
```

Optional overlays and rewrites:

```bash
./target/release/asgrep index . --scip path/to/index.json   # JSON SCIP; degrades, never fails
./target/release/asgrep codemod --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' --dry-run .
```

Unprefixed queries run **hybrid** retrieval. Two-channel `AND` / `AND NOT` is recognized only when both sides are prefixed (`defs:`, `callers:`, `imports:`, `pattern:`, `literal:`, `regex:`, `word:`, or `semantic:`). See the [query grammar](docs/QUERY_GRAMMAR.md).

[Getting started →](docs/getting-started.md) · [Architecture →](docs/ARCHITECTURE.md) · [Docs index →](docs/README.md)

---

## What "semantic" means here

ast-sgrep embeds **symbol chunks** (function/method/type with name, kind, callers, callees, excerpt), expanded with code-domain concept groups (auth ↔ credential ↔ token, refresh ↔ renewal, …). Chunks persist **per-field vectors**; query intent weights those fields instead of concatenating everything into one blob.

```text
Query: "credential renewal"
  → semantic pass ranks auth_refresh (zero token overlap)
```

Provider chain: **neural** (optional `--features neural-embed` + `ASGREP_NEURAL_EMBED`) → **local hashed semantic** (always available). Large repos may use a persisted IVF-ANN sidecar (`.asgrep/semantic.ivf`). There is no cloud or Ollama embed client.

After fusion, the critic reviews the shortlist in-process. Agent envelopes expose `why` (including `critic:` notes) and causal `follow_up_queries`.

[Semantic layer →](docs/semantic-search.md) · [Fusion and critic →](docs/fusion-ranking.md) · [Planner →](docs/cascade-query-planner.md)

---

## Benchmarks (honest summary)

These are **checked-in run summaries**, not portable guarantees. Hardware, corpus, cache state, and flags all matter. Status vocabulary: [benchmarks/README.md](benchmarks/README.md).

| Recorded comparison | Status | Published result | Evidence |
|---------------------|--------|------------------|----------|
| 2.5.0 re-pin, self corpus (702 tracked / 650 indexed) | `reproducible-in-tree` | Warm literal 8.5 ms vs rg 13.1 ms; `pattern:SearchHit` 9.5 ms vs ast-grep 61.3 ms hand-written (279.8 with generated file); semantic NL 15.5 ms; fff warm 0.4 ms (ranked, latency-only) | [speed.md](benchmarks/results/speed.md) |
| 2026-09-20 one-shot fix row (~650 tracked files) | `reproducible-in-tree` | Warm literal 7.9 ms vs rg 13.0 ms; `pattern:SearchHit` 10.2 ms vs ast-grep 63.3 ms; semantic NL 18.5 ms | [speed.md](benchmarks/results/speed.md) |
| 2026-08-28 self corpus (445 tracked files) | `reproducible-in-tree` | Cold index 4.58 s p95; warm literal 19.0 ms vs rg 11.1 ms; `pattern:SearchHit` 129 ms vs ast-grep 26.5 ms; semantic NL 20.3 ms | [speed.md](benchmarks/results/speed.md) |
| Warm lexical / structural at 23k–100k | `historical` / `UNREPRODUCIBLE` | Large speedups in that dump; latency-only for structural | [head-to-head.md](benchmarks/results/head-to-head.md) |
| Cross-tool bake-off | `UNREPRODUCIBLE` | Mixed; inspect every row | [bakeoff.md](benchmarks/results/bakeoff.md) |
| Known regressions | `UNREPRODUCIBLE` | Published without suppression | [losses.md](benchmarks/results/losses.md) |

Measured 2026-09-20 on Apple M5 Max from the 2.5.0 tree atop `b23a2454` (`release-perf`, rustc 1.98.0): a `git ls-files` copy of this tree (650 files indexed, schema 16, `index.db` 292 MiB). Warm `literal:SearchHit` **8.5 ms p95** vs ripgrep **13.1 ms** (asgrep 1.54×); warm-tgrep server leads at 5.8 ms; `grep -rn` 58.1 ms. Warm `pattern:SearchHit` **9.5 ms p95** vs ast-grep **61.3 ms** on hand-written code (**6.5×**, latency-only, not match-set) — 279.8 ms with the generated 1 MB+ file included, which ast-grep parses per query and the index does not. Semantic NL **15.5 ms p95**. fff-mcp warm `grep` answers in **0.4 ms p95** but surfaces ranked matches where exhaustive tools find ~300 lines (latency-only by design). Older rows are kept in [speed.md](benchmarks/results/speed.md) for history. Full protocol there.

Canonical table: [head-to-head.md](benchmarks/results/head-to-head.md). Index: [benchmarks/README.md](benchmarks/README.md).

**Quality snapshot (UNREPRODUCIBLE):** cite only fingerprint `self-hybrid-d3eab74` in [baselines.md](benchmarks/results/baselines.md#retrieval-quality--self-corpus-18-gold-queries) -- hybrid MRR **0.712**, Recall@k **0.889**, nDCG@k **0.751**. The gold harness is absent. Do not quote the superseded ≈0.75 / 0.94 row (`self-hist-pre-29129bd`) as current. On some foreign corpora the offline embedder currently adds little over lexical + AST.

---

## Interfaces

| Interface | Build | Use case |
|-----------|-------|----------|
| **CLI** | `cargo build --release -p ast-sgrep-cli` | Terminal, scripts, `call-path`, `codemod` |
| **MCP** | `cargo build --release -p ast-sgrep-mcp` | AI agents (stdio); `structuredContent` / `outputSchema` |
| **Code Mode** | `ast-sgrep-codemode` | Programmatic tool-calling / multi-step plans (Pi) |
| **LSP** | `cargo build --release -p ast-sgrep-lsp` | Editor navigation |
| **Library** | `ast-sgrep-core` | Embed search in Rust tools |
| **JSON plugins** | `--format agent\|github\|gitlab\|agent-capsule` | Agents / CI |

---

## Documentation

| Doc | Contents |
|-----|----------|
| [docs/README.md](docs/README.md) | Full documentation index |
| [Getting started](docs/getting-started.md) | Install, index, queries, flags |
| [Pi package guide](docs/pi-package.md) | Pi install, tools, data, security, updates, rollback, uninstall |
| [Architecture](docs/ARCHITECTURE.md) | Index schema, search pipeline, crates |
| [Query grammar](docs/QUERY_GRAMMAR.md) | Prefixes, two-channel `AND` / `AND NOT` |
| [Semantic search](docs/semantic-search.md) | Chunks, providers, IVF-ANN |
| [Fusion ranking](docs/fusion-ranking.md) | RRF, post-fusion critic, `why` |
| [Cascade planner](docs/cascade-query-planner.md) | Retrieval cascade and causal follow-ups |
| [Benchmarks](benchmarks/README.md) | Methodology, reproduction, losses |
| [Comparison](docs/comparison.md) | vs ripgrep / ast-grep |
| [MCP](docs/mcp.md) · [Code Mode](docs/codemode.md) · [Use cases](docs/use-cases.md) · [Releasing](docs/RELEASING.md) | Agents, PTC, LSP, release checklist |

---

## Workspace layout

| Path | Role |
|------|------|
| `crates/ast-sgrep-core` | Index, SQLite store, hybrid search, critic, planner |
| `crates/ast-sgrep-cli` | `asgrep` / `ast-sgrep` CLI + supervisor |
| `crates/ast-sgrep-lang` | Tree-sitter extraction (15 languages) |
| `crates/ast-sgrep-embed` | In-process embedding backends + optional rerank |
| `crates/ast-sgrep-mmap` | Memory-map helpers |
| `crates/ast-sgrep-lsp` | Language server |
| `crates/ast-sgrep-mcp` | MCP server |
| `crates/ast-sgrep-codemode` | Code Mode / programmatic tool-calling |
| `crates/ast-sgrep-plugins` | Output formats |
| `crates/ast-sgrep-testkit` | Shared fixtures for search/index/Pi tests |
| `tests/` | Search, index, and Pi behavior tests |
| `packages/pi/` | Pi extension, launcher, and native packages |
| `packages/agent-plugin/` | Portable Agent Plugins + MCP |
| `benchmarks/` | Published results (`results/`) and studies (`studies/`) |
| `docs/` | User and architecture docs |

---

## Project status and verification

**v2.5.2.** Local-first embeddings, index schema 16, two-channel conjunction, post-fusion critic, causal follow-ups, SCIP overlay, `call-path`, indexed `codemod`, and Pi Code Mode (results on the model path) are in place. 15 languages (Dart, MoonBit newest), fusion-normalized ranking, and the hashed semantic layer remain. Search is read-only by default; MCP runs on official rmcp.

GitHub Actions workflows are **manual-only** (`workflow_dispatch`) to control Actions minutes. Local quality bar for contributors:

```bash
cargo check --workspace -j1
cargo test -p ast-sgrep-core --test parity -j1 -- --test-threads=1
cargo test -p ast-sgrep-cli --test cli_smoke -j1 -- --test-threads=1
cargo build --release -p ast-sgrep-cli -j1
./target/release/asgrep --help
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## License

MIT. See [LICENSE](LICENSE).