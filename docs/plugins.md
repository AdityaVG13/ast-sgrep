# Output format plugins

`ast-sgrep-plugins` adapts native search JSON for CI, platform, and agent integrations.

## CLI usage

```bash
# Native (default)
asgrep --json "auth refresh"

# Agent / LLM tool-calling (follow-up query hints)
asgrep --json --format agent "credential renewal"

# GitHub code search API shape
asgrep --json --format github "process_request"

# GitLab code search API shape
asgrep --json --format gitlab "auth refresh"

# Semantic-only (defaults to agent JSON when --json)
asgrep semantic "credential renewal" --json
```

## Formats

| Format | Flag | Top-level keys |
|--------|------|----------------|
| Native | `native` (default) | `query`, `limit`, `hits` |
| Agent | `agent`, `llm`, `ai` | `hits`, `suggested_next`, `has_semantic_hits`, `stack_hint` |
| GitHub | `github`, `gh` | `total_count`, `items`, `provider` |
| GitLab | `gitlab`, `gl` | `data`, `query`, `provider` |

See [agent.md](agent.md) for AI integration patterns.

## Library usage

```rust
use ast_sgrep_core::Searcher;
use ast_sgrep_plugins::{format_response, OutputFormat};

let response = searcher.search("auth refresh")?;
let agent = format_response(&response, OutputFormat::Agent);
```

Each hit preserves ast-sgrep metadata (`kind`, `score`, caller/callee) and agent format adds `follow_up_queries` per hit.
