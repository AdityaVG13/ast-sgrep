# Comparison: ast-sgrep vs ast-grep vs ripgrep vs Semgrep (vs tgrep, fff)

For **searching an indexed repo**, ast-sgrep replaces the scan tools. Identifiers, strings, structural shapes, defs/callers, and conceptual NL are one ranked list. The other tools remain specialists for jobs that are not ranked search.

## Summary

| | **ast-sgrep** | **[ast-grep](https://github.com/ast-grep/ast-grep)** | **[ripgrep](https://github.com/BurntSushi/ripgrep)** |
|---|:---:|:---:|:---:|
| **Primary goal** | Navigate & understand codebases | Structural search & codemods | Fast text search |
| **Search model** | Persistent SQLite index + hybrid ranking | Pattern match per run | Streaming regex scan |
| **Natural-language queries** | Yes | No | No |
| **Synonym / semantic queries** | Yes (default on) | No | No |
| **Symbol definitions** | Yes (`defs:`) | Via pattern only | No |
| **Caller / callee graph** | Yes (`callers:`) | No | No |
| **Import tracking** | Yes (`imports:`) | No | No |
| **Structural patterns** | Native indexed subset (`pattern:`) | Native (full rules/rewrites) | No |
| **Polyglot AST** | 15 languages, unified index | Yes | Text only |
| **CI / platform JSON** | GitHub & GitLab shapes | No | `--json` (ripgrep format) |
| **LSP** | `asgrep-lsp` | Separate ecosystem | No |
| **Agent-oriented JSON** | `--format agent` + follow-ups | Limited | Line-based JSON |
| **One-shot CLI latency** | 8.5 ms p95 (`literal:`, indexed) | 61.3 ms p95 (pattern, hand-written code) | 13.1 ms p95 (scan) |
| **Index required** | Yes (`.asgrep/`) | No | No |
| **API key for semantic** | No (offline default) | N/A | N/A |

Latency cells are the 2.5.0 re-pin (self corpus, 650 indexed files,
`release-perf`): [head-to-head](../benchmarks/results/head-to-head.md).
Warm-server tools are compared separately below. A cold one-shot
process and a warm server are different contests.

### Also in the arena: tgrep and fff

**tgrep** (warm trigram server,
1.0.9) answers the same `SearchHit` query in **5.8 ms p95**. It leads
asgrep's 8.5 ms cold one-shot. asgrep's own warm path (`codemode-serve`,
0.75 ms p50) is the apples-to-apples contest, not the one-shot CLI.

**fff** (warm ranked file-finder, fff-mcp 0.10.6) answers in **0.4 ms
p95** but surfaces ~50 ranked matches where exhaustive tools find ~300
lines. It owns sub-ms ranked file-finding, not exhaustive grep. Compare
latency only, never match sets.

## When to use which

| You want to… | Reach for |
|---|---|
| Find a token, a definition, a caller, a shape, or an idea in an indexed repo | **ast-sgrep** |
| Ask *“where does X happen?”* | **ast-sgrep** |
| Query with different words than the code uses (*“credential renewal”*) | **ast-sgrep** |
| Feed ranked, structured hits to an AI agent | **ast-sgrep** (`--json --format agent`) |
| Jump to defs/refs/call hierarchy in an editor | **ast-sgrep** (`asgrep-lsp`) |
| Match a syntactic shape (`class $C { $$$ }`) | **ast-sgrep** `pattern:` (native indexed subset) |
| Rewrite code with full ast-grep YAML rules | **ast-grep** |
| Run SAST rule packs | **Semgrep** |
| Grep logs, configs, or an unindexed tree | **ripgrep** |
| Search inside a single huge file without indexing | **ripgrep** |

## Stack positioning

```
┌─────────────────────────────────────────────────────────┐
│  Indexed repo: search                                   │
│    asgrep  (hybrid / defs / callers / pattern / NL)     │
├─────────────────────────────────────────────────────────┤
│  Not search                                             │
│    ripgrep   → logs / unindexed / generated             │
│    ast-grep  → full-rule rewrites                       │
│    Semgrep   → SAST rule packs                          │
└─────────────────────────────────────────────────────────┘
```

On an indexed tree, do not spawn a second search tool. `pattern:` is a native
indexed subset (not an ast-grep subprocess). ripgrep still wins raw scan of
unindexed bytes; that is a different job. See `docs/structural-patterns.md`.

## Search quality (the product bar)

A coding agent in an indexed repo should never need a second search tool.
Ranking, not CLI milliseconds vs `rg`, is the contest:

- Exact / PascalCase identifiers rank the definition first (`Searcher` before
  `bench_searcher`).
- Conceptual NL prefers code over markdown that repeats the query
  (`credential renewal` → `auth_refresh`, not the README).
- Vocabulary expansion is a precision tool, not a co-occurrence firehose.
- Semantic has to retrieve on this repo, not only `tests/fixtures/sample`.

Engine pieces that enforce that:

- A deterministic post-fusion **critic** gates embed-only hits on
  corroboration, boosts multi-channel agreement, demotes generic entrypoint
  callers, and explains every hit (`why` on agent JSON). See
  `docs/fusion-ranking.md`.
- **Causal follow-ups**: `follow_up_queries` are derived from the actual
  top hit (kind, symbol, margin).
- **Two-channel conjunction**: `pattern:... AND callers:x` performs a
  span-level graph/structure join; `AND NOT` subtracts. See
  `docs/QUERY_GRAMMAR.md`.

`pattern:` does not claim full ast-grep rule identity. Keep-gates vs pinned
ripgrep / ast-grep are opt-in (`ASGREP_DIFF_RG`, `ASGREP_DIFF_AST_GREP`) and
measure match-set overlap, not "throw those binaries away for every task."

### Agent policy for indexed source

Do not spawn `rg` for source already covered by a current ast-sgrep index. Use
`literal:` for exact substring presence and the normal hybrid query for ranked
navigation. Keep a terminal index current with `asgrep watch`; Pi and Code Mode
refresh before search, while LSP applies open/change/save/close document updates
before its next request. Ripgrep remains appropriate for logs and unindexed or
unsupported files. ast-sgrep never invokes it as a hidden fallback.

## Feature deep dive

### Persistent index vs stateless scan

**ripgrep** reads files on every invocation. Best when files change constantly, you need one-off searches, or you are searching outside a project tree.

**ast-sgrep** amortizes parse cost into `.asgrep/index.db`. Best when you search the same repo repeatedly, terminal, LSP, or agent loops. Incremental updates keep the index fresh with hash + mtime skipping.

**ast-grep** walks the tree per pattern run. Excellent for CI codemods; not optimized for *“show me everything about auth_refresh”* as a single ranked view.

### Graph awareness

Only **ast-sgrep** builds a **caller/callee graph** at index time:

```bash
asgrep "callers:process_request"
asgrep "defs:auth_refresh"
```

ast-grep can match call *syntax* with patterns but does not maintain a queryable graph. ripgrep can regex for a name but cannot distinguish definition from reference reliably across languages.

### Semantic / intent

Only **ast-sgrep** ships a **semantic pass** by default:

- Symbol-chunk embeddings with call-graph context
- Offline concept expansion (no API key)
- Optional in-process neural upgrade (ONNX MiniLM / BGE; never HTTP)

ast-grep matches **structure**, not **meaning**. ripgrep matches **text**, not **intent**.

### Structural patterns

ast-grep is the specialist for YAML rules, rewrites, and deeply nested templates. ast-sgrep ships a native subset for indexed signatures, declaration/call shapes, and single-statement nested templates (`fn $N($$$) { $STMT }`, `if ($COND) { $BODY }`):

```bash
asgrep "pattern:fn $NAME($$$)"
asgrep 'pattern:if ($COND) { $BODY }'
```

This does **not** require the ast-grep CLI. Unsupported shapes return no hits rather than spawning a subprocess. Results appear as `PATTERN` hits. See `docs/structural-patterns.md`.

### Output for automation

| Tool | JSON shape | Agent affordances |
|------|------------|-------------------|
| ast-sgrep | `native`, `agent`, `github`, `gitlab` | `follow_up_queries`, `suggested_next`, `stack_hint` |
| ast-grep | Scan result JSON | Pattern-oriented |
| ripgrep | Match lines | No symbol/graph context |

## Performance expectations

Measured cells (2.5.0 re-pin, self corpus, 650 indexed files;
see [head-to-head](../benchmarks/results/head-to-head.md)):

| Scenario | ast-sgrep | ripgrep | ast-grep |
|----------|-----------|---------|----------|
| Cold index the repo | 14.9 s p95 (one-time) | No index | No index |
| One-shot token query | 8.5 ms p95 (`literal:`, indexed) | 13.1 ms p95 (scan) | N/A |
| One-shot structural query | 9.5 ms p95 (`pattern:`, indexed) | N/A | 61.3 ms p95 (hand-written code) |
| Warm-server query | 0.75 ms p50 (`codemode-serve`) | N/A (no server) | N/A (no server) |

ast-sgrep pays an upfront indexing cost; ripgrep pays per scan. Choose based on query frequency and whether you need graph/semantic ranking. On trees with large generated files, scan/parse-per-query tools pay per query while the index does not (ast-grep 279.8 ms with the generated file included vs 61.3 ms without, while asgrep stays at 9.5 ms).

## Migration mental model

| Coming from | ast-sgrep equivalent |
|-------------|---------------------|
| `rg auth_refresh` | `asgrep "auth_refresh"` or `asgrep "defs:auth_refresh"` |
| `rg -l` for files | `asgrep --json` → aggregate by `file` |
| ast-grep `fn $NAME` | `asgrep "pattern:fn $NAME"` |
| “Ask Copilot where X is” | `asgrep --json --format agent "where is X"` |

## Related docs

- [Getting started](getting-started.md)
- [Semantic search](semantic-search.md)
- [Use cases](use-cases.md)
