# Commands

```bash
export PATH="$HOME/.local/bin:$PATH"
export RCH_CANONICAL_PROJECT_ROOT=/Users/aditya

rch exec -- cargo test -p ast-sgrep-codemode --lib
# ok. 2 passed (index_repo_invalidates_searcher_on_index_err + foreign_root_is_rejected_under_session_workspace)

rch exec -- cargo test -p ast-sgrep-codemode --test catalog --test session_plan
# catalog: 3 passed; session_plan: 4 passed

rg -n "fn root_arg|fn sandbox_root|starts_with|escapes configured workspace" \
  crates/ast-sgrep-codemode/src/session.rs
```
