# Pass 2 — Golden Infrastructure Gaps

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (audit + one hygiene landing; no full infra)  
**Date:** 2026-08-07  
**Skill:** `testing-golden-artifacts`  
**Prior:** [`PASS1_GOLDEN_INVENTORY.md`](PASS1_GOLDEN_INVENTORY.md) (42 rows, maturity ~4/10)  
**Scope:** golden-test **infrastructure only** -- `assert_golden`, UPDATE mode, insta vs custom, `*.actual`, diffs, PROVENANCE, scrubbers, layout. No bead filing, no commit, no product WIP.

---

## 1. Current state evidence (skill checklist)

| Checklist item | Y/N/Partial | Evidence |
|----------------|-------------|----------|
| **assert_golden / shared compare helper with UPDATE_GOLDENS** | **N** | Zero hits for `assert_golden`, `UPDATE_GOLDENS`, `UPDATE_SNAPSHOTS`, `INSTA_UPDATE` outside this audit tree. No helper in `crates/ast-sgrep-testkit/`. Closest local pattern is private `fixture()` + `assert_eq!` in `crates/ast-sgrep-cli/tests/machine_contracts.rs:66-73,92-99` (load + equality only; no update mode). |
| **Scrubber handles dynamic values** | **Partial** | Ad-hoc field replacement only: `capabilities["version"] = "<version>"` (`machine_contracts.rs:95-98`); `value["command"] = "<command>"` / `value["error"]["message"] = "<message>"` (`:309-325`). Placeholders live in `crates/ast-sgrep-cli/tests/fixtures/envelopes.json`. **No** shared registry, **no** UUID/timestamp/path/duration scrubbers, **no** `Scrubber` type. |
| **Every golden reviewed before first commit (process)** | **Partial** | CLI machine fixtures and `tests/fixtures/ranking/cases.json` look deliberate (deny_unknown_fields, explicit scrub). No written process, no `PROVENANCE.md`, no review checklist for new freeze files. Extraction "goldens" are hand-maintained expectation **tuples** in Rust (`extraction_goldens.rs`), not reviewed dump files. |
| **PROVENANCE.md for goldens** | **N** | No `PROVENANCE.md` under test trees. `benchmarks/results/baselines.md` records **metric** provenance (honesty ledger), not test golden generation. Search: only npm release `provenance` and product `signal_provenance` (false friends for this checklist). |
| **.gitignore `*.actual`** | **Y** (landed this pass) | Was **N** (Pass 1). Now under `# Tests` in [`.gitignore`](../../../.gitignore): `*.actual` with comment that assert_golden mismatch dumps must not be committed. No `*.actual` files existed in-tree. |
| **CI fails on golden mismatch (no auto-update)** | **Partial** | `.github/workflows/ci.yml` `build-and-test` runs `cargo test --workspace --release` (lines ~44). Mismatches fail via ordinary `assert_eq!` -- **correct default** (no write path exists). Gaps: workflow is **manual-only** (`workflow_dispatch`); no golden-specific job; no forbid of update env; no upload of `*.actual` / `.snap.new` on failure; no check that update mode is unset in CI. |
| **Diff output in failure messages** | **Partial** | Stock Rust `assert_eq!` prints Debug left/right (e.g. `machine_contracts.rs:96` full JSON Value). No unified diff, no path to golden, no "re-run with ASGREP_UPDATE_GOLDENS=1" hint. No `pretty_assertions`, `similar`, `diffy`, or `insta` in workspace `Cargo.toml` / crate manifests. |
| **Goldens organized by feature/module** | **Partial** | Per-crate local fixtures only: `crates/ast-sgrep-cli/tests/fixtures/{capabilities,envelopes,machine_shapes}.json`; `crates/ast-sgrep-lang/tests/fixtures/extract/*` (**inputs**, not expected dumps); `tests/fixtures/{sample,ranking}/` (corpus + oracle). **Missing:** `tests/golden/`, any `**/snapshots/`, any `*.snap` / `*.golden` files. |
| **Cross-platform canonicalization** | **Partial** | `NO_COLOR=1` on CLI spawn (`machine_contracts.rs:12`); sample root `canonicalize()` in `testkit/src/fixture.rs:5-6`; Windows smoke uses absolute paths. No golden pipeline step for `\r\n`→`\n`, path separators, temp-dir scrub, or sorted-key JSON pretty-print before freeze. |
| **insta / cargo-insta workflow** | **N** | No `insta` dependency; no `cargo-insta`; no `.snap` files. |
| **Binary / semantic golden helpers** | **N** | Not present (and not urgently needed for current text/JSON surfaces). |

