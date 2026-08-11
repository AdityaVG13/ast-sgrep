# Pass 6 — Happy-path control flow (source call-traces)

| Field | Value |
|-------|-------|
| Loop | 6 / happy-path-control-flow |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (product freeze retained; HEAD may hold prior pass books) |
| Axes | representation:**call-trace** · observer:**runtime** · scale:**entrypoint→sink** · time:**normal** |
| Axes vs pass 5 | **4** (requirements→properties / user+caller / system→function / docs+source+tests → this set) |
| Mode | audit (no product edits) |
| Evidence | source-level static traces (no invented runtime timings/MRR); zerostack engines unavailable (`fszero-codemode` missing) |

**Success** here means: parse → dispatch → open store/searcher or indexer → channel/cascade work → format envelope → return ok. Error branches deferred to pass 7.

---

## Path index

| ID | Surface | Entry | Shared sink | Primary invariants |
|----|---------|-------|-------------|-------------------|
| **HP-CLI-SEARCH** | CLI | bare `QUERY` / `search` | `Searcher::search` → hybrid | INV-LIMITS, INV-CASCADE-*, INV-RANK-FUSION, INV-INDEX-PATH-PREC, INV-EMBED-ALLOW (if embed) |
| **HP-MCP-SEARCH** | MCP | `tools/call` keyword/ast/semantic | channel-split `Searcher::*` | INV-MCP-SANDBOX, INV-LIMITS, INV-MCP-SEARCHER-INV (warm path) |
| **HP-CM-CALL** | Code Mode / NAPI | `Session.call` / `CodeModeSession::call` | catalog → `session.search` → hybrid | INV-CM-ROOT-FREE, INV-LIMITS (surface 500), INV-CASCADE-*, INV-RO-CATALOG (advisory) |
| **HP-PI-ASGREP** | Pi extension | tool `asgrep` / `asgrep_search` | sticky NAPI → HP-CM-CALL | INV-XOR-CM-MCP (host), INV-CM-ROOT-FREE, INV-EDIT-ROOT (sibling edit only) |
| **HP-INDEX** | CLI / MCP / CM | `index` / `index_repo` | `Indexer::index_all` | INV-INDEX-PATH-PREC/PRIV, INV-DURABILITY-FC, INV-MCP-SEARCHER-INV / INV-CM-SEARCHER-INV |
| **HP-CASCADE** | core (shared) | `Searcher::search` Hybrid arm | `search_hybrid` → RRF → `finish_response` | INV-CASCADE-NO-WIDEN, INV-CASCADE-STRUCT-EMPTY (**C1**), INV-RANK-FUSION |

---

## HP-CLI-SEARCH — CLI hybrid search

### Intent
User/agent runs `asgrep "auth refresh"` (bare) or `asgrep search "…"` / `keyword` / `semantic`.

### Call chain (happy)

```
bin/asgrep.rs::main
  → cli/lib.rs::main  [unix: supervisor wrap → run_process]
  → run_process: Cli::try_parse_from → run_cli
  → run_cli:
       command=None  → run_default_search          (lib.rs ~337)
       Search(_)     → search_cmd::run_search(..., semantic=false)
       Keyword(_)    → run_keyword_search
       Semantic(_)   → run_search(..., semantic=true)
  → open_searcher(root, cli)                       (index_cmd.rs ~245)
       ensure_existing_root  (dir exists; unambiguous --root vs positional)
       search_options → SearchOptions { root, index_path, limit clamp, embed flags… }
       Searcher::new(opts)                         (search/mod.rs ~110)
         validate_search_feature_flags
         canonicalize root; clamp limit 1..=MAX_OUTPUT_RESULTS
         IndexStore::open → try_index_db_path      (INV-INDEX-PATH-PREC)
       ensure_nonempty_index
  → do_search_with_cli → do_search
       semantic? search_semantic : search          (search_cmd.rs ~195)
  → Searcher::search(query)                        (search/mod.rs ~386)
       validate_query_arg (MAX_QUERY_CHARS)        (INV-LIMITS)
       ParsedQuery::parse → QueryMode::Hybrid (typical bare NL)
       → search_hybrid + intent.route_hits + apply_weighted_rrf   [see HP-CASCADE]
       → finish_response_checked
  → print: format_hit_line | plugins format_response_with_budget
```

### Anchors

