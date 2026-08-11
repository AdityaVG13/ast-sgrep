# Pass 8 — Source-to-sink traces (data provenance)

| Field | Value |
|-------|-------|
| Loop | 8 / data-provenance-validation-and-sinks |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (product freeze retained) |
| Axes | representation:**dataflow** · observer:**data-owner+adversary** · scale:**source→sink** · evidence:**source+schema** |
| Axes vs pass 7 | **4** (exception-graph / failure-handler / entrypoint→cleanup / degradation → this set) |
| Mode | audit (no product edits under crates/ or packages/) |
| Evidence | source+schema+tests; zerostack engines unavailable (`fszero-codemode` missing, B-ZS-ENGINES) |

**Gate:** Every critical sink has a traced source and enforcement path; pattern-only suspicions are not findings.

Legend: **V** = validation site · **T** = transform · **S** = sink · **BY** = bypass / non-enforcement

---

## Path index

| ID | Data class | Primary sources | Critical sinks | Linked INV |
|----|------------|-----------------|----------------|------------|
| **DF-QUERY** | query text | CLI argv, MCP wire, CM/NAPI args, Pi tools, LSP | search engines, embed HTTP body, response/miss JSON, caches | INV-LIMITS, INV-EMBED-ALLOW, INV-CASCADE-*, INV-RANK-FUSION |
| **DF-ROOT** | root / project path | MCP `root`, CM `root`, CLI `--root`/positional, Pi `projectRoot`, LSP workspace | WalkDir index, Searcher open, code_read root, edit root | INV-MCP-SANDBOX, INV-CM-ROOT-FREE, INV-SURFACE-ROOT-PARITY (C2) |
| **DF-NODE** | node ids / compact ids | agent `code_read.ids`, path_registry from search envelopes | `File::open` → agent content JSON | INV-MCP-SANDBOX, INV-EDIT-ROOT (sibling), CL-INDEX-FAIL-REGISTRIES |
| **DF-INDEX** | index DB path | explicit `index_path`, `ASGREP_INDEX_PATH`, generation, `.asgrep`, cache | SQLite create/open/write | INV-INDEX-PATH-PREC, INV-INDEX-PATH-PRIV, INV-DURABILITY-FC |
| **DF-EMBED** | embed URL / env / API key | `ASGREP_EMBED_*`, Ollama env, allowlist | ureq POST + `Authorization` header | INV-EMBED-ALLOW |
| **DF-FILE** | file contents (read/edit/index) | disk under root; model `contents`/`old_string`/`new_string` | agent JSON, `writeFile`, SQLite chunks | INV-EDIT-ROOT, INV-MCP-SANDBOX |
| **DF-PLANREF** | plan `$ref` values | prior step JSON outputs | next tool args (query/root/…) | INV-CM-ROOT-FREE, INV-RO-CATALOG |
| **DF-CMD** | external command path | `ASGREP_AST_GREP` + allow flag | `Command::new` spawn (bench) | INV-AST-GREP |

---

## DF-QUERY — query text

### Sources (untrusted / high-control)

| Surface | Ingress |
|---------|---------|
| CLI | bare `QUERY`, `search` / `keyword` / `semantic` positional |
| MCP | `AgentSearchWire.query` via `tools/call` (`deny_unknown_fields`) |
| Code Mode / NAPI | `args.query` string on `search` / `chain` / catalog tools |
| Pi | sticky NAPI → CM path; CLI subprocess argv for non-NAPI |
| LSP | search/query params on executeCommand / workspace methods |

### Validation order

1. **Wire schema (MCP):** `deny_unknown_fields` + tools/list `minLength:1` / `maxLength: MAX_QUERY_CHARS` (advisory for hosts; runtime re-checks).
2. **MCP parse:** `query.trim()`; non-empty; `chars().count() <= MAX_QUERY_CHARS` (`parse_agent_search`).
3. **CM/NAPI:** `validate_query_len` before searcher (`session.rs` search/chain).
4. **Core (all surfaces that open Searcher):** `validate_query_arg` → `validate_query_len` at each `search*` entry (`search/mod.rs`); empty allowed at core (no hits).
5. **Regex arm:** pattern length `MAX_REGEX_PATTERN_CHARS` (4096) before `Regex::new` (`search/passes/regex.rs`).
6. **Limit (orthogonal):** clamp 1..=MAX_OUTPUT_RESULTS (1000) core; agent MCP `MAX_AGENT_LIMIT` 100; CM surface clamp 500.

### Transforms

- `ParsedQuery::parse` / mode prefixes (`literal:`, `regex:`, `pattern:`, hybrid NL).
- Lexicon `prose_terms` → `query_expansions` (response metadata; not re-validated as new queries).
- Hybrid cascade stages retain query identity; embed pass embeds **query string** as vector input.