---

## 2. Existing near-misses

Code that **almost** is golden infrastructure. Each row: what it provides vs what is still missing.

### 2.1 CLI machine fixture loader + scrub (strongest near-miss)

| | |
|--|--|
| **Paths** | `crates/ast-sgrep-cli/tests/machine_contracts.rs:66-99,270-325`; `crates/ast-sgrep-cli/tests/fixtures/*.json` |
| **Provides** | Named fixture load via `include_str!`; full `Value` equality after manual field scrub; structural key-shape freeze (`assert_shape`); failure envelope templates with `<command>` / `<message>` / `<version>`. |
| **Missing** | Shared API; file write on update; `.actual` on fail; unified diff; scrub registry; path relative to a golden root; reuse from other crates. |

```66:99:crates/ast-sgrep-cli/tests/machine_contracts.rs
fn fixture(name: &str) -> Value {
    let raw = match name {
        "capabilities" => include_str!("fixtures/capabilities.json"),
        "shapes" => include_str!("fixtures/machine_shapes.json"),
        "envelopes" => include_str!("fixtures/envelopes.json"),
        _ => panic!("unknown fixture {name}"),
    };
    serde_json::from_str(raw).expect("valid JSON fixture")
}
// ...
fn capabilities_and_version_match_goldens() {
    // ...
    capabilities["version"] = "<version>".into();
    assert_eq!(capabilities, fixture("capabilities"));
```

### 2.2 Ranking oracle fixture loader

| | |
|--|--|
| **Paths** | `crates/ast-sgrep-core/tests/ranking_oracle.rs`; `tests/fixtures/ranking/cases.json` |
| **Provides** | Versioned JSON schema (`deny_unknown_fields`), deserialize + behavioral constraints (`must_include` / `max_rank`). Correct **oracle** pattern. |
| **Missing** | Not a dump golden; no update loop for full ranked lists; no scrub of scores/paths for full-response freezes. |

### 2.3 Extraction "goldens" + shared conformance harness

| | |
|--|--|
| **Paths** | `crates/ast-sgrep-lang/tests/extraction_goldens.rs`; `crates/ast-sgrep-testkit/src/lang.rs` (`assert_language_conformance`, `LanguageConformanceCase`) |
| **Provides** | Named suite; 13 language **input** fixtures; shared presence/forbid/pattern contract in testkit (right home for shared assert helpers). |
| **Missing** | Expectations are Rust tuples, not freeze files; no serialize-and-diff ExtractionResult; no UPDATE path. |

### 2.4 Determinism loop (ephemeral self-golden)

| | |
|--|--|
| **Paths** | `crates/ast-sgrep-core/tests/determinism_loop.rs` |
| **Provides** | First-run JSON baseline, 50× byte-stable compare -- proves search JSON can be goldenized. |
| **Missing** | Baseline is in-memory only (not committed); no file I/O; no update/scrub helpers. |

### 2.5 Plugins formatters (dense hand goldens)

| | |
|--|--|
| **Paths** | `crates/ast-sgrep-plugins/tests/capsule_format.rs` |
| **Provides** | Fixed synthetic `SearchResponse`; many exact field `assert_eq!`s -- ideal first consumers of file goldens. |
| **Missing** | Expected payloads embedded in Rust; review UX is PR-diff of assertions, not of artifact files. |

### 2.6 Testkit fixture root + isolation (support layer)

| | |
|--|--|
| **Paths** | `crates/ast-sgrep-testkit/src/fixture.rs` (`sample_root`, `sample_file`); `isolation.rs`; `lib.rs` re-exports |
| **Provides** | Canonical sample corpus path; temp index sessions; published crate for all integration tests (`publish = false`). Natural home for `assert_golden`. |
| **Missing** | Only input helpers today (~10 lines in `fixture.rs`); no golden I/O module. |

### 2.7 Pi release contract freeze (JS, parallel pattern)

| | |
|--|--|
| **Paths** | `packages/pi/release-contract.json`; `packages/pi/scripts/check-contract.mjs` |
| **Provides** | Checked-in contract + strict structural checks; deterministic 2-space JSON policy. |
| **Missing** | Separate from Rust golden infra; not wired to UPDATE_GOLDENS. Treat as sibling discipline, not the shared helper. |

### 2.8 Env flag conventions (naming near-miss)

