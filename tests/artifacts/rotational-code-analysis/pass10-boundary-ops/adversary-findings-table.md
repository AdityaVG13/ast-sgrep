# Pass 10 — Adversary-oriented findings table

| Field | Value |
|-------|-------|
| Loop | 10 / boundary-adversary + ops |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may hold books) |
| Axes | representation:**attack-surface+ops** · observer:**attacker+operator** · scale:**boundary** · evidence:**source+config** |
| Mode | audit (no product edits under crates/ or packages/) |
| Evidence path | native rg/read (zerostack fszero engines unavailable — B-ZS-ENGINES) |

Threat model (reaffirmed from pass 4): **local OS user / installing agent**. No multi-tenant authn. Adversary is untrusted model argv/JSON/plan, co-resident process under same UID, or misconfigured env. No invented remote CVE classes.

## Severity legend

| Sev | Meaning |
|-----|---------|
| high | Durable wrong index / secret disclosure / multi-writer silent staleness under realistic host deploy |
| medium | Privilege/config footgun or crash window; host-dependent |
| low | Soft consistency, docs, or observability-only gap |
| info | Positive control / reaffirmation |

Status: **CONSISTENT** (control holds) · **GAP** (missing control or untested) · **CONTRADICTION** (two policies disagree) · **BY-DESIGN** (named intentional, residual is host duty)

---

## Findings (prioritized)

| Rank | ID | Severity | Status | Path / locus | Evidence | Residual / residual risk |
|------|-----|----------|--------|--------------|----------|---------------------------|
| 1 | **C2** / **BY-CM-ROOT** × concurrent index | high | **CONTRADICTION** (parity) + **GAP** (host) | `crates/ast-sgrep-codemode/src/session.rs` `root_arg` L105–111; `index_repo` L248–266; MCP `sandbox_root` L547–567 | CM: `args.root` → raw `PathBuf`, no under-workspace check. MCP: canonicalize + `starts_with(&self.root)`. Same `Indexer`/`Searcher`. Free root + **session `index_path`** (config/env) walks foreign tree and writes **relative paths** into the shared DB (`collect_index_candidates` strip_prefix root; `prune_missing_files` on success). Concurrent search on pinned path sees mixed/pruned corpus; `set_meta("root", …)` overwrites meta root. Plan `$ref` can bind prior output into `root` (`plan.rs` resolve_value). | Host must jail `root` before Session/NAPI; no product test for foreign-root + shared `ASGREP_INDEX_PATH` prune. Pass 11: dual-evidence for **R-CODEMODE-ROOT-UNSANDBOXED** only if product intent flips to jail. |
| 2 | **GAP-WATCH-XPROC** multi-writer | high | **GAP** | CLI `watch.rs` L9–80; MCP `tool_index_repo` single-flight only in-process L861–920; CM no xproc lock | `asgrep watch` mutates index via `update_paths` / `index_all`; stderr-only progress. MCP/CM warm Searcher + generation not invalidated by external writer. SQLite WAL allows concurrent readers; app caches do not. Deploy shape: watch + MCP agent + CI `index` on same DB. | No flock/lease across processes; no filesystem notify to MCP. Ops residual: document single-writer or share invalidate channel. |
| 3 | **CL-MID-SIDECAR-CACHE** / **RW-MCP-MID-SIDECAR** | high | **GAP** | `index.rs` `index_all`: bulk commit then `rebuild_dirty_sidecars` L281–284; MCP invalidate only **after** `index_all()?` Ok L882–890 | On post-commit sidecar Err, SQLite already committed; MCP/CM skip `invalidate_searcher_cache` and `path_registry` clear. Agent sees `isError:true` text; disk advanced; warm Searcher pre-mutation. | Same as pass 9; boundary axis: error surface lies relative to durable state (ops false-negative). |
| 4 | **INV-INDEX-PATH-PRIV** / **GAP-INDEX-PATH-DOC** | medium–high | **BY-DESIGN** + **GAP** docs/test | `store/mod.rs` `try_index_db_path` L198–214; `index.rs` `generation_layout_root` L542–546; `reindex_all` L527–540 | Explicit `index_path` / `ASGREP_INDEX_PATH` accepted **anywhere** writable; **no** under-root constraint. Pinned path **disables** generation atomic reindex → in-place `clear_all_data` crash window (CL-PINNED-REINDEX). Privilege = OS write to that path. | Document as privileged sink; warn when pin disables gen layout. Residual tests for escape/absolute pin. |
| 5 | **BY-REGISTRY-STALE** / **CL-INDEX-FAIL-REGISTRIES** | medium | **GAP** | MCP `path_registry` clear only on index Ok L889–890; `resolve_compact_id` L820–832; `read_node` L955–984 | Failed/mid-sidecar index leaves compact id→path map. `code_read` still re-validates path under **current** sandboxed root (starts_with + canonicalize + TOCTOU) — not raw FS escape. Soft capability: stale ids resolve to wrong relative files under root or fail. | Clear registries on any index attempt end (success or Err). |
| 6 | **FastUnsafe** ops footgun | medium | **BY-DESIGN** (named) + **GAP** ops | `store/mod.rs` Durability L18–72; `write_pragma` FastUnsafe → `OFF`; CLI `cli_args.rs` L200–209; clap/env fail-closed unknown | Opt-in `ASGREP_DURABILITY=fast-unsafe` / `--durability fast-unsafe`. Steady state restores NORMAL; power loss during write batch can corrupt index. MCP/CM inherit `Durability::from_env()` via `IndexOptions::default` — no per-tool confirmation. Status exposes `durability` string. | Operator may set env globally for CI speed and forget in agent MCP. Doctor does not flag FastUnsafe as issue. |
| 7 | **GAP-EMBED-REDIR-IT** (reaffirm control) | info / low residual | **CONSISTENT** hop-final | `embed/src/embedder.rs` `embed_url_is_allowed` L27+; agent `redirects(0)` L144–152; tests L621–676 | Allowlist blocks metadata IP/file/non-listed hosts; HTTP non-loopback needs insecure flag; **no redirects** so allowlist is final hop. Unit tests pin SSRF targets + redirects:0. | Residual: live IT under retry/timeout still optional; not a product hole given unit pin + comments. |
| 8 | **GAP-CM-ROOT** test absence | medium | **GAP** | codemode tests: batch parallelization present; **no** root-escape fail-closed test | Catalog advertises free `"root"` override on mutators. | Add adversarial test only if contract becomes jail; else document host duty in codemode.md / security. |
| 9 | **ESC-3** deadline post-mutate | low–med | known semantic | MCP `tool_index_repo` invalidate then deadline ensure L886–918 | Index durable + soft deadline Err → agent sees failure after success work. | Observability: error string does not say "index committed". |
| 10 | **C1** cascade docs | low (docs) | **CONTRADICTION** retained | hybrid empty-structural continues (pass 6–8) | Not boundary primary; retained ledger. | Doc fix pass 11+ optional. |
| 11 | **B-SECURITY-NAPI-DOC** | low | **GAP** | NAPI inherits CM free root | Docs incomplete on NAPI = full-user indexer. | Document. |
| 12 | **GAP-XOR-RUNTIME** / **GAP-RO-HOST** | low | **GAP** | Code Mode XOR MCP is policy/docs only | Host can run both against same index → multi-writer. | Runtime detect optional. |

