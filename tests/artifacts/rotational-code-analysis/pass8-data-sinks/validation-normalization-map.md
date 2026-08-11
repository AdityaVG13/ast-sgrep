# Pass 8 — Validation / normalization map

Maps each critical data class: order of checks, normalization, encoding, ranges, aliasing, lifetime, redaction. Links to sinks in `source-to-sink-traces.md`.

## Summary matrix

| Data | Normalize | Bounds | Authz / isolation | Encoding | Redaction | Lifetime |
|------|-----------|--------|-------------------|----------|-----------|----------|
| Query text | MCP `trim` | 1..=4096 MCP; ≤4096 core; regex ≤4096 | n/a (content free) | UTF-8 chars count | none | request + caches |
| Root (MCP) | canonicalize | must exist + is_dir | under `ASGREP_ROOT` | OS path | n/a | per call |
| Root (CM/CLI) | optional later | exists (CLI) | **none** (OS user) | OS path | n/a | session / process |
| Node id | compact expand | line numbers, relative components | join+canon under root + TOCTOU | path string UTF-8 | n/a | registry session; read ephemeral |
| Index path | `as_db_path` | existence optional | **no root jail** | PathBuf | n/a | process env + args |
| Embed URL | trim, lower host | scheme/host allowlist | SSRF allowlist + no redirects | URL string | API key Debug redacted | env / request |
| Edit path | repair quotes/NFC | replace XOR write | `containedInRoot` + device deny | UTF-8 file I/O | n/a | single edit |
| Plan `$ref` | strip `$` | step graph only | none (composition) | JSON | n/a | plan run |
| Durability | parse lower | known enum only | fail-closed default Balanced | env str | n/a | open time |
| Ast-grep bin | absolute path | ALLOW + is_file | dual opt-in | path | n/a | bench only |

---

## Query text

| Step | Site | Rule |
|------|------|------|
| 1 | MCP tools/list schema | `minLength` 1, `maxLength` MAX_QUERY_CHARS (host-side) |
| 2 | `AgentSearchWire` | `deny_unknown_fields` |
| 3 | `parse_agent_search` | `trim`; non-empty; char count ≤ 4096 |
| 4 | CM `search`/`chain` | `validate_query_len` |
| 5 | `Searcher::*` | `validate_query_arg` → same len; empty OK |
| 6 | regex pass | `MAX_REGEX_PATTERN_CHARS` before compile |
| 7 | limit | clamp surface-specific (100 / 500 / 1000) |

**Units:** Unicode scalar chars (not bytes) for query length.  
**Nullability:** missing query → hard error on agent surfaces; empty string rejected MCP, allowed core.  
**Aliasing:** cache keys include raw query; expansions are separate metadata.

---

## Root / path

### MCP `sandbox_root`

| Step | Rule |
|------|------|
| exist? | fail if missing |
| canonicalize | resolve symlinks |
| `starts_with(self.root)` | Path component prefix (not string prefix) |
| is_dir | required |

**Does not re-run after tool body** — tools must not reintroduce raw candidate.

### CM `root_arg`

`PathBuf::from(str)` only. Normalization deferred to Searcher/Indexer open.

### CLI

Unambiguous root selection + directory exists. No multi-tenant jail.

---

## Node ids

| Step | Rule |
|------|------|
| ids count | 1..=20 |
| context_lines | 0..=100 |
| max_chars | 1..=1_000_000 (default applied if absent) |
| compact resolve | digit range + registry path |
| parse_node_id | `#Lstart-Lend`; start>0; end≥start; canonical decimal; relative components only |
| join pre-check | `unresolved.starts_with(root)` |
| canonicalize | must stay under root |
| type | regular file |
| TOCTOU | metadata identity + re-canon equal |
| scan | ≤ 64 MiB; UTF-8 lines |
| output | char truncate to budget |

**Normalization:** line numbers as u32→usize; path not cleaned beyond component filter (no `..`).

**Registry lifetime:** filled on successful search envelopes; cleared on successful `index_repo`; **not** cleared on index Err (see CL-INDEX-FAIL-REGISTRIES). Cap 4096 keys.

---

## Index path

| Step | Rule |
|------|------|
| explicit | win immediately via `as_db_path` |
| env | `ASGREP_INDEX_PATH` next |
| generation | active manifest if DB exists |
| legacy local | if exists |
| cache | only if `ASGREP_USE_CACHE` truthy **and** HOME/XDG set |
| default | local path even if missing |

**No** tenant/root dimension retained on absolute paths (security dimension drop vs project root).

---

## Embed URL / key

| Step | Rule |
|------|------|
| scheme | http/https only |
| host parse | strip userinfo, port, brackets; lowercase |
| allowlist | defaults + `ASGREP_EMBED_URL_ALLOWLIST` CSV |
| http policy | loopback free; else `ASGREP_EMBED_ALLOW_INSECURE_HTTP` |
| agent | `redirects(0)` |
| endpoint join | re-allowlist Ollama embeddings URL |
| key | env only; Header at send; Debug `<redacted>` |

**Query/chunk body:** no redaction before send (operator chooses embed).

---

## File edit (Pi)

| Step | Rule |
|------|------|
| parseEditParams | shape + XOR modes |
| repairEditPath | trim, smart quotes, NFC, strip wrapping quotes |
| resolve | `path.resolve(projectRoot, raw)` |
| device deny | `/dev/*`, `/proc/*/fd/*` |
| containedInRoot | `relative` not `..` / absolute |
| apply | size/binary checks on replace; mkdir on write create |

**Note:** repair runs **before** containment — must not produce escape (resolve+containedInRoot closes).

---

## Plan `$ref`

| Step | Rule |
|------|------|
| prefix `$` | only strings starting with `$` resolve |
| parts | step id then `.field` / array index |
| failure | unknown id / missing path / non-container |

No encoding change; values cloned as JSON.

---

## Cross-surface limit divergence (not bypass of MAX)

| Surface | limit clamp | query |
|---------|-------------|-------|
| Core Searcher | 1..=1000 | ≤4096 |
| MCP agent | 1..=100 | 1..=4096 |
| Code Mode | 1..=500 | ≤4096 |
| CLI | core clamp | core |

Documented as surface policy, not validation invalidation.

---

## Transformations that drop security dimensions

| Transform | Dimension lost | Residual |
|-----------|----------------|----------|
| CM accepts free `root` | workspace jail | C2 / BY-CM-ROOT |
| `try_index_db_path` absolute | project containment | GAP-INDEX-PATH-PRIV |
| path_registry after failed index | freshness of id→path map | CL-INDEX-FAIL-REGISTRIES |
| query → embed body | local-only processing | intentional egress |
| `$ref` into next args | any prior validation of string as non-path | host must treat as untrusted again |
