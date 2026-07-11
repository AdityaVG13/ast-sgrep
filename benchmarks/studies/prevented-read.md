# Prevented-read study

> **Published record** of measured results. No runnable harnesses ship in this tree.

Date: 2026-07-11  
Corpus: ripgrep 14.1.1 fixture in `benchmarks/corpora/ripgrep`  
Gold set: `benchmarks/gold/ripgrep.json` (20 natural-language queries, top 10)

## Question

How many source bytes can a retrieval client avoid fetching when it consumes ranked excerpts instead of opening every distinct file represented in the same top-10 result set?

This study holds retrieval success equal by construction. Both arms use the identical asgrep ranking. The whole-file baseline charges the complete byte size of each distinct file containing a returned hit; the excerpt arm charges the UTF-8 byte length of the returned excerpts. Both arms therefore have the same top-10 gold-file success for every query.

## Method

Run from the repository root:

```sh
cargo build --profile release-perf -p ast-sgrep-cli --bin asgrep --jobs 1
```

The script creates a temporary index, disables embeddings for an offline/reproducible lexical-structural run, executes all 20 ripgrep gold queries with `--limit 10 --json --format agent`, checks the existing gold file-suffix contract, and writes per-query observations plus aggregate totals to `benchmarks/results-prevented-read.json`.

The six additions that bring the established ripgrep gold set from 14 to 20 cover configuration loading, PCRE2 matcher construction, terminal hyperlinks, diagnostics, haystack opening, and regex configuration. They use the same file-level relevance schema and top-10 criterion as the existing evaluation machinery.

## Measured result

| measure | whole-file baseline | excerpt path |
|---|---:|---:|
| top-10 gold successes | 12 / 20 | 12 / 20 |
| bytes | 5,679,524 | 84,180 |

The proxy estimates **5,595,344 prevented bytes**, or **98.518%** of the whole-file baseline. The aggregate whole-file-to-excerpt ratio is **67.469x**. These are sums over queries; if the same source file appears in separate queries it is charged once per query, while duplicate hits in one file within one query charge that file only once.

The checked-in JSON is the source of truth for exact per-query ranks, hit counts, byte estimates, and ratios. Re-running the script replaces it with a new timestamped measurement.

## Instrumentation semantics

Every `SearchResponse` now carries:

- `read_bytes_estimate`: metadata byte sizes summed over distinct files containing returned hits;
- `returned_excerpt_bytes`: UTF-8 bytes in returned hit excerpts;
- `prevented_read_bytes`: the saturating difference between the two.

Agent and agent-capsule JSON expose the byte fields. Capsule output recomputes returned bytes from the actual body excerpts, or from previews when bodies are omitted. When `ASGREP_LEDGER_PATH` is set, each query appends one JSONL object containing `ts`, `query`, `hits`, and a `bytes` object with the three estimates. Ledger I/O is best-effort so telemetry cannot turn a successful search into a failure.

## Limitations

- This is a **proxy metric**, not an observation of bytes read by a live coding agent. An agent may open fewer files than the baseline, expand additional spans after seeing an excerpt, use cached content, or read through another tool.
- Equal success here means equal top-10 gold-file success for the same ranked results. It does not establish equal task-completion success, answer quality, or latency.
- Only 12 of 20 queries found a gold file in the top 10 in the offline no-embedding configuration. Byte savings do not compensate for the eight retrieval misses.
- File metadata size approximates the cost of opening a hit file. It excludes filesystem blocks, protocol framing, JSON scaffolding, tokenizer effects, repeated reads, and cache behavior.
- The gold set was authored by the ast-sgrep team after inspecting ripgrep and may favor its terminology. File-level relevance also does not prove that the returned span contains the answer.
- Results cover one checked-in Rust corpus and one local run. They are not evidence of a general 10x reduction across repositories or completed agent tasks.
- `ASGREP_LEDGER_PATH` records asgrep-side estimates only. Correlating those records with whatever file-read path an agent uses, and measuring real task completion, remain follow-up work outside this repository.
