# PASS 5 — Corpus / Dictionary / Structure-Aware Plan

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Scope:** Seed corpora, dictionaries, structure-aware (`Arbitrary` / grammar) input design only.  
**Not in scope:** Full harness implementation, beads, production edits, commits, mass binary corpus under `fuzz/corpus/` (gitignored).  
**Builds on:** PASS1 (target matrix), PASS2 (existing harness defects D2–D4/D10), PASS3 (pure-ready surfaces), PASS4 (oracle upgrades / WP-A–C).  
**Doctrine:** skill `testing-fuzzing` + `references/CORPUS.md`, `references/DICTIONARIES.md`.

---

## 0. Policy (repo facts)

| Fact | Implication for this pass |
|------|---------------------------|
| `.gitignore:138–139` ignores `fuzz/corpus/` ("regenerable seeds") | **Do not** rely on git-tracked evolved corpus. Commit **seed recipes** + optional tiny seeds under `tests/artifacts/fuzz-audit/seeds/` or `fuzz/seed_corpus/` (later bead). |
| PASS2: no dicts, no seeds, unbounded sizes | Existing targets cold-start every machine; libFuzzer already recommended a dict for `query_grammar`. |
| CHANGELOG "~867 fuzz corpus fixtures" | Stale (PASS2 D12). Regeneration strategy must replace folklore. |
| Design-only here | Concrete seed **strings/hex layouts** documented below; optional tiny text seeds only under `tests/artifacts/fuzz-audit/seeds/` if added later. |

**Recommended layout (later implementation beads, not this pass):**

```
fuzz/
  dictionaries/           # committed .dict files (small text)
  seed_corpus/            # committed hand seeds (≥5/target) — NOT ignored
  corpus/                 # gitignored evolved corpus (cmin output, campaign merges)
  scripts/
    gen_seed_corpus.sh    # regenerates seed_corpus + optional binary seeds
    cmin_all.sh           # cargo fuzz cmin per target
tests/artifacts/fuzz-audit/seeds/   # design snapshots / recipes (this audit tree)
```

---

## 1. Structure-aware decision rubric

| Input class | Prefer | Avoid raw bytes when… |
|-------------|--------|------------------------|
| Free-form user text with **known tokens** | Raw `&str` **+ dictionary** (modes, magic headers) | Magic never reachable without dict |
| Multi-field structured (k, dim, ranks, ranges) | `Arbitrary` / libfuzzer structured types + caps | Fields correlated (length prefixes) |
| Length-prefixed binary | Structure-aware generator **or** Custom Mutator | Splice destroys u32 lengths constantly |
| Dual untrusted (pattern × source × lang) | Structured triple + caps + language enum | Independent random bytes never parse together |
| JSON / NDJSON / JSON-RPC | Serde on `&str` + JSON dict **or** typed Arbitrary → `to_string` | Random bytes rarely hit `type`/`method` tags |
| Source code (tree-sitter) | Grammar snippets per lang + dict keywords | Pure random rarely hits `fn`/`class` AST shapes |

**CMPLOG / value-profile (plateau step 2):** useful for magic `ASIVF\0`, `Content-Length:`, mode prefixes, JSON tags when still using raw bytes. Prefer dict+seeds first (cheaper than switching engines).

---

## 2. Per-target plans

Each target: minimum seed set (empty / minimal valid / typical / boundary / adversarial), dictionary tokens, structure-aware vs raw, size budget.  
**High-ROI pure-ready set** = PASS3 §3 + existing targets + PASS4 WP-A.

### 2.1 Existing: `query_grammar` (`ParsedQuery::parse`)

| Field | Plan |
|-------|------|
| **Harness today** | `fuzz_target!(|input: &str| { let _ = ParsedQuery::parse(input); })` — crash-only |
| **Structure-aware?** | **Raw `&str` OK** for free-form hybrid text. **Dictionary required** for mode prefixes (PASS2 D4). Optional upgrade: structured enum `ModePrefix + payload` for deeper mode coverage; keep one raw-bytes companion for tokenizer edge cases. |
| **Size budget** | Harness: `if input.len() > 8_192 { return; }` (PASS2: 4–64 KiB; **8 KiB** default). libFuzzer `-max_len=8192`. |

**Minimum seed set (concrete strings):**

| ID | Class | Content |
|----|-------|---------|
| `empty` | empty | `` (zero-length) |
| `ws` | boundary | `"   \t\n  "` |
| `minimal_hybrid` | minimal valid | `foo` |
| `typical_hybrid` | typical | `open file handle timeout` |
| `mode_callers` | typical | `callers:parse_header` |
| `mode_defs` | typical | `defs:SemanticAnnIndex` |
| `mode_imports` | typical | `imports:serde_json` |
| `mode_pattern` | typical | `pattern:fn $F($$$ )` |
| `mode_literal` | typical | `literal:Content-Length` |
| `mode_regex` | typical | `regex:foo.*bar` |
| `mode_word` | typical | `word:Index` |
| `stopwords` | boundary | `the a an of to` (hits STOPWORDS path) |
| `prefix_only` | boundary | `pattern:` / `regex:` / `callers:` |
| `mixed_case` | boundary | `Callers:Foo` (prefix case sensitivity) |
| `unicode` | boundary | `defs:café_résumé` / emoji `word:🚀` |
| `long_token` | boundary | `a` × 4096 |
| `adversarial_nested` | adversarial | `pattern:pattern:regex:$$$` |
| `adversarial_control` | adversarial | `"callers:\0defs:\xff"` (NUL / high bytes if fed as bytes→lossy str) |
| `adversarial_huge_prefix` | adversarial | `literal:` + 7 KiB of `.` |

**Dictionary tokens (`fuzz/dictionaries/query_grammar.dict`):**

```
# mode prefixes (critical — libFuzzer already recommended these)
pq_callers="callers:"
pq_defs="defs:"
pq_imports="imports:"
pq_pattern="pattern:"
pq_literal="literal:"
pq_regex="regex:"
pq_word="word:"
# structural pattern helpers (when payload is pattern:)
pq_fn="fn "
pq_def="def "
pq_class="class "
pq_dollar="$"
pq_ddd="$$$"
pq_call="$F($$$)"
# common symbols from product
pq_open="("
pq_close=")"
pq_space=" "
pq_colon=":"
```

