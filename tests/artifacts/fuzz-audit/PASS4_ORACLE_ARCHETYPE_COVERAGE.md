# PASS 4 — Oracle Hierarchy + Archetype Coverage

**Date:** 2026-08-07  
**Scope:** Oracle strength mapping and seven harness archetypes only. No harness implementation, no beads, no production edits.  
**Inputs:** PASS1 / PASS2 / PASS3 under `tests/artifacts/fuzz-audit/`; skill Oracle Hierarchy + Seven Archetypes; in-tree `fuzz/`, `metamorphic.rs`, `properties.rs`, IVF/ANN units.  
**Doctrine:** Use strongest oracle available. Crash-only is acceptable only when no stronger oracle exists.

### Oracle hierarchy (skill)

| Rank | Oracle | Meaning in this repo |
|-----:|--------|----------------------|
| 1 | Reference implementation (differential) | Two impls of same contract; every divergence is a bug |
| 2 | Simplified shadow model | Small deterministic model mirrors expected state |
| 3 | Inverse operation (round-trip) | `decode(encode(x)) == x` (or load(save(x)) equals) |
| 4 | Metamorphic relation | Relation across transformed inputs (subset, scale-invariance, …) |
| 5 | Crash oracle | No panic / no sanitizer hit (worst acceptable) |

### Seven archetypes (skill)

| # | Archetype | Typical oracle |
|---|-----------|----------------|
| 1 | Crash Detector | Crash (5) |
| 2 | Round-Trip | Inverse (3) |
| 3 | Differential | Reference (1) |
| 4 | Stateful | Shadow model (2) ± invariants |
| 5 | Grammar-Based | Crash + structure invariants |
| 6 | Custom Mutator | Varies (structure-preserving mutation) |
| 7 | Concurrency | Invariant under TSan / bit-identical threads |

---

## 1. Surface map — pure-ready + near-ready

Fuzzability status from PASS3. Oracle ranks are **best achievable today** without product seams unless noted. "Stronger blocked by" explains why rank-1/2/3 are unavailable.

### 1.1 Pure-ready (YES today — no product refactor)

