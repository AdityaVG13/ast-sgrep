# Pass 5 — Scrubbing & Non-Determinism Audit

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization`  
**Date:** 2026-08-07  
**Skill:** `testing-golden-artifacts` (+ `references/SCRUBBERS.md`)  
**Prior:** PASS1–4 under `tests/artifacts/golden-audit/`  
**Scope:** non-determinism sources that would break goldens; existing scrub inventory; proposed registry + per-surface recipes. **No scrubber implementation, no beads, no commit.**

---

## Executive summary

| Dimension | Grade | Notes |
|-----------|-------|-------|
| **Shared Scrubber registry** | **0/10** | Absent from `ast-sgrep-testkit` (and elsewhere). |
| **Ad-hoc scrub completeness** | **4/10** | Only `version` / `command` / `error.message` field assigns in `machine_contracts.rs`. |
| **Product-side stability (no-embed)** | **8/10** | Ranking tie-breakers, relative index paths, sorted counts, compact path ids (FNV), `determinism_loop` 50× JSON identity. |
| **Risk if promoting live dump goldens without scrub** | **High** | Absolute roots, doctor `tty`, embed scores, status cache counters, bench wall times, usage messages that embed paths. |
| **Over-scrub risk (today)** | **Medium** | Blanking **all** `error.message` on usage/operational envelopes hides teaching regressions (Pass 3 F4). |

**Maturity call:** product is **mostly deterministic under pinned config** (`--no-embed`, fixed limit/query/sample root); **test infrastructure has no scrub pipeline**. New file goldens for search hits, doctor, status, MCP tool bodies, and chain **must land with** field/path scrub presets (Pass 2 P0) or they will flake across machines and CI.

---

## 1. Threat model

Sources × where they appear × severity if unscrubbed in a true golden × recommended scrub/canonicalize.

| # | Source | Appears where | Severity if unscrubbed | Recommended scrub / canonicalize |
|---|--------|---------------|------------------------|----------------------------------|
| T1 | **Package version** (`env!("CARGO_PKG_VERSION")`) | `capabilities` JSON `version`; `version --json`; agent format `version`; MCP `serverInfo.version`; plugins provider block | **Critical** (every release) | Field scrub → `"<version>"` (already done for capabilities/version goldens). **Do not** scrub `schema_version` / `machine_schema_version` (`"1.0.0"`). |
| T2 | **Absolute paths** (`/Users/…`, `/tmp/…`, Windows `C:\…`) | Doctor `root`, `index_path`, issue `message`, `suggested_commands`; status `root`/`index_path`; CLI args reflected in messages; MCP compact `p` map values when root is temp | **Critical** (host + CI) | Prefer **root-relative** paths in product (hits already are). For absolute fields: replace workspace/temp roots with `<root>` / `<index>`; regex scrub `/Users/[^/]+/` → `/HOME/`, `/tmp/…` → `/TMP/`, `C:\\Users\\…` → `/HOME/`. |
| T3 | **TempDir path strings** | Any test that freezes CLI/MCP stdout against live `tempfile` corpus | **Critical** | Same as T2; or fix layout under sample fixture and never freeze temp absolute paths. |
| T4 | **Timestamps / Unix epoch** | Bench history JSON (`duration_since(UNIX_EPOCH)`), potential future logs | **High** for bench history goldens; **N/A** for current machine goldens | Scrub ISO-8601 and `\b1[6-9]\d{8}\b` only if a golden freezes that surface; **do not golden bench history**. |
| T5 | **Durations / wall times** | `bench` avg_ms, index_ms, suite timings; codemode batch wall; any `elapsed` | **Critical** if frozen | **Anti-golden:** assert shapes/flags only (`suite_ok`, `compared:false`). Never scrub-and-freeze timings. |
| T6 | **UUIDs / PIDs / memory addresses** | Not observed in machine search envelopes today | **Low** (latent) | Standard registry rules ready; apply if logs/diagnostics enter goldens. |
| T7 | **JSON map key order** | Machine stdout via `serde_json` `Map` (insertion order); capabilities advertises stable ordering | **Low–Medium** | Prefer **Value equality** after parse (order-insensitive for object keys) over raw string golden. If string golden: pretty-print with **sorted keys** or rely on fixed struct field order. Shapes already **sort keys** before compare. |
| T8 | **HashMap iteration order** | Chain seed selection uses `HashMap` (`best_per_file`); count-only path rebuilds from HashMap then **sorts by path** | **Medium** for chain dumps if nodes/edges append order follows HashMap | **Canonicalize before compare:** sort `nodes` by `(file, line_start, symbol)`; sort `edges` by `(from_file, to_file, label, depth)`. Prefer sorted golden over hoping HashMap is stable. |
| T9 | **Floating scores / margins** (`f64`) | Every `SearchHit.score` / `margin`; chain node scores; fusion RRF | **Low** under `--no-embed` + fixed index (byte-stable in `determinism_loop`); **High** with embed/ANN/rerank | Pin `--no-embed`; keep scores. If embed goldens ever needed: round to N decimals or scrub to `<score>` **only for embed-driven fields** -- prefer not goldenizing embed order. |
| T10 | **Embed / ANN non-determinism** | Semantic hits, IVF probes, cache hit/miss counters | **Critical** for full-rank dumps | Exclude from exact goldens; use ranking oracle `max_rank` / structural asserts. Status: scrub or omit `embed_cache_hits`/`misses`. |
| T11 | **Parallel ranking ties** | Multi-hit same score | **Low** product-side | Product `cmp_ranked_hits` secondary keys: `file` then `line_start` (deterministic). Document: goldens assume this key; do not scrub order. Regression if tie-break removed. |
| T12 | **Thread counts / rayon / OS** | Not in machine JSON payloads observed | **Low** | N/A for current surfaces. |
| T13 | **Platform line endings / path separators** | Help text, handbook, any Windows path in messages | **Medium** for text goldens | `canonicalize_text`: `\r\n`→`\n`, trim trailing WS; path scrub `\`→`/` after root relative. `NO_COLOR=1` already on CLI tests. |
| T14 | **TTY / environment** | Doctor `tty: stdout().is_terminal()`; color (mitigated); `CI` env | **High** if doctor values goldened | **Drop or scrub `tty`** always (`true`/`false` both valid). Do not freeze env-dependent flags unless test env is pinned. |
| T15 | **Schema / binary versions** | `schema_version` `"1.0.0"`; clap-derived capabilities catalog | **Intentional break** | **Never scrub** schema constants. Capabilities full equality is a **churny-but-correct** contract golden (version field only scrubbed). |
| T16 | **Session-local path ids** | Compact/MCP `p` map + hit ids (`fnv1a64` → base36) | **Low** if paths stable | Deterministic given same path set; collision suffix is deterministic too (`BTreeMap`). Scrub only if absolute path values appear in `p`. |
| T17 | **Optional / skip_serializing fields** | `symbol`/`caller`/`callee`/`language` skipped when `None` | **Medium** for string goldens | Prefer JSON Value compare; or canonicalize by re-serializing through the same structs. |
| T18 | **Usage / operational messages with paths** | Failure envelopes; doctor issues; suggested_commands | **High** | Path-free usage cases: freeze **full message**. Pathful: scrub paths then freeze, or keep `<message>` + separate teaching string golden for path-free cases only. |
| T19 | **Pretty vs compact JSON** | Default machine pretty; compact format and some codemode paths compact | **Medium** if string-diff goldens | Compare parsed `Value`, or pin pretty 2-space + trailing newline in freeze path. |
| T20 | **CARGO_BIN / target profile paths** | Test harness only (bin path) | **N/A** for product goldens | Never put `target/debug/...` into golden content. |

### Severity legend

- **Critical:** different machine/CI run fails without scrub.  
- **High:** fails across env (TTY, temp roots) or release train.  
- **Medium:** fails under some formats/platforms or HashMap-order dumps.  
- **Low:** theoretical or already stabilized by product.

---

## 2. Inventory of EXISTING scrubbers / stabilizers

### 2.1 Explicit scrub (field overwrite) — only site

| Location | What is scrubbed | How | Completeness |
|----------|------------------|-----|--------------|
| [`crates/ast-sgrep-cli/tests/machine_contracts.rs`](crates/ast-sgrep-cli/tests/machine_contracts.rs) `capabilities_and_version_match_goldens` | `capabilities["version"]`, `version["version"]` | Assign `"<version>"` then `assert_eq!` vs fixture | **Good for version only** |
| Same file operational failure loop | `command` → `"<command>"`, `error.message` → `"<message>"` | Assign before compare to `envelopes.json` | **Over-scrubs messages** (shape only) |
| Same file usage envelope | `error.message` → `"<message>"` | Same | **Over-scrubs teaching text** |

Fixtures with placeholders:  
- [`crates/ast-sgrep-cli/tests/fixtures/envelopes.json`](crates/ast-sgrep-cli/tests/fixtures/envelopes.json) — `<command>`, `<message>`, `<version>`  
- [`crates/ast-sgrep-cli/tests/fixtures/capabilities.json`](crates/ast-sgrep-cli/tests/fixtures/capabilities.json) — `"version": "<version>"` (rest exact)

**Grade: ad-hoc scrub = D (works for 3 fields; not reusable; no paths/times/UUIDs).**

### 2.2 Structural “soft scrub” (not string masking)

| Location | Mechanism | Effect |
|----------|-----------|--------|
| `assert_shape` in `machine_contracts.rs` | Sort object keys, compare to frozen key arrays in `machine_shapes.json` | Values (incl. paths, caches) ignored |
| Ranking oracle `tests/fixtures/ranking/cases.json` | `must_include` + `max_rank` | Order beyond bound not frozen |
| Extraction goldens `extraction_goldens.rs` | Presence tuples via `assert_language_conformance` | Full dump order/spans not frozen |
| MCP `tools/list` | Exact **name vector** only | Schemas/descriptions free |
| Plugins `capsule_format.rs` | Synthetic relative paths + fixed float literals | No path scrub needed |

### 2.3 Product / harness stabilizers (not scrubbers, but reduce need)

| Stabilizer | Where | Golden impact |
|------------|-------|---------------|
| `NO_COLOR=1` on CLI spawn | `machine_contracts::run` | Stable stdout/stderr |
| Sample root `canonicalize()` | `testkit/src/fixture.rs` | Stable absolute sample path within a host (still host-specific if frozen raw) |
| Index paths **root-relative** UTF-8 | `core/src/index.rs` strip_prefix | Hit `file` fields portable across machines for same relative tree |
| Ranking tie-break `file` then `line_start` | `core/src/search/mod.rs` `cmp_ranked_hits` | Stable hit order for equal scores |
| Count-only: HashMap → **sort by path** | `finish_response_checked` | Stable `counts` |
| Compact path ids: FNV1a + base36 + `BTreeMap` | `plugins` compact formatter | Session-stable ids given same path set |
| `determinism_loop` 50× `serde_json::to_string` identity | `core/tests/determinism_loop.rs` | Proves no-embed search JSON is process-stable |
| Capabilities claim | `agent_contract.deterministic` text | Documents intent (serde_json key order + NO_COLOR) |

### 2.4 Missing (confirmed)

| Expected by skill / Pass 2 | Status |
|----------------------------|--------|
| `Scrubber` type / registry in testkit | **Absent** |
| `assert_golden` / `UPDATE_GOLDENS` | **Absent** |
| Regex path / UUID / timestamp scrub | **Absent** |
| `canonicalize_text` (`\r\n`, trailing space) in golden path | **Absent** |
| `scrub_json_fields` JSON-pointer helper | **Absent** |
| insta filters / snapshots | **Absent** (no insta) |

**Overall scrubbing maturity: ~2/10 infrastructure, ~6/10 product determinism under pinned flags.**

---

## 3. Proposed standard Scrubber registry (project-specific)

Design target (Pass 2): `crates/ast-sgrep-testkit/src/golden/scrub.rs` — **proposal only; not implemented this pass.**

### 3.1 Text-level rules (ordered; first match wins where overlapping)

Apply after optional `canonicalize_text` (CRLF→LF, trailing whitespace strip). Prefer **field-level** scrub for JSON; use regex for free-form strings (messages, help).

| ID | Pattern (regex / rule) | Replacement | When |
|----|------------------------|-------------|------|
| R1 | Field pointer `/version` (package) | `"<version>"` | capabilities, version cmd, agent provider |
| R2 | Field `/serverInfo/version` | `"<version>"` | MCP initialize |
| R3 | **Never** scrub `/schema_version`, `/machine_schema_version` | — | Contract constants |
| R4 | Absolute Unix home: `/Users/[^/\s"']+/` | `/HOME/` | Messages, roots |
| R5 | Absolute Linux home: `/home/[^/\s"']+/` | `/HOME/` | CI runners |
| R6 | Temp: `/var/folders/[^/\s"']+/` (macOS), `/tmp/[^/\s"']*` | `/TMP/` | tempfile |
| R7 | Windows: `[A-Za-z]:\\Users\\[^\\]+\\` and `\\`→`/` after | `/HOME/` | Cross-platform |
| R8 | Workspace root (inject at runtime): exact prefix of sample/temp root | `<root>/` or empty relative | CLI/MCP live dumps |
| R9 | Index path absolute | `<index>` | status/doctor |
| R10 | UUID (any version shape) | `[UUID]` | Future logs |
| R11 | ISO-8601 timestamps | `[TIMESTAMP]` | Bench history only if ever goldened |
| R12 | Duration tokens `\d+(\.\d+)?\s*(ms|us|µs|ns|s)\b` | `[DURATION]` | Free-text only; **not** for score floats in JSON |
| R13 | `pid[=: ]\d+` | `pid=[PID]` | Diagnostics |
| R14 | `0x[0-9a-fA-F]{6,16}` | `[ADDR]` | Debug dumps only |
| R15 | Doctor field `tty` | delete or `"<tty>"` | Always for doctor value goldens |
| R16 | `embed_cache_hits` / `embed_cache_misses` | `"<n>"` or omit | Status value goldens |
| R17 | Optional: float fields under embed-only goldens | round 6 dp or `"<score>"` | Prefer avoid |

**Anti-rule:** Do **not** globally replace all `\d+\.\d+` — that destroys line numbers in strings and legitimate integer-like scores.

### 3.2 Presets

| Preset | Rules / actions | Migrates from |
|--------|-----------------|---------------|
| `Scrubber::standard()` | R4–R7, R10–R14 + canonicalize_text | Skill catalog |
| `Scrubber::machine_contract()` | R1 + field assigns for optional message; **not** auto-blank all messages | machine_contracts version scrub |
| `Scrubber::search_dump(root)` | R8 relative paths on hit.file / envelope roots; keep scores; pin no-embed assumed | Future F1 goldens |
| `Scrubber::doctor()` | R8, R9, R15; path scrub inside issue messages and suggested_commands | Future G9 |
| `Scrubber::status()` | R8, R9, R16; keep counts/flags | Future G8 |
| `Scrubber::none()` | canonicalize_text only | Handbook, MCP tools/list schemas |

### 3.3 JSON compare policy (canonicalization, not scrub)

1. Parse both sides to `serde_json::Value`.  
2. Optional: deep-sort object keys for string snapshot mode.  
3. For arrays that are sets (chain nodes/edges, contributor lists if unordered): **explicit sort** with documented key.  
4. Prefer Value `PartialEq` over byte string equality for pretty/compact variance.

### 3.4 Unit tests the registry must carry (when implemented)

- Version field scrub leaves `schema_version` intact.  
- `/Users/alice/proj` and `/tmp/foo` → placeholders.  
- Windows path string unit test (even on macOS CI).  
- `standard()` is pure and idempotent (`scrub(scrub(x)) == scrub(x)` for common cases).

---

## 4. Per-surface scrub recipes

### 4.1 CLI search JSON (agent / agent-capsule / compact / native / github / gitlab)

| Item | Recipe |
|------|--------|
| **Pin** | `--json --no-embed --format <fmt> --limit N --index-path <sample.db>`; fixed query (e.g. sample `process_request`); `NO_COLOR=1`. |
| **Paths** | Hit `file` should already be **relative** to index root -- freeze as-is if sample fixture. If any absolute root appears on envelope, R8. Compact `p` values: relative paths only. |
| **Scores** | **Keep** under no-embed (supported by `determinism_loop`). |
| **Version** | Agent envelope may include package version → R1. |
| **Order** | Keep product order (score + tie-break). Do not sort hits in scrub (would hide ranking bugs). |
| **Optional fields** | Value-compare; or re-serialize through same types. |
| **Do not freeze** | Embed-on dumps; bench; wall times. |
| **Compare** | Scrubbed exact JSON Value vs committed golden (Pass 3 F1–F3). |

### 4.2 Extraction dump (if promoted beyond presence tuples)

| Item | Recipe |
|------|--------|
| **Input** | Fixed `include_str!` fixtures under `lang/tests/fixtures/extract/*` -- no paths. |
| **Scrub** | **None** for content. **Canonicalize:** sort `symbols` by `(name, kind, start_byte)`; sort `imports` by `module_path`; sort `calls` by `(caller, callee, line)`. |
| **Spans** | Freeze byte/line spans only if fixtures are immutable (they are); document that reformatting fixtures updates golden. |
| **Platform** | LF-only sources already in repo. |
| **Alternative** | Keep presence-oracle (current) -- lower scrub burden, lower dump coverage. |

### 4.3 MCP `tools/list` (+ initialize)

| Item | Recipe |
|------|--------|
| **tools/list** | Full `result.tools[]` (name, description, inputSchema): **exact / no path scrub**. Static catalog. Sort tools by `name` only if product order is not contract (today order is the contract -- **keep list order**). |
| **initialize** | Scrub `serverInfo.version` → `<version>`; keep `protocolVersion` `"2024-11-05"` exact; keep `serverInfo.name` exact. |
| **Tool call bodies** | Temp root → R8 on `p` map; keep compact tuples; pin limit/query; prefer keyword path without embed for dump golden. Session path ids: deterministic given paths -- freeze relative paths in `p`. |

### 4.4 Handbook (`robot_guide_markdown` / robot-docs)

| Item | Recipe |
|------|--------|
| **Body** | **Exact golden, no scrub** today: static `&str`, no package version, no paths. |
| **JSON envelope** | Structural fields (`command`, `topic`, `format`) + body equality; no path scrub. |
| **If version ever enters prose** | Light scrub R1-style token replace. |
| **Platform** | canonicalize_text for CRLF only if Windows ever writes the file. |

### 4.5 Capabilities

| Item | Recipe |
|------|--------|
| **Existing** | Full equality after `version` → `<version>` -- **keep**. |
| **Do not scrub** | Flag lists, env catalog, `schema_version`, command usage strings (intentional contract). |
| **Churn** | Expected on CLI surface adds; review golden diffs, do not auto-blank new keys. |

### 4.6 Adjacent surfaces (brief)

| Surface | Recipe |
|---------|--------|
| **Doctor triage JSON** | Scrub `root`, `index_path`, pathful messages/commands; drop/scrub `tty`; freeze issue `kind` + template skeleton. |
| **Status JSON** | Scrub paths; scrub or omit cache counters; freeze counts/flags/`semantic_ivf_present`. |
| **Chain expand** | Sort nodes/edges (T8); root-relative files; keep or round scores under no-embed. |
| **LSP search** | Same as CLI hits + scrub any URI `file://` absolute. |
| **Bench / codemode wall** | **No golden** of timings. |

---

## 5. Anti-patterns (over-scrubbing hides real regressions)

| Anti-pattern | Why it hurts | Project-specific note |
|--------------|--------------|----------------------|
| **Blank all `error.message`** | Teaching / did-you-mean / Example lines are agent UX | **Already present** in operational/usage envelope goldens -- Pass 3 F4 |
| **Scrub `schema_version`** | Silent protocol break | Never |
| **Scrub scores under `--no-embed`** | Hides ranking / fusion regressions | Prefer pin no-embed and keep floats |
| **Sort hits by file in scrub** | Hides ranking order bugs | Only sort true sets (chain edges), not ranked lists |
| **Global float regex** | Corrupts line numbers, limits, exit codes in text | Use field-aware JSON scrub |
| **Scrub capabilities flag lists** | Defeats the point of the golden | Version field only |
| **Golden + scrub wall times** | Still flaky or vacuously always `<duration>` | Anti-golden |
| **Freeze doctor `tty`** | Fails headless vs interactive | Drop field |
| **Over-wide path regex** | Eats relative segments like `Users/` in repo trees | Prefer exact root-prefix replace (R8) over naive `/Users/` when possible; document residual risk |
| **Dual ad-hoc scrubs per crate** | Divergent placeholders (`<ver>` vs `<version>`) | Single testkit registry + presets |
| **Update goldens in CI** | Launders flakes | Fail-closed; human review |
| **Presence-only as if dump-safe** | Extraction/MCP look covered but miss schema extras | Pass 4: structural ≠ scrubbed exact |

---

## 6. Aggregated findings for beads (max 4 deep items)

*Do not file this pass. Specs for later `br create`.*

### B1 — P0: Shared Scrubber registry + machine_contract preset (testkit)

| | |
|--|--|
| **Problem** | Only three ad-hoc field assigns; no path/version/TTY/temp scrub reusable across crates. |
| **Evidence** | `machine_contracts.rs` version/message scrub; Pass 2 P0; zero hits for `Scrubber`/`assert_golden` in testkit. |
| **Acceptance** | `Scrubber::standard()` + `machine_contract()` + `search_dump(root)` in testkit; unit tests for version/path/idempotency; machine_contracts migrates version scrub to preset (messages may stay explicit until B3). |
| **Depends** | Pass 2 assert_golden can land with or just after this. |

### B2 — P1: Path + TTY + cache scrub recipes wired for status/doctor/search dumps

| | |
|--|--|
| **Problem** | Promoting live CLI dumps (Pass 3 F1, Pass 4 G7–G9) without root-relative absolute scrub and without dropping `tty`/cache counters will flake on every host. |
| **Evidence** | Doctor emits absolute `root`, pathful messages, `tty`; status exposes `embed_cache_hits/misses` + absolute `root`/`index_path`; sample_root is host-absolute if frozen raw. |
| **Acceptance** | Documented presets `doctor`/`status`/`search_dump`; at least one scrubbed fixture path for doctor missing_root **or** status sample; PROVENANCE notes scrub preset. |
| **Depends** | B1. |

### B3 — P2: Stop over-scrubbing path-free usage messages (teaching goldens)

| | |
|--|--|
| **Problem** | Envelope golden replaces every usage/operational message with `<message>`, so did-you-mean / Example regressions do not fail cargo tests. |
| **Evidence** | `envelopes.json`; operational/usage loops in machine_contracts; agent_surface R-002/R-003 outside cargo. |
| **Acceptance** | Path-free usage cases freeze full `error.message` (or full failure JSON); pathful operational may keep message scrub or use R8. At least one teaching string asserted in `cargo test -p ast-sgrep-cli`. |
| **Depends** | Optional B1 field helper; can be fixture-only. |

### B4 — P1: Chain / set-like array canonicalization helper

| | |
|--|--|
| **Problem** | Chain construction uses `HashMap` for per-file best hits; unsorted node/edge dumps risk order flake even with stable scores. |
| **Evidence** | `chain.rs` `HashMap`; Pass 4 G10 scrub note; contrast with count-only path which already sorts. |
| **Acceptance** | testkit (or test-local) `canonicalize_chain_response` sorts nodes/edges by documented keys; used by any chain golden; unit test that two HashMap insertion orders still match after canonicalize. |
| **Depends** | B1 nice-to-have; can land independently for G10. |

**Not bead-worthy here (keep as policy):** anti-golden for bench timings; keep ranking oracle; keep determinism_loop; do not scrub schema_version.

---

## 7. Cross-links to prior passes

| Prior | Relevance to Pass 5 |
|-------|---------------------|
| **Pass 1** | Inventory: only true scrubbed file goldens are capabilities + envelopes; extraction is presence not dump. |
| **Pass 2** | Infra design: Scrubber in testkit, presets, canonicalize_text -- **still unimplemented**. |
| **Pass 3** | §4 scrubbing map + over-scrub of messages; F1 search dumps need path scrub + no-embed. |
| **Pass 4** | G2–G10 recipes; G18 wall-time anti-golden; handbook zero scrub. |

---

## 8. Evidence trail (what was read)

- Skill: `testing-golden-artifacts/SKILL.md`, `references/SCRUBBERS.md`  
- Prior audits: PASS1–4 under `tests/artifacts/golden-audit/`  
- Scrub sites: `crates/ast-sgrep-cli/tests/machine_contracts.rs`, fixtures `capabilities.json`, `envelopes.json`, `machine_shapes.json`  
- Stabilizers: `core/src/search/mod.rs` (`cmp_ranked_hits`), `core/tests/determinism_loop.rs`, `core/src/index.rs` (relative paths), `plugins` compact FNV path ids, `testkit` fixture/lang  
- Surfaces: `cli/src/agent.rs` (doctor/handbook), `cli/src/machine.rs`, `mcp/tests/protocol.rs`, `lang/tests/extraction_goldens.rs`, `core/src/chain.rs`, `core/src/search/types.rs` (`score: f64`)  
- Confirmed **no** Scrubber / assert_golden / UPDATE_GOLDENS in `ast-sgrep-testkit`

**Not done (by mission):** implement scrubber code, file beads, commit, run full test suite.

---

## 9. Pass 5 scorecard (scrub readiness)

| Surface | Ready for exact golden without new scrub? | Blocking non-determinism |
|---------|-------------------------------------------|---------------------------|
| Capabilities (existing) | **Yes** (version field scrub only) | Package version |
| Failure envelopes (existing) | **Yes** shape-only; **No** for message content | Message over-scrub (policy) |
| Handbook body | **Yes** exact | None today |
| MCP tools/list full descriptors | **Yes** (scrub initialize version only) | Package version on initialize |
| CLI search hit dumps | **No** until path policy + pin no-embed | Abs roots if any; embed scores |
| Doctor / status values | **No** | root, tty, cache counters |
| Extraction full dump | **Canonicalize sort** only | Array order |
| Chain expand | **No** until sort canonicalize | HashMap order, scores |
| Bench timings | **Never** | Wall time |

**Bottom line:** Ship B1 scrub registry before (or with) any new live-dump goldens. Treat product no-embed determinism as a feature to **preserve in tests**, not something to paper over with aggressive float scrubbing.
