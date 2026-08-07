# Pass 4/10 — Differential & Reference-Impl Gaps

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (audit only; no beads, no implementation, no commits)  
**Date:** 2026-08-07  
**Skill:** `testing-conformance-harnesses` Pattern 1 (differential testing)  
**Prior:** [`PASS1_SPEC_SURFACE_INVENTORY.md`](./PASS1_SPEC_SURFACE_INVENTORY.md), [`PASS2_HARNESS_ARCHITECTURE.md`](./PASS2_HARNESS_ARCHITECTURE.md), [`PASS3_COVERAGE_ACCOUNTING.md`](./PASS3_COVERAGE_ACCOUNTING.md)  
**Scope:** External oracles, existing differentials, gap matrix, honesty vs bench "parity clean". **No implementation. No beads. No commits.**

---

## 0. Executive summary

This repo has a **strong internal oracle / peer-parity culture** and **almost no Pattern-1 external differential correctness harnesses**.

| Class | Present? | Role today |
|-------|:--------:|------------|
| Peer-surface HitKey parity (CLI / core / LSP) | **Yes** | Detects surface drift; all three can share a bug |
| Internal reference path (IVF CE-003 vs brute force) | **Yes** | Real differential against a mathematical reference |
| Fixture oracles (ranking / graph / extraction) | **Yes** | Soft / presence oracles -- not competitor diffs |
| Bench timing vs `rg` / `ast-grep` / `semgrep` | **Yes** (optional / scripts) | **Latency / quality ledgers only** |
| In-tree CI result-set equality vs external tools | **No** | Explicitly deferred (`docs/validation/jell-deferral.md`) |
| Official MCP / LSP protocol suites | **No** | Own process tests only |

**Differential maturity (this pass): 3 / 10** for external Pattern-1; **6 / 10** if internal reference paths (CE-003, HitKey peer) count as differential technique.

**Honesty rule (mandatory):** Rows in `benchmarks/results/speed.md` and `head-to-head.md` that say **"parity clean"** are **historical speed/quality ledgers**. They are **not** evidence of an in-tree correctness differential suite. Treat them as **UNREPRODUCIBLE historical claims** unless a harness + corpus + pins regenerate the match-set diffs in this tree (Agents.md / baselines discipline).

---

## 1. Inventory of external oracles possible today

Probed on this host 2026-08-07. Pins from `Cargo.lock` / docs / `benchmarks/results/*.md`.

### 1.1 Binaries on PATH (host probe)

| Binary | Version on host | Role as oracle | Gated / production? |
|--------|-----------------|----------------|---------------------|
| **`rg` (ripgrep)** | 15.1.0 (rev `48a6ad93f1`) | Lexical match-set / file:line oracle for `literal:` / `word:` / bare text | Optional; **not** production dependency |
| **`ast-grep` / `sg`** | 0.45.0 (Homebrew) | Structural pattern match-set for **supported subset** of `pattern:` | Production **never** spawns for search (`docs/structural-patterns.md`). Bench only if `ASGREP_ALLOW_AST_GREP=1` **and** absolute `ASGREP_AST_GREP` (`docs/env-trust.md`) |
| **`semgrep`** | 1.172.0 | Historical bake-off structural competitor | Not a product dependency; quality ledger only |
| **`hyperfine`** | 1.20.0 | Latency driver for `scripts/run-benchmarks.sh` | Speed only -- not correctness |
| **`asgrep` (self)** | workspace build | DUT | — |

### 1.2 Crates / library pins (in-tree, always available)

| Oracle surface | Pin (Cargo.lock) | Notes |
|----------------|------------------|-------|
| **tree-sitter** core | **0.26.10** | Parse/extract engine; workspace `tree-sitter = "0.26"` |
| tree-sitter-rust | 0.24.2 | |
| tree-sitter-python / go / javascript | 0.25.0 | |
| tree-sitter-typescript | 0.23.2 | |
| tree-sitter-java | 0.23.5 | |
| tree-sitter-c | 0.24.2 | |
| tree-sitter-cpp | 0.23.4 | |
| tree-sitter-ruby | 0.23.1 | |
| tree-sitter-php | 0.24.2 | |
| tree-sitter-swift | 0.7.3 | |
| tree-sitter-kotlin-ng | 1.1.0 | |
| tree-sitter-c-sharp | 0.23.5 | |
| tree-sitter-language | 0.1.7 | |

