# Assumptions

1. Option C lite is product-authorized primary (generation/mtime epoch peers poll); full IPC bus / flock lease deferred.
2. Same-process stamp bump is an honest dual-writer stand-in for watch×MCP (no two-process harness required for closed-fail).
3. Stamp lives at index home (`.asgrep/writer_generation` or beside pinned DB), not inside generation candidate dirs.
4. Zerostack unavailable this pass; RCH with `RCH_CANONICAL_PROJECT_ROOT=/Users/aditya` used for verify.
5. Pre-existing dirty-tree `SearchHit`/`SearchResponse` test initializers missing new fields were fixed only enough to compile lib tests.
