use super::super::fusion::{FileCsr, NameIntern};
use crate::query::ParsedQuery;
use crate::rank::{
    best_symbol_score_normalized, max_normalize_idf, normalize_query_terms, score_caller_weighted,
    score_def, score_def_weighted, term_idf, SCORE_ANCHOR, SCORE_GRAPH,
};
use crate::search::types::matches_lang;
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::bundle_graph::file_tally;
use crate::store::sql::{
    caller_terms_filter, escape_like_term, like_terms_filter, query_limit_map,
};
use crate::store::IndexStore;
use crate::Result;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
const SYMBOL_SQL_LIMIT: usize = 500;
const CALLER_SQL_LIMIT: usize = 500;
const MODE_SQL_LIMIT: usize = 200;

fn mode_sql_limit(options: &SearchOptions) -> usize {
    MODE_SQL_LIMIT.max(options.retained_limit())
}
const TYPE_SYMBOL_WEIGHT: f64 = 0.65;

/// Def rows in a file whose stem shares a distinctive query term
/// (`default_types.rs` <- "types", `gitignore.rs` <- "gitignore") carry
/// direct textual evidence for the query. TYPE_SYMBOL_WEIGHT (0.65)
/// otherwise truncates statics/consts off the retained window before
/// fusion — the critic never sees them. 1.7 > 1/0.65 so an equal-term
/// stem-matching static outranks the function crowd.
const FILE_STEM_DEF_AFFINITY: f64 = 1.7;

/// Distinctive query terms eligible for file-stem affinity: 4+ chars,
/// not generic concept tokens, not English function words. Shared by the
/// def-row score boost and the stem-affine admission pass below.
fn def_stem_terms(parsed: &ParsedQuery) -> Vec<&str> {
    parsed
        .terms
        .iter()
        .filter(|term| {
            term.chars().count() >= 4
                && !crate::search::critic::is_generic_concept_token(term)
                && !crate::search::cascade_stopword(term)
        })
        .map(String::as_str)
        .collect()
}

/// Files whose stem contains a distinctive query term
/// (`default_types.rs` <- "types"): direct textual evidence for the query.
fn stem_affine_files(parsed: &ParsedQuery, allowed_files: &HashSet<String>) -> HashSet<String> {
    let stem_terms = def_stem_terms(parsed);
    if stem_terms.is_empty() {
        return HashSet::new();
    }
    allowed_files
        .iter()
        .filter(|path| {
            let stem = file_stem_lower(path);
            stem_terms.iter().any(|term| stem.contains(term))
        })
        .cloned()
        .collect()
}

fn file_stem_lower(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    };
    stem.to_ascii_lowercase()
}
const SYMBOL_KIND_ORDER: &str =
    " ORDER BY CASE WHEN s.kind IN ('function','method') THEN 0 ELSE 1 END, s.id";
/// Path-deterministic twin of [`cmp_symbol_rows`] for the hybrid lane: the
/// cold window must sort (kind group, path, line, name) like the warmed
/// table, never rowid order (parallel indexing scrambles `s.id`).
const SYMBOL_PATH_ORDER: &str = " ORDER BY CASE WHEN s.kind IN ('function','method') THEN 0 WHEN s.kind IN ('type','enum','class','interface','struct') THEN 1 ELSE 2 END, f.path, s.line_start, s.name";
const SYMBOL_SELECT: &str = "SELECT f.path, f.language, s.name, s.kind, s.line_start, s.line_end
         FROM symbols s JOIN files f ON f.id = s.file_id";
const CALLER_SELECT: &str = "SELECT f.path, f.language, c.caller, c.callee, c.line_no
         FROM callers c JOIN files f ON f.id = c.file_id";
type CallerQueryRow = (String, Option<String>, String, String, u32);
type CallerFilter = fn(&[String], Option<&str>) -> (String, Vec<String>);
pub type SymbolSpanRow = (String, Option<String>, String, String, u32, u32);

/// Snapshot-held unique hybrid: one SQL load of defs, then RAM substring match.
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct WarmedSymbolTable {
    rows: Arc<Vec<SymbolSpanRow>>,
    names_lower: Arc<Vec<String>>,
    by_file: Arc<FileCsr>,
}

#[allow(dead_code)]
impl WarmedSymbolTable {
    pub(crate) fn load(store: &IndexStore) -> Result<Self> {
        const WARMED_SYMBOL_CAP: usize = 262_144;
        let rows = query_symbol_spans(store, "", Vec::new(), SYMBOL_KIND_ORDER, WARMED_SYMBOL_CAP)?;
        let names_lower: Vec<String> = rows.iter().map(|row| row.2.to_lowercase()).collect();
        let by_file = FileCsr::build(
            rows.iter()
                .enumerate()
                .map(|(i, row)| (row.0.as_str(), i as u32))
                .collect(),
        );
        Ok(Self {
            rows: Arc::new(rows),
            names_lower: Arc::new(names_lower),
            by_file: Arc::new(by_file),
        })
    }

    fn matches_for_files(
        &self,
        options: &SearchOptions,
        terms: &[String],
        allowed_files: &HashSet<String>,
        limit: usize,
    ) -> Vec<SymbolSpanRow> {
        let terms_lower: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
        // Select indices first, materialize rows last: the old shape cloned every
        // substring match (hundreds on common terms) before sorting and truncating
        // to `limit`. Same iteration order, same sort keys, same quotas, only the
        // rows that survive the budget are cloned.
        let mut matched: Vec<usize> = Vec::new();
        for path in allowed_files {
            for &i in self.by_file.get(path) {
                let i = i as usize;
                if !matches_lang(self.rows[i].1.as_deref(), options.lang_filter.as_deref()) {
                    continue;
                }
                if terms_lower
                    .iter()
                    .any(|term| self.names_lower[i].contains(term.as_str()))
                {
                    matched.push(i);
                }
            }
        }
        let rows = &self.rows;
        matched.sort_by(|&left, &right| cmp_symbol_rows(&rows[left], &rows[right]));
        // Quotas precede admission; term-fairness applies within each quota
        // bucket (see `quota_term_fair_partition`). Mirrors the cold SQL
        // lane's three fair arms.
        let mut matched = quota_term_fair_partition(
            matched,
            |&i| kind_rank(rows[i].3.as_str()),
            |&i| {
                terms_lower
                    .iter()
                    .position(|term| self.names_lower[i].contains(term.as_str()))
                    .unwrap_or(terms_lower.len())
            },
            limit,
        );
        matched.sort_by(|&left, &right| cmp_symbol_rows(&rows[left], &rows[right]));
        matched.into_iter().map(|i| rows[i].clone()).collect()
    }

    fn term_cooccurrences(&self, norm_terms: &[String]) -> Vec<u64> {
        cooccurrences_over_names(norm_terms, self.names_lower.iter())
    }

    /// Per-term IDF weights over the warmed table (the whole symbol table):
    /// one pass counting, per normalized term, how many names contain it —
    /// the same predicate as matching. Mirrors [`name_table_idf_weights`]
    /// so warm and cold lanes score defs identically.
    fn idf_weights(&self, normalized_terms: &[String]) -> Vec<f64> {
        self.idf_weights_with_dfs(normalized_terms).0
    }

