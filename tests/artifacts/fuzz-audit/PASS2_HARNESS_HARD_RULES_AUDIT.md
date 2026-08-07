# PASS 2 — Existing Harness Hard-Rules Audit

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Scope:** Audit existing `fuzz/` infrastructure against `testing-fuzzing` hard rules 1–15 + shipping checklist.  
**Out of scope:** New harnesses, beads, production source edits, multi-minute campaigns.  
**Prior:** Pass 1 discovery at `tests/artifacts/fuzz-audit/PASS1_TARGET_DISCOVERY.md`.

### Evidence methods (cheap only)

| Check | Result |
|-------|--------|
| `cargo fuzz --version` | `cargo-fuzz 0.13.2` |
| `cargo +nightly fuzz list` (cwd `fuzz/`) | `query_grammar`, `rank` |
| `cargo +nightly fuzz build query_grammar` | **OK** (`Finished release` ~3m19s first build) |
| `cargo +nightly fuzz build rank` | **OK** |
| `cargo +nightly fuzz run parsed_query …` | **FAIL**: `no bin target named parsed_query` (available: `query_grammar`, `rank`) |
| 5s `query_grammar` run | ~179k runs / 6s → **~30k exec/s**, corp 621, cov 435, rss ~526Mb |
| 3–5s `rank` run | ~218k runs / 4s → **~54k–70k exec/s**, corp 294, cov 299, rss ~159Mb |
| Seed dirs before smoke | **NO** `fuzz/corpus`, **NO** dicts/options, **NO** checked-in artifacts |
| `git ls-files fuzz/` | Only `Cargo.toml` + 2 targets |
| Production dep leak | `cargo tree -p ast-sgrep-core --no-dev` → no `arbitrary`/`libfuzzer`/`bolero` |

---

## 1. Per-target audit

### 1.1 `query_grammar` (`fuzz/fuzz_targets/query_grammar.rs`)

```6:8:fuzz/fuzz_targets/query_grammar.rs
fuzz_target!(|input: &str| {
    let _ = ParsedQuery::parse(input);
});
```

| Checklist / rule aspect | Verdict | Evidence |
|-------------------------|---------|----------|
| Narrow parser boundary | **PASS** | Calls `ParsedQuery::parse` only (`crates/ast-sgrep-core/src/query.rs:20`), not full search pipeline |
| Input size guard | **FAIL** | No `if input.len() > MAX { return; }` |
| Seed corpus (≥5 valid/boundary) | **FAIL** | No tracked seeds; `.gitignore:138–139` ignores `fuzz/corpus/`; no seed generator script |
| Dictionary (mode prefixes) | **FAIL** | No `*.dict` / `fuzz/dicts/`; 5s run emitted “Recommended dictionary” (never checked in) |
| Structure-aware input | **PARTIAL** | Raw `&str` OK for free-form query text; mode prefixes (`callers:`, `defs:`, `pattern:`, …) would benefit from dict/grammar |
| Oracle strength | **FAIL / weak** | Crash-only (`let _ =`); `parse` is **infallible** (`-> Self`), so only panics matter; no mode/terms invariants |
| Init outside body | **PASS** | No heavy per-iteration setup; pure parse |
| Sanitizers | **PARTIAL** | cargo-fuzz default ASan on build; no explicit UBSan/MSan campaign for this target |
| Crash → regression | **FAIL** | No `include_bytes!` regression tests under `tests/`/`crates/` for fuzz crashes |
| Measured exec/s | **PASS** | ~30k exec/s ≫ 1000 floor (parser) |
| Harness bloat | **PARTIAL** | Depends on full `ast-sgrep-core` (`fuzz/Cargo.toml:12`) → tree-sitter stack linked; rss ~526Mb for string parse |

**Archetype:** Crash detector (weak).  
**Overall target grade:** **D** (compiles, fast, too thin on oracle/corpus/guards).

---

### 1.2 `rank` (`fuzz/fuzz_targets/rank.rs`)

