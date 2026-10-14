# sgrep architecture teardown vs asgrep (2026-10-14)

Why sgrep won its home suite (see `sgrep-suite-2026-10-14.md`), read from
source at the benchmarked commit `0fbe8e5` (sgrep) and the tested tree
`f2bbc3d2` + working-tree diff (asgrep), plus a miss classification over the
2,200 banked query records from that run.

## sgrep: 1,793 lines, two channels, one trained model

sgrep's entire retrieval core (`src/main.rs`, 598 lines) is:

1. **Corpus**: `rg --files` enumerates files (ripgrep is a hard dependency).
2. **Chunking** (`splitter.rs` + `codechunker.rs`): tree-sitter AST-node
   boundaries, greedy-merged upward to a 2048-*character* budget. A file
   under 2048 chars is one chunk. No symbol extraction, no call graph.
3. **Lexical channel**: hand-rolled BM25 (k1=1.5, b=0.75) over per-chunk
   term counts. Terms come from camelCase-split, lowercased words minus a
   ~90-word stopword list. **The file path's words are counted into every
   chunk of that file** (`make_chunk`), and the embedded text is
   `"{path}\n{content}"`, so path evidence participates in both channels.
4. **Semantic channel**: `minishlab/potion-code-16M-v2` — a Model2Vec
   *static* embedding table (63,457 subword tokens x 256 dims, f16, 32 MB),
   distilled from nomic-ai/CodeRankEmbed and contrastively fine-tuned on
   CornStack NL->code pairs. Embedding = mean of token vectors, L2
   normalized; similarity = dot product. No transformer runs at query time;
   "semantic" knowledge is entirely in the trained table. A prose model
   (potion-base-8M) is used when all candidates are docs.
5. **Candidate generation**: top-200 chunks by BM25 (score > 0) UNION
   top-200 by cosine, computed over **every chunk in the corpus**, every
   query. No gating: the semantic channel always votes.
6. **Fusion** (`rank` in main.rs): min-max normalize each channel *within
   the candidate pool*, then `0.25 * bm25_rel + 0.75 * semantic_rel`.
   Exact-identifier override: if the query contains a token with `_`, `$`,
   camelCase, or a digit (or the whole query is one identifier), chunks
   containing that identifier as a whole word sort before everything else.
   Contained-span dedup (a chunk inside an already-returned span of the
   same file is dropped). That is the entire ranking logic.
7. **Index** (`index.rs`): one mmap'd file per corpus — f32 vector matrix,
   chunks as JSON, term postings — keyed by sha256 of every file; any file
   change rebuilds. Query path re-embeds only the query and dot-products
   against the stored matrix (exhaustive; no ANN).

## asgrep (tested configuration): eight channels, no trained model

1. **Index**: SQLite — FTS5 lines, symbols, callers, pattern nodes,
   trigrams, and per-chunk embedding blobs (concatenated + 5 per-field
   vectors: name/docs/body/graph/tests). Median index 2,582 MB across the
   61 benchmark scopes vs sgrep's 173 MB.
2. **Chunks** (`semantic_chunk.rs`): one per function/method symbol
   (excerpt = full symbol span, plus up to 2 child pattern-node excerpts),
   rendered as `excerpt: ... symbol: ... kind: ... doc: ... called_by: ...
   calls: ...`. Richer units than sgrep's, but only function/method
   symbols become retrievable semantic units.
3. **Default embedder** (`ast-sgrep-embed/src/semantic.rs`, "semantic-v2"):
   **not a learned model.** Each token contributes a BLAKE3-derived +-1
   sign vector (weight 1.0); each character trigram the same at 0.35.
   Cosine between two texts is therefore a hashed bag-of-words overlap
   measure with random collisions — it cannot know that "combine adjacent
   strings" relates to `_merge_string_group` beyond shared tokens.
   A hand-written table of 20 concept groups (`CONCEPT_GROUPS`) injects
   synonym tokens, but its vocabulary is ast-sgrep's own self-gold domain
   (auth/refresh/wal/rrf/interning...). The neural embedder
   (fastembed/ONNX) exists but is feature-gated off in default builds.
4. **Candidate generation is a gated cascade** (`search/mod.rs`): lexical
   FTS first, capped at `retained_limit` (~3x output limit) files; symbol
   (def/caller) passes run **only inside those files**; the embed pass
   runs **only if total hits so far < the requested limit**
   (`structural_evidence_is_thin`), and even then only over chunks in the
   cascade's file set. The code comment is explicit: the hashed embedder
   "cannot outvote structural evidence" and its presence measurably hurt
   self-gold MRR (0.850 embed-on vs 1.000 off), so it was demoted to a
   fallback for thin pages.