    /// [`Self::idf_weights`] plus the raw per-term document counts, so
    /// callers can distinguish "term is rare" from "term names no symbol
    /// at all" (verb-alias discovery gates on df == 0, and a df of zero
    /// maximizes IDF, so weights alone cannot make that distinction).
    fn idf_weights_with_dfs(&self, normalized_terms: &[String]) -> (Vec<f64>, Vec<u64>) {
        let mut dfs = vec![0u64; normalized_terms.len()];
        for name in self.names_lower.iter() {
            for (i, term) in normalized_terms.iter().enumerate() {
                if name.contains(term.as_str()) {
                    dfs[i] += 1;
                }
            }
        }
        let total = self.names_lower.len() as u64;
        let mut weights: Vec<f64> = dfs.iter().map(|&df| term_idf(df, total)).collect();
        max_normalize_idf(&mut weights);
        (weights, dfs)
    }
}

pub fn kind_rank(kind: &str) -> u8 {
    match kind {
        "function" | "method" => 0,
        "type" | "enum" | "class" | "interface" | "struct" => 1,
        _ => 2,
    }
}

fn first_containing_term(terms: &[String], name: &str) -> usize {
    let lower = name.to_ascii_lowercase();
    terms
        .iter()
        .position(|term| lower.contains(term.to_ascii_lowercase().as_str()))
        .unwrap_or(terms.len())
}

fn packed_hybrid_def_rows(
    bundle: &crate::store::read_bundle::ReadBundle,
    terms: &[String],
    allowed_files: &HashSet<String>,
    lang: Option<&str>,
    limit: usize,
    affine: &HashSet<String>,
) -> Vec<SymbolSpanRow> {
    let mut all = crate::store::bundle_graph::symbol_matches_for_files(
        bundle,
        terms,
        allowed_files,
        lang,
        usize::MAX,
    );
    all.sort_by(cmp_symbol_rows);
    let affine_src = if affine.is_empty() {
        None
    } else {
        Some(all.clone())
    };
    let mut rows = quota_term_fair_partition(
        all,
        |row| kind_rank(row.3.as_str()),
        |row| first_containing_term(terms, &row.2),
        limit,
    );
    rows.sort_by(cmp_symbol_rows);
    rows = quota_partition(rows, |row| kind_rank(row.3.as_str()), limit);
    if let Some(src) = affine_src {
        let mut seen: HashSet<(String, u32, String)> = rows
            .iter()
            .map(|row| (row.0.clone(), row.4, row.2.clone()))
            .collect();
        let before = rows.len();
        for row in src
            .into_iter()
            .filter(|row| affine.contains(&row.0))
            .take(32)
        {
            if seen.insert((row.0.clone(), row.4, row.2.clone())) {
                rows.push(row);
            }
        }
        if rows.len() != before {
            rows.sort_by(cmp_symbol_rows);
        }
    }
    rows
}

/// Canonical symbol-row order shared by the warmed table and the cold SQL
/// lane: (kind group, path, line, name). Deterministic across index builds
/// — never rowid order, which parallel indexing scrambles.
pub fn cmp_symbol_rows(left: &SymbolSpanRow, right: &SymbolSpanRow) -> std::cmp::Ordering {
    kind_rank(left.3.as_str())
        .cmp(&kind_rank(right.3.as_str()))
        .then_with(|| left.0.cmp(&right.0))
        .then_with(|| left.4.cmp(&right.4))
        .then_with(|| left.2.cmp(&right.2))
}

fn minority_slot_limit(limit: usize) -> usize {
    ((limit / 2).max(4)).min(limit)
}

/// Quota buckets shared by the warmed table and the cold SQL lane: input
/// must already be in [`cmp_symbol_rows`] order; output is functions first,
/// then reserved type/other slots. Without the reservation, functions crowd
/// minority kinds off over-budget pages. Groups are [`kind_rank`] codes
/// (0 = function/method, 1 = type family, 2 = other) so callers can bucket
/// indices without borrowing rows.
pub fn quota_partition<T>(items: Vec<T>, group_of: impl Fn(&T) -> u8, limit: usize) -> Vec<T> {
    // Functions otherwise spend the whole budget before enums/structs
    // (`Durability`, `SnapshotStamp`) that conceptual gold names.
    let other_keep = minority_slot_limit(limit);
    let mut fns = Vec::new();
    let mut types = Vec::new();
    let mut others = Vec::new();
    for item in items {
        match group_of(&item) {
            0 => fns.push(item),
            1 => types.push(item),
            _ => others.push(item),
        }
    }
    types.truncate(other_keep);
    let leftover = other_keep.saturating_sub(types.len());
    others.truncate(leftover);
    fns.truncate(limit.saturating_sub(types.len() + others.len()));
    fns.into_iter().chain(types).chain(others).collect()
}

/// Combined admission for multi-source fairness: kind quotas partition the
/// budget first — minorities are reserved from their full bucket, exactly
/// as [`quota_partition`] — and each bucket then fills its budget
/// term-fairly via [`term_fair_select`]. Running term-fair truncation over
/// the whole list first would re-create the starvation quotas exist to
/// prevent: canonical order is kind-first, so a flooded term's function
/// rows would spend the window before any minority row is considered.
/// Input must be in [`cmp_symbol_rows`] order; output is functions, then
/// types, then others (round-robin order within each bucket; callers
/// re-sort canonically). The cold SQL lane implements the same policy as
/// three fair arms in `query_quota_symbol_spans`; keep them in lockstep.
pub fn quota_term_fair_partition<T>(
    items: Vec<T>,
    group_of: impl Fn(&T) -> u8,
    term_of: impl Fn(&T) -> usize,
    limit: usize,
) -> Vec<T> {
    let other_keep = minority_slot_limit(limit);
    let mut fns = Vec::new();
    let mut types = Vec::new();
    let mut others = Vec::new();
    for item in items {
        match group_of(&item) {
            0 => fns.push(item),
            1 => types.push(item),
            _ => others.push(item),
        }
    }
    let types = term_fair_select(types, |item| term_of(item), other_keep);
    let leftover = other_keep.saturating_sub(types.len());
    let others = term_fair_select(others, |item| term_of(item), leftover);
    let fn_budget = limit.saturating_sub(types.len() + others.len());
    let fns = term_fair_select(fns, |item| term_of(item), fn_budget);
    fns.into_iter().chain(types).chain(others).collect()
}

