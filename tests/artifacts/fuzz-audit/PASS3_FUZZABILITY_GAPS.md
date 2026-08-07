# PASS 3 — Fuzzability Gaps (Extract-Parse-Process)

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Scope:** Can high-value targets be called with `&[u8]`/`&str`/memory `Read` for a **deterministic, side-effect-free** result **as-is**?  
**Out of scope:** New harnesses, beads, production source edits.  
**Prior:** Pass 1 discovery (`PASS1_TARGET_DISCOVERY.md`); Pass 2 harness audit (`PASS2_HARNESS_HARD_RULES_AUDIT.md`).  
**Doctrine:** skill `testing-fuzzing` Fuzzability Test + `references/FUZZABILITY.md` Extract-Parse-Process.

### Fuzzability Test (applied per target)

> Can you call this function with `&[u8]` (or equivalent) and get a deterministic result with no side effects?

| Verdict | Meaning |
|---------|---------|
| **YES** | Signature already accepts bytes/str/memory reader; pure compute; harness-ready without product refactor |
| **PARTIAL** | Core is pure but private, visibility-gated, needs thin public/crate seam, or minor env/thread-local caveat that does not block a pure harness |
| **NO** | Entangled with FS / SQLite / network / wall-clock / process spawn; needs Extract-Parse-Process before high-ROI fuzz |

**Effort:** S = expose/reuse existing pure fn in harness only; M = small product seam (pub(crate)/bytes variant, inject clock); L = multi-stage split across I/O + store + compute.

**Pure harness without product refactor?** Yes means a cargo-fuzz target can depend on existing public (or same-crate via integration tricks are **not** counted) APIs today. Private pure fns count as PARTIAL + seam required unless the harness crate can call them (it cannot across crate boundaries).

---

## 1. Per-target fuzzability (priority surfaces + top ~15)

Evidence paths verified against source on 2026-08-07 (line numbers may drift slightly).

### 1.1 `ParserRegistry::parse` / tree-sitter extract

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-lang/src/lib.rs:219` → `LanguageParser::parse` → `extract::parse_and_extract_for` (`extract.rs:12`) |
| **Signature** | `pub fn parse(&self, language: Language, source: &str) -> anyhow::Result<ExtractionResult>` |
| **Fuzzable today?** | **YES** |
| **Blocking issues** | None for crash harness. Thread-local `TS_PARSERS` (`extract.rs:6–8`) reuses parsers per language (faster, still deterministic per input). Native C grammars → plan **MSan** later; not a seam blocker. `ParserRegistry::new()` is heavy — init **once** outside body (`OnceLock`). |
| **Minimal seam** | None required. Optional: `pub fn parse_source_bytes(lang: Language, source: &[u8])` that UTF-8-checks then calls `parse` (reject invalid UTF-8 without panicking). |
| **Effort / pure harness** | **S** / **yes** (no product refactor) |
| **Harness sketch** | `OnceLock<ParserRegistry>`; pick `Language` from `data[0] % N`; `let src = std::str::from_utf8(&data[1..max])`; `let _ = reg.parse(lang, src);` size-cap source (e.g. 64KiB). |

---

### 1.2 `match_pattern` / `match_literal_pattern`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-lang/src/pattern.rs:83`, `:105` |
| **Signature** | `pub fn match_pattern(lang: Language, source: &str, pattern: &str) -> anyhow::Result<Vec<PatternMatch>>`  
| | `pub fn match_literal_pattern(lang: Language, source: &str, pattern: &str) -> anyhow::Result<Vec<PatternMatch>>` |
| **Fuzzable today?** | **YES** |
| **Blocking issues** | Dual untrusted strings; tree-sitter C via `parse_source` (`pattern.rs:233`). No FS/DB. Fresh `Parser` per call (no TLS here). |
| **Minimal seam** | None. Structure-aware Arbitrary: `(Language, source: String, pattern: String)` with length caps. |
| **Effort / pure harness** | **S** / **yes** |

---

