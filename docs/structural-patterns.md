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

The fixed 29-pattern bake-off contains declarations and bare identifiers only.
All 29 are pinned by `pattern_native_suite.rs` and run through the native index
without requiring an `ast-grep` installation.

A release-mode RCH probe populated 23,001 indexed files, ran 101 exact
`struct RegexMatcherBuilder` lookups, and measured a 0.00375ms p50. This isolates
the in-process indexed matcher rather than CLI startup and is over four orders
of magnitude below the 50ms acceptance ceiling. Reproduce with:

```bash
cargo test --locked --release -p ast-sgrep-core \
  --test pattern_native_suite indexed_pattern_p50_is_below_50ms_at_23k_files \
  -- --ignored --exact --nocapture --test-threads=1
```
