use super::symbol::name_table_idf_weights_with_dfs;
use crate::rank::normalize_query_terms;
use crate::store::sql::like_terms_filter;
use crate::store::IndexStore;
use crate::Result;
use std::collections::HashSet;

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
pub(crate) fn has_aliasable_verb(norm_terms: &[String]) -> bool {
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
pub(crate) struct AliasExpansion {
    /// Lowercased alias tokens appended to the discovery (LIKE/contains) set.
    pub(crate) discovery: Vec<String>,
    /// Same tokens appended to the normalized scoring set.
    pub(crate) score: Vec<String>,
    /// Discounted IDF weights aligned with `score`.
    pub(crate) weights: Vec<f64>,
}

pub(crate) fn kept_verb_aliases(
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
pub(crate) fn name_table_term_cooccurrences(store: &IndexStore, norm_terms: &[String]) -> Result<Vec<u64>> {
    if norm_terms.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(co) = crate::names_sidecar::sidecar_cooccurrences_for_store(
        store,
        crate::names_sidecar::NamePopulation::Symbols,
        norm_terms,
    ) {
        return Ok(co);
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