### 1.3 `classify_native`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-lang/src/pattern.rs:138` |
| **Signature** | `pub fn classify_native(pattern: &str) -> Option<NativeKind>` |
| **Fuzzable today?** | **YES** |
| **Blocking issues** | None — pure Rust string grammar (`$F`, decl prefixes, call paths). High exec/s. |
| **Minimal seam** | None. Optional oracle: `classify_native` vs `needs_ast_grep_fallback` consistency. |
| **Effort / pure harness** | **S** / **yes** |

---

### 1.4 IVF `read_header`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-core/src/semantic_ivf.rs:388` |
| **Signature** | `fn read_header(bytes: &[u8], expected_fingerprint: Option<[u8; 32]>) -> Option<IvfHeader>` (**private**) |
| **Fuzzable today?** | **PARTIAL** (pure body, **not callable** from `fuzz/` across crate) |
| **Blocking issues** | (1) **Private** — only used from `map_and_parse`. (2) Header type `IvfHeader` also private. Logic itself: magic `ASIVF\0`, VERSION=2, HEADER_SIZE=80, bounds — pure `Cursor` over `&[u8]`. |
| **Minimal seam** | ```rust
// semantic_ivf.rs — recommended
pub fn parse_ivf_header(bytes: &[u8], expected_fingerprint: Option<[u8; 32]>) -> Option<IvfHeaderView> {
    // thin re-export of read_header + pub header fields needed by fuzz/tests
}
```
Or `pub(crate)` + `#[cfg(any(test, fuzzing))]` if keeping API surface tight. Prefer **public pure parse** matching skill rule 3. |
| **Effort / pure harness** | **S–M** seam / pure harness **after** one-line visibility + optional view type |

---

### 1.5 IVF `map_and_parse` / load path

| Field | Detail |
|-------|--------|
| **Path** | `semantic_ivf.rs:323` (`map_and_parse`); public entry `load_semantic_ivf` / `load_semantic_ivf_unchecked` (`:273–281`) |
| **Signature** | `fn map_and_parse(path: &Path, expected_fingerprint: Option<[u8; 32]>) -> Result<Option<ParsedMapping>>` |
| **Fuzzable today?** | **NO** |
| **Blocking issues** | **Filesystem I/O**: open file, metadata, `map_readonly` (mmap unsafe wrapper), then pure parse of mapped bytes. Not in-process byte-driven. |
| **Minimal seam** | Extract post-map pure function: ```rust
pub fn parse_ivf_mapping(
    bytes: &[u8],
    expected_fingerprint: Option<[u8; 32]>,
) -> Option<ParsedIvfView> {
    let header = read_header(bytes, expected_fingerprint)?;
    // same bounds as map_and_parse: index_end, vector_end, alignment
    // SemanticAnnIndex::read_clusters_bounded(...);
    // bytemuck::try_cast_slice f32 check
}
```
Keep `map_and_parse` as: open → mmap → `parse_ivf_mapping(&mmap[..], …)`. |
| **Effort / pure harness** | **M** / pure harness **after** extract (no product mmap in loop) |

---

### 1.6 `SemanticAnnIndex::read_clusters_bounded`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-core/src/semantic_ann.rs:104` |
| **Signature** | `pub fn read_clusters_bounded(bytes: &[u8], k: usize, dim: usize, chunk_count: usize) -> std::io::Result<Self>` |
| **Fuzzable today?** | **YES** |
| **Blocking issues** | None for pure bytes. Fuzzer must supply structure-aware `(k, dim, chunk_count)` consistent with payload or accept Err. Cap `k*dim` and `chunk_count` to avoid OOM (e.g. k≤64, dim≤128, chunks≤4096). |
| **Minimal seam** | None. Optional custom mutator that emits valid-ish cluster layout. |
| **Effort / pure harness** | **S** / **yes** |

Also: `read_clusters_from<R: Read>` (`semantic_ann.rs:76`) is pure over memory `Cursor` — secondary YES.

---

