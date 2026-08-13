# Wave 2 / Pass 7 — Loop 12 serialization / protocol / version

Mission: attack wire + storage contracts under **old+new peers** and **upgrade**, without redoing pass-6 generation fallthrough.

Residual closed: **R-NEWER-SCHEMA-SILENT-OPEN** — `init_schema` used `version >= SCHEMA_VERSION`, so an older binary silently opened a newer on-disk index.

Books live here (campaign mirror). Canonical skill state: `TARGET/.rotational-code-analysis/state.json`.