**Plateau:** if hybrid-only coverage after 30m with dict → switch to structure-aware `enum Mode { Hybrid(String), Callers(String), … }` Arbitrary with cap 256 payload chars.

---

### 2.2 Existing: `rank` (`score_symbol` + `fuse_rrf`)

| Field | Plan |
|-------|------|
| **Harness today** | Structured `(&str, &str, Vec<usize>)` + finite/range/reverse-RRF oracle (PASS2 **C+**, PASS4 "already strong") |
| **Structure-aware?** | **Already adequate** for shape. Need **value caps**, not grammar. Dict **low value** (string equality / substring). |
| **Size budget** | `term.len() ≤ 256`, `symbol.len() ≤ 512`, `ranks.len() ≤ 64` (PASS2 suggested ≤256; **64** keeps exec/s high). Drop ranks > 1_000_000 or map via `% 1024`. |

**Minimum seed set (logical; encode as libFuzzer corpus via small generator or fixed bin files later):**

| ID | Class | `(term, symbol, ranks)` |
|----|-------|-------------------------|
| `empty_empty_empty` | empty | `("", "", [])` |
| `exact` | minimal valid | `("foo", "foo", [0])` |
| `substring` | typical | `("parse", "parse_header", [0, 1, 2])` |
| `no_match` | typical | `("zzz", "aaa", [5, 1, 3])` |
| `case` | boundary | `("Foo", "foo", [0])` |
| `short_sub` | boundary | `("a", "ab", [0])` (MIN_SUBSTRING_SYMBOL_CHARS=2) |
| `unicode` | boundary | `("café", "café_bar", [0, 1])` |
| `ranks_dup` | boundary | `("x", "x", [0, 0, 0])` |
| `ranks_rev` | adversarial | `("t", "t", [0,1,2,…,63])` then reverse path exercises oracle |
| `huge_rank_vals` | adversarial | ranks with `usize::MAX`, `0`, alternating (must clamp in harness) |
| `long_symbol` | boundary | term `"a"`, symbol length 512 of `"b"` |

**Dictionary:** optional short idents only (`parse`, `index`, `search`, `_`, `::`) — **priority P3**.

**Checked adequate:** structure-aware tuple + reverse-RRF oracle already strong (PASS4 §7.1). Gaps = guards + seeds + CI, not dict/grammar.

---

### 2.3 Pure-ready P0: `SemanticAnnIndex::read_clusters_bounded`

| Field | Plan |
|-------|------|
| **Input** | `(bytes, k, dim, chunk_count)` — k/dim must match leading u32 and centroid layout |
| **Structure-aware?** | **Required.** Raw bytes alone almost never satisfy: `k` prefix, `k*dim` f32s, second `k`, k length-prefixed member lists, partition of `{0..chunk_count-1}` no dups, exact consumption. |
| **Size budget** | `k ≤ 8`, `dim ≤ 32`, `chunk_count ≤ 64`; payload ≤ `4 + k*dim*4 + 4 + sum(4+4*len) ≤ 16 KiB`. Reject overflow before call. |

**Binary layout (from `semantic_ann.rs:104–160`):**

```
u32 k
  (f32 LE × dim) × k          # centroids
u32 k                         # must equal k again
  for each of k clusters:
    u32 len
    u32 member_idx × len      # permutation of 0..chunk_count-1, full cover
```

**Minimum seed set (describe bytes; generate with Python/`write_to` in seed script):**

| ID | Class | Construction |
|----|-------|--------------|
| `empty` | empty | `b""` with `(k=1,dim=1,chunk=1)` → Err |
| `minimal_valid` | minimal valid | k=1, dim=1, chunk=1: centroid `[0.0]`, cluster `[0]` |
| `typical_k2` | typical | k=2, dim=4, chunk=4: two centroids, partition e.g. `[0,1]` / `[2,3]` |
| `boundary_k_eq_chunk` | boundary | k=chunk_count, each cluster length 1 |
| `boundary_one_cluster_all` | boundary | k=1, one cluster with all indices 0..n-1 |
| `adversarial_dup_member` | adversarial | valid shape but member `0` twice |
| `adversarial_oob_member` | adversarial | member `== chunk_count` |
| `adversarial_short` | adversarial | truncate last u32 |
| `adversarial_k_mismatch` | adversarial | first u32=2 but second cluster-count u32=1 |
| `adversarial_extra_tail` | adversarial | valid body + trailing `0x00` (offset != len check) |
| `adversarial_nan_centroid` | adversarial | centroid bits `0x7fc00000` (NaN) — parser may accept; search oracle separate |
| `rt_from_write_to` | typical | `SemanticAnnIndex::write_to` on tiny built index (best valid seed source) |

**Dictionary (binary — hex form for libFuzzer dict):**

```
ann_u32_0="\x00\x00\x00\x00"
ann_u32_1="\x01\x00\x00\x00"
ann_u32_2="\x02\x00\x00\x00"
ann_f32_0="\x00\x00\x00\x00"
ann_f32_1="\x00\x00\x80\x3f"
ann_f32_neg1="\x00\x00\x80\xbf"
ann_nan="\x00\x00\xc0\x7f"
ann_inf="\x00\x00\x80\x7f"
```

**Custom mutator (PASS4 WP-C, later):** mutate centroid floats in place; swap members within/between clusters with repair; resize one cluster and fix partition. **Do not** rely on bit flips alone after plateau.

---

### 2.4 Pure-ready P0: `ParserRegistry::parse` (lang × source)

| Field | Plan |
|-------|------|
| **Structure-aware?** | **Yes:** `(Language, source: String)` with enum of 14 langs. Raw source alone wastes cycles on wrong grammar pairing if lang random from bytes. |
| **Size budget** | source ≤ **4 KiB** for default campaign; weekly deep run ≤ 64 KiB. Init registry **once** (static/`OnceLock`). |

**Languages (all):** Rust, TypeScript, JavaScript, Python, Go, Java, CSharp, Ruby, Swift, C, Cpp, Kotlin, Php (+ check `Language::all()` for completeness).