| Surface | Best oracle TODAY | Strength | Recommended archetype(s) | Why stronger unavailable / available | Evidence |
|---------|-------------------|:--------:|--------------------------|-------------------------------------|----------|
| `ParsedQuery::parse` | **Metamorphic + structural invariants** (not crash-only) | **4** | Grammar-Based + Crash Detector | Inverse: no `Display`/re-serialize of full grammar. Reference: no second query parser. Shadow: can hand-check mode prefix → mode/target. **Upgrade is free** — API returns `ParsedQuery` with `raw`/`mode`/`target`/`terms`. | `query.rs:20–71`; existing fuzz is crash-only (`query_grammar.rs`); proptest `parse_never_panics` only |
| `score_symbol` / `fuse_rrf` | **Metamorphic + range invariants** | **4** | Crash Detector + MR (already) | Closed-form RRF `1/(k+r+1)` is unit-oracle, not fuzz-primary. No external ranker. Reverse-rank commutativity already in harness. | `fuzz/rank.rs:9–19`; metamorphic header rejects `rrf_rank_monotony` as non-MR |
| `SemanticAnnIndex::read_clusters_bounded` | **Crash + structural invariants** | **4–5** | Crash Detector; **Custom Mutator** for length-prefixed clusters | Round-trip **available**: `write_to` + `read_clusters_bounded` (`semantic_ann.rs:57`, `:104`). Differential: vs `read_clusters_from` on same bytes. No external ANN format reference. | PASS3 pure YES; unit sidecar bit-identical tests |
| `ParserRegistry::parse` (source × lang) | **Crash** (+ light invariants on `ExtractionResult`) | **5** (+4 optional) | Crash Detector + **Grammar-Based** corpus | No second tree-sitter pipeline in-tree. External tree-sitter CLI not a stable product oracle. MR: parse twice = same structure (idempotent extract). Differential across langs is weak (different ASTs). | PASS1 score 12; native C grammars |
| `match_pattern` / `match_literal_pattern` | **Crash + set invariants** | **4–5** | Crash Detector + structure-aware dual input | External **ast-grep** is opt-in bench only (`ASGREP_ALLOW_AST_GREP` + absolute path); not default production reference. For native-classified patterns, differential vs external is **possible but gated**. Shadow: match set ⊆ source lines. | `pattern.rs` native path; `core/pattern.rs:99–113` fail-closed |
| `classify_native` | **Crash + consistency MR** | **4** | Grammar-Based | Inverse N/A. MR: `classify_native(p).is_none() == needs_ast_grep_fallback(p)` (with documented `$` edge cases). No external classifier. | `pattern.rs:47–58`, `:138` |
| LSP `read_message` | **Crash** | **5** | Crash Detector | Inverse: framing re-encode possible (headers from body) → **round-trip upgrade** if harness builds valid frames. Spec reference (LSP Content-Length) is a weak shadow model. | `support.rs:16`; Cursor-ready |
| `try_apply_text_edit` / `utf16_char_to_byte` | **Crash + bounds invariants** (+ weak inverse) | **4** | Crash Detector; optional Round-Trip on identity edits | Identity edit (empty change / full replace then restore) is MR/inverse-ish. Full inverse of arbitrary UTF-16 edits is hard. No second editor model in-tree. | `support.rs:264–310` |
| `file_uri_to_path` / `uri_to_rel_path` | **Crash + confinement invariant** | **4** | Crash Detector | Inverse: path → file URI → path for absolute paths under root. MR: no `..` escape past fixed root. No RFC path library as differential peer. | `support.rs:194–261` |
| `embed_from_bytes` / `embed_to_bytes` | **Round-trip (inverse)** | **3** | Round-Trip | **Strongest free oracle on pure surface.** Finite LE f32 vectors: `from(to(v)) == Ok(v)`. Crash-only on random bytes still needed for bad lengths. No external codec. | `ast-sgrep-embed/src/lib.rs:40–52`; PASS3 "already good" |
| `ServeRequest` / `BatchRequest` serde | **Crash** (+ schema reject invariants) | **5** | Grammar-Based / structure-aware JSON | Inverse: `to_string` of valid enum ≈ parse (serde round-trip). Stateful serve is separate (near-ready). | `batch.rs:85+` |
| User `Regex::new` (mirror `regex_pass`) | **Crash + timeout soft oracle** | **5** | Crash Detector | No second regex engine as differential (onig vs regex would be optional research). ReDoS hangs ≠ panic — wall budget is harness-level. | PASS3 §1.8 |
| `cached_pattern_signatures` / `structural_term_signatures` | **Crash + determinism** | **4–5** | Crash Detector | MR: same pattern → same signatures. No inverse. | `signature.rs` |
| `split_content_lines` | **Crash + join MR** (optional) | **4** | Crash Detector | Weak inverse: join lines with original newlines hard without retaining terminators. Low ROI. | `index.rs:30` |
| `fts::escape_fts_term` | **Property / inverse-ish** | **3–4** | Crash Detector + invariant | Property: output always quoted; `"` doubled. Round-trip not exact. Injection oracle = escaped form never unquoted. | `lib.rs` fts helpers |
| `tokenize` / local embed path | **Crash + finite-vector invariants** | **4–5** | Crash Detector | Math MRs exist in embed `property_tests` (NaN reject, score order). Scale-invariance is ANN-side. | `math.rs` property_tests; semantic embed |
| `Language::parse` / enum maps | Crash (trivial) | **5** | — | Too small; skip harness investment. | PASS1 low score |

### 1.2 Near-ready (PARTIAL / seam before high-ROI)

