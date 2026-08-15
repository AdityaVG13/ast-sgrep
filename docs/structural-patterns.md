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
| `fn $NAME($$$)` (and sibling decl metavars) | **in** | `$NAME` = one identifier; `$$$` = args |
| Free call `process_request($$$)` / `$FUNC($$$)` | **in** | |
| Member call `$OBJ.$METHOD($$$)` | **in** | Same family as `$OBJECT.$METHOD($$$)` in the narrative examples |
| Function body templates (`fn $N($$$) { $STMT }`, `fn $N($$$) {}`) | **in** | `{ $STMT }` = exactly one statement; `{}` = empty body; `{ $$$ }` = any body |
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
indexed `pattern_nodes` signatures — the index cannot express statement
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
separately: ast-grep parses `fn $NAME($$$)` as a trait signature item
(matching no declarations), is visibility-exact (`pub fn` never matches a
pattern without `pub`), and does not match `struct AppContext` against
`struct AppContext {}`. The native engine normalizes all of these; the test
comments in `tests/core/pattern_diff.rs` carry the per-pattern rationale.

## Unsupported

Multi-statement body templates, concrete if conditions, relational
metavariable constraints, rule YAML, rewrites, and predicates beyond the
indexed `kind:` signature return no hits or fail-closed. Use the standalone
`ast-grep` CLI directly when those features are required. They are not
silently delegated, so structural search latency has no process startup tail.

Smoke coverage lives in `crates/ast-sgrep-lang` pattern tests, ranking oracle
pattern modes, and `tests/core/pattern_diff.rs`.
