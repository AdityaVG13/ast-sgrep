//! Core pipeline/oracle fixtures: corpus+DB pairs, option builders, and
//! response-key projections shared by the `tests/core/oracle_*` suites.
//!
//! # Contract
//!
//! - One canonical copy of the file-local helpers formerly triplicated across
//!   `oracle_pipeline` / `oracle_stages` / `oracle_scoring` (corpus fixture,
//!   index/search options, full-build and searcher legs, hit-key projections).
//! - Builders index real tempdir corpora into real on-disk SQLite (`index.db`
//!   under a private [`tempfile::TempDir`]); `index_path` is always explicit,
//!   so ambient `ASGREP_INDEX_PATH` / XDG cache cannot leak across tests.
//! - Embeddings are off by construction (`embed_semantic: false`,
//!   `use_embed: false`): these are the lexical/pipeline oracles.
//! - Helpers panic (never `Result`) on IO/index failure, matching suite
//!   convention: a broken fixture is a test failure, not a fallible op.
//! - Key projections are pure functions of the response.

use ast_sgrep_core::fusion::{
    apply_weighted_rrf, ChannelRanks, FusionCandidate, FusionChannel, FusionExample,
};
use ast_sgrep_core::intent::{route_hits, ChannelWeights};
use ast_sgrep_core::query::{ParsedQuery, QueryMode};
use ast_sgrep_core::search::{HitKind, SearchHit};
use ast_sgrep_core::store::{CallerRow, SymbolRow, UpsertFileInput};
use ast_sgrep_core::{
    IndexOptions, Indexer, IndexStore, SearchOptions, SearchResponse, Searcher,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// INTENT: private writable corpus + private on-disk DB kept alive for one
/// pipeline test. The caller keeps the fixture alive; `root`/`db` feed the
/// option builders below.
pub struct CorePipelineFixture {
    /// Owns the corpus tree; must outlive `root` use.
    pub _corpus: TempDir,
    /// Owns the DB directory; must outlive `db` use.
    pub _index: TempDir,
    /// Writable corpus root.
    pub root: PathBuf,
    /// Explicit on-disk index path (`index.db` under the private index dir).
    pub db: PathBuf,
}

/// INTENT: build the corpus+DB pair from `(rel, body)` files (parents
/// created). Panics on IO failure.
pub fn write_core_fixture(files: &[(&str, &str)]) -> CorePipelineFixture {
    let corpus = tempfile::tempdir().expect("corpus tempdir");
    for (rel, body) in files {
        let path = corpus.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&path, body).expect("write fixture");
    }
    let index = tempfile::tempdir().expect("index tempdir");
    let db = index.path().join("index.db");
    CorePipelineFixture {
        root: corpus.path().to_path_buf(),
        db,
        _corpus: corpus,
        _index: index,
    }
}

/// INTENT: force-reindex, embed-off index options over the fixture.
/// Pure constructor.
pub fn core_index_options(root: &Path, db: &Path) -> IndexOptions {
    IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    }
}

/// INTENT: embed-off search options with an explicit limit. Pure constructor.
pub fn core_search_options(root: &Path, db: &Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        limit,
        use_embed: false,
        ..SearchOptions::default()
    }
}

/// INTENT: full build leg — fresh indexer over the fixture, whole tree
/// indexed. Panics on index failure.
pub fn build_core_index(fixture: &CorePipelineFixture) -> Indexer {
    let mut indexer =
        Indexer::new(core_index_options(&fixture.root, &fixture.db)).expect("indexer new");
    indexer.index_all().expect("index_all");
    indexer
}

/// INTENT: searcher over the fixture DB with the given limit. Panics when the
/// searcher cannot be constructed.
pub fn core_searcher(fixture: &CorePipelineFixture, limit: usize) -> Searcher {
    Searcher::new(core_search_options(&fixture.root, &fixture.db, limit)).expect("searcher new")
}