| Surface | Best oracle TODAY (on pure slice) | Strength | Recommended archetype(s) | Stronger blocked by | Seam note |
|---------|-----------------------------------|:--------:|--------------------------|---------------------|-----------|
| IVF `read_header` | Crash + magic/version/fp invariants | **4–5** | Crash Detector | Round-trip needs full image parse | Private; expose via `parse_ivf_bytes` |
| IVF `map_and_parse` / load | After seam: **Round-trip** `save`→`parse_ivf_bytes` | **3** | Round-Trip + Crash | Path+mmap I/O today | PASS3 §5.1; `save_semantic_ivf` exists (`semantic_ivf.rs:169`) |
| IVF encode/decode clusters | **Round-trip** `write_to`↔`read_clusters_bounded` | **3** | Round-Trip | Already pure if harness builds index in-memory | Units already do sidecar bit-identical |
| Flat ANN vs IVF (`probes >= n`) | **Differential** vs `brute_force_flat` | **1** | Differential | Not a load parser — search oracle | Unit: `ivf_search_matches_brute_force_*` in `semantic_ivf_roundtrip.rs`; fuzzable if pure search harness |
| `Indexer::index_content` | Crash after prepare seam | **5** | Crash Detector | SQLite + wall clock | `prepare_file_content` (PASS3 §5.2) |
| MCP `handle_request` | Crash on parse; meta methods: schema | **5** | Grammar + later Stateful | Private + tool I/O | `parse_jsonrpc_line` + meta dispatch |
| CodeMode `run_serve` | Serde crash today; sequence: shadow | **2–5** | Stateful (later) | Session/tool effects | Mock backend / no-op tools |
| `IgnoreMatcher` / `compile_glob` | Crash + match MR | **4** | Grammar | FS / private | `from_rule_lines` / pub glob |
| `regex_pass` full | Crash | **5** | — prefer Regex::new | Store + threads | Skip full pass |
| `search_pattern` full | Crash / fail-closed | **5** | — prefer match_pattern | Store + WalkDir | Skip full |

---

## 2. Archetype × presence matrix

What exists **today** in `fuzz/` or closely related automated tests that exercise the same oracle style. Y = present at meaningful strength; P = partial / unit-only / not cargo-fuzz; N = absent.

| Archetype | In `fuzz/`? | Related tests / evidence | Grade |
|-----------|:-----------:|--------------------------|-------|
| **1 Crash Detector** | **Y** | `query_grammar` (pure crash); tree-sitter/IVF/LSP **not** fuzzed | Thin: only 2 bins |
| **2 Round-Trip** | **N** | **P:** `semantic_ivf_roundtrip.rs` (save/load + search equal); ANN `write_to`/`read` unit; embed codec **unfuzzed** but API ready; store upsert/delete in `properties.rs` | Strong units, zero fuzz RT |
| **3 Differential** | **N** | **P:** IVF vs brute_force when `probes >= n_clusters` (`semantic_ivf_roundtrip.rs`, `downstream_correctness.rs`); kmeans serial vs parallel units; **no** cargo-fuzz differential; external ast-grep **not** default differential (bench-gated) | High value, unharnessed |
| **4 Stateful** | **N** | **P:** metamorphic reindex idempotent / compound reindex+limit (Searcher + Indexer sequences); MCP protocol tests process-level; CodeMode serve not fuzzed | Sequences in integration tests only |
| **5 Grammar-Based** | **P** | `query_grammar` raw `&str` without dict/grammar (PASS2 D4); `rank` structured Arbitrary tuple; no pattern/JSON grammars in fuzz | Structure-aware only on rank |
| **6 Custom Mutator** | **N** | IVF/ASIVF length-prefixed binary needs structure-preserving mutator; none in tree | Gap for binary formats |
| **7 Concurrency** | **N** | **P:** `mr_kmeans_threads_bit_identical`; unit `kmeans_bit_identical_under_1_and_4_rayon_threads`; CodeMode rayon batch — **no TSan fuzz campaign** | Unit/MR only |

**Summary:** In-tree fuzz is almost entirely **Crash Detector** (+ one **MR-enriched** rank harness). Strong **Round-Trip**, **Differential**, and **Metamorphic** oracles live in **tests**, not in `fuzz/`. Custom mutators and concurrency fuzz are absent.

---

