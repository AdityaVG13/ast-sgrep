use crate::intent::ChannelWeights;
use crate::rank::{rrf_score, RRF_K};
use crate::search::{HitKind, NameIntern, SearchHit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FusionChannel {
    Lexical,
    Definition,
    Caller,
    Graph,
    Anchor,
    Semantic,
    Pattern,
    Import,
}

impl FusionChannel {
    pub const ALL: [Self; 8] = [
        Self::Lexical,
        Self::Definition,
        Self::Caller,
        Self::Graph,
        Self::Anchor,
        Self::Semantic,
        Self::Pattern,
        Self::Import,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRanks {
    pub lexical: Option<usize>,
    pub definition: Option<usize>,
    pub caller: Option<usize>,
    pub graph: Option<usize>,
    pub anchor: Option<usize>,
    pub semantic: Option<usize>,
    pub pattern: Option<usize>,
    pub import: Option<usize>,
}

const RANK_GETTERS: [fn(&ChannelRanks) -> Option<usize>; 8] = [
    |r| r.lexical,
    |r| r.definition,
    |r| r.caller,
    |r| r.graph,
    |r| r.anchor,
    |r| r.semantic,
    |r| r.pattern,
    |r| r.import,
];

const RANK_SETTERS: [fn(&mut ChannelRanks, Option<usize>); 8] = [
    |r, v| r.lexical = v,
    |r, v| r.definition = v,
    |r, v| r.caller = v,
    |r, v| r.graph = v,
    |r, v| r.anchor = v,
    |r, v| r.semantic = v,
    |r, v| r.pattern = v,
    |r, v| r.import = v,
];

const WEIGHT_GETTERS: [fn(&ChannelWeights) -> f64; 8] = [
    |w| w.lexical,
    |w| w.def,
    |w| w.caller,
    |w| w.graph,
    |w| w.anchor,
    |w| w.embed,
    |w| w.pattern,
    |w| w.import,
];

const WEIGHT_SETTERS: [fn(&mut ChannelWeights, f64); 8] = [
    |w, v| w.lexical = v,
    |w, v| w.def = v,
    |w, v| w.caller = v,
    |w, v| w.graph = v,
    |w, v| w.anchor = v,
    |w, v| w.embed = v,
    |w, v| w.pattern = v,
    |w, v| w.import = v,
];

/// HitKind discriminant order → FusionChannel::ALL index.
const HIT_KIND_CHANNEL_IDX: [usize; 8] = [0, 1, 2, 3, 4, 7, 6, 5];

/// HitKind discriminant order → canonical merge priority (lower wins).
const CANONICAL_PRIORITY: [usize; 8] = [6, 0, 1, 2, 5, 4, 3, 7];

impl ChannelRanks {
    pub fn get(&self, channel: FusionChannel) -> Option<usize> {
        RANK_GETTERS[channel.index()](self)
    }

    fn set_best(&mut self, channel: FusionChannel, rank: usize) {
        let idx = channel.index();
        let current = RANK_GETTERS[idx](self);
        if current.is_none_or(|value| rank < value) {
            RANK_SETTERS[idx](self, Some(rank));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionCandidate {
    pub id: String,
    pub relevance: f64,
    pub ranks: ChannelRanks,
}

fn hit_kind_idx(kind: HitKind) -> usize {
    match kind {
        HitKind::Asgrep => 0,
        HitKind::Def => 1,
        HitKind::Caller => 2,
        HitKind::Graph => 3,
        HitKind::Anchor => 4,
        HitKind::Import => 5,
        HitKind::Pattern => 6,
        HitKind::Embed => 7,
    }
}

fn channel_for_kind(kind: HitKind) -> FusionChannel {
    FusionChannel::ALL[HIT_KIND_CHANNEL_IDX[hit_kind_idx(kind)]]
}

fn clamp_channel_weight(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.25, 2.0)
    } else {
        1.0
    }
}

/// Rail a channel weight into the fused range. `#[doc(hidden)]`: exposed for
/// the testkit-only training module (`fusion_train`); not runtime API.
#[doc(hidden)]
pub fn weight(weights: &ChannelWeights, channel: FusionChannel) -> f64 {
    clamp_channel_weight(WEIGHT_GETTERS[channel.index()](weights))
}

/// Set a channel weight through the fused rail. `#[doc(hidden)]`: exposed for
/// the testkit-only training module (`fusion_train`); not runtime API.
#[doc(hidden)]
pub fn set_weight(weights: &mut ChannelWeights, channel: FusionChannel, value: f64) {
    WEIGHT_SETTERS[channel.index()](weights, clamp_channel_weight(value));
}

pub fn weighted_rrf_score(ranks: &ChannelRanks, weights: &ChannelWeights) -> f64 {
    FusionChannel::ALL
        .into_iter()
        .filter_map(|channel| {
            ranks
                .get(channel)
                .map(|rank| weight(weights, channel) * rrf_score(rank, RRF_K))
        })
        .sum()
}

fn canonical_priority(kind: HitKind) -> usize {
    CANONICAL_PRIORITY[hit_kind_idx(kind)]
}

/// Drop extra-channel votes when an embed score is below
/// `1 - DECISIVE_MARGIN_RATIO` of the query's best. A result the embed
/// channel never scored keeps `baseline`: zeroing those votes is a
/// different rule and fails Hit@100.
fn score_below_cosine_support(
    ranks: &ChannelRanks,
    magnitude: Option<f64>,
    semantic_top: f64,
    weights: &ChannelWeights,
    baseline: f64,
) -> f64 {
    let cut = 1.0 - crate::search::planner::DECISIVE_MARGIN_RATIO;
    if semantic_top <= 0.0 || ranks.semantic.is_none() {
        return baseline;
    }
    let Some(magnitude) = magnitude else {
        return baseline;
    };
    if magnitude >= cut * semantic_top {
        return baseline;
    }
    let mut score = weight(weights, FusionChannel::Semantic)
        * rrf_score(ranks.semantic.expect("checked"), RRF_K);
    score +=
        weight(weights, FusionChannel::Semantic) * rrf_score(0, RRF_K) * (magnitude / semantic_top);
    score
}

/// Fused scores, plus the pre-critic embed magnitude of each fused row
/// (same order). The critic reads that vector to stop a far-cosine hit
/// from outranking the best embed on a description query. Callers that
/// only want the scores ignore the return.
/// One result row of the `ASGREP_FUSION_DUMP` capture: the merged result key
/// with its pre-fusion per-channel ranks, contributor kinds, semantic
/// magnitude, and final fused score. Owned (not borrowed) so the row can
/// outlive the `SearchHit` it describes.
#[derive(serde::Serialize)]
struct FusionDumpRow {
    file: String,
    line_start: u32,
    line_end: u32,
    kinds: Vec<&'static str>,
    ranks: ChannelRanks,
    /// Per-field embed cosines when this result entered the embed lane's
    /// field-rescore pool (None outside it) — the record for field-
    /// combination lever diagnostics.
    embed_fields: Option<crate::search::field_weight::EmbedFieldScores>,
    semantic_magnitude: Option<f64>,
    score: f64,
}

/// Append one JSONL line for this fusion invocation to the env-named path.
/// Exactly one line per `apply_weighted_rrf` call, empty input included, so a
/// harness can join lines to queries by byte offset. Best-effort: IO errors
/// are swallowed (a debug facility must not fail the query; a harness that
/// points the env at a bad path notices the missing/short dump itself).
fn write_fusion_dump(path: &std::path::Path, input_hits: usize, rows: &[FusionDumpRow]) {
    #[derive(serde::Serialize)]
    struct Line<'a> {
        input_hits: usize,
        results: &'a [FusionDumpRow],
    }
    let Ok(text) = serde_json::to_string(&Line {
        input_hits,
        results: rows,
    }) else {
        return;
    };
    use std::io::Write as _;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{text}");
    }
}

/// Path-scope prior scale for test/example/bench trees. Kept 2026-10-26 on
/// paired evidence: semgrep-lane real-binary arms flask 0.647 -> 0.658
/// MRR@10 (fl-routes miss -> rank 6), ripgrep flat; frozen RepoQA-600
/// hit1 preserved exactly, hit100 +.0017, mrr10 -.0008 (two documented
/// test-gold residuals the query guard cannot save); CoSQA-500
/// bit-identical. `ASGREP_SCOPE_SCALE` overrides the constant for paired
/// A/B arms (1.0 = off); unparseable values fall back to the default so a
/// typo never silently disables the prior.
pub(crate) fn scope_scale(path: &str) -> f64 {
    const DEFAULT_SCALE: f64 = 0.4;
    if let Some(v) = std::env::var_os("ASGREP_SCOPE_SCALE") {
        if let Ok(f) = v.to_string_lossy().parse::<f64>() {
            if f.is_finite() && f > 0.0 {
                return f;
            }
        }
    }
    if is_test_path(path) {
        DEFAULT_SCALE
    } else {
        1.0
    }
}

/// C/C++ declaration-stub demotion scale. Kept-on evidence 2026-10-26
/// (round-H follow-up; full paired-arm numbers and retry predicates in
/// docs/progress/perf-negative-results.md). Round-H header-prototype
/// extraction floods header-heavy corpora with 1-line stub hits
/// (`tcp.h:101-101`) that insert above doc-rich golds (oatpp-01 5->12,
/// oatpp-04 6->22, node-sass-07 13->22). A hit in a C/C++-family file
/// whose span is at most 2 lines is scaled by DEFAULT_SCALE; everything
/// else is 1.0. Known residual: a gold file whose delegate row is an
/// orphan stub sinks with the flood (oatpp-05 7->15). Two orphan-rescue
/// witness variants — shortlist-membership and corpus-level index
/// lookup — were both REJECTED 2026-10-26 (see ledger): un-demoting
/// orphan stubs costs more mrr10 than it recovers. `ASGREP_DECL_STUB_
/// SCALE` overrides the constant for paired arms (1.0 = off);
/// unparseable values fall back to the default so a typo never
/// silently disables the demotion.
pub(crate) fn decl_stub_scale(path: &str, line_start: u32, line_end: u32) -> f64 {
    const DEFAULT_SCALE: f64 = 0.5;
    const MAX_STUB_SPAN_LINES: u32 = 2;
    let mut scale = DEFAULT_SCALE;
    if let Some(v) = std::env::var_os("ASGREP_DECL_STUB_SCALE") {
        if let Ok(f) = v.to_string_lossy().parse::<f64>() {
            if f.is_finite() && f > 0.0 {
                scale = f;
            }
        }
    }
    if scale >= 1.0 {
        return 1.0;
    }
    let span = line_end.saturating_sub(line_start) + 1;
    if span <= MAX_STUB_SPAN_LINES && is_c_family_path(path) {
        scale
    } else {
        1.0
    }
}

/// Extension-only C/C++ family classification (headers and sources).
pub(crate) fn is_c_family_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    match name.rsplit_once('.') {
        Some((_, ext)) => matches!(
            ext,
            "h" | "hpp" | "hh" | "hxx" | "h++" | "c" | "cc" | "cpp" | "cxx" | "c++"
        ),
        None => false,
    }
}

/// Whole-segment and filename test-path classification, ASCII-folded.
/// Segments: tests/test/testing/testdata/spec/specs/fixtures/bench/benches/
/// example/examples/sample/samples/demo/demos. Filenames: `test_*`,
/// `conftest.py`, `*_test.{py,go,rs,ts,js,rb,java}`, `*_spec.{ts,js,rb}`.
pub(crate) fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.split('/').any(|seg| {
        matches!(
            seg,
            "tests"
                | "test"
                | "testing"
                | "testdata"
                | "spec"
                | "specs"
                | "fixtures"
                | "bench"
                | "benches"
                | "example"
                | "examples"
                | "sample"
                | "samples"
                | "demo"
                | "demos"
        )
    }) {
        return true;
    }
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if name.starts_with("test_") || name == "conftest.py" {
        return true;
    }

    ["py", "go", "rs", "ts", "js", "rb", "java"]
        .iter()
        .any(|e| name.ends_with(&format!("_test.{e}")))
        || ["ts", "js", "rb"]
            .iter()
            .any(|e| name.ends_with(&format!("_spec.{e}")))
}