| Step | File:symbol |
|------|-------------|
| process entry | `crates/ast-sgrep-cli/src/bin/asgrep.rs` `main` |
| dispatch | `crates/ast-sgrep-cli/src/lib.rs` `run_cli` / `run_default_search` / `run_command` |
| open | `crates/ast-sgrep-cli/src/index_cmd.rs` `open_searcher` / `search_options` |
| search body | `crates/ast-sgrep-cli/src/search_cmd.rs` `run_search` / `do_search_with_cli` |
| core | `crates/ast-sgrep-core/src/search/mod.rs` `Searcher::search` |

### Invariants on path

| INV | Role on happy path | Enforcement |
|-----|--------------------|-------------|
| INV-LIMITS | query len + limit clamp | **enforced** in `Searcher::new` / `validate_query_arg` |
| INV-INDEX-PATH-PREC | index DB resolution | **enforced** `try_index_db_path` |
| INV-INDEX-PATH-PRIV | abs path may leave root | **unenforced** containment (GAP labeled) |
| INV-CASCADE-NO-WIDEN | hybrid file set | **enforced** inside `search_hybrid` |
| INV-CASCADE-STRUCT-EMPTY | empty structural | **code B** (lexical fallback) -- doc A contradicted (C1) |
| INV-RANK-FUSION | RRF after cascade | **enforced** `apply_weighted_rrf` then `finish_response` |
| INV-EMBED-ALLOW | only if embed HTTP used | **enforced** in embedder (not on no-embed path) |
| INV-MCP-SANDBOX | n/a (CLI has OS-user FS) | not applicable |

### Divergences
- CLI has **no** MCP-style workspace jail; root is OS-user trust.
- Default bare query is **hybrid**; MCP has **no** hybrid tool (channel-split only).

---

## HP-MCP-SEARCH — MCP tools/call search (channel-split)

### Intent
Agent host issues JSON-RPC `tools/call` with `keyword_search` | `ast_search` | `semantic_search` | deprecated `code_search`.

### Call chain (happy)

```
asgrep-mcp main.rs::main
  → McpServer::from_env                     (lib.rs ~193)
       ASGREP_ROOT canonicalize → self.root
       ASGREP_INDEX_PATH / limit / embed flags
  → run_stdio: line → handle_request
  → "tools/call" → handle_tools_call
  → dispatch_tool(name, args)
       parse_agent_search → resolve_root → sandbox_root   (INV-MCP-SANDBOX)
       tool_agent_search(mode)
  → searcher_for(root, limit)
       warm Searcher cache keyed by (root, index_path, limit, use_embed)
       Searcher::new if miss
  → mode dispatch:
       Keyword  → searcher.search_lexical
       Ast      → searcher.search("pattern: {query}")
       Semantic → searcher.search_semantic
  → restore_searcher (generation-aware)
  → hits empty? to_compact_miss_json : Compact envelope + optional budget
  → elide_seen_snippets (session memory)
  → JSON-RPC content + structuredContent
```

### Anchors

| Step | File:symbol |
|------|-------------|
| entry | `crates/ast-sgrep-mcp/src/main.rs` |
| server | `crates/ast-sgrep-mcp/src/lib.rs` `McpServer::from_env` / `run_stdio` / `handle_tools_call` / `dispatch_tool` |
| jail | same `sandbox_root` (~547), `resolve_root` (~453) |
| search | `tool_agent_search` (~661) |
| cache | `searcher_for` / `restore_searcher` / `invalidate_searcher_cache` |

### Invariants on path

| INV | Role | Enforcement |
|-----|------|-------------|
| INV-MCP-SANDBOX | every tool root under workspace | **enforced** happy-path gate before search |
| INV-LIMITS | query max + agent limit | **enforced** parse + clamp_agent_limit |
| INV-MCP-SEARCHER-INV | warm cache integrity | **enforced** after mutators; generation on restore |
| INV-CASCADE-* / INV-RANK-FUSION | **not on this path** | MCP never calls hybrid fusion |
| INV-SURFACE-ROOT-PARITY | vs CM | **contradiction** (C2) -- MCP jailed, CM free |

### Divergences (critical)
- **No hybrid cascade** on MCP search tools; three channels are explicit tools.
- Compact agent envelope (`p`/`h`/`~` elision) vs CLI line/plugins formats.
- Searcher **generation** model stronger than Code Mode (Option clear only).

---

## HP-CM-CALL — Code Mode `Session.call` / catalog search

### Intent
In-process host (NAPI, CLI `codemode-serve`/`codemode-batch`, tests) calls `session.call("search", {query,…})`.

### Call chain (happy)

