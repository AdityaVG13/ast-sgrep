# Pass 11 — Dual-evidence re-verification (high findings)

| Field | Value |
|-------|-------|
| Loop | 11 / independent-verification |
| Observer | **skeptic non-originator** (does not trust pass 10 narrative alone) |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze retained | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (books-era) |
| HEAD at pass 11 | `7cb1a28d53d5a5752ea62010b970e0b491d2dc75` (dirty tree; product loci re-read) |
| Evidence engines | native `rg`/`sed`/`cargo test` (zerostack tokenzero **unavailable** — B-ZS-ENGINES) |
| Open beads at start | **51** → markdown work-queue only (no `br create`; flood gate) |

Content fingerprints (file SHA256 prefix, pass 11):

| File | sha256[:16] |
|------|-------------|
| `crates/ast-sgrep-mcp/src/lib.rs` | `249b1bf84739c89e` |
| `crates/ast-sgrep-codemode/src/session.rs` | `51d9fea3123a271b` |
| `crates/ast-sgrep-core/src/index.rs` | `f44d7d7a3bfb60e3` |
| `crates/ast-sgrep-cli/src/watch.rs` | `ece9831cac7d099f` |

---

## Method (loop-27 style)

For each TOP high finding:

1. **Source re-read** both sides of the claim with line anchors (not paraphrase of pass 10).
2. **Second channel** — existing unit/integration test that pins related behavior, or explicit **negative** (test proves Ok path only / no Err pin / no xproc pin).
3. **Verdict** — CONFIRMED / WEAKENED / REFUTED / DESIGN-INTENT with dual-evidence status.
4. **Promotion** — product bead only if high + dual evidence + clear fix; else design ASK or ops packet.

No invented CVEs. No benchmark numbers.

---

## H1 — C2 / BY-CM-ROOT (CM free root vs MCP sandbox)

### Claim under test

Code Mode `root` is unsandboxed; MCP jails tool roots under configured workspace. Shared session/`ASGREP_INDEX_PATH` can index a foreign tree into the pinned DB and prune workspace-relative paths.

### Evidence A — source (both sides)

**MCP jail (fail-closed):**

```547:573:crates/ast-sgrep-mcp/src/lib.rs
    fn sandbox_root(&self, candidate: PathBuf) -> anyhow::Result<PathBuf> {
        let canonical = if candidate.exists() {
            candidate
                .canonicalize()
                .with_context(|| format!("canonicalize root {}", candidate.display()))?
        } else {
            anyhow::bail!(
                "project root does not exist or is not a directory: {}",
                candidate.display()
            );
        };
        anyhow::ensure!(
            canonical.starts_with(&self.root),
            "root {} escapes configured workspace {}",
            canonical.display(),
            self.root.display()
        );
        // ...
        Ok(canonical)
    }
```

`resolve_root` always routes through `sandbox_root` (`lib.rs` ~454–460).

**CM free root (no under-workspace check):**

```105:111:crates/ast-sgrep-codemode/src/session.rs
    fn root_arg(&self, args: &Value) -> PathBuf {
        args.get("root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.config.root.clone())
    }
```

**CM index binds free root + session index_path:**

```248:266:crates/ast-sgrep-codemode/src/session.rs
    pub(crate) fn index_repo(&mut self, args: &Value) -> anyhow::Result<Value> {
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args),
            index_path: self.config.index_path.clone(),
            embed_backend: EmbedBackend::Auto,
            ..IndexOptions::default()
        })?;
        // index_all / reindex_all then invalidate_searcher_cache
```

Both surfaces inherit `ASGREP_INDEX_PATH` into config (`session.rs` ~29; `mcp/src/lib.rs` ~202).

**Indexer writes relative to walk root into opened DB; meta root overwritten; prune on unseen paths:**

- `Indexer::new` → `store.set_meta("root", …)` (`index.rs` ~218)
- `collect_index_candidates` walks `options.root`, keys via `strip_prefix(root)` (~405–449)
- `prune_missing_files` removes store paths not in `seen_paths` (~454–479)

### Evidence B — tests

| Test | Result | What it pins |
|------|--------|--------------|
| `tool_roots_are_sandboxed_under_configured_workspace` (`crates/ast-sgrep-mcp/tests/protocol.rs`) | **PASS** (ran pass 11) | MCP refuses outside root with `"escapes configured workspace"` |
| CM foreign-root + shared `index_path` prune | **ABSENT** | No codemode test asserts jail or foreign-root corpus rewrite |

