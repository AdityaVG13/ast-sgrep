# PASS 6 — Sanitizers + Perf Floors + CI Pipeline

**Mission:** Sanitizer campaigns, performance floors (exec/s, size guards, init outside body), crash triage → regression, and CI/nightly continuous fuzzing design.  
**Scope:** Artifact only under `tests/artifacts/fuzz-audit/`. No production edits, no workflow edits, no beads, no commits.  
**Skill refs:** `SANITIZERS.md`, `CI-FUZZING.md`, `PERFORMANCE-TUNING.md`, `TRIAGE.md`.  
**Prior:** PASS1–PASS5 in this directory. Measured exec/s only from PASS2; unmeasured labeled **[E]**.

---

## 1. Current-state evidence (inventory)

### 1.1 Workflows under `.github/workflows/`

| File | Trigger | Fuzz? | Notes |
|------|---------|-------|-------|
| `ci.yml` | **`workflow_dispatch` only** | Yes (`bounded-fuzz` job) | Entire workflow manual; no `pull_request` / `push` / `schedule` |
| `pi-npm-release.yml` | `workflow_dispatch` | Indirect | Installs nightly + `cargo-fuzz`; release gate runs `scripts/local-release-gate.sh` (rank only) |
| `bakeoff.yml` | `workflow_dispatch` | No | Benchmark bake-off |
| `speed.yml` | `workflow_dispatch` | No | Latency thresholds |
| `install-smoke.yml` | `workflow_dispatch` | No | Post-publish install |
| `pi-cross-smoke.yml` | `workflow_dispatch` | No | macOS grouped build |
| `pi-native-artifacts.yml` | `workflow_dispatch` | No | Native packaging |

**No** `schedule:` / nightly continuous-fuzz workflow exists. **No** PR-triggered fuzz (or any CI) exists in-tree today.

Evidence — `ci.yml` header and fuzz job:

```yaml
# .github/workflows/ci.yml:1–6
name: CI
# Manual-only to avoid burning GitHub Actions minutes on every push/PR.
on:
  workflow_dispatch:
```

```yaml
# .github/workflows/ci.yml:140–170
bounded-fuzz:
  if: github.event_name == 'workflow_dispatch'
  name: Bounded parser fuzzing
  runs-on: ubuntu-latest
  timeout-minutes: 10
  steps:
    # … nightly, rust-cache (fuzz -> target), cargo-fuzz install …
    - name: Fuzz legacy query parser
      working-directory: fuzz
      run: cargo +nightly fuzz run parsed_query -- -max_total_time=30 -timeout=5

    - name: Fuzz ranking invariants
      working-directory: fuzz
      run: cargo +nightly fuzz run rank -- -max_total_time=30 -timeout=5
```

### 1.2 Local release gate

```bash
# scripts/local-release-gate.sh:10–17
# requires cargo-fuzz; then:
cd fuzz
cargo +nightly fuzz run rank -- -max_total_time=30 -timeout=5
# query_grammar is never run
```

### 1.3 Docs: CONTRIBUTING / SECURITY

| Doc | Claim | Reality |
|-----|-------|---------|
| `CONTRIBUTING.md:40–45` | Release gate = fmt/clippy/tests + **30s rank fuzz**; full build/test/clippy/audit/fuzz = `workflow_dispatch` | Accurate for local gate; CI matrix is also dispatch-only |
| `CONTRIBUTING.md:45` | "GitHub Actions runs `forbid-soundness` and `cargo check` on every `pull_request`" | **Stale** — `ci.yml` has no `pull_request` trigger; those jobs only run when someone dispatches CI |
| `CONTRIBUTING.md:54` | Do not commit `fuzz/target/` | Correct (`.gitignore`) |
| `SECURITY.md:29–33` | `fuzz/` excluded from workspace; CI still exercises parsers via bounded jobs | Intent correct; **query job is broken** (name mismatch) |
| `Cargo.toml:17–19` | `exclude = ["fuzz"]`; "Still covered by bounded-fuzz CI" | Same broken coverage for query parser |

### 1.4 Fuzz package ground truth

