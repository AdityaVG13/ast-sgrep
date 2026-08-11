# Pass 5 — Invariant ledger (contracts & properties)

| Field | Value |
|-------|-------|
| Loop | 5 / contracts-and-invariants |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained) |
| Axes | representation:requirements→properties · observer:user+caller · scale:system→function · evidence:docs+source+tests |
| Axes vs pass 4 | all four changed (entrypoint/attack-surface/adversary/config → this set) |
| Mode | audit (no product edits) |
| Evidence path | native (zerostack engines unavailable: `fszero-codemode` missing) |

**Status vocabulary:** CONSISTENT | CONTRADICTION | GAP | UNKNOWN

Each record is falsifiable: a concrete violate-oracle and a named source.

---

## INV-MCP-SANDBOX — MCP tool roots stay under workspace

| Field | Content |
|-------|---------|
| **id** | INV-MCP-SANDBOX |
| **statement** | Every MCP tool `root` argument must canonicalize to a directory under the server's configured `ASGREP_ROOT` (or default root); paths outside fail closed with an error mentioning workspace escape. |
| **criticality** | high (authorization / isolation) |
| **source** | doc: `docs/env-trust.md` "MCP workspace root"; code: `crates/ast-sgrep-mcp/src/lib.rs` `sandbox_root`; test: `crates/ast-sgrep-mcp/tests/protocol.rs` `tool_roots_are_sandboxed_under_configured_workspace` |
| **how to violate** | Call any MCP tool with `"root"` set to an existing path outside the configured workspace; expect success without "escapes configured workspace". |
| **enforcement evidence** | `sandbox_root` uses `canonicalize` + `starts_with(&self.root)`; protocol test asserts `isError` and escape string. |
| **status** | **CONSISTENT** |

---

## INV-CM-ROOT-FREE — Code Mode `root` is not workspace-jailed

| Field | Content |
|-------|---------|
| **id** | INV-CM-ROOT-FREE |
| **statement** | Code Mode / NAPI `root` argument is accepted as an OS path with no `sandbox_root`-equivalent check (authority = host + process user, not MCP jail). |
| **criticality** | high if hosts pass model-supplied roots; design-level if multi-root intentional |
| **source** | code: `crates/ast-sgrep-codemode/src/session.rs` `root_arg` (`PathBuf::from` only); doc: `docs/codemode.md` ("no isolate sandbox… orchestration speed, not OS isolation"); prior residual GAP-CM-ROOT |
| **how to violate** | (of a claimed MCP-parity jail) Call `index_repo` / `search` with `root` = foreign absolute path and observe index/search under that path succeeds. |
| **enforcement evidence** | `root_arg` has no `starts_with` / canonicalize-under-config check. No negative codemode test rejects foreign roots. |
| **status** | **GAP** (vs MCP isolation contract); **CONSISTENT** with Code Mode docs that deny OS jail. Cross-surface isolation is **not** uniform. |

Related: **INV-SURFACE-ROOT-PARITY** (below) marks the cross-surface expectation conflict.

---

## INV-SURFACE-ROOT-PARITY — Surfaces share isolation semantics

| Field | Content |
|-------|---------|
| **id** | INV-SURFACE-ROOT-PARITY |
| **statement** | *(Stated/implied by pass-4 trust map / agent expectations of "one core")* Agent surfaces that accept a project `root` enforce the same containment policy. |
| **criticality** | high (authorization consistency) |
| **source** | inferred from dual docs (`docs/env-trust.md` MCP jail vs `docs/codemode.md` no OS jail); pass-4 policy map P1 |
| **how to violate** | Documented: MCP rejects outside root; Code Mode accepts it. |
| **enforcement evidence** | Asymmetric implementations (INV-MCP-SANDBOX vs INV-CM-ROOT-FREE). |
| **status** | **CONTRADICTION** (if parity is required) / **GAP** (no single written parity requirement—only diverging surface contracts). **Marked CONTRADICTION** under user+caller observer: callers can reasonably expect one jail policy. |

