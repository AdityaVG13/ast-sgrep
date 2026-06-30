# Agent & AI integration

ast-sgrep is designed as an **intent layer** alongside ripgrep and ast-grep. Use it when an agent needs ranked, structured context — not raw grep lines.

## Quick start for agents

```bash
# Index once (semantic on by default, no API key required)
asgrep index .

# Agent-optimized JSON (default for `semantic` subcommand)
asgrep semantic "credential renewal" --json

# Hybrid search with agent format
asgrep --json --format agent "where is auth refreshed"
```

## Output format: `--format agent`

```json
{
  "provider": "ast-sgrep",
  "version": "1.1",
  "query": "credential renewal",
  "hit_count": 3,
  "has_semantic_hits": true,
  "stack_hint": "Use ast-sgrep for intent/navigation; ast-grep for patterns; ripgrep for grep.",
  "suggested_next": [
    "asgrep semantic \"credential renewal\"",
    "defs:auth_refresh",
    "callers:auth_refresh"
  ],
  "hits": [{
    "kind": "embed",
    "semantic": true,
    "score": 3.42,
    "file": "src/main.rs",
    "lines": { "start": 19, "end": 22 },
    "symbol": "auth_refresh",
    "excerpt": "fn auth_refresh() { ... }",
    "follow_up_queries": ["defs:auth_refresh", "callers:auth_refresh"]
  }]
}
```

Each hit includes `follow_up_queries` so agents can drill into defs/callers without guessing syntax.

## Tool stack for LLM pipelines

| Task | Tool | Example |
|------|------|---------|
| Natural language / synonyms | **asgrep** | `asgrep semantic "persist access token"` |
| Symbol definitions | **asgrep** | `asgrep "defs:process_request"` |
| Caller graph | **asgrep** | `asgrep "callers:main"` |
| Structural patterns | **ast-grep** | `asgrep "pattern:fn $NAME($$$)"` |
| Raw text / logs | **ripgrep** | `rg "ERROR" logs/` |

## LSP for editor-integrated agents

```json
{
  "command": "asgrep-lsp",
  "initializationOptions": {
    "asgrep": {
      "annThreshold": 2000,
      "embedBackend": "auto"
    }
  }
}
```

Execute commands:

```json
{ "command": "asgrep.search.semantic", "arguments": ["credential renewal"] }
{ "command": "asgrep.search", "arguments": ["callers:main"] }
```

## No API key required

Without `ASGREP_EMBED_API_KEY`, indexing and search use the **offline semantic local** embedder (concept expansion + symbol chunks). Cloud and Ollama are optional upgrades — not required for the default workflow.

## Recommended agent loop

1. `asgrep index .` — build persistent index
2. `asgrep --json --format agent "<user intent>"` — ranked hits with follow-ups
3. For each symbol of interest: `defs:` and `callers:` queries
4. For structural rewrites: delegate to ast-grep via `pattern:`
5. For unindexed files or logs: ripgrep