/// INTENT: root-only opener — default [`SearchOptions`] over `root`, so the
/// index resolves through the default state layout (`index_path: None`):
/// the lib-side anchor over CLI/MCP-indexed roots. Delta vs
/// [`core_searcher`]: that needs a [`CorePipelineFixture`] with an explicit
/// DB; this opens whatever default state the root carries. Panics when the
/// searcher cannot be constructed.
pub fn searcher_at_root(root: &Path) -> Searcher {
    let opts = SearchOptions {
        root: root.to_path_buf(),
        ..SearchOptions::default()
    };
    Searcher::new(opts).expect("lib Searcher::new succeeds on indexed root")
}

/// INTENT: order-sensitive identity of a ranked response (file, span, kind,
/// symbol, exact score bits) — two full runs agree iff these agree. Pure
/// projection; [`crate::response_hit_keys`] has no score bits, so pipeline
/// bit-identity needs this richer key.
pub fn response_hit_keys_with_scores(
    response: &SearchResponse,
) -> Vec<(String, u32, u32, String, Option<String>, u64)> {
    response
        .hits
        .iter()
        .map(|hit| {
            (
                hit.file.clone(),
                hit.line_start,
                hit.line_end,
                hit.kind.as_str().to_string(),
                hit.symbol.clone(),
                hit.score.to_bits(),
            )
        })
        .collect()
}

/// INTENT: order-free file-set comparison for shaping facets. Pure projection.
pub fn sorted_hit_files(response: &SearchResponse) -> Vec<String> {
    let mut files: Vec<String> = response.hits.iter().map(|hit| hit.file.clone()).collect();
    files.sort();
    files
}

/// INTENT: minimal `SearchOptions` (root + limit, rerank off) shared by every
/// finish-gate facet. Pure constructor.
pub fn finish_options(root: &Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        limit,
        file_filter: None,
        count_only: false,
        use_rerank: false,
        ..SearchOptions::default()
    }
}

/// INTENT: indexed rank setter — `ChannelRanks` has no indexed setters,
/// so fusion metamorphic legs build per-channel fixtures through this.
/// Pure mutation of the caller's struct.
pub fn set_rank(ranks: &mut ChannelRanks, channel: FusionChannel, value: Option<usize>) {
    match channel {
        FusionChannel::Lexical => ranks.lexical = value,
        FusionChannel::Definition => ranks.definition = value,
        FusionChannel::Caller => ranks.caller = value,
        FusionChannel::Graph => ranks.graph = value,
        FusionChannel::Anchor => ranks.anchor = value,
        FusionChannel::Semantic => ranks.semantic = value,
        FusionChannel::Pattern => ranks.pattern = value,
        FusionChannel::Import => ranks.import = value,
    }
}

/// INTENT: indexed weight setter — the [`ChannelWeights`] counterpart of
/// [`set_rank`] (note the `def`/`embed` field renames vs the channel
/// names). Pure mutation of the caller's struct.
pub fn set_weight(weights: &mut ChannelWeights, channel: FusionChannel, value: f64) {
    match channel {
        FusionChannel::Lexical => weights.lexical = value,
        FusionChannel::Definition => weights.def = value,
        FusionChannel::Caller => weights.caller = value,
        FusionChannel::Graph => weights.graph = value,
        FusionChannel::Anchor => weights.anchor = value,
        FusionChannel::Semantic => weights.embed = value,
        FusionChannel::Pattern => weights.pattern = value,
        FusionChannel::Import => weights.import = value,
    }
}

/// INTENT: single-channel rank fixture — every channel absent except
/// `channel` at `rank`. Pure constructor.
pub fn single_rank(channel: FusionChannel, rank: usize) -> ChannelRanks {
    let mut ranks = ChannelRanks::default();
    set_rank(&mut ranks, channel, Some(rank));
    ranks
}

/// INTENT: canonical worse-first lexical pair (ranks 1 vs 0) — the shared
/// learn/sensitivity fixture. Pure constructor.
pub fn pair_examples() -> Vec<FusionExample> {
    vec![FusionExample {
        query: "q".to_string(),
        candidates: vec![
            FusionCandidate {
                id: "worse".to_string(),
                relevance: 0.0,
                ranks: single_rank(FusionChannel::Lexical, 1),
            },
            FusionCandidate {
                id: "better".to_string(),
                relevance: 1.0,
                ranks: single_rank(FusionChannel::Lexical, 0),
            },
        ],
    }]
}