```6:19:fuzz/fuzz_targets/rank.rs
fuzz_target!(|data: (&str, &str, Vec<usize>)| {
    let (term, symbol, mut ranks) = data;
    let symbol_score = score_symbol(term, symbol);
    assert!(symbol_score.is_finite());
    assert!((0.0..=SCORE_EXACT_SYMBOL).contains(&symbol_score));

    let fused = fuse_rrf(&ranks, 60.0);
    assert!(fused.is_finite());
    assert!(fused >= 0.0);

    ranks.reverse();
    let reversed = fuse_rrf(&ranks, 60.0);
    let tolerance = f64::EPSILON * ranks.len().max(1) as f64;
    assert!((fused - reversed).abs() <= tolerance);
});
```

| Checklist / rule aspect | Verdict | Evidence |
|-------------------------|---------|----------|
| Narrow computational boundary | **PASS** | Pure `score_symbol` + `fuse_rrf` — no I/O |
| Structure-aware | **PASS** | Structured Arbitrary tuple `(&str, &str, Vec<usize>)` via libfuzzer-sys |
| Input / value size bounds | **FAIL** | Unbounded `&str` and `Vec<usize>`; 3s run grew `lim` to ~3k bytes; OOM risk under longer runs |
| Seed corpus | **FAIL** | Same as above — no tracked seeds |
| Dictionary | **N/A–low** | Ranking is numeric/string similarity; dict less critical than query prefixes |
| Oracle strength | **PASS (good)** | Finite + range bounds + **commutativity of RRF under reverse** (metamorphic-style); strength ~4 on skill hierarchy |
| Init outside body | **PASS** | No per-iteration init |
| Sanitizers | **PARTIAL** | Default ASan; pure safe Rust so MSan/TSan lower priority |
| Crash → regression | **FAIL** | No fuzz-crash regression fixtures |
| Measured exec/s | **PASS** | ~54k–70k exec/s ≫ 100 (and ≫ 1000) |

**Archetype:** Crash detector + invariant / metamorphic oracle.  
**Overall target grade:** **C+** (strong oracle and speed; missing guards/corpus/CI wiring quality).

---

## 2. Workspace-level audit

| Area | Status | Evidence |
|------|--------|----------|
| cargo-fuzz package layout | **PASS** | `fuzz/Cargo.toml` has `package.metadata.cargo-fuzz = true`, two `[[bin]]` targets, `publish = false` |
| Workspace isolation | **PASS** | Root `Cargo.toml:17–19` `exclude = ["fuzz"]`; documented in `SECURITY.md` “fuzz/ exclusion” |
| Fuzz deps not in production | **PASS** | No libfuzzer/arbitrary/bolero in `ast-sgrep-core` normal deps |
| Seed corpus dirs (tracked) | **FAIL** | `git ls-files fuzz/` has no corpus; `.gitignore:138–139` ignores all of `fuzz/corpus/` with comment “regenerable seeds” but **no regenerator** |
| Dictionaries / `.options` | **FAIL** | None under `fuzz/` |
| Artifacts / crash store policy | **PARTIAL** | `fuzz/target/` gitignored (good); no `artifacts` policy or regression bridge |
| CI bounded fuzz job | **FAIL (broken name)** | `.github/workflows/ci.yml:164–166` runs `parsed_query` (missing); `rank` name is correct (`:168–170`) |
| CI trigger frequency | **FAIL vs checklist** | `bounded-fuzz` gated on `workflow_dispatch` only (`ci.yml:141`); **not** on every PR |
| Nightly / continuous fuzz | **FAIL** | No `schedule:`/`cron` fuzz workflow; no OSS-Fuzz/ClusterFuzzLite |
| Release gate | **PARTIAL** | `scripts/local-release-gate.sh:15–16` runs **only** `rank` 30s; never `query_grammar` |
| Sanitizer multi-campaign | **FAIL** | ASan via cargo-fuzz default only; no UBSan combine, no MSan/TSan docs or jobs |
| Corpus minimization ops | **FAIL** | No `cmin`/`tmin` automation, no triage docs |
| CONTRIBUTING claims | **PARTIAL** | Mentions bounded 30s fuzz + cargo-fuzz (`CONTRIBUTING.md` ~40–45); does not document broken CI target name or seed policy gap |
| CHANGELOG “~867 fuzz corpus fixtures” | **STALE / absent** | `CHANGELOG.md:38` references historical corpus volume; **not present** in tree now |