**tree-sitter as differential oracle:** The product **is** tree-sitter-backed extractors. A true differential would compare **our extraction tuples** (or full AST dumps) against either (a) a second consumer of the same grammar (e.g. tree-sitter CLI `parse` / query), or (b) golden dumps regenerated after intentional grammar bumps -- not "vs a different parser." Today only **presence/forbid tuples** exist (`assert_language_conformance`).

### 1.3 Protocol / ecosystem oracles (partial)

| Oracle | Pin | In-tree use |
|--------|-----|-------------|
| MCP protocolVersion | `2024-11-05` | Own server process tests (`crates/ast-sgrep-mcp/tests/protocol.rs`) -- **not** official MCP conformance runner |
| Machine JSON schema | `1.0.0` | CLI goldens + Pi release-contract -- peer contract freeze |
| IVF wire | magic `ASIVF\0`, version `2`, header 80 | Round-trip + corrupt reject + CE-003 (internal) |
| SQLite schema | `user_version = 7` | Migrations / open integrity -- not external engine diff |
| Pi agent range | `>=0.80.6 <1` | Release contract; not Pi's own suite |

### 1.4 Historical competitor pins (docs only; often unreproducible)

From `benchmarks/results/baselines.md` / `speed.md` (do not re-quote as live CI):

| Competitor | Example published pins | Correctness harness in tree? |
|------------|------------------------|:----------------------------:|
| ripgrep | 14.1.1, 15.1.0 | **No** match-set suite |
| ast-grep | 0.44.1, 0.45.0 | **No** match-set suite (timing only) |
| semgrep | 1.168.0 (ledger) / host 1.172.0 | **No** in-tree suite |

### 1.5 Explicit deferral document

| Path | Claim |
|------|--------|
| [`docs/validation/jell-deferral.md`](../../../docs/validation/jell-deferral.md) | Full cross-engine differential (asgrep vs rg vs ast-grep on shared corpora with identical hit IDs) is **deferred**. Ships ranking/graph/parity oracles instead. Structural = native subset; lexical = FTS-backed, not rg-compatible. |

This is the authoritative honesty statement for external Pattern-1 work.

---

## 2. Existing differential tests (inventory)

Skill Pattern 1 = run reference + DUT, compare outputs. Below: what exists and what it actually compares.

### 2.1 True or near-true differentials

| ID | Path | A side | B side | Comparator | External? | Verdict style |
|----|------|--------|--------|------------|:---------:|---------------|
| **D1 HitKey peer** | `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs` | CLI JSON | core `Searcher` + LSP search | Sorted `SurfaceHitKey` / `HitKey` (file, line, kind, symbol, callee, caller) | **No** (peer) | panic |
| **D2 HitKey multi-format unit** | `crates/ast-sgrep-testkit/src/hit.rs` | native / agent / GitHub / GitLab JSON shapes | expected HitKey | field normalization | **No** | panic |
| **D3 IVF CE-003** | `crates/ast-sgrep-core/tests/semantic_ivf_roundtrip.rs` → `ivf_search_matches_brute_force_top_k_indices_ce003` | IVF `search_flat_with_probes(..., probes=MAX)` | `top_k_flat_similarity` brute set | `HashSet` of top-k indices | **No** (math ref) | panic |
| **D4 Adaptive IVF recall** | same file, quality budget test | adaptive probe path | all-cluster / exact reference | recall@10 ≥ SLO (threshold, not set-eq) | **No** | panic + `#[ignore]` on related tradeoff |
| **D5 IVF byte preserve** | `parity.rs` `index_all_preserves_semantic_ivf_on_noop_and_file_failure` | sidecar before | sidecar after noop/fail | byte `assert_eq!` | **No** | panic |