### Sinks

| Sink | What leaves the process / trust domain |
|------|----------------------------------------|
| **S-SEARCH-ENGINE** | Query drives FTS / structural / hybrid ranking inside SQLite + in-process engines |
| **S-EMBED-HTTP** | `embed_via_api`: JSON `{model, input: text}` where `text` is the query; `Authorization: Bearer <key>`. `embed_via_ollama`: `{model, prompt}`. |
| **S-RERANK** | Query + hit docs into rerank helper (optional) |
| **S-RESPONSE** | Echoed in hit envelopes / agent JSON / miss `why` context |
| **S-CACHE-KEY** | Searcher response cache key and process-wide query-embed cache key include query bytes |

### Bypass / residual

| ID | Notes | Class |
|----|-------|-------|
| BY-QUERY-CONTENT | No content allowlist on query before embed egress; **URL** allowlisted, **payload** is free-form up to 4096 chars | **CONSISTENT** with design (INV-EMBED-ALLOW is host SSRF, not DLP) |
| BY-QUERY-EXPANSION | Expansions derived from query + lexicon are not re-capped as separate queries | low residual |
| BY-REGEX-CPU | Length-capped regex still can be pathological (no ReDoS budget beyond length) | residual → pass 9/20 |

**Enforcement vs INV-LIMITS:** **CONSISTENT** — length gate evidenced at MCP + CM + core + unit tests.

---

## DF-ROOT — root / path arguments

### Sources

| Surface | Field | Trust assumption |
|---------|-------|------------------|
| MCP | optional `root` on every tool | untrusted agent; must stay under `ASGREP_ROOT` |
| Code Mode / NAPI | optional `root` | host/process OS user; **no jail** (docs) |
| CLI | `--root` / positional ROOT | local operator |
| Pi edit / code-mode | `projectRoot` from host session | host-trusted workspace |
| LSP | workspace folder | editor-trusted |

### Validation order (MCP)

1. Wire `root: Option<String>` → `PathBuf::from`.
2. `resolve_root` → `sandbox_root`:
   - must **exist** (else fail closed);
   - `canonicalize`;
   - `canonical.starts_with(&self.root)` (component-wise `Path` prefix);
   - must be directory.
3. Downstream tools use **canonical** root only.

### Validation order (Code Mode)

1. `root_arg`: `args.root` string → `PathBuf::from` **or** session `config.root`.
2. **No** canonicalize-under-config check.
3. Passed into `Searcher::new` / `Indexer` which may canonicalize for open.

### Validation order (CLI)

1. `ensure_unambiguous_root` (no dual `--root` + positional conflict).
2. `ensure_existing_root` — exists as directory; no workspace jail.

### Sinks

| Sink | Effect |
|------|--------|
| **S-INDEX-WALK** | `WalkDir::new(root)` → read project files into index |
| **S-SEARCH-OPEN** | Index open + search scoped by root |
| **S-CODE-READ-ROOT** | MCP `code_read` joins node paths under this root |
| **S-EDIT-ROOT** | Pi `planEdit(projectRoot)` containment base |

### Bypasses

| ID | Path | Classification |
|----|------|----------------|
| **BY-CM-ROOT** | Model-supplied absolute `root` on CM `index_repo`/`search` → full OS-user FS | **CONSISTENT** with INV-CM-ROOT-FREE / docs; **CONTRADICTION** with INV-SURFACE-ROOT-PARITY (C2) |
| **BY-CLI-ROOT** | Operator CLI has no jail (expected local tool) | CONSISTENT local trust |
| **BY-INDEX-PATH** | Index DB path independent of root containment (see DF-INDEX) | GAP privilege label |
| MCP symlink | canonicalize then `starts_with` — outside symlink fails | CONSISTENT (not a bypass) |

---

## DF-NODE — node ids and compact path ids

### Sources

1. Agent-supplied `ids[]` on MCP `code_read` (and Pi code-mode ref strings).
2. Server-side **path_registry** filled from compact search envelopes via `resolve_compact_paths` → `remember_compact_paths` (max 4096 entries).

### Validation order (MCP `code_read`)

1. Wire: `deny_unknown_fields`; ids length 1..=20; budgets for `context_lines` / `max_chars`.
2. `resolve_root` → sandbox (DF-ROOT).
3. Per id: `resolve_compact_id` (if compact `path_id:start-end` and registry hit → `path#Lstart-Lend`; else passthrough).
4. `read_node`:
   - `parse_node_id`: must end `#Lstart-Lend`; numeric lines; **no** `ParentDir` / only `Normal`|`CurDir` components (relative only; absolute rejected).
   - `root.join(file)` then `starts_with(root)` pre- and post-`canonicalize`.
   - regular file only; open; **TOCTOU**: `same_opened_file` + re-canonicalize equality.
   - line range within EOF; UTF-8; scan cap `MAX_SCAN_BYTES` (64 MiB); char truncate `max_chars`.

