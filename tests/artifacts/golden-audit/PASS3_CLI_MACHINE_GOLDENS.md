# Pass 3 — CLI & Machine-Contract Golden Quality

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (audit only; no product/test refactors)  
**Date:** 2026-08-07  
**Skill:** `testing-golden-artifacts`  
**Prior:** [`PASS1_GOLDEN_INVENTORY.md`](PASS1_GOLDEN_INVENTORY.md), [`PASS2_INFRASTRUCTURE_GAPS.md`](PASS2_INFRASTRUCTURE_GAPS.md)  
**Scope:** CLI machine contracts and JSON fixtures only -- `machine_contracts.rs`, `tests/fixtures/{capabilities,envelopes,machine_shapes}.json`, `agent_surface/*`, plugins `capsule_format` where coupled to machine output. No beads, no commit, no large implementation.

---

## 1. Executive summary

CLI machine contracts are the **strongest true golden suite in the repo**, but they freeze **envelopes and key shapes**, not **successful search hit payloads**. That split is deliberate and high quality for failure modes; it leaves the most agent-visible surface (ranked hits across formats) protected only by bounds, non-empty checks, and synthetic plugin asserts.

| Area | Score (1–10) | One-line |
|------|-------------:|----------|
| `capabilities.json` full equality | **9** | True scrubbed golden; excellent contract freeze |
| `envelopes.json` failure/version | **8** | True scrubbed golden; message body over-scrubbed |
| `machine_shapes.json` key sets | **7** | Good structural freeze; incomplete format matrix |
| Success search hit payloads (CLI) | **3** | Shape/bounds only; no value-level dump golden |
| Format matrix (6 advertised formats) | **5** | agent / agent-capsule / compact covered; native / github / gitlab thin |
| Failure messaging / update path | **3** | stock `assert_eq!`; no UPDATE / `.actual` / unified diff |
| `agent_surface` shell scripts | **4** | Useful greps; not goldens; not in `cargo test` |
| `capsule_format` (plugins) | **7** | Dense exact asserts on fixed synthetic response; not file goldens |

**CLI machine golden maturity (this pass):** ~6.5/10 content quality, ~3/10 workflow (update/diff) -- matches Pass 1/2, refined.

---

## 2. Inventory of clusters reviewed

| Cluster | Paths | Pattern today |
|---------|-------|---------------|
| A. Capabilities + version goldens | `fixtures/capabilities.json`; `machine_contracts.rs::capabilities_and_version_match_goldens` | Scrubbed exact `Value` equality |
| B. Failure / version envelopes | `fixtures/envelopes.json`; operational + usage + version tests | Scrubbed exact template equality |
| C. Shape key freezes | `fixtures/machine_shapes.json`; index/status/doctor/agent* tests | Structural sorted key arrays |
| D. Agent search bounds | `agent_search_modes_are_stable_and_bounded`, `format_alone_*` | Hand bounds + shape |
| E. Envelope presence (chain/eval/bench) | `chain_eval_and_bench_*`, bench CV / suite tests | Sparse field asserts |
| F. Discovery / clap parity | `capabilities_lists_all_clap_*`, boolish envs, aliases/typos | Presence + teaching substrings |
| G. Agent-surface scripts | `tests/agent_surface/R-001..003` | Shell greps; optional skip if no binary |
| H. Plugins formatters | `crates/ast-sgrep-plugins/tests/capsule_format.rs` | Hand exact on synthetic `SearchResponse` |
| I. Adjacent CLI (not file goldens) | `cli_smoke.rs`, `no_embed_hit_key_parity.rs` | Structural / peer-oracle |

---

## 3. Quality scorecard (detail)

Scores: **1** = anti-pattern / no protection, **5** = adequate structural, **8+** = true reviewed golden with scrub + stable compare, **10** = skill-complete (update path, diffs, provenance).

### 3.1 Cluster A -- `capabilities.json` + version golden

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **Yes.** Full stdout JSON after scrub equals committed fixture. |
| **Scrubbing** | `version` → `"<version>"` only. Package version is the only intended dynamic field. |
| **Update path** | **None.** Hand-edit fixture or re-capture offline; no `ASGREP_UPDATE_GOLDENS`. |
| **Failure UX** | `assert_eq!(capabilities, fixture(...))` Debug dump of large object -- painful. |
| **Volatility** | **3/5.** Flag lists, env table, `commands[]`, `search_formats` change when CLI surface changes -- correct contract churn, not flake. |
| **Anti-patterns** | Pretty-printed (good). Full equality acts as implicit `deny_unknown_fields` (excellent). Risk: large intentional flag adds force noisy golden edits -- still preferred over silent drift. |
| **Score** | **9/10** |

