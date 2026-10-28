# Native structural patterns

`pattern:` search never starts an external `ast-grep` process. It first queries
exact signatures stored in `pattern_nodes`, then reparses candidate files with
tree-sitter only when the index has no matching signature. PATH is **not**
searched. Optional bench spawn requires `ASGREP_ALLOW_AST_GREP=1` plus an
absolute `ASGREP_AST_GREP` path (`crates/ast-sgrep-core/src/pattern.rs`); that
path is not the search channel.

Formal id: **`DISC-pattern-native-subset`**.

## Supported / unsupported matrix

| Form | In subset? | Notes |
|---|---|---|
| Exact identifier (`process_request`) | **in** | Indexed signature + native match |
| Indexed `kind:function_item` signatures | **in** | |
| Declarations `fn` / `def` / `function` / `func` / `class` / `struct` / `interface` | **in** | Exact name equality (`struct App` ≠ `struct AppContext`) |
| `fn $NAME($$$)` (and sibling decl metavars) | **in** | `$NAME` = one identifier; `$$$` = args; body-less answers follow the reference per grammar — see below |
| No-parens pure names (`fn $A`, `fn plain_ret`, `function plain`) | **in** | Every fn-shaped node answers (rust: trait sigs ∪ function items, generics wildcarded); see below |
| No-parens + any section (`fn $A -> u32`, `func $A int`, `fn $A;`) | **in** | Answers EMPTY everywhere except go's bare metavar result (`func $A $R` ≡ no section) |
| Free call `process_request($$$)` / `$FUNC($$$)` | **in** | |
| Member call `$OBJ.$METHOD($$$)` | **in** | Same family as `$OBJECT.$METHOD($$$)` in the narrative examples |
| Function body templates (`fn $N($$$) { $STMT }`, `fn $N($$$) {}`) | **in** | `{ $STMT }` = exactly one statement; `{}` = empty body; `{ $$$ }` = any body |
| Spelled return sections (`fn $A($$$) -> u32`, `-> $R`, `func $A($$$) int`, `: number`, `-> int:`) | **in** | Receipted grammars only (rust/go/ts/php/py); see below |
| `;`-completed fn forms (`fn $A($$$);`) | **in** | Rust: no-return `;` = sigs WITHOUT returns; with a return spelled the `;` is a no-op. go/ts/php answer empty |
| If templates (`if ($COND) { $BODY }`, `if $COND { $BODY }`, `if $COND: $BODY`) | **in** | Condition must be a metavariable; paren/brace/colon forms are normalized |
| Concrete if conditions (`if (x > 0) { $BODY }`) | **out** | Fail-closed |
| Multi-statement body templates (`{ $A; $B }`) | **out** | Fail-closed |
| Statement-count templates on type bodies (`struct $N { $FIELD }`) | **out** | Fail-closed |
| Relational metavariable constraints (`$A == $B`) | **out** | |
| Rule YAML / rewrites / autofix | **out** | Use standalone ast-grep CLI |
| Predicates beyond indexed `kind:` | **out** | |

`$NAME` matches one identifier. `$$$` matches an argument sequence. Exact
signatures are compared by indexed equality, so `struct App` does not match
`struct AppContext`.

## Nested statement templates

Body templates count named non-comment statements in the body/consequence
node: `{ $STMT }` (any single metavariable) matches exactly one statement,
`{}` matches an empty body, and `{ $$$ }` / `{ $$$BODY }` matches any body.
This mirrors ast-grep semantics on brace languages.

If templates are **normalized, not token-exact**: `if ($COND) { $BODY }`,
`if $COND { $BODY }`, and `if $COND: $BODY` are the same template, matched
against if-nodes in every indexed language (a paren pattern still matches a
Rust `if x > 0 { ... }`, and a brace pattern still matches a Python suite).
This intentionally diverges from ast-grep, which parses the pattern with one
language grammar; the Pattern-1 differential therefore compares only the
token-exact Rust forms against ast-grep. Ruby modifier-ifs and ternaries do
not match. `else if` / `elif` chains match only where the grammar nests a real
if-node (Rust/TS/Go/Java `else if`; Python `elif` is a distinct node and does
not match).

Body templates are matched only by the native tree-sitter scan, never by
indexed `pattern_nodes` signatures. The index cannot express statement
counts, so serving these shapes from it would over-match
(`cached_pattern_signatures` returns `None` for any `{` pattern).

## Pattern-1 differential

Bounded match-set compare vs `ast-grep` lives in `tests/core/pattern_diff.rs`
(fixture `tests/fixtures/pattern_diff/lib.rs`). Default `cargo test` runs the
native in/out rows and leaves the external row Not-run when its environment
variable is absent.

```bash
# Native contract only; external equality remains Not-run
cargo test -p ast-sgrep-core --test pattern_diff

# Local keep-gate; the test hard-requires ast-grep 0.45.1
ASGREP_DIFF_AST_GREP=/absolute/path/to/ast-grep \
  cargo test -p ast-sgrep-core --test pattern_diff --
```

Unset `ASGREP_DIFF_AST_GREP` must not be reported as match-set Pass. The
conformance registry records the row as Not-run unless the variable is set;
an unpinned competitor is a hard failure.

