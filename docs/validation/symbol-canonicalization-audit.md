# Symbol canonicalization audit

**Branch:** `fix/csharp-grammar-pattern-difu-5`  
**Bead:** `ast-sgrep-0b1a`  
**Scope:** Every Rust site that produces or consumes a symbol/key, with the
equivalence contract each site assumes, and cross-site divergences.

## Canonical forms in use

| Form | Definition | Used by |
|------|------------|---------|
| **Surface name** | Exact text extracted from the AST (`SymbolDef.name`, caller/callee identifiers). Case and Unicode preserved. | Index write path, LSP cursor symbol, plugin JSON |
| **Rank key** | ASCII-aware lowercase via `rank::normalized_symbol` (borrow when already ASCII-lowercase; else `to_lowercase`) | `score_symbol`, `best_symbol_score`, `coverage_symbol_score`, `score_def`, `score_caller` |
| **Query terms** | Hybrid/mode tokens lowercased in `query::tokenize_for_scoring` / `ParsedQuery::parse` | Lexical FTS prep, symbol-pass scoring terms |
| **Prefixed target** | Raw substring after `defs:` / `callers:` / `imports:` (case preserved in `target`) | Mode routing; scoring still goes through rank normalization |
| **SQL lookup key** | `lower(column) = lower(?)` or `lower(col) LIKE …` | `symbols_named`, caller filters (`store/sql.rs`) |
| **Language id** | `Language::as_str` (`rust`, `csharp`, …) | Indexed `files.language`, native pattern hits, `--lang` |
| **Intent / hit-kind id** | Lowercase `as_str` enums (`literal`, `def`, …) | Intent routing, serialization |
| **ANN / IVF fingerprint** | Blake3 over chunk count, max id, dim, backend, `data_version` — **not** a symbol string | Sidecar invalidation only |

There is **no single shared `canonicalize_symbol()`** today. Ranking and SQL agree
on case-insensitive match; storage and LSP keep surface names.

---

## Produce sites

| Site | What is written | Canonic form | Notes |
|------|-----------------|--------------|-------|
| `ast-sgrep-lang` extractors (`extract.rs` / `langs.rs`) | `SymbolDef.name`, `CallSite.caller` / `callee` | Surface name | No lowercasing; language-specific identifier text |
| `index.rs` upsert | `symbols.name`, `callers.caller`/`callee`, `semantic_chunks.symbol_name` | Surface name | Kind stored as lowercase via `format!("{:?}", kind).to_lowercase()` |
| `semantic_chunk.rs` | Chunk text includes `symbol: {name}` | Surface name inside prose | Embedder tokenizes/lowercases features separately |
| `pattern.rs` native | `SearchHit.symbol` = pattern string; `language` = `Language::as_str` | Pattern text + as_str lang | |
| `pattern.rs` ast-grep parse | `SearchHit.language` from JSON | **Now** `Language::normalize_id` → as_str | Was Title Case (`"Rust"`); fixed under `amm8` |
| `search` passes | Hits carry DB surface names / languages | Surface + as_str lang | |
| `intent.rs` | Channel weight map keys | Lowercase channel names | Not symbol keys |
| `semantic_ivf.rs` / `semantic_ann.rs` | IVF fingerprint / session key | Numeric + backend string | No symbol text in fingerprint |
| `store/embed_support.rs` | Cache identity hashes `symbol_name` bytes | Surface bytes | Case-sensitive cache key (same as stored name) |
| LSP `backend.rs` / `support.rs` | WorkspaceSymbol / Capsule use hit.symbol or innermost indexed name | Surface name | Cursor fallback: identifier under byte span |
| `plugins` capsule / agent / github JSON | Pass-through `hit.symbol` / caller / callee | Surface name | No normalization |

---

## Consume sites

| Site | How symbols are compared | Contract | Divergence risk |
|------|--------------------------|----------|-----------------|
| `rank.rs` `score_symbol*` | Both sides via `normalized_symbol` | Case-insensitive; substring needs ≥2 chars (ASCII byte len / Unicode char gate) | OK vs SQL; Unicode length gate differs from byte-oriented callers |
| `query.rs` | Terms lowercased; `target` raw | Prefixed modes rely on rank/SQL for case fold | OK |
| `search/passes/symbol.rs` | `best_symbol_score` / `score_*` + `callee.to_lowercase() == primary` for GRAPH | Rank contract + explicit lowercase equality | OK |
| `store/sql.rs` filters | `lower(col)` predicates | Case-insensitive, Unicode `lower()` (SQLite) | Rust `to_lowercase` vs SQLite `lower` can disagree on rare Unicode |
| `store/sqlite.rs` `symbols_named` | `lower(s.name)=lower(?1)` | Case-insensitive exact | OK |
| `search/types.rs` `matches_lang` | Language filter | **Now** `Language::normalize_id` on both sides | Was exact string equality; broke on ast-grep `"Rust"` vs `--lang rust` |
| `semantic_ann` / embed search | Vector similarity on chunk text | Not string-equality on symbol | Symbol rename without re-embed → stale vectors (expected) |
| LSP `defs:` / `callers:` from cursor | Formats `defs:{surface}` | Relies on SQL/rank case fold | OK |
| Plugins follow-ups | `defs:{sym}` / `callers:{sym}` from hit | Surface name | OK if search was case-insensitive |

---

## Equivalence rules (intended)

1. **Definition / caller lookup:** `A` ≡ `B` iff `lower(A) == lower(B)` (Unicode lowercase), for exact match. Substring ranking additionally requires the rank-key length gate.
2. **Indexed storage** keeps the first-seen surface spelling; equivalence does not rewrite rows.
3. **Language filters:** `A` ≡ `B` iff `Language::normalize_id(A) == Language::normalize_id(B)`.
4. **Hit dedup** (`dedup_hits`) keys on kind + path + span + symbol/caller/callee **surface strings** — two hits that differ only by symbol casing are **not** merged. (Latent divergence vs rank/SQL.)
5. **IVF fingerprints** ignore symbol strings; content changes bump `data_version`.

---

## Divergences / follow-ups

| ID | Issue | Severity | Status |
|----|-------|----------|--------|
| amm8 | ast-grep `language: "Rust"` vs native `"rust"` broke `matches_lang` | High | **Fixed** this PR (`Language::normalize_id` + case-tolerant `matches_lang`) |
| dedup-case | `dedup_hits` is case-sensitive on symbol/caller/callee | Medium | Open — consider normalizing dedup keys with `normalized_symbol` |
| sql-unicode | SQLite `lower()` vs Rust `to_lowercase()` on non-ASCII idents | Low | Open — document; add goldens if non-ASCII symbols ship |
| embed-cache-case | Embed cache hashes raw `symbol_name` bytes | Low | Acceptable while storage is surface-preserving |
| single-canonical-fn | No shared `canonicalize_symbol` reused everywhere | Process | Open — introduce after dedup-case decision |

---

## Clear bugs fixed while auditing

1. **`parse_ast_grep_json`** now maps external language labels through `Language::normalize_id`.
2. **`matches_lang`** compares normalized language ids, not raw strings.

No other produce/consume mismatch rose to a definite correctness bug on this branch tip;
remaining rows are tracked as follow-ups above rather than silent behavior changes.