/// Term-fair admission over a canonically sorted match list: when matches
/// exceed the row budget, draw rows round-robin across per-term groups
/// (`term_of` = index of the first query term the row matches) instead of
/// truncating in canonical order. Path-order truncation lets one flooded
/// term ("run" matching hundreds of defs in early paths) spend the whole
/// window before a rare term's defs are ever scored; interleaving keeps
/// every term represented while still filling the budget. Under budget
/// this is the identity. Applied per kind-quota bucket by
/// [`quota_term_fair_partition`]; the cold SQL lane implements the same
/// policy with ROW_NUMBER partitioned by (kind bucket, term) (see
/// `query_quota_symbol_spans`); the two must stay in lockstep or
/// warm/cold results diverge.
pub fn term_fair_select<T>(items: Vec<T>, term_of: impl Fn(&T) -> usize, limit: usize) -> Vec<T> {
    if items.len() <= limit {
        return items;
    }
    let mut groups: Vec<VecDeque<T>> = Vec::new();
    for item in items {
        let term = term_of(&item);
        if groups.len() <= term {
            groups.resize_with(term + 1, VecDeque::new);
        }
        groups[term].push_back(item);
    }
    let mut out = Vec::with_capacity(limit);
    while out.len() < limit {
        let mut progressed = false;
        for group in &mut groups {
            if out.len() == limit {
                break;
            }
            if let Some(item) = group.pop_front() {
                out.push(item);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

/// Per-term IDF weights over a name column (`symbols.name` for defs,
/// distinct `callers.callee` for caller hits): one scan counts, per
/// normalized term, how many names contain it under the same ASCII
/// case-insensitive containment predicate as matching. The population is the
/// name vocabulary — one entry per distinct name — so a callee called from
/// 500 sites does not make its tokens 500x more common as name evidence (and
/// the distinct scan stays cheap where the raw callers scan is not on this
/// repo). `source`/`column` are internal constants, never user input.
/// Mirrors `WarmedSymbolTable::idf_weights` so warm and cold lanes score
/// identically.
///
/// Counting runs in Rust over one streamed scan of the vocabulary instead of
/// per-term SQL `LIKE '%term%'` aggregates: SQLite's LIKE loop walked every
/// row once per term inside the SUM(CASE) projection, while one
/// `content_matches_literal` sweep per row (memchr2-accelerated) is far
/// cheaper. For ASCII terms and ASCII-case folds the predicate is the same
/// class SQLite's LIKE implements; for non-ASCII case folds it now agrees
/// with `WarmedSymbolTable::idf_weights` (Rust lowercase folding), which the
/// SQL LIKE lane never did — this makes the warm/cold idf contract hold
/// exactly instead of approximately.
fn name_table_idf_weights(
    store: &IndexStore,
    population: crate::names_sidecar::NamePopulation,
    normalized_terms: &[String],
) -> Result<Vec<f64>> {
    Ok(name_table_idf_weights_with_dfs(store, population, normalized_terms)?.0)
}

/// [`name_table_idf_weights`] plus the raw per-term document counts
/// (verb-alias discovery gates on df == 0; see
/// [`WarmedSymbolTable::idf_weights_with_dfs`]).
fn name_table_idf_weights_with_dfs(
    store: &IndexStore,
    population: crate::names_sidecar::NamePopulation,
    normalized_terms: &[String],
) -> Result<(Vec<f64>, Vec<u64>)> {
    if normalized_terms.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    // Sidecar lane: folded vocabularies mmap'd once per store, exact
    // df/total inputs (generation-gated; any doubt falls through to SQL).
    if let Some((dfs, total)) =
        crate::names_sidecar::sidecar_dfs_for_store(store, population, normalized_terms)
    {
        let mut weights: Vec<f64> = dfs.iter().map(|&df| term_idf(df, total)).collect();
        max_normalize_idf(&mut weights);
        return Ok((weights, dfs));
    }
    let (source, column) = population.sql_source_and_column();
    let sql = format!("SELECT {column} FROM {source}");
    let conn = store.connection();
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query([])?;
    let mut dfs = vec![0u64; normalized_terms.len()];
    let mut total: u64 = 0;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        total += 1;
        for (i, term) in normalized_terms.iter().enumerate() {
            if crate::search::passes::literal::content_matches_literal(
                &name,
                term,
                Some(term),
                false,
            ) {
                dfs[i] += 1;
            }
        }
    }
    let mut weights: Vec<f64> = dfs.iter().map(|&df| term_idf(df, total)).collect();
    max_normalize_idf(&mut weights);
    Ok((weights, dfs))
}

enum CallerMatchMode {
    Hybrid,
    CalleeOnly,
}
fn map_caller_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CallerQueryRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}
fn restrict_to_files(
    where_clause: &mut String,
    bind: &mut Vec<String>,
    allowed_files: Option<&HashSet<String>>,
) {
    let Some(allowed_files) = allowed_files else {
        return;
    };
    let mut paths: Vec<String> = allowed_files.iter().cloned().collect();
    paths.sort_unstable();
    // Quantize the placeholder count up to a power of two by repeating
    // the last path. IN-membership is unchanged by
    // duplicates, and the SQL text becomes stable within a bucket so
    // prepare_cached stops re-parsing a fresh statement per distinct file
    // count (the tail-profile showed sqlite3RunParser/yy_reduce churn from
    // per-count statement text).
    //
    // gauntlet-r11 fix: round UP with `n.next_power_of_two()`. The old
    // `(n - 1).next_power_of_two()` produced bucket < n when n was exactly a
    // power of two plus one (n=9 -> 8), leaving 8 placeholders while all 9
    // paths were bound — "Wrong number of parameters passed to query" for any
    // hybrid query whose prefilter survived exactly 2^k + 1 files.
    let n = paths.len();
    if n == 0 {
        // An empty allow-list admits nothing; produce a valid, always-false
        // predicate instead of the old malformed `IN ()`.
        where_clause.push_str(" AND 0 = 1");
        return;
    }
    let bucket = n.next_power_of_two().max(8);
    where_clause.push_str(" AND f.path IN (");
    where_clause.push_str(&vec!["?"; bucket].join(","));
    where_clause.push(')');
    if bucket > n {
        if let Some(last) = paths.last().cloned() {
            for _ in n..bucket {
                paths.push(last.clone());
            }
        }
    }
    debug_assert_eq!(paths.len(), bucket);
    bind.extend(paths);
}

fn query_caller_rows(
    store: &IndexStore,
    filter: CallerFilter,
    terms: &[String],
    lang_filter: Option<&str>,
    allowed_files: Option<&HashSet<String>>,
    limit: usize,
) -> Result<Vec<CallerQueryRow>> {
    let (mut where_clause, mut bind) = filter(terms, lang_filter);
    restrict_to_files(&mut where_clause, &mut bind, allowed_files);
    let sql = format!("{CALLER_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    query_limit_map(store.connection(), &sql, bind, limit, map_caller_row)
}
/// Convert caller and graph evidence, optionally attaching excerpts and
/// classifying resolution when a store is supplied for candidate counts.
fn caller_rows_to_hits(
    excerpt_store: &IndexStore,
    rows: Vec<CallerQueryRow>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    mode: CallerMatchMode,
    store: Option<&IndexStore>,
    attach_excerpts: bool,
) -> Result<Vec<SearchHit>> {
    let primary_lower = parsed.primary_symbol().map(|s| s.to_lowercase());
    // am6l: normalize query terms once per query, not once per scored row.
    let norm_terms = normalize_query_terms(&parsed.terms);
    // Rarity weights over the distinct-callee vocabulary: a callee
    // matching a term few callees carry is stronger evidence than one
    // matching a flooded term. Same IDF policy as the def lane. Skipped
    // when there is nothing to score.
    let idf_weights = if rows.is_empty() {
        Vec::new()
    } else if let Ok(Some(bundle)) = excerpt_store.read_bundle(None) {
        crate::store::bundle_graph::callee_idf_weights(&bundle, &norm_terms)
    } else {
        name_table_idf_weights(
            excerpt_store,
            crate::names_sidecar::NamePopulation::DistinctCallees,
            &norm_terms,
        )?
    };
    let mut caller_hits = Vec::new();
    let mut graph_hits = Vec::new();
    for (path, language, caller, callee, line_no) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        let callee_score = best_symbol_score_normalized(&norm_terms, &callee);
        let matched = match mode {
            CallerMatchMode::Hybrid => {
                callee_score > 0.0 || best_symbol_score_normalized(&norm_terms, &caller) > 0.0
            }
            CallerMatchMode::CalleeOnly => callee_score > 0.0,
        };
        if !matched {
            continue;
        }
        caller_hits.push(SearchHit::caller(
            path.clone(),
            language.clone(),
            caller.clone(),
            callee.clone(),
            line_no,
            String::new(),
            score_caller_weighted(&norm_terms, &idf_weights, &callee),
        ));
        let graph = match mode {
            CallerMatchMode::CalleeOnly => Some(SCORE_GRAPH),
            CallerMatchMode::Hybrid => {
                let exact = callee_score >= crate::rank::SCORE_EXACT_SYMBOL
                    || primary_lower
                        .as_ref()
                        .is_some_and(|s| callee.to_lowercase() == *s);
                exact.then_some(SCORE_GRAPH)
            }
        };
        if let Some(graph_score) = graph {
            graph_hits.push(SearchHit::graph_scored(
                path,
                language,
                caller,
                callee,
                line_no,
                graph_score,
            ));
        }
    }
    retain_scored_hits(&mut caller_hits, options);
    retain_scored_hits(&mut graph_hits, options);
    if attach_excerpts {
        attach_indexed_excerpts(excerpt_store, &mut caller_hits)?;
    }
    if let Some(store) = store {
        let mut intern = NameIntern::new(caller_hits.len() + graph_hits.len());
        let mut slots: Vec<Option<(Vec<(String, usize)>, usize)>> = Vec::new();
        let scip_refs = store.scip_fact_set(false)?;
        attach_caller_resolutions(store, &mut intern, &mut slots, &scip_refs, &mut caller_hits)?;
        attach_caller_resolutions(store, &mut intern, &mut slots, &scip_refs, &mut graph_hits)?;
    }
    caller_hits.extend(graph_hits);
    Ok(caller_hits)
}

fn retain_scored_hits(hits: &mut Vec<SearchHit>, options: &SearchOptions) {
    // Keep a scoring pool above `options.limit`. TYPE_SYMBOL_WEIGHT (0.65)
    // otherwise truncates concept types (`SnapshotStamp`) before the critic.
    let limit = options.retained_limit().max(32);
    hits.sort_unstable_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    hits.truncate(limit);
}

pub(crate) fn attach_indexed_excerpts_if_empty(
    store: &IndexStore,
    hits: &mut [SearchHit],
) -> Result<()> {
    for hit in hits.iter_mut() {
        if !hit.excerpt.is_empty() {
            continue;
        }
        hit.excerpt = store.indexed_excerpt_in_range(&hit.file, hit.line_start, hit.line_end)?;
    }
    Ok(())
}
pub(crate) fn attach_indexed_excerpts(store: &IndexStore, hits: &mut [SearchHit]) -> Result<()> {
    for hit in hits {
        hit.excerpt = store.indexed_excerpt_in_range(&hit.file, hit.line_start, hit.line_end)?;
    }
    Ok(())
}

fn attach_caller_resolutions(
    store: &IndexStore,
    intern: &mut NameIntern,
    slots: &mut Vec<Option<(Vec<(String, usize)>, usize)>>,
    scip_refs: &[(String, u32, String)],
    hits: &mut [SearchHit],
) -> Result<()> {
    for hit in hits {
        let Some(callee) = hit.callee.as_deref() else {
            continue;
        };
        let id = intern.intern(callee) as usize;
        while slots.len() <= id {
            slots.push(None);
        }
        if slots[id].is_none() {
            slots[id] = Some(store.symbol_name_candidate_counts(callee)?);
        }
        let (by_file, repo) = slots[id].as_ref().expect("slot filled above");
        let same_file = file_tally(by_file, &hit.file);
        let counted =
            crate::resolution::Resolution::from_candidates(same_file, *repo, std::iter::empty());
        hit.resolution = Some(if scip_has(scip_refs, &hit.file, hit.line_start, callee) {
            counted.upgrade(crate::resolution::Resolution::ScipOccurrence)
        } else {
            counted
        });
    }
    Ok(())
}
fn read_symbol_span_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolSpanRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}
fn query_symbol_spans(
    store: &IndexStore,
    where_clause: &str,
    bind: Vec<String>,
    order: &str,
    limit: usize,
) -> Result<Vec<SymbolSpanRow>> {
    let sql = format!(
        "{SYMBOL_SELECT}{where_clause}{order} LIMIT ?{}",
        bind.len() + 1
    );
    query_limit_map(store.connection(), &sql, bind, limit, read_symbol_span_row)
}
/// Bare-column within-bucket order for CTEs that already materialized
/// aliased columns (no `s.`/`f.` prefixes available). Kind precedence is
/// carried by the bucket partition, so only path order remains.
const FAIR_WITHIN_ORDER: &str = " ORDER BY path, line_start, name";

/// Admit minorities before functions in one SQLite snapshot, rather than
/// cutting a function-first global window before the quota can see them.
fn query_quota_symbol_spans(
    store: &IndexStore,
    terms: &[String],
    where_clause: &str,
    mut bind: Vec<String>,
    limit: usize,
) -> Result<Vec<SymbolSpanRow>> {
    let other_keep = minority_slot_limit(limit);
    if terms.len() < 2 {
        // Each generated filter contains only anonymous placeholders. Repeat
        // its bound values for the two arms; the final numbered parameter is
        // the shared row budget. Only the typed, bounded quota is interpolated.
        let budget_param = bind.len() * 2 + 1;
        bind.extend_from_within(..);
        let sql = format!(
            "WITH minority AS MATERIALIZED (
                 {SYMBOL_SELECT}{where_clause} AND s.kind NOT IN ('function','method')
                 {SYMBOL_PATH_ORDER} LIMIT {other_keep}
             ), functions AS (
                 {SYMBOL_SELECT}{where_clause} AND s.kind IN ('function','method')
                 {SYMBOL_PATH_ORDER} LIMIT (?{budget_param} - (SELECT count(*) FROM minority))
             )
             SELECT * FROM functions UNION ALL SELECT * FROM minority"
        );
        return query_limit_map(store.connection(), &sql, bind, limit, read_symbol_span_row);
    }
    // Quota-then-fair admission, mirroring `quota_term_fair_partition` on
    // the warmed lane: each row carries its kind bucket and the index of
    // the first query term it matches (the WHERE clause ORs the same
    // patterns, so the CASE always fires); ROW_NUMBER ranks within each
    // (bucket, term) group in path order; each bucket arm then picks by
    // (rn, term_idx), interleaving its term groups round-robin within the
    // bucket's quota budget. Term-fair truncation over the whole match set
    // first would starve the minority quotas: canonical order is
    // kind-first, so a flooded term's functions would spend the window
    // before any type row is considered. The CASE binds precede the WHERE
    // binds in statement order; the final numbered parameter is the shared
    // row budget.
    let case_arms: Vec<String> = terms
        .iter()
        .enumerate()
        .map(|(idx, _)| format!("WHEN s.name LIKE '%' || ? || '%' ESCAPE '\\' THEN {idx}"))
        .collect();
    let mut binds: Vec<String> = terms.iter().map(|term| escape_like_term(term)).collect();
    let budget_param = binds.len() + bind.len() + 1;
    binds.append(&mut bind);
    let sql = format!(
        "WITH matched AS MATERIALIZED (
             SELECT f.path AS path, f.language AS language, s.name AS name,
                    s.kind AS kind, s.line_start AS line_start, s.line_end AS line_end,
                    (CASE {} END) AS term_idx,
                    (CASE WHEN s.kind IN ('function','method') THEN 0
                          WHEN s.kind IN ('type','enum','class','interface','struct') THEN 1
                          ELSE 2 END) AS kind_bucket
             FROM symbols s JOIN files f ON f.id = s.file_id{where_clause}
         ), fair AS MATERIALIZED (
             SELECT path, language, name, kind, line_start, line_end, term_idx, kind_bucket,
                    ROW_NUMBER() OVER (PARTITION BY kind_bucket, term_idx{FAIR_WITHIN_ORDER}) AS rn
             FROM matched
         ), types AS MATERIALIZED (
             SELECT path, language, name, kind, line_start, line_end
             FROM fair WHERE kind_bucket = 1 ORDER BY rn ASC, term_idx ASC LIMIT {other_keep}
         ), others AS MATERIALIZED (
             SELECT path, language, name, kind, line_start, line_end
             FROM fair WHERE kind_bucket = 2 ORDER BY rn ASC, term_idx ASC
             LIMIT ({other_keep} - (SELECT count(*) FROM types))
         ), functions AS (
             SELECT path, language, name, kind, line_start, line_end
             FROM fair WHERE kind_bucket = 0 ORDER BY rn ASC, term_idx ASC
             LIMIT (?{budget_param} - (SELECT count(*) FROM types) - (SELECT count(*) FROM others))
         )
         SELECT * FROM functions UNION ALL SELECT * FROM types UNION ALL SELECT * FROM others",
        case_arms.join(" ")
    );
    query_limit_map(store.connection(), &sql, binds, limit, read_symbol_span_row)
}