### 1.7 `Indexer::index_content`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-core/src/index.rs:518` |
| **Signature** | `pub fn index_content(&mut self, rel_path: &str, content: &str) -> Result<FileIndexStats>` |
| **Fuzzable today?** | **NO** |
| **Blocking issues** | (1) **SQLite** via `IndexStore` (`Indexer::new` opens DB). (2) **`SystemTime::now()`** for mtime (`:519–520`) → non-determinism. (3) Hash/skip paths hit store meta. (4) Full path is I/O + parse + upsert. |
| **Minimal seam** | Split prepare from commit: ```rust
/// Pure-ish prepare: detect lang, parse, materialize rows — no DB writes.
pub fn prepare_file_content(
    rel_path: &str,
    content: &str,
    options: &IndexOptions,
    mtime: (i64, u32), // inject clock
) -> Result<PreparedFileContent> {
    // hash_content, detect_language, parsers.parse, rows_from_extraction, materialize_upsert
}
```
`index_content` becomes: prepare → `store.upsert_file`. Already private pieces: `extract_rows` (`:679`), `hash_content` (`:774`), `materialize_upsert` (`:787`), `rows_from_extraction` (`:911`) — all compute once DB removed.  
**Higher ROI shortcut:** fuzz `ParserRegistry::parse` + `split_content_lines` instead of full indexer until prepare is extracted. |
| **Effort / pure harness** | **L** full / pure harness **without** refactor: **no** (temp SQLite harness is stateful, slow, non-isolating — skill says I/O-bound is unfuzzable) |

---

### 1.8 `regex_pass` / user `Regex::new`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-core/src/search/passes/regex.rs:26` |
| **Signature** | `pub fn regex_pass(store: &IndexStore, options: &SearchOptions, parsed: &ParsedQuery) -> Result<Vec<SearchHit>>` |
| **Fuzzable today?** | **NO** for full pass; **YES** for compile boundary alone |
| **Blocking issues** | Full pass: **SQLite** (`all_indexed_lines` / trigram), **thread pool**, **wall-clock budget** (`Instant`, `ASGREP_REGEX_BUDGET_MS` env), nondeterministic scheduling. |
| **Minimal seam** | ```rust
// thin pure boundary already almost present at lines 35–40:
pub fn compile_user_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
    let re = if case_insensitive {
        Regex::new(&format!("(?i){pattern}"))
    } else {
        Regex::new(pattern)
    };
    re.map_err(|e| e.to_string())
}
// harness: let _ = compile_user_regex(...); optional re.is_match(haystack) with size caps
```
Do **not** fuzz full `regex_pass` until store is mockable; ReDoS on compile + match against capped haystack is the high-ROI surface. |
| **Effort / pure harness** | Compile: **S** (extract optional) / **yes** via direct `regex::Regex::new` even without extract. Full pass: **L** / no |

---

### 1.9 `search_pattern`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-core/src/pattern.rs:66` |
| **Signature** | `pub fn search_pattern(pattern: &str, store: &IndexStore, root: &Path, lang_filter: Option<&str>) -> Result<Vec<SearchHit>>` |
| **Fuzzable today?** | **NO** |
| **Blocking issues** | **SQLite** signature lookup; **filesystem walk** (`WalkDir` in `search_pattern_native_profiled`); **env** `ASGREP_DISABLE_AST_GREP`; optional **external binary** discovery. |
| **Minimal seam** | Already exist pure cores — **fuzz those instead of orchestrator**: `match_pattern`, `classify_native`, `cached_pattern_signatures`, `needs_ast_grep_fallback`. Optional later: ```rust
pub fn search_pattern_in_memory(
    pattern: &str,
    files: &[(Language, &str /* path */, &str /* source */)],
) -> Result<Vec<PatternMatch>>
``` |
| **Effort / pure harness** | Orchestrator **L**; pure cores **S** / **yes** without product change |

---