Evidence: `machine_contracts.rs:91-99`; fixture top-level keys include `search_formats`, `commands` (16), `environment` (22), `output_limits`, `machine_schema`, etc. `version` frozen as `"<version>"`.

### 3.2 Cluster B -- `envelopes.json` (operational / usage / version)

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **Yes** for envelope *skeleton* (schema_version, tool, ok, exit_code, error.kind). |
| **Scrubbing** | operational: `command`→`<command>`, `error.message`→`<message>`; usage: message only; version: `version`→`<version>`. |
| **Update path** | None. |
| **Failure UX** | Same `assert_eq!` Debug. Small fixtures → readable enough. |
| **Volatility** | **2/5** skeleton; message text would be vol 3 if frozen. |
| **Gaps** | (1) **Message content never frozen** -- usage teaching regressions slip unless other tests grep. (2) Usage golden hard-codes `"command":"search"` only -- correct for current tests, not a multi-command usage matrix. (3) Doctor unhealthy is **not** an envelope golden (hand fields). (4) Minified one-line JSON hurts review vs capabilities pretty form. |
| **Score** | **8/10** skeleton; **5/10** for agent-facing error *text* coverage |

Evidence: `envelopes.json` (453 B, single line); `operational_failures_are_json_and_exit_two` (`:262-310`); `bounded_arguments_are_json_usage_errors` (`:312-327`); product writer `src/machine.rs::print_machine_failure` matches frozen fields.

### 3.3 Cluster C -- `machine_shapes.json`

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **Structural only** -- sorted top-level keys, not values. |
| **Scrubbing** | N/A (keys only). |
| **Update path** | None. |
| **Volatility** | **3/5.** New top-level fields fail tests (good); value regressions silent (expected for this pattern). |
| **Gaps** | Covers `index`, `status`, `doctor`, `agent`, `agent-capsule`, `compact`. **Missing shapes for advertised** `search_formats`: `native`, `github`, `gitlab`. No shapes for `watch` / `keyword` / `semantic` / `robot-docs` / codemode success envelopes. Nested hit object keys not frozen. |
| **Score** | **7/10** for covered commands; **4/10** for full format matrix |

Evidence: shapes file; `assert_shape` at `machine_contracts.rs:75-90,101-148,150-199`; capabilities `search_formats`: `native|agent|agent-capsule|compact|github|gitlab`.

### 3.4 Cluster D -- Agent search success (CLI live index)

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **No.** Live sample corpus; asserts shape + bounds only. |
| **What is checked** | Top-level keys; `hits.len() <= limit`; capsule preview ≤121 chars; excerpt lines ≤2; compact `b` budget triple; non-empty-ish presence. |
| **What is not** | Hit `file`/`ref`/`symbol`/`score`/`signal` values; hit key sets; ranking order; full JSON page per format. |
| **Volatility if goldened raw** | Paths under temp/sample root (**need scrub**); with `--no-embed` scores/order largely stable (determinism_loop elsewhere proves JSON can freeze). |
| **Score** | **3/10** as golden quality; **6/10** as smoke/bounds |

Evidence: `agent_search_modes_are_stable_and_bounded` (`:149-199`); `format_alone_implies_json_machine_output` only checks some key exists (`:502-521`).

### 3.5 Cluster E -- chain / eval / bench envelopes

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **No.** Envelope header via `assert_success` + sparse fields. |
| **Notes** | Eval writes **ephemeral** gold JSON in temp (`:242-260`) -- not a checked-in eval golden. Bench asserts `cv_pct`, skips vacuous ast-grep speedup, suite single-envelope -- good behavioral gates, not dump goldens. Latency/cv values not frozen (correct). |
| **Score** | **5/10** |