**Minimum seed set (source snippets — document; seed file names = `rust_fn.rs.txt` etc.):**

| ID | Class | Lang | Source (abbreviated) |
|----|-------|------|----------------------|
| `empty` | empty | any | `""` |
| `ws` | boundary | Rust | `"\n\n"` |
| `rust_fn` | minimal valid | Rust | `fn main() {}` |
| `rust_struct` | typical | Rust | `pub struct Foo { x: u32 }\nimpl Foo { fn bar(&self) {} }` |
| `ts_class` | typical | TS | `export class C { m(x: number): void {} }` |
| `js_arrow` | typical | JS | `const f = (a) => a + 1;` |
| `py_def` | typical | Python | `def foo(x):\n    return x\nclass C:\n    pass\n` |
| `go_func` | typical | Go | `package p\nfunc F(x int) int { return x }` |
| `java_class` | typical | Java | `class C { void m() {} }` |
| `cs_class` | typical | CSharp | `namespace N { class C { void M() {} } }` |
| `c_fn` | typical | C | `int f(int x) { return x; }` |
| `cpp_template` | boundary | Cpp | `template<typename T> T id(T x) { return x; }` |
| `unclosed` | adversarial | Rust | `fn main() {` |
| `nul` | adversarial | any | `"a\0b"` |
| `invalid_utf8` | adversarial | — | only if harness takes `&[u8]`; else skip (str harness) |
| `deep_nest` | adversarial | JS | 200× nested `{` then close |
| `mixed_lang` | adversarial | Python source with Rust `fn` | exercises wrong-lang parse resilience |
| `huge_line` | boundary | any | single line 4 KiB |

**Dictionary (`tree_sitter_source.dict` shared with match_pattern):**

```
ts_fn="fn "
ts_def="def "
ts_class="class "
ts_struct="struct "
ts_impl="impl "
ts_func="function "
ts_func2="func "
ts_pub="pub "
ts_return="return "
ts_import="import "
ts_package="package "
ts_namespace="namespace "
ts_brace_o="{"
ts_brace_c="}"
ts_paren_o="("
ts_paren_c=")"
ts_semi=";"
ts_arrow="=>"
ts_colon_colon="::"
```

**Plateau:** coverage stuck outside extractors → seeds with **imports/calls/symbols** per lang (from unit tests under `crates/ast-sgrep-lang`); enable MSan campaign shared corpus (PASS2 rule 6).

---

### 2.5 Pure-ready P0: `match_pattern` / `match_literal_pattern`

| Field | Plan |
|-------|------|
| **Structure-aware?** | **Required dual input:** `(Language, source, pattern)` with caps. Pair corpus: same-lang snippet + matching pattern. |
| **Size budget** | source ≤ 2 KiB, pattern ≤ 256 bytes. |

**Minimum seed pairs:**

| ID | Class | Lang | Source | Pattern |
|----|-------|------|--------|---------|
| `empty_pat` | empty | Rust | `fn f(){}` | `""` |
| `fn_meta` | minimal valid | Rust | `fn foo() {}` | `fn $F($$$ )` or `fn $F()` per native grammar |
| `fn_named` | typical | Rust | `fn bar() {}` | `fn bar()` |
| `class_C` | typical | Python | `class C:\n  pass\n` | `class $C` / `class C` |
| `call_simple` | typical | Rust | `fn m(){ foo(); }` | `foo($$$ )` |
| `call_method` | typical | TS | `obj.m(1)` | `$O.$M($$$ )` |
| `literal_substr` | typical | any | `hello world` | `hello` (literal path) |
| `no_match` | boundary | Rust | `fn a(){}` | `fn b()` |
| `exotic_fallback` | adversarial | Rust | `fn a(){}` | pattern with `$` + structure that `classify_native` rejects (e.g. complex `{…}` rule) — exercises `needs_ast_grep_fallback` **without** spawning if harness only calls native match |
| `dollar_garbage` | adversarial | Rust | `fn a(){}` | `$$$word<<<` (native match-none) |
| `unbalanced` | adversarial | any | `fn a(){}` | `fn $F(` |

**Dictionary:** merge `query_grammar` structural tokens + `tree_sitter_source.dict` +:

```
pat_fn_meta="fn $F"
pat_class_meta="class $C"
pat_struct_meta="struct $S"
pat_call_meta="$F($$$)"
pat_method="$O.$M($$$)"
pat_ddd="$$$"
pat_dollar="$"
```

---

### 2.6 Pure-ready: `classify_native` (+ `needs_ast_grep_fallback`)

| Field | Plan |
|-------|------|
| **Structure-aware?** | Raw `&str` **+ strong dict** sufficient (pure grammar, max exec/s). Optional grammar Arbitrary for `Decl`/`Call` shapes. |
| **Size budget** | pattern ≤ **512** bytes. |

**Seeds:**

| ID | Class | Pattern |
|----|-------|---------|
| `empty` | empty | `""` |
| `fn_named` | minimal | `fn foo` |
| `fn_meta` | typical | `fn $F` |
| `def_py` | typical | `def bar` |
| `function_js` | typical | `function baz` |
| `func_go` | typical | `func Main` |
| `class_C` | typical | `class $C` |
| `struct_S` | typical | `struct Foo` |
| `interface_I` | typical | `interface I` |
| `type_T` | typical | `type T` |
| `call_plain` | typical | `foo($$$ )` |
| `call_path` | typical | `a.b.c($$$ )` |
| `call_meta` | typical | `$O.$M($$$ )` |
| `call_global` | typical | `::foo($$$ )` |
| `prefix_only` | boundary | `fn ` / `class ` |
| `bad_ident` | boundary | `fn 123` |
| `trailing_junk` | boundary | `foo($$$ )xx` |
| `args_bad` | adversarial | `foo(1,2)` |
| `nested_dollar` | adversarial | `fn $F($$$x, $Y)` |
| `fallback_shape` | adversarial | `$A { $B }` or similar with structure chars but not classify_native |

**Dictionary:** same as pattern structural tokens (`fn `, `def `, `class `, `$F`, `$$$`, `.`, `(`, `)`).

---