**CE-003 discipline (protect):** Requires `vector_count >= DEFAULT_ANN_THRESHOLD` (2048 used) so the IVF **cluster path** is exercised -- not the small-n brute early-return that made older tests vacuous. This is the best **internal** Pattern-1-shaped test in the tree.

**HitKey discipline (protect):** Explicit anti soft-skip on empty embed channel; sorted keys for tie-order independence; multi-mode table (`defs:`, `callers:`, `imports:`, `pattern:`, hybrid NL). Extensible comparator for a future external normalizer (file + 1-based line).

### 2.2 Named "parity" / "oracle" that are **not** Pattern-1 external

| Path | What it is | Why not external differential |
|------|------------|-------------------------------|
| `core/tests/parity.rs` | Thin e2e smoke (options, IVF preserve, defs/hybrid/chain) | Self-consistency on sample corpus (Pass 2 score **3/10**) |
| `ranking_oracle.rs` + `cases.json` | Soft `must_include` + `max_rank` | Fixture oracle; no competitor |
| `graph_oracle.rs` | Case-fold defs/callers/imports/chain | Hand fixture; no external graph engine |
| `extraction_goldens` / `assert_language_conformance` | Presence/forbid/pattern tuples over tree-sitter | Same engine family; not dump-vs-CLI |
| `metamorphic.rs` | Relations under transforms | **Explicitly** not absolute oracle; docs point differential at CE-003-style refs |
| `machine_contracts` bench skip | Product field `skipped_reason` when ast-grep not compared | Honesty for vacuous hybrid speedup claims -- still not match-set diff |

### 2.3 Bench / optional external spawn (timing only)

| Path | Behavior |
|------|----------|
| `crates/ast-sgrep-cli/src/bench.rs` → `ast_grep_comparison` | Only for `pattern:` queries; emits `speedup_vs_ast_grep` or `skipped_reason` -- **no hit equality** |
| `crates/ast-sgrep-core/src/pattern.rs` → `bench_ast_grep` | Spawn only if allowlist env + absolute path + `--version` probe |
| `scripts/run-benchmarks.sh` | hyperfine: warm `literal:` vs `rg -n`; structural `pattern:` vs `ast-grep -p` -- **latency JSON only** |
| Historical structural "parity" | `head-to-head.md`: `parity_clean == true` at 23k/100k -- **artifact not in-tree**; definition was normalized file + 1-based line set-diff on a discarded-prefix multi-run speed harness |

### 2.4 Product paths related to ast-grep (correctness-adjacent, not diffs)

| Path | Behavior |
|------|----------|
| `docs/structural-patterns.md` | Native subset; never starts external process for search |
| `pattern.rs` `search_pattern` | Index signatures ∪ native tree-sitter; fail-closed exotic when fallback unavailable (`iva9_7_*`) |
| `lang/tests/pattern.rs` | `needs_ast_grep_fallback` classification for supported vs exotic shapes |
| `docs/comparison.md` / parts of `how-it-works.md` | **Stale prose** still says `pattern:` → ast-grep subprocess / "Requires ast-grep CLI" -- conflicts with structural-patterns + env-trust. **Doc drift risk**, not a harness |

---

## 3. Gap matrix: surface × external oracle × present? × recommended harness

