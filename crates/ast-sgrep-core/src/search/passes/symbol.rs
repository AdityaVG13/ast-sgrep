use crate::query::ParsedQuery;
use crate::rank::{
    best_symbol_score_normalized, max_normalize_idf, normalize_query_terms, score_caller_weighted,
    score_def, score_def_weighted, term_idf, SCORE_ANCHOR, SCORE_GRAPH,
};
use crate::search::types::matches_lang;
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::sql::{
    caller_terms_filter, escape_like_term, like_terms_filter, query_limit_map,
};
use crate::store::IndexStore;
use crate::Result;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
const SYMBOL_SQL_LIMIT: usize = 500;
const CALLER_SQL_LIMIT: usize = 500;
const MODE_SQL_LIMIT: usize = 200;

fn mode_sql_limit(options: &SearchOptions) -> usize {
    MODE_SQL_LIMIT.max(options.retained_limit())
}
const TYPE_SYMBOL_WEIGHT: f64 = 0.65;
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
pub(crate) struct WarmedSymbolTable {
    rows: Arc<Vec<SymbolSpanRow>>,
    names_lower: Arc<Vec<String>>,
    by_file: Arc<HashMap<String, Vec<usize>>>,
}

impl WarmedSymbolTable {
    pub(crate) fn load(store: &IndexStore) -> Result<Self> {
        const WARMED_SYMBOL_CAP: usize = 262_144;
        let rows = query_symbol_spans(store, "", Vec::new(), SYMBOL_KIND_ORDER, WARMED_SYMBOL_CAP)?;
        let mut names_lower = Vec::with_capacity(rows.len());
        let mut by_file: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, row) in rows.iter().enumerate() {
            names_lower.push(row.2.to_lowercase());
            by_file.entry(row.0.clone()).or_default().push(i);
        }
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
            let Some(idxs) = self.by_file.get(path) else {
                continue;
            };
            for &i in idxs {
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

    /// Per-term IDF weights over the warmed table (the whole symbol table):
    /// one pass counting, per normalized term, how many names contain it —
    /// the same predicate as matching. Mirrors [`name_table_idf_weights`]
    /// so warm and cold lanes score defs identically.
    fn idf_weights(&self, normalized_terms: &[String]) -> Vec<f64> {
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
        weights
    }
}

pub fn kind_rank(kind: &str) -> u8 {
    match kind {
        "function" | "method" => 0,
        "type" | "enum" | "class" | "interface" | "struct" => 1,
        _ => 2,
    }
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
/// normalized term, how many names contain it under the same LIKE
/// predicate as matching. The population is the name vocabulary — one
/// entry per distinct name — so a callee called from 500 sites does not
/// make its tokens 500x more common as name evidence (and the distinct
/// scan stays ~2 ms where the raw callers scan is ~11 ms on this repo).
/// `source`/`column` are internal constants, never user input. Mirrors
/// `WarmedSymbolTable::idf_weights` so warm and cold lanes score
/// identically.
fn name_table_idf_weights(
    store: &IndexStore,
    source: &str,
    column: &str,
    normalized_terms: &[String],
) -> Result<Vec<f64>> {
    if normalized_terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = String::from("SELECT count(*)");
    for i in 0..normalized_terms.len() {
        sql.push_str(&format!(
            ", COALESCE(SUM(CASE WHEN {column} LIKE '%' || ?{} || '%' ESCAPE '\\' THEN 1 ELSE 0 END), 0)",
            i + 1
        ));
    }
    sql.push_str(&format!(" FROM {source}"));
    let bound: Vec<String> = normalized_terms
        .iter()
        .map(|term| escape_like_term(term))
        .collect();
    let params: Vec<&dyn rusqlite::ToSql> =
        bound.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let conn = store.connection();
    let mut stmt = conn.prepare_cached(&sql)?;
    let n_terms = normalized_terms.len();
    let (total, dfs) = stmt.query_row(params.as_slice(), |row| {
        let total: i64 = row.get(0)?;
        let mut dfs = Vec::with_capacity(n_terms);
        for i in 0..n_terms {
            dfs.push(row.get::<_, i64>(i + 1)?);
        }
        Ok((total, dfs))
    })?;
    let mut weights: Vec<f64> = dfs
        .iter()
        .map(|&df| term_idf(df.max(0) as u64, total.max(0) as u64))
        .collect();
    max_normalize_idf(&mut weights);
    Ok(weights)
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
    } else {
        name_table_idf_weights(
            excerpt_store,
            "(SELECT DISTINCT callee FROM callers)",
            "callee",
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
        let mut candidate_counts = HashMap::new();
        let scip_refs = store.scip_fact_set(false)?;
        attach_caller_resolutions(store, &mut candidate_counts, &scip_refs, &mut caller_hits)?;
        attach_caller_resolutions(store, &mut candidate_counts, &scip_refs, &mut graph_hits)?;
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
    candidate_counts: &mut HashMap<String, (HashMap<String, usize>, usize)>,
    scip_refs: &HashSet<(String, u32, String)>,
    hits: &mut [SearchHit],
) -> Result<()> {
    for hit in hits {
        let Some(callee) = hit.callee.as_deref() else {
            continue;
        };
        if !candidate_counts.contains_key(callee) {
            candidate_counts.insert(
                callee.to_owned(),
                store.symbol_name_candidate_counts(callee)?,
            );
        }
        let (by_file, repo) = &candidate_counts[callee];
        let same_file = by_file.get(&hit.file).copied().unwrap_or(0);
        let counted =
            crate::resolution::Resolution::from_candidates(same_file, *repo, std::iter::empty());
        hit.resolution = Some(
            if scip_refs.contains(&(hit.file.clone(), hit.line_start, callee.to_string())) {
                counted.upgrade(crate::resolution::Resolution::ScipOccurrence)
            } else {
                counted
            },
        );
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
    let mut hits = Vec::with_capacity(rows.len());
    for (path, language, name, sym_kind, line_start, line_end) in rows {
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        hits.push(SearchHit::span(SpanHitInput {
            kind,
            file: path,
            line_start,
            line_end,
            score: score_for(&name) * symbol_match_weight(spelling, &name, &sym_kind),
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
        if scip_defs.contains(&(hit.file.clone(), hit.line_start, name.to_string())) {
            let current = hit
                .resolution
                .clone()
                .unwrap_or(crate::resolution::Resolution::NameOnly);
            hit.resolution = Some(current.upgrade(crate::resolution::Resolution::ScipOccurrence));
        }
    }
    Ok(())
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
    symbol_pass_for_files_warmed(store, options, parsed, allowed_files, include_callers, None)
}

pub(crate) fn symbol_pass_for_files_warmed(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
    include_callers: bool,
    warmed: Option<&WarmedSymbolTable>,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || allowed_files.is_empty() {
        return Ok(Vec::new());
    }
    // File-restricted hybrid does not need the 500-row exhaustive window;
    // finish keeps `limit` hits. 32-64 rows is enough to score defs/callers
    // inside the 100-file cascade without a 1-5 ms SQLite LIKE walk.
    let sql_limit = options.retained_limit().clamp(32, SYMBOL_SQL_LIMIT);
    let rows = if let Some(table) = warmed {
        table.matches_for_files(options, &parsed.terms, allowed_files, sql_limit)
    } else {
        let (mut where_clause, mut bind) =
            like_terms_filter("s.name", &parsed.terms, options.lang_filter.as_deref());
        restrict_to_files(&mut where_clause, &mut bind, Some(allowed_files));
        // Quotas must precede admission: even a 500-row function-first
        // window can hide every minority kind. Fetch a bounded quota page in
        // one statement, then preserve the shared canonical selection policy.
        let mut rows =
            query_quota_symbol_spans(store, &parsed.terms, &where_clause, bind, sql_limit)?;
        rows.sort_by(cmp_symbol_rows);
        quota_partition(rows, |row| kind_rank(row.3.as_str()), sql_limit)
    };
    let norm_terms = normalize_query_terms(&parsed.terms);
    let idf_weights = match warmed {
        Some(table) => table.idf_weights(&norm_terms),
        None => name_table_idf_weights(store, "symbols", "name", &norm_terms)?,
    };
    let mut hits = symbol_span_rows_to_hits_opts(
        store,
        rows,
        options,
        parsed,
        HitKind::Def,
        |name| score_def_weighted(&norm_terms, &idf_weights, name),
        false,
    )?;
    if include_callers {
        hits.extend(caller_rows_to_hits(
            store,
            query_caller_rows(
                store,
                caller_terms_filter,
                &parsed.terms,
                options.lang_filter.as_deref(),
                Some(allowed_files),
                sql_limit,
            )?,
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
pub(crate) fn def_hits_for_terms(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(vec![]);
    }
    let (where_clause, bind) =
        like_terms_filter("s.name", &parsed.terms, options.lang_filter.as_deref());
    let rows = query_symbol_spans(store, &where_clause, bind, SYMBOL_KIND_ORDER, limit)?;
    let norm_terms = normalize_query_terms(&parsed.terms);
    let idf_weights = name_table_idf_weights(store, "symbols", "name", &norm_terms)?;
    symbol_span_rows_to_hits(store, rows, options, parsed, HitKind::Def, |name| {
        score_def_weighted(&norm_terms, &idf_weights, name)
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
    // callers:Name uses equality on indexed callee column.
    let name = q.terms.first().map(String::as_str).unwrap_or("");
    if name.is_empty() {
        return Ok(vec![]);
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
    // defs:Name is an exact symbol lookup — equality uses the name index.
    let name = q.terms.first().map(String::as_str).unwrap_or("");
    if name.is_empty() {
        return Ok(vec![]);
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
