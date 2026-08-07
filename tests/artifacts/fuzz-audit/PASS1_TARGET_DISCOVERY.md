# PASS 1 — Fuzz Target Discovery Matrix

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Scope:** Target discovery only (no harness implementation, no beads, no production edits).  
**Doctrine:** `testing-fuzzing` skill — fuzz the narrowest untrusted input boundary; score Untrusted × Complexity × Unsafe/Native × Prior-CVE surface.

---

## 1. Method

### Commands used (from repo root)

```bash
# Public APIs accepting untrusted-ish byte/str/Read inputs
rg -n 'pub fn.*(&\[u8\]|&str|impl Read|impl BufRead)' crates --glob '**/src/**/*.rs' --type rust

# Unsafe density (note: many hits are #![forbid(unsafe_code)] comments)
rg -c 'unsafe' crates --glob '**/src/**/*.rs' --type rust | sort -t: -k2 -rn

# Parse / decode / deserialize / from_bytes / tree-sitter / mmap
rg -n 'from_bytes|from_str|decode|deserialize|Parser::|tree_sitter|mmap|read_message|parse\(' \
  crates --glob '**/src/**/*.rs' -i

# Native / FFI / deps
rg -n 'napi|extern \"C\"|tree_sitter|memmap2' crates --glob '*.{rs,toml}'
rg -n 'tree-sitter|serde|mmap|napi|memmap' crates/*/Cargo.toml Cargo.toml

# Existing fuzz baseline
cat fuzz/Cargo.toml
head fuzz/fuzz_targets/query_grammar.rs fuzz/fuzz_targets/rank.rs
```

### Inventory summary

| Signal | Result |
|--------|--------|
| Crates under `crates/` | 11 (`cli`, `codemode`, `codemode-napi`, `core`, `embed`, `lang`, `lsp`, `mcp`, `mmap`, `plugins`, `testkit`) |
| `pub fn` matches for `&[u8]` / `&str` / `Read` / `BufRead` | ~131 (includes non-parser helpers) |
| Intentional first-party `unsafe` | **1 site**: `ast-sgrep-mmap::map_readonly` wrapping `memmap2` |
| Product crates | Nearly all `#![forbid(unsafe_code)]` |
| Native C surface | tree-sitter grammars (14 langs) via `ast-sgrep-lang`; N-API in `ast-sgrep-codemode-napi` |
| Binary formats | IVF sidecar `ASIVF\0` v2 (`semantic_ivf.rs` + `semantic_ann.rs`); LE f32 embedding blobs (`embed_from_bytes`) |
| Wire protocols | LSP `Content-Length` framing; MCP JSON-RPC lines; CodeMode NDJSON serve |
| Existing fuzz targets | 2: `query_grammar`, `rank` (both on `ast-sgrep-core` only) |

### Scoring model (this pass)

| Component | Values |
|-----------|--------|
| **Untrusted?** | Y (external/user/editor/network/file), P (partial / validated-ish), N (internal only) |
| **Complexity** | 1 trivial … 5 large state / multi-stage / concurrent |
| **Unsafe/Native?** | Y if `unsafe`, FFI, tree-sitter C, mmap cast, N-API |
| **Prior CVE surface** | Ecosystem / bug-class exposure (tree-sitter, ReDoS, framing, path traversal, binary parsers) — folded into **Score** as 0–2 |
| **Score** | `U(Y=3,P=1,N=0) + Complexity + Native(Y=2,N=0) + CVE(0–2)` — range ~0–12 |

Higher score → invest first. Prefer the **validation boundary**, not post-validated helpers.

---

## 2. Full scored matrix (candidates with evidence paths)

