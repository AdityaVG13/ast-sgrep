# Parity report — Pass 7 (error-path)

## Suite

```text
cd packages/pi/extension && npm test
# 88 pass, 0 fail (node:test, ASGREP_CODEMODE_BACKEND=cli)
```

Evidence (selected):

- `execution boundary` — OUTPUT_LIMIT, MALFORMED_OUTPUT, PROCESS_FAILED, timeout/exec mapping
- `machine compatibility` — TOOL_MISMATCH / PROTOCOL_MISMATCH / version
- `classified runtime failures` — ok:false OPERATIONAL_ERROR for zero and nonzero exits
- `index format upgrades` — INDEX_REBUILD_FAILED recovery details preserved
- Full suite: code-mode, codemode, commands, runtime, security, session-pool, skill-workflow, tools

## Differential

Behavior-preserving refactor: no intentional public contract change. Characterization covered by existing runtime tests (envelope error codes, rebuild recovery, exec classification).

## Cargo

Not run — no Rust touched.

## Verdict

**pass** — 88/88 extension tests green after transforms.