fn kind_weight(kind: &str) -> f64 {
    match kind {
        "function" | "method" => 1.0,
        _ => TYPE_SYMBOL_WEIGHT,
    }
}

/// Exact-case identifier names beat folded homonyms and never take the type penalty.
fn symbol_match_weight(query_spelling: Option<&str>, name: &str, kind: &str) -> f64 {
    let kind_w = kind_weight(kind);
    let Some(query) = query_spelling.filter(|s| !s.is_empty()) else {
        return kind_w;
    };
    if name == query {
        2.5
    } else if query.chars().any(|c| c.is_uppercase()) && name.eq_ignore_ascii_case(query) {
        kind_w * 0.55
    } else {
        kind_w
    }
}
fn symbol_span_rows_to_hits(
    store: &IndexStore,
    rows: Vec<SymbolSpanRow>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    kind: HitKind,
    score_for: impl Fn(&str) -> f64,
) -> Result<Vec<SearchHit>> {
    symbol_span_rows_to_hits_opts(store, rows, options, parsed, kind, score_for, true)
}
fn symbol_span_rows_to_hits_opts(
    store: &IndexStore,
    rows: Vec<SymbolSpanRow>,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    kind: HitKind,
    score_for: impl Fn(&str) -> f64,
    attach_excerpts: bool,
) -> Result<Vec<SearchHit>> {
    let spelling = parsed.identifier_spelling();
    // Ledger lever (conformance: file-affinity for def rows): distinctive
    // 3+/4+ char non-generic query terms matched against the file STEM.
    let stem_terms: Vec<&str> = if kind == HitKind::Def {
        def_stem_terms(parsed)
    } else {
        Vec::new()
    };
    let mut hits = Vec::with_capacity(rows.len());
    for (path, language, name, sym_kind, line_start, line_end) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        let stem_boost = if !stem_terms.is_empty() {
            let stem = file_stem_lower(&path);
            if stem_terms.iter().any(|term| stem.contains(term)) {
                FILE_STEM_DEF_AFFINITY
            } else {
                1.0
            }
        } else {
            1.0
        };
        hits.push(SearchHit::span(SpanHitInput {
            kind,
            file: path,
            line_start,
            line_end,
            score: score_for(&name) * symbol_match_weight(spelling, &name, &sym_kind) * stem_boost,
            excerpt: String::new(),
            symbol: Some(name),
            language,
            byte_span: None,
        }));
    }
    retain_scored_hits(&mut hits, options);
    if attach_excerpts {
        attach_indexed_excerpts(store, &mut hits)?;
    }
    if kind == HitKind::Def {
        attach_scip_def_resolutions(store, &mut hits)?;
    }
    Ok(hits)
}