/// INTENT: mixed 8-channel corpus plus a 9th Def hit — the
/// permutation/determinism corpus (every channel present, distinct raws).
/// Pure constructor.
pub fn mixed_hits() -> Vec<SearchHit> {
    vec![
        crate::mk_hit(HitKind::Caller, "m.rs", 7, 8.0),
        crate::mk_hit(HitKind::Def, "m.rs", 7, 5.0),
        crate::mk_hit(HitKind::Asgrep, "a.rs", 1, 3.0),
        crate::mk_hit(HitKind::Graph, "g.rs", 2, 6.0),
        crate::mk_hit(HitKind::Anchor, "n.rs", 3, 1.0),
        crate::mk_hit(HitKind::Embed, "e.rs", 4, 2.0),
        crate::mk_hit(HitKind::Pattern, "p.rs", 5, 4.0),
        crate::mk_hit(HitKind::Import, "i.rs", 6, 7.0),
        crate::mk_hit(HitKind::Def, "z.rs", 9, 9.0),
    ]
}

/// INTENT: fused-keys runner — fuse a copy of `hits` under default (unit)
/// weights and project every row to [`crate::fused_key`]. The unit weights
/// are the identity, so this pins input-order irrelevance, not weight
/// application (weighted legs use [`route_fuse_pipeline`]).
pub fn fused_keys(hits: &[SearchHit]) -> Vec<(HitKind, String, u32, u32, u64, Vec<HitKind>)> {
    let mut fused = hits.to_vec();
    apply_weighted_rrf(&mut fused, &ChannelWeights::default());
    fused.iter().map(crate::fused_key).collect()
}

/// INTENT: scoring-drill pipeline runner — route raw producer scores
/// through per-hit ceilings, then fuse under caller weights. Pure runner
/// (mutates only its owned copy).
pub fn route_fuse_pipeline(
    parsed: &ParsedQuery,
    hits: Vec<SearchHit>,
    weights: &ChannelWeights,
) -> Vec<SearchHit> {
    let mut hits = hits;
    route_hits(parsed, &mut hits);
    apply_weighted_rrf(&mut hits, weights);
    hits
}

/// INTENT: final fused ranking — score descending with (file, line)
/// tiebreaks. Pure runner. Delta vs `lang_pipeline::rank_hits`: that ranks
/// `PatternMatch` by an in-test score lane; this ranks fused `SearchHit`s
/// by their fused score.
pub fn rank_fused(mut fused: Vec<SearchHit>) -> Vec<SearchHit> {
    fused.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line_start.cmp(&b.line_start))
    });
    fused
}

/// INTENT: order-sensitive file projection for exact-order asserts. Pure
/// projection. Delta vs [`sorted_hit_files`]: that sorts (order-free);
/// this keeps rank order.
pub fn hit_files_in_order(hits: &[SearchHit]) -> Vec<String> {
    hits.iter().map(|h| h.file.clone()).collect()
}

/// INTENT: crossover corpus — opposite within-channel orders per file
/// (Asgrep A > B, routed Def A < B) for the weight-tilt drill. Pure
/// constructor.
pub fn crossover_corpus() -> Vec<SearchHit> {
    // Asgrep raws A=3.0 > B=2.0; Def raws A=6.5 (routed 0.5) < B=13.0
    // (routed 1.0, both symbol "foo", ceiling 13). Opposite within-channel
    // orders: A = (lex 0, def 1), B = (lex 1, def 0).
    let mut a_def = crate::mk_hit(HitKind::Def, "a.rs", 1, 6.5);
    a_def.symbol = Some("foo".to_string());
    let mut b_def = crate::mk_hit(HitKind::Def, "b.rs", 1, 13.0);
    b_def.symbol = Some("foo".to_string());
    vec![
        crate::mk_hit(HitKind::Asgrep, "a.rs", 1, 3.0),
        a_def,
        crate::mk_hit(HitKind::Asgrep, "b.rs", 1, 2.0),
        b_def,
    ]
}