### 1.10 LSP `read_message` / `read_content_length`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-lsp/src/support.rs:16`, `:33` (private helper) |
| **Signature** | `pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>>` |
| **Fuzzable today?** | **YES** |
| **Blocking issues** | None — feed `std::io::Cursor<&[u8]>` or `Cursor<Vec<u8>>`. Cap: `MAX_MESSAGE_BYTES = 8MiB` already enforced; harness should still size-guard input (e.g. 64KiB–1MiB) for exec/s. Partial headers / huge Content-Length return errors — crash oracle + no panic. |
| **Minimal seam** | None. Optional pure: `pub fn parse_lsp_frame(data: &[u8]) -> io::Result<Option<String>>` wrapping `Cursor`. |
| **Effort / pure harness** | **S** / **yes** |

---

### 1.11 LSP text edits (`try_apply_text_edit` / `apply_text_edit`)

| Field | Detail |
|-------|--------|
| **Path** | `support.rs:277`, `:281` |
| **Signature** | `pub fn try_apply_text_edit(content: &str, change: &TextDocumentContentChangeEvent) -> anyhow::Result<String>`  
| | `pub fn apply_text_edit(content: &str, change: &TextDocumentContentChangeEvent) -> String` |
| **Fuzzable today?** | **YES** |
| **Blocking issues** | None. Build `TextDocumentContentChangeEvent` (`types.rs:123`) from structured Arbitrary (range / rangeLength / text). Pure UTF-16 position math via `pos_to_byte` / `utf16_char_to_byte`. |
| **Minimal seam** | None. Invariants: no panic; if `Ok`, result is valid UTF-8 (Rust `String`); length relation holds. |
| **Effort / pure harness** | **S** / **yes** |

---

### 1.12 LSP URI (`file_uri_to_path` / `uri_to_rel_path` / `pct_dec`)

| Field | Detail |
|-------|--------|
| **Path** | `support.rs:194`, `:209`; `pct_dec` private `:247` |
| **Signature** | `pub fn file_uri_to_path(uri: &str) -> anyhow::Result<PathBuf>`  
| | `pub fn uri_to_rel_path(uri: &str, root: &Path) -> anyhow::Result<String>` |
| **Fuzzable today?** | **YES** (with fixed synthetic root) |
| **Blocking issues** | Minor: platform path semantics (`file://` on macOS/Windows); not random I/O if root is a fixed `PathBuf` like `/tmp/fuzz-ws` without real FS ops — implementation uses string/path logic + traversal checks, may call `canonicalize` only in other helpers. Verify harness does not create dirs. `pct_dec` is pure but private — covered via public URI entry. |
| **Minimal seam** | Optional `pub fn pct_decode_uri_component(s: &str) -> String` for unit-focused fuzz. |
| **Effort / pure harness** | **S** / **yes** |

---

### 1.13 MCP JSON-RPC `handle_request`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-mcp/src/lib.rs:106` (`run_stdio`), `:135` (`handle_request`) |
| **Signature** | `fn handle_request(&self, request: &JsonRpcRequest) -> Option<Result<Value, Value>>` (**private**, on `McpServer`)  
| | Wire: `serde_json::from_str::<JsonRpcRequest>(&line)` (`:113`) — **`JsonRpcRequest` private** |
| **Fuzzable today?** | **NO** for full handle; **PARTIAL** for raw JSON line parse via `serde_json` only if types become public |
| **Blocking issues** | (1) Private types/methods. (2) `tools/call` path needs **Searcher/SQLite**, root confinement, path registry mutexes. (3) `run_stdio` is real stdin/stdout. |
| **Minimal seam** | ```rust
#[derive(Deserialize)]
pub struct JsonRpcRequest { pub id: Option<Value>, pub method: String, #[serde(default)] pub params: Value }

pub fn parse_jsonrpc_line(line: &str) -> Result<JsonRpcRequest, serde_json::Error> {
    serde_json::from_str(line)
}

/// Pure routing for methods that need no store (initialize / tools/list / ping / unknown).
pub fn dispatch_jsonrpc_meta(request: &JsonRpcRequest) -> Option<Result<Value, Value>> { ... }
```
Fuzz: parse + meta dispatch crash/invariants; leave `tools/call` for integration/stateful later. |
| **Effort / pure harness** | Parse+meta **M**; full MCP **L** / full pure harness **no** |