### 3.6 Cluster F -- Discovery, typos, doctor paths

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **No.** Presence asserts and substring checks. |
| **Strengths** | Clap subcommand list gate; edit-distance suggestions; dry-run non-mutation; doctor `suggested_commands` echo root; format typo usage kind. |
| **Golden fitness** | Teaching strings are high-value scrubbed-exact candidates (overlap with agent_surface). Absolute `root` in doctor is session-temp -- must scrub if frozen. |
| **Score** | **6/10** behavioral; **3/10** golden |

### 3.7 Cluster G -- `agent_surface` scripts

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **No.** Substring `grep` oracles. |
| **Wiring** | **Not** invoked from `cargo test` / workspace CI path found in-tree; manual / audit harness. Skip-if-no-binary soft-exits. |
| **R-001** | Absence oracle (no panic on SIGPIPE) -- correct non-golden. |
| **R-002 / R-003** | Teaching strings + usage envelope fragment -- promotion candidates. |
| **Score** | **4/10** |

### 3.8 Cluster H -- `capsule_format.rs` (plugins; tightly coupled)

| Criterion | Assessment |
|-----------|------------|
| **True golden?** | **Hand exact** on **synthetic** fixed `SearchResponse` -- golden *semantics*, not golden *files*. |
| **Strengths** | Deterministic; no temp paths; freezes refs, previews, contributors, compact IDs/budgets, github incomplete_results, gitlab HEAD meta, suggested_next asgrep-only, minified size invariant. Best **reviewable** candidate to migrate to files. |
| **Gaps** | No `include_str!` fixture; PR review is Rust assert noise; no shared scrub/update; does not cover live CLI wiring of formats (that's CLI's job). |
| **Volatility** | **2/5** with synthetic input. |
| **Score** | **7/10** protection; **4/10** golden workflow |

### 3.9 Cluster I -- Adjacent CLI tests

| Test | Role vs goldens | Score |
|------|-----------------|------:|
| `cli_smoke.rs` | Non-empty hits; signal/margin types; capsule no excerpt; compact single-line; github items meta types | **4** |
| `no_embed_hit_key_parity.rs` | Peer oracle CLI↔core↔LSP HitKeys -- **not** a freeze file; excellent complementary gate | **7** as parity (N/A as file golden) |

---

## 4. Scrubbing map

### 4.1 Scrubbed today (ad-hoc in `machine_contracts.rs`)

| Field / site | Placeholder | Where |
|--------------|-------------|--------|
| `capabilities.version` / `version.version` | `"<version>"` | `capabilities_and_version_match_goldens` |
| operational `command` | `"<command>"` | `operational_failures_are_json_and_exit_two` |
| operational / usage `error.message` | `"<message>"` | operational + usage tests |
| Implicit | `NO_COLOR=1` on spawn | all `run()` helpers -- not a value scrub, stabilizes human vs machine |

No UUID, ISO timestamp, duration, or absolute-path scrubbers exist (Pass 2). No shared `Scrubber` type.

### 4.2 Not scrubbed -- and currently OK (not frozen as values)

| Value | Why OK today |
|-------|----------------|
| Sample / temp absolute paths in status, doctor, search | Only key-shape or path-contains asserts |
| Hit scores, file paths, excerpts | Not in file goldens |
| Bench timings / `cv_pct` | Type presence only |
| Doctor `issues[].message`, `suggested_commands` | Structural / contains root |

### 4.3 Should scrub **before** promoting new freezes

| Target golden | Scrub / canonicalize |
|---------------|----------------------|
| Live search JSON (any format) | Absolute `root` / `index_path` / hit `file` / compact `p` values → `<root>/…` or path relative to sample root; keep relative sample paths if session root is the sample corpus fixed tree |
| Live search with embed | Prefer `--no-embed` freezes; if embed on, scrub scores or use structural-only |
| Doctor / failure messages with paths | Path scrub or keep message as `<message>` + separate teaching golden for path-free templates |
| Package version | Keep `"<version>"` (already) |
| Handbook / robot-docs body | Version strings inside markdown if any; otherwise exact |
| Capsule_format synthetic | **No path scrub needed** if sample stays relative (`src/auth.rs`) -- pure exact golden |

### 4.4 Over-scrub risk

| Practice | Risk |
|----------|------|
| Blanking **all** usage `error.message` | Loses regression detection on did-you-mean / Example / Tip lines that agents depend on |
| Scrubbing scores under `--no-embed` | Usually unnecessary; can hide ranking bugs |
| Scrubbing `schema_version` | **Do not** -- contract constant `"1.0.0"` must hard-fail on change |