## 3. In-tree metamorphic / property tests → fuzz oracles

### 3.1 `crates/ast-sgrep-core/tests/metamorphic.rs` (primary MR suite)

Implemented MRs (Score ≥ 2.0; names = `fn mr_*`):

| MR | Category | Fuzz-oracle upgrade path |
|----|----------|--------------------------|
| `reindex_idempotent_hits` | equivalence | Stateful index sequence: build → search keys → rebuild → same keys |
| `limit_top_k_subset` | inclusive | Search harness: keys(top_k) ⊆ keys(top_K); score non-increase |
| `keyword_file_must_surface` | inclusive PQS | Seeded corpus: token in file ⇒ hit contains file |
| `ann_query_scale_invariance` (+ proptest) | multiplicative | Pure ANN: scale query by α>0 ⇒ same candidate indices |
| `kmeans_threads_bit_identical` | equivalence | Concurrency: 1 vs N rayon threads identical assignments/centroids |
| `compound_reindex_then_limit` | composition | Stateful + limit subset composed |
| `lang_filter_subset` | inclusive | Filter hits ⊆ unfiltered |
| `query_trim_search_equivalence` | equivalence | `search(q)` vs `search(trim(q))` |
| `ann_probe_monotone_candidates` (+ proptest) | inclusive | `candidates(p) ⊆ candidates(P)` for explicit probes |
| `search_flat_limit_subset` (+ proptest) | inclusive | Flat ANN top-k ⊆ top-K |
| `compound_scale_then_probe_proptest` | composition | Scale MR then probe monotony |

**Explicitly not MRs (prefer differential / unit / inverse):**

| Candidate (documented) | Disposition | Fuzz use |
|------------------------|-------------|----------|
| `ann_exact_eq_bruteforce` | differential / unit | **Best differential fuzz** for IVF search |
| `ivf_write_read_roundtrip` | inverse unit | **Best round-trip fuzz** after pure bytes load |
| `parse_whitespace_equivalence` | DROP — use properties | Upgrade `query_grammar` with structure asserts, not trim-only MR |
| `rrf_rank_monotony` | closed-form unit | Keep in rank unit tests; rank fuzz already has reverse-commutativity |

### 3.2 Other property / oracle tests

| Artifact | What it proves | Fuzz feed |
|----------|----------------|-----------|
| `tests/properties.rs` | `parse_never_panics`; clamp limits; finite scores; store delete round-trip | Parse panic-free already covered by fuzz; **does not** assert parse structure — gap |
| `embed/math.rs` `property_tests` | NaN rejection, score order, normalize | Lift finite/NaN asserts into ANN/embed fuzz |
| `semantic_ivf_roundtrip.rs` | save/load equality; brute_force top-k match at full probes; recall budgets | Seed corpus + differential/RT oracles for future IVF harnesses |
| `ranking_oracle.rs` + fixtures | must_include ranks on sample corpus | Not a fuzz oracle (fixed gold); keep as regression, not fuzzer |
| `pattern_routing.rs` | native empty fail-closed without external ast-grep | Fail-closed invariant for pattern harness |
| `semantic_ann.rs` unit kmeans serial/parallel | Bit-identical concurrency | Oracle for archetype 7 campaigns |
| lang `all_languages_round_trip_as_str_parse` | Language enum string RT | Trivial; skip |

### 3.3 Upgrade recipe (MR → cargo-fuzz)

1. Prefer **pure** slices (`search_flat_with_probes`, `build_from_flat`, `ParsedQuery`) over full Indexer/Searcher where possible (exec/s).  
2. Port assertion body of `mr_*` into harness; generate structure-aware Arbitrary (dim, k, flat, query, probes).  
3. Keep integration MRs (reindex, keyword surface) as **stateful** fuzz only after prepare/store seams or with TempDir + size guards (expect <<1k exec/s).  
4. Do not re-implement rejected MRs as fuzz oracles.

---

## 4. Differential opportunities (detail)