fn attach_scip_def_resolutions(store: &IndexStore, hits: &mut [SearchHit]) -> Result<()> {
    let scip_defs = store.scip_fact_set(true)?;
    if scip_defs.is_empty() {
        return Ok(());
    }
    for hit in hits {
        let Some(name) = hit.symbol.as_deref() else {
            continue;
        };
        if scip_has(&scip_defs, &hit.file, hit.line_start, name) {
            let current = hit
                .resolution
                .clone()
                .unwrap_or(crate::resolution::Resolution::NameOnly);
            hit.resolution = Some(current.upgrade(crate::resolution::Resolution::ScipOccurrence));
        }
    }
    Ok(())
}

fn scip_has(facts: &[(String, u32, String)], file: &str, line: u32, name: &str) -> bool {
    facts
        .binary_search_by(|(path, line_no, ident)| {
            path.as_str()
                .cmp(file)
                .then(line_no.cmp(&line))
                .then(ident.as_str().cmp(name))
        })
        .is_ok()
}
pub fn symbol_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let mut hits = def_hits_for_terms(store, options, parsed, SYMBOL_SQL_LIMIT)?;
    hits.extend(caller_hits_for_terms(
        store,
        options,
        parsed,
        CALLER_SQL_LIMIT,
    )?);
    Ok(hits)
}
pub fn symbol_pass_for_files(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
    include_callers: bool,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || allowed_files.is_empty() {
        return Ok(Vec::new());
    }
    // File-restricted hybrid does not need the 500-row exhaustive window;
    // finish keeps `limit` hits. 32-64 rows is enough to score defs/callers
    // inside the 100-file cascade without a 1-5 ms SQLite LIKE walk.
    // Alias-expanded discovery widens the candidate mass enormously (every
    // `*match*` symbol in the tree): the 32-64-row path-sorted window fills
    // with alphabetically early noise and the alias target (`Types::matched`
    // in `crates/ignore/`) falls off the tail before scoring sees it. Alias
    // queries therefore floor the window at 256 rows — bounded, scored,
    // fusion decides, not the path alphabet.
    let sql_limit = options.retained_limit().clamp(32, SYMBOL_SQL_LIMIT);
    let norm_terms = normalize_query_terms(&parsed.terms);
    let bundle = store.read_bundle(None).ok().flatten();
    let idf_weights = {
        let _span = crate::perf_profile::Span::start(
            "symbol_idf_weights",
            "search",
            "name_table_idf_weights",
        );
        name_table_idf_weights(
            store,
            crate::names_sidecar::NamePopulation::Symbols,
            &norm_terms,
        )?
    };
    let aliases = if has_aliasable_verb(&norm_terms) {
        let cooccurrences = name_table_term_cooccurrences(store, &norm_terms)?;
        kept_verb_aliases(&norm_terms, &cooccurrences, |candidates| {
            name_table_idf_weights_with_dfs(
                store,
                crate::names_sidecar::NamePopulation::Symbols,
                candidates,
            )
        })?
    } else {
        AliasExpansion::default()
    };
    let sql_limit = if aliases.discovery.is_empty() {
        sql_limit
    } else {
        sql_limit.max(256).min(SYMBOL_SQL_LIMIT)
    };
    let mut def_discovery_terms = parsed.terms.clone();
    def_discovery_terms.extend(aliases.discovery.iter().cloned());
    let affine = stem_affine_files(parsed, allowed_files);
    let rows = {
        let _span = crate::perf_profile::Span::start(
            "symbol_quota_rows",
            "search",
            "query_quota_symbol_spans",
        );
        if let Some(bundle) = bundle
            .as_ref()
            .filter(|b| crate::store::bundle_graph::symbols_section_present(b))
        {
            packed_hybrid_def_rows(
                bundle,
                &def_discovery_terms,
                allowed_files,
                options.lang_filter.as_deref(),
                sql_limit,
                &affine,
            )
        } else {
            let (mut where_clause, mut bind) = like_terms_filter(
                "s.name",
                &def_discovery_terms,
                options.lang_filter.as_deref(),
            );
            restrict_to_files(&mut where_clause, &mut bind, Some(allowed_files));
            let mut rows =
                query_quota_symbol_spans(store, &def_discovery_terms, &where_clause, bind, sql_limit)?;
            rows.sort_by(cmp_symbol_rows);
            let mut rows = quota_partition(rows, |row| kind_rank(row.3.as_str()), sql_limit);
            if !affine.is_empty() {
                let (mut sup_where, mut sup_bind) = like_terms_filter(
                    "s.name",
                    &def_discovery_terms,
                    options.lang_filter.as_deref(),
                );
                restrict_to_files(&mut sup_where, &mut sup_bind, Some(&affine));
                if let Ok(supplement) =
                    query_symbol_spans(store, &sup_where, sup_bind, SYMBOL_KIND_ORDER, 32)
                {
                    if !supplement.is_empty() {
                        let mut seen: HashSet<(String, u32, String)> = rows
                            .iter()
                            .map(|row| (row.0.clone(), row.4, row.2.clone()))
                            .collect();
                        let before = rows.len();
                        for row in supplement {
                            if seen.insert((row.0.clone(), row.4, row.2.clone())) {
                                rows.push(row);
                            }
                        }
                        if rows.len() != before {
                            rows.sort_by(cmp_symbol_rows);
                        }
                    }
                }
            }
            rows
        }
    };
    let mut score_terms = norm_terms;
    let mut score_weights = idf_weights;
    score_terms.extend(aliases.score);
    score_weights.extend(aliases.weights);
    let mut hits = {
        let _span = crate::perf_profile::Span::start(
            "symbol_rows_to_hits",
            "search",
            "symbol_span_rows_to_hits_opts",
        );
        symbol_span_rows_to_hits_opts(
            store,
            rows,
            options,
            parsed,
            HitKind::Def,
            |name| score_def_weighted(&score_terms, &score_weights, name),
            false,
        )?
    };
    if include_callers {
        let _span =
            crate::perf_profile::Span::start("symbol_caller_pass", "search", "query_caller_rows");
        let caller_rows = if let Some(bundle) = bundle
            .as_ref()
            .filter(|b| crate::store::bundle_graph::callers_section_present(b))
        {
            crate::store::bundle_graph::callers_matches_for_files(
                bundle,
                &parsed.terms,
                allowed_files,
                options.lang_filter.as_deref(),
                sql_limit,
            )
        } else {
            query_caller_rows(
                store,
                caller_terms_filter,
                &parsed.terms,
                options.lang_filter.as_deref(),
                Some(allowed_files),
                sql_limit,
            )?
        };
        hits.extend(caller_rows_to_hits(
            store,
            caller_rows,
            options,
            parsed,
            CallerMatchMode::Hybrid,
            None,
            false,
        )?);
    }
    Ok(hits)
}