---

## 5. Failure messaging & update path (CLI-specific)

| Skill expectation | CLI machine contracts today |
|-------------------|----------------------------|
| Unified / line diff | **No** -- Debug `assert_eq!` |
| Write `*.actual` | **No** (gitignore prepared in Pass 2 only) |
| `ASGREP_UPDATE_GOLDENS=1` rewrite | **No** -- fixtures are `include_str!` + manual edit |
| Hint in panic | **No** |
| Pretty fixtures for review | **capabilities** yes; **envelopes** / **shapes** minified one-liners |

Closest path: Pass 2 design (testkit `assert_golden_json` + machine_contract scrub preset). First migration target remains these three fixture files.

---

## 6. Volatility / flakiness risks

| Risk | Level | Notes |
|------|-------|-------|
| Capabilities flag/env list churn | Medium (intentional) | Full equality fails on any CLI surface add -- good contract, noisy PRs |
| Key order in JSON | Low | Product claims stable serde_json key order (`agent_contract.deterministic`); shapes sort keys before compare |
| Platform paths in current goldens | Low | Not value-frozen |
| Live hit order with embed | High if goldened without scrub | All current success searches use `--no-embed` in machine_contracts agent tests -- good |
| `agent_surface` binary path | Medium skip risk | Soft skip if no debug/release-perf binary -- silent non-run |
| Capsule_format float scores | Low | Fixed literals `5.5` / `3.2` |
| Bench suite `suite_ok` | Accepts 0 or 2 | Intentionally not a golden pass/fail on suite content |

**Flake verdict:** existing **file goldens are stable**. Risk appears when promoting **live search dumps** without path scrub and without pinning `--no-embed` / limit / format / query.

---

## 7. Coverage gaps (CLI machine contracts)

Priority-ordered gaps (content, not infra):

1. **Successful search hit payloads** -- no frozen agent / agent-capsule / compact / native page for fixed sample query.
2. **Format matrix incomplete** -- shapes + deep asserts missing for `native`, `github`, `gitlab` (capabilities advertises six formats).
3. **Usage / teaching message bodies** -- envelope golden scrubs message; agent_surface greps partial; no single scrubbed golden for usage text.
4. **robot-docs body** -- only contains `"agent handbook"`; long agent-facing markdown unprotected.
5. **Success shapes** for watch / keyword / semantic / codemode envelopes (optional; lower value than search hits).
6. **Doctor unhealthy full envelope** -- not template-frozen (partial hand asserts).
7. **Eval gold fixture** -- only ephemeral temp file in test; no checked-in sample gold for CLI eval contract (honesty/baselines are separate).

Non-gaps (correctly non-golden):

- Bench wall times / speedup numbers.
- SIGPIPE behavior (absence oracle).
- Peer surface HitKey parity (different pattern).
- Historical `benchmarks/results/*` metrics.

---

## 8. Anti-pattern check

| Anti-pattern | Present? | Notes |
|--------------|----------|-------|
| Unreviewed freezes | **Low risk** | Capabilities/envelopes look deliberately authored with placeholders |
| Implementation-detail goldens | **Low** | Public machine JSON, not private Debug |
| Huge golden files | **No** | capabilities ~10 KB; envelopes/shapes <2 KB |
| Snapshot without scrub of dynamics | **Avoided** for version | Message scrub is aggressive (see over-scrub) |
| Dual sources of truth | **Mild** | Capsule_format synthetic vs CLI live bounds for same formats -- complementary, not conflicting |
| Blind re-bless culture | **N/A** | No update mode yet -- human must edit files |

---

## 9. Recommended promotion path

Order by **(value × ease × depends on Pass 2 helper)**. Prefer **file goldens** next to CLI fixtures or under future `tests/golden/cli/` once assert_golden lands.

