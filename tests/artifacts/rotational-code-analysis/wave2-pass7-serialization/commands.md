# Commands — Wave 2 Pass 7

```bash
export PATH="$HOME/.local/bin:$PATH"
export RCH_CANONICAL_PROJECT_ROOT=/Users/aditya

# Pre-fix evidence: migration pins still asserted schema 7 while SCHEMA_VERSION=9
rch exec -- cargo test -p ast-sgrep-core --test semantic_chunk_migration -- --nocapture
# → FAILED: left: 9 / right: 7 (×2 tests)

# After fail-closed future schema + pin fix
rch exec -- cargo test -p ast-sgrep-core --test semantic_chunk_migration -- --nocapture
# → ok. 4 passed (incl. newer_than_binary_schema_refuses_open)
```
