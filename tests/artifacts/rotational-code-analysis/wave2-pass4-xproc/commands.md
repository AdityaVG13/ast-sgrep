# Commands (wave2 pass4)

```bash
export PATH="$HOME/.local/bin:$PATH"
export RCH_CANONICAL_PROJECT_ROOT=/Users/aditya
export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/rch_target_ast-sgrep"

rch exec -- env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
  cargo test -p ast-sgrep-core --lib writer_generation
# → 3 passed (bump / pinned / generation-home)

rch exec -- env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
  cargo test -p ast-sgrep-mcp --lib cache_tests
# → 4 passed (incl. external_writer_generation_invalidates_warm_searcher)

rch exec -- env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
  cargo test -p ast-sgrep-codemode --lib
# → 3 passed (incl. external_writer_generation_invalidates_warm_searcher)
```

Note: without `RCH_CANONICAL_PROJECT_ROOT=/Users/aditya`, RCH refuses `/Users/aditya/Developer/ast-sgrep` (symlink under `/Users/aditya/AI` resolves outside canonical_root).