pub fn anchor_pass_for_files(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    anchor_pass_impl(store, options, parsed, Some(allowed_files), false)
}

pub fn anchor_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    anchor_pass_impl(store, options, parsed, None, true)
}

fn anchor_pass_impl(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: Option<&HashSet<String>>,
    attach_excerpts: bool,
) -> Result<Vec<SearchHit>> {
    if allowed_files.is_some_and(HashSet::is_empty) {
        return Ok(Vec::new());
    }
    let anchor_symbol = match parsed.primary_symbol() {
        Some(symbol) => symbol.to_string(),
        None => parsed
            .terms
            .iter()
            .find(|term| term.len() > 3)
            .cloned()
            .unwrap_or_default(),
    };
    if anchor_symbol.is_empty() {
        return Ok(Vec::new());
    }
    let terms = vec![anchor_symbol];
    let (mut where_clause, mut bind) =
        like_terms_filter("s.name", &terms, options.lang_filter.as_deref());
    restrict_to_files(&mut where_clause, &mut bind, allowed_files);
    let rows = query_symbol_spans(
        store,
        &where_clause,
        bind,
        SYMBOL_KIND_ORDER,
        SYMBOL_SQL_LIMIT,
    )?;
    let term_count = parsed.terms.len();
    symbol_span_rows_to_hits_opts(
        store,
        rows,
        options,
        parsed,
        HitKind::Anchor,
        |name| {
            let matched = parsed
                .terms
                .iter()
                .filter(|term| crate::rank::score_symbol(term, name) > 0.0)
                .count();
            if matched == 0 {
                0.0
            } else {
                SCORE_ANCHOR * (matched as f64 / term_count as f64).sqrt()
            }
        },
        attach_excerpts,
    )
}
/// NL verb → code-verb surface tokens. Static lookup is the substrate here
/// (no model): each row maps a natural-language verb to the identifier
/// vocabulary that implements it in real code. Applied ONLY to terms that
/// never co-occur with another query term inside any indexed symbol name
/// (`check`→`matched`, `register`→`add_url_rule`, `parse`→`loads`): a verb
/// whose literal surface is corroborated by the rest of the query (e.g.
/// query "check all the items" against a `check_all` symbol) must keep its
/// literal reading — mere population (df > 0) does NOT block aliasing,
/// because unrelated symbols routinely carry common verbs (measured:
/// `check` df=5, `register` df=20, `parse` df=1 in the lane corpora, all
/// with zero co-occurrence). Aliases score at [`ALIAS_DISCOUNT`] so a
/// literal match outranks an alias match, and an alias is dropped when its
/// own document frequency exceeds [`ALIAS_DF_CEILING`] (ultra-common tokens
/// such as `is`/`new` would flood the discovery window without adding
/// signal).
const ALIAS_DISCOUNT: f64 = 0.65;
const ALIAS_DF_CEILING: u64 = 2048;
static VERB_ALIASES: &[(&str, &[&str])] = &[
    ("check", &["match", "verify", "validate", "test"]),
    ("verify", &["check", "validate", "confirm"]),
    ("validate", &["check", "verify"]),
    ("parse", &["load", "loads", "read", "decode", "deserialize"]),
    ("read", &["load", "parse", "open"]),
    ("load", &["read", "parse", "open", "from"]),
    ("serialize", &["dump", "dumps", "encode", "write"]),
    ("register", &["add", "create", "define", "make"]),
    ("add", &["insert", "append", "push", "register"]),
    ("remove", &["delete", "drop"]),
    ("build", &["new", "create", "make", "from"]),
    ("create", &["new", "build", "make"]),
    ("make", &["new", "build", "create"]),
    ("map", &["convert", "transform", "from", "lookup"]),
    ("decide", &["choose", "select", "pick"]),
    ("detect", &["find", "has", "sense"]),
    ("serve", &["send", "handle", "respond"]),
    ("handle", &["process", "dispatch", "serve"]),
    ("escape", &["encode", "quote"]),
    ("print", &["display", "render", "format", "log"]),
    ("walk", &["visit", "traverse", "iter"]),
    ("apply", &["use", "exec", "run", "call"]),
    ("turn", &["make", "convert", "transform"]),
];