| Pair | Oracle rank | Feasibility | Notes |
|------|:-----------:|-------------|-------|
| **IVF / ANN full-probe vs `brute_force_flat`** | **1** | High (pure) | Same crate; `search_flat_with_probes(..., Some(usize::MAX))` must match brute indices (existing unit `ivf_search_matches_brute_force_top_k_indices_ce003`). Ideal Differential harness. |
| **ANN `search_flat` adaptive vs explicit probes** | 4 (MR) / partial 1 | High | Not always equal — use probe monotony MR, not strict equality |
| **kmeans parallel vs serial row reference** | **1** | Medium | Units exist; concurrency + differential hybrid; control rayon pool |
| **Native `match_pattern` vs external ast-grep** | **1** (conditional) | Medium–low | Requires `ASGREP_ALLOW_AST_GREP=1` + `ASGREP_AST_GREP` abs path; process spawn kills exec/s; flaky if binary missing. Use **only** for offline corpus campaigns on native-classified patterns where semantics are documented to agree. Default product is native-only / fail-closed. |
| **Native vs `needs_ast_grep_fallback` gate** | **4** consistency | High | Not full differential — routing invariant, pure and fast |
| **Flat ANN path (`n < DEFAULT_ANN_THRESHOLD`) vs IVF path** | **1** when forced | Medium | Force both paths on same data; equal top-k when probes cover all clusters |
| **embed LE codec vs hand-rolled `from_le_bytes`** | **1** weak | Low ROI | Same language; trivial |
| **ParsedQuery reparse** | **4** | High | `parse(s)` twice equal (idempotent); not true reparse of `Display` unless added |
| **LSP `read_message` vs manual Content-Length parser** | **2** shadow | Medium | Small shadow model of headers is enough |

**Not recommended as differential:** hybrid ranking vs "true" rank (no absolute oracle — metamorphic suite diagnosis); approximate ANN top-k vs brute at low probes (recall, not equality).

---

## 5. Stateful opportunities (detail)

| System | Sequence sketch | Shadow model | Blockers |
|--------|-----------------|--------------|----------|
| **Index ops** | create index → `index_content`/`prepare` → search → delete file → reindex → search | Expected file set + symbol keys in shadow `BTreeSet` | SQLite + clock (PASS3); use TempDir; slow |
| **ANN build ops** | `build_from_flat` → write → read → search_flat → (optional) rebuild | Cluster counts, member sets, deterministic assignments | Pure-ready if stay in-memory |
| **MCP session** | initialize → tools/list → tools/call × N → shutdown | Method allowlist; id correlation; error shape | Private `handle_request`; tool I/O |
| **CodeMode serve** | NDJSON Call/Batch/End on sticky session | Per-id response; End closes; batch serial/parallel policy | Full tools side effects; mock `CodeModeSession` |
| **CodeMode batch parallel** | Batch of read-only tools under rayon | Result order/id matching serial | Env + index open; TSan optional |
| **LSP document** | open → apply edits → uri resolve | Document buffer shadow string | Needs session harness, not only pure edit |

**Near-term stateful ROI:** in-memory ANN build/search sequences and (post-seam) pure `prepare_file_content` without SQLite. Defer MCP/serve stateful fuzz until parse/meta seams + mock tools.

---

## 6. TOP upgrades — crash-only → stronger oracle (ROI)

Ranked by (oracle strength gain × surface risk × readiness). For later bead aggregation — not atomic micro-beads.