### Positive controls (boundary)

| ID | Status | Note |
|----|--------|------|
| MCP `sandbox_root` | **CONSISTENT** | fail-closed escape |
| MCP `read_node` path jail + TOCTOU | **CONSISTENT** | relative-only node id; canonicalize under root |
| MCP deny unknown wire fields | **CONSISTENT** | serde strict wire |
| Embed allowlist + no-redirect | **CONSISTENT** | reaffirmed |
| Durability unknown env/clap | **CONSISTENT** | fail-closed parse (not silent downgrade) |
| CM batch no mutator∥reader parallel | **CONSISTENT** | `choose_parallel` |
| MCP index single-flight in-process | **CONSISTENT** | `index_lock` |
| SQL ident allowlists | **CONSISTENT** | j97d.045r |
| Integrity_check quarantine corrupt | **CONSISTENT** | open path |

---

## Attack narratives (boundary × time)

### N1 — Model free root + pinned index (C2 × index)

1. Host sets `ASGREP_INDEX_PATH=/proj/.asgrep/index.db`, starts CM/NAPI Session with config root=/proj.
2. Model calls `index_repo` with `root=/tmp/evil-tree` (or `$ref` path from prior tool).
3. Indexer walks evil tree; relative keys land in pinned DB; prune removes proj paths absent from evil tree.
4. Concurrent `search` on same Session (shared index_path) returns evil or empty corpus; meta `root` points at evil.
5. MCP would have refused step 2; CM does not.

### N2 — Watch + MCP dual writer

1. Operator runs `asgrep watch .` for live index.
2. Agent MCP `agent_search` warms Searcher generation G.
3. Watch commits updates; MCP Searcher still generation G until process-local invalidate (never).
4. Agent answers from pre-watch snapshot; `status`/doctor on CLI may show new counts — split brain.

### N3 — FastUnsafe brownout

1. CI exports `ASGREP_DURABILITY=fast-unsafe`.
2. Same env inherited by long-running MCP/CM.
3. Power/kill mid bulk write → possible corrupt DB; open path quarantines to `index.db.corrupt` and fails closed (good), but recovery cost is full reindex; no pre-flight doctor warning.

### N4 — Mid-sidecar lie

1. `index_repo` commits bulk; tantivy/IVF rebuild fails.
2. Tool returns error text; Searcher cache not advanced/cleared.
3. Operator retries; agent may still serve pre-mutation hits until cache key churn.