---

## INV-INDEX-PATH-PREC — Index DB path resolution order

| Field | Content |
|-------|---------|
| **id** | INV-INDEX-PATH-PREC |
| **statement** | `try_index_db_path(root, index_path)` resolves in order: (1) explicit `index_path` arg, (2) `ASGREP_INDEX_PATH` env, (3) active generation under `root/.asgrep` if present, (4) legacy `root/.asgrep/index.db` if present, (5) cache path if `ASGREP_USE_CACHE` truthy (else still return local path). Explicit always wins over env. |
| **criticality** | high (consistency / isolation) |
| **source** | code: `crates/ast-sgrep-core/src/store/mod.rs` `try_index_db_path`; docs: `docs/getting-started.md` (`--index-path` / `ASGREP_INDEX_PATH`); testkit isolation: `crates/ast-sgrep-testkit/src/isolation.rs` (explicit path must ignore poison env) |
| **how to violate** | Set `ASGREP_INDEX_PATH` to path A, pass explicit `index_path` B; observe open of A. Or with only env unset and generation pointer present, observe wrong generation. |
| **enforcement evidence** | Early `return` on `Some(index_path)` before env read. Testkit asserts session isolation under poison env. |
| **status** | **CONSISTENT** |

---

## INV-INDEX-PATH-PRIV — Absolute index path may leave project root

| Field | Content |
|-------|---------|
| **id** | INV-INDEX-PATH-PRIV |
| **statement** | Absolute `ASGREP_INDEX_PATH` / `--index-path` values are accepted without requiring containment under project `root` (privileged placement is allowed). |
| **criticality** | medium (privilege / multi-tenant footgun) |
| **source** | code: `try_index_db_path` applies `as_db_path` with no `starts_with(root)`; residual U-INDEX-PATH-PRIV / GAP-INDEX-PATH |
| **how to violate** | *(of a "must stay under root" claim)* Point env to `/tmp/other.db` while `root` is a project; observe open/write there succeeds. |
| **enforcement evidence** | No containment check in resolver. Docs list the env var but do not label it as privileged escape. |
| **status** | **GAP** (privilege contract undocumented / untested as security property). Behavior is intentional for isolation fixtures; security labeling is missing. |

---

## INV-MCP-SEARCHER-INV — MCP mutators drop warm Searcher

| Field | Content |
|-------|---------|
| **id** | INV-MCP-SEARCHER-INV |
| **statement** | After MCP `index_repo` mutates the on-disk index, the warm Searcher cache entry is empty **and** the generation counter advances so a stale in-flight Searcher cannot be restored. Path registry and emitted-snippet elision maps clear. |
| **criticality** | high (consistency / correctness) |
| **source** | code: `tool_index_repo` → `invalidate_searcher_cache` before deadline fail-out; tests: `index_repo_invalidates_searcher_after_disk_mutation`, `reindex_generation_rejects_in_flight_stale_searcher`; protocol elision reindex test |
| **how to violate** | Warm searcher, `index_repo`, then observe cache still holding pre-mutation Searcher or generation unchanged. |
| **enforcement evidence** | Unit tests assert `entry.is_none()`, generation advanced, registries empty. |
| **status** | **CONSISTENT** |

---

## INV-CM-SEARCHER-INV — Code Mode mutators drop warm Searcher

| Field | Content |
|-------|---------|
| **id** | INV-CM-SEARCHER-INV |
| **statement** | After Code Mode `index_repo`, `searcher_cache` is cleared so the next search reopens the index. |
| **criticality** | high (consistency) |
| **source** | code: `session.rs` `index_repo` → `invalidate_searcher_cache`; module docs on warm Searcher |
| **how to violate** | Warm searcher on session, mutate index via `index_repo`, search again and observe hits from pre-mutation DB (cache not cleared). |
| **enforcement evidence** | Code clears mutex cache. **No** codemode unit test mirrors MCP's `index_repo_invalidates_searcher_*` (generation model also weaker: no generation counter). |
| **status** | **GAP** (implementation present; parity test + generation restore race untested). Soft-CONSISTENT on happy path code read. |