### Sinks

| Sink | Effect |
|------|--------|
| **S-FILE-READ** | File slice → JSON `{id,file,lines,content,truncated}` to agent |

### Bypasses / soft capabilities

| ID | Notes | Class |
|----|-------|-------|
| **BY-REGISTRY-STALE** | On `index_repo` **Err** before clear, path_registry may retain pre-mutation id→path (CL-INDEX-FAIL-REGISTRIES). Read still re-validates path under current sandboxed root | **GAP** soft capability / consistency, not raw escape |
| **BY-REGISTRY-TRUST** | Registry values come from **server-produced** search envelopes (hit files), not raw agent path table injection into `p` | CONSISTENT if agent cannot forge server envelope mid-session without going through search |
| Compact miss | Unknown path_id leaves opaque id → `parse_node_id` fails closed | CONSISTENT |

Pi `resolveReadableFile` mirrors: `inside(root)`, `realpath`, device-path deny, stable open — sibling to MCP.

---

## DF-INDEX — index database path

### Sources (precedence = INV-INDEX-PATH-PREC)

1. Explicit `index_path` argument / CLI `--index-path`
2. Env `ASGREP_INDEX_PATH` (MCP/CM `from_env` also preload into config)
3. Active generation under `root/.asgrep` if DB exists
4. Legacy `root/.asgrep/index.db` if exists
5. `ASGREP_USE_CACHE` → `XDG_CACHE_HOME`/`HOME` hashed path (refuses shared `/tmp` without home)
6. Else return local path (may not exist yet — create on write)

`as_db_path`: if path has no `.db` extension, append `index.db`.

### Validation

- **Order only** — no `starts_with(root)` on absolute paths.
- Cache path requires HOME/XDG (fail closed vs `/tmp`).
- Durability profile via `Durability::from_env` (unknown → Balanced default) at SQLite open.

### Sinks

| Sink | Effect |
|------|--------|
| **S-SQLITE-OPEN** | Create/open DB at resolved path |
| **S-SQLITE-WRITE** | Index mutations, embeddings tables, bulk writes |
| **S-CROSS-ROOT-DB** | Absolute env path can place DB **outside** project root (multi-tenant footgun / intentional fixture isolation) |

### Bypasses

| ID | Notes | Class |
|----|-------|-------|
| **BY-INDEX-ABS** | Absolute `ASGREP_INDEX_PATH` not contained under tool root | **GAP** INV-INDEX-PATH-PRIV (behavior intentional; privilege unlabeled in operator docs) |
| MCP env preload | `self.index_path` from env at server start, not re-sandboxed per tool call | same GAP; operator env is trusted plane |

---

## DF-EMBED — embed URL, env, API key

### Sources

| Env / config | Role |
|--------------|------|
| `ASGREP_EMBED_API_URL` | Cloud endpoint (default OpenAI) |
| `ASGREP_EMBED_API_KEY` | Bearer secret |
| `ASGREP_EMBED_MODEL` | model name in JSON body |
| `ASGREP_EMBED_URL_ALLOWLIST` | extra hosts (operator privilege) |
| `ASGREP_EMBED_ALLOW_INSECURE_HTTP` | non-loopback http |
| `ASGREP_OLLAMA_URL` / `ASGREP_OLLAMA_EMBED` | local embed |
| `ASGREP_EMBED_FALLBACK` / neural flags | chain policy |

### Validation order

1. Parse scheme (`http`/`https` only); extract host (strip userinfo/port/IPv6 brackets).
2. Host ∈ default allowlist ∪ env allowlist.
3. `http` + non-loopback requires `ASGREP_EMBED_ALLOW_INSECURE_HTTP`.
4. On request: re-check URL; Ollama re-check after `/api/embeddings` join.
5. HTTP agent `redirects(0)` so allowlist is final hop (no 30x to link-local).

### Sinks

| Sink | Data |
|------|------|
| **S-EMBED-POST** | Query (and at index time, **file chunk text**) → remote model provider |
| **S-AUTH-HEADER** | API key in `Authorization` only (not Debug; redacted in `CloudEmbeddingConfig::fmt`) |
| **S-EMBED-CACHE** | Vectors in SQLite embed cache / process query-embed map |

### Bypasses

| ID | Notes | Class |
|----|-------|-------|
| Redirect SSRF | Blocked by `redirects(0)` | **CONSISTENT** INV-EMBED-ALLOW |
| Evil host | Unit tests deny metadata/file/evil hosts | CONSISTENT |
| Operator allowlist expand | Intentional privilege elevation | CONSISTENT operator trust |
| GAP-EMBED-REDIR-IT | Live redirect IT still listed residual (policy in code + unit) | low GAP |

