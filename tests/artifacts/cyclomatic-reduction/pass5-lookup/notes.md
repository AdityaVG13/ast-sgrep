# Pass 5 — Lookup-table transforms

Technique: `lookup_table` (data-driven dispatch; not per-case arrow maps that re-home CC).

## Transforms

### 1. `argvFor` — `packages/pi/extension/src/codemode/dispatch.ts`

- **Before:** `switch (tool)` with 10 cases + default (CC 22).
- **After:** `ARGV_SPEC: Record<string, ArgvSpec>` data table + thin interpreter.
- Forms: `capsule` (search/defs/callers/imports), `semantic`, `chain`, `status` (index_status + catalog_*), `index_repo` (force ternary kept — essential).
- Helper: `argStr` (shared `??` once).
- Default unknown tool → capsule/query (same as prior `default`).

### 2. `searchToolCall` — `packages/pi/extension/src/index.ts`

- **Before:** `switch (mode)` with fallthrough prefix group (CC 17).
- **After:** `SEARCH_CALL_SPEC: { [M in SearchMode]: SearchCallSpec }` exhaustive typed table + form interpreter.
- Specs: semantic / chain / defs|callers|imports / search(+optional prefix).
- Prefix modes (`pattern|word|literal|regex`) share `tool: "search"` + `prefix` field (no fallthrough).

### 3. `literal_sql` — `crates/ast-sgrep-core/src/search/passes/literal.rs`

- **Before:** nested `if case_insensitive { if lang … } else { if lang … }` SQL selection (CC 18).
- **After:** `LITERAL_SQL: [[&str; 2]; 2]` indexed `[case_insensitive][has_lang]` + `literal_sql_template`.
- Pattern escape path still branches (LIKE vs GLOB) — required for correct escaping.
- Word-mode postfilter + context map left in place (essential_domain).

## Failed first attempt (metric game, reverted)

Per-case arrow maps (`ARGV_BUILDERS` / per-mode builders) dropped function CC but **file ΣCC flat or +1** (Kolmogorov dump). Replaced with pure data tables + shared interpreter.

## Public API

Unchanged: `argvFor(tool, args)`, internal `searchToolCall`, private `literal_sql`.