---

## INV-BATCH-NO-MUT-PAR — Batch never parallelizes mutations with readers

| Field | Content |
|-------|---------|
| **id** | INV-BATCH-NO-MUT-PAR |
| **statement** | When any batch call targets a non-`read_only` catalog tool (e.g. `index_repo`), the batch runs serial even if `parallel`/`parallel_mode` request parallelism. |
| **criticality** | high (consistency / race) |
| **source** | code: `batch.rs` `choose_parallel`; test: `batch_never_parallelizes_index_repo_with_readers` |
| **how to violate** | Batch `search` + `index_repo` with `parallel: true` and observe true parallel execution (shared racey Searcher). |
| **enforcement evidence** | `calls.iter().any(|c| !is_read_only)` forces `false`; dedicated test. |
| **status** | **CONSISTENT** |

---

## INV-RO-CATALOG — `read_only` means host-safe without approval

| Field | Content |
|-------|---------|
| **id** | INV-RO-CATALOG |
| **statement** | Catalog field `read_only: true` means "safe to call from code-execution sandboxes without human approval"; `index_repo` is the sole mutator with `read_only: false`. Hosts that honor PTC `allowed_callers` only auto-allow read-only tools from the code-execution caller. |
| **criticality** | medium (authorization at host boundary) |
| **source** | code: `catalog.rs` comment + `index_repo` flag; adapters set `allowed_callers` **only when** `read_only` (openai/anthropic); residual GAP-RO-FLAG |
| **how to violate** | Session/NAPI/Pi calls `index_repo` with no host approval gate and succeeds (expected today). Host that ignores `read_only` auto-invokes mutator from sandboxed code. |
| **enforcement evidence** | Metadata + adapter hints only. `session.call` does **not** reject non-read-only without approval. Pi sticky `index_repo` is reachable from the model path. |
| **status** | **GAP** (catalog contract is advisory; not runtime-enforced in-process) |

---

## INV-XOR-CM-MCP — Code Mode XOR MCP (one agent surface)

| Field | Content |
|-------|---------|
| **id** | INV-XOR-CM-MCP |
| **statement** | A single agent client must load either Code Mode (`pi-ast-sgrep` / codemode) **or** MCP (`asgrep-mcp` / agent-plugin), never both. |
| **criticality** | medium (operability / model confusion; not a memory-safety issue) |
| **source** | docs: `docs/codemode.md`, `docs/mcp.md`, `docs/ARCHITECTURE.md`, `docs/pi-package.md`; skills: `packages/pi/extension/skills/ast-sgrep/SKILL.md`, `packages/agent-plugin/skills/ast-sgrep/SKILL.md` |
| **how to violate** | Register both surfaces in one Pi/MCP host session; no runtime error from either package. |
| **enforcement evidence** | Documentation and skill prose only. No process-level mutex, no mutual import (coupling avoided) but both can be co-loaded by a host. |
| **status** | **GAP** (social contract; residual GAP-XOR-RUNTIME) |

---

## INV-EMBED-ALLOW — Embed HTTP hosts fail closed on allowlist

| Field | Content |
|-------|---------|
| **id** | INV-EMBED-ALLOW |
| **statement** | Cloud/Ollama embed URLs must pass `embed_url_is_allowed` before HTTP; non-allowlisted hosts and non-http(s) schemes error; non-loopback `http://` requires `ASGREP_EMBED_ALLOW_INSECURE_HTTP`. HTTP agent uses `redirects(0)` so allowlisted hosts cannot 30x into disallowed hops. |
| **criticality** | high (SSRF safety) |
| **source** | doc: `docs/env-trust.md`; code: `embedder.rs` `embed_url_is_allowed` + `embed_http_agent`; tests: unit asserts on metadata IP / evil host / file://; validation ledger `docs/validation/negative-ledgers.md` |
| **how to violate** | Set `ASGREP_EMBED_API_URL=http://169.254.169.254/...` and observe successful embed HTTP; or allowlisted host 302 to link-local followed by client. |
| **enforcement evidence** | Unit tests fail closed; `from_env` returns `None` when allowlist fails; `ureq::builder().redirects(0)`. |
| **status** | **CONSISTENT** |