### 2.7 Pure-ready: IVF full image / `read_header` (after seam `parse_ivf_bytes`)

| Field | Plan |
|-------|------|
| **Today** | `read_header` **private**; plan seeds for post-seam harness (PASS3 §5.1, PASS4 WP-C/D). |
| **Structure-aware?** | **Required** for valid images. Magic `ASIVF\0` (6) + VERSION=2 LE u32 + header_size=80 u16 + fields. |
| **Size budget** | header always 80 bytes; full image harness: vectors region capped e.g. `chunk_count*dim*4 ≤ 64 KiB`, file ≤ **128 KiB**. |

**Header layout (80 bytes, LE):**

```
[0..6)   magic b"ASIVF\0"
[6..10)  version u32 = 2
[10..12) header_size u16 = 80
[12..20) chunk_count u64
[20..24) dim u32
[24..56) fingerprint [u8;32]
[56..60) k u32
[60..68) index_len u64
[68..76) vector_offset u64
[76..80) reserved [0;4]
```

Constraints from `read_header`: chunk_count>0, dim>0, k>0, k≤256, k≤chunk_count, reserved zeros; optional fingerprint match.

**Seeds:**

| ID | Class | Notes |
|----|-------|-------|
| `empty` | empty | 0 bytes |
| `short79` | boundary | 79 zero bytes |
| `magic_only` | minimal invalid | `ASIVF\0` + zeros |
| `valid_header_min` | minimal valid header | magic+ver+80 + chunk=1,dim=1,k=1, fp=0, index_len=0, vec_off=4096, reserved 0 — may fail later body checks |
| `bad_magic` | adversarial | `ASIVFx` / `BSIVF\0` |
| `bad_version` | adversarial | version=1 or 3 |
| `bad_hdr_size` | adversarial | header_size=64 |
| `k_gt_chunk` | adversarial | k=2, chunk=1 |
| `k_gt_256` | adversarial | k=257 |
| `nonzero_reserved` | adversarial | last 4 bytes nonzero |
| `fp_mismatch` | adversarial | expected_fp Some([1;32]) vs header zeros |
| `rt_from_save` | typical | bytes produced by `save_semantic_ivf*` on tiny index (unit test path) — **best seed** |

**Dictionary:**

```
ivf_magic="ASIVF\x00"
ivf_ver2="\x02\x00\x00\x00"
ivf_hdr80="\x50\x00"
ivf_align_4k="\x00\x10\x00\x00\x00\x00\x00\x00"
```

**Custom mutator:** preserve magic/version; mutate dim/k/chunk with consistency repair; flip fingerprint bits; adjust index_len/vector_offset with bounds.

---

### 2.8 Pure-ready: `embed_from_bytes` / `embed_to_bytes` (round-trip)

| Field | Plan |
|-------|------|
| **Structure-aware?** | Raw `&[u8]` **adequate** (length % 4). RT oracle: `from(to(v)) == v` for finite floats; or `to(from(b))?` length. |
| **Size budget** | bytes ≤ **4096** (1024 f32s); prefer dim ∈ {1, 4, 32, 384, 768} as seed dims. |

**Seeds:**

| ID | Class | Bytes |
|----|-------|-------|
| `empty` | empty / minimal valid | `[]` → Ok(vec![]) |
| `one_f32_0` | minimal | `00 00 00 00` |
| `one_f32_1` | typical | `00 00 80 3f` (1.0) |
| `dim4` | typical | 16 bytes of sequential floats |
| `len_mod_1` | boundary | 1 byte |
| `len_mod_3` | boundary | 3 bytes |
| `nan` | adversarial | `00 00 c0 7f` |
| `inf` | adversarial | `00 00 80 7f` |
| `neg_zero` | boundary | `00 00 00 80` |
| `max_f32` | boundary | `ff ff 7f 7f` |

**Dictionary:** f32 bit patterns as in ANN dict. Priority **P2** (simple format; seeds alone go far).

---

### 2.9 Pure-ready: LSP `read_message` / Content-Length frames

| Field | Plan |
|-------|------|
| **Structure-aware?** | **Preferred:** Arbitrary body string → encode `Content-Length: {n}\r\n\r\n{body}`. Companion raw-bytes harness for header chaos. |
| **Size budget** | body ≤ **64 KiB** (product max 8 MiB — do **not** fuzz near 8 MiB). Multi-message streams ≤ 4 frames. |

**Seeds:**

| ID | Class | Bytes (escaped) |
|----|-------|-----------------|
| `empty` | empty | `` |
| `minimal_json` | minimal valid | `Content-Length: 2\r\n\r\n{}` |
| `typical_init` | typical | CL + `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}` |
| `extra_headers` | typical | `Content-Length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}` |
| `crlf_only` | boundary | `\r\n\r\n` incomplete |
| `missing_cl` | adversarial | `Content-Type: x\r\n\r\n{}` |
| `bad_cl` | adversarial | `Content-Length: abc\r\n\r\n` |
| `cl_zero` | boundary | `Content-Length: 0\r\n\r\n` |
| `cl_too_big` | adversarial | `Content-Length: 999999999\r\n\r\n` (hits MAX check) |
| `cl_short_body` | adversarial | CL=100 but body 2 bytes (blocking read / error path) |
| `partial_header` | boundary | `Content-Length: 2\r\n` no blank line |
| `utf8_body` | typical | body with multi-byte UTF-8 |
| `two_messages` | typical | frame{}frame{} for stream harness |

**Dictionary (`lsp_frame.dict`):**

```
lsp_cl="Content-Length: "
lsp_ct="Content-Type: "
lsp_crlf="\x0d\x0a"
lsp_crlf2="\x0d\x0a\x0d\x0a"
lsp_jsonrpc="\"jsonrpc\""
lsp_20="\"2.0\""
lsp_method="\"method\""
lsp_params="\"params\""
lsp_id="\"id\""
lsp_initialize="initialize"
lsp_didopen="textDocument/didOpen"
lsp_completion="textDocument/completion"
```

**Round-trip oracle (PASS4):** `write_message` then `read_message` recovers body.

---

### 2.10 Pure-ready: `try_apply_text_edit` + `utf16_char_to_byte`