### CI name mismatch (critical)

```text
CI:     cargo +nightly fuzz run parsed_query …
Actual: [[bin]] name = "query_grammar"
Repro:  cargo +nightly fuzz run parsed_query → error: no bin target named `parsed_query`
```

Any `workflow_dispatch` run of `bounded-fuzz` **fails the query step** before completing the matrix.

---

## 3. Hard rules 1–15 scorecard

| # | Rule | Verdict | Evidence / notes |
|---|------|---------|------------------|
| 1 | ≥1000 exec/s parsers / ≥100 stateful | **PASS** (existing targets) | `query_grammar` ~30k; `rank` ~55k+ in short smokes. *Not measured* for unfuzzed heavy targets (tree-sitter, IVF, wire). |
| 2 | Fuzz the parser, not the application | **PASS** (existing) / **FAIL** (coverage of surfaces) | Existing harnesses are narrow. Pass 1 high-value surfaces (tree-sitter, pattern match, index_content, regex, wire) still unfuzzed. |
| 3 | Every untrusted `pub fn` bytes/str/Read must have a target | **FAIL** | Only 2 targets; Pass 1 matrix shows many high-score unfuzzed boundaries. |
| 4 | Structure-aware beats random bytes | **PARTIAL** | `rank` structured; `query_grammar` raw `&str` without grammar/dict. |
| 5 | ASan without UBSan is half a tool | **FAIL** | cargo-fuzz builds used `-Zsanitizer=address` only (observed in failed `parsed_query` build flags). No UBSan flags in CI/scripts. |
| 6 | Separate sanitizer campaigns, shared corpora | **FAIL** | Single default campaign; no MSan/TSan jobs; corpus not shared/versioned. |
| 7 | MSan needs full instrumented deps | **N/A → FAIL for native paths** | Current targets stay in safe Rust scoring/parse. Tree-sitter/C deps (Pass 1) would need MSan plan when harnessed; **none planned**. |
| 8 | Corpus bloat kills speed — minimize | **FAIL** | No `cmin` process; gitignore drops corpus entirely so org-wide shared minimized corpus cannot exist. |
| 9 | Minimize before debugging | **FAIL** | No triage pipeline / `tmin` docs/scripts. |
| 10 | Every crash → regression test | **FAIL** | Zero crash→unit-test bridge found. |
| 11 | Dedup crashes by stack hash | **FAIL** | No triage/dedup tooling or policy. |
| 12 | Refactor until isolatable | **PASS** (for these two) | Both targets call pure-ish APIs. Broader I/O-entangled surfaces deferred to Pass 3. |
| 13 | One-time init outside body | **PASS** | Both targets have trivial bodies; no init-in-loop anti-pattern. |
| 14 | ROI power law — breadth before depth | **FAIL** | Only 2 thin targets; long-tail unfuzzed (Pass 1). |
| 15 | Coverage plateau → change strategy | **FAIL** | No dicts, no CMPLOG/hybrid docs, no plateau playbook; 5s run already recommended a dict that was discarded. |

**Hard-rules summary:** **4 PASS**, **2 PARTIAL**, **1 N/A/FAIL**, **8 FAIL** (counting N/A→FAIL as fail for workspace readiness).

---

## 4. Shipping checklist scorecard