---

### 1.14 CodeMode `run_serve` / `ServeRequest` NDJSON

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-codemode/src/batch.rs:258` (`run_serve`); enum `ServeRequest` `:85` (**public**) |
| **Signature** | `pub fn run_serve(config: SessionConfig, stdin: impl BufRead, mut stdout: impl Write) -> Result<(), CallError>` |
| **Fuzzable today?** | **PARTIAL** (serde of `ServeRequest` **YES**; full serve **NO**) |
| **Blocking issues** | Full serve: sticky `CodeModeSession` → tool `call` hits **index/search/FS**. Wall-clock `Instant` in batch results (`wall_ms`). Stdin/stdout I/O (can be Cursor, but session side effects remain). |
| **Minimal seam** | Already public: `serde_json::from_str::<ServeRequest>(line)`. Recommended: ```rust
pub fn parse_serve_line(line: &str) -> Result<ServeRequest, CallError> { ... } // already inline at :267
// pure validate without session:
pub fn validate_serve_request(req: &ServeRequest) -> Result<(), CallError>
```
Do not fuzz full `run_serve` until session tools are mockable / no-op. Overlap: `BatchRequest` serde + `validate_calls` for one-shot batch. |
| **Effort / pure harness** | Serde/validate **S** / **yes**; full sticky serve **L** / **no** |

---

### 1.15 `embed_from_bytes`

| Field | Detail |
|-------|--------|
| **Path** | `crates/ast-sgrep-embed/src/lib.rs:43` |
| **Signature** | `pub fn embed_from_bytes(bytes: &[u8]) -> Result<Vec<f32>, &'static str>` |
| **Fuzzable today?** | **YES** |
| **Blocking issues** | None. Companion `embed_to_bytes` (`:40`) enables **round-trip** oracle: `embed_from_bytes(&embed_to_bytes(&v)) == Ok(v)` for finite floats. |
| **Minimal seam** | None. |
| **Effort / pure harness** | **S** / **yes** |

---

### 1.16 Already-covered baselines (Pass 1/2)

| Target | Path | Fuzzable? | Notes |
|--------|------|-----------|-------|
| `ParsedQuery::parse` | `core/query.rs:20` | **YES** | Covered by `fuzz/query_grammar` (weak oracle) |
| `score_symbol` / `fuse_rrf` | `core/rank.rs` | **YES** | Covered by `fuzz/rank` (stronger oracle) |

---

### 1.17 High-value secondary pure surfaces (validated)

| Target | Path | Verdict | Notes |
|--------|------|---------|-------|
| `cached_pattern_signatures` | `lang/signature.rs:15` | **YES** | `pub fn(pattern: &str) -> Option<Vec<String>>` |
| `structural_term_signatures` | `lang/signature.rs:88` | **YES** | `pub fn(term: &str) -> [String; 6]` |
| `split_content_lines` | `core/index.rs:30` | **YES** | Pure CRLF/LF split |
| `fts::escape_fts_term` | `core/lib.rs:40` | **YES** | Pure string escape |
| `tokenize` / `embed_text` | `embed/semantic.rs:88`, `:165` | **YES** | Pure tokenize + local embed math (no network) |
| `detect_language` | `lang/lib.rs:166` | **YES** | Path + optional content; inject synthetic path string |
| `utf16_char_to_byte` | `lsp/support.rs:264` | **YES** | Pure |
| `needs_ast_grep_fallback` | `lang/pattern.rs:47` | **YES** | Pure gate |
| `compile_glob` | `core/search/mod.rs:868` | **PARTIAL** | Pure body but **private** — seam or fuzz via public search option path |
| `glob_matches` | `core/gitignore.rs:189` | **PARTIAL** | Pure but **private**; `IgnoreMatcher::new` does **FS** read of ignore files → **NO** as whole |

---

## 2. Summary table (Pass 1 top + required priority surfaces)