Command:

```text
cargo test -p ast-sgrep-mcp --test protocol tool_roots_are_sandboxed -- --nocapture
# ok; 1 passed
```

### Verdict

| Item | Status |
|------|--------|
| Asymmetry MCP jail vs CM free | **CONFIRMED** (source both sides + MCP test) |
| Foreign root + pinned DB prune narrative | **CONFIRMED by composition** (free root + shared index_path + strip_prefix + prune) — no live multi-root fixture executed this pass |
| Product intent | **DESIGN ASK** — jail may break intentional multi-root hosts; host duty may remain |
| Dual-evidence status | **DUAL-OK** for asymmetry; **PARTIAL** for live prune repro (source composition only) |
| Promote fix bead? | **No auto-fix** — packet is design/docs/test gate |

Residual ID: **R-CM-ROOT-POLICY** (packet 01).

---

## H2 — CL-MID-SIDECAR-CACHE (commit then sidecar Err; MCP invalidate only on Ok)

### Claim under test

After bulk SQLite commit, `rebuild_dirty_sidecars` failure returns `Err` from `index_all`. MCP `tool_index_repo` uses `index_all()?` and only then invalidates Searcher / clears registries. Agent sees tool error; disk advanced; warm Searcher may serve pre-mutation hits.

### Evidence A — source (core + MCP)

**Commit then sidecar (order is structural):**

```271:284:crates/ast-sgrep-core/src/index.rs
            self.store.begin_bulk_tx()?;
            let write_result =
                self.commit_prepared_files(&candidates, prepared, &mut stats, &mut semantic_ivf_dirty);
            self.store.apply_bulk_write_result(write_result)?;
        }
        self.rebuild_dirty_sidecars(&stats, semantic_ivf_dirty)?;
        self.post_index_hooks()?;
        Ok(stats)
```

`apply_bulk_write_result` on `Ok(())` **commits** (`sqlite.rs` ~540–548):

```540:548:crates/ast-sgrep-core/src/store/sqlite.rs
    pub fn apply_bulk_write_result(&self, write_result: Result<()>) -> Result<()> {
        match write_result {
            Ok(()) => self.commit_bulk_tx(),
            Err(e) => match self.rollback_bulk_tx() {
                Ok(()) => Err(e),
                Err(rb) => Err(rb),
            },
        }
    }
```

Sidecar rebuild is fallible (`rebuild_dirty_sidecars` → tantivy/IVF `?` paths, `index.rs` ~481–518).

**MCP invalidate only after successful `?`:**

```882:897:crates/ast-sgrep-mcp/src/lib.rs
        let stats = if force {
            indexer.reindex_all()?
        } else {
            indexer.index_all()?
        };
        // Index mutated on disk — always drop cached Searcher / path ids / elisions
        // before any post-work deadline check. ...
        self.invalidate_searcher_cache();
        Self::lock_or_recover(&self.path_registry, |registry| registry.clear()).clear();
        Self::lock_or_recover(&self.emitted_snippets, |seen| seen.clear()).clear();
```

Comment claims "always drop" after mutation, but **Rust `?` skips the drop on any `index_all` Err**, including post-commit sidecar failure. Same pattern on CM Ok-only invalidate (`session.rs` ~261 after successful index).

### Evidence B — tests

| Test | Result | What it pins |
|------|--------|--------------|
| `cache_tests::index_repo_invalidates_searcher_after_disk_mutation` | **PASS** | Ok-path: generation advances, cache empty, registries cleared |
| `cache_tests::reindex_generation_rejects_in_flight_stale_searcher` | **PASS** | generation fence vs restore |
| Mid-sidecar / `index_all` Err → still invalidate | **ABSENT** | No unit injects sidecar failure |

```text
cargo test -p ast-sgrep-mcp --lib -- --nocapture
# ok; 3 passed (write_resp + 2 cache)
```

Ok-path test is dual-evidence that invalidate is **post-success**, not `finally`/scope-guard. Combined with commit-before-sidecar source order → **GAP CONFIRMED**.

Note: `cargo test -p ast-sgrep-core --lib apply_bulk_write_result` **failed to compile** this HEAD (unrelated missing `SearchHit.resolution` / `SearchResponse` fields in test helpers). Not used as negative; bulk commit semantics still read from source.

