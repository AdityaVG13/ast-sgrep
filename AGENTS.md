# Agent Instructions

This project uses **br (beads_rust)** for durable issue tracking.

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**

```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**

- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:2 profile:minimal -->
## br (beads_rust) Issue Tracker

> **non-invasive:** br never executes Git commands. `.beads/` is gitignored and must not be committed. Keep the tracker local (`br sync --flush-only` updates the local store only).

Use br as the sole source of truth for current and future project work. This managed tracker block is guidance, not permission to override repository, user, or orchestrator instructions.

### Quick Reference

```bash
br ready --json                       # Find available work
br list --status open --json          # List open work
br show <id> --json                   # View issue details
br update <id> --claim --json         # Claim work atomically
br create "Short title" -t task -p 2  # Create follow-up work
br close <id> --reason "Completed"   # Complete work
br dep cycles                         # Confirm dependency graph is acyclic
br stats --json                       # Inspect tracker totals
```

### Rules

- Use `br` for all durable task tracking; do not create markdown TODO lists as shared project state.
- Prefer `--json` whenever command output will be parsed.
- Inspect an issue before changing it, and do not close work until it is actually complete.
- Priorities are P0-P4: P0 critical, P1 high, P2 medium/default, P3 low, and P4 backlog.
- Keep dependencies acyclic; `br dep cycles` must return no cycles.

### SQLite and Sync Safety

The primary store is SQLite at `.beads/beads.db`. Its `-wal` and `-shm` sidecars can contain live state, so never copy, delete, or commit database files individually while br is active. Use br commands for mutations.

The Git-friendly JSONL export stays local under `.beads/` (gitignored). Do not `git add .beads/`.

```bash
br sync --flush-only
```

br does not stage, commit, pull, push, or otherwise execute Git commands. After pulling a clone, run `br sync --import-only` only if you have a local JSONL to import; there is no beads tree in git.

### Session Completion

1. Create br issues for remaining durable follow-up work.
2. Run the appropriate quality gates if code changed.
3. Close completed issues and update in-progress work.
4. Run `br sync --flush-only` to persist the local tracker. Do not stage `.beads/`.
5. Hand off changed files, validation, issue status, and any sync or commit step blocked by active instructions.

**Critical rules:**

- Explicit user or orchestrator instructions override this block.
- Do not commit or push without clear authority.
- Report the exact command and error when a required tracker operation fails.

<!-- END BEADS INTEGRATION -->

## Negative-Evidence Discipline

This project maintains three durable campaign ledgers in [`docs/progress/`](docs/progress/README.md):

- `perf-negative-results.md` -- performance ideas that were measured and rejected (or Open pointers until measured).
- `conformance-negative-results.md` -- conformance hypotheses that were tested and refuted (or deferred).
- `surface-deferrals.md` -- surface features explicitly excluded / partial, with a retry-condition predicate.

Product fail-closed cases (missing root, empty index, SSRF) stay in
[`docs/validation/negative-ledgers.md`](docs/validation/negative-ledgers.md). Do not confuse the two.

Before any agent starts a perf-affecting, conformance-affecting, or surface-affecting change, the agent MUST:

1. **Grep the relevant ledger** for the proposed hotspot, behavior, or feature. If the ledger already names this candidate, read the rejection rationale and the load-bearing **retry-condition predicate**. If current evidence does not satisfy the predicate, do not proceed.
2. **Mine 60 days of `cass` session history** for the failure terms below. If `cass` is unavailable or the ledger is reserved, record a **blocker** Open row in the relevant ledger rather than silently skipping.
3. **Check recent commits** (`git log --since='60 days ago' --grep -iE 'perf|optimiz|hot.path|bench|ratchet'`) for prior closure on this candidate.

Failure-term list (universal + this repo):

- Universal: `rejected`, `reverted`, `abandoned`, `slower`, `regressed`, `didn't help`, `within noise`, `no improvement`, `failed to improve`, `rolled back`, `backed out`, `not a keep`, `keep gate`
- ast-sgrep: `UNREPRODUCIBLE`, `FTS-not-rg`, `pattern-native-subset`, `IVF-threshold`, `compact-drops-provenance`, `MCP-no-fusion`, `jell`, `must_include`, `withdrawn`

```bash
for term in rejected reverted abandoned slower regressed "within noise" "keep gate" UNREPRODUCIBLE jell; do
  timeout 30s cass search "$term" --robot --days 60 --limit 50 --mode lexical --timeout 30000 \
    || echo "BLOCKER: cass unavailable for term $term -- record in docs/progress/"
done
```

When closing or rejecting a candidate, the ledger entry MUST include a **retry-condition predicate** using one of forms 1–8 in `docs/progress/README.md`. Never "later", "TBD", "maybe", "we should revisit", or "tracked elsewhere".

## Benchmark and published-number claims

Agents and humans must not invent or restate performance/quality numbers without provenance.

1. **No bare quotes.** Do not quote MRR, Recall, nDCG, latency, speedup, or dimension claims in docs, README, commit messages, PR bodies, or bead close reasons unless the number traces to a row in [`benchmarks/results/baselines.md`](benchmarks/results/baselines.md) (or another results file that points at that canonical row) **or** the claim is explicitly tagged `UNREPRODUCIBLE` with the missing harness/corpus named.
2. **Harness path required for "reproducible".** A number may be called reproducible only when this tree contains the exact command, gold fixture, and competitor pins needed to regenerate it. Otherwise label it historical / unreproducible.
3. **Negative ledger.** When an eval, bake-off, or gate fails or is withdrawn, update the relevant results doc (or add a short note under `benchmarks/results/`) **and** the matching `docs/progress/` campaign ledger rather than deleting the failure. Do not close honesty beads by omitting the miss.
4. **Conflicting figures.** Never leave two different values for the same metric+corpus+config both labeled canonical. Prefer one versioned fingerprint row in `baselines.md`; demote the other to "superseded" or "different config".