| # | Function | Crate/path | Untrusted? | Complexity | Unsafe/Native? | Notes | Score |
|---|----------|------------|:----------:|:----------:|:--------------:|-------|------:|
| 1 | `ParserRegistry::parse` / `parse_and_extract_for` | `ast-sgrep-lang` `lib.rs:219`, `extract.rs:12` | Y | 5 | Y | Polyglot tree-sitter parse + symbol/call/import extract on user file content; 14 C grammars | **12** |
| 2 | `match_pattern` / `match_literal_pattern` | `ast-sgrep-lang` `pattern.rs:83–121` | Y | 5 | Y | Pattern + source both untrusted; structural walk + queries over tree-sitter | **12** |
| 3 | `read_header` + `map_and_parse` (IVF load) | `ast-sgrep-core` `semantic_ivf.rs:323–420` | Y | 5 | Y | Custom binary magic `ASIVF\0`, bounds, fingerprint, mmap + `bytemuck` f32 views | **12** |
| 4 | `SemanticAnnIndex::read_clusters_bounded` | `ast-sgrep-core` `semantic_ann.rs:104` | Y | 4 | Y | Structure-aware binary index body; length/member/dup checks — pure `&[u8]` boundary | **11** |
| 5 | `regex_pass` → `Regex::new` | `ast-sgrep-core` `search/passes/regex.rs:26–40` | Y | 4 | N | User `regex:` patterns; known ReDoS class; has wall-clock budget (still fuzz-worthy) | **10** |
| 6 | `read_message` / `read_content_length` | `ast-sgrep-lsp` `support.rs:14–56` | Y | 3 | N | LSP framing over `impl BufRead`; `MAX_MESSAGE_BYTES=8MiB`; UTF-8 body | **9** |
| 7 | `classify_native` | `ast-sgrep-lang` `pattern.rs:138` | Y | 4 | N | Pure-Rust structural pattern grammar (`$F`, decls, call paths); query-prefix path | **9** |
| 8 | `try_apply_text_edit` / `apply_text_edit` | `ast-sgrep-lsp` `support.rs:277–310` | Y | 4 | N | Editor-controlled ranges (UTF-16 positions/spans) into document bytes | **9** |
| 9 | MCP `serde_json::from_str` + `handle_request` | `ast-sgrep-mcp` `lib.rs:113–140` | Y | 4 | N | Untrusted stdio JSON-RPC lines → tool dispatch; method/params surface | **9** |
| 10 | `run_serve` (`ServeRequest` NDJSON) | `ast-sgrep-codemode` `batch.rs:258` | Y | 4 | N | Sticky worker: line JSON → tool call/batch; stateful session | **9** |
| 11 | `Indexer::index_content` | `ast-sgrep-core` `index.rs:518` | Y | 5 | Y | File content → lang detect → tree-sitter extract → SQLite rows; full ingest boundary | **11** |
| 12 | `search_pattern` | `ast-sgrep-core` `pattern.rs:66` | Y | 4 | Y | Orchestrates native match + optional external `ast-grep`; fail-closed paths | **10** |
| 13 | `embed_from_bytes` | `ast-sgrep-embed` `lib.rs:43` | Y | 2 | N | Sole pure `&[u8]` public decoder; DB/sidecar blobs; length % 4 check only | **7** |
| 14 | `file_uri_to_path` / `uri_to_rel_path` / `pct_dec` | `ast-sgrep-lsp` `support.rs:194–261` | Y | 3 | N | URI + percent-decode + workspace confinement; traversal checks exist | **8** |
| 15 | `utf16_char_to_byte` + `pos_to_byte` | `ast-sgrep-lsp` `support.rs:264–360` | Y | 3 | N | UTF-16 ↔ byte mapping used by edits and identifiers | **7** |
| 16 | `structural_term_signatures` / `cached_pattern_signatures` | `ast-sgrep-lang` `signature.rs:15–88` | Y | 3 | N | Pattern/term → signature strings for prefilter index | **7** |
| 17 | `IgnoreMatcher` / `glob_matches` | `ast-sgrep-core` `gitignore.rs:42–189` | Y | 3 | N | Custom gitignore-like rules from repo files; glob engine is home-grown | **7** |
| 18 | `compile_glob` | `ast-sgrep-core` `search/mod.rs:868` | Y | 2 | N | Glob → regex conversion for path filters | **6** |
| 19 | `fts::escape_fts_term` / `escape_fts_query` | `ast-sgrep-core` `lib.rs:40–50` | Y | 2 | N | User terms into FTS5 quoted queries; injection/escape boundary | **6** |
| 20 | `split_content_lines` | `ast-sgrep-core` `index.rs:30` | Y | 2 | N | CRLF/LF splitting of untrusted file bodies; high call volume | **5** |
| 21 | `tokenize` / `embed_text` (semantic) | `ast-sgrep-embed` `semantic.rs:88–165` | Y | 3 | N | Query/doc text tokenization + embedding math | **6** |
| 22 | N-API `CodeModeSession` methods | `ast-sgrep-codemode-napi` `lib.rs` (`#[napi]`) | Y | 3 | Y | JS-controlled tool args → Rust session; FFI glue | **8** |
| 23 | `map_readonly` | `ast-sgrep-mmap` `lib.rs:24` | P | 1 | Y | Sole intentional `unsafe`; thin wrapper — better fuzz **callers** (IVF load) than OS map alone | **6** |
| 24 | `ParsedQuery::parse` | `ast-sgrep-core` `query.rs:20` | Y | 3 | N | Query grammar (`callers:`, `pattern:`, `regex:`, hybrid tokenize) | **7** |
| 25 | `score_symbol` / `fuse_rrf` | `ast-sgrep-core` `rank.rs` | P | 2 | N | Ranking numerics; structure-aware input already | **4** |
| 26 | `detect_language` | `ast-sgrep-lang` `lib.rs:166` | Y | 2 | N | Path + optional content heuristics | **5** |
| 27 | `Language::parse` / `EmbedBackend::parse` / `OutputFormat::parse` / `ToolName::parse` | multi | P | 1 | N | Tiny enum/string maps — low ROI | **2** |
| 28 | `assert_sql_ident` / `escape_like_term` | `ast-sgrep-core` `store/sql.rs:88–142` | P | 2 | N | SQL identifier allowlist + LIKE escape; mostly trusted schema names | **4** |
| 29 | `read_text_capped` | `ast-sgrep-core` `io_bounds.rs:12` | Y | 2 | N | Cap at `MAX_INDEX_FILE_BYTES` (64MiB); simple I/O bound | **5** |
| 30 | `extract_identifier_at` | `ast-sgrep-lsp` `support.rs:355` | Y | 2 | N | Cursor identifier extraction | **5** |
| 31 | `run_batch` / `BatchRequest` serde | `ast-sgrep-codemode` `batch.rs:65–163` | Y | 3 | N | One-shot batch JSON (CLI/agent); overlaps serve | **7** |
| 32 | CLI batch `serde_json::from_str` | `ast-sgrep-cli` `lib.rs:~241` | Y | 2 | N | Batch requests JSON from CLI path | **5** |
| 33 | `SemanticAnnIndex::search_flat` / vector math | `ast-sgrep-core` `semantic_ann.rs:231` | P | 3 | N | Dim mismatch / NaN on query vectors; better after load validation | **5** |
| 34 | `intent::classify` | `ast-sgrep-core` `intent.rs:25` | P | 2 | N | Downstream of `ParsedQuery`; fuzz grammar first | **3** |
| 35 | `needs_ast_grep_fallback` | `ast-sgrep-lang` `pattern.rs:47` | Y | 2 | N | Cheap heuristic gate for exotic patterns | **4** |

