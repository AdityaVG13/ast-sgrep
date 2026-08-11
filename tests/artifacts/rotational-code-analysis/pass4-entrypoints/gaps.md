# Pass 4 — Gaps / UNKNOWN / residuals

## Missing owner / contract / test (state-changing)

| Gap ID | Entry | Missing | Severity |
|--------|-------|---------|----------|
| GAP-CM-ROOT | EP-CODEMODE-SESSION / EP-NAPI | No workspace jail; no negative test that foreign `root` is rejected | **high** (if host passes model root) / design if multi-root intentional |
| GAP-INDEX-PATH | EP-ENV ASGREP_INDEX_PATH | Contract unclear whether absolute outside-root is supported privilege | medium (document as privileged) |
| GAP-WATCH-ADV | EP-CLI-SUB-WATCH | Sparse adversarial tests (symlink/race) | medium → pass 9 |
| GAP-PLUGIN-TEST | EP-AGENT-PLUGIN | Automated packaging/load gate not evidenced this pass | low |
| GAP-VSCODE-TEST | EP-VSCODE | Extension test density UNKNOWN | low |
| GAP-XOR | Code Mode XOR MCP | Docs-only; no runtime mutex | medium policy |
| GAP-RO-FLAG | catalog `read_only` | Host enforcement of approval for index_repo not verified in Pi host | medium → pass 5 |

## UNKNOWN reachability

| ID | Question |
|----|----------|
| U-LSP-MULTIROOT | Only first workspace folder used? (`resolve_root` behavior) |
| U-SERVE-AUTH | codemode-serve trusts any local stdin writer (expected for sticky worker?) |
| U-BATCH-ROOT | batch request.root vs CLI --root precedence fully pinned in tests? (defaults apply; escape same as CM) |
| U-SUPERVISOR-FORGE | same-user env forge of worker nonce without parent pid check on all platforms |

## Inherited open (prior passes)

- B-DIRTY-FREEZE
- B-ZS-ENGINES
- B-NO-COVERAGE-GATE / B-NO-MUTATION-GATE
- B-SECURITY-NAPI-DOC

## Not gaps (explicitly classified)

- CLI full-FS access -- intentional local tool
- plugins crate -- not external entry
- fuzz -- dev-only, workspace-excluded
