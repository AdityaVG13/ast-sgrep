# Pass 1 — Surface scan (UBS / clippy)

Branch: `perf/software-optimization`  
Date: 2026-08-07  
Scope: product crates under `crates/ast-sgrep-{core,cli,mcp,lsp,embed,...}/`

## Scanner commands

```bash
# UBS (one project root at a time; multi-path argv not supported)
ubs crates/ast-sgrep-core --only=rust --format=text -v
ubs crates/ast-sgrep-cli  --only=rust --format=text -v
ubs crates/ast-sgrep-mcp  --only=rust --format=text
ubs crates/ast-sgrep-lsp  --only=rust --format=text

# Clippy (default + panic/unwrap/indexing inventory)
cargo clippy -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp --lib
cargo clippy -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp --all-targets -- \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic

# Build sanity
cargo check -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp
```

Artifacts: `tests/artifacts/bug-hunt/ubs-{core,cli,mcp}.*`

## Summary counts

| Scanner | Target | Critical | Warning | Info | Notes |
|---------|--------|---------:|--------:|-----:|-------|
| UBS | `ast-sgrep-core` | 50 | 2657 | 824 | Mostly test panic/unwrap + FP “secrets” |
| UBS | `ast-sgrep-cli` | 15 | 429 | 257 | Supervisors RNG/Command noise |
| UBS | `ast-sgrep-mcp` | ~1 shape | (see txt) | — | Constant-time FP on non-crypto compare |
| UBS | `ast-sgrep-lsp` | 0 panics | — | — | No panic! macros; no unsafe |
| clippy (default lib) | core | 0 errors | ~10 | — | needless_question_mark, style only |
| clippy (unwrap/expect/indexing) | core lib | 0 errors | ~117 | — | Inventory; not auto-bugs |
| cargo check | core+cli+mcp | clean | — | — | Finished ok |

**Triage result:** ~3 real issues filed; the rest of UBS “critical” mass is FP or test-only.

## Triage table (representative)

| Finding | Location | Verdict | Action |
|---------|----------|---------|--------|
| `read_exact` result ignored on `/dev/urandom` → all-zero nonce still hex-valid | `cli/supervisor.rs` `generate_worker_nonce` | **REAL** (auth shape bypass) | **Fixed** + bead `ast-sgrep-d2a1.1` closed |
| `let _ = restore_synchronous()` after COMMIT/ROLLBACK | `core/store/sqlite.rs` `end_file_tx` / bulk path | **REAL** (durability fail-open under FastUnsafe) | Bead `ast-sgrep-d2a1.2` open P2 |
| `percentile_99` indexes empty vec | `core/bench_suite.rs` | **Latent** (call site guards 100..=1000) | Bead `ast-sgrep-d2a1.3` open P3 |
| UBS: panic! in tests / `bench_suite` / poison inject test | `search/mod.rs` cfg(test), tests/* | FP / intentional | No bead |
| UBS: “secret == compare” on `RefreshToken` symbols | e2e_smoke etc. | FP | No bead |
| UBS: hardcoded “token” strings in metamorphic tests | tests/metamorphic.rs | FP | No bead |
| UBS: Command::new(env AST_GREP path) | `pattern.rs` find_ast_grep | FP (gated allow + absolute path) | No bead |
| UBS: DefaultHasher fallback for worker nonce | supervisor | Style/hardening (secondary to zero-nonce) | Covered by nonce fix |
| clippy: `expect` local semantic embedder | embedder.rs | FP (HashedEmbedder always available) | No bead |
| clippy: indexing_slicing fusion/channel tables | fusion.rs etc. | Style (enum-indexed const tables) | No bead |
| clippy: Mutex unwrap in cfg(test) poison test | search/mod.rs | FP | No bead |
| SQL “concatenation” LIKE template builders | sql.rs or_like_filter | FP (bound params `?`) | No bead |
| machine.rs `expect` failure envelope serialize | cli/machine.rs | FP (static JSON shape) | No bead |
| embed `expect` / Hashed path | embedder | FP under current impl | No bead |

## Beads created

| ID | Title | Pri | Status |
|----|-------|-----|--------|
| `ast-sgrep-d2a1` | bug-hunt multi-pass PR27 (epic) | P2 | open |
| `ast-sgrep-d2a1.1` | worker nonce ignores urandom read_exact | P2 | **closed** (fixed this pass) |
| `ast-sgrep-d2a1.2` | restore_synchronous errors swallowed | P2 | open |
| `ast-sgrep-d2a1.3` | percentile_99 empty panic | P3 | open |

Labels: `bug-hunt`, `pass1-surface`

## Optional fix applied

- `crates/ast-sgrep-cli/src/supervisor.rs`: require successful `read_exact`; all-zero buffer forces fallback entropy mix; unit test `worker_nonce_is_32_hex_and_not_all_zero`.
- Evidence: `cargo test -p ast-sgrep-cli --lib worker_nonce -- --nocapture` → ok.

## Checked clean (≥3)

1. **No `unsafe` blocks** in core UBS rust ownership scan.
2. **No product `todo!` / `unimplemented!`** in product `src/` (only test panics).
3. **Default clippy correctness** on core/cli/mcp lib: no correctness errors (style-only).
4. **`cargo check -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp`**: finished ok.
5. **Request-derived SQL sinks**: UBS OK (parameterized queries).
6. **MCP node-id path escape**: relative components + canonicalize + starts_with root (spot-checked).

## Notes for later passes

- UBS `--format=jsonl` emits **totals only** unless verbose text is used; prefer `--format=text -v` for triage.
- UBS accepts a single project dir, not multiple crate paths in one argv.
- Pass 2 should focus logic/concurrency (response cache races, file_tx nesting, IVF load paths) rather than re-litigating unwrap inventories.