/// Does any query term name an aliasable verb? (Fast path: when false, the
/// co-occurrence scan is skipped entirely.)
fn has_aliasable_verb(norm_terms: &[String]) -> bool {
    norm_terms
        .iter()
        .any(|term| VERB_ALIASES.iter().any(|(verb, _)| *verb == term.as_str()))
}

/// Alias tokens worth probing for this query: only terms whose literal
/// surface is uncorroborated (zero co-occurrence with any other query term
/// across symbol names) expand. Deduplicated; never a token the query
/// already carries. `cooccurrences` may be empty when
/// [`has_aliasable_verb`] is false — the early return guarantees the
/// vectors are never read in that case.
fn verb_alias_candidates(norm_terms: &[String], cooccurrences: &[u64]) -> Vec<String> {
    if !has_aliasable_verb(norm_terms) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for (i, term) in norm_terms.iter().enumerate() {
        if cooccurrences.get(i).copied().unwrap_or(0) > 0 {
            continue;
        }
        for (verb, aliases) in VERB_ALIASES {
            if *verb != term.as_str() {
                continue;
            }
            for alias in aliases.iter() {
                let norm = normalize_query_terms(&[(*alias).to_string()])
                    .pop()
                    .unwrap_or_else(|| (*alias).to_lowercase());
                if !norm_terms.iter().any(|t| t == &norm) && !out.contains(&norm) {
                    out.push(norm);
                }
            }
        }
    }
    out
}

/// Alias tokens that survived the population probe: they name real symbols
/// (df > 0) without being so common they only add noise (df <= ceiling).
#[derive(Default)]
struct AliasExpansion {
    /// Lowercased alias tokens appended to the discovery (LIKE/contains) set.
    discovery: Vec<String>,
    /// Same tokens appended to the normalized scoring set.
    score: Vec<String>,
    /// Discounted IDF weights aligned with `score`.
    weights: Vec<f64>,
}

fn kept_verb_aliases(
    norm_terms: &[String],
    cooccurrences: &[u64],
    probe: impl FnOnce(&[String]) -> Result<(Vec<f64>, Vec<u64>)>,
) -> Result<AliasExpansion> {
    let candidates = verb_alias_candidates(norm_terms, cooccurrences);
    if candidates.is_empty() {
        return Ok(AliasExpansion::default());
    }
    let (alias_weights, alias_dfs) = probe(&candidates)?;
    let mut expansion = AliasExpansion::default();
    for (cand, (weight, df)) in candidates
        .iter()
        .zip(alias_weights.iter().zip(alias_dfs.iter()))
    {
        if *df == 0 || *df > ALIAS_DF_CEILING {
            continue;
        }
        expansion.discovery.push(cand.clone());
        expansion.score.push(cand.clone());
        expansion.weights.push(weight * ALIAS_DISCOUNT);
    }
    Ok(expansion)
}

/// Per-term co-occurrence counts over symbol names: how many names contain
/// this term AND at least one other query term. One streamed pass; the
/// per-name cost is one substring check per term — the same class as the
/// df sweep in [`name_table_idf_weights_with_dfs`].
fn cooccurrences_over_names(
    norm_terms: &[String],
    names: impl Iterator<Item = impl AsRef<str>>,
) -> Vec<u64> {
    let mut co = vec![0u64; norm_terms.len()];
    let mut contains: Vec<bool> = Vec::with_capacity(norm_terms.len());
    for name in names {
        let lower = name.as_ref().to_lowercase();
        contains.clear();
        let mut count = 0usize;
        for (i, term) in norm_terms.iter().enumerate() {
            let hit = lower.contains(term.as_str());
            contains.push(hit);
            count += hit as usize;
        }
        if count >= 2 {
            for (i, hit) in contains.iter().enumerate() {
                if *hit {
                    co[i] += 1;
                }
            }
        }
    }
    co
}

/// Cold lane: co-occurrence counts streamed from the symbols table.
fn name_table_term_cooccurrences(store: &IndexStore, norm_terms: &[String]) -> Result<Vec<u64>> {
    if norm_terms.is_empty() {
        return Ok(Vec::new());
    }
    let (source, column) = crate::names_sidecar::NamePopulation::Symbols.sql_source_and_column();
    let sql = format!("SELECT {column} FROM {source}");
    let conn = store.connection();
    let mut stmt = conn.prepare_cached(&sql)?;
    let names = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(cooccurrences_over_names(
        norm_terms,
        names.into_iter().map(|r| r.unwrap_or_default()),
    ))
}