---

## 3. Top-N ranked targets (investment order)

Recommended first ~20 for harness investment (after existing coverage).

| Rank | Target | Score | Recommended archetype | Why |
|-----:|--------|------:|----------------------|-----|
| 1 | `ParserRegistry::parse` (source × lang) | 12 | **Crash Detector** + **Grammar**/structure-aware sources; optional **Differential** across langs | Deepest native surface; indexing and pattern both depend on it |
| 2 | `match_pattern` (lang × source × pattern) | 12 | **Crash Detector** + **Structure-aware** (pair corpus: patterns + snippets) | Dual untrusted inputs; tree-sitter query/walk |
| 3 | IVF `read_header` / `map_and_parse` / load path | 12 | **Crash Detector** on crafted `ASIVF` bytes; **Round-Trip** save→load | Custom binary + mmap + casts; only unsafe-adjacent product path |
| 4 | `SemanticAnnIndex::read_clusters_bounded` | 11 | **Crash Detector** / **Custom Mutator** for u32-length clusters | Pure `&[u8]` unit boundary; easier than full file mmap |
| 5 | `Indexer::index_content` | 11 | **Crash Detector** (rel_path + content); guard size | Full ingest pipeline; catches extract→store panics |
| 6 | `regex_pass` / `Regex::new` user patterns | 10 | **Crash Detector** + timeout oracle; seed ReDoS-ish patterns | Explicit budget exists — fuzz should assert no hang beyond budget / no panic |
| 7 | `search_pattern` | 10 | **Crash Detector** + fail-closed invariant checks | Native + external fallback policy surface |
| 8 | LSP `read_message` | 9 | **Crash Detector** on `impl BufRead` / byte streams | Framing, huge Content-Length, partial headers |
| 9 | `classify_native` | 9 | **Grammar** / structure-aware strings | Fast pure-Rust; high exec/s; seeds pattern channel |
| 10 | `try_apply_text_edit` | 9 | **Crash Detector** + **Round-Trip**/invariant (no OOB, UTF-8 validity) | Classic editor-protocol bugs |
| 11 | MCP JSON-RPC line + `tools/call` params | 9 | **Structure-aware** (JSON schema) + **Stateful** session optional | Untrusted agent transport |
| 12 | CodeMode `run_serve` / `ServeRequest` | 9 | **Stateful** + structure-aware NDJSON | Sticky session; tool name/args sequencing |
| 13 | N-API tool entry (via Rust `call_tool` / session) | 8 | **Crash Detector** on args `Value` (prefer Rust side over full Node) | FFI boundary; same logical API as codemode |
| 14 | `file_uri_to_path` / `uri_to_rel_path` | 8 | **Crash Detector** + invariant (no escape outside root when root fixed) | Path/URI security |
| 15 | Pattern signatures (`structural_term_signatures` / `cached_pattern_signatures`) | 7 | **Crash Detector** + optional **Round-Trip** stability | Query-derived strings |
| 16 | `embed_from_bytes` | 7 | **Crash Detector** + **Round-Trip** with `embed_to_bytes` | Small, pure, always-on DB path |
| 17 | `IgnoreMatcher` / `glob_matches` | 7 | **Grammar**/structure-aware ignore lines | Home-grown glob |
| 18 | `fts::escape_fts_term` | 6 | **Crash Detector** + property (escaping quotes) | Injection-ish boundary into FTS5 |
| 19 | `compile_glob` | 6 | **Crash Detector** | Glob→regex compiler |
| 20 | `tokenize` / `embed_text` | 6 | **Crash Detector** | Text embedding path without network |
| 21 | `utf16_char_to_byte` | 7 | **Crash Detector** + bounds invariants | Shared by edits |
| 22 | `run_batch` / `BatchRequest` | 7 | **Structure-aware** JSON | Overlaps serve; one-shot easier than sticky |
| 23 | `split_content_lines` | 5 | **Crash Detector** (optional, low priority) | Simple splitter; volume high |
| 24 | `ParsedQuery::parse` | 7 | **Grammar** / already **Crash Detector** | **Already covered** by `query_grammar` |
| 25 | `score_symbol` / `fuse_rrf` | 4 | **Custom** invariants / already structure-aware | **Already covered** by `rank` |

