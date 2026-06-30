# ast-sgrep

Polyglot hybrid code search.

```bash
cargo install --path crates/ast-sgrep-cli
asgrep index .
asgrep "where is auth refreshed"
```

**Not** [ast-grep](https://github.com/ast-grep/ast-grep) (codemods). **Not** ripgrep (raw text).

**Is:** keyword search + AST symbol graph context.

Supports **Rust**, **TypeScript**, **JavaScript**, **Python**, and **Go**.

## Tagline

ast-grep finds shapes. **ast-sgrep** finds intent with graph context.

## Install

```bash
git clone https://github.com/ast-sgrep/ast-sgrep
cd ast-sgrep
cargo install --path crates/ast-sgrep-cli
```

Binary name: `asgrep`

## Quick start

```bash
# Build index (stored in .asgrep/index.db)
asgrep index .

# Keyword / natural-language search
asgrep "auth refresh"
asgrep "how does process_request work"

# Prefixed queries
asgrep "callers:main"
asgrep "defs:Handler::serve"
asgrep "imports:serde"

# JSON output for agents / CI
asgrep --json --limit 32 "process_request"

# Index management
asgrep status .
asgrep reindex .
```

## CLI flags

| Flag | Description |
|------|-------------|
| `--root` | Project root (default: `.`) |
| `--limit` | Max results (default: 16, env: `ASGREP_LIMIT`) |
| `--json` | JSON output |
| `--index-path` | Custom index DB path |
| `--lang` | Filter by language (`rust`, `typescript`, `javascript`, `python`, `go`) |

## Output kinds

| Kind | Meaning |
|------|---------|
| `ASGREP` | Lexical line hit |
| `DEF` | Symbol definition |
| `CALLER` | Caller → callee edge |
| `GRAPH` | Graph neighborhood |
| `ANCHOR` | Anchor excerpt around known symbol |
| `IMPORT` | Import statement |

### Line output example

```
ASGREP: src/main.rs:5-5: let _ = process_request("x");
DEF: src/main.rs: process_request span=6..12 | fn process_request(...)
CALLER: src/main.rs: main -> process_request
GRAPH: src/main.rs: main calls process_request
ANCHOR: src/main.rs:6-12: fn process_request(...) { ... }
```

### JSON output example

```json
{
  "query": "how does process_request work",
  "limit": 16,
  "hits": [{
    "kind": "anchor",
    "file": "src/main.rs",
    "line_start": 6,
    "line_end": 12,
    "symbol": "process_request",
    "language": "rust",
    "score": 6.0,
    "excerpt": "fn process_request(...) { ... }"
  }]
}
```

## Architecture

```
ast-sgrep/
  crates/ast-sgrep-core/   # Index + hybrid search engine
  crates/ast-sgrep-cli/    # asgrep binary
  crates/ast-sgrep-lang/   # tree-sitter parsers (Rust, TS, JS, Python, Go)
  tests/fixtures/          # Polyglot test fixtures
```

### Search passes

1. **Lexical** — FTS5 BM25-like line scoring
2. **Symbol + graph** — defs, callers, graph edges
3. **Anchor** — excerpt around matched symbol

Results are fused, deduplicated, and ranked.

### Index

SQLite database at `.asgrep/index.db`:

- `files`, `lines`, `symbols`, `callers`, `imports`
- Incremental single-file reindex (content hash + mtime)
- Respects `.gitignore` and `.asgrepignore`

## Library usage

```rust
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};

let mut indexer = Indexer::new(IndexOptions {
    root: ".".into(),
    index_path: None,
    lang_filter: None,
    respect_gitignore: true,
})?;
indexer.index_all()?;

let searcher = Searcher::new(SearchOptions {
    root: ".".into(),
    index_path: None,
    limit: 16,
    lang_filter: None,
})?;
let response = searcher.search("auth refresh")?;
```

## License

MIT — see [LICENSE](LICENSE).