| # | Target | Fuzzable? | Effort | Pure harness w/o product refactor? | Top blocker |
|---|--------|:---------:|:------:|:----------------------------------:|-------------|
| 1 | `ParserRegistry::parse` | **YES** | S | **yes** | Init registry once; native MSan later |
| 2 | `match_pattern` | **YES** | S | **yes** | Dual-input structure-aware only |
| 3 | `classify_native` | **YES** | S | **yes** | — |
| 4 | IVF `read_header` | **PARTIAL** | S–M | **no** (private) | Visibility |
| 5 | IVF `map_and_parse` | **NO** | M | **no** | Path + mmap I/O |
| 6 | `read_clusters_bounded` | **YES** | S | **yes** | Bound k/dim/chunks |
| 7 | `Indexer::index_content` | **NO** | L | **no** | SQLite + `SystemTime::now` |
| 8 | `regex_pass` | **NO** | L | **no** | Store + threads + clock |
| 8b | `Regex::new` user pattern | **YES** | S | **yes** | Direct `regex` crate / thin wrap |
| 9 | `search_pattern` | **NO** | L | **no** | Store + WalkDir + env |
| 10 | LSP `read_message` | **YES** | S | **yes** | — |
| 11 | `try_apply_text_edit` | **YES** | S | **yes** | — |
| 12 | MCP `handle_request` | **NO** | M–L | **no** | Private + Searcher I/O |
| 13 | CodeMode `run_serve` | **NO** | L | **no** | Session/tools side effects |
| 13b | `ServeRequest` serde | **YES** | S | **yes** | Public enum already |
| 14 | `embed_from_bytes` | **YES** | S | **yes** | — |
| 15 | URI helpers | **YES** | S | **yes** | Fixed synthetic root |
| — | `ParsedQuery::parse` | **YES** | — | yes (exists) | Weak oracle (Pass 2) |
| — | `score_symbol`/`fuse_rrf` | **YES** | — | yes (exists) | Size guards (Pass 2) |

---

## 3. Already fuzzable pure boundaries (ready for harness beads)

Ship these **without product refactor** (only `fuzz/` deps + crates as libraries). Ordered by Pass 1 score × readiness:

1. **`SemanticAnnIndex::read_clusters_bounded`** — custom binary IVF index body; highest unique risk among pure public APIs.  
2. **`ParserRegistry::parse`** — tree-sitter polyglot (init once).  
3. **`match_pattern` + `match_literal_pattern`** — dual untrusted pattern×source.  
4. **`classify_native`** (+ optional `needs_ast_grep_fallback`) — pure grammar, max exec/s.  
5. **LSP `read_message`** via `Cursor<&[u8]>`.  
6. **`try_apply_text_edit`** (+ `utf16_char_to_byte`).  
7. **`embed_from_bytes` / `embed_to_bytes` round-trip**.  
8. **`file_uri_to_path` / `uri_to_rel_path`** (fixed root).  
9. **`cached_pattern_signatures` / `structural_term_signatures`**.  
10. **`ServeRequest` / `BatchRequest` `serde_json::from_str`**.  
11. **User regex compile** via `regex::Regex::new` matching `regex_pass` rules (`(?i)` prefix).  
12. **`split_content_lines`**, **`fts::escape_fts_term`**, **`tokenize`**.  
13. Existing: **`ParsedQuery::parse`**, **`score_symbol`/`fuse_rrf`** (fix Pass 2 defects: size guards, corpus, oracles).

---

## 4. Requires API seam / refactor before high-ROI fuzz