```
[optional NAPI]
  codemode-napi Session::new → CodeModeSession::new (max_calls=10_000)
  Session::call(tool, args)
    → CodeModeSession::call                     (session.rs ~83)
         bump_call (budget)
         tools::call_tool
              ToolName::parse("search"|"semantic"|…)
              Search → session.search
              Semantic → inject semantic_only=true → session.search
              IndexRepo → session.index_repo
              …
  session.search:
       validate_query_len
       limit clamp 1..=500
       root_arg(args)  // PathBuf::from only -- NO sandbox   (INV-CM-ROOT-FREE)
       searcher_for(root, limit)  // warm cache by root/index/embed/open_limit
       semantic_only? search_semantic : search   // hybrid default
       truncate hits to call limit
       format_response_with (AgentCapsule default)
```

CLI sticky serve path: `run_codemode_serve` → `ast_sgrep_codemode::run_serve` → same `CodeModeSession`.

Batch path: `run_batch` → `choose_parallel` forces serial if any non-read_only (INV-BATCH-NO-MUT-PAR) → per-call `call_tool`.

### Anchors

| Step | File:symbol |
|------|-------------|
| API | `crates/ast-sgrep-codemode/src/session.rs` `call` / `search` / `root_arg` / `index_repo` |
| dispatch | `crates/ast-sgrep-codemode/src/tools.rs` `call_tool` / `ToolName` |
| catalog | `crates/ast-sgrep-codemode/src/catalog.rs` (`read_only` metadata) |
| batch | `crates/ast-sgrep-codemode/src/batch.rs` `choose_parallel` |
| NAPI | `crates/ast-sgrep-codemode-napi/src/lib.rs` `Session::call` |
| CLI bridge | `crates/ast-sgrep-cli/src/lib.rs` `run_codemode_serve` / `run_codemode_batch` |

### Invariants on path

| INV | Role | Enforcement |
|-----|------|-------------|
| INV-CM-ROOT-FREE | free root | **consistent with design**; no jail gate |
| INV-SURFACE-ROOT-PARITY | vs MCP | **CONTRADICTION** on happy success with foreign root |
| INV-RO-CATALOG | mutator approval | **GAP** -- `call` does not check `read_only` |
| INV-BATCH-NO-MUT-PAR | batch serial mutators | **enforced** when using batch API |
| INV-CM-SEARCHER-INV | clear cache after index | **code present**; generation weaker than MCP |
| INV-LIMITS | 500 surface max | **enforced** (tighter than core 1000) |
| INV-CASCADE-* / RANK | via `Searcher::search` | same as CLI hybrid |

### Divergences
- Default format **AgentCapsule** vs MCP Compact vs CLI human/native.
- Limit ceiling **500** (schema/catalog) vs MCP agent clamp vs CLI 1000.
- `root_arg` unsandboxed by design (`docs/codemode.md`).

---

## HP-PI-ASGREP — Pi tool → sticky NAPI → Code Mode

### Intent
Pi agent invokes primary tool `asgrep` (JS Code Mode program) or one-shot `asgrep_search` / `asgrep_index`.

### Call chain (happy) — primary `asgrep`

```
packages/pi/extension/src/index.ts  registerTool("asgrep")
  execute:
    freshness.ensureFresh(warmRuntime, cwd)   // may call index_repo if stale
    resolveRoot(ctx.cwd)
    pool.acquire(root)   // NativeSessionPool
      1) loadCodemodeNative → new binding.Session({root,…})  // NAPI
      2) else CLI sticky worker (codemode-serve) if binary
    createAsgrepConnector(batchHost)
      typed asgrep.search/semantic/chain/defs/…/indexRepo
      → dispatcher.host.call(tool, args)
           sticky worker.call → NAPI Session.call → HP-CM-CALL
           or CLI batch/serve
    runCodemode(params.code, bundle.asgrep)   // VM runs model JS
    return details { result, stats, backend }
```

### One-shot `asgrep_search`

```
asgrep_search.execute
  → freshness.ensureFresh
  → sticky.call(...searchToolCall(params))  // maps mode → tool name
     or runCli(searchArgs)
  → success envelope
```

### One-shot `asgrep_index`

```
asgrep_index.execute
  → sticky.call("index_repo", {force}) | CLI index/reindex --json
  // no host approval gate on index_repo  (INV-RO-CATALOG GAP)
```

### Anchors