| Field | Plan |
|-------|------|
| **Structure-aware?** | **Required:** `(doc: String, start_line, start_col, end_line, end_col, replacement)` with ranges derived from doc UTF-16 lengths (clamp in Arbitrary). |
| **Size budget** | doc ≤ 2 KiB; replacement ≤ 512. |

**Seeds:**

| ID | Class | Doc / edit |
|----|-------|------------|
| `empty_doc` | empty | `""`, insert `"a"` at 0,0 |
| `identity` | minimal | `"hello"`, range covering nothing / empty replacement at mid |
| `replace_all` | typical | `"abc"`, range full → `"XYZ"` |
| `unicode_emoji` | boundary | `"a🚀b"`, edit around emoji (UTF-16 surrogate pairs) |
| `crlf` | boundary | `"a\r\nb"`, edit line 1 |
| `oob_line` | adversarial | start_line >> line count |
| `inverted_range` | adversarial | end before start |
| `utf16_mid_surrogate` | adversarial | col pointing into high/low surrogate |

**Dictionary:** low value; include `\r\n`, `\n`, space. Priority **P3**.

---

### 2.11 Pure-ready: URI helpers (`file_uri_to_path` / `uri_to_rel_path` / `pct_dec`)

| Field | Plan |
|-------|------|
| **Structure-aware?** | Raw `&str` + **URI dict**. Fixed synthetic root `/tmp/fuzz-ws` (or `C:\fuzz-ws` on Windows notes). |
| **Size budget** | uri ≤ 2 KiB. |

**Seeds:**

| ID | Class | URI / path |
|----|-------|------------|
| `empty` | empty | `""` |
| `file_simple` | minimal | `file:///tmp/fuzz-ws/src/main.rs` |
| `rel_ok` | typical | under fixed root |
| `pct_space` | typical | `file:///tmp/fuzz-ws/a%20b.rs` |
| `dotdot` | adversarial | `file:///tmp/fuzz-ws/../etc/passwd` |
| `dotdot_pct` | adversarial | `%2e%2e%2f` sequences |
| `double_encode` | adversarial | `%252e` |
| `no_file_scheme` | boundary | `https://evil/x` |
| `unc_windows` | boundary | platform-specific if macOS skip |
| `long_pct` | boundary | many `%41` |

**Dictionary:**

```
uri_file="file://"
uri_file3="file:///"
uri_tmp="/tmp/fuzz-ws/"
uri_pct20="%20"
uri_pct2e="%2e"
uri_pct2f="%2f"
uri_dotdot="../"
uri_slash="/"
```

---

### 2.12 Pure-ready: JSON-RPC (MCP) — parse-only seam

| Field | Plan |
|-------|------|
| **Today** | Full `handle_request` blocked (PASS3); plan for `serde_json::from_str` / future `parse_jsonrpc_line`. |
| **Structure-aware?** | Typed Arbitrary → JSON **or** raw + JSON dict + method dict. |
| **Size budget** | line ≤ **8 KiB**. |

**Seeds:**

| ID | Class | Line |
|----|-------|------|
| `empty` | empty | `` |
| `minimal` | minimal | `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}` |
| `tools_list` | typical | method `tools/list` |
| `tools_call` | typical | `tools/call` + `{"name":"search","arguments":{"query":"foo"}}` |
| `notify_no_id` | boundary | no `id` (notification) |
| `bad_method` | adversarial | `method":"not/a/method"` |
| `huge_params` | boundary | large params object within cap |
| `not_json` | adversarial | `{{{{` |
| `wrong_version` | boundary | `"jsonrpc":"1.0"` |

**Dictionary:** JSON base + 

```
rpc_jsonrpc="\"jsonrpc\""
rpc_20="\"2.0\""
rpc_method="\"method\""
rpc_params="\"params\""
rpc_id="\"id\""
rpc_init="initialize"
rpc_tools_list="tools/list"
rpc_tools_call="tools/call"
rpc_name="\"name\""
rpc_args="\"arguments\""
```

---

### 2.13 Pure-ready: NDJSON `ServeRequest` serde

| Field | Plan |
|-------|------|
| **Structure-aware?** | Serde on line + dict; optional enum Arbitrary. |
| **Size budget** | line ≤ 8 KiB; batch `calls` ≤ **32** (`MAX_BATCH_CALLS`). |

**Seeds:**

| ID | Class | JSON |
|----|-------|------|
| `empty` | empty | `` |
| `end` | minimal valid | `{"type":"end"}` |
| `call_search` | typical | `{"type":"call","id":"1","tool":"search","args":{"query":"foo"}}` |
| `call_index_status` | typical | tool `index_status` |
| `batch_serial` | typical | `{"type":"batch","id":"b1","calls":[{"tool":"search","args":{}}],"parallel_mode":"serial"}` |
| `batch_max` | boundary | 32 calls |
| `batch_over` | adversarial | 33 calls (validate path if present) |
| `unknown_type` | adversarial | `{"type":"explode"}` |
| `missing_id` | boundary | call without id |
| `tool_unknown` | typical | tool `"nope"` |

**Tool name tokens for dict:** `search`, `semantic`, `chain`, `defs`, `callers`, `imports`, `index_status`, `index_repo`, `filter_hits`, `select`, `catalog_search`, `catalog_describe`.

```
srv_type="\"type\""
srv_call="\"call\""
srv_batch="\"batch\""
srv_end="\"end\""
srv_id="\"id\""
srv_tool="\"tool\""
srv_args="\"args\""
srv_pm="\"parallel_mode\""
srv_serial="\"serial\""
srv_parallel="\"parallel\""
srv_auto="\"auto\""
```

---

### 2.14 Pure-ready: user regex compile (`Regex::new` / optional `(?i)` prefix)

| Field | Plan |
|-------|------|
| **Structure-aware?** | Raw pattern + **regex dict**. Separate haystack match campaign with timeout (not in-lib). |
| **Size budget** | pattern ≤ **256**; never unbounded ReDoS fodder without wall timeout in harness process (`-timeout=2` ~ product 2s budget). |

**Seeds:**