---

## 4. Coverage: existing `fuzz/` vs gaps

### Already covered (baseline)

| Fuzz target | Entry | Oracle | Notes |
|-------------|-------|--------|-------|
| `fuzz/fuzz_targets/query_grammar.rs` | `ParsedQuery::parse(&str)` | Crash-only | Covers mode prefixes + hybrid tokenization only |
| `fuzz/fuzz_targets/rank.rs` | `score_symbol`, `fuse_rrf` | Numeric invariants (finite, bounds, reverse-order RRF) | Structure-aware-ish tuple input; **not** full fusion/intent |

**Crate coverage today:** `ast-sgrep-core` only (via `fuzz/Cargo.toml` dep).  
**No fuzz** for: `lang`, `lsp`, `mcp`, `embed`, `mmap`, `codemode`, `codemode-napi`, `cli`, `plugins`.

### High-value gaps (ordered)

1. **Tree-sitter parse/extract** (`ParserRegistry::parse`) — native C, every indexed file.  
2. **Pattern match channel** (`match_pattern` / `classify_native` / `search_pattern`).  
3. **IVF / ANN binary load** (`read_header`, `read_clusters_bounded`, load path) — custom format + mmap.  
4. **User regex** (`regex_pass` / `Regex::new`) — ReDoS + panic.  
5. **LSP framing + text edits + URI** (`read_message`, `try_apply_text_edit`, `file_uri_to_path`).  
6. **Agent wire formats** (MCP JSON-RPC, CodeMode NDJSON serve/batch).  
7. **`embed_from_bytes` round-trip** and semantic tokenize.  
8. **Index ingest** (`index_content` / `split_content_lines`) — needs temp DB / harness care.  
9. **Gitignore/glob** home-grown matchers.  
10. **N-API** (prefer fuzzing shared Rust `call_tool` rather than full Node process).

