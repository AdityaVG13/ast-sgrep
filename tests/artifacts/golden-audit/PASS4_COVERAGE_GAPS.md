# Pass 4 — Complex-Output Coverage Gaps

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (audit only; no tests, beads, or commits)  
**Date:** 2026-08-07  
**Skill:** `testing-golden-artifacts`  
**Prior:** [`PASS1_GOLDEN_INVENTORY.md`](PASS1_GOLDEN_INVENTORY.md), [`PASS2_INFRASTRUCTURE_GAPS.md`](PASS2_INFRASTRUCTURE_GAPS.md), [`PASS3_CLI_MACHINE_GOLDENS.md`](PASS3_CLI_MACHINE_GOLDENS.md)  
**Scope:** Surfaces that emit **complex structured or long-text output** but lack **true golden freezes** (committed expected dumps + equality after scrub). Does **not** re-litigate Pass 3 machine-contract quality; search-hit / teaching / handbook items already designed there are **cross-referenced**, not duplicated as new bead specs.

---

## 1. Executive summary

Pass 1–3 established: machine **envelopes/shapes** are the only true file goldens; everything else is presence, bounds, peer-oracle, or dense hand `assert_eq!`. Pass 4 maps the remaining **complex-output** surface area.

| Class | True dump golden? | Dominant protection today |
|-------|-------------------|---------------------------|
| CLI machine envelopes / capabilities | **Yes** | Scrubbed exact JSON fixtures (Pass 3 score ~8–9) |
| CLI success **hit payloads** (6 formats) | **No** | Shape + bounds; plugins synthetic exact (Pass 3 F1–F3) |
| CLI human long text (`--help`, handbook, teaching) | **No** | Substring / shell greps (Pass 3 F4/F6) |
| Lang extraction full trees | **No** | Presence/forbid tuples (`extraction_goldens.rs`) |
| Core index status / doctor **values** | **No** | Key-shape only (+ sparse field asserts) |
| Core chain / graph expand JSON | **No** | Non-empty + symbol presence oracles |
| Cascade planner / ranking reports | **No** | Behavioral invariants; ranking `must_include`/`max_rank` |
| MCP / LSP protocol dumps | **No** | Tool **names** / field-type smoke; schemas not frozen |
| Codemode catalog + host adapters | **No** | Name presence + a few schema property checks |
| Pi extension tool schemas / argv matrices | **No** | Hand `deepEqual` on names/limits (not files) |
| Benchmark baselines | **Ledger only** | UNREPRODUCIBLE honesty docs -- **not** CI goldens |

**Pass 4 maturity of complex freezes:** ~2/10 true dump coverage outside CLI envelopes. Behavioral nets are stronger than that score implies; they still miss extras, renames, ordering, and full schema drift.

**Top 5 gaps (this pass):**

1. **Language extraction full dumps** (13 langs) -- presence tuples miss extras / kind / span drift  
2. **MCP `tools/list` full tool descriptors** -- names frozen; `inputSchema` + descriptions free-float  
3. **Agent handbook body** (`robot-docs` / `--robot-help`) -- only `"agent handbook"` substring  
4. **Codemode tool catalog + host adapter JSON** -- name lists; full schemas/adapters not frozen  
5. **Chain expand machine JSON** -- graph structure smoke only; no frozen nodes/edges page  

