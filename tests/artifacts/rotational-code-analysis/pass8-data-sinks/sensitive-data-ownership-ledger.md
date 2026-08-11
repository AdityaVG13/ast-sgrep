# Pass 8 — Sensitive-data and ownership ledger

Observer: **data-owner + adversary**. Who owns each datum, who may read/write, where it is stored, and what an adversary-controlled agent can force.

## Ownership roles

| Role | Meaning |
|------|---------|
| **Operator** | Human who starts process; sets env (`ASGREP_*`), workspace root |
| **Host** | Agent host (Cursor/Claude/Pi); loads MCP or Code Mode; owns process FS as OS user |
| **Model/agent** | Untrusted high-control source of tool args |
| **Core** | `ast-sgrep-core` libraries (shared sinks) |
| **Remote embed** | Third-party HTTP embed API (when enabled) |

---

## Ledger

| Datum | Owner (source of truth) | Readers | Writers | Storage | Agent control? | Sensitivity |
|-------|-------------------------|---------|---------|---------|----------------|-------------|
| Workspace `ASGREP_ROOT` / MCP root | Operator/host | MCP server | process env only | process memory | Can only pick **sub** roots under jail | isolation base |
| CM `root` arg | Model (if host passes through) | CM session | each call | ephemeral args | **Yes — full OS path** | high if model-controlled |
| Query text | Model / user | engines, embed, logs/responses | each call | cache keys, response JSON | Yes (≤4096 chars) | medium (code intent); high if embeds |
| Search hit paths | Index / disk under root | agent via envelopes | indexer | SQLite + path_registry | Indirect (search shapes registry) | medium |
| Compact path_registry | MCP session | `code_read` resolve | search success; clear on index ok | in-memory Mutex map | Cannot forge `p` table without search | soft capability |
| Node id strings | Model | `read_node` | each call | none durable | Yes — constrained grammar | medium |
| File slice content | Project files (disk) | agent (code_read/search) | editor/Pi edit; indexer copies | disk + SQLite | Read yes (jailed MCP); write Pi under root | high (source code) |
| Index DB bytes | Operator placement | Searcher/Indexer | index_repo / CLI index | path from DF-INDEX | Indirect via index tools | high (corpus mirror) |
| `ASGREP_INDEX_PATH` | Operator env | all surfaces | env / flags | env | No (env plane) | privileged placement |
| Embed API key | Operator env | ureq Authorization | env only | env + process mem | No | **secret** |
| Embed URL | Operator env | embedder allowlist | env | env | No (unless host lets model set env — out of scope) | SSRF-sensitive |
| Query/chunk vectors | derived | search rank | embed path | SQLite embed cache + process cache | Indirect | low–medium |
| Miss `why`/`next` | product logic | agent | plugins | response JSON | Influences next actions | agent-control surface |
| Plan step outputs | CM plan runner | later `$ref` | each step | in-memory plan | Yes (composition) | can carry paths |
| Ast-grep binary path | Operator | bench spawn | env | env | No without dual flags | command exec |

---

## Adversary goals vs enforcement

| Goal | Blocked by | Residual |
|------|------------|----------|
| Read `/etc/passwd` via MCP `code_read` | relative-only node id + sandbox root + canon | none observed |
| Escape MCP workspace via `root` | `sandbox_root` | none observed (symlink-resolved) |
| Escape via CM `root` | **not blocked** (by design) | C2 / GAP-CM-ROOT |
| Write index DB outside project | **not blocked** for abs index path | GAP-INDEX-PATH-PRIV |
| SSRF to metadata IP via embed | allowlist + redirects(0) + scheme | GAP-EMBED-REDIR-IT (live IT) |
| Exfil source via embed API | none (operator-enabled feature) | intentional; query/chunks leave host |
| Inject API key into logs | Debug redaction | other log paths not fully audited this pass |
| Spawn shell via ast-grep | dual opt-in absolute path | CONSISTENT |
| Poison path_registry then read foreign file | registry paths still pass `parse_node_id` + root join | stale paths under **same** root only |
| Use `$ref` to retarget CM root | none in CM | elevates free-root risk |
| Oversized query DoS | 4096 char cap | regex CPU residual |

---

## Redaction & storage notes

1. **API key:** `CloudEmbeddingConfig` Debug redacts; request header is the live secret. No code path found that serializes key into search JSON.
2. **Index DB:** may contain full file text and embeddings; if placed under shared cache or absolute path, **ownership leaves project tree**.
3. **path_registry:** not durable across process restart; durable risk is within long MCP sessions after partial index failure.
4. **Miss envelopes / compact JSON:** control surface for agent next steps — not secret, but untrusted guidance (product-owned strings).

---

## Sink criticality ranking (data-owner view)

| Rank | Sink | Why critical |
|------|------|--------------|
| 1 | Embed HTTP (query + index chunks) + API key header | network egress + secret |
| 2 | Index SQLite write at absolute path | durable corpus + placement privilege |
| 3 | CM free `root` → WalkDir / search | model-driven FS scope |
| 4 | MCP `code_read` content | source exfil to model (jailed) |
| 5 | Pi `asgrep_edit` write | source mutation |
| 6 | Ast-grep spawn | command execution (opt-in) |
| 7 | Agent miss/suggested_next | soft control, not FS |