| Surface | External oracle | Present today? | Skill fit | Recommended harness shape (not implementing) | Feasibility |
|---------|-----------------|:--------------:|-----------|----------------------------------------------|-------------|
| **`pattern:` supported subset** | `ast-grep --pattern` (JSON) | **No** equality suite; timing optional | P1 | Fixture table of **supported** patterns; normalize `(relpath, line)`; assert asgrep ⊆ oracle **or** set-eq with documented supersets; **XFAIL/DISC** for intentional empties (nested rules) | **High** -- binary on PATH; need output normalizer + corpus |
| **`pattern:` unsupported / exotic** | ast-grep (would match) | Fail-closed product tests only | P1 + DISC | Cases where ast-grep hits and asgrep returns empty **or** errors: register as **DISC-NNN** ExpectedFailure, not silent green | **High** -- product already documents unsupported |
| **Lexical `literal:` / `word:`** | `rg -n` / `rg --json` | **No** | P1 (subset) | Indexed FTS vs scan: compare **file set** or line hits on stable corpus; document index lag / ignore rules as DISC | **Medium** -- semantics differ (FTS vs scan, gitignore, encoding) |
| **Bare hybrid / NL** | None absolute | ranking soft oracle | MR / soft oracle | Keep metamorphic + ranking_oracle; **do not** force rg/ast-grep equality | n/a by design |
| **Retrieval quality vs semgrep** | semgrep hand patterns | Historical MRR only | quality ledger | Keep bake-off as **eval** with gold + fingerprint; not CI set-eq | Medium -- corpus/gold often unreproducible |
| **Lang extraction** | tree-sitter CLI parse / query, or golden dumps from same crate version | Presence tuples only | P1 dump / P2 golden | Per-lang golden extract JSON with grammar version in meta; regenerate on lock bump with review | **High** (golden) / Medium (CLI second path) |
| **Symbol graph (defs/callers)** | No standard external graph tool | graph_oracle | fixture oracle | Optional: scip / lsif / rust-analyzer dump as **research** oracle -- not default CI | Low–medium |
| **MCP tools** | Official MCP conformance suite / SDK | Own stdio tests | P6 | Process-based against reference client only if suite exists and is stable | Low (suite churn) |
| **LSP methods** | vscode-languageclient / protocol fixtures | Smoke + HitKey peer | P6 | Protocol fixture runner for UTF-16 positions | Medium |
| **IVF / ANN** | Brute force (internal) | **CE-003 yes** | P1 internal | Keep; add versioned binary frame goldens (Pass 3 residual) | Already strong |
| **CLI machine envelope** | Peer Pi/MCP consumers | Goldens internal | P2/P5 | Multi-consumer freeze -- not external competitor | Separate from P1 |
| **Speed vs rg/ast-grep** | hyperfine + competitors | `run-benchmarks.sh` + ledgers | **perf only** | Keep separate from correctness; never gate "conformant" on p95 | Present |

### 3.1 What "parity clean" meant historically (not CI)

From `head-to-head.md` / `speed.md` prose:

- Structural parity = **set diff of normalized (relative file, 1-based line)** after multi-run speed measurement.
- Artifacts: machine-readable dumps **not retained in-tree** (`results-structural-speed.json` etc. referenced but absent).
- Semgrep suite: match totals and "Semgrep-only normalized locations" discussed; many patterns **rejected** by Semgrep -- not pure match-set CI.

**Therefore:** "parity clean" in those docs ≠ proof-pack gate ≠ Pattern-1 conformance.

---

## 4. Honesty: speed / head-to-head ≠ correctness conformance

| Claim class | Location | May treat as correctness evidence? |
|-------------|----------|:----------------------------------:|
| `parity_clean == true` (23k/100k structural) | `head-to-head.md` | **No** -- historical; harness/corpus/artifacts not in tree |
| Warm lexical 24/24 wins vs rg | `head-to-head.md` | **No** -- latency wins; "no unexplained result diffs" is historical prose, not a checked-in suite |
| ≈ ripgrep / structural p95 on self corpus | `speed.md`, `run-benchmarks.sh` | **No** -- timing; may even **lose** on small corpora (honest rows present) |
| MRR vs ripgrep/semgrep gold | `baselines.md`, `losses.md` | Quality ledger only; fingerprints often **UNREPRODUCIBLE** |
| Bench `speedup_vs_ast_grep` | CLI bench JSON | **No** -- timing; skipped when binary missing or non-`pattern:` |
| Proof pack oracles | `proof-pack.md` | **Internal** ranking/graph/machine/MCP/math -- not competitor equality |

**Rule for later passes / docs / beads:** Do not close a "conformance" or "differential correctness" item by pointing at speed.md. Require an **in-tree** driver that:

