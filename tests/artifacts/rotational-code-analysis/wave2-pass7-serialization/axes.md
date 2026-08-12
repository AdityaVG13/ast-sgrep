# Axes — Wave 2 Pass 7 vs Pass 6

| Axis | Pass 6 (prior) | Pass 7 (this) |
|------|----------------|---------------|
| representation | state-store-model (SQLite tx · sidecars · active.json · flat legacy) | **wire/storage-format** (MCP protocolVersion · machine JSON · compact `v` · `PRAGMA user_version`) |
| observer | data-integrity (one coherent corpus after crash) | **old+new-peer** (downgrade binary vs newer on-disk index; client/server negotiate) |
| time | commit+recovery | **upgrade** (schema migrate landing · refuse future schema · rolling peers) |
| evidence | store path + generation_swap pin | **init_schema gate + semantic_chunk_migration** |

**≥2 axes changed:** representation, observer, time.
**V-SAME-GAZE avoided:** not re-opening missing-generation fallthrough / try_index_db_path (pass 6).