| Step | File:symbol |
|------|-------------|
| tools | `packages/pi/extension/src/index.ts` tools `asgrep` / `asgrep_search` / `asgrep_index` |
| connector | `packages/pi/extension/src/codemode/connector.ts` `createAsgrepConnector` |
| pool | `packages/pi/extension/src/codemode/session-pool.ts` `NativeSessionPool` |
| native load | `packages/pi/extension/src/codemode/native.ts` |
| edit sibling | `packages/pi/extension/src/edit.ts` `planEdit` (INV-EDIT-ROOT; not on search path) |

### Invariants on path

| INV | Role | Enforcement |
|-----|------|-------------|
| INV-XOR-CM-MCP | don't co-load MCP | **docs/social only** (GAP) |
| INV-CM-ROOT-FREE | Pi root = project cwd | free relative to OS; pool keys by resolved root |
| INV-RO-CATALOG | index from model | **GAP** -- sticky `index_repo` always callable |
| INV-EDIT-ROOT | only on `asgrep_edit` | **enforced** on edit path, not search |
| cascade/rank | via CM search | same as HP-CM-CALL / HP-CASCADE |

### Divergences
- Dual backend: **NAPI preferred**, CLI sticky degraded.
- Freshness hook may insert **index write** before search (side-effect on "search" UX).
- Model-authored JS composition layer absent from CLI/MCP.

---

## HP-INDEX — Index / reindex write path (multi-entry → one sink)

### Intent
Build or refresh on-disk index so subsequent searches see content.

### Converging chains

```
A) CLI
   Commands::Index|Reindex
     → with_index → open_indexer → Indexer::new(index_options)
     → index_all | reindex_all
     → print_index_stats / machine JSON
   (no Searcher warm cache in CLI process)

B) MCP
   tools/call index_repo
     → parse_index_repo → resolve_root → sandbox_root
     → tool_index_repo
          index_lock single-flight + INDEX_REPO_DEADLINE
          Indexer::new → index_all | reindex_all
          invalidate_searcher_cache (+ generation++)
          clear path_registry + emitted_snippets
     → stats JSON

C) Code Mode / Pi
   call("index_repo", {force?, root?})
     → session.index_repo
          root_arg (unsandboxed)
          Indexer::new(EmbedBackend::Auto, …)
          index_all | reindex_all
          invalidate_searcher_cache  // Option::None only; no generation
     → {ok, force, stats}
```

### Shared sink (core)

```
Indexer::new(IndexOptions)
  canonicalize root
  IndexStore::open_with_durability(..., durability)   (INV-DURABILITY-FC via parse)
  IgnoreMatcher

Indexer::index_all
  collect_index_candidates (walk + gitignore)
  par prepare_file (hash / tree-sitter extract / embed chunks if enabled)
  begin_bulk_tx → commit_prepared_files → apply_bulk_write_result
  rebuild_dirty_sidecars (tantivy/ANN/IVF as needed)
  post_index_hooks
  → IndexStats
```

Embed HTTP (if cloud/ollama): `embed_url_is_allowed` (INV-EMBED-ALLOW) before network.

### Anchors

| Step | File:symbol |
|------|-------------|
| CLI | `cli/lib.rs` `Commands::Index|Reindex`; `index_cmd.rs` `with_index` / `open_indexer` / `index_options` |
| MCP | `mcp/lib.rs` `tool_index_repo` (~861) |
| CM | `codemode/session.rs` `index_repo` (~248) |
| core | `core/src/index.rs` `Indexer::new` / `index_all` / `reindex_all` |
| path | `core/src/store/mod.rs` `try_index_db_path` |

### Invariants on path

| INV | Role | Enforcement |
|-----|------|-------------|
| INV-INDEX-PATH-PREC | DB path order | **enforced** |
| INV-INDEX-PATH-PRIV | abs path privilege | **GAP** labeling / tests |
| INV-DURABILITY-FC | unknown durability | **enforced** parse/from_env |
| INV-MCP-SANDBOX | MCP root only | **enforced** on B |
| INV-CM-ROOT-FREE | CM/Pi foreign root index | **allowed** on C |
| INV-MCP-SEARCHER-INV | post-mutation cache | **enforced** + tests on B |
| INV-CM-SEARCHER-INV | post-mutation cache | **code**; weaker generation (**GAP** test) |
| INV-BATCH-NO-MUT-PAR | if batched with readers | **enforced** batch API |
| INV-RO-CATALOG | host approval | **GAP** on C/Pi |
| INV-EMBED-ALLOW | embed during index | **enforced** if HTTP embed |