| | |
|--|--|
| **Paths** | `crates/ast-sgrep-testkit/src/safety.rs` (`ASGREP_REAL_NETWORK_TESTS`, product `ASGREP_*` table) |
| **Provides** | Established **`ASGREP_` prefix** for product and test gates (`=1` truthy style). |
| **Missing** | No golden update env yet -- when added, must match `ASGREP_*`, not bare `UPDATE_GOLDENS` or `AST_SGREP_*`. |

---

## 3. Design recommendation (this repo)

### 3.1 Decision: **custom `assert_golden` in `ast-sgrep-testkit`**, not insta

| Factor | Custom testkit | insta |
|--------|----------------|-------|
| Workspace size | 11 crates; testkit already shared by CLI/core/lang/LSP tests | Extra workspace dep + `cargo-insta` CLI for humans |
| Existing pattern | `include_str!` fixtures + `assert_eq!` + ad-hoc scrub | Would introduce a second style beside machine_contracts |
| Review UX | `git diff` on plain JSON/md under known paths; agents can update with env + re-run | Best UX is interactive `cargo insta review` TUI -- poor for headless agents / bead loops |
| Agent friendliness | `ASGREP_UPDATE_GOLDENS=1 cargo test -p …` → review file diff → commit | `.snap` metadata headers, `.snap.new`, settings crates -- more ceremony |
| Dependency policy | testkit stays thin (serde_json, tempfile already); optional `similar` only if diffs need it | New crate + macros; filters/redactions are powerful but another skill surface |
| CI | Compare-only by default if update env unset (matches current fail-closed tests) | Needs `INSTA_UPDATE=no` discipline |

**Recommendation:** implement a small **custom golden module** in `ast-sgrep-testkit`. Revisit insta only if many **inline** Debug snapshots of Rust types become the dominant need (today the high-value freezes are **JSON / markdown files**).

### 3.2 Proposed API sketch (signatures only)

```rust
// crates/ast-sgrep-testkit/src/golden.rs  (future)

/// True when ASGREP_UPDATE_GOLDENS is set to a truthy value ("1", "true", "yes", "on").
pub fn updating_goldens() -> bool;

/// Canonicalize text for comparison: UTF-8, strip trailing whitespace per line optional,
/// normalize `\r\n` → `\n`. Paths: caller should scrub before call.
pub fn canonicalize_text(s: &str) -> String;

/// Pretty JSON with stable key order if needed (or document that serde_json Map order
/// is already product-stable for machine contracts).
pub fn canonicalize_json(value: &serde_json::Value) -> String;

/// Compare `actual` to golden file at `tests/golden/{name}` (or crate-relative override).
/// On mismatch: write `{golden}.actual`, panic with unified diff + update hint.
/// When updating_goldens(): write golden, skip fail (or pass after write).
pub fn assert_golden(name: &str, actual: &str);

/// JSON convenience: serialize → scrub → assert_golden.
pub fn assert_golden_json(name: &str, value: &serde_json::Value);

/// Optional: apply registered scrubbers then assert.
pub fn assert_golden_scrubbed(name: &str, actual: &str, scrubber: &Scrubber);

pub struct Scrubber { /* ordered (Regex, replacement) rules */ }
impl Scrubber {
    pub fn standard() -> Self;           // version, UUID, ISO time, abs paths, durations
    pub fn with(self, pattern: &str, replacement: &'static str) -> Self;
    pub fn scrub(&self, input: &str) -> String;
}

/// Machine-contract style field redaction helpers (migrate from machine_contracts).
pub fn scrub_json_fields(value: &mut serde_json::Value, paths: &[(&str /*json pointer*/, &str /*placeholder*/)]);
```

**Not required for v1:** binary goldens, fuzzy numeric goldens, insta redaction paths.

### 3.3 Where goldens should live

| Kind | Location | Rationale |
|------|----------|-----------|
| **Shared / multi-crate surfaces** (sample search dumps, ranking full lists, handbook bodies) | **`tests/golden/<feature>/…`** | Already have `tests/fixtures/` for inputs; mirror with `tests/golden/` for expected outputs. One PROVENANCE.md at `tests/golden/PROVENANCE.md`. |
| **Crate-private freezes** (CLI capabilities/envelopes already there; plugins capsule) | Keep or migrate to **`crates/<crate>/tests/fixtures/`** or `…/golden/` next to the test | `include_str!` / `CARGO_MANIFEST_DIR` already used; no need to force every artifact to repo root. |
| **Extraction inputs** | Stay in `crates/ast-sgrep-lang/tests/fixtures/extract/` | Inputs ≠ goldens. Future dumps → `tests/golden/extract/<lang>.json` or crate `tests/golden/`. |
| **Benchmark ledgers** | Stay in `benchmarks/results/` | Honesty docs, not CI assert_golden. |