| Step | Promote what | From | To pattern | Scrub preset |
|------|--------------|------|------------|--------------|
| **1 (first)** | Synthetic formatter dumps from `capsule_format` sample() | Hand `assert_eq!`s | `crates/ast-sgrep-plugins/tests/fixtures/*.json` or shared golden files for agent / agent-capsule / compact / github / gitlab | None (relative paths) |
| **2** | Live CLI `--no-embed` search for fixed query `process_request` (or `callers:process_request`) limit 2–5 | Cluster D bounds | Scrubbed exact JSON per format: `agent.json`, `agent-capsule.json`, `compact.json` | Sample-root-relative paths; keep scores |
| **3** | Top-level shapes for `native`, `github`, `gitlab` | Missing matrix | Extend `machine_shapes.json` + one shape test each | Keys only |
| **4** | Usage teaching envelopes (format typo, missing query, edit-distance) | R-002/R-003 + typo tests | Scrubbed exact stderr-or-JSON message goldens (path-free cases first) | Optional `<root>` if needed |
| **5** | `robot-docs` / `--robot-help` markdown body | Substring only | Exact or lightly scrubbed `.md` golden | Version tokens if present |
| **6 (later)** | Doctor unhealthy template + operational messages **subkinds** | Partial asserts | Skeleton golden + optional message class tests | `<message>` or path scrub |

**Do not promote first:** bench timings, embed-on search dumps, full capabilities re-dump without review (already golden).

**Hand asserts to keep** (do not replace with opaque dumps):

- Budget inequalities (preview ≤121, excerpt lines, compact `b[2] ≤ budget`).
- `suggested_next` starts with `asgrep `.
- Compact UTF-8 safety (empty snippet rather than split codepoint).
- Peer HitKey parity.
- Exit code matrices (0/1/2).

---

## 10. Aggregated findings for beads (max 6)

Deep items only. **Do not file `br` from this pass** (mission). Acceptance criteria written for later beads after Pass 2 P0 helper ideally lands -- content beads can start with local `assert_eq!` + fixture files if needed.

### F1 -- Freeze successful search hit payloads (CLI, `--no-embed`)

| | |
|--|--|
| **Priority** | **P1** |
| **Problem** | Agent-visible search JSON is only shape/bounds-tested. Ranking, ref/preview/excerpt, compact dictionaries, and field renames can ship without failing machine_contracts. |
| **Evidence** | `machine_contracts.rs:149-199,502-521`; `cli_smoke.rs` non-empty/type checks; Pass 1 top candidate #1; no hit dump under `tests/fixtures/`. |
| **Acceptance** | For at least one fixed sample query and limit, commit scrubbed goldens for **agent**, **agent-capsule**, and **compact** CLI stdout (envelope + hits). Test fails on hit field rename/order/content drift. Paths canonicalized to sample-root-relative. Document generating command. Prefer `--no-embed`. |

### F2 -- Complete search format matrix shapes (native / github / gitlab)

| | |
|--|--|
| **Priority** | **P1** |
| **Problem** | `capabilities.search_formats` lists six formats; `machine_shapes.json` and deep machine_contracts coverage only seriously exercise three. github is smoke-typed in `cli_smoke`; native/gitlab nearly absent at CLI integration. |
| **Evidence** | `capabilities.json` `search_formats`; `machine_shapes.json` keys; `machine_contracts` format strings only `agent|agent-capsule|compact|invalid`; `cli_smoke` github items meta only. |
| **Acceptance** | `machine_shapes.json` includes sorted top-level keys for `native`, `github`, `gitlab`. machine_contracts (or sibling) runs one success search per format and `assert_shape`. Optional: one nested hit-key shape array per format. |

### F3 -- Migrate plugins `capsule_format` exact asserts to file goldens

| | |
|--|--|
| **Priority** | **P2** |
| **Problem** | Dense hand goldens on synthetic responses are hard to review and diverge stylistically from CLI fixtures; intentional formatter changes require editing many Rust literals. |
| **Evidence** | `crates/ast-sgrep-plugins/tests/capsule_format.rs` (8 tests, 32 `assert_eq!`); Pass 1 candidate #4; Pass 2 near-miss §2.5. |
| **Acceptance** | At least agent-capsule + compact (+ optionally github/gitlab) full `Value` dumps as JSON fixtures compared after format; keep behavioral asserts (budgets, UTF-8, size ratio, suggested_next prefix). Update path documented (or uses testkit helper when available). |

### F4 -- Teaching / usage message goldens (machine JSON + human)

