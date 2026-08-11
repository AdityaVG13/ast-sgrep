# Pass 12 — Residual stay-proof (named checks)

| Field | Value |
|-------|-------|
| Loop | 12 / absolute-convergence-seal |
| Mode | residual-only re-rotate (observer: skeptic) |
| Prior | `../pass11-independent/residual-scorecard.md` + dual-evidence |
| Product source edits this pass | **ZERO** |
| Engines | native `rg`/`sed`/`shasum`/`cargo test` (zerostack fszero-codemode unavailable this host — B-ZS-ENGINES) |

## Method

For every open residual packet / TOP high from pass 11: re-read source anchors; confirm still present with same semantics; do **not** invent new axes or expand ledger. Second channel = content fingerprint parity with pass 11 + re-run of pass 11 cheap tests.

## Named checks (≥5)

| # | Check | Expected | Observed | Outcome |
|---|-------|----------|----------|---------|
| **N1** | Content fingerprints of residual loci match pass 11 | sha256[:16] of `mcp/lib.rs`, `cm/session.rs`, `core/index.rs`, `cli/watch.rs` equal pass 11 table | `249b1bf84739c89e` · `51d9fea3123a271b` · `f44d7d7a3bfb60e3` · `ece9831cac7d099f` — **identical** | **PASS** (no product drift at anchors) |
| **N2** | H1 / R-CM-ROOT-POLICY — MCP jail still fail-closed | `sandbox_root` `starts_with(&self.root)` + escapes message | L547–573 still present; `resolve_root` L453 routes tools | **STILL VALID** |
| **N3** | H1 / R-CM-ROOT-POLICY — CM free root still unsandboxed | `root_arg` maps string → `PathBuf` with no under-workspace check; `index_repo` binds free root + session `index_path` | L105–111 free; L248–266 `Indexer::new` + invalidate only after Ok `?` | **STILL VALID** (asymmetry CONFIRMED) |
| **N4** | H2 / R-INDEX-ERR-CACHE-SYNC — commit-before-sidecar order | `apply_bulk_write_result` then `rebuild_dirty_sidecars?` | `index.rs` L278–282 unchanged; `sqlite.rs` `apply_bulk_write_result` L540–548 commits on Ok | **STILL VALID** |
| **N5** | H2 / R-INDEX-ERR-CACHE-SYNC — MCP invalidate Ok-only (`?`) | `invalidate_searcher_cache` + registry clear **after** `index_all()?`; no scopeguard/finally | L882–897 comment still claims "always drop"; control flow still skips on Err; `rg` no scopeguard invalidate | **STILL VALID** (comment/code mismatch retained) |
| **N6** | H3 / R-XPROC-MULTIWRITER — watch mutates without peer notify | stderr-only progress; no flock/lease/IPC/generation broadcast | `watch.rs` L9–80 self-contained; no xproc symbols; MCP `index_lock: Mutex<()>` L182 process-local | **STILL VALID** |
| **N7** | Ok-path dual tests still green (pass 11 cheap suite) | MCP lib 3 + sandbox protocol 1 | `cargo test -p ast-sgrep-mcp --lib` → **3 passed**; `tool_roots_are_sandboxed` → **1 passed** (pass 12 re-run) | **PASS** (pins Ok-path only; Err/xproc ABSENT retained) |
| **N8** | R-OPS-DOCS-FOOTGUNS still optional hygiene | No product doctor/docs landing required for seal | Packet 04 remains OPTIONAL; not dual-evidence high | **STILL VALID** (open, non-blocking) |
| **N9** | No NEW material R-* invented this pass | Prefer zero new packets; any new high needs dual-evidence | Residual scan only; zero new GAP/CONTRADICTION IDs; slot 5 unused | **PASS** (no new R-*) |
| **N10** | Product tree for residual crates not edited by this pass | ZERO edits under `crates/` for seal work | Seal writes only under `tests/artifacts/.../pass12-convergence/` + `.rotational-code-analysis/` books | **PASS** |

## Residual disposition matrix (pass 11 → pass 12)

| Residual ID | Pass 11 | Pass 12 re-rotate | Anchors | Product claim |
|-------------|---------|-------------------|---------|---------------|
| **R-CM-ROOT-POLICY** | high DESIGN ASK · dual-OK asymmetry | **STILL VALID** (N2+N3+N1) | MCP `sandbox_root` vs CM `root_arg` | Not fixed; host/policy open |
| **R-INDEX-ERR-CACHE-SYNC** | high FIX CANDIDATE · dual-OK | **STILL VALID** (N4+N5+N7) | commit→sidecar; Ok-only invalidate | Not fixed; fix still recommended |
| **R-XPROC-MULTIWRITER** | high DESIGN ASK · dual-OK | **STILL VALID** (N6+N7) | watch stderr-only + in-proc lock | Not fixed; multi-writer unsafe |
| **R-OPS-DOCS-FOOTGUNS** | med/low OPTIONAL | **STILL VALID** (N8) | docs/doctor bundle | Open hygiene |

**None REFUTED.** No severity change. No dual-evidence withdrawal.

## Explicit non-claims

- Stay-proof does **not** mean product is multi-writer safe, cache-consistent on index Err, or CM-root-jailed.
- Stay-proof **does** mean the audit residual ledger is still accurate on current HEAD content at named anchors.
- HEAD `b2af241959461f4f71d37ee92e4a94779f59d8d7` differs from pass 11 HEAD `7cb1a28…` but residual-locus **file digests match** pass 11 (N1).

## Commands (evidence)

```text
shasum -a 256 crates/ast-sgrep-mcp/src/lib.rs \
  crates/ast-sgrep-codemode/src/session.rs \
  crates/ast-sgrep-core/src/index.rs \
  crates/ast-sgrep-cli/src/watch.rs
# prefixes: 249b1bf84739c89e 51d9fea3123a271b f44d7d7a3bfb60e3 ece9831cac7d099f

rg -n "fn sandbox_root|fn root_arg|apply_bulk_write_result|rebuild_dirty_sidecars|invalidate_searcher_cache|index_lock" \
  crates/ast-sgrep-mcp/src/lib.rs crates/ast-sgrep-codemode/src/session.rs \
  crates/ast-sgrep-core/src/index.rs crates/ast-sgrep-cli/src/watch.rs

cargo test -p ast-sgrep-mcp --lib
# ok; 3 passed

cargo test -p ast-sgrep-mcp --test protocol tool_roots_are_sandboxed
# ok; 1 passed
```
