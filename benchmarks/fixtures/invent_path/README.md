# Invent-path semantic fixture

Offline hashed concept-group retrieval without neural weights, downloads, or a
daemon. Targets omit the query prose on purpose: ranking must come from concept
expansion (and optional repository vocabulary), not token overlap.

Gold labels: `benchmarks/gold/invent_path.json`.

```bash
cargo run -q -p ast-sgrep-cli --bin asgrep -- \
  --json eval --gold benchmarks/gold/invent_path.json \
  benchmarks/fixtures/invent_path
```
