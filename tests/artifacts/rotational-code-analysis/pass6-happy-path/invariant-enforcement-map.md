# Pass 6 — Invariant enforcement map (happy-path lenses)

Maps pass-5 INV-* onto success-path traces. Status from pass 5 retained; "on-path" says whether the happy path **exercises** the check (not whether fail paths are covered -- pass 7).

Legend: **E** = enforced on this happy path · **U** = unenforced / advisory · **N** = not on path · **C** = contradiction exercised by success shape · **G** = gap visible on success

| INV | HP-CLI-SEARCH | HP-MCP-SEARCH | HP-CM-CALL | HP-PI-ASGREP | HP-INDEX | HP-CASCADE |
|-----|:-------------:|:-------------:|:----------:|:------------:|:--------:|:----------:|
| INV-MCP-SANDBOX | N | **E** | N | N | E (MCP branch) | N |
| INV-CM-ROOT-FREE | N | N | **G** (success w/ free root) | **G** | G (CM branch) | N |
| INV-SURFACE-ROOT-PARITY | -- | **C** vs CM | **C** vs MCP | **C** | **C** | N |
| INV-INDEX-PATH-PREC | **E** | **E** | **E** | **E** | **E** | N (via open) |
| INV-INDEX-PATH-PRIV | G | G | G | G | **G** | N |
| INV-MCP-SEARCHER-INV | N | E (warm/restore) | N | N | **E** MCP | N |
| INV-CM-SEARCHER-INV | N | N | E soft | E soft | **G** test | N |
| INV-BATCH-NO-MUT-PAR | N | N | E if batch | E if batch | E if batch | N |
| INV-RO-CATALOG | N | N (MCP tools separate) | **G** | **G** | **G** CM/Pi | N |
| INV-XOR-CM-MCP | N | U docs | U docs | **G** host | N | N |
| INV-EMBED-ALLOW | E if embed | E if embed | E if embed | E if embed | E if embed | E if stage D |
| INV-DURABILITY-FC | N (read open) | N | N | N | **E** write open | N |
| INV-CASCADE-NO-WIDEN | via cascade | N | via cascade | via cascade | N | **E** |
| INV-CASCADE-STRUCT-EMPTY | **C** shape | N | **C** | **C** | N | **C** (code B) |
| INV-AST-GREP | N default | N (native pattern) | N | N | N | N (pattern mode separate) |
| INV-EDIT-ROOT | N | N | N | N search; E edit | N | N |
| INV-LIMITS | **E** | **E** | **E** (500) | **E** | N | via finish |
| INV-RANK-FUSION | via cascade | N | via cascade | via cascade | N | **E** |

## Challenge outcomes (pass 5 residuals)

### C1 cascade docs vs code (INV-CASCADE-STRUCT-EMPTY)

**Happy-path evidence:** `search_hybrid` sets `working_files = lexical_files` when `structural_files.is_empty()` (search/mod.rs ~511–515), then may still run `embed_pass_for_files` on that set. CLI/CM/Pi hybrid success with lexical-only survivors is the **code B** contract. Docs claiming full stop remain contradicted; not re-litigated as new product bug this pass.

### C2 MCP/CM root parity (INV-SURFACE-ROOT-PARITY / INV-CM-ROOT-FREE)

**Happy-path evidence:**
- MCP: `sandbox_root` before every tool including search/index -- outside workspace cannot succeed.
- CM: `root_arg` → `PathBuf::from` → `Searcher::new` / `Indexer::new` with no `starts_with` -- foreign absolute root can succeed under OS permissions.

Same core indexer/searcher; isolation is **surface policy**, not core.

### MCP searcher invalidation (INV-MCP-SEARCHER-INV)

**Happy-path sequence:** warm `searcher_for` → later `tool_index_repo` → `invalidate_searcher_cache` (generation++) + clear registries → next search rebuilds. Unit tests pin empty entry + generation advance.

### CM searcher invalidation (INV-CM-SEARCHER-INV)

**Happy-path sequence:** `index_repo` → `invalidate_searcher_cache` clears `Mutex<Option<_>>`. No generation: concurrent in-flight hold of old Searcher is a weaker race model than MCP (residual for pass 7/9).

### Pi index_repo without approval (INV-RO-CATALOG)

**Happy-path:** `asgrep_index` and Code Mode `asgrep.indexRepo` call sticky `index_repo` with no in-process approval. Catalog `read_only: false` is metadata only.

## Divergent implementations (summary)

1. **Hybrid vs channel-split:** CLI/CM/Pi hybrid; MCP three tools, no fusion.
2. **Root policy:** MCP jail vs CM free vs CLI OS-user.
3. **Searcher cache:** MCP generation+restore vs CM Option vs CLI cold open.
4. **Index control plane:** MCP deadline+lock+registry clear vs CM simple clear vs CLI no cache.
5. **Limit ceilings:** CLI/core 1000 · CM 500 · MCP agent clamp · Pi connector soft 100 on typed methods.
6. **Output envelopes:** human/plugins · Compact · AgentCapsule · Pi details wrapper.
7. **Pi dual backend:** NAPI Session vs CLI sticky serve/batch.
