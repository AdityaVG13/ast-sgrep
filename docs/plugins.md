# Output format plugins

`ast-sgrep-plugins` adapts native search JSON for CI and platform integrations.

## CLI usage

```bash
# Native (default)
asgrep --json "auth refresh"

# GitHub code search API shape
asgrep --json --format github "process_request"

# GitLab code search API shape
asgrep --json --format gitlab "auth refresh"
```

## Formats

| Format | Flag | Top-level keys |
|--------|------|----------------|
| Native | `native` (default) | `query`, `limit`, `hits` |
| GitHub | `github`, `gh` | `total_count`, `items`, `provider` |
| GitLab | `gitlab`, `gl` | `data`, `query`, `provider` |

## Library usage

```rust
use ast_sgrep_core::Searcher;
use ast_sgrep_plugins::{format_response, OutputFormat};

let response = searcher.search("auth refresh")?;
let github = format_response(&response, OutputFormat::GitHub);
```

Each hit preserves ast-sgrep metadata (`kind`, `score`, caller/callee) in a `metadata` / `meta` field for agent pipelines.