### Divergences
- MCP: single-flight + soft deadline + path/elision registry clear.
- CM: no deadline/single-flight at session layer; simpler invalidation.
- CLI: process-local; no warm Searcher to invalidate.
- Pi freshness may auto-index without explicit `asgrep_index`.

---

## HP-CASCADE — Hybrid constraint cascade (shared core sink)

### Intent
Unprefixed hybrid query fuses lexical → structural → optional semantic, then ranks.

### Call chain (happy; inside `Searcher::search` Hybrid arm)

```
ParsedQuery::parse → QueryMode::Hybrid
  if intent == Literal (quoted): literal_pass only
  else:
    search_hybrid(parsed):
      A. literal_prefilter_pass → lexical_files
         if empty → []  (cascade stops; no widen)
      B. structural_index_pass + symbol_pass_for_files
         + anchor_pass_for_files + AST matches
         → structural_files ⊆ lexical_files
      C. working_files =
           structural_files.empty? lexical_files : structural_files
           // ht1h.3 -- DOC CONTRADICTION C1 / INV-CASCADE-STRUCT-EMPTY
         lexical.retain ∈ working_files
         hits = lexical ∪ structural
      D. if use_embed && early_exit_reason(hits).is_none():
           embed_pass_for_files(..., &working_files)
           // INV-CASCADE-NO-WIDEN: files gated by working_files
         else skip semantic (plan.skipped)
      record_plan
    intent::route_hits
    fusion::apply_weighted_rrf(hits, weights_for(intent))   // INV-RANK-FUSION
    finish_response_checked:
      dedup, file_filter, signal margins, confidence, sort, truncate limit
```

### Anchors

| Step | File:symbol |
|------|-------------|
| dispatch | `search/mod.rs` `Searcher::search` Hybrid arm (~410–426) |
| cascade | `search/mod.rs` `search_hybrid` (~480–538) |
| RRF | `crates/ast-sgrep-core/src/fusion.rs` `apply_weighted_rrf` |
| finish | `search/mod.rs` `finish_response` / `finish_response_checked` (~726+) |
| tests | `crates/ast-sgrep-core/tests/cascade_planner.rs` (containment + empty-structural-nonempty) |
| docs | `docs/cascade-query-planner.md` (claims empty structural → stop -- C1) |

### Invariants on path

| INV | Status on this path |
|-----|---------------------|
| INV-CASCADE-NO-WIDEN | **enforced** (`embed_pass_for_files` + lexical retain) |
| INV-CASCADE-STRUCT-EMPTY | **CONTRADICTION** docs A vs code/tests B (lexical fallback + optional semantic) |
| INV-RANK-FUSION | **enforced** after cascade |
| INV-EMBED-ALLOW | only when embed stage actually HTTP-embeds query |

### Who reaches HP-CASCADE?
- **Yes:** CLI default/search (non-semantic-only), CM `search`, Pi `asgrep.search` / natural `asgrep_search`.
- **No:** MCP keyword/ast/semantic tools; CLI `keyword` / `semantic`; CM `semantic` tool.

---

## Cross-path comparison (observer: runtime)

| Concern | CLI | MCP | Code Mode / NAPI | Pi |
|---------|-----|-----|------------------|-----|
| Default retrieval | hybrid | channel tools | hybrid `search` | hybrid via CM |
| Root isolation | OS user | **sandbox** | **free** | free (+ project cwd) |
| Warm Searcher | per-process open | gen-aware cache | Option cache | sticky Session |
| Index invalidate | n/a (cold) | gen + registries | clear Option | via CM |
| Mutator gate | user ran CLI | host trusts MCP | **none** | **none** |
| Output | human/plugins | Compact | AgentCapsule | CM envelope + Pi details |

---

## Residual observations for pass 7 (error flow)

Not expanded here; name only:

1. MCP `sandbox_root` fail-closed escape errors vs CM foreign-root **success**.
2. Empty index: CLI `ensure_nonempty_index` bail vs MCP miss envelope with `indexed_files`.
3. `index_repo` deadline / single-flight contention (MCP only).
4. Codemode `max_calls` budget / NAPI lock poison.
5. Cascade empty lexical → empty hits (stop) vs empty structural → continue (C1).
6. Embed allowlist failures mid-index / mid-search.
7. Feature-flag fail-closed (`validate_search_feature_flags` e.g. rerank without feature).
8. Pi sticky miss → CLI fallback argv errors; freshness rebuild failures.
9. Batch mixed mutator/reader forced serial -- error ordering / partial results.
10. Query length / unknown tool / parse errors at each boundary.