/// True when the query itself asks about testing/specs/benchmarks
/// (word-boundary match, ASCII-folded). Such queries must NOT demote
/// test-path hits — the user is explicitly test-seeking. Word list covers
/// the frozen-suite regression set (lazydocker-06, croc-06, go-zero-06,
/// lux-05/06 name testing; fzf-09/reactpy-03 do not and stay demoted).
pub(crate) fn query_mentions_testing(query: &str) -> bool {
    const WORDS: &[&str] = &[
        "test",
        "tests",
        "testing",
        "tested",
        "spec",
        "specs",
        "coverage",
        "mock",
        "mocks",
        "fixture",
        "fixtures",
        "benchmark",
        "benchmarks",
        "bench",
    ];
    query
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .collect::<String>()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| WORDS.contains(&w))
}

pub fn apply_weighted_rrf(
    hits: &mut Vec<SearchHit>,
    weights: &ChannelWeights,
    query: &str,
) -> Vec<Option<f64>> {
    // ASGREP_FUSION_DUMP=<path> captures the pre-fusion channel-rank table
    // this ledger's retry predicates demand (loss attribution + the
    // confidence-scaled-fusion / rank-floor retry conditions). Unset → one
    // `var_os` read and no other cost; ranking is untouched either way.
    let dump_path = std::env::var_os("ASGREP_FUSION_DUMP").map(std::path::PathBuf::from);
    let capture = dump_path.is_some();
    // Test-seeking queries keep test-path hits at full strength.
    let scope_disabled = query_mentions_testing(query);
    let input_hits = hits.len();
    if hits.is_empty() {
        if let Some(path) = dump_path.as_deref() {
            write_fusion_dump(path, 0, &[]);
        }
        return Vec::new();
    }
    let mut channels: [Vec<usize>; 8] = std::array::from_fn(|_| Vec::new());
    struct Acc {
        file: u32,
        line: u32,
        members: Vec<usize>,
        semantic: f64,
        ranks: ChannelRanks,
    }
    let mut intern = NameIntern::new(hits.len());
    let mut accs: Vec<Acc> = Vec::new();
    let mut hit_slot = vec![usize::MAX; hits.len()];
    for (index, hit) in hits.iter().enumerate() {
        if hit.score.is_finite() && hit.score > 0.0 {
            channels[channel_for_kind(hit.kind).index()].push(index);
            let file = intern.intern(&hit.file);
            let slot = accs
                .iter()
                .position(|acc| acc.file == file && acc.line == hit.line_start)
                .unwrap_or_else(|| {
                    let slot = accs.len();
                    accs.push(Acc {
                        file,
                        line: hit.line_start,
                        members: Vec::new(),
                        semantic: 0.0,
                        ranks: ChannelRanks::default(),
                    });
                    slot
                });
            accs[slot].members.push(index);
            if hit.kind == HitKind::Embed && hit.score > accs[slot].semantic {
                accs[slot].semantic = hit.score;
            }
            hit_slot[index] = slot;
        }
    }
    let semantic_top = accs.iter().map(|acc| acc.semantic).fold(0.0f64, f64::max);
    let mut declaration_lexical = vec![false; hits.len()];
    for &index in &channels[FusionChannel::Lexical.index()] {
        let lexical = &hits[index];
        let line = lexical.excerpt.trim();
        let slot = hit_slot[index];
        if (line.ends_with('{') || line.ends_with(':'))
            && accs[slot].members.iter().any(|&member| {
                let definition = &hits[member];
                definition.kind == HitKind::Def
                    && definition.line_end > definition.line_start
                    && definition
                        .symbol
                        .as_deref()
                        .is_some_and(|name| !name.is_empty() && line.contains(name))
                    && (definition.excerpt.is_empty()
                        || definition.excerpt.lines().next().map(str::trim) == Some(line))
            })
        {
            declaration_lexical[index] = true;
        }
    }
    for channel in FusionChannel::ALL {
        let members = &mut channels[channel.index()];
        members.sort_by(|left, right| {
            hits[*right]
                .score
                .total_cmp(&hits[*left].score)
                .then_with(|| hits[*left].file.cmp(&hits[*right].file))
                .then_with(|| hits[*left].line_start.cmp(&hits[*right].line_start))
                .then_with(|| hits[*left].line_end.cmp(&hits[*right].line_end))
        });
        let mut seen = vec![false; accs.len()];
        let mut rank = 0usize;
        for index in members.iter().copied() {
            if declaration_lexical[index] {
                continue;
            }
            let slot = hit_slot[index];
            if !seen[slot] {
                seen[slot] = true;
                accs[slot].ranks.set_best(channel, rank);
                rank += 1;
            }
        }
    }

    let mut order: Vec<usize> = (0..accs.len()).collect();
    order.sort_by(|&left, &right| {
        intern
            .get(accs[left].file)
            .cmp(intern.get(accs[right].file))
            .then(accs[left].line.cmp(&accs[right].line))
    });
    let mut fused = Vec::with_capacity(order.len());
    let mut magnitudes = Vec::with_capacity(order.len());
    let mut dump_rows = Vec::new();
    for slot in order {
        let mut members = std::mem::take(&mut accs[slot].members);
        members.sort_by(|left, right| {
            canonical_priority(hits[*left].kind)
                .cmp(&canonical_priority(hits[*right].kind))
                .then_with(|| hits[*right].score.total_cmp(&hits[*left].score))
                .then_with(|| hits[*left].line_end.cmp(&hits[*right].line_end))
                .then_with(|| hits[*left].symbol.cmp(&hits[*right].symbol))
                .then_with(|| hits[*left].caller.cmp(&hits[*right].caller))
                .then_with(|| hits[*left].callee.cmp(&hits[*right].callee))
                .then_with(|| hits[*left].language.cmp(&hits[*right].language))
                .then_with(|| hits[*left].excerpt.cmp(&hits[*right].excerpt))
        });
        let mut result = hits[members[0]].clone();
        let mut contributors = members
            .iter()
            .map(|index| hits[*index].kind)
            .collect::<Vec<_>>();
        contributors.sort_by_key(|kind| channel_for_kind(*kind).index());
        contributors.dedup();
        let kinds = if capture {
            contributors
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // Enrich identifying fields from members: a merged same-line result
        // keeps the canonical kind/priority but surfaces the symbol a semantic
        // chunk carries for that line (semantic_cache_version contract).
        if result.symbol.is_none() {
            if let Some(sym) = members
                .iter()
                .map(|index| hits[*index].symbol.as_deref())
                .find(|s| s.is_some())
                .flatten()
            {
                result.symbol = Some(sym.to_string());
            }
        }
        if result.embed_fields.is_none() {
            result.embed_fields = members
                .iter()
                .find_map(|index| hits[*index].embed_fields.clone());
        }
        result.contributors = contributors;
        // Rank-only RRF (K=60) is nearly flat across the top ranks, so a
        // result corroborated by several weak lexical channels outvotes a
        // decisive semantic match that only the embed channel found. The
        // semantic channel therefore casts a second, magnitude vote worth
        // up to one extra rank-0 vote, scaled by this result's cosine
        // relative to the query's best. Monotonic in cosine, so
        // within-channel order is unchanged; non-semantic results are
        // bit-identical to pure weighted RRF.
        let ranks = &accs[slot].ranks;
        let magnitude = (accs[slot].semantic > 0.0).then_some(accs[slot].semantic);
        let mut score = weighted_rrf_score(ranks, weights);
        if ranks.semantic.is_some() && semantic_top > 0.0 {
            if let Some(magnitude) = magnitude {
                score += weight(weights, FusionChannel::Semantic)
                    * rrf_score(0, RRF_K)
                    * (magnitude / semantic_top);
            }
        }
        score = score_below_cosine_support(ranks, magnitude, semantic_top, weights, score);
        // Path-scope prior: production sources outrank test/example/bench
        // trees on equal evidence ("where does the code do X" retrieval).
        // Bounded multiplicative scale — a test-only match still surfaces.
        // Disabled for test-seeking queries (query_mentions_testing) and
        // via the paired-arm kill switch ASGREP_SCOPE_SCALE=1.0.
        score *= if scope_disabled {
            1.0
        } else {
            scope_scale(&result.file)
        };
        // C/C++ declaration-stub demotion: 1-2-line prototype stubs sink
        // below doc-rich blocks (round-H follow-up; predicate in
        // docs/progress/perf-negative-results.md). Paired-arm kill switch
        // ASGREP_DECL_STUB_SCALE=1.0. Orphan-rescue witness variants
        // (shortlist-membership AND corpus-level index lookup) were both
        // REJECTED 2026-10-26 — un-demoting orphan stubs costs more mrr10
        // than it recovers in either formulation. A per-file stub CAP
        // (escalate a file's 4th-onward stub) was also REJECTED 2026-10-26:
        // it sank gold-delegate stubs with the flood (uvw-01 1->3) for a
        // net cpp mrr10 .6196->.6168 — same classifier-ambiguity root.
        score *= decl_stub_scale(&result.file, result.line_start, result.line_end);
        result.score = score;
        if capture {
            dump_rows.push(FusionDumpRow {
                file: result.file.clone(),
                line_start: result.line_start,
                line_end: result.line_end,
                kinds,
                ranks: ranks.clone(),
                embed_fields: result.embed_fields.clone(),
                semantic_magnitude: magnitude,
                score,
            });
        }
        fused.push(result);
        magnitudes.push(magnitude);
    }
    if let Some(path) = dump_path.as_deref() {
        write_fusion_dump(path, input_hits, &dump_rows);
    }
    *hits = fused;
    magnitudes
}

#[cfg(test)]
mod training_surface_moved {
    /// Guard: the offline training surface lives in testkit-only
    /// `fusion_train`, never in the shipped binary. If this fails, a runtime
    /// path grew a training dependency — move it back out.
    #[test]
    fn no_training_fns_in_core_fusion() {
        let src = include_str!("fusion.rs");
        // `concat!` splits keep this guard's own literals from matching
        // themselves — the contiguous pattern never appears in source.
        for leaked in [
            concat!("fn pairwise", "_loss("),
            concat!("fn ranking", "_churn("),
            concat!("fn channel", "_sensitivity("),
            concat!("fn analyze", "_weight_sensitivity("),
            concat!("fn learn", "_fusion_weights("),
            concat!("struct Fusion", "Example {"),
            concat!("struct Learned", "FusionModel {"),
            concat!("struct Weight", "Sensitivity {"),
        ] {
            assert!(
                !src.contains(leaked),
                "training surface leaked into shipped fusion.rs: {leaked}"
            );
        }
    }
}