| ID | Class | Pattern |
|----|-------|---------|
| `empty` | empty | `""` |
| `literal` | minimal | `foo` |
| `dotstar` | typical | `a.*b` |
| `case_flag` | typical | harness applies `(?i)` like product |
| `classes` | typical | `[a-zA-Z_][a-zA-Z0-9_]*` |
| `bad_paren` | boundary | `(abc` |
| `nested` | adversarial | `(` × 50 + `)` × 50 |
| `redos_classic` | adversarial | `(a+)+$` with careful timeout oracle |
| `unicode` | boundary | `\p{L}+` if supported |

**Dictionary:** regex metacharacters `( ) [ ] { } * + ? . | ^ $ \d \w \s`.

---

### 2.15 Secondary pure surfaces (compressed plans)

| Target | Seeds (min 5 classes) | Dict | Structure? | Max len |
|--------|----------------------|------|------------|---------|
| `cached_pattern_signatures` / `structural_term_signatures` | empty; `fn foo`; `class $C`; `foo($$$ )`; garbage `$` | pattern dict | raw+dict | 512 |
| `split_content_lines` | empty; `a\nb`; `a\r\nb`; no trailing nl; `\r` only; huge line | `\r\n`,`\n` | raw | 8 KiB |
| `fts::escape_fts_term` | empty; `foo`; `"quote"`; `*`; `AND OR`; unicode | `"` `*` | raw | 512 |
| `tokenize` / `embed_text` | empty; english sentence; code snippet; unicode; long 2 KiB | optional stopwords | raw | 2 KiB |
| `detect_language` | path `x.rs` / `x.py` / no ext + content sniffs | extensions `.rs` `.ts`… | `(path, content?)` struct | path 256 / content 1 KiB |
| ANN `write_to`→`read` RT / differential | build tiny index in Arbitrary (not corpus files) | N/A | **full Arbitrary** | k≤8 dim≤32 n≤64 |
| `fuse_rrf` alone | already under rank | — | ranks vec | len≤64 |

---

## 3. Format-specific master plans

### 3.1 IVF binary (`ASIVF\0` v2)

| Item | Spec |
|------|------|
| Magic / version | `ASIVF\0`, u32 LE **2**, header size **80**, vector align **4096** |
| Body | index bytes (ANN cluster encoding) + padding to `vector_offset` + LE f32 matrix `chunk_count * dim` |
| Seed strategy | (1) unit-test round-trip files regenerated by `gen_seed_corpus.sh` via public save API; (2) hand-crafted header mutants; (3) after seam, full `parse_ivf_bytes` |
| Structure | Custom generator mandatory for **valid** path; raw+dict for reject path |
| Dict | magic, ver2, hdr80, 4K offset patterns |
| Mutator | preserve magic; repair k≤chunk; recompute index_len optional |

### 3.2 LE f32 embeddings

| Item | Spec |
|------|------|
| Format | packed `f32::to_le_bytes` only; reject `len % 4 != 0` |
| Seeds | empty, 1 float, dim packings, NaN/Inf, odd lengths |
| Structure | raw bytes OK; RT oracle structure-aware on `Vec<f32>` with finite filter |
| Budget | 4 KiB default |

### 3.3 LSP Content-Length frames

| Item | Spec |
|------|------|
| Framing | headers until `\r\n\r\n`, require `Content-Length:`, body exact length, max 8 MiB |
| Seeds | §2.9 table |
| Structure | encode-from-body Arbitrary + raw chaos companion |
| Dict | `Content-Length:`, CRLF, JSON-RPC method names |
| Plateau | CMPLOG helps numeric length fields if still raw |

### 3.4 JSON-RPC (MCP)

| Item | Spec |
|------|------|
| Shape | `jsonrpc`/`id`/`method`/`params` |
| Methods of interest | `initialize`, `tools/list`, `tools/call` |
| Seeds | §2.12 |
| Structure | serde typed Arbitrary preferred for meta-dispatch seam; raw for parse crashes |
| Dict | JSON + method names |

### 3.5 NDJSON `ServeRequest`

| Item | Spec |
|------|------|
| Tags | `type`: `call` \| `batch` \| `end` |
| Seeds | §2.13 |
| Structure | enum Arbitrary → `serde_json::to_string` |
| Dict | type tags + tool catalog names |
| Cap | batch ≤ 32 calls |

### 3.6 Tree-sitter source snippets (14 languages)

| Item | Spec |
|------|------|
| Goal | hit parse + extract paths (functions, classes, calls, imports) |
| Seeds | ≥1 minimal compile unit per language + adversarial unclosed/deep nest |
| Structure | `(Language, source)` |
| Dict | language keywords shared |
| Corpus growth | distill from `crates/**/tests/**` and small fixtures; **PII N/A** (code only) |
| Do not | pull multi-MB real repos into seed_corpus |

### 3.7 Structural patterns (`$F`, `class $C`, …)

| Item | Spec |
|------|------|
| Native shapes | decl prefixes `fn/def/function/func/class/struct/interface/type` + call paths with `$` / `$$$` |
| Seeds | §2.5–2.6 |
| Structure | for `match_pattern`, **paired** with source that can match |
| Dict | metavars + decl keywords + `($$$)` |
| Fallback patterns | keep rare exotic seeds for gate fuzz only; do not require external ast-grep in default CI |

---

## 4. Corpus minimization / regeneration strategy

Because **`fuzz/corpus/` is gitignored**, the org needs a **reproducible seed path** + **optional artifact cache**, not tribal evolved corpora.

### 4.1 Three layers

| Layer | Location | Git? | Contents |
|-------|----------|------|----------|
| **L0 Recipes** | this file + `fuzz/scripts/gen_seed_corpus.sh` | yes | How to build every seed |
| **L1 Seed corpus** | `fuzz/seed_corpus/<target>/` (recommended) or `tests/artifacts/fuzz-audit/seeds/<target>/` | yes (tiny text/hex only) | ≥5–20 hand seeds per target |
| **L2 Evolved corpus** | `fuzz/corpus/<target>/` | **no** | cmin output, campaign merges, sanitizer shared |

### 4.2 Regeneration commands (design)