| Item | Value |
|------|-------|
| Package | `fuzz/Cargo.toml` → `ast-sgrep-fuzz`, `cargo-fuzz = true` |
| Bins | **`query_grammar`**, **`rank`** only (`[[bin]]` names) |
| Harnesses | `fuzz/fuzz_targets/query_grammar.rs`, `rank.rs` |
| Seeds committed | **None** (`fuzz/corpus/` gitignored; local smoke corpora exist untracked) |
| Dicts | **None** in tree |
| Regression fixtures | **None** (`tests/**/fuzz_regressions/` absent; no `include_bytes!` of crash inputs) |
| Sanitizer flags in CI/scripts | **None** explicit (cargo-fuzz default ASan only) |

### 1.5 Measured throughput (PASS2 only — do not invent)

| Target | Measurement (PASS2) | Floor (skill) | Status |
|--------|---------------------|---------------|--------|
| `query_grammar` | ~179k runs / 6s → **~30k exec/s** | ≥1k (parser) | **PASS** |
| `rank` | ~218k runs / 4s → **~54k–70k exec/s** | ≥1k (trivial/parser-class) | **PASS** |
| Future `ParserRegistry::parse` | **[E]** ~100–2k exec/s depending on lang/size (native C + tree build) | ≥500 parser / ≥100 stateful | Must remeasure after harness |
| Future IVF / wire / mmap | **[E]** 10–500 exec/s | stateful 50–100 | Init + size budgets critical |

### 1.6 Gitignore / artifacts policy

- `.gitignore`: `fuzz/target/`, `fuzz/Cargo.lock`, `fuzz/corpus/`
- `fuzz/artifacts/` not explicitly listed but local empty dirs from smokes exist untracked
- No upload-artifact step for crashes in CI today

---

## 2. P0 CI bin-name bug (`parsed_query` vs `query_grammar`)

| Field | Detail |
|-------|--------|
| **Severity** | **P0** — CI query-parser step cannot succeed |
| **CI site** | `.github/workflows/ci.yml` **lines 164–166** |
| **CI command** | `cargo +nightly fuzz run parsed_query -- -max_total_time=30 -timeout=5` |
| **Declared bin** | `fuzz/Cargo.toml` **lines 14–16**: `name = "query_grammar"`, path `fuzz_targets/query_grammar.rs` |
| **PASS2 repro** | `cargo +nightly fuzz run parsed_query` → `error: no bin target named parsed_query` (available: `query_grammar`, `rank`) |
| **Step name lies** | Job step titled "Fuzz **legacy** query parser" while bin is `query_grammar` — rename leftover from earlier target name |
| **Blast radius** | Any `workflow_dispatch` of `bounded-fuzz` fails on first fuzz step; **`rank` step never reached** if the job fails fast on query |
| **Release path** | `local-release-gate.sh` only runs `rank` — so releases can green while query harness is never exercised in CI or gate |
| **Fix (not applied this pass)** | Change line 166 to `query_grammar` **or** rename bin to `parsed_query` and update all docs; prefer rename CI → `query_grammar` (matches `fuzz list` + PASS1–5). Add `cargo +nightly fuzz list` assert before run |

```diff
# Proposed (do not apply in PASS6)
- run: cargo +nightly fuzz run parsed_query -- -max_total_time=30 -timeout=5
+ run: cargo +nightly fuzz run query_grammar -- -max_total_time=30 -timeout=5 -max_len=8192
```

---

## 3. Sanitizer campaign plan

### 3.1 Matrix for this repo

| Campaign | Flags / mechanism | Targets (now) | Targets (when harnessed) | When | Shared corpus |
|----------|-------------------|---------------|--------------------------|------|---------------|
| **ASan + UBSan (default)** | cargo-fuzz default ASan + `RUSTFLAGS="-Zsanitizer=address,undefined"` (or cargo-fuzz multi-sanitizer once pinned) | `query_grammar`, `rank` | All pure + native | **Always** for discovery runs | `fuzz/corpus/<target>/` + L1 seeds |
| **MSan** | `RUSTFLAGS="-Zsanitizer=memory"`; full instrumented deps rebuild | *not justified for current pure-Rust bins* | **`ParserRegistry` / tree-sitter**, optional **`ast-sgrep-mmap`** | Only after native harness exists | Separate build dir; **cmin-merge** into shared corpus |
| **TSan** | `RUSTFLAGS="-Zsanitizer=thread"` | N/A for current single-threaded harnesses | **kmeans / rayon pattern batch / regex_pass thread pool / CodeMode parallel** | After concurrency harness | Shared L2 corpus; short runs (overhead 5–15×) |
| **none (speed)** | `cargo fuzz run … --sanitizer=none` | Local microbench only | Never ship as sole campaign | Perf tuning only | N/A |

