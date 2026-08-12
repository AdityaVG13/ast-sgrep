# Assumptions

- Product authorized **Option A**: jail CM/NAPI `root` under session workspace like MCP `sandbox_root`.
- NAPI has no separate root resolver; it wraps `CodeModeSession` — Session jail is the cascade.
- Threat model remains local OS user / installing agent (policy contradiction, not remote CVE).
- Multi-root hosts that intentionally pointed tool `root` outside session workspace will now fail closed (breaking change by design).
- zerostack unavailable this pass; books under `tests/artifacts/…` + `.rotational-code-analysis/`.