```bash
# 1) Generate L1 from recipes (Python/Rust bin — bead later)
./fuzz/scripts/gen_seed_corpus.sh

# 2) Bootstrap L2 from L1
for t in query_grammar rank; do
  mkdir -p "fuzz/corpus/$t"
  cp -f "fuzz/seed_corpus/$t/"* "fuzz/corpus/$t/" 2>/dev/null || true
done

# 3) Campaign
cargo fuzz run query_grammar -- -max_len=8192 -dict=fuzz/dictionaries/query_grammar.dict

# 4) Minimize weekly (or after any long campaign)
cargo fuzz cmin query_grammar
cargo fuzz cmin rank
# future targets...

# 5) Crash minimize before filing
cargo fuzz tmin query_grammar fuzz/artifacts/query_grammar/crash-*

# 6) Merge sanitizer corpora
cargo fuzz cmin query_grammar -- fuzz/corpus_msan/query_grammar/
```

### 4.3 Binary seeds (IVF / ANN)

- Prefer **generator** calling `write_to` / save IVF over checking in large binaries.
- If a golden binary is needed for CI regression: keep **one** minimized valid file under `fuzz/seed_corpus/read_clusters/` (< 4 KiB) with hex dump also in this doc.

### 4.4 CI policy (ties PASS2)

| Job | Corpus |
|-----|--------|
| PR | L1 seeds only + `-runs=N` or short max_total_time; deterministic |
| Nightly / dispatch | L2 evolved if present on runner cache; else L1 |
| Release gate | L1 + any crash regression fixtures under `tests/**/fuzz_regressions/` (committed minimized crashes as unit tests preferred) |

### 4.5 Bloat controls

- Target L2 size: **< 5 MB / target**, **< 500 files** after cmin.
- Reject inputs > harness max_len when merging.
- Never commit L2 under `fuzz/corpus/`.

---

## 5. Plateau strategy (rule 15 playbook)

Apply per target when `cov`/`ft` flat:

| Stage | Time stuck (order of magnitude) | Action |
|------:|----------------------------------|--------|
| 0 | start | L1 seeds + size guards |
| 1 | 10–30 min | Add/expand **dictionary**; enable `-use_value_profile=1` |
| 2 | 30–120 min | **CMPLOG** / AFL++ cmp log if available; or libFuzzer value profile already on |
| 3 | 2–4 h | Switch or add **structure-aware Arbitrary** / grammar generator |
| 4 | 4–24 h | **Custom mutator** (IVF/ANN length prefixes); hybrid symbolic only if still critical |
| 5 | 24 h+ | Accept saturation; **new target** (80/20 breadth — CORPUS.md) |

**Target-specific plateau notes:**

| Target | Likely barrier | First lever | Structure switch |
|--------|----------------|-------------|------------------|
| `query_grammar` | mode prefixes | dict (stage 1) | Mode enum Arbitrary (stage 3) |
| `rank` | low — oracle deep | seeds + caps only | already structured |
| `read_clusters_bounded` | u32 lengths + partition | seeds from `write_to` | **start** structured; mutator stage 4 |
| IVF header | magic/version | dict magic | structured header builder |
| `parse` / `match_pattern` | AST shapes | lang snippets + keyword dict | always structured pair |
| LSP frames | header grammar | CL dict + encoder seeds | body Arbitrary encoder |
| JSON / Serve | tags/methods | JSON+tool dict | typed enum Arbitrary |
| regex | metachar classes | regex dict | optional AST regex gen (low ROI) |

---

## 6. Checked but already adequate (≥3)

Evidence-based "do not rework shape/oracle for corpus sake":