**Incompatibilities (skill):** ASan⊥MSan, ASan⊥TSan — separate campaigns, same seed/corpus set.

### 3.2 Why MSan is conditional (not default)

| Surface | Evidence | MSan? |
|---------|----------|-------|
| `ParsedQuery::parse`, `score_symbol`/`fuse_rrf` | Safe Rust; workspace `unsafe_code = "forbid"` | **No** — low ROI |
| Tree-sitter grammars via `ast-sgrep-lang` | C parsers; `extract.rs` TLS `Parser` reuse | **Yes** when `ParserRegistry` harness lands |
| `ast-sgrep-mmap` | Sole sealed `unsafe` (`memmap2::MmapOptions::map`) | **Yes** for mmap-read harness (file-backed fixtures, not unbounded maps) |
| Product crates generally | Forbid unsafe | MSan only through FFI/native deps |

### 3.3 Why TSan is deferred

PASS4 already has **unit/MR bit-identical** concurrency oracles (`kmeans_bit_identical_under_1_and_4_rayon_threads`, `mr_kmeans_threads_bit_identical`). TSan is **hardening**, not a substitute for those oracles. Surfaces when a concurrency fuzz target exists:

1. `semantic_ann` kmeans build with controlled rayon pool size as fuzzer-controlled u8 (cap 1–4).
2. `regex_pass` multi-thread line scan (budget + thread count).
3. Indexer path: one `ParserRegistry` per rayon worker (product already uses TLS registry — race interest is shared mutable state, not TLS itself).

### 3.4 Recommended env defaults (CI + local)

```bash
# ASan+UBSan campaign
export ASAN_OPTIONS="abort_on_error=1:detect_leaks=0:allocator_may_return_null=1:malloc_context_size=30"
export UBSAN_OPTIONS="print_stacktrace=1:halt_on_error=1"
# Optional if RSS OOM masks real bugs on small runners:
# cargo fuzz run TARGET -- -rss_limit_mb=0

# MSan campaign (native only)
export MSAN_OPTIONS="halt_on_error=1"

# TSan campaign
export TSAN_OPTIONS="halt_on_error=1:second_deadlock_stack=1"
```

### 3.5 Campaign schedule (ops)

| Cadence | Sanitizer | Duration | Notes |
|---------|-----------|----------|-------|
| PR smoke | ASan (default build) | regression corpus only | No multi-sanitizer matrix on PR (time) |
| Nightly deep | ASan+UBSan | 30–60 min/target **[E]** wall budget | Primary discovery |
| Weekly / release | MSan (native targets only) | 30–120 min **[E]** | Rebuild cost high |
| Nightly optional job | TSan (concurrency targets) | 15–30 min **[E]** | After harness exists |

---

## 4. Perf floor plan (cite PASS5 + PASS2)

### 4.1 Skill floors (PERFORMANCE-TUNING)

| Class | Min | Good | Excellent |
|-------|-----|------|-----------|
| Trivial (hash/score) | 5k | 10k | 50k+ |
| Parser | 500 | 1k | 5k+ |
| Stateful | 50 | 100 | 500+ |

### 4.2 Size guards (PASS5 budgets — implement in harness + libFuzzer)

| Target | Harness hard cap | libFuzzer `-max_len` | Rationale (PASS5) |
|--------|------------------|----------------------|-------------------|
| `query_grammar` | `input.len() > 8192 → return` | 8192 | Mode/token grammar; 8 KiB default |
| `rank` | `term ≤ 256`, `symbol ≤ 512`, `ranks.len() ≤ 64`; map huge ranks `% 1024` | structured (Arbitrary) | Keep exec/s; avoid multi-MB lim |
| Future `ParserRegistry` | source ≤ **64–256 KiB** (prefer 64 KiB for CI) | 65536 | Product indexes whole files; harness tightens for exec/s |
| Future LSP frame | ≤ product `MAX_MESSAGE_BYTES` but harness **64 KiB–1 MiB** | same | PASS3 |
| Future IVF / binary | magic+header+capped vectors | 4–64 KiB | PASS5 structure tables |

**Today:** both harnesses **lack** size guards (PASS2 D3). `rank` can allocate huge `Vec<usize>` from Arbitrary.

### 4.3 Init outside body (ParserRegistry)

Product evidence:

```rust
// crates/ast-sgrep-lang/src/lib.rs — ParserRegistry::new() builds HashMap of all langs
// crates/ast-sgrep-lang/src/extract.rs:6–8 — thread_local! TS_PARSERS reuses tree-sitter Parser
// crates/ast-sgrep-core/src/index.rs:~871 — "One ParserRegistry per rayon worker"
```

**Harness rule for future `tree_sitter_source` (name TBD):**

```rust
// Sketch only — not implementing harness this pass
use std::sync::OnceLock;
use ast_sgrep_lang::ParserRegistry;

static REGISTRY: OnceLock<ParserRegistry> = OnceLock::new();

fuzz_target!(|data: (u8 /* lang */, &[u8] /* source */)| {
    if data.1.len() > 65_536 { return; }
    let reg = REGISTRY.get_or_init(ParserRegistry::new);
    let lang = map_lang_byte(data.0);
    let Ok(src) = std::str::from_utf8(data.1) else { return; };
    let _ = reg.parse(lang, src);
});
```

| Init pattern | OK? |
|--------------|-----|
| `OnceLock` / `OnceCell` of `ParserRegistry` | **Required** — `new()` constructs all language parsers |
| Rely on product TLS `TS_PARSERS` only | OK for single-threaded libFuzzer; still need registry once |
| `ParserRegistry::new()` inside body every input | **Forbidden** — destroys exec/s **[E]** order-of-magnitude regression |
| Per-input temp files / process spawn (`ast-grep`) | Offline only; kills PR budgets |

### 4.4 Existing targets: keep floors green

| Target | Floor action |
|--------|--------------|
| `query_grammar` | Already ~30k; add 8 KiB guard so RSS stays bounded (PASS2 saw ~526 Mb without guard) |
| `rank` | Cap vec length; expect stay ≥10k exec/s **[E]** after guards |
| CI assertion (optional later) | Parse libFuzzer `exec/s:` from final stats; fail if `< 1000` on pure targets after warm-up — **do not** invent numbers now |

### 4.5 Max input budgets (product vs harness)

| Product bound (examples) | Harness policy |
|--------------------------|----------------|
| LSP `MAX_MESSAGE_BYTES` | Harness **stricter** |
| IVF `k ≤ 256` | Cap Arbitrary k further for speed |
| Batch `MAX_BATCH_CALLS=32` | Same or lower |  
(From PASS5: harnesses tighten further for exec/s, not re-derive product limits.)

---

## 5. Crash triage pipeline (repo-specific)

### 5.1 End-to-end flow

```
crash in fuzz/artifacts/<target>/crash-*
        │
        ▼
[1] tmin  ── cargo +nightly fuzz tmin <target> <crash-path>
        │      (or libFuzzer -minimize_crash=1)
        ▼
[2] reproduce ── same sanitizer flags as discovery; confirm deterministic
        │
        ▼
[3] stack-hash dedup ── hash top-N frames (see 5.2); check ledger
        │
        ▼
[4] classify ── ASan/UBSan/MSan/TSan summary line → severity
        │
        ▼
[5] regression fixture ── commit minimized bytes + unit test (5.3)
        │
        ▼
[6] re-fuzz ── short campaign / PR corpus includes new seed
```

### 5.2 Stack-hash dedup convention

| Item | Convention |
|------|------------|
| Hash input | Sanitizer stack frames **inside product crates** only (`ast_sgrep_*`, not libFuzzer/ASan runtime) |
| Frames | Top **5** function+file paths (strip addresses/offsets) |
| Algorithm | `sha256(join("\n", frames))[:16]` hex |
| Ledger file (future) | `tests/artifacts/fuzz-audit/crash-ledger.jsonl` **or** comments in regression mods — one line per unique hash |
| Duplicate action | Keep smallest tmin input; discard larger repros |

Pseudo:

```bash
# After tmin, extract SUMMARY + frames #0..#N from asan log
# hash=$(printf '%s\n' "${frames[@]}" | shasum -a 256 | cut -c1-16)
# grep -q "$hash" tests/.../crash-ledger.jsonl && echo DUPLICATE
```

### 5.3 Regression test location convention