(Search hit payloads remain the highest agent-visible gap overall but are **Pass 3 F1/F2/F3** -- extend there; not re-spec'd below.)

---

## 2. Gap matrix

Columns: **Surface** | **Current assertion style** | **Recommended golden pattern** | **Priority** | **Effort**

Effort: **S** ≤1 day, **M** 1–3 days, **L** multi-day / multi-crate. Priority: **P0–P3** (product risk × complexity × weakness of current asserts). Pattern names follow skill: exact / scrubbed / structural / fuzzy / N/A.

| # | Surface | Current assertion style | Recommended golden pattern | Pri | Effort | Notes |
|---|---------|-------------------------|----------------------------|-----|--------|-------|
| G1 | **Lang extraction full dumps** (`ExtractionResult` per `fixtures/extract/*`) | Hand presence/forbid/call/pattern tuples via `assert_language_conformance` -- not dump files | **Canonicalized JSON golden** per lang: sort symbols/imports/calls; optionally drop or normalize `byte_*` if volatile; keep kinds + names + lines | **P1** | **M→L** | 13 fixtures; `ExtractionResult` already `Serialize`. Tuples stay as identity oracles. |
| G2 | **MCP `tools/list` full descriptors** | Exact **name vector** only (`protocol.rs`); tool results shape/kind | **Scrubbed exact JSON** of full `result.tools[]` (name, description, inputSchema) | **P1** | **S** | Descriptions embed compact-contract prose -- freeze is the point. |
| G3 | **MCP tool call success bodies** (compact search hits, code_read nodes) | Shape: 5-tuple hits, fixed ids on temp files; error paths sparse exact | **Scrubbed exact** for one fixed mini-repo + query per tool; keep budget/error hand asserts | **P2** | **M** | Temp root → relative; session path ids need scrub or fixed layout. |
| G4 | **robot-docs / `--robot-help` handbook body** | Substring `"agent handbook"` + envelope field smoke | **Exact markdown golden** of `robot_guide_markdown()` (~2.3 KB); JSON envelope = structural + body equality | **P1** | **S** | Static `&str` in `agent.rs` -- zero path scrub. Pass 3 F6. |
| G5 | **CLI teaching / usage human + machine messages** | Shell greps (R-002/R-003, not in cargo test); envelope golden blanks `message` | **Scrubbed exact** path-free usage JSON + human teaching strings; wire into cargo test | **P2** | **S–M** | Pass 3 F4. |
| G6 | **CLI root `--help` / subcommand help** | Sparse: sibling binaries substring; capabilities `--help` must not list search-tuning flags | **Exact or lightly scrubbed** clap help text for root + 2–3 key subcommands **or** structural snapshot of section headers only | **P3** | **M** | High clap churn (vol 4); prefer section anchors over full dump if help reformats often. |
| G7 | **CLI success search hit payloads** (native/agent/agent-capsule/compact/github/gitlab) | Shape keys + bounds; plugins synthetic exact; no live dump golden | **Scrubbed exact** per format (`--no-embed`, fixed query/limit); extend shapes for native/github/gitlab | **P1** | **M** | **See Pass 3 F1–F3** -- do not re-open design here. |
| G8 | **Index `status` machine values** | Sorted **key** shape only; no value freeze | **Scrubbed exact** on sample index: freeze counts/flags; scrub `root`, `index_path`; leave embed cache counters out or scrub to `<n>` | **P2** | **S** | Live sample index after deterministic index. Cache hit/miss fields are volatile. |
| G9 | **Doctor triage machine payload** | Key shape + unhealthy path (`healthy:false`, `missing_root` kind, non-empty issues) | **Scrubbed exact** for 1–2 **issue templates** (missing_root, blocked index); healthy path optional structural | **P2** | **S** | Absolute `root` must scrub. Do not freeze `tty`. |
| G10 | **Chain expand JSON** (`ChainResponse` via CLI `--json chain` or core `expand_chain`) | Non-empty nodes/edges; symbol presence; truncation tests | **Canonicalized exact**: sort nodes/edges by (file, line, symbol, label); scrub scores if float-noisy else keep `--no-embed` fixed | **P1** | **M** | High graph regression value; sample corpus has known call graph. |
| G11 | **Graph query search hits** (defs/callers/imports) | Oracle sets + case-fold parity | Keep **structural oracle**; optional sorted symbol-list golden for one fixture file | **P3** | **S** | Prefer oracle over full hit dump unless format fields drift. |
| G12 | **Cascade planner “reports”** | Behavioral: multi-channel fusion, no leak outside lexical survivors, empty stages | **N/A dump golden** -- keep behavioral. Optional: freeze **contributor sets** for one query as small structural golden | **P3** | **S** | No public cascade-report JSON API; `pipeline_parts::Report` is **timing** (anti-golden). |
| G13 | **Ranking full ordered lists / eval reports** | `must_include` + `max_rank` oracle; CLI eval uses **ephemeral** temp gold | Keep ranking oracle; optional **checked-in eval gold fixture** for sample (query→must_hit) separate from historical MRR | **P2** | **M** | Full score-order dumps = vol 4 with embed. Prefer no-embed + max_rank. |
| G14 | **LSP search / executeCommand payloads** | Smoke: non-empty hits, types of signal/score/contributors; definition range exact on synthetic buffer | **Scrubbed JSON golden** for `asgrep.search` on sample_backend fixed query (hit keys + excerpt scrub paths) | **P2** | **M** | Complements CLI hit goldens (HitKey parity exists). |
| G15 | **LSP initialize / capabilities transcript** | Implicit via backend; serverInfo version not frozen as file | **Scrubbed exact** initialize result (version → `<version>`) | **P3** | **S** | Low churn once frozen. |
| G16 | **Codemode `tool_catalog` full defs** | Name **presence** of ~11 tools; `catalog_describe("search")` has `query` property | **Exact/scrubbed JSON** of all `ToolDef` (name, kind, input_schema, descriptions) | **P1** | **S–M** | Progressive discovery contract for hosts. |
| G17 | **Codemode host adapters** (Anthropic / OpenAI / Cloudflare) | Sparse field asserts (`code_execution`, PTC type, progressiveDiscovery keys) | **Exact JSON goldens** per adapter full list | **P2** | **S** | Synthetic, deterministic, no paths. |
| G18 | **Codemode batch / serve NDJSON** | Mode/count/wall-time behavior | Keep structural; **not** wall-time goldens | **P3** | **—** | Anti-golden for timings. |
| G19 | **Pi extension tool schemas + argv matrices** | Hand `deepEqual` names/limits/arg vectors in TS | Optional **JSON fixture** for tool parameter schemas; argv matrices can stay hand exact | **P3** | **S** | Parallel to MCP; lower priority if Rust MCP/codemode freeze first. |
| G20 | **Pi `release-contract.json`** | Already strict structural freeze | **Keep** -- not a gap | — | — | Excellent sibling pattern. |
| G21 | **Benchmark baselines / bakeoff / speed** | Published UNREPRODUCIBLE ledger (`baselines.md`) | **Do not** CI-golden numeric rows without harness | — | — | Agents.md honesty. |
| G22 | **Plugins capsule/compact/github/gitlab** (synthetic) | Dense hand exact on fixed `SearchResponse` | File goldens (Pass 3 F3) | **P2** | **S** | Cross-ref Pass 3. |

---

## 3. Explicit NON-goals / anti-golden surfaces

Do **not** freeze these as exact CI goldens:

| Surface | Why not |
|---------|---------|
| **`benchmarks/results/baselines.md` (and bakeoff/h2h/losses/speed)** | Historical UNREPRODUCIBLE metrics; no harness/gold corpus in tree. Honesty ledger only. Never gate CI on MRR/Recall/nDCG without a reproducible harness + gold fixture. |
| **Embedding vectors / model weight dumps** | Non-deterministic across backends/versions; prefer dim/backend unit asserts + IVF roundtrip. |
| **ANN recall % / fuzzy retrieval thresholds as exact dumps** | Fuzzy gates OK; exact golden of recall numbers is brittle and dishonest if unreproducible. |
| **Wall-clock latency, bench `cv_pct`, codemode batch wall_ms, `pipeline_parts` median_ms** | Timing noise; budget gates (`all_under_budget`) stay behavioral. |
| **Cascade internal stage sets as huge dumps** | No agent-facing cascade report; current multi-stage leak tests are the right pattern. |
| **Metamorphic / determinism self-baselines** | First-run in-memory baselines -- excellent flakiness guards, not committed goldens. |
| **Implementation `Debug` of private structs / store internals** | Anti-pattern (skill); freeze public machine JSON / Serialize public types only. |
| **Doctor `tty` field / embed cache hit-miss counters** | Session- or run-dependent. |
| **SIGPIPE / broken-pipe absence oracles** | Correct non-golden (R-001). |
| **Peer HitKey parity** (`no_embed_hit_key_parity`) | Cross-surface oracle; do not replace with a single dump. |
| **Blind full clap `--help` if reformatting is frequent** | Prefer section/structural freezes over byte-stable help unless product commits to help stability. |

**Infra dependency (Pass 2):** File goldens for volatile paths should wait for (or land with) `assert_golden` + `Scrubber` in `ast-sgrep-testkit` and `ASGREP_UPDATE_GOLDENS`. Static content (handbook, MCP schemas, codemode catalog) can use `include_str!` + `assert_eq!` without waiting.

---

## 4. Ranked program of work (phased, max 8 items)

Aggregated across Pass 3 promotions and Pass 4 complex gaps. **Max 8.** Order is recommended implementation sequence, not raw priority alone.

| Phase | # | Item | Depends on | Closes gaps |
|-------|---|------|------------|-------------|
| **A -- quick wins** | **1** | Freeze **handbook markdown** body (+ robot-docs JSON body equality) | None | G4 / Pass 3 F6 |
| **A** | **2** | Freeze **MCP `tools/list` full tools array** (schemas + descriptions) | None | G2 |
| **A** | **3** | Freeze **codemode `tool_catalog` JSON** + one adapter (Anthropic) full dump; optional OpenAI/CF in same PR | None | G16, part G17 |
| **B -- medium** | **4** | **CLI search hit dump goldens** (`--no-embed`, agent / agent-capsule / compact) + complete format **shapes** for native/github/gitlab | Pass 2 scrub preferred; Pass 3 F1/F2 | G7 |
| **B** | **5** | **Teaching / usage message goldens** into cargo test (path-free); pretty envelopes optional | Pass 3 F4/F5 | G5 |
| **B** | **6** | **Chain expand scrubbed golden** on sample (`process_request` or known symbol), sorted nodes/edges | Sample index harness; path scrub | G10 |
| **C -- large** | **7** | **Extraction full dumps** for all 13 langs (canonical sort); keep presence tuples as secondary gate | Serialize path + review process | G1 |
| **C** | **8** | **LSP search payload golden** on `sample_backend` + optional MCP success-body golden for one search tool | Path scrub; align hit field names with CLI | G14, G3 |

**Deferred beyond the 8:** G6 full `--help` dumps, G8/G9 value-level status/doctor (partially covered by shapes -- only if hit/chain goldens land first), G11 graph hit dumps, G12 cascade dumps, G13 full ranking order, G15 LSP initialize, G18 batch timings, G19 Pi schema files, G21 baselines.

**Plugins capsule file migration (Pass 3 F3)** can piggyback on item **4** or land as a half-day attach to **A** if synthetic fixtures need zero scrub.

---

## 5. Top 5 gaps -- sketches (not implemented)

### Gap 1 -- Language extraction full dumps

| | |
|--|--|
| **Why** | `extraction_goldens.rs` only asserts **presence** of selected symbols/imports/calls and **forbid** terms. Extra symbols, wrong kinds, missing secondary defs, and span drift can pass. `ExtractionResult` is fully `Serialize`. |
| **Evidence** | `crates/ast-sgrep-lang/tests/extraction_goldens.rs` + `crates/ast-sgrep-testkit/src/lang.rs::assert_language_conformance`; 13 inputs under `crates/ast-sgrep-lang/tests/fixtures/extract/`; no expected JSON dumps. |
| **Pattern** | Canonicalized exact JSON golden per language. |
| **Volatility** | **3/5** -- extractor changes intentionally; byte offsets may move on whitespace tweaks in fixtures (fixtures are frozen inputs). |
| **Scrubbing** | Prefer: sort arrays by (name/kind/line) or (caller,callee,line); optionally omit `byte_start`/`byte_end` **or** freeze them if fixtures are never reformatted. No paths/UUIDs. Pretty 2-space JSON. |
| **Sample test sketch** | |

```rust
// crates/ast-sgrep-lang/tests/extraction_dump_goldens.rs  (sketch only)
#[test]
fn rust_extraction_matches_golden() {
    let src = include_str!("fixtures/extract/rust.rs");
    let result = ast_sgrep_testkit::parse(Language::Rust, src);
    let mut v = serde_json::to_value(&result).unwrap();
    canonicalize_extraction(&mut v); // sort lists; drop bytes if policy says so
    // Future: ast_sgrep_testkit::assert_golden_json("extract/rust", &v);
    // Interim: assert_eq!(v, serde_json::from_str(include_str!("golden/extract/rust.json")).unwrap());
}

// Keep existing assert_language_conformance as identity gate for "must find GoldenWidget".
```

**Generate command (sketch):** unit test with `ASGREP_UPDATE_GOLDENS=1` writing `tests/golden/extract/<lang>.json` or crate-local `tests/golden/`.

---

### Gap 2 -- MCP `tools/list` full tool descriptors

| | |
|--|--|
| **Why** | Hosts cache `inputSchema` and long descriptions (compact contract prose). Today only the ordered **name list** is exact; property renames, required-field drops, and description contract edits ship silently. |
| **Evidence** | `crates/ast-sgrep-mcp/tests/protocol.rs::tools_list_exposes_search_and_index_tools` (names only); schemas built in `crates/ast-sgrep-mcp/src/lib.rs` `handle_tools_list`. |
| **Pattern** | Scrubbed exact JSON of entire `result` (or `result.tools`). |
| **Volatility** | **3/5** -- intentional API surface churn. |
| **Scrubbing** | `serverInfo.version` if present elsewhere; tools/list itself is static. Normalize JSON key order via `serde_json` Value compare (already insertion-order stable if built the same way). |
| **Sample command / test sketch** | |

```bash
# Manual capture (stdio MCP):
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
              '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | asgrep-mcp | tail -n1 | jq '.result.tools' > tests/golden/mcp/tools_list.json
```

```rust
#[test]
fn tools_list_matches_golden() {
    let r = rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let tools = &r["result"]["tools"];
    // assert_golden_json("mcp/tools_list", tools);
    // Keep name-vector assert as cheap fail-fast optional dual check.
}
```

---

### Gap 3 -- Agent handbook body (`robot-docs` / `--robot-help`)

| | |
|--|--|
| **Why** | Long agent-facing markdown (~2.3 KB) is the documented onboarding path; only substring `"agent handbook"` is checked. Section deletions and wrong triad commands would pass. |
| **Evidence** | `robot_guide_markdown()` in `crates/ast-sgrep-cli/src/agent.rs`; tests at `machine_contracts.rs` robot-help / robot-docs block (~348–367). |
| **Pattern** | Exact markdown golden; JSON envelope may remain structural with `body` equality. |
| **Volatility** | **3/5** -- intentional doc updates; should fail CI when handbook changes (desired). |
| **Scrubbing** | None if body stays version-free (current text has no package version). If version strings appear later, scrub to `<version>`. |
| **Sample command / test sketch** | |

```bash
# Capture (after build):
asgrep robot-docs guide > tests/golden/cli/robot_docs_guide.md
asgrep --json robot-docs guide | jq -r .body | diff -u tests/golden/cli/robot_docs_guide.md -
```

```rust
#[test]
fn robot_docs_body_matches_golden() {
    let out = run(&bin, &["robot-docs", "guide"]);
    assert_eq!(out.status.code(), Some(0));
    let body = String::from_utf8_lossy(&out.stdout);
    // assert_golden("cli/robot_docs_guide.md", body.trim_end());
    let json = run(&bin, &["robot-docs", "--json"]);
    let v: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["body"].as_str().unwrap(), body.trim_end());
}
```

---

### Gap 4 -- Codemode tool catalog + host adapter dumps

| | |
|--|--|
| **Why** | Progressive discovery (`catalog_search` / `catalog_describe`) and Anthropic/OpenAI/CF adapters are the multi-host contract. Tests only require name **presence** and a few shape samples (`query` property exists; first tool is `code_execution`). Full schema drift is invisible. |
| **Evidence** | `crates/ast-sgrep-codemode/tests/catalog.rs` (57 lines); catalog built in `crates/ast-sgrep-codemode/src/catalog.rs`; adapters under `src/adapters/`. |
| **Pattern** | Exact JSON goldens (deterministic, no filesystem). |
| **Volatility** | **3/5**. |
| **Scrubbing** | None for pure schema JSON. Pretty-print for review. |
| **Sample test sketch** | |

```rust
#[test]
fn tool_catalog_matches_golden() {
    let catalog: Vec<Value> = tool_catalog()
        .into_iter()
        .map(|t| json!({
            "name": t.name,
            "kind": format!("{:?}", t.kind), // or stable string field if exists
            "input_schema": t.input_schema,
            // include description fields the public API exposes
        }))
        .collect();
    // assert_golden_json("codemode/tool_catalog", &catalog);
}

#[test]
fn anthropic_tools_match_golden() {
    // assert_golden_json("codemode/adapters/anthropic_tools", &anthropic_tools());
}
```

---

### Gap 5 -- Chain expand machine JSON

| | |
|--|--|
| **Why** | `ChainResponse` (seeds/nodes/edges/depths/labels) is a complex multi-graph dump used by agents via `asgrep chain --json`. Coverage is non-empty structure + symbol touch oracles (`graph_oracle.rs`), plus truncation in `downstream_correctness` -- not a frozen page. |
| **Evidence** | `crates/ast-sgrep-core/src/chain.rs` (`ChainResponse`); CLI `search_cmd::run_chain`; `graph_oracle.rs` chain loop asserts presence only. |
| **Pattern** | Canonicalized scrubbed exact JSON (core unit or CLI machine). |
| **Volatility** | **3–4/5** -- graph extraction changes move edges; scores are `f64` (stabilize with fixed config + no embed). |
| **Scrubbing** | Paths: sample-root-relative; sort `nodes` by `(file, line_start, symbol)`; sort `edges` by `(from_file, to_file, label, depth)`; round or keep scores at fixed precision if needed. |
| **Sample command / test sketch** | |

```bash
# After indexing tests/fixtures/sample:
asgrep --json --no-embed chain "process_request" "$SAMPLE_ROOT" \
  | jq 'del(./*)|. + {query,seeds,nodes,edges,max_depth,node_count,edge_count}' \
  > /tmp/chain.actual.json
# Then canonicalize paths/sort in testkit before compare.
```

```rust
#[test]
fn chain_process_request_matches_golden() {
    let indexed = index_sample(IndexOptions { embed_semantic: false, ..Default::default() });
    let store = /* open store */;
    let r = expand_chain(&store, "process_request", &ChainConfig {
        top_n: 1, max_depth: 2, limit: 32, ..Default::default()
    }).unwrap();
    let v = canonicalize_chain(serde_json::to_value(&r).unwrap(), indexed.root());
    // assert_golden_json("core/chain_process_request", &v);
}
```

---

## 6. Cross-reference: Pass 3 items (extend, do not duplicate)

| Pass 3 ID | Surface | Pass 4 matrix |
|-----------|---------|---------------|
| F1 | CLI success search hit payloads | G7 |
| F2 | Format matrix shapes native/github/gitlab | G7 |
| F3 | Plugins capsule_format → files | G22 |
| F4 | Teaching / usage messages | G5 |
| F5 | Pretty envelopes + compare UX | infra (Pass 2) enabler |
| F6 | robot-docs body | G4 (detailed here as Top-5 #3) |

Pass 4 **adds** G1 (extraction dumps), G2/G3 (MCP), G10 (chain), G14 (LSP), G16/G17 (codemode), G8/G9 (status/doctor values), and explicit anti-golden ledger for benchmarks/cascade timings.

---

## 7. Core surfaces note (status, doctor, cascade, ranking)

| Surface | Gap depth | Recommendation |
|---------|-----------|----------------|
| **status JSON** | Keys frozen; values not | After sample index, scrubbed value golden (G8) is cheap; scrub paths + cache counters. Human `print_status` text is low priority. |
| **doctor JSON** | Shape + unhealthy kinds partially asserted | Template goldens for issue kinds (G9); healthy path less valuable than handbook/MCP. |
| **cascade planner** | Behavioral only -- correct | No dump golden for planner stages (G12 NON-goal). |
| **ranking reports** | Oracle constraints, not full lists | Keep `cases.json` must_include; optional eval gold fixture for sample is honesty-positive without citing UNREPRODUCIBLE baselines (G13). |
| **pipeline_parts / sub1ms Report** | Timing Serialize struct | Anti-golden (values); optional freeze of **part name list** `CORE_PARTS` only if churn matters. |

---

## 8. Method notes

- Read Pass 1 inventory (42 rows), Pass 2 infra design, Pass 3 CLI machine scorecard F1–F6.
- Inspected: `extraction_goldens.rs`, `testkit/lang.rs`, `ExtractionResult` Serialize fields; `machine_contracts` robot-docs/doctor/status; `agent.rs` handbook; agent_surface R-002/R-003; `mcp/tests/protocol.rs` + tools/list builder; `lsp/tests/lsp.rs` smoke; `codemode/tests/catalog.rs`; `cascade_planner.rs`; `graph_oracle` chain loop; `chain.rs` / `search_cmd::run_chain`; `pipeline_parts::Report`; `benchmarks/results/baselines.md` UNREPRODUCIBLE banner; Pi `tools.test.ts` / `release-contract.json`.
- No product or test code changed. No beads. No commit. No WIP touched (metamorphic, mcp uncommitted, testkit mock-free).

---

## 9. Deliverable checklist

| Required | Section |
|----------|---------|
| Gap matrix (surface × style × pattern × priority × effort) | §2 |
| NON-goals / anti-golden | §3 |
| Ranked phased program (A/B/C, max 8) | §4 |
| Top 5: sketch + scrub + volatility | §5 |
| Summary top 5 | §1 |

---

*End of Pass 4.*