**Convention:** relative name in API maps to `tests/golden/{name}.json` (or `.md` / `.txt` by extension in the name). Crate-local override via `assert_golden_at(path, actual)` if needed.

**Do not** introduce insta default `src/snapshots/` unless adopting insta.

### 3.4 UPDATE env name

| Candidate | Verdict |
|-----------|---------|
| `UPDATE_GOLDENS` | Skill default -- **reject** (inconsistent with product) |
| `AST_SGREP_UPDATE_GOLDENS` | **reject** (crate is `ast-sgrep`, env is already `ASGREP_*`) |
| **`ASGREP_UPDATE_GOLDENS=1`** | **Adopt** -- matches `ASGREP_REAL_NETWORK_TESTS`, `ASGREP_NO_EMBED`, bench gates |

Optional aliases: none in v1 (one name, document in testkit module docs + PROVENANCE).

Truthy parsing: reuse the same boolish style already exercised for agent envs in machine_contracts (`1`/`true`/`yes`/`on`) if a shared `env_flag` helper is exported from safety/testkit; otherwise document `=1` only.

### 3.5 Scrubber placement

1. **Registry:** `crates/ast-sgrep-testkit/src/golden/scrub.rs` (or `scrub.rs` beside `golden.rs`).
2. **Standard rules** (opt-in via `Scrubber::standard()`): cargo package version strings, ISO timestamps, absolute paths (`/Users/…`, `/tmp/…`, Windows drive paths), UUID-shaped strings, durations.
3. **Domain rules** (call-site or named presets):
   - `Scrubber::machine_contract()` -- `<version>`, optional message length bounds (migrate from machine_contracts).
   - Search response preset: temp root → `<root>`, scores if non-deterministic under embed → `<score>` only when needed (prefer no-embed freezes).
4. **Do not** scrub inside product formatters; scrub only in the test path before compare/write.

### 3.6 Diff + failure UX (required behavior)

On mismatch (compare mode):

1. Write `actual` to `{golden_path}.actual` (gitignored).
2. Panic message includes: golden path, `.actual` path, unified text diff (line-oriented; implement with minimal code or optional `similar` dev-dep), and:
   `hint: ASGREP_UPDATE_GOLDENS=1 cargo test -p <crate> --test <name> -- <filter>`
3. Never write the golden file unless update env is set.
4. CI: leave env unset; optionally `find … -name '*.actual'` fail-closed artifact step later (P2).

### 3.7 Process (lightweight)

- First commit of a new golden: human or agent must **open the file** and sanity-check (not blind accept).
- One-line entry in `tests/golden/PROVENANCE.md`: date, generating command, scrub preset, intentional semantics.
- Updates: same path -- update mode → `git diff tests/golden` → review → commit with `fix:`/`feat:` explaining behavior change.

---

## 4. Gap severity ranking (infrastructure only)

Aggregate micro-items into **6** substantive gaps for later bead aggregation. **Do not implement here** except the landed `*.actual` gitignore.

### P0 — Shared assert_golden + update mode + failure artifacts

| | |
|--|--|
| **Problem** | No shared compare/update helper. Each test hand-rolls `assert_eq!` / presence checks; intentional freezes cannot be refreshed safely. |
| **Why it matters** | Blocks converting Pass 1 high-value candidates (search dumps, formatters, handbook, extraction dumps) without per-suite snowflakes. Agent loops cannot regenerate goldens uniformly. |
| **Acceptance** | `ast-sgrep-testkit` exports `assert_golden` / `assert_golden_json` + `updating_goldens()` reading **`ASGREP_UPDATE_GOLDENS`**. Compare mode fails closed; update mode rewrites golden. Mismatch writes `*.actual`. At least one migration of an existing file golden (e.g. machine envelopes or a plugins case) uses the helper. Module docs document env + paths. |

### P0 — Scrubber registry (minimal standard + machine-contract preset)