### Verdict

| Item | Status |
|------|--------|
| Post-commit sidecar can Err after durable write | **CONFIRMED** (source order) |
| MCP/CM skip invalidate on that Err | **CONFIRMED** (`?` control flow) |
| Ok-path invalidate works | **CONFIRMED** (unit tests) |
| Dual-evidence status | **DUAL-OK** (source + Ok-path pin; Err-path untested) |
| Promote fix bead? | **Yes as product work packet** (clear fix: invalidate + clear registries on any index attempt that may have mutated disk, or on any Err after begin; ideally always after Indexer::new opened write path) |

Residual ID: **R-INDEX-ERR-CACHE-SYNC** (packet 02). Bundles BY-REGISTRY-STALE / CL-INDEX-FAIL-REGISTRIES.

---

## H3 — GAP-WATCH-XPROC (watch mutates; no IPC to MCP cache)

### Claim under test

`asgrep watch` mutates the index in another process. MCP holds a warm Searcher keyed by process-local generation. No flock/lease/notify invalidates MCP/CM caches across processes.

### Evidence A — source (both sides)

**Watch mutates index (stderr-only progress):**

```9:72:crates/ast-sgrep-cli/src/watch.rs
pub(crate) fn run_watch(root: &Path, cli: &Cli, debounce_ms: u64) -> anyhow::Result<()> {
    // ... notify watcher ...
    let mut indexer = Indexer::new(opts)?;
    let initial = indexer.index_all()?;
    // debounce loop:
    //   full → indexer.index_all()
    //   paths → indexer.update_paths(&paths)
    //   deferred → indexer.flush_deferred_rebuilds()
    // all progress via eprintln! only
```

No reference to MCP, generation broadcast, or filesystem lease for peer processes (file is self-contained ~80 LOC).

**MCP single-flight is in-process only:**

```182:182:crates/ast-sgrep-mcp/src/lib.rs
    index_lock: Mutex<()>,
```

```861:866:crates/ast-sgrep-mcp/src/lib.rs
    fn tool_index_repo(&self, args: IndexRepoArgs) -> anyhow::Result<String> {
        // ...
        let _flight = Self::lock_or_recover(&self.index_lock, |_| {});
```

Searcher generation is process-local (`SearcherCache.generation`). No watch of `index.db` mtime / WAL generation for external writers.

### Evidence B — tests

| Test | Result | What it pins |
|------|--------|--------------|
| In-process invalidate after `tool_index_repo` Ok | **PASS** | same process only |
| Two-process watch vs MCP stale Searcher | **ABSENT** | no xproc harness |

Second channel = **independent surface pair** (CLI watch writer + MCP cache reader) both lack xproc invalidate, plus Ok-path unit proves invalidate only runs on local tool path.

### Verdict

| Item | Status |
|------|--------|
| Watch writes index without notifying peers | **CONFIRMED** |
| MCP cache not xproc-aware | **CONFIRMED** |
| Dual-evidence status | **DUAL-OK** (two source surfaces + absence of xproc test / IPC) |
| Promote fix bead? | **Design ASK** — lease vs doc single-writer vs FS notify; not a one-line fix |

Residual ID: **R-XPROC-MULTIWRITER** (packet 03). Bundles GAP-XOR-RUNTIME / GAP-RO-HOST host co-location.

---

## Secondary reaffirmations (cheap, not full dual campaign)

| ID | Outcome | Evidence |
|----|---------|----------|
| Embed SSRF allowlist + redirects(0) | **CONSISTENT** retained | pass 10; not re-run this pass (explicit non-goal invent CVE) |
| FastUnsafe named opt-in | **BY-DESIGN** + ops GAP | not re-expanded; packet 04 |
| MCP Ok-path cache invalidate | **CONSISTENT** | dual tests green |

---

## Dual-evidence scoreboard (TOP 3)

| ID | Sev | Independent verdict | Dual status | Product action |
|----|-----|---------------------|-------------|----------------|
| C2 / BY-CM-ROOT | high | CONTRADICTION retained | DUAL-OK asymmetry; PARTIAL live prune | Design ASK + host contract |
| CL-MID-SIDECAR-CACHE | high | GAP retained | DUAL-OK | **Fix packet** |
| GAP-WATCH-XPROC | high | GAP retained | DUAL-OK | Design ASK |

No finding REFUTED. No severity inflation. No new high beyond campaign ledger.