| Surface | Preferred location | Pattern |
|---------|-------------------|---------|
| Core query/rank/search | `crates/ast-sgrep-core/tests/fuzz_regressions/` | `#[test] fn fuzz_reg_<hash_prefix>()` |
| Lang / tree-sitter | `crates/ast-sgrep-lang/tests/fuzz_regressions/` | same |
| LSP framing | `crates/ast-sgrep-lsp/tests/` or existing `tests/lsp.rs` style | unit |
| Mmap | `crates/ast-sgrep-mmap/src/` tests module | unit |
| Cross-crate / binary | `tests/fuzz_regressions/` (workspace integration) | last resort |

**Fixture storage:**

```
crates/ast-sgrep-core/tests/fuzz_regressions/
  mod.rs                    # #[path] or mod declarations
  query_grammar_a1b2c3d4.in # minimized bytes (or .txt)
  query_grammar_a1b2c3d4.rs # optional if large setup
```

Template:

```rust
#[test]
fn fuzz_reg_query_grammar_a1b2c3d4() {
    let input = include_str!("fuzz_regressions/query_grammar_a1b2c3d4.in");
    // Must not panic / must satisfy same oracle as harness
    let q = ast_sgrep_core::ParsedQuery::parse(input);
    let _ = (q.mode, q.target, q.terms, q.raw);
}
```

**Rules:**

- Prefer **unit tests** over `cargo fuzz run` for PR (deterministic, no nightly required).
- Also copy minimized seed into **L1** `fuzz/seed_corpus/<target>/` (PASS5) so discovery fuzzer continues to cover the path.
- Never commit raw multi-MB crashes; **tmin first**.
- `fuzz/artifacts/` stays local/CI-upload only — not the long-term source of truth.

### 5.4 Missing today

| Piece | Status |
|-------|--------|
| `tmin` / `cmin` scripts | Absent (PASS2 D8; PASS5 outlined ops only) |
| Stack-hash ledger | Absent |
| `fuzz_regressions` dirs | Absent |
| CI upload of `fuzz/artifacts/` on failure | Absent |
| CONTRIBUTING triage section | Absent |

---

## 6. CI matrix design

### 6.1 Tiers

| Tier | Trigger | Goal | Targets | Time budget | Sanitizer |
|------|---------|------|---------|-------------|-----------|
| **PR short smoke** | `pull_request` (+ optional `push` to main) | Regression: old seeds must not crash; unit fuzz_regressions | All **shipped** bins + `cargo test` fuzz_reg modules | **5–8 min** wall job timeout | Default ASan build **or** plain unit tests for fixtures |
| **Dispatch / release gate** | `workflow_dispatch` + `local-release-gate.sh` | Bounded discovery smoke | `query_grammar` + `rank` (both) | **30s each** `max_total_time` (status quo) | Default ASan; prefer ASan+UBSan when stable |
| **Nightly deep** | `schedule: cron` + `workflow_dispatch` | Coverage growth, new crashes | All targets + future native | **30–60 min / target** **[E]**; job timeout 120–360 min | ASan+UBSan primary; optional MSan/TSan jobs |
| **Release gate (official)** | `pi-npm-release` → `local-release-gate.sh` | Ship bar | rank today; **must add query_grammar** | 30s each | Default |

### 6.2 Per-target max_total_time / runs

| Context | `query_grammar` | `rank` | Future tree-sitter | Notes |
|---------|-----------------|--------|--------------------|-------|
| PR (corpus regression) | `-runs=10000` **or** `-max_total_time=60` | same | `-max_total_time=120` **[E]** | Prefer committed L1 seeds; deterministic-ish |
| Dispatch bounded | `-max_total_time=30 -timeout=5` | same | 30–60 | Match current CI intent |
| Nightly deep | `-max_total_time=1800` **[E]** | 1800 | 3600 (native slower) **[E]** | Cache corpus between runs |
| Release gate | 30 | 30 | optional if harnessed | Fail closed |

Also always: harness max + `-max_len=…`, `-timeout=5` (or 10 for native), optional `-dict=…` when dicts land (PASS5).

### 6.3 PR vs nightly artifact policy

| Artifact | PR | Nightly |
|----------|----|---------|
| Crash upload | `actions/upload-artifact` on failure | same + retain longer |
| Corpus cache | restore L2 optional; **never** require | cache `fuzz/corpus/` with restore-keys |
| cmin after run | no | yes (best-effort) |
| Unit regressions | **must pass** | must pass |

### 6.4 Proposed CI YAML outline (markdown only — not applied)

