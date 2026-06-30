# Comparison: ast-sgrep vs ast-grep vs ripgrep

Three tools, three jobs. ast-sgrep is the **navigation and intent layer** you add when you need persistent, structured understanding of a codebase — not a replacement for fast grep or structural codemods.

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
| **Structural patterns** | Yes (`pattern:` → ast-grep) | Native | No |
| **Polyglot AST** | 8 languages, unified index | Yes | Text only |
| **CI / platform JSON** | GitHub & GitLab shapes | No | `--json` (ripgrep format) |
| **LSP** | `asgrep-lsp` | Separate ecosystem | No |
| **Agent-oriented JSON** | `--format agent` + follow-ups | Limited | Line-based JSON |
| **Typical latency** | ~0.3 ms/search (indexed) | Pattern-dependent | ms–s per full scan |
| **Index required** | Yes (`.asgrep/`) | No | No |
| **API key for semantic** | No (offline default) | N/A | N/A |

## When to use which

| You want to… | Reach for |
|---|---|
| Ask *“where does X happen?”* across a whole repo | **ast-sgrep** |
| Find who calls a function or where a symbol is defined | **ast-sgrep** |
| Query with different words than the code uses (*“credential renewal”*) | **ast-sgrep** |
| Feed ranked, structured hits to an AI agent | **ast-sgrep** (`--json --format agent`) |
| Jump to defs/refs/call hierarchy in an editor | **ast-sgrep** (`asgrep-lsp`) |
| Rewrite code with AST-aware rules | **ast-grep** |
| Match a syntactic shape (`class $C { $$$ }`) | **ast-grep** or `asgrep "pattern:…"` |
| Grep logs, configs, or any file type fast | **ripgrep** |
| One-off regex across unindexed or generated files | **ripgrep** |
| Search inside a single huge file without indexing | **ripgrep** |

## Stack positioning

```
┌─────────────────────────────────────────────────────────┐
│  Your workflow                                          │
├─────────────────────────────────────────────────────────┤
│  ripgrep        →  scan anything, no setup             │
│  ast-grep       →  patterns & codemods                  │
│  ast-sgrep      →  persistent navigation + intent       │
└─────────────────────────────────────────────────────────┘
```

**ast-sgrep complements the others.** It delegates structural queries to ast-grep via `pattern:` and does not compete with ripgrep on raw scan speed over arbitrary unindexed files.

## Feature deep dive

### Persistent index vs stateless scan

**ripgrep** reads files on every invocation. Best when files change constantly, you need one-off searches, or you are searching outside a project tree.

**ast-sgrep** amortizes parse cost into `.asgrep/index.db`. Best when you search the same repo repeatedly — terminal, LSP, or agent loops. Incremental updates keep the index fresh with hash + mtime skipping.

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
- Optional neural upgrade (Ollama, cloud)

ast-grep matches **structure**, not **meaning**. ripgrep matches **text**, not **intent**.

### Structural patterns

ast-grep is the specialist. ast-sgrep exposes it:

```bash
asgrep "pattern:fn $NAME($$$)"
```

Requires ast-grep CLI installed. Results appear as `PATTERN` hits in ast-sgrep output.

### Output for automation

| Tool | JSON shape | Agent affordances |
|------|------------|-------------------|
| ast-sgrep | `native`, `agent`, `github`, `gitlab` | `follow_up_queries`, `suggested_next`, `stack_hint` |
| ast-grep | Scan result JSON | Pattern-oriented |
| ripgrep | Match lines | No symbol/graph context |

## Performance expectations

| Scenario | ast-sgrep | ripgrep | ast-grep |
|----------|-----------|---------|----------|
| First-time full-repo search | Index build + fast query | Full scan | Full scan per pattern |
| Repeated queries same repo | ~sub-ms (indexed) | Full scan each time | Full scan each time |
| 10k-file monorepo NL query | Indexed + optional IVF | Seconds per scan | Not applicable |

ast-sgrep pays an upfront indexing cost; ripgrep pays per scan. Choose based on query frequency and whether you need graph/semantic ranking.

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