| | |
|--|--|
| **Priority** | **P2** |
| **Problem** | Envelope golden blanks all messages; agent_surface greps are partial, skip-prone, and outside cargo test. Agent UX regressions in did-you-mean / Example / triad footer are weakly gated. |
| **Evidence** | `envelopes.json` `"<message>"`; `operational`/`usage` scrub assigns; `agent_surface/R-002`, `R-003`; `edit_distance_two_typos_*`; `format_aliases_typos_*` only checks `error.kind == usage`. |
| **Acceptance** | Path-free usage cases freeze full message (or full failure JSON) as goldens; human teaching cases either join cargo test or share the same fixture strings. R-001 remains non-golden absence check. Wire at least one teaching test into `cargo test -p ast-sgrep-cli`. |

### F5 -- CLI fixture workflow: pretty envelopes/shapes + golden compare UX

| | |
|--|--|
| **Priority** | **P2** (depends on Pass 2 P0/P1 infra; can pretty-print sooner) |
| **Problem** | envelopes/shapes are one-line minified; failures use Debug dumps; no update env. Raises cost of F1–F4 and of capabilities flag churn reviews. |
| **Evidence** | `envelopes.json` / `machine_shapes.json` single-line; Pass 2 §1 diff/update gaps; `assert_eq!` at `:96,309,326`. |
| **Acceptance** | Pretty-print envelopes + shapes (stable 2-space, trailing newline) for git review; machine_contracts compares via shared helper or at least prints golden path on mismatch; when testkit golden lands, migrate the three fixture compares first. |

### F6 -- robot-docs / handbook body freeze

| | |
|--|--|
| **Priority** | **P3** |
| **Problem** | Long agent handbook only checked for substring `"agent handbook"`; envelope fields checked lightly. Large doc drift is invisible. |
| **Evidence** | `machine_contracts.rs:348-367`; Pass 1 candidate #3. |
| **Acceptance** | Golden file for default guide body (exact or scrubbed); `--json` envelope may remain structural + body hash/equality. |

---

## 11. Already excellent (protect -- do not rewrite)

1. **True scrubbed equality** for `capabilities` and failure/version envelopes -- rare and valuable; keep full-object compare (implicit deny-unknown).
2. **Placeholder convention** (`<version>`, `<command>`, `<message>`) in fixtures -- clear, reviewable.
3. **`assert_shape` + sorted keys** for operational success surfaces -- cheap, stable, correct structural pattern.
4. **Machine stderr contract** (`assert_success` / doctor require empty stderr) + `NO_COLOR=1` -- stabilizes agent parsing.
5. **Exit code taxonomy** frozen in capabilities + tested (0 success, 1 usage, 2 operational/doctor unhealthy).
6. **`--no-embed` default in success contract tests** -- reduces flaky ranking if hit goldens are added.
7. **Product `print_machine_failure` shape matches envelope golden** -- single writer, not ad-hoc per command.
8. **Plugins synthetic `SearchResponse`** -- deterministic formatter lab; keep as unit-level source of truth for format *projection* rules.
9. **Compact stability re-run** (`assert_eq!(compact, again)`) and path-table uniqueness -- excellent non-dump invariants.
10. **Peer HitKey parity** (`no_embed_hit_key_parity`) -- complements goldens; do not replace with a dump.
11. **Broken-pipe non-panic** (product `write_stdout_line` + R-001) -- correct absence oracle, not a golden.
12. **Capabilities lists search formats and clap commands** -- discovery contract for agents; full equality protects drift.

---

## 12. Method notes

- Read full `machine_contracts.rs` (15 tests, 57 `assert_eq!`, 3 `include_str!` fixtures).
- Parsed all three fixture JSON files; compared format matrix to `capabilities.search_formats`.
- Reviewed `agent_surface` R-001..003; `capsule_format.rs` (8 tests); `cli_smoke.rs`; `no_embed_hit_key_parity.rs`; `src/machine.rs` envelope writer.
- Cross-checked Pass 1 inventory rows and Pass 2 near-miss §2.1 / scrub partials.
- No code changes; no beads; no commit.

---

## 13. Deliverable checklist

| Required section | Status |
|------------------|--------|
| Quality scorecard per cluster | §3 |
| Scrubbing map | §4 |
| Promotion path for hand asserts → file goldens | §9 |
| Aggregated bead-ready findings (3–6) | §10 (F1–F6) |
| Already excellent | §11 |

*End of Pass 3.*
