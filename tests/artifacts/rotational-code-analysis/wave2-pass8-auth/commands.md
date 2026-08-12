# Commands — Wave 2 Pass 8

```bash
export PATH="$HOME/.local/bin:$PATH"
export RCH_CANONICAL_PROJECT_ROOT=/Users/aditya
export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/rch_target_ast-sgrep"

rch exec -- cargo test -p ast-sgrep-cli --test watch_incremental -- --nocapture
# ok. 2 passed (incl. update_paths_refuses_symlink_escape_into_index)
```

Note: without `RCH_CANONICAL_PROJECT_ROOT=/Users/aditya`, RCH may refuse the Developer symlink path under `/Users/aditya/AI`.