1. **`rank` structure-aware input** — tuple `(&str, &str, Vec<usize>)` already (PASS2 PASS structure; PASS4 §7.1). Needs **caps + seeds**, not a new grammar.  
2. **`rank` numeric oracle** — finite, range, reverse-RRF equality already stronger than crash-only; dict investment is low ROI.  
3. **`embed_from_bytes` format simplicity** — single modulo-4 check; hand seeds + RT oracle beat elaborate Arbitrary (PASS3 §6 / PASS4 WP-A #1).  
4. **ANN full-probe vs `brute_force_flat` (units)** — differential oracle and fixtures already exist; port to fuzz with **in-harness generators**, not large static corpora (PASS4 §7.2).  
5. **IVF save/load round-trip (units)** — golden path via save API is better seed source than hand-hex of full files (PASS4 §7.3).  
6. **Product max bounds already present** — LSP `MAX_MESSAGE_BYTES`, batch `MAX_BATCH_CALLS=32`, IVF `k≤256`; harnesses should **tighten further** for exec/s, not re-derive product limits.

---

## 7. Ranked dictionary / corpus work (for later bead folding)

No beads filed this pass. Aggregate packages:

| Rank | Package | Targets | Effort | Unlocks |
|-----:|---------|---------|--------|---------|
| **1** | **Query grammar seeds + dict + max_len** | `query_grammar` | S | Fixes PASS2 D2/D4; free mode coverage |
| **2** | **Rank seeds + value caps** | `rank` | S | Fixes PASS2 D3; OOM safety |
| **3** | **ANN cluster generator seeds + structured harness inputs** | `read_clusters_bounded` | M | P0 binary surface; enables RT/differential later |
| **4** | **Tree-sitter polyglot snippet pack + shared source dict** | `parse`, `match_pattern` | M | Native crash surface breadth |
| **5** | **Structural pattern seed pairs + pattern dict** | `match_pattern`, `classify_native` | S–M | Dual-input + grammar consistency |
| **6** | **LSP frame seeds + `lsp_frame.dict` + encoder** | `read_message` | S | Framing + RT |
| **7** | **ServeRequest + JSON-RPC seed lines + tool/method dicts** | serde surfaces | S | Wire protocol parse |
| **8** | **embed LE seed pack (incl. NaN/odd len)** | `embed_from_bytes` | S | RT campaign |
| **9** | **URI traversal seed pack + uri dict** | uri helpers | S | Security-adjacent |
| **10** | **IVF header/mutant generator** (post-seam) | `parse_ivf_bytes` | M | Completes binary product path |
| **11** | **Regex metachar dict + timeout campaign seeds** | `Regex::new` | S | ReDoS class |
| **12** | **Shared cmin/regen scripts + CI L1-only PR** | all | M | PASS2 D8/D10 program |
| **13** | **Custom mutator ANN/IVF** | binary targets | L | Plateau stage 4 (PASS4 WP-C) |
| **14** | **rank/query optional secondary dict polish** | rank | S | Diminishing returns |

**Folding guidance:** WP-A (PASS4) should **include** ranks 1–2 and 8 as mandatory companions to new harnesses; ranks 3–5 with binary/native harnesses; ranks 6–7 with protocol harnesses; rank 12 as cross-cutting infra bead; rank 13 only after stage-3 plateau on binary targets.

---

## 8. Suggested seed file inventory (documentation only)

Paths relative to `tests/artifacts/fuzz-audit/seeds/` **or** future `fuzz/seed_corpus/` (not created en masse this pass):

```
query_grammar/
  empty
  minimal_hybrid          # foo
  mode_callers            # callers:parse_header
  mode_defs               # defs:SemanticAnnIndex
  mode_pattern            # pattern:fn $F
  mode_regex              # regex:a.*b
  mode_word               # word:Index
  adversarial_prefix_only # pattern:
rank/                     # logical seeds via gen script → binary corpus files
  exact_match
  substring
  empty_ranks
  long_tail_ranks
read_clusters/
  minimal_k1_d1.hex
  typical_k2.hex
  dup_member.hex
  short_truncate.hex
parse_snippets/
  rust_fn.rs
  python_class.py
  go_func.go
  ts_class.ts
  unclosed_rust.rs
patterns/
  fn_meta.txt
  class_meta.txt
  call_method.txt
  dollar_garbage.txt
lsp/
  minimal_frame
  missing_cl
  cl_too_big
serve/
  end.json
  call_search.json
  batch_one.json
embed/
  empty
  f32_one
  odd_len_3
  nan_bits
uri/
  file_ok
  dotdot
  pct_dotdot
```

Concrete contents are specified in §2 tables (implementers copy verbatim).

---

## 9. Dictionary file inventory (to commit under `fuzz/dictionaries/`)

| File | Used by | Priority |
|------|---------|----------|
| `query_grammar.dict` | query_grammar | **P0** |
| `pattern_structural.dict` | classify_native, match_pattern, signatures | **P0** |
| `tree_sitter_source.dict` | parse, match_pattern | **P0** |
| `ann_binary.dict` | read_clusters_bounded | **P1** |
| `ivf_header.dict` | parse_ivf / header | **P1** (post-seam) |
| `lsp_frame.dict` | read_message | **P1** |
| `json.dict` + `jsonrpc_mcp.dict` | MCP parse | **P1** |
| `serve_ndjson.dict` | ServeRequest | **P1** |
| `regex.dict` | user regex | **P2** |
| `uri.dict` | uri helpers | **P2** |
| `embed_f32.dict` | embed_from_bytes | **P3** |

Combine only when a single harness consumes multiple formats (e.g. match_pattern: `pattern_structural` + `tree_sitter_source`).

---

## 10. Input size budget summary

| Target | Harness hard cap | libFuzzer `-max_len` | Notes |
|--------|------------------|----------------------|-------|
| query_grammar | 8 KiB | 8192 | |
| rank strings | 256 / 512 | n/a structured | ranks len ≤ 64 |
| read_clusters | 16 KiB payload | 16384 | k≤8 dim≤32 n≤64 |
| parse source | 4 KiB (64 KiB deep) | 4096 | |
| match_pattern | src 2 KiB + pat 256 | structured | |
| classify_native | 512 | 512 | |
| embed | 4 KiB | 4096 | |
| LSP body | 64 KiB | 65536 | never 8 MiB |
| JSON-RPC / Serve | 8 KiB | 8192 | batch ≤ 32 |
| regex | 256 | 256 | `-timeout=2` |
| URI | 2 KiB | 2048 | |
| IVF full image | 128 KiB | 131072 | post-seam |

---

## 11. One-page roll-up

| Question | Answer |
|----------|--------|
| Biggest corpus gap today? | **Zero** tracked seeds; gitignore + no regenerator (PASS2 D2) |
| Biggest dict gap? | **`query_grammar` mode prefixes** (libFuzzer already asked) + pattern/ASIVF magic |
| Where structure-aware is mandatory? | ANN clusters, IVF images, match_pattern triples, text-edit ranges, rank (done) |
| Where raw+dict is enough? | query text, classify_native, embed LE, URI, regex, split_lines, fts escape |
| Custom mutator first target? | `read_clusters_bounded` / IVF index body after structured seeds exist |
| How to live with gitignored corpus? | Commit **L1 seeds + dicts + gen/cmin scripts**; cache L2 on CI only |
| Bead-fold priority? | query seeds/dict → rank caps/seeds → ANN generator → polyglot snippets → protocol frames |

---

## 12. Method / evidence index

| Source | Use |
|--------|-----|
| PASS1 matrix | Target ranking, format inventory |
| PASS2 D2–D4, D10, seed policy | Existing corpus/dict defects |
| PASS3 pure YES list | Harness-ready surfaces |
| PASS4 WP-A/C, oracle upgrades | RT/differential seed needs |
| `semantic_ivf.rs` MAGIC/VERSION/HEADER_SIZE/read_header | IVF seed layout |
| `semantic_ann.rs` write_to/read_clusters_bounded | ANN binary layout |
| `query.rs` mode prefixes | query dict tokens |
| `pattern.rs` DECL_PATTERN_PREFIXES / classify_native | pattern seeds |
| `support.rs` Content-Length / MAX_MESSAGE_BYTES | LSP seeds |
| `batch.rs` ServeRequest / MAX_BATCH_CALLS | NDJSON seeds |
| `embed/lib.rs` embed_from/to_bytes | LE f32 seeds |
| `codemode/catalog.rs` tool names | serve dict |
| skill CORPUS.md / DICTIONARIES.md | min seed set, plateau, cmin |

---

*End of PASS 5 — corpus / dictionary / structure-aware plan only. Artifact: `tests/artifacts/fuzz-audit/PASS5_CORPUS_DICT_STRUCTURE.md`.*
