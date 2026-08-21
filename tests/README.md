# tests/

All program tests live here. Production crate sources must not contain
`#[test]`, `mod tests`, or `#[cfg(test)] #[path]` stubs.

`ast-sgrep-testkit` is the shared library. Integration tests use it to
index a sample tree and assert search, index, and Pi behavior. Do not add
a unit file for every module.

| Path | What |
|---|---|
| `tests/core/` | Index, store, hybrid/semantic search |
| `tests/cli/` | `asgrep` search, auto-index, machine output, watch |
| `tests/pi/` | Pi extension and launcher |
| `tests/codemode/` | In-process search/index session |
| `tests/lang/` | Extraction used by indexing |
| `tests/mcp/` | MCP search/index protocol |
| `tests/fixtures/` | Shared corpora |

Each crate's `Cargo.toml` points here with `[[test]] path = "../../tests/..."`.