```yaml
# Outline A: fix + split tiers (illustrative)
# --- .github/workflows/ci.yml (extend triggers) ---
name: CI
on:
  pull_request:
  workflow_dispatch:

jobs:
  # Keep existing forbid-soundness / cargo-check on PR when re-enabled.
  fuzz-regression:
    if: github.event_name == 'pull_request' || github.event_name == 'workflow_dispatch'
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: fuzz -> target
      - name: Install cargo-fuzz
        run: command -v cargo-fuzz >/dev/null || cargo install cargo-fuzz --locked
      - name: Assert fuzz bin names
        working-directory: fuzz
        run: |
          list=$(cargo +nightly fuzz list)
          echo "$list" | grep -qx query_grammar
          echo "$list" | grep -qx rank
      # Unit-level regressions (no nightly required once fixtures exist):
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p ast-sgrep-core --test fuzz_regressions --locked
        continue-on-error: true  # until module exists; remove later
      # Seed/corpus smoke (after L1 seeds committed — PASS5):
      - name: Fuzz query_grammar regression budget
        working-directory: fuzz
        run: |
          cargo +nightly fuzz run query_grammar -- \
            -max_total_time=60 -timeout=5 -max_len=8192 -runs=10000
      - name: Fuzz rank regression budget
        working-directory: fuzz
        run: |
          cargo +nightly fuzz run rank -- \
            -max_total_time=60 -timeout=5
      - name: Upload crashes
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: fuzz-crashes-pr
          path: fuzz/artifacts/

  bounded-fuzz:
    if: github.event_name == 'workflow_dispatch'
    name: Bounded discovery fuzz
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      # … same toolchain/cache as today …
      - name: Fuzz query_grammar  # FIXED name
        working-directory: fuzz
        run: cargo +nightly fuzz run query_grammar -- -max_total_time=30 -timeout=5 -max_len=8192
      - name: Fuzz rank
        working-directory: fuzz
        run: cargo +nightly fuzz run rank -- -max_total_time=30 -timeout=5

# --- .github/workflows/fuzz-nightly.yml (new outline) ---
name: Fuzz nightly
on:
  schedule:
    - cron: '0 2 * * *'   # 02:00 UTC
  workflow_dispatch:

jobs:
  deep-asan-ubsan:
    runs-on: ubuntu-latest
    timeout-minutes: 180
    strategy:
      fail-fast: false
      matrix:
        target: [query_grammar, rank]
        # extend: tree_sitter_source, … when harnessed
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - uses: actions/cache@v4
        with:
          path: fuzz/corpus/${{ matrix.target }}
          key: fuzz-corpus-${{ matrix.target }}-${{ github.sha }}
          restore-keys: fuzz-corpus-${{ matrix.target }}-
      - run: cargo install cargo-fuzz --locked
        # if missing
      - name: Deep fuzz
        working-directory: fuzz
        env:
          ASAN_OPTIONS: abort_on_error=1:detect_leaks=0:allocator_may_return_null=1
          UBSAN_OPTIONS: print_stacktrace=1:halt_on_error=1
          # When cargo-fuzz supports combined sanitizers in-tree, pin here.
          # RUSTFLAGS: -Zsanitizer=address,undefined
        run: |
          cargo +nightly fuzz run ${{ matrix.target }} -- \
            -max_total_time=1800 -timeout=10 -print_final_stats=1
      - name: cmin (best effort)
        if: always()
        working-directory: fuzz
        run: cargo +nightly fuzz cmin ${{ matrix.target }} || true
      - uses: actions/upload-artifact@v4
        if: failure()
        with:
          name: fuzz-crashes-nightly-${{ matrix.target }}
          path: fuzz/artifacts/${{ matrix.target }}/

  # Optional weekly job outline:
  # deep-msan: only matrix targets that touch tree-sitter / mmap
  # deep-tsan: only concurrency targets
```

### 6.5 Release gate parity outline

```bash
# scripts/local-release-gate.sh — proposed parity (not applied)
(
  cd fuzz
  cargo +nightly fuzz run query_grammar -- -max_total_time=30 -timeout=5 -max_len=8192
  cargo +nightly fuzz run rank -- -max_total_time=30 -timeout=5
)
```

---

## 7. Gap list ranked for bead aggregation (epic-level language)

Not micro-beads — aggregate into few epics later.