---

## INV-DURABILITY-FC — Unknown durability does not enable fast-unsafe

| Field | Content |
|-------|---------|
| **id** | INV-DURABILITY-FC |
| **statement** | Unrecognized `ASGREP_DURABILITY` / CLI durability strings must not silently select `FastUnsafe`. Env falls back to `Balanced` default; CLI parse errors. Known tokens: `strict`, `balanced`/`default`, `fast-unsafe`/`unsafe`. |
| **criticality** | medium (data integrity) |
| **source** | code: `Durability::parse` / `from_env` comment "must not silently downgrade"; tests: `store_pragmas.rs` parse cases; CLI `parse_durability` |
| **how to violate** | Set `ASGREP_DURABILITY=off` and observe `synchronous=OFF` write pragma without explicit unsafe token. |
| **enforcement evidence** | `parse("off") == None`; `from_env` → default Balanced. |
| **status** | **CONSISTENT** |

---

## INV-CASCADE-NO-WIDEN — Semantic cannot introduce files outside cascade survivors

| Field | Content |
|-------|---------|
| **id** | INV-CASCADE-NO-WIDEN |
| **statement** | For unprefixed hybrid search, every returned hit's file must be in the lexical prefilter survivor set; semantic ranking cannot widen the file set beyond the cascade working set. |
| **criticality** | high (retrieval correctness contract) |
| **source** | doc: `docs/cascade-query-planner.md`, `docs/semantic-search.md`; code: `search_hybrid` `embed_pass_for_files(..., &working_files)`; tests: `cascade_planner.rs` asserts all hits ⊆ lexical files |
| **how to violate** | Hybrid query returns a hit file with zero lexical prefilter membership. |
| **enforcement evidence** | `working_files` gates embed pass; tests assert containment. |
| **status** | **CONSISTENT** |

---

## INV-CASCADE-STRUCT-EMPTY — Empty structural stage behavior

| Field | Content |
|-------|---------|
| **id** | INV-CASCADE-STRUCT-EMPTY |
| **statement** | *(Docs claim)* Hybrid cascade returns **no** hits when the structural stage has no survivors. *(Code/tests claim)* When structural is empty, **lexical survivors remain** the working set and semantic may still run on them (ht1h.3). |
| **criticality** | high (public retrieval contract) |
| **source** | **Doc A:** `docs/cascade-query-planner.md` L6–13 ("If none match, the cascade stops"; "no hybrid hits when either the lexical or structural stage has no survivors"). **Code B:** `search/mod.rs` `working_files = if structural_files.is_empty() { lexical_files } else { structural_files }`. **Test B:** `cascade_stops_when_a_stage_has_no_survivors` requires non-empty hits when structural empty. |
| **how to violate** | Run hybrid on a token with lexical hits but no structural symbols; observe non-empty results (violates Doc A) or empty results (violates Code/Test B). |
| **enforcement evidence** | Code and tests enforce B; docs still state A. |
| **status** | **CONTRADICTION** |

---

## INV-AST-GREP — External ast-grep requires dual opt-in + absolute path

| Field | Content |
|-------|---------|
| **id** | INV-AST-GREP |
| **statement** | Production pattern search does not spawn `ast-grep` unless **both** `ASGREP_ALLOW_AST_GREP` is truthy **and** `ASGREP_AST_GREP` is an absolute existing file path. Relative/PATH names are ignored. |
| **criticality** | high (process injection / supply chain) |
| **source** | doc: `docs/env-trust.md`; code: `pattern.rs` dual gate |
| **how to violate** | Set only `ASGREP_AST_GREP=ast-grep` (PATH) without ALLOW and observe spawn; or relative path accepted. |
| **enforcement evidence** | Early return if ALLOW off; absolute+is_file check. |
| **status** | **CONSISTENT** |

