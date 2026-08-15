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
| Nested statement templates (`if ($COND) { $BODY }`, `fn $N($$$) { $STMT }`) | **out** | Empty or fail-closed; not delegated |
| Relational metavariable constraints (`$A == $B`) | **out** | |
| Rule YAML / rewrites / autofix | **out** | Use standalone ast-grep CLI |
| Predicates beyond indexed `kind:` | **out** | |

`$NAME` matches one identifier. `$$$` matches an argument sequence. Exact
signatures are compared by indexed equality, so `struct App` does not match
`struct AppContext`.

## Pattern-1 differential

Bounded match-set compare vs `ast-grep` lives in `tests/core/pattern_diff.rs`
(fixture `tests/fixtures/pattern_diff/lib.rs`). Default `cargo test` runs the
native in/out rows only.

```bash
# Not-run (CI default): equality test is #[ignore]
cargo test -p ast-sgrep-core --test pattern_diff

# Local, when ast-grep is installed (do not enable on PR CI without sign-off):
ASGREP_DIFF_AST_GREP=/absolute/path/to/ast-grep \
  cargo test -p ast-sgrep-core --test pattern_diff -- --ignored
```

Unset `ASGREP_DIFF_AST_GREP` must not be reported as match-set Pass.

## Unsupported

Nested statement templates, relational metavariable constraints, rule YAML,
rewrites, and predicates beyond the indexed `kind:` signature return no hits
or fail-closed. Use the standalone `ast-grep` CLI directly when those features
are required. They are not silently delegated, so structural search latency
has no process startup tail.

Smoke coverage lives in `crates/ast-sgrep-lang` pattern tests, ranking oracle
pattern modes, and `tests/core/pattern_diff.rs`.