1. Pins competitor version (`rg --version`, `ast-grep --version`).
2. Pins corpus (path + fingerprint or fixture tree).
3. Emits structured per-case Pass/Fail/XFAIL with normalized hit keys.
4. Records intentional divergences in DISCREPANCIES (or equivalent).

---

## 5. Aggregated findings for beads (max 4 deep items)

> **Not filed this pass.** Themes only -- prioritize **feasible** work without inventing green numbers.

### F1 — Supported-subset `pattern:` × ast-grep match-set differential (highest ROI external P1)

**Why:** Pass 3 ranks pattern subset worst (~0.55 MUST est.); users equate product with ast-grep; production is intentionally native-only.  
**Shape:** Small fixture corpus + table of patterns already in `lang/tests/pattern.rs` / ranking; run DUT `pattern:` and `ast-grep -p` (or JSON); compare normalized `(file, line)` sets; env-gate competitor (`ASGREP_DIFF_AST_GREP` absolute path) so default CI stays offline-friendly **or** require binary in optional job.  
**Deliverables:** harness + DISC entries for unsupported shapes (ExpectedFailure when oracle hits and we empty).  
**Do not:** claim full ast-grep feature parity; do not gate on latency.

### F2 — Formalize jell deferral as DISC + COVERAGE (honesty before bulk harness)

**Why:** `jell-deferral.md` is correct but invisible to CI/report culture; stale docs (`comparison.md`, `how-it-works.md`) still claim ast-grep delegation.  
**Shape:** DISCREPANCIES registry (DISC-pattern-native-subset, DISC-lexical-not-rg, DISC-no-jell-harness) + one COVERAGE row "external differential = deferred"; fix stale comparison prose to match `structural-patterns.md`.  
**Feasible without green numbers:** documentation + XFAIL hooks only.

### F3 — Lexical `literal:` file/line subset vs ripgrep (bounded, with DISC)

**Why:** Competitors and agents expect "grep-like" hits; FTS vs scan differences are real and currently only speed-tested.  
**Shape:** Fixed multi-file fixture; `rg --json` vs asgrep `literal:` / `word:`; compare **file sets** first (weaker) or line sets with known DISC for ignore rules / binary / symlink.  
**Risk:** Overclaiming equality; keep threshold "must not miss any rg hit on fixture" **or** explicit bidirectional DISC -- decide before implementing.  
**Do not:** use hybrid NL or semantic queries against rg.

### F4 — Extraction dump goldens with grammar pins (tree-sitter lock-coupled)

