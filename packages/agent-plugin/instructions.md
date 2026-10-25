# Instruction drops — teach the retrieval habit

Paste the block below into the harness's instruction file (`CLAUDE.md`,
`AGENTS.md`, `.cursorrules`, or equivalent). It carries the same division of
labor the Hermes plugin and MCP tool descriptions use, so hosts without skill
support still reach for asgrep at the right moment.

MCP server tool names are used below; on Hermes the tools are
`asgrep_search` / `asgrep_read` / `asgrep_status`.

```markdown
## Code retrieval — asgrep first
- Know a name, even partially? `search` the fragment; definitions rank first.
- Want relationships? Chain channels off what you found: `defs:X` → `callers:X` → `imports:X`.
- `code_read` expands returned IDs into exact source — don't re-grep for the body.
- Raw strings, logs, config values: your normal text search is still right.
- One repo, one persistent index; it warms as the session works (`index_status` to check).
```