The equality list holds only patterns where ast-grep's token-exact parse
agrees with the native match set (`process_request`, `process_request($$$)`,
`$OBJ.$METHOD($$$)`, `if $COND { $BODY }`); last verified locally against
ast-grep 0.45.1 on the fixture. Native-normalized forms are asserted
separately: ast-grep is visibility-exact (`pub fn` never matches a
pattern without `pub`) and does not match `struct AppContext` against
`struct AppContext {}`. The native engine normalizes both; the test
comments in `tests/core/pattern_diff.rs` carry the per-pattern rationale.

### Body-less fn patterns answer per grammar (br-f4w, 2026-10-27)

`fn $NAME($$$)` — argument list present, no `{...}` section — used to be
normalized onto function declarations. It now follows the reference's
parse-recovery semantics, probed against ast-grep 0.45.2 and 0.51.0
(3 stable rounds each; pinned in
`tests/lang/pattern.rs::bodyless_fn_patterns_match_reference_grammar_specific_kinds`):

- **Rust**: answers `function_signature_item` nodes only — trait method
  declarations. A pattern-absent return type is a wildcard (`fn $A($$$)`
  matches `fn m(&self) -> u32;` and `fn m(&self);` alike). Function items
  with bodies never answer.
- **Go**: `func` is natively body-less valid syntax; `func $A($$$)` answers
  `function_declaration` nodes with body and return type wildcarded.
- **TypeScript / PHP**: the reference's recovery answers nothing; these
  lanes return empty (not fail-closed).
- **Both rust and go refuse generic candidates** (`fn f<T>(...)` /
  `func f[T any](...)`): spelled type parameters are required to align, and
  the native pattern subset cannot spell them.
- JavaScript / Python and patterns without an argument list
  (`fn $NAME`, `def $NAME`) keep the legacy normalization path — their
  reference behavior is unprobed, deliberately unchanged.

Body-having templates (`fn $N($$$) { $$$B }`) are untouched: visibility
wildcarded, return-type agreement still enforced (a pattern without a
return section matches only declarations without one).

### Spelled return sections and `;`-completed forms (br-hs1, 2026-10-27)

A return-type section between the argument list and the body/terminator is
now parsed and matched for the receipted grammars — rust `-> T`, go bare
`T` (the result position), ts/php `: T`, py `-> T:` before the suite.
`$R` binds the declared return text; concrete text must match exactly
(admissible concrete: path identifiers plus one generic layer, e.g.
`Option<u32>`). Everything else — other grammars, `map[k]v` results,
multi-token types — stays loudly fail-closed. Pinned in
`tests/lang/pattern.rs::spelled_return_and_semicolon_forms_match_reference_grammar_specific_kinds`
(ast-grep 0.45.2, 3 stable rounds per cell):

- **Body-less + return**: rust answers sigs with a matching return; go
  answers declarations (metavar `$R` wildcards even ABSENT results — a
  body-less go `$R` matches `func nop()` too); php answers
  `function_definition` with a matching return (its no-return body-less
  lane stays empty); ts answers nothing either way.
- **Body + return**: function declarations whose return agrees — a
  metavariable requires a PRESENT return (go included: `$R { $$$B }`
  refuses `func nop()`), concrete text requires equality. ts/php answer
  free functions only; py methods answer (same node kind as functions).
- **`;`-completed**: rust `fn $A($$$);` (no return spelled) answers sigs
  WITHOUT returns only — the strict signature reading; with any return
  spelled the `;` is a no-op terminator (`-> u32;` answers the `-> u32`
  sigs). go/ts/php `;` forms answer empty.
- Unreceipted grammars keep the loud fail-closed census class (the
  answerability gate refuses; the walk stays silent-empty as defense).

### No-parens decl patterns (br-osk, 2026-10-27)

A decl pattern with NO argument list is the reference's name lane:

- **Pure name** (`fn $A`, `fn plain_ret`, `function plain`): EVERY fn-shaped
  node of the grammar answers — rust answers trait signature items AND
  function items (impl-nested included), generics wildcarded (`fn gen<T>`
  and `[T any]` decls match like any other). Concrete names filter by name
  across both node kinds (`fn sig_r` answers the trait sig).
- **Any spelled section on a no-parens form answers EMPTY**: rust `fn $A -> u32`,
  `fn $A -> $R`, `fn $A;`, ts `function $A: number`, py `def $A -> int`,
  go `func $A int` — concrete names too (`fn sig_r -> u32`). The ONE
  exception: go's bare metavar result `func $A $R`, which the reference
  reads as no section at all (answers every decl).
- Document-order answers: the name lane sorts by byte offset (it unions
  several queries).
- **No-parens + ANY body template answers EMPTY** (`fn $A { $$$B }`,
  `fn $A { $STMT }`, `fn $A {}`, `def $A: $$$B` — concrete names too):
  only the pure name answers; a no-parens form with any tail is
  unmatchable in the reference (probed rust/go/ts/php/py).
- **ts/php answer FREE FUNCTIONS only** on this face — class methods and
  arrows never answer the `function` keyword spelling (the legacy table's
  method rows serve other lanes, not the name lane).
- **go bare metavar binds**: `func $A $R` binds `$R` to the parameter-list
  text WITH parens (`(a int)`, `()` for the empty list).

## Unsupported

Multi-statement body templates, concrete if conditions, relational
metavariable constraints, rule YAML, rewrites, and predicates beyond the
indexed `kind:` signature return no hits or fail-closed. Use the standalone
`ast-grep` CLI directly when those features are required. They are not
silently delegated, so structural search latency has no process startup tail.

Smoke coverage lives in `crates/ast-sgrep-lang` pattern tests, ranking oracle
pattern modes, and `tests/core/pattern_diff.rs`.