### Partial / shallow coverage notes

- `ParsedQuery::parse` is covered for **crashes**, not differential display vs mode constructors (`literal`/`regex`/`word`) or intent/fusion coupling.  
- `rank` covers two helpers only — not `coverage_symbol_score`, weighted RRF fusion, or `intent::classify`.  
- IVF load has strong unit validation (`read_clusters_bounded`) but **no fuzz campaign**.

---

## 5. Checked but low priority (≥3)

| Target | Path | Why low priority |
|--------|------|------------------|
| `Language::parse` / `normalize_id` | `ast-sgrep-lang/src/lib.rs:57` | Tiny string→enum table; unit tests suffice |
| `EmbedBackend::parse`, plugins `OutputFormat::parse`, `ToolName::parse` | multi | Closed vocabularies; complexity 1 |
| `env_flag::is_boolish_true` / `clamp_*` limits | `core` env/limits | Trivial predicates; no structure |
| `map_readonly` in isolation | `ast-sgrep-mmap` | OS/fs dependent; fuzz **IVF parser** that consumes mapped bytes instead |
| `assert_sql_ident` | `store/sql.rs` | Allowlist of internal table names; not user-facing wire input |
| Testkit `parse` / factory auth helpers | `ast-sgrep-testkit` | Non-product; fixtures for tests only |
| `intent::classify` alone | `intent.rs` | Validated `ParsedQuery` input; fuzz query grammar first |
| `send_response` / `write_message` | LSP support | Output encoding; low crash value vs input framing |
| CLI clap `Cli::parse` | `ast-sgrep-cli` | Rely on clap; prefer library boundaries |
| External `ast-grep` subprocess invocation | `core/pattern.rs` | Process/I/O heavy; fail-closed env already; not isolation-friendly |
| Cloud/Ollama `embed_via_*` network | `embed/embedder.rs` | Network/mock; not coverage-guided byte fuzz |
| `count_star` / SQLite status helpers | `store` | No untrusted parser surface |

---

## 6. Archetype cheat-sheet (for Pass 2+)

