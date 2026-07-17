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

> **non-invasive:** br never executes Git commands. After `br sync --flush-only`, manually stage `.beads/` and commit it when the active instructions authorize a commit.

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

Export the database to the Git-friendly files explicitly, then handle Git yourself:

```bash
br sync --flush-only
git add .beads/
git commit -m "sync beads"
```

br does not stage, commit, pull, push, or otherwise execute Git commands. The Git commands above are manual steps and must be omitted when current instructions prohibit committing. After receiving updated `.beads/` files, use `br sync --import-only` to import JSONL into SQLite.

### Session Completion

1. Create br issues for remaining durable follow-up work.
2. Run the appropriate quality gates if code changed.
3. Close completed issues and update in-progress work.
4. Run `br sync --flush-only`; if authorized, manually stage `.beads/` and commit.
5. Hand off changed files, validation, issue status, and any sync or commit step blocked by active instructions.

**Critical rules:**

- Explicit user or orchestrator instructions override this block.
- Do not commit or push without clear authority.
- Report the exact command and error when a required tracker operation fails.
<!-- END BEADS INTEGRATION -->