| Checklist item | Verdict | Evidence |
|----------------|---------|----------|
| Target function identified (narrowest boundary) | **PASS** (per existing) | `ParsedQuery::parse`, `score_symbol`/`fuse_rrf` |
| Code is fuzzable (bytes/reader, no I/O, deterministic) | **PASS** | Both pure; no sockets/FS in harness |
| Seed corpus (empty + valid + boundary + adversarial, ≥5) | **FAIL** | None tracked; gitignore + no generator |
| Dictionary for structured formats | **FAIL** | Query mode prefixes unseeded; no `.dict` |
| Input size bounded | **FAIL** | Both targets unbounded |
| Value sizes bounded (Arbitrary) | **FAIL** | `rank` `Vec<usize>` / strings unbounded |
| Strongest available oracle | **PARTIAL** | `rank` good; `query_grammar` crash-only on infallible API |
| ASan + UBSan enabled | **PARTIAL** | ASan default; UBSan missing |
| MSan if unsafe/native | **FAIL** (plan) | Not planned; native code exists elsewhere |
| TSan if concurrency | **N/A** (current targets) | No concurrent harness |
| `let _ =` for expected failures; `assert!` for invariants | **PASS** | Matches skill convention in both files |
| Invariant checks at end of harness | **PARTIAL** | `rank` yes; `query_grammar` none |
| Init outside body (exec/s > 1000) | **PASS** | Measured above floor |
| Crash artifacts → regression tests | **FAIL** | Missing pipeline |
| CI runs regression corpus on every PR | **FAIL** | Fuzz not on PR; no regression corpus |
| Nightly continuous fuzzing | **FAIL** | Absent |
| No fuzz deps in production | **PASS** | Verified via cargo tree |
| Coverage report reviewed | **FAIL** | No harness coverage report / gate |

**Checklist tally:** **6 PASS**, **3 PARTIAL**, **1 N/A**, **8 FAIL**.

---

## 5. Defect list (severity-ranked)

| Sev | ID | Defect | Path anchors |
|-----|-----|--------|--------------|
| **P0** | D1 | **CI fuzz target name mismatch** — `parsed_query` does not exist; job cannot fuzz the query parser | `.github/workflows/ci.yml:164–166` vs `fuzz/Cargo.toml:14–16` (`query_grammar`) |
| **P1** | D2 | **No seed corpus and corpus explicitly untracked** — every machine cold-starts; “regenerable seeds” policy without a regenerator | `.gitignore:138–139`; missing `fuzz/corpus/*` in git; no `scripts/*fuzz*seed*` |
| **P1** | D3 | **No input/value size guards** — OOM can mask real bugs; rank lim already multi-KB in seconds | `fuzz/fuzz_targets/query_grammar.rs:6–8`; `rank.rs:6` (`Vec<usize>`) |
| **P1** | D4 | **No dictionary for query mode grammar** — wastes cycles on invalid-ish mutations; libFuzzer itself recommended dict tokens | Missing `fuzz/dicts/query_grammar.dict`; runtime “Recommended dictionary” on 5s run |
| **P1** | D5 | **Fuzz not on PR path; dispatch-only + release-gate partial** — checklist wants regression corpus every PR | `ci.yml:141` `workflow_dispatch`; `local-release-gate.sh:15–16` only `rank` |
| **P2** | D6 | **`query_grammar` oracle too weak** — infallible parse + crash-only misses logic bugs (mode prefix, term casing, raw retention) | `query_grammar.rs:6–8`; `query.rs:20` `-> Self` |
| **P2** | D7 | **No multi-sanitizer campaigns / no UBSan** | CI/scripts lack `undefined`/`memory`/`thread` sanitizer matrix |
| **P2** | D8 | **No crash triage → regression pipeline** (tmin, stack-hash dedup, unit test fixture) | No tests referencing `fuzz/artifacts` or crash bytes |
| **P2** | D9 | **Rule 3 coverage gap** — only 2 of many untrusted surfaces fuzzed | Pass 1 matrix; only `query_grammar` + `rank` bins |
| **P3** | D10 | **No corpus minimization / plateau strategy** | No `cmin` automation; rule 8/15 |
| **P3** | D11 | **Heavy dependency surface for thin query harness** | `fuzz/Cargo.toml:12` path-depends full core → large rss for str parse |
| **P3** | D12 | **Stale CHANGELOG corpus claim** | `CHANGELOG.md:38` “~867 fuzz corpus fixtures” vs empty tracked corpus |

---

## 6. Checked but already correct (≥3)

