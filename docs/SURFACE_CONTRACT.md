# Surface Contract

One engine, four surfaces: the `asgrep` CLI, the `asgrep serve` worker
(codemode session), the MCP server, and host plugins (Hermes, Pi). This
document is the contract that keeps them one product instead of four tools
sharing a name. The parity gate (`tests/mcp/cross_surface_parity.rs`) is the
enforcement mechanism; this file is the spec it enforces.

## Lane vocabulary

| lane | semantics | canonical name |
|---|---|---|
| fused hybrid | lexical + structural + semantic, ranked (definitions first) | `search` |
| keyword/literal | FTS/literal line hits, no fusion | `find` (aliases `grep`, `keyword`) |
| budgeted read | file windows for refs/hit ids | `read` |
| channels | defs / callers / imports / chain / semantic / word / literal / regex / pattern | query prefixes on `search` |

**`search` is the fused lane everywhere.** The keyword lane is a different
tool, not a degraded mode of the same one. Host adapters that expose "fused
search" (the Hermes plugin's `asgrep_search`, the Pi `searchParameters`)
MUST dispatch to `search`, never `find`. Legacy MCP-era names
(`ast_search`, `keyword_search`, `semantic_search`) translate into the
session vocabulary with `recovered` stamps at the codemode boundary.

A bare query (no channel prefix) is interpreted by the fused lane on every
surface. `find`/`grep`/`keyword` prefixed or aliased calls are the keyword
lane on every surface. There is no surface where a bare query means
"literal only" — that was the original drift (CLI/MCP fused vs serve-worker
habit), closed at the plugin dispatch and pinned by the gate.

## Hit contract

Canonical hit fields (the serve-worker rendering):

- `file` — path **relative to the index root**
- `lines.start` / `lines.end` — 1-indexed, inclusive
- `preview` — the hit line(s), capsule-bounded
- `ref` — `file#Lstart-Lend` (read tool accepts this verbatim)
- `score`, `why`, `kind`, `symbol`, `caller`, `callee` — ranking provenance

Surfaces may re-encode for transport (the MCP server's compact v1 wire
format: positional hit arrays with a path table `p` and relative-root table
`r`; the CLI's verbose object envelope) under these rules:

1. **Hit set and order are invariant.** Same index generation + same query
   ⇒ identical hit sequence on every surface (pinned by the gate).
2. **Anchors must resolve unambiguously in-band.** Path abbreviation (MCP's
   package-relative roots) is allowed only when the tables to reconstruct
   the root-relative path ship with the response.
3. **Ranking provenance may be dropped for token budget, never invented.**

## Read/anchor grammar

Canonical: `src/a.rs#L12-L40`. LSP-style `src/a.rs:12` is accepted as an
exact equivalent by every `read` surface. MCP capsule ids (`<path_id>:12-40`)
are a transport encoding of the same span and MUST map back through the
response's path table.

## Parameters

- Default limit is 16 on every surface (`SearchOptions::default_limit`).
- `excerpt_lines` widens previews on serve/plugin/CLI; MCP exposes the same
  behavior as its byte-budget triple (`zb`). The name convergence
  (`excerpt_lines` vs `preview` vs `zb`) is deferred — see
  `docs/progress/surface-deferrals.md` for the retry condition.
- `file_filter` (glob) and `lang` filter identically on every lane that
  supports them.

## Error taxonomy

Two kinds, rendered idiomatically per surface but semantically constant:

- **usage** — the caller's fault (missing/invalid argument). MUST name the
  field and the fix. CLI: `error.kind: "usage"` + exit 1; serve:
  `ok:false` + message; MCP: JSON-RPC `-32602` with the field named.
- **operation** — engine/index failure. MUST carry a recovery command
  (e.g. the exact `asgrep reindex <root> --json` invocation) on every
  surface.

## Freshness and cold roots

- **CLI one-shot**: fail-closed on a missing index — `ok:false`, exit 2,
  structured error naming the `asgrep index` fix. A one-shot query never
  ambiently builds.
- **serve worker**: ambiently builds and incrementally refreshes the index
  for its root; hosts get zero-setup behavior. Freshness cost lands on the
  worker, off the host's call path.
- **Host habit lanes** (Hermes `search_files` takeover): render results in
  the host's stock result shape; the only visible deltas are ranking
  (definitions first) and repo-relative paths. Guardrails, redaction, and
  result shaping belong to the host pipeline and MUST be preserved by
  delegation, not reimplemented.

## Determinism

Repeat runs of the same query against the same index generation return
byte-identical hit sequences on every surface (property pinned by the
gate's construction: one shared index, three arms, sequence equality).

## Enforcement

`tests/mcp/cross_surface_parity.rs` drives CLI, serve worker, and MCP over
one fixture tree and a seven-lane corpus (bare, multiword, `literal:`,
`regex:`/pattern, `defs:`, `callers:`, miss) with per-lane non-vacuity
asserts — a lane that silently empties on all three surfaces fails the
gate. Mutation-proven: dispatch, lane, and normalization mutations each
fail a distinct assert. New surfaces (napi, LSP) join the gate by adding an
arm; new query lanes join by adding a corpus entry plus the same
non-vacuity rule.
