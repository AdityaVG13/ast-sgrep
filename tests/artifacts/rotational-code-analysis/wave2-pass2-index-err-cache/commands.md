# Commands

```bash
export PATH="$HOME/.local/bin:$PATH"
export RCH_CANONICAL_PROJECT_ROOT=/Users/aditya
cd /Users/aditya/AI/ast-sgrep
export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/rch_target_ast-sgrep"
rch exec -- cargo test -p ast-sgrep-mcp --lib
rch exec -- cargo test -p ast-sgrep-codemode --lib
```

Note: without `RCH_CANONICAL_PROJECT_ROOT=/Users/aditya`, realpath `/Users/aditya/Developer/ast-sgrep` escapes configured `canonical_root=/Users/aditya/AI`.