1. **Proper cargo-fuzz package metadata and dual bin targets** — `fuzz/Cargo.toml:7–8`, `14–26`; `cargo fuzz list` returns both names.  
2. **Product forbid-soundness isolation** — workspace `exclude = ["fuzz"]` (`Cargo.toml:17–19`) + `SECURITY.md` rationale; fuzz may use facilities product forbids without weakening crates.  
3. **No production pollution by fuzz crates** — `libfuzzer-sys` only in `fuzz/`; core normal tree clean of `arbitrary`/`libfuzzer`/`bolero`.  
4. **`rank` uses a stronger-than-crash oracle** — finite/range asserts + reverse-order RRF equality (`rank.rs:9–19`), matching skill invariant/`let _` vs `assert!` convention.  
5. **Measured throughput exceeds skill floors** — both targets tens of thousands of exec/s on short local smokes (rule 1).  
6. **Narrow boundaries for existing targets** — not whole-program fuzzing; parse and pure rank only (rule 2 for what exists).  
7. **Build artifact hygiene** — `fuzz/target/` and `fuzz/Cargo.lock` gitignored (`.gitignore:105–106`); CONTRIBUTING warns not to commit `fuzz/target/`.

---

## 7. Recommendations (for later beads — do **not** file now)

Fold into aggregated follow-ups later; ordered for ROI:

1. **Fix CI target name** `parsed_query` → `query_grammar` (or rename bin to match docs). Smoke-verify `cargo +nightly fuzz list` in CI before run.  
2. **Introduce tracked seed sets** without abandoning gitignore of *evolving* corpus: e.g. commit `fuzz/seed_corpus/{query_grammar,rank}/` (≥5 each: empty, minimal, mode-prefix, unicode, adversarial) and copy into `fuzz/corpus/` at run start; keep large auto-grown corpus ignored.  
3. **Add dictionaries** for query prefixes: `callers:`, `defs:`, `imports:`, `pattern:`, `literal:`, `regex:`, `word:`, common tokens. Wire via cargo-fuzz dict flag or `fuzz/dict/*.dict`.  
4. **Size guards**: query string cap (e.g. 4–64 KiB); rank cap string lens + `ranks.len()` (e.g. ≤256).  
5. **Strengthen `query_grammar` oracle**: after parse, assert mode/prefix consistency, `raw` retention, term non-panic properties; optional differential vs constructor paths (`literal`/`regex`/`word`).  
6. **CI matrix**: PR job runs **regression corpus only** (fast, deterministic); keep 30s discover run on release/dispatch; add weekly schedule for longer fuzz.  
7. **Release gate parity**: run both targets (or corpus regression of both) in `local-release-gate.sh`.  
8. **Sanitizer campaigns**: document ASan+UBSan default; optional MSan when tree-sitter/mmap harnesses land.  
9. **Crash → regression**: template test that `include_bytes!` minimized crashes under `crates/…/tests/fuzz_regressions/`.  
10. **Breadth (Pass 1 feed):** prioritize new harnesses for tree-sitter extract, pattern match, index decode, regex path, wire/MCP framing — rule 14.  
11. **Optional:** slim fuzz deps via a `ast-sgrep-core` feature flag exposing only query/rank modules to cut rss/link time.  
12. **Ops:** script `fuzz-cmin.sh` / document `cargo fuzz cmin` + `tmin` in CONTRIBUTING.

---

## 8. Grade roll-up

| Layer | Grade | One-line |
|-------|-------|----------|
| `query_grammar` | D | Fast crash-only shell; no seeds/dict/guards; CI misnamed |
| `rank` | C+ | Good invariants + speed; still unseeded/unbounded |
| Workspace fuzz program | **D+** | Skeleton + isolation good; CI broken for query; no corpus/dict/sanitizer/triage program |

**Ship-ready?** **No.** Minimum bar to call the existing harnesses “shipped” under this skill: fix D1, add seeds+size guards (D2/D3), wire correct names into CI/release, and add at least a regression-corpus PR check.

---

## 9. Artifact control

| Field | Value |
|-------|-------|
| This file | `tests/artifacts/fuzz-audit/PASS2_HARNESS_HARD_RULES_AUDIT.md` |
| Prior | `tests/artifacts/fuzz-audit/PASS1_TARGET_DISCOVERY.md` |
| Production edits | **None** |
| Beads created | **None** |
| Commits | **None** |
| Local smoke side-effects (untracked) | `fuzz/target/`, `fuzz/corpus/query_grammar/`, `fuzz/corpus/rank/`, `fuzz/artifacts/` (gitignored; do not commit) |
