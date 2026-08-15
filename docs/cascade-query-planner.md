# Cascade query planner

Unprefixed `Searcher::search` queries use one constraint cascade instead of independent retrieval fanout:

1. **Literal/trigram prefilter.** Case-insensitive literal terms select at most 100 candidate files. Large indexes use the trigram table; smaller indexes use bounded indexed-line matching. No later stage can introduce a file that did not survive this stage.
2. **Structural match.** Tree-sitter-derived symbols, graph anchors, and indexed AST signatures are evaluated and retained only inside those candidate files.
3. **Working-file set + semantic rerank.** When structural survivors exist, they become the working set. When the structural stage is **empty**, the cascade **continues** on the lexical survivors (ht1h.3 / INV-CASCADE-STRUCT-EMPTY): plain-content files stay findable and optional semantic ranking runs on those lexical files. Semantic retrieval cannot widen beyond that working set.

The final result gate receives lexical, structural, and semantic evidence from the surviving files. It performs ordinary deduplication, per-kind ceilings, file filtering, ranking, and signal-margin assignment. A result's `signal` remains the producer provenance even when semantic evidence changes the final ordering.

## Stop behavior

| Stage empty | Hybrid behavior |
|-------------|-----------------|
| **Lexical** | Cascade stops — no hybrid hits. Semantic similarity is not an unconstrained repository-wide fallback. |
| **Structural** | Cascade **continues** on lexical survivors (+ optional embed on those files). |

Use `asgrep semantic "<query>"` or `Searcher::search_semantic` when repository-wide semantic discovery is intended. Prefixed `literal:`, `regex:`, `pattern:`, `defs:`, `callers:`, and `imports:` modes continue to execute their dedicated retrieval path directly.

> **Historical note:** Earlier drafts of this doc claimed empty structural stopped the cascade. That mismatched `search_hybrid` and `cascade_planner` tests (C1 / INV-CASCADE-STRUCT-EMPTY). Current text matches code.

## Causal follow-up planner

The agent envelope's `follow_up_queries` and `suggested_next` are computed by
a deterministic planner (`search/planner.rs`) from the evidence each returned
hit actually carries, not from a template:

- A hit missing definition evidence gets `defs:<symbol>`; one missing usage
  evidence (caller/graph) gets `callers:<symbol>`.
- A hit with complete evidence but an indecisive within-signal margin (below
  10% of its own score) gets `literal:<symbol>` for exact-text confirmation.
- A hit whose definition, usage, and ordering are all settled gets no
  follow-ups: an empty list means "you are done", not "nothing to offer".
- A critic-flagged identifier collision drills into the compound identifier
  the query named, not the colliding fragment.

`suggested_next` starts from the actual top hit's follow-ups, adds an
`asgrep semantic '<query>'` re-run only when the shortlist contains no
semantic evidence, and always ends with the agent-format re-run. Every query
argument is POSIX single-quoted (including embedded quote encoding), so every
entry is a safe executable `asgrep` command.

## Work bounds

- Structural rows outside lexical candidate files are discarded before they can become survivors.
- Semantic vector ranking receives only chunks from the working-file set (structural survivors, or lexical survivors when structural is empty).
- Candidate order is deterministic because final ordering and deduplication remain centralized in `finish_response`.
- Empty **lexical** short-circuits without running later work; empty **structural** does not.