| Surface | Why blocked | Recommended seam (signature) | Effort |
|---------|-------------|------------------------------|--------|
| IVF full file parse | `map_and_parse` is path+mmap; `read_header` private | `pub fn parse_ivf_bytes(bytes: &[u8], expected_fp: Option<[u8;32]>) -> Option<ParsedIvf>` combining header + cluster bounds + f32 cast checks | **M** |
| `Indexer::index_content` | SQLite + wall clock | `pub fn prepare_file_content(rel, content, opts, mtime) -> Result<PreparedFileContent>` then thin `commit_prepared` | **L** |
| `regex_pass` | Store + threads + Instant | `pub fn compile_user_regex(pattern, case_insensitive) -> Result<Regex, String>`; match harness separate with budget as soft timeout outside lib | **S** extract / skip full pass |
| `search_pattern` | Store + FS walk + env | Prefer fuzzing `match_pattern`/`classify_native`/`cached_pattern_signatures`; optional `search_pattern_in_memory(...)` later | **S** avoid / **M** optional |
| MCP `handle_request` | Private + tool I/O | Public `JsonRpcRequest` + `parse_jsonrpc_line` + pure `dispatch_jsonrpc_meta` | **M** |
| CodeMode `run_serve` | Session tools | Keep fuzzing `ServeRequest` serde; DI for `CodeModeSession` tools or no-op backend for stateful later | **L** for full |
| `IgnoreMatcher::new` | Reads `.gitignore` from disk | `pub fn IgnoreMatcher::from_rules(rules: &[str])` / `from_rule_lines(base, lines: &str)` | **M** |
| `compile_glob` | private | `pub(crate)` or `pub fn compile_glob_pattern(pattern: &str) -> Result<Regex, String>` | **S** |
| `glob_matches` | private | `pub fn glob_matches_public(pattern: &str, text: &str) -> bool` | **S** |

---

## 5. Recommended extraction patterns (concrete)

### 5.1 IVF: bytes-first load (highest-value product seam)

```rust
// crates/ast-sgrep-core/src/semantic_ivf.rs

/// Pure parse of a full IVF sidecar image (header + index + vector region checks).
/// No filesystem or mmap — safe for fuzzing and unit tests.
pub fn parse_ivf_bytes(
    bytes: &[u8],
    expected_fingerprint: Option<[u8; 32]>,
) -> Option<ParsedIvf> {
    let header = read_header(bytes, expected_fingerprint)?;
    // … copy bounds logic from map_and_parse using `bytes` instead of mmap …
    let index = SemanticAnnIndex::read_clusters_bounded(
        &bytes[HEADER_SIZE..index_end],
        header.k,
        header.dim,
        header.chunk_count,
    ).ok()?;
    bytemuck::try_cast_slice::<u8, f32>(&bytes[header.vector_offset..vector_end]).ok()?;
    Some(ParsedIvf { header, index, vector_offset: header.vector_offset, vector_end })
}

fn map_and_parse(path: &Path, expected: Option<[u8; 32]>) -> Result<Option<ParsedMapping>> {
    // open + mmap only …
    let parsed = match parse_ivf_bytes(&mmap, expected) {
        Some(p) => p,
        None => return Ok(None),
    };
    // wrap mmap + ranges into ParsedMapping
}
```

### 5.2 Indexer: inject clock + pure prepare

```rust
pub struct PreparedFileContent { /* rows, hashes, lines — no store handles */ }

pub fn prepare_file_content(
    rel_path: &str,
    content: &str,
    parsers: &ParserRegistry,
    options: &IndexOptions,
    mtime: (i64 /* secs */, u32 /* nanos */),
) -> Result<PreparedFileContent> { /* extract_rows + materialize without upsert */ }

// index_content:
//   let prepared = prepare_file_content(..., system_time_to_parts(SystemTime::now()))?;
//   self.store.upsert_file(...)
```

### 5.3 MCP: Extract-Parse-Process

```rust
pub fn parse_jsonrpc_line(line: &str) -> Result<JsonRpcRequest, serde_json::Error>;

pub fn handle_jsonrpc_meta(req: &JsonRpcRequest) -> Option<Result<Value, Value>>;
// initialize | tools/list | ping | method-not-found

// tools/call stays on McpServer (I/O)
```

### 5.4 Regex: fuzz the boundary product already uses

```rust
// harness can call regex crate mirroring regex_pass without product change:
let pattern: &str = …;
let re = if case_insensitive {
    Regex::new(&format!("(?i){pattern}"))
} else {
    Regex::new(pattern)
};
let _ = re; // or re.is_match(haystack) with MAX_HAYSTACK
```

Optional product extract only if you want one canonical API and regression sharing.