5. **Fusion** (`fusion.rs` + `intent.rs`): weighted RRF (k=60) over 8
   channels with per-intent weights (Conceptual: embed 1.45, def 1.35,
   lexical 0.75, ...), then a rule-based critic. Sophisticated — but on
   this suite the channels it fuses are mostly lexical/symbolic.

## What actually happened on the 1,100 queries (banked records)

Miss classification (RepoQA, 600 queries, top-50 results each):

| outcome | asgrep | sgrep |
|---|---:|---:|
| gold span at rank 1 | 8.8% | 51.0% |
| gold span in top 10 | 17.8% | 31.2% |
| gold span present, ranked > 10 | 16.3% | 12.0% |
| gold file present, gold span never returned | 24.2% | 3.2% |
| gold file absent entirely | 32.8% | 2.7% |

CoSQA (500 queries): asgrep gold file absent 64.2% vs sgrep 5.2%.

Channel attribution (`contributors` in asgrep's own JSON, RepoQA top-10
slots, ~6,000 total): **def 5,513; lexical 510; pattern 243; caller 166;
graph 114; embed 29.** Of the gold-span hits that did reach the top 10,
embed contributed to **2**. With `--limit 50`, the cascade almost always
fills the page structurally, so the semantic channel effectively never ran
— and when it runs, it is hashed token overlap, not semantics.

asgrep out-ranked sgrep on only 45/600 RepoQA queries; both missed top-10
on 90.

## The five differences that explain the gap, in order

1. **A trained NL->code embedding table vs hashed token overlap.** This is
   most of the gap. RepoQA descriptions deliberately paraphrase; only a
   model trained on (description, code) pairs bridges that. sgrep's model
   is small (32 MB), static (no inference runtime), and MIT-licensed.
2. **Ungated exhaustive semantic scan vs gated cascade.** sgrep scores
   every chunk semantically on every query, so zero-lexical-overlap gold
   is reachable. asgrep's gold file is unreachable whenever FTS misses it
   (32.8% of RepoQA queries), regardless of embedder quality.
3. **Fusion math.** Min-max-normalized linear blend (0.75 semantic)
   preserves score *magnitudes*; the semantic winner wins big. RRF keeps
   only ranks, and asgrep's RRF pool is dominated by def-channel hits
   whose "evidence" is symbol-name term overlap.
4. **Chunk granularity + span reachability.** sgrep returns arbitrary
   AST-bounded spans, so the gold function is usually inside some returned
   chunk (3.2% wrong-span). asgrep returns whole-symbol/line excerpts
   chosen by other channels; in 24.2% of queries it found the right file
   but never surfaced the gold function's span in 50 results.
5. **Path-as-evidence.** sgrep embeds and BM25-counts the file path in
   every chunk. Cheap, and on file-named-for-content corpora it is free
   signal asgrep's chunk text lacks (path appears only in test/example
   classification, not in the embedded text).

## What sgrep does NOT have (asgrep's actual moat)

- Structural/pattern search, definitions/callers/graph navigation,
  codemods — sgrep has none; it is a single-purpose passage ranker with a
  ripgrep passthrough mode.
- Incremental indexing: any file change invalidates sgrep's whole index
  (sha256 manifest equality); large repos pay full re-embed.
- Answer calibration/provenance: no confidence, no per-channel why, no
  query grammar. Score magnitudes are pool-relative (min-max), so they
  are meaningless across queries.
- Scale: exhaustive dot product over all chunks per query; fine at these
  corpus sizes, no ANN story.

## Prompting/steering implications for asgrep work

- Treat "semantic-v2" as a lexical channel wearing a semantic name. Any
  task phrased as "improve semantic search quality" must start with
  replacing or bypassing the embedder; weight tuning cannot create
  knowledge the vectors do not contain.
- The cascade's file gating is a hard recall ceiling independent of
  embedder quality. Prompts about retrieval quality should name the gate
  (`structural_evidence_is_thin`, `embed_pass_for_files`) explicitly.
- Self-gold is a biased judge here: `CONCEPT_GROUPS` was literally tuned
  to self-gold vocabulary (see the `cg5` comment in semantic.rs), and the
  embed channel was demoted based on self-gold MRR. External gold
  (RepoQA/CoSQA via `benchmarks/sgrep_suite/run_suite.py`) now exists in
  tree; steer agents to validate ranking changes against it, not only
  self-gold.
- sgrep proves the quality bar is reachable with a 32 MB static table +
  exhaustive scan + 0.25/0.75 blend, inside asgrep's existing speed
  budget (its p50 was 57 ms vs asgrep's 16 ms, both fresh-process). The
  cheapest credible catch-up path is: real static code embeddings as the
  default embedder, an ungated embed pass (at least for Conceptual
  intent), and magnitude-preserving fusion for that channel — each
  separately measurable on the frozen suite.
