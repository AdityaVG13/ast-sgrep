# Cascade query planner

Unprefixed `Searcher::search` queries use one constraint cascade instead of independent retrieval fanout:

1. **Literal/trigram prefilter.** Case-insensitive literal terms select at most 100 candidate files. Large indexes use the trigram table; smaller indexes use bounded indexed-line matching. No later stage can introduce a file that did not survive this stage.
2. **Structural match.** Tree-sitter-derived symbols, graph anchors, and indexed AST signatures are evaluated and retained only inside those candidate files. If none match, the cascade stops.
3. **Semantic rerank.** The query embedding is compared only with chunks whose files survived the structural stage. Semantic retrieval cannot widen the candidate set.

The final result gate receives lexical, structural, and semantic evidence from the surviving files. It performs ordinary deduplication, per-kind ceilings, file filtering, ranking, and signal-margin assignment. A result's `signal` remains the producer provenance even when semantic evidence changes the final ordering.

## Stop behavior

The cascade returns no hybrid hits when either the lexical or structural stage has no survivors. This is deliberate: semantic similarity is a reranker, not an unconstrained repository-wide fallback. Use `asgrep semantic "<query>"` or `Searcher::search_semantic` when repository-wide semantic discovery is intended. Prefixed `literal:`, `regex:`, `pattern:`, `defs:`, `callers:`, and `imports:` modes continue to execute their dedicated retrieval path directly.

## Work bounds

- Structural rows outside lexical candidate files are discarded before they can become survivors.
- Semantic vector ranking receives only chunks from structural-survivor files.
- Candidate order is deterministic because final ordering and deduplication remain centralized in `finish_response`.
- Empty stages short-circuit without running later work.