---

## INV-EDIT-ROOT — Pi edit paths stay under projectRoot

| Field | Content |
|-------|---------|
| **id** | INV-EDIT-ROOT |
| **statement** | `planEdit` refuses paths that resolve outside `projectRoot` (and device-path cases at the boundary). Replace XOR write is enforced so illegal bags never enter planning. |
| **criticality** | high (source write isolation) |
| **source** | code: `packages/pi/extension/src/edit.ts` `planEdit` / `containedInRoot`; tests under `packages/pi/extension/test/` (edit path suite, prior pass-4) |
| **how to violate** | Edit with `../outside` or absolute foreign path and observe write. |
| **enforcement evidence** | Containment check throws before apply. |
| **status** | **CONSISTENT** |

---

## INV-LIMITS — Shared query/limit ceilings

| Field | Content |
|-------|---------|
| **id** | INV-LIMITS |
| **statement** | Search result limits clamp to `1..=MAX_OUTPUT_RESULTS` (1000); agent soft clamp `DEFAULT_AGENT_LIMIT` (100); query length rejects above `MAX_QUERY_CHARS` (4096). Code Mode schema max 500 is a surface-local tighter bound. |
| **criticality** | medium (resource / DoS) |
| **source** | code: `limits.rs` + unit tests; Searcher::new clamps limit; codemode schema maximum 500 |
| **how to violate** | Pass limit 10_000 and observe uncapped response size; query of 5000 chars accepted without error. |
| **enforcement evidence** | Unit tests for clamp and query boundary; Searcher construction clamps. |
| **status** | **CONSISTENT** |

---

## INV-RANK-FUSION — Hybrid fusion uses RRF and deterministic ties

| Field | Content |
|-------|---------|
| **id** | INV-RANK-FUSION |
| **statement** | Hybrid multi-channel scores fuse via weighted RRF (`Σ w/(60+rank+1)`); final order centralized in `finish_response`; ranking oracle fixture `must_include` constraints hold on the sample corpus. |
| **criticality** | medium (quality contract; not security) |
| **source** | doc: `docs/fusion-ranking.md`; code: `finish_response` / `cmp_ranked_hits`; test: `ranking_oracle.rs` + `tests/fixtures/ranking/cases.json` |
| **how to violate** | Change fusion formula without updating docs; oracle cases fail. |
| **enforcement evidence** | Fixture-driven oracle loads cases; fusion docs match intended formula. Absolute MRR/nDCG numbers are **out of scope** here (see benchmarks honesty policy). |
| **status** | **CONSISTENT** (local oracle); published global metrics not asserted this pass |

---

## Summary counts

| Status | Count | IDs |
|--------|------:|-----|
| CONSISTENT | 11 | INV-MCP-SANDBOX, INV-INDEX-PATH-PREC, INV-MCP-SEARCHER-INV, INV-BATCH-NO-MUT-PAR, INV-EMBED-ALLOW, INV-DURABILITY-FC, INV-CASCADE-NO-WIDEN, INV-AST-GREP, INV-EDIT-ROOT, INV-LIMITS, INV-RANK-FUSION |
| CONTRADICTION | 2 | INV-SURFACE-ROOT-PARITY, INV-CASCADE-STRUCT-EMPTY |
| GAP | 5 | INV-CM-ROOT-FREE, INV-INDEX-PATH-PRIV, INV-CM-SEARCHER-INV, INV-RO-CATALOG, INV-XOR-CM-MCP |
| UNKNOWN | 0 new (inherited U-* still open) |

Exact inventory: **18** invariant records (band 8–20).

**Folded into others (not separate IDs):** embed redirects→INV-EMBED-ALLOW; feature-flag fail-closed noted under ranking/search surfaces; `ASGREP_USE_CACHE` no-/tmp under INV-INDEX-PATH-PREC path chain.