| Rank | Gap ID | Theme | Why it matters | Evidence |
|------|--------|-------|----------------|----------|
| **1** | **G-CI-NAME** | P0 wrong fuzz bin in CI | Query fuzz never runs in Actions | `ci.yml:166` vs `fuzz/Cargo.toml:14–16`; PASS2 repro |
| **2** | **G-CI-TRIGGERS** | No PR / no schedule fuzz; docs claim PR soundness | Continuous regression bar missing; CONTRIBUTING stale | All workflows `workflow_dispatch`; no `schedule` |
| **3** | **G-RELEASE-PARITY** | Gate only runs `rank` | Ship without query harness smoke | `local-release-gate.sh:15–16` |
| **4** | **G-SIZE-PERF** | No size guards; rank Arbitrary unbounded | OOM/RSS masks bugs; floor risk under load | PASS2 D3; PASS5 budgets; query RSS ~526 Mb smoke |
| **5** | **G-SEED-DICT** | No committed L1 seeds/dicts | Cold start; weak mode coverage | PASS2/5; gitignore corpus |
| **6** | **G-SANITIZER-MATRIX** | ASan-only; no UBSan/MSan/TSan campaigns | Skill rule 5–7 fail; native C unguarded when harnessed | PASS2 D7; tree-sitter + mmap justification |
| **7** | **G-TRIAGE-REGRESSION** | No tmin → hash → unit fixture pipeline | Crashes won't stick; no PR regression corpus | PASS2 D8; zero `fuzz_regressions` |
| **8** | **G-NATIVE-HARNESS-OPS** | No `ParserRegistry` harness yet + OnceLock + MSan plan | Highest-value surface unfuzzed; perf floor depends on init | PASS1 #1; PASS3 OnceLock note; §4.3 this doc |
| **9** | **G-CONCURRENCY-TSAN** | TSan campaign absent; unit oracles only | Races in rayon/regex paths | PASS4 archetype 7 |
| **10** | **G-CORPUS-OPS** | No cmin/tmin automation; no crash artifact upload | Nightly corpus rot; triage friction | PASS5 ops; CI missing upload |

---

## 8. Gap table (pipeline-focused summary)

| Gap | Priority | Effort | Depends on | Acceptance check |
|-----|----------|--------|------------|------------------|
| Fix `parsed_query` → `query_grammar` | P0 | XS | none | `cargo +nightly fuzz list` + dispatch job green |
| Release gate runs both bins | P0/P1 | XS | name fix | `local-release-gate.sh` greps both |
| PR fuzz regression job | P1 | S | seeds helpful | PR runs ≤15 min; fails on crash |
| Size guards + max_len | P1 | S | none | Harness early-return; RSS stable |
| L1 seeds + dict (PASS5) | P1 | S | none | Committed ≥5 seeds/target |
| Crash → unit regression convention | P1 | S | first real crash or synthetic | `cargo test` includes fixture |
| ASan+UBSan default flags | P2 | S | CI matrix | Documented env; nightly uses them |
| Nightly deep + corpus cache | P2 | M | name fix, seeds | schedule workflow exists |
| MSan for tree-sitter/mmap | P2 | M | native harness | Separate job; shared cmin |
| TSan concurrency campaign | P3 | M | concurrency harness | Bit-identical oracle + TSan clean |
| exec/s CI floor assert | P3 | S | stable stats parse | Fail if pure target <<1k **[E]** |

---

## 9. Already correct (≥3)

1. **`fuzz/` workspace exclusion** with documented soundness rationale (`Cargo.toml` exclude + `SECURITY.md`) — product `forbid(unsafe_code)` stays intact while cargo-fuzz can use ASan tooling.
2. **Bounded-time fuzz intent** already encoded: `-max_total_time=30 -timeout=5` in CI and local release gate; job `timeout-minutes: 10` on `bounded-fuzz`.
3. **Measured exec/s far above skill floors** for both existing targets (~30k / ~55k+ from PASS2 short smokes) — init-outside-body is fine for current pure targets.
4. **`rank` ships a real oracle** (finite scores, range, reverse-RRF metamorphic) — stronger than crash-only (PASS2/PASS4).
5. **Tooling cache skeleton** present: `Swatinem/rust-cache` with `workspaces: fuzz -> target` + `actions/cache` for `cargo-fuzz` binary in `ci.yml` and `pi-npm-release.yml`.
6. **Gitignore of evolved corpus / target** (`fuzz/corpus/`, `fuzz/target/`) matches PASS5 L2 policy (keep L1 seeds separate when added).
7. **Concurrency logic oracles already in unit/MR tests** for kmeans threads — TSan is additive hardening, not a total gap in correctness coverage (PASS4).