| ROI | Upgrade | From → To | Archetype | Why high ROI | Depends on |
|----:|---------|-----------|-----------|--------------|------------|
| 1 | **`embed_from_bytes`/`embed_to_bytes` round-trip harness** | none → rank **3** | Round-Trip | Pure, 5-line oracle, always-on DB path; zero product change | Nothing |
| 2 | **IVF/ANN differential: full-probe search vs `brute_force_flat`** | unit-only → fuzz rank **1** | Differential | Strongest oracle in repo; pure math; units already define equality | Structure-aware (flat, dim, query, k) |
| 3 | **ANN cluster `write_to` → `read_clusters_bounded` round-trip** | crash load → rank **3** | Round-Trip | Validates only intentional unsafe-adjacent binary path | Size caps; optional Custom Mutator later |
| 4 | **`query_grammar` oracle upgrade** (mode/prefix/`raw`/terms invariants; reparse equality) | rank 5 → **4** | Grammar + Crash | Existing harness; free structural asserts; fixes PASS2 D6 | Dict/seeds (PASS2) |
| 5 | **`read_clusters_bounded` crash harness + structural invariants** (bounds, dups, k/dim) | none → **4–5** | Crash (+ Custom Mutator) | Highest unique pure binary risk (PASS1/3 P0) | Size guards |
| 6 | **Port ANN MRs into pure fuzz** (scale-invariance, probe monotony, limit subset) | unit proptest → continuous fuzz | Crash + MR | Proven Score≥2 MRs; high fault sensitivity | `build_from_flat` init cost — cache or small n |
| 7 | **`classify_native` ↔ `needs_ast_grep_fallback` consistency** | none → **4** | Grammar | Pure, max exec/s, documents routing contract | Edge-case table for `$` patterns |
| 8 | **`try_apply_text_edit` bounds + UTF-8 validity + identity-edit MR** | none → **4** | Crash + MR | Classic editor bugs; pure | Structured Arbitrary ranges |
| 9 | **LSP `read_message` + frame round-trip** (encode Content-Length then parse) | none → **3–5** | Crash + Round-Trip | Framing bugs; Cursor-ready | Cap body size |
| 10 | **IVF full image `parse_ivf_bytes` + save/load RT** | blocked → **3** | Round-Trip + Crash | Completes binary product path | Small seam `parse_ivf_bytes` (PASS3 §5.1) |
| 11 | **URI confinement invariants** (escape / pct_dec) | none → **4** | Crash + invariant | Security-adjacent, pure, fixed root | Synthetic root |
| 12 | **Optional: native pattern vs external ast-grep corpus campaign** | none → **1** offline | Differential | Only when binary pinned; low exec/s | Env allow + corpus of native-equivalent patterns |

**Honorable (lower ROI now):** fts escape properties; ServeRequest serde RT; rank size-guards (oracle already strong — PASS2 C+).

---

## 7. Checked but already strong (≥3 with evidence)

Surfaces / harnesses that **already** sit above pure crash-only and should not be "upgraded for the sake of it":

1. **`fuzz/rank` (`score_symbol` + `fuse_rrf`)** — finite + range bounds + reverse-order RRF equality within `f64::EPSILON * n` (`fuzz/fuzz_targets/rank.rs:9–19`). Oracle strength ~**4**. Gaps are guards/corpus/CI, not oracle hierarchy.  
2. **ANN full-probe vs brute_force (unit)** — reference differential equality for indices at `probes = usize::MAX` / all clusters (`semantic_ivf_roundtrip.rs` `ivf_search_matches_brute_force_top_k_indices_ce003`; also `downstream_correctness`). Strength **1** in tests; needs fuzz **port**, not redesign.  
3. **IVF save/load round-trip (unit)** — fingerprint gate + `search_flat` equality after load (`semantic_ivf_roundtrip_and_fingerprint_gate`). Strength **3**. Explicitly preferred over MR in metamorphic header.  
4. **Metamorphic ANN suite** — scale-invariance, probe monotony, limit subset with proptest variants and Score matrix (`metamorphic.rs` header + `mr_ann_*`). Strength **4**; continuous fuzz is additive throughput, not stronger logic.  
5. **kmeans thread bit-identical (unit + MR)** — 1 vs 4 rayon threads identical assignments/centroids (`semantic_ann.rs` units; `mr_kmeans_threads_bit_identical`). Concurrency oracle **present in tests**; TSan campaign optional hardening, not missing logic oracle.  
6. **embed NaN/score property micro-harness** — `math::property_tests` rejects NaN admission and asserts score order (`embed/src/math.rs`). Strength **4** for ranking numerics; separate from LE byte codec RT gap.