---

## DF-FILE — file contents (code_read / edit / index)

### Read path (MCP `code_read` / Pi code-mode)

See DF-NODE. Content leaves as agent-visible JSON (exfil to model context is host/agent boundary).

### Write path (Pi `asgrep_edit`)

1. `parseEditParams`: object; path string; **replace XOR write** (not both/neither).
2. `repairEditPath` (quotes/NFC) — repair **before** trust boundary finalize.
3. `planEdit`: `resolve(projectRoot, path)` → `assertSafeEditTarget` (device/proc fd deny) → `containedInRoot`.
4. `applyEdit`: `writeFile` or read+replace+write; size/binary guards on replace.

### Index path (shared core)

`Indexer::index_all` / `index_file`: WalkDir under root → read source → tree-sitter extract → SQLite (+ optional embed of **chunk text** → DF-EMBED sinks).

### Sinks

| Sink | Notes |
|------|-------|
| **S-AGENT-CONTENT** | code_read / search excerpts to model |
| **S-DISK-WRITE** | Pi edit under projectRoot |
| **S-INDEX-CORPUS** | Source text stored in index DB (may be off-root via DF-INDEX) |
| **S-EMBED-CHUNKS** | Indexed file text may egress via embed API |

### Bypasses

| ID | Notes | Class |
|----|-------|-------|
| CM no code_read jail twin | CM returns search excerpts without MCP `read_node` TOCTOU stack; root still free | C2 residual |
| Edit path repair | Repair narrows model mistakes; does not widen outside root after `containedInRoot` | CONSISTENT INV-EDIT-ROOT |

---

## DF-PLANREF — Code Mode plan `$ref`

### Flow

`run_plan` → per step `resolve_value` on args: strings starting with `$` → `resolve_ref` walks prior step outputs by `.` path / array index only.

### Validation

- Unknown step id / missing path → `InvalidArgs`.
- **Not** a filesystem API; no path sandbox of its own.

### Sink / bypass

Resolved values feed **next tool args**. If a prior step returns a path string and a later step binds it to `root`, **BY-CM-ROOT** applies. Classification: intentional composition under INV-CM-ROOT-FREE; elevates host duty for model-supplied roots.

---

## DF-CMD — external ast-grep binary

### Flow

`find_ast_grep_binary`: requires `ASGREP_ALLOW_AST_GREP` **and** absolute existing `ASGREP_AST_GREP` file; version probe; then `Command::new` with pattern + root (bench).

### Classification

**CONSISTENT** INV-AST-GREP — dual opt-in; no PATH search.

---

## Cross-cutting validation order (MCP tools/call)

```
JSON-RPC line
  → method tools/call
  → name dispatch
  → serde deny_unknown_fields (wire)
  → field bounds (query len, limit, ids, budgets)
  → resolve_root → sandbox_root          [path authz]
  → tool body
       search: Searcher → validate_query_arg → engines / optional embed
       code_read: resolve_compact_id → parse_node_id → join+canonicalize+TOCTOU → File::open
       index_repo: Indexer → try_index_db_path (no root jail on DB path) → disk write → invalidate caches
  → envelope / miss / error
```

Later transforms that could **invalidate** earlier validation:

| Transform | Re-check? | Risk |
|-----------|-----------|------|
| compact id → path#L… | re-validated in `parse_node_id` + join | low if registry only server-filled |
| Ollama URL + `/api/embeddings` | re-`embed_url_is_allowed` | covered |
| plan `$ref` into `root` | no re-jail on CM | C2 |
| `as_db_path` join | no containment | INDEX-PRIV |
| query → regex compile | length only | ReDoS residual |

---

## Evidence anchors (absolute paths)

| Symbol | Path |
|--------|------|
| `sandbox_root` | `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-mcp/src/lib.rs` ~547 |
| `parse_node_id` / `read_node` | same file ~901 / ~955 |
| `root_arg` | `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-codemode/src/session.rs` ~105 |
| `try_index_db_path` | `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-core/src/store/mod.rs` ~198 |
| `embed_url_is_allowed` / `redirects(0)` | `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-embed/src/embedder.rs` ~27 / ~152 |
| `validate_query_len` | `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-core/src/limits.rs` ~32 |
| `planEdit` | `/Users/aditya/Developer/ast-sgrep/packages/pi/extension/src/edit.ts` ~177 |
| `resolve_ref` | `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-codemode/src/plan.rs` ~109 |
| Tests | `crates/ast-sgrep-mcp/tests/protocol.rs` (`tool_roots_*`, `code_read_*`, compact id expand); embedder unit allowlist tests |