| Archetype | Best first targets here |
|-----------|-------------------------|
| **Crash Detector** | tree-sitter parse, match_pattern, IVF load, read_message, embed_from_bytes |
| **Round-Trip** | `embed_to_bytes`↔`embed_from_bytes`; IVF save↔load (when write path available in-process) |
| **Differential** | multi-lang parse of same snippet; native pattern vs classify gate |
| **Stateful** | CodeMode `run_serve`; MCP session tools/call sequence |
| **Grammar / structure-aware** | `classify_native`, `ParsedQuery` (extend), ServeRequest/MCP JSON, IVF mutator |
| **Custom Mutator** | IVF header+index body; LSP Content-Length framing |
| **Concurrency** | `regex_pass` thread pool + budget (TSan campaign later); not first |

---

## 7. Unsafe / native density (for prioritization)

| Location | Role |
|----------|------|
| `crates/ast-sgrep-mmap/src/lib.rs` | **Only** intentional `unsafe` (`MmapOptions::map`) |
| `crates/ast-sgrep-lang` + tree-sitter-* crates | C parsers (FFI) — highest historical CVE class in this tree |
| `crates/ast-sgrep-codemode-napi` | N-API bindgen boundary |
| `semantic_ivf` / `bytemuck` | Safe Rust casts of mmaped bytes — still treat as binary-parser risk |
| Nearly all other product crates | `#![forbid(unsafe_code)]` |

---

## 8. Suggested Pass-2 seed order (not implementing now)

1. IVF `&[u8]` load (`read_clusters_bounded` then full header).  
2. `match_pattern` + `classify_native` structure-aware.  
3. `ParserRegistry::parse` with size-capped sources (multi-lang seed corpus).  
4. LSP `read_message` + `try_apply_text_edit`.  
5. `Regex::new` / thin wrapper around pattern compile + budget.  
6. MCP/CodeMode JSON deserializers (structure-aware).  
7. `embed_from_bytes` round-trip.

---

## 9. Evidence index (primary symbols)

| Symbol | File:line (approx) |
|--------|-------------------|
| `ParsedQuery::parse` | `crates/ast-sgrep-core/src/query.rs:20` |
| `score_symbol` / `fuse_rrf` | `crates/ast-sgrep-core/src/rank.rs` |
| `Indexer::index_content` | `crates/ast-sgrep-core/src/index.rs:518` |
| `split_content_lines` | `crates/ast-sgrep-core/src/index.rs:30` |
| `search_pattern` | `crates/ast-sgrep-core/src/pattern.rs:66` |
| `regex_pass` | `crates/ast-sgrep-core/src/search/passes/regex.rs:26` |
| `read_header` / IVF | `crates/ast-sgrep-core/src/semantic_ivf.rs:388` |
| `read_clusters_bounded` | `crates/ast-sgrep-core/src/semantic_ann.rs:104` |
| `embed_from_bytes` | `crates/ast-sgrep-embed/src/lib.rs:43` |
| `ParserRegistry::parse` | `crates/ast-sgrep-lang/src/lib.rs:219` |
| `match_pattern` / `classify_native` | `crates/ast-sgrep-lang/src/pattern.rs:83,138` |
| `read_message` | `crates/ast-sgrep-lsp/src/support.rs:16` |
| `try_apply_text_edit` | `crates/ast-sgrep-lsp/src/support.rs:281` |
| `file_uri_to_path` | `crates/ast-sgrep-lsp/src/support.rs:194` |
| MCP stdio loop | `crates/ast-sgrep-mcp/src/lib.rs:113` |
| `run_serve` | `crates/ast-sgrep-codemode/src/batch.rs:258` |
| `map_readonly` | `crates/ast-sgrep-mmap/src/lib.rs:24` |
| Existing fuzz | `fuzz/fuzz_targets/query_grammar.rs`, `fuzz/fuzz_targets/rank.rs` |

---

*End of PASS 1 — discovery only. No harnesses added, no commits, no beads.*