| | |
|--|--|
| **Problem** | Only ad-hoc field writes in machine_contracts; no reusable scrub for paths/versions/timestamps. |
| **Why it matters** | Full search JSON and multi-platform CLI output will flake without scrub; ad-hoc scrubs diverge and hide real diffs. |
| **Acceptance** | `Scrubber::standard()` + `Scrubber::machine_contract()` (or equivalent presets) in testkit; machine_contracts (or a sibling) uses preset instead of inline string assigns for the same fields; unit test that standard scrub is deterministic. |

### P1 — Unified (or line) diff in failure messages

| | |
|--|--|
| **Problem** | `assert_eq!` Debug dumps of large JSON are unreadable; no pointer to golden/actual paths. |
| **Why it matters** | Review time and false "just re-bless" pressure; agents cannot localize the changed field cheaply. |
| **Acceptance** | On mismatch, panic body contains a line-oriented diff (or first N changed hunks), absolute/relative golden path, `.actual` path, and update-env hint. |

### P1 — Golden directory layout + PROVENANCE

| | |
|--|--|
| **Problem** | No `tests/golden/`; no PROVENANCE for test freezes; organization is accidental per-crate fixtures. |
| **Why it matters** | New freezes will scatter; reviewers cannot tell how a golden was produced or which scrub applies. |
| **Acceptance** | Create `tests/golden/` with `PROVENANCE.md` template (command, date, scrub, notes). Document crate-local vs shared rule (this doc §3.3). At least one shared golden lives under `tests/golden/` **or** PROVENANCE entries for the three CLI fixture JSON files if they remain crate-local. |

### P2 — CI golden hygiene (still no auto-update)

| | |
|--|--|
| **Problem** | CI is manual workflow_dispatch only; no explicit "update env must be unset"; no upload of `*.actual` on failure. |
| **Why it matters** | Today fail-closed by accident (no write path). Once update mode exists, a mis-set secret/env could rewrite goldens in CI or hide mismatches. Artifact upload speeds remote triage. |
| **Acceptance** | Document in workflow or gate script: unset/`0` for `ASGREP_UPDATE_GOLDENS`. On test failure, upload `**/*.actual` (and any future snapshot sidecars) as artifacts. Optional: post-step `find` that fails if any `.actual` remains (compare-mode pollution). **Do not** add auto-update in CI. |

### P3 — Cross-platform canonicalize helper in the golden path

| | |
|--|--|
| **Problem** | Partial: NO_COLOR + path canonicalize for fixtures; no golden-level CRLF/path scrub. |
| **Why it matters** | Windows smoke already in CI; future text goldens of paths/excerpts will flake on separators and newlines. |
| **Acceptance** | `canonicalize_text` always applied inside `assert_golden`; optional path scrub in standard Scrubber verified on a Windows-style path string unit test. Matrix job need not grow solely for this. |

### Explicitly out of scope for infra beads (product/coverage)

- Adding search-hit / MCP schema / handbook **content** goldens (Pass 1 candidates -- separate coverage beads after P0 lands).
- Replacing ranking oracle or metamorphic suites with dump goldens.
- Treating `benchmarks/results/baselines.md` as CI goldens.
- Adopting insta unless a later pass reopens §3.1.

---

## 5. Hygiene landed this pass

| Change | Why |
|--------|-----|
| [`.gitignore`](../../../.gitignore): `*.actual` under `# Tests` | Skill checklist; safe before assert_golden exists so future mismatch dumps never get committed. |

**Not done (by mission):** full assert_golden implementation, scrubber crate, PROVENANCE file body, CI workflow edits, br beads, git commit.

---

## 6. Summary for later aggregation

| Area | State after Pass 2 |
|------|--------------------|
| Shared assert_golden + UPDATE | **Absent** (P0) |
| Scrubber registry | **Ad-hoc only** (P0) |
| Diff UX | **assert_eq! only** (P1) |
| Layout + PROVENANCE | **No tests/golden/** (P1) |
| `*.actual` gitignore | **Done** |
| CI compare-only | **De facto** via no write path; harden when write path lands (P2) |
| Cross-platform canonicalize | **Partial** (P3) |
| insta | **Rejected** for primary design |
| Env name | **`ASGREP_UPDATE_GOLDENS`** |
| Home for helper | **`ast-sgrep-testkit`** |

**Near-misses to migrate first:** `machine_contracts` fixture/scrub → testkit golden + scrub; then `capsule_format` hand asserts → file goldens; keep ranking/extraction as structural oracles until dump goldens are intentional.

**Maturity (infra only):** still ~3/10 after this pass (gitignore + design locked); jumps to ~6/10 when P0 lands, ~7–8/10 with P1.