### 5.5 LSP framing: zero-copy pure wrapper (optional)

```rust
pub fn parse_lsp_message_bytes(data: &[u8]) -> io::Result<Option<String>> {
    read_message(&mut std::io::Cursor::new(data))
}
```

Not required — harness can call `read_message` directly.

---

## 6. Checked but already good (≥3)

Surfaces that **already pass** the Fuzzability Test (bytes/str + deterministic + no side effects) without refactor:

1. **`embed_from_bytes` / `embed_to_bytes`** — textbook pure codec; round-trip oracle ready (`ast-sgrep-embed/src/lib.rs:40–52`).  
2. **`classify_native`** — pure pattern grammar, no native code (`pattern.rs:138`).  
3. **`read_clusters_bounded`** — public `&[u8]` IVF index validator with full bounds/dup checks (`semantic_ann.rs:104`).  
4. **`try_apply_text_edit`** — pure editor edit algebra on `&str` + struct (`support.rs:281`).  
5. **`read_message` over `impl BufRead`** — desocketed by design; Cursor works (`support.rs:16`).  
6. **`ParsedQuery::parse` / rank helpers** — already wired into cargo-fuzz (Pass 2: improve guards/corpus, not fuzzability).  
7. **`ServeRequest` serde** — public tagged enum, pure deserialize (`batch.rs:85`).

---

## 7. Top fuzzability blockers (executive)

| Rank | Blocker | Surfaces hit | Action |
|-----:|---------|--------------|--------|
| 1 | **SQLite / IndexStore coupling** | `index_content`, `regex_pass`, `search_pattern`, MCP tools | Extract pure prepare/match; fuzz parsers not apps |
| 2 | **Filesystem path + mmap orchestration** | IVF `map_and_parse` / `load_semantic_ivf*` | Extract `parse_ivf_bytes(&[u8])`; keep I/O thin |
| 3 | **Private pure parsers** | `read_header`, `compile_glob`, `glob_matches`, MCP `JsonRpcRequest` | `pub` / `pub(crate)` pure entry points |
| 4 | **Wall clock / env / threads** | `index_content` mtime, `regex_pass` budget, serve `wall_ms` | Inject time; fuzz compile/match without budget threads |
| 5 | **Session/tool side effects** | MCP `tools/call`, CodeMode `run_serve` | Structure-aware serde first; stateful mock later |

---

## 8. Recommended investment order (fuzzability-aware)

Aligned with Pass 1 scores but **gated by YES today**:

| Priority | Action | Refactor? |
|---------:|--------|-----------|
| P0 | Harness `read_clusters_bounded` + (after tiny seam) `parse_ivf_bytes` / `read_header` | Seam for full IVF image |
| P0 | Harness `ParserRegistry::parse`, `match_pattern`, `classify_native` | No |
| P1 | Harness LSP `read_message`, `try_apply_text_edit`, URI | No |
| P1 | Harness `embed_from_bytes` round-trip | No |
| P1 | Harness user `Regex::new` (+ optional haystack match) | No (optional extract) |
| P2 | Public MCP JSON-RPC parse + meta dispatch | Small product seam |
| P2 | CodeMode `ServeRequest`/`BatchRequest` serde (+ validate) | No |
| P3 | Indexer `prepare_file_content` then pure prepare fuzz | Large seam |
| P3 | IgnoreMatcher from lines; `compile_glob` pub | Small seam |
| — | Do **not** prioritize harnesses on `search_pattern` / full `regex_pass` / full `run_serve` / full `index_content` until seams exist | — |

---

## 9. Method / evidence

- Re-read Pass 1 ranked list and required priority surfaces.  
- Verified signatures with `rg` + `sed` on source under `crates/`.  
- Applied skill Fuzzability Test (bytes, deterministic, side-effect free).  
- Did **not** implement harnesses, create beads, or edit production code.

---

*End of PASS 3 — fuzzability gaps only. Artifact: `tests/artifacts/fuzz-audit/PASS3_FUZZABILITY_GAPS.md`.*