/// INTENT: clamp-tie corpus — 100x-different raws collapsing to 1.0 in
/// routing, so within-channel ranks fall back to (file, line) tiebreaks.
/// Pure constructor.
pub fn tie_corpus() -> Vec<SearchHit> {
    // Def raws 13/26/130/1300 (symbol "foo", ceiling 13) all route to 1.0
    // (1.0, then 2/10/100 clamped); Caller raws 11.5/115 (callee "foo",
    // ceiling 11.5) both route to 1.0. Within-channel ranks fall back to
    // (file, line): def a=0 b=1 c=2 d=3; caller a=0 b=1.
    let mut defs = Vec::new();
    for (file, raw) in [("a.rs", 13.0), ("b.rs", 26.0), ("c.rs", 130.0), ("d.rs", 1300.0)] {
        let mut hit = crate::mk_hit(HitKind::Def, file, 1, raw);
        hit.symbol = Some("foo".to_string());
        defs.push(hit);
    }
    let mut a_caller = crate::mk_hit(HitKind::Caller, "a.rs", 1, 11.5);
    a_caller.callee = Some("foo".to_string());
    let mut b_caller = crate::mk_hit(HitKind::Caller, "b.rs", 1, 115.0);
    b_caller.callee = Some("foo".to_string());
    defs.push(a_caller);
    defs.push(b_caller);
    defs
}

/// INTENT: multi-term/spelled [`ParsedQuery`] fixture — `ParsedQuery::parse`
/// cannot express "Foo bar" raw with lowercase terms, so routing legs
/// needing the identifier-spelling branch build the struct directly. Pure
/// constructor.
pub fn parsed_query(raw: &str, terms: &[&str]) -> ParsedQuery {
    ParsedQuery {
        raw: raw.to_string(),
        mode: QueryMode::Hybrid,
        target: None,
        terms: terms.iter().map(|t| t.to_string()).collect(),
        path_scope: None,
        path_scope_error: None,
        path_scope_exact: false,
    }
}

fn chain_base<'a>(
    path: &'a str,
    lines: &'a [(u32, String)],
    hash: &'a str,
    symbols: &'a [SymbolRow],
    callers: &'a [CallerRow],
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("rust"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols,
        callers,
        imports: &[],
        pattern_nodes: &[],
        depth_truncated: false,
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
    }
}

/// INTENT: two-file caller->callee store (`caller.rs` FooBar calls `baz`
/// in `callee.rs`) — the chain-decay fixture. Panics on index failure.
pub fn chain_store(dir: &tempfile::TempDir) -> IndexStore {
    let store = IndexStore::open(dir.path(), None).unwrap();
    let caller_symbols = [SymbolRow {
        name: "FooBar".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 24,
    }];
    let callers = [CallerRow {
        caller: "FooBar".into(),
        callee: "Baz".into(),
        line_no: 1,
        byte_start: 14,
        byte_end: 17,
    }];
    store
        .upsert_file(chain_base(
            "caller.rs",
            &[(1, "fn FooBar() { Baz(); }".into())],
            "caller-hash",
            &caller_symbols,
            &callers,
        ))
        .unwrap();
    let callee_symbols = [SymbolRow {
        name: "baz".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 11,
    }];
    store
        .upsert_file(chain_base(
            "callee.rs",
            &[(1, "fn baz() {}".into())],
            "callee-hash",
            &callee_symbols,
            &[],
        ))
        .unwrap();
    store
}

/// INTENT: all-ones channel weights — the neutral element every weighted-RRF
/// facet builds from. Pure constructor.
pub fn unit_channel_weights() -> ChannelWeights {
    ChannelWeights {
        lexical: 1.0,
        def: 1.0,
        caller: 1.0,
        graph: 1.0,
        anchor: 1.0,
        embed: 1.0,
        pattern: 1.0,
        import: 1.0,
    }
}