**Why:** 13 languages; grammar bumps silently drop symbols; presence tuples miss span/order regressions (Pass 1 gap #8, Pass 3 lang n/a full contract).  
**Shape:** Pattern 2 goldens generated from **current** extractors (or dual-pass: extract → golden on UPDATE); meta records `tree-sitter-*` versions from lock; fail on dump mismatch. Optional second path: tree-sitter CLI for AST shape only.  
**Why differential-adjacent:** Same family as "reference regenerate fixtures" in skill loop step 3/8; not competitor CLI but versioned reference outputs.

**Out of bead scope (intentionally):** Full jell multi-engine hit-ID system; official MCP suite; scip graph oracle; re-publishing unreproducible 23k/100k parity_clean rows as CI.

---

## 6. Non-goals (intentional)

| Non-goal | Rationale |
|----------|-----------|
| **Full ast-grep feature parity** | Product is an intentional **subset** (no YAML rules, rewrites, nested statement templates, relational metavariable constraints). Use standalone `ast-grep` for those. |
| **rg-compatible FTS** | Lexical modes are index/FTS-backed; ignore rules, cold scan, and unindexed trees differ by design. |
| **Absolute hybrid / NL ranking equality vs any competitor** | Oracle problem (metamorphic + soft ranking_oracle policy). |
| **Treating speed.md / head-to-head as conformance** | Latency and historical quality; many rows UNREPRODUCIBLE. |
| **Spawning ast-grep in production search** | env-trust + structural-patterns; security and latency honesty. |
| **Official MCP protocol suite as default gate** | Own pin + process tests; external suite churn not product-owned. |
| **Replacing metamorphic / CE-003 with external tools** | Internal math refs are the right oracle for ANN. |
| **Inventing green MRR / "parity clean" regeneration without harness** | Agents.md honesty law. |

---

## 7. Top differential opportunities (ranked)

| Rank | Opportunity | External? | Effort | Correctness leverage |
|:----:|-------------|:---------:|:------:|----------------------|
| **1** | Supported `pattern:` set-eq (or ⊆) vs ast-grep on fixture table | Yes | M | High -- closes user confusion + Pass 3 S3 |
| **2** | DISC/COVERAGE + doc alignment for native-subset / jell deferral | Process | S | High honesty; unblocks later P1 |
| **3** | Extraction full dumps + grammar pins | Same-family | M | High -- index quality root |
| **4** | Lexical literal vs rg on controlled fixture | Yes | M | Medium -- semantics need DISC |
| **5** | Extend HitKey peer to MCP compact (+ formats) | Peer | S–M | Surface drift only |
| **6** | Keep CE-003; add historical IVF frame corpus | Internal | S–M | Wire compat (Pass 3 S7) |
| **7** | Optional CI job: env-gated competitor install | Infra | S | Enables 1 & 4 without default CI bloat |
| **8** | Historical parity_clean re-implementation | Yes | L | Only if product wants claim regen; not required for "correct" |

---

## 8. Mapping to skill Pattern 1 architecture

What would a minimal Pattern-1 shell look like **here** (design only):

```text
tests/differential/   (or testkit module)
├── fixtures/corpus/          # small multi-lang tree
├── fixtures/cases.json       # id, mode, pattern/query, oracle, expect
├── normalize.rs              # → (relpath, line[, col]) HitKey-lite
├── runners/
│   ├── asgrep.rs             # CLI --json or core Searcher
│   ├── ast_grep.rs           # Command + JSON parse (gated)
│   └── ripgrep.rs            # rg --json (gated)
├── compare.rs                # set-eq / subset / DISC-aware
└── DISCREPANCIES.md          # intentional divergences
```

**Reuse first:** `HitKey` / `SurfaceHitKey`, ranking `cases.json` loader style (`deny_unknown_fields`), anti soft-skip from testkit safety, CE-003 non-vacuous threshold culture, machine_contracts multi-case failure bags.

**Do not reinvent:** full ConformanceTest crate on day one -- Pass 2 F1 shared shell can host this later.

---

## 9. Cross-links to prior passes

| Pass | Carry-forward for differential |
|------|--------------------------------|
| **P1** | External differential correctness **absent**; false friends `parity` / `parity clean`; competitor table |
| **P2** | HitKey peer score 6; CE-003 protect; metamorphic not conformance; no DISC/COVERAGE |
| **P3** | S3 pattern ~0.55; B2 pattern matrix + DISC vs ast-grep; jell not scored as MUST |

---

## 10. Out of scope (confirmed)

- No implementation of harnesses, goldens, DISC files, or doc fixes  
- No `br` beads filed  
- No commits  
- No re-run of full suite / hyperfine bake-off  
- No new published numbers

---

## 11. Report card

| Item | Value |
|------|--------|
| **Deliverable** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/conformance-audit/PASS4_DIFFERENTIAL_REFERENCE.md` |
| **External oracles inventoried** | rg, ast-grep/sg, semgrep, hyperfine, tree-sitter pins (13 langs), MCP date pin |
| **True differentials in-tree** | HitKey peer; IVF CE-003 (+ related); optional **timing** spawn only |
| **External match-set CI** | **None** (jell deferred) |
| **Honesty** | speed/head-to-head "parity clean" ≠ correctness conformance |
| **Bead themes (unfiled)** | **4** -- F1 pattern×ast-grep · F2 DISC/jell honesty · F3 literal×rg · F4 extract goldens |
| **Top opportunity** | Supported-subset structural match-set vs ast-grep with DISC for intentional empties |
| **Differential maturity** | **3/10** external · **6/10** counting internal refs |
| **Beads filed** | none (per mission) |
