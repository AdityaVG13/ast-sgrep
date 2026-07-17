---
name: beads
description: Use when working in a repository that uses br or beads_rust for durable project task tracking, issue dependencies, blocker management, multi-session handoff, or shared work memory. Trigger when the user asks to find ready work, claim or close tasks, create follow-up work, inspect blockers, or recover project context.
---

# br (beads_rust)

> **non-invasive:** br never executes Git commands. After `br sync --flush-only`, manually stage `.beads/` and commit it when the active instructions authorize a commit.

Use br as the shared project task system and the durable source of truth for current and future work. Agent-local plans are only for the current execution checklist.

## Core Workflow

1. Find and inspect work:

```bash
br ready --json
br list --status open --json
br list --status in_progress --json
br show <id> --json
```

2. Claim work atomically:

```bash
br update <id> --claim --json
```

3. Create durable follow-up work:

```bash
br create "Short title" --description "Why this exists and what needs to be done" --type task --priority 2 --json
```

4. Manage dependencies and inspect totals:

```bash
br dep add <child-id> <parent-id>
br dep cycles
br stats --json
```

5. Close completed work:

```bash
br close <id> --reason "Completed" --json
```

6. Export tracker state, then perform the Git steps manually if authorized:

```bash
br sync --flush-only
git add .beads/
git commit -m "sync beads"
```

After receiving newer Git-tracked JSONL, run `br sync --import-only` before continuing.

## What Belongs in br

Use br for shared project tasks, blockers, dependencies, discovered follow-up work, resumable status, and knowledge another person or agent must inherit. Do not use markdown TODO files as the shared source of truth.

## Priority Scale

- P0: critical
- P1: high
- P2: medium and the default
- P3: low
- P4: backlog

## SQLite and WAL Safety

The primary store is `.beads/beads.db`. SQLite may hold current state in `.beads/beads.db-wal` and coordinate access through `.beads/beads.db-shm`. Never copy, delete, or commit these database files individually while br is active; use br commands to mutate issues and `br sync --flush-only` to produce the Git-friendly export.

## Rules

- Prefer `--json` whenever output will be parsed.
- Inspect an issue before editing it.
- Do not mutate or close work unless the requested change is actually complete.
- Keep dependencies acyclic; `br dep cycles` must report no cycles.
- br only updates tracker files. It never stages, commits, pulls, pushes, or otherwise executes Git.
- Explicit user, repository, and orchestrator instructions govern whether the manual Git steps are allowed.
