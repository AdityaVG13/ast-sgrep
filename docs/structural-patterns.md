# Native structural patterns

`pattern:` search never starts an external `ast-grep` process. It first queries
exact signatures stored in `pattern_nodes`, then reparses candidate files with
tree-sitter only when the index has no matching signature.

## Supported

- exact identifiers and indexed signatures, including `kind:function_item`
- declarations: `fn`, `def`, `function`, `func`, `class`, `struct`, and
  `interface`
- declaration metavariables such as `fn $NAME($$$)`
- free calls such as `process_request($$$)` and `$FUNC($$$)`
- member calls such as `$OBJECT.$METHOD($$$)`
- exact call-path segment equality; one-segment call patterns match only the
  final callee segment

`$NAME` matches one identifier. `$$$` matches an argument sequence. Exact
signatures are compared by indexed equality, so `struct App` does not match
`struct AppContext`.

## Unsupported

Nested statement templates, relational metavariable constraints, rule YAML,
rewrites, and predicates beyond the indexed `kind:` signature return no hits.
Use the standalone `ast-grep` CLI directly when those features are required.
They are not silently delegated, so structural search latency has no process
startup tail.

Smoke coverage lives in `crates/ast-sgrep-lang` pattern tests and the ranking
oracle on this branch — not a bake-off name list.
