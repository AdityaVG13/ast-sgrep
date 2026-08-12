# Commands — Wave 2 Pass 5

```bash
export PATH="$HOME/.local/bin:$PATH"
export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/rch_target_ast-sgrep"
# AI/ast-sgrep symlink realpaths into Developer/ → widen topology for this host
export RCH_CANONICAL_PROJECT_ROOT=/Users/aditya
cd /Users/aditya/Developer/ast-sgrep

rch exec -- env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
  cargo test -p ast-sgrep-cli --lib doctor_surfaces
# → doctor_surfaces_fast_unsafe_from_status / from_cli_flag / silent_on_balanced

rch exec -- env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
  cargo test -p ast-sgrep-mcp --lib
# → 5 passed (compile + cache_tests; ESC-3 string present in tool_index_repo)

rg -n 'durability_fast_unsafe|index may have committed|Privileged|escapes configured workspace|empty structural' \
  crates/ast-sgrep-cli/src/agent.rs \
  crates/ast-sgrep-mcp/src/lib.rs \
  docs/cascade-query-planner.md \
  docs/env-trust.md \
  docs/index-consistency.md \
  docs/codemode.md \
  docs/mcp.md
```