---

## 10. Roll-up for epic-level bead language

Use these as **epic titles / problem statements** when aggregating (do **not** file beads in this pass):

### Epic A — **Make continuous fuzz real (CI truth)**
> CI claims bounded parser fuzzing but runs a non-existent bin `parsed_query` (`ci.yml:166`); release gate only smokes `rank`. All Actions workflows are `workflow_dispatch` only, and CONTRIBUTING incorrectly claims PR `forbid-soundness`. Fix bin name, dual-target gate, PR regression tier, and optional nightly schedule with corpus cache.

### Epic B — **Perf floors & harness hygiene on existing bins**
> Add PASS5 size budgets (`query_grammar` 8 KiB; `rank` string/vec caps), L1 seed corpus + dict, keep ≥1k exec/s. No new harnesses required for this epic.

### Epic C — **Sanitizer program (ASan+UBSan default; MSan/TSan gated)**
> Default discovery = ASan+UBSan with shared corpora. MSan only for tree-sitter/`ParserRegistry` and sealed mmap once harnessed. TSan only for rayon/regex/CodeMode concurrency surfaces after dedicated harnesses; reuse existing bit-identical oracles.

### Epic D — **Crash factory: tmin → stack-hash → unit regression**
> Standardize `cargo fuzz tmin`, stack-hash dedup, and `crates/<pkg>/tests/fuzz_regressions/` + L1 seed promotion so every unique crash becomes a PR-blocking unit test.

### Epic E — **Native path readiness (ops constraints only here)**
> When `ParserRegistry` harness lands (other pass): `OnceLock` registry, 64 KiB source cap, MSan campaign, TLS-aware single-thread libFuzzer first. Do not rebuild registry per input.

---

## 11. Top pipeline gaps (executive)

1. **P0 name mismatch** — `parsed_query` vs `query_grammar` breaks CI query fuzz.  
2. **No PR / nightly continuous fuzz** — dispatch-only; docs overclaim.  
3. **No crash→regression bridge** — tmin/dedup/fixtures missing.  
4. **No multi-sanitizer campaigns** — ASan default only; MSan/TSan unplanned in YAML.  
5. **Release gate incomplete** — `rank` only.  
6. **Size guards absent** — perf/OOM floor risk under longer campaigns.

---

## 12. Evidence index (paths)

| Path | Role |
|------|------|
| `/Users/aditya/Developer/ast-sgrep/.github/workflows/ci.yml` | Bounded fuzz; **P0 at :166** |
| `/Users/aditya/Developer/ast-sgrep/.github/workflows/pi-npm-release.yml` | Installs cargo-fuzz; release-acceptance |
| `/Users/aditya/Developer/ast-sgrep/scripts/local-release-gate.sh` | Rank-only 30s fuzz |
| `/Users/aditya/Developer/ast-sgrep/fuzz/Cargo.toml` | Bins `query_grammar`, `rank` |
| `/Users/aditya/Developer/ast-sgrep/fuzz/fuzz_targets/*.rs` | Harnesses |
| `/Users/aditya/Developer/ast-sgrep/CONTRIBUTING.md` | Fuzz docs (partially stale) |
| `/Users/aditya/Developer/ast-sgrep/SECURITY.md` | fuzz exclusion |
| `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-lang/src/lib.rs` | `ParserRegistry` |
| `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-lang/src/extract.rs` | TLS parsers |
| `/Users/aditya/Developer/ast-sgrep/crates/ast-sgrep-mmap/src/lib.rs` | Sealed unsafe mmap |
| `/Users/aditya/Developer/ast-sgrep/tests/artifacts/fuzz-audit/PASS2_*.md` | Measured exec/s + D1–D8 |
| `/Users/aditya/Developer/ast-sgrep/tests/artifacts/fuzz-audit/PASS5_*.md` | Size budgets, corpus layers |

---

## 13. Constraints honored

- Only wrote under `tests/artifacts/fuzz-audit/`.
- No production edits, workflow edits, beads, or commits.
- No new harness implementations.
- Exec/s figures only from PASS2; other rates marked **[E]**.

**Artifact path:** `tests/artifacts/fuzz-audit/PASS6_SANITIZERS_PERF_CI.md`
