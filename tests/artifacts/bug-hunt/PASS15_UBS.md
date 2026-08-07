# Pass 15 — UBS verification rescan

**Date:** 2026-08-07  
**Scope:** Final automated rescan for **new** real bugs only (not style).  
**Surfaces:** `crates/ast-sgrep-core` (UBS); optional clippy on `ast-sgrep-core` + `ast-sgrep-cli`.

## Commands

```bash
ubs crates/ast-sgrep-core --only=rust --format=text 2>&1 | tee tests/artifacts/bug-hunt/ubs-core-pass15.txt | tail -40
CARGO_TARGET_DIR=target-pass15 cargo clippy -p ast-sgrep-core -p ast-sgrep-cli -- -D warnings
# without -D (inventory):
CARGO_TARGET_DIR=target-pass15 cargo clippy -p ast-sgrep-core -p ast-sgrep-cli --message-format=short
```

Artifact: [`ubs-core-pass15.txt`](ubs-core-pass15.txt)

## Summary counts vs Pass 1

| Scanner | Target | Pass1 | Pass15 | Delta |
|---------|--------|------:|-------:|------:|
| UBS Critical | core | 50 | 52 | +2 (same FP classes; more files/samples) |
| UBS Warning | core | 2657 | 3248 | inventory noise |
| UBS Info | core | 824 | 904 | inventory noise |
| Files scanned | core | 66 | 71 | +5 sources since pass1 |

## Critical triage (52 = 23+24+1+4)

| Class | Count | Product path? | Verdict |
|-------|------:|---------------|---------|
| `panic!` / `todo!` / `unimplemented!` | 23 | Only test/`cfg(test)` + bench identity oracle | **FP / intentional** |
| Secret/token `==` / `!=` | 24 | Tests only (`RefreshToken` **symbol** names, HitKind asserts) | **FP** (not crypto secrets) |
| `Command::new` untrusted-looking | 1 | `pattern.rs:417` `find_ast_grep_binary` | **FP** — dual gate `ASGREP_ALLOW_AST_GREP` + absolute `ASGREP_AST_GREP` file; version probe + kill on timeout; bench-only |
| Hardcoded secrets | 4 | `tests/metamorphic.rs` fixture tokens | **FP** |

### Product `panic!` inventory (complete)

| Location | Context |
|----------|---------|
| `src/bench_suite.rs:412` | `#[cfg(test)]` — identity oracle must exist for every suite case |
| `src/search/mod.rs:1494` | `#[test]` poison-inject for `lock_clear_on_poison` |
| `src/search/passes/embed.rs:412` | `#[test]` intentional QCACHE poison |

**No product-runtime `panic!` / `todo!` / `unimplemented!` in library search/index paths.**

### Checked clean (product-critical)

1. **`#![forbid(unsafe_code)]`** on core lib; no `unsafe` blocks in product `src/`.
2. **No request-derived SQL sinks** (UBS OK; `or_like_filter` uses bound `?` params).
3. **No shell `-c`/`-lc`**, no JWT bypass, no TLS verify-off.
4. **Pass1 real bugs already closed:** `d2a1.1` nonce, `d2a1.2` restore_synchronous, `d2a1.3` percentile_99 — all **closed**; re-scan does not re-open.
5. **Hot-path unwraps re-checked:** `sql.rs` `or_like_filter` single-column `unwrap` is after `len()==1` guard; `finish_response` expect is after invalid glob cleared for compatibility path only.

## Clippy (optional)

| Mode | Result |
|------|--------|
| `-D warnings` | **Fails** on style: `clippy::manual_pattern_char_comparison` in `ast-sgrep-embed` (dep of core) — **not a correctness bug** |
| Default clippy (no `-D`) | Style only: needless `return`/`?`, manual checked_div (intentional), index loop in ANN, etc. **No correctness errors** |

Not filed; out of scope for “real bugs only”.

## New real bugs

**None.** Residual criticals match Pass 1 inventory classes (tests, symbol-name “token” FPs, gated ast-grep spawn). Count drift (+2 critical) is extra test/fixture sampling, not new product defect classes.

## Beads

**None filed.** Do not promote Pass1 FPs to beads.

## Verdict

**PASS15 CLEAN** for product-critical paths under UBS rescan.

**PRODUCT ZERO-CHANGE** this pass (docs/artifact only).

No commit (per mission).