/// Query-level verb-alias enrichment for lexical FILE discovery: the same
/// static table and co-occurrence gate as the def lane, exposed for the
/// hybrid cascade entry. The gold file for a verb query (`check`→
/// `types.rs::matched`) often has NO literal foothold on the query's own
/// terms, so unless the alias tokens join the prefilter's discovery set the
/// file never enters the cascade and the def lane's alias expansion can
/// never see it.
pub(crate) fn user_verb_aliases(store: &IndexStore, terms: &[String]) -> Result<Vec<String>> {
    let norm_terms = normalize_query_terms(terms);
    if !has_aliasable_verb(&norm_terms) {
        return Ok(Vec::new());
    }
    let cooccurrences = name_table_term_cooccurrences(store, &norm_terms)?;
    let out = kept_verb_aliases(&norm_terms, &cooccurrences, |candidates| {
        name_table_idf_weights_with_dfs(
            store,
            crate::names_sidecar::NamePopulation::Symbols,
            candidates,
        )
    })?;
    Ok(out.discovery)
}

/// Symbol-nominated alias files: files whose SYMBOL names carry an alias
/// token. This is the file-discovery channel a verb query actually needs —
/// content-level alias scans are hopeless on code corpora (`match` names
/// 4.5k lines in the rg corpus and the rarity cap drops it), while the
/// symbol surface is small and precise (`Types::matched` → `types.rs`).
/// Bounded by `cap`; empty when no aliases.
pub(crate) fn alias_nominated_files(
    store: &IndexStore,
    aliases: &[String],
    cap: usize,
) -> Result<HashSet<String>> {
    if aliases.is_empty() {
        return Ok(HashSet::new());
    }
    let (where_clause, bind) = like_terms_filter("s.name", aliases, None);
    let sql = format!(
        "SELECT DISTINCT f.path FROM symbols s JOIN files f ON f.id = s.file_id\
         {where_clause} ORDER BY f.path LIMIT ?{}",
        bind.len() + 1
    );
    let conn = store.connection();
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(
        bind.iter().chain([cap.to_string()].iter()),
    ))?;
    let mut files = HashSet::new();
    while let Some(row) = rows.next()? {
        files.insert(row.get::<_, String>(0)?);
    }
    Ok(files)
}

pub(crate) fn def_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(vec![]);
    }
    let norm_terms = normalize_query_terms(&parsed.terms);
    let idf_weights = name_table_idf_weights(
        store,
        crate::names_sidecar::NamePopulation::Symbols,
        &norm_terms,
    )?;
    let aliases = if has_aliasable_verb(&norm_terms) {
        let cooccurrences = name_table_term_cooccurrences(store, &norm_terms)?;
        kept_verb_aliases(&norm_terms, &cooccurrences, |candidates| {
            name_table_idf_weights_with_dfs(
                store,
                crate::names_sidecar::NamePopulation::Symbols,
                candidates,
            )
        })?
    } else {
        AliasExpansion::default()
    };
    let mut discovery_terms = parsed.terms.clone();
    discovery_terms.extend(aliases.discovery.iter().cloned());
    let (where_clause, bind) =
        like_terms_filter("s.name", &discovery_terms, options.lang_filter.as_deref());
    let rows = query_symbol_spans(store, &where_clause, bind, SYMBOL_KIND_ORDER, limit)?;
    let mut score_terms = norm_terms;
    let mut score_weights = idf_weights;
    score_terms.extend(aliases.score);
    score_weights.extend(aliases.weights);
    symbol_span_rows_to_hits(store, rows, options, parsed, HitKind::Def, |name| {
        score_def_weighted(&score_terms, &score_weights, name)
    })
}
fn exact_eq_filter(column: &str, value: &str, lang: Option<&str>) -> (String, Vec<String>) {
    use crate::store::sql::{append_lang_filter, where_clause};
    let mut bind = vec![value.to_string()];
    let mut parts = vec![format!("lower({column}) = lower(?)")];
    append_lang_filter(&mut parts, &mut bind, lang);
    (where_clause(&parts), bind)
}
pub(crate) fn caller_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(vec![]);
    }
    caller_rows_to_hits(
        store,
        query_caller_rows(
            store,
            caller_terms_filter,
            &parsed.terms,
            options.lang_filter.as_deref(),
            None,
            limit,
        )?,
        options,
        parsed,
        CallerMatchMode::Hybrid,
        None,
        true,
    )
}
fn prefixed_mode_query(parsed: &ParsedQuery) -> Option<ParsedQuery> {
    let symbol = parsed.lookup_symbol();
    (!symbol.is_empty()).then(|| ParsedQuery {
        terms: vec![symbol],
        ..parsed.clone()
    })
}
pub fn search_callers(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let Some(q) = prefixed_mode_query(parsed) else {
        return Ok(vec![]);
    };
    let name = q.terms.first().map(String::as_str).unwrap_or("");
    if name.is_empty() {
        return Ok(vec![]);
    }
    if let Ok(Some(bundle)) = store.read_bundle(None) {
        let rows = crate::store::bundle_graph::callers_exact(
            &bundle,
            name,
            options.lang_filter.as_deref(),
            mode_sql_limit(options),
        );
        return caller_rows_to_hits(
            store,
            rows,
            options,
            &q,
            CallerMatchMode::CalleeOnly,
            Some(store),
            true,
        );
    }
    let (where_clause, bind) = exact_eq_filter("c.callee", name, options.lang_filter.as_deref());
    let sql = format!("{CALLER_SELECT}{where_clause} LIMIT ?{}", bind.len() + 1);
    let rows = query_limit_map(
        store.connection(),
        &sql,
        bind,
        mode_sql_limit(options),
        map_caller_row,
    )?;
    caller_rows_to_hits(
        store,
        rows,
        options,
        &q,
        CallerMatchMode::CalleeOnly,
        Some(store),
        true,
    )
}
pub fn search_defs(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let Some(q) = prefixed_mode_query(parsed) else {
        return Ok(vec![]);
    };
    let name = q.terms.first().map(String::as_str).unwrap_or("");
    if name.is_empty() {
        return Ok(vec![]);
    }
    if let Ok(Some(bundle)) = store.read_bundle(None) {
        let mut rows = crate::store::bundle_graph::defs_exact(
            &bundle,
            name,
            options.lang_filter.as_deref(),
            mode_sql_limit(options),
        );
        rows.sort_by(cmp_symbol_rows);
        return symbol_span_rows_to_hits(store, rows, options, &q, HitKind::Def, |n| {
            score_def(&q.terms, n)
        });
    }
    let (where_clause, bind) = exact_eq_filter("s.name", name, options.lang_filter.as_deref());
    let rows = query_symbol_spans(
        store,
        &where_clause,
        bind,
        SYMBOL_KIND_ORDER,
        mode_sql_limit(options),
    )?;
    symbol_span_rows_to_hits(store, rows, options, &q, HitKind::Def, |n| {
        score_def(&q.terms, n)
    })
}
pub fn search_imports(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let module = parsed.lookup_symbol();
    let module = (!module.is_empty()).then_some(module.as_str());
    Ok(store
        .query_imports(
            module,
            options.lang_filter.as_deref(),
            mode_sql_limit(options),
        )?
        .into_iter()
        .map(|(path, language, module_path, line_no)| {
            SearchHit::import(path, language, module_path, line_no)
        })
        .collect())
}