---

## 8. Recommendations for later bead aggregation

Phrase as **work packages**, not one-bead-per-assert:

### WP-A — Oracle-first pure harnesses (no product change)
- New targets: embed LE **round-trip**; ANN **differential** (full probe vs brute); cluster **write/read RT**; `read_clusters_bounded` crash+invariants; `classify_native` consistency; LSP framing/edit/uri; user Regex compile.  
- Upgrade: `query_grammar` structural oracle + dict/seeds (ties PASS2 D4/D6).  
- Shared: size guards, seed corpora, crash→regression bridge (PASS2 D8).

### WP-B — Lift proven MRs into cargo-fuzz (pure ANN path)
- Port `mr_ann_query_scale_invariance`, `mr_ann_probe_monotone_candidates`, `mr_search_flat_limit_subset` into structure-aware pure harnesses.  
- Keep Indexer/Searcher MRs as integration or post-seam stateful fuzz.

### WP-C — Binary format structure awareness
- Custom mutator or structure-aware Arbitrary for ASIVF / cluster length prefixes (Archetype 6).  
- Depends on WP-A load harnesses.

### WP-D — Product seams that unlock stronger oracles
- `parse_ivf_bytes` → full IVF RT + header fuzz (oracle 3).  
- MCP `parse_jsonrpc_line` + meta dispatch → grammar then stateful.  
- `prepare_file_content` → crash + optional reindex MR without wall clock.

### WP-E — Optional offline differential
- Pinned ast-grep binary campaign for native-equivalent patterns only; document semantic agreement subset; never block CI if binary absent.

### WP-F — Concurrency (only after pure oracles ship)
- TSan campaign on kmeans build / CodeMode parallel batch with existing bit-identical oracle; not a substitute for WP-A.

### Explicit non-goals (this pass)
- Do not replace ranking_oracle goldens with fuzz.  
- Do not differential-fuzz approximate ANN at low probes for equality.  
- Do not implement harnesses or file beads in this pass.

---

## 9. Method / evidence index

| Source | Use |
|--------|-----|
| PASS1 matrix + archetype cheat-sheet | Risk ranking, surface list |
| PASS2 query_grammar D / rank C+ | Existing oracle grades |
| PASS3 pure YES / seams | Readiness for stronger harnesses |
| `fuzz/fuzz_targets/{query_grammar,rank}.rs` | Live oracles |
| `metamorphic.rs` strength matrix + rejected candidates | MR inventory |
| `properties.rs`, `semantic_ivf_roundtrip.rs`, `ranking_oracle.rs` | Property / differential / gold |
| `semantic_ann.rs` write/read/brute_force/kmeans | RT + differential + concurrency oracles |
| `ast-sgrep-embed` embed_to/from_bytes + math property_tests | RT + numeric invariants |
| `core/pattern.rs` external ast-grep gates | Differential feasibility constraints |
| Skill Oracle Hierarchy + Seven Archetypes (`testing-fuzzing/SKILL.md`) | Taxonomy |

---

## 10. One-page roll-up

| Question | Answer |
|----------|--------|
| Strongest unused free oracle? | embed LE **round-trip**; ANN **brute_force differential**; cluster **write/read RT** |
| Weakest existing fuzz oracle? | `query_grammar` crash-only on structured `ParsedQuery` |
| Archetypes missing in fuzz/? | Round-Trip, Differential, Stateful, Custom Mutator, Concurrency (all N or P outside fuzz) |
| Best reference impl in-tree? | `brute_force_flat` for ANN search |
| Best inverse in-tree? | `embed_to_bytes`/`from_bytes`; IVF save/load; ANN `write_to`/`read_clusters_*` |
| Best MR suite? | `metamorphic.rs` ANN + search relations (already strong in tests) |
| External ast-grep differential? | Possible offline only; not default product path |

*End of PASS 4 — oracle + archetype coverage only. Artifact: `tests/artifacts/fuzz-audit/PASS4_ORACLE_ARCHETYPE_COVERAGE.md`.*
