#![forbid(unsafe_code)]

pub mod budget;
pub use budget::{DetailLevel, OutputBudget, RenderedHit};

use ast_sgrep_core::{follow_ups_for_hit, hit_why, plan_suggested_next, HitKind, SearchResponse};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Native,
    GitHub,
    GitLab,
    Agent,
    AgentCapsule,
    Compact,
}
impl OutputFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "native" | "asgrep" => Some(Self::Native),
            "github" | "gh" => Some(Self::GitHub),
            "gitlab" | "gl" => Some(Self::GitLab),
            "agent" | "llm" | "ai" => Some(Self::Agent),
            "agent-capsule" | "capsule" => Some(Self::AgentCapsule),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }
}

/// Snippet limits for compact agent output.
///
/// A token unit is one UTF-8 byte. This is a deterministic, model-independent
/// upper bound for byte-fallback tokenizers rather than an estimate tied to one
/// vendor vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactBudget {
    pub per_result_tokens: usize,
    pub response_tokens: usize,
}

impl Default for CompactBudget {
    fn default() -> Self {
        Self {
            per_result_tokens: 96,
            response_tokens: 768,
        }
    }
}

pub fn format_response(response: &SearchResponse, format: OutputFormat) -> serde_json::Value {
    format_response_with(response, format, 0)
}
pub fn format_response_with(
    response: &SearchResponse,
    format: OutputFormat,
    excerpt_lines: usize,
) -> serde_json::Value {
    format_response_with_budget(response, format, excerpt_lines, CompactBudget::default())
}

pub fn format_response_with_budget(
    response: &SearchResponse,
    format: OutputFormat,
    excerpt_lines: usize,
    compact_budget: CompactBudget,
) -> serde_json::Value {
    match format {
        OutputFormat::Native => serde_json::to_value(response)
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()})),
        OutputFormat::GitHub => to_github_json(response),
        OutputFormat::GitLab => to_gitlab_json(response),
        OutputFormat::Agent => to_agent_json(response),
        OutputFormat::AgentCapsule => to_agent_capsule_json(response, excerpt_lines),
        OutputFormat::Compact => to_compact_json(response, compact_budget),
    }
}
/// Project a response into a GitHub-like page. total_count is the returned page size (no corpus-wide match count).
pub fn to_github_json(response: &SearchResponse) -> serde_json::Value {
    let items: Vec<_> = response.hits.iter().map(|hit| serde_json::json!({
        "name": hit.file.rsplit('/').next().unwrap_or(&hit.file), "path": hit.file, "score": hit.score, "language": hit.language,
        "text_matches": [{"fragment": hit.excerpt, "matches": [ { "text": hit.symbol.as_deref().or(hit.callee.as_deref()).unwrap_or(""), "indices": [0] }]}],
        "metadata": {
            "kind": hit.kind.as_str(), "signal": hit.signal, "contributors": hit.contributors, "score": hit.score, "margin": hit.margin,
            "line_start": hit.line_start, "line_end": hit.line_end,
            "symbol": hit.symbol, "caller": hit.caller, "callee": hit.callee, }
    })).collect();
    let incomplete = response.limit > 0 && items.len() >= response.limit;
    serde_json::json!({
        "total_count": items.len(), "incomplete_results": incomplete, "items": items, "query": response.query, "provider": "ast-sgrep"
    })
}
/// Project local results into a GitLab-like shape. This adapter has no repository context,
/// so ref is always HEAD and project_id is always null; consumers must supply that context.
pub fn to_gitlab_json(response: &SearchResponse) -> serde_json::Value {
    let data: Vec<_> = response.hits.iter().map(|hit| serde_json::json!({
        "basename": hit.file.rsplit('/').next().unwrap_or(&hit.file), "data": hit.excerpt,
        "path": hit.file, "filename": hit.file, "ref": "HEAD", "startline": hit.line_start, "project_id": null,
        "meta": {
            "kind": hit.kind.as_str(), "signal": hit.signal, "contributors": hit.contributors, "score": hit.score, "margin": hit.margin,
            "language": hit.language,
            "line_end": hit.line_end, "symbol": hit.symbol, "caller": hit.caller, "callee": hit.callee, }
    })).collect();
    serde_json::json!({"data": data, "query": response.query, "provider": "ast-sgrep"})
}
fn hit_symbol(hit: &ast_sgrep_core::SearchHit) -> Option<&str> {
    hit.symbol
        .as_deref()
        .or(hit.callee.as_deref())
        .or(hit.caller.as_deref())
}
pub fn to_agent_json(response: &SearchResponse) -> serde_json::Value {
    let hits: Vec<_> = response.hits.iter().map(|hit| {
        // Causal planner: follow-ups reflect this hit's actual evidence gap
        // (missing def/usage channels, critic notes, margin), not a template.
        let follow_ups = follow_ups_for_hit(&response.query, hit);
        serde_json::json!({
            "kind": hit.kind.as_str(), "signal": hit.signal, "contributors": hit.contributors, "semantic": hit.contributors.contains(&HitKind::Embed),
            "score": hit.score, "margin": hit.margin,
            "file": hit.file, "lines": {"start": hit.line_start, "end": hit.line_end},
            "symbol": hit.symbol, "caller": hit.caller, "callee": hit.callee, "language": hit.language,
            "why": hit_why(hit),
            "excerpt": hit.excerpt, "follow_up_queries": follow_ups, })
    }).collect();
    let has_semantic = hits.iter().any(|h| h["semantic"] == true);
    // Causal planner: suggested_next is derived from the actual top hit.
    let suggested = plan_suggested_next(response);
    serde_json::json!({
        "provider": "ast-sgrep", "version": env!("CARGO_PKG_VERSION"), "query": response.query, "limit": response.limit, "hit_count": hits.len(),
        "read_bytes_estimate": response.read_bytes_estimate, "returned_excerpt_bytes": response.returned_excerpt_bytes,
        "prevented_read_bytes": response.prevented_read_bytes, "has_semantic_hits": has_semantic,
        "stack_hint": "Use asgrep for hybrid search; defs:/callers:/literal: prefixes for graph and exact text; asgrep semantic for embedding-only.", "suggested_next": suggested, "hits": hits,
    })
}
const PREVIEW_MAX_CHARS: usize = 120;
pub fn to_agent_capsule_json(response: &SearchResponse, excerpt_lines: usize) -> serde_json::Value {
    let hits: Vec<_> = response.hits.iter().map(|hit| {
        let mut capsule = serde_json::json!({
            "file": hit.file, "lines": {"start": hit.line_start, "end": hit.line_end}, "symbol": hit.symbol, "caller": hit.caller, "callee": hit.callee,
            "kind": hit.kind.as_str(), "signal": hit.signal, "contributors": hit.contributors, "score": hit.score, "margin": hit.margin,
            "confidence": hit.confidence,
            "why": hit_why(hit),
            "preview": preview_line(&hit.excerpt),
            "ref": format!("{}#L{}-L{}", hit.file, hit.line_start, hit.line_end), });
        if excerpt_lines > 0 {
            let body: Vec<_> = hit.excerpt.lines().take(excerpt_lines).collect();
            capsule["excerpt"] = serde_json::Value::String(body.join("\n"));
        }
        capsule
    }).collect();
    let returned: u64 = hits
        .iter()
        .filter_map(|h| {
            h.get("excerpt")
                .or_else(|| h.get("preview"))
                .and_then(serde_json::Value::as_str)
        })
        .map(|e| e.len() as u64)
        .sum();
    serde_json::json!({
        "provider": "ast-sgrep", "mode": "capsule", "query": response.query, "limit": response.limit,
        "hit_count": hits.len(), "read_bytes_estimate": response.read_bytes_estimate,
        "returned_excerpt_bytes": returned, "prevented_read_bytes": response.prevented_read_bytes,
        "expand_hint": "re-run with --excerpt-lines N for bodies, or read each ref span with your file reader (path + line window)", "hits": hits,
    })
}
/// Compact envelope whose per-result detail is chosen under a token budget
/// (m38g), instead of truncating every excerpt to the same ceiling.
pub fn to_budgeted_compact_json(
    response: &SearchResponse,
    budget: budget::OutputBudget,
) -> serde_json::Value {
    let rendered = budget::select(&response.hits, budget);
    let mut envelope = to_compact_json(response, CompactBudget::default());
    if let Some(hits) = envelope
        .get_mut("h")
        .and_then(serde_json::Value::as_array_mut)
    {
        for (row, plan) in hits.iter_mut().zip(rendered.iter()) {
            let Some(row) = row.as_array_mut() else {
                continue;
            };
            if row.len() >= 5 {
                row[4] = serde_json::Value::String(plan.body.clone());
            }
            // Detail level is per-result, so it has to travel with the result.
            row.push(serde_json::Value::String(plan.detail.as_str().to_owned()));
        }
    }
    envelope["zd"] = serde_json::json!([budget.max_tokens, budget::plan_cost(&rendered)]);
    envelope
}

pub fn to_compact_json(response: &SearchResponse, budget: CompactBudget) -> serde_json::Value {
    let mut paths = std::collections::BTreeMap::<String, String>::new();
    let mut path_ids = std::collections::BTreeMap::<String, String>::new();
    for path in response
        .hits
        .iter()
        .map(|hit| hit.file.as_str())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let base = base36(fnv1a64(path.as_bytes()));
        let mut id = base.clone();
        let mut suffix = 1_u32;
        while paths.get(&id).is_some_and(|existing| existing != path) {
            id = format!("{base}~{suffix}");
            suffix += 1;
        }
        paths.insert(id.clone(), path.to_owned());
        path_ids.insert(path.to_owned(), id);
    }

    let mut remaining = budget.response_tokens;
    let mut used = 0_usize;
    let mut truncated = 0_usize;
    let hits: Vec<_> = response
        .hits
        .iter()
        .map(|hit| {
            let path_id = &path_ids[&hit.file];
            let id = format!("{path_id}:{}-{}", hit.line_start, hit.line_end);
            let allowance = budget.per_result_tokens.min(remaining);
            let (snippet, was_truncated) = utf8_prefix(hit.excerpt.trim(), allowance);
            let snippet_bytes = snippet.len();
            remaining -= snippet_bytes;
            used += snippet_bytes;
            truncated += usize::from(was_truncated);
            serde_json::json!([
                id,
                compact_kind(hit.kind),
                compact_signal(hit.signal),
                hit_symbol(hit),
                snippet
            ])
        })
        .collect();

    // am4a: fold shared directory prefixes out of the path table when doing so
    // is actually smaller on the wire.
    let (roots, paths) = fold_path_roots(&paths);

    // 9q0l: serde_json orders object keys alphabetically (its Map is a
    // BTreeMap unless `preserve_order` is enabled), so key NAMES decide wire
    // order. Per-call accounting is therefore named `z*` to sort after the
    // content keys, giving every envelope a stable head and a volatile tail:
    //
    //   h (hits) . p (paths) . q (query) . r (roots) . v (schema) . zb . zn . zt
    //
    // A consumer can drop the `z*` tail without touching the content, and
    // repeated identical calls differ only in that tail.
    let mut envelope = serde_json::json!({
        "v": 1,
        "q": response.query,
        "p": paths,
        "h": hits,
        "zn": hits.len(),
        "zb": [budget.per_result_tokens, budget.response_tokens, used],
        "zt": truncated,
    });
    if !roots.is_empty() {
        envelope["r"] = serde_json::Value::from(roots);
    }
    envelope
}

/// Fold shared directory prefixes out of a path table (am4a).
///
/// Returns the root list and the rewritten table. A folded entry is
/// `[root_index, suffix]`; an unfolded entry stays a plain string, so mixed
/// results never pay for a root they cannot use.
///
/// The encoding is only applied when it is strictly smaller than the verbatim
/// table, measured on the serialized bytes rather than estimated. A result set
/// with no shared structure is returned untouched.
fn fold_path_roots(
    paths: &std::collections::BTreeMap<String, String>,
) -> (Vec<String>, serde_json::Value) {
    let verbatim = serde_json::to_value(paths).unwrap_or(serde_json::Value::Null);
    if paths.len() < 2 {
        return (Vec::new(), verbatim);
    }

    // Candidate roots are the directory prefixes shared by at least two paths.
    // BTreeMap iteration keeps this deterministic for a given hit set.
    let mut usage = std::collections::BTreeMap::<&str, usize>::new();
    for path in paths.values() {
        for (index, _) in path.match_indices('/') {
            *usage.entry(&path[..=index]).or_default() += 1;
        }
    }
    let candidates: Vec<&str> = usage
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(root, _)| *root)
        .collect();
    if candidates.is_empty() {
        return (Vec::new(), verbatim);
    }

    // Choosing the longest shared prefix per path is NOT optimal: a deeper root
    // re-emits its own parent, which can cost more than it saves. Add roots
    // greedily, and only while the measured payload actually shrinks.
    let verbatim_bytes = serde_json::to_string(&verbatim)
        .map(|text| text.len())
        .unwrap_or(0);
    let mut chosen: Vec<&str> = Vec::new();
    let mut best_bytes = verbatim_bytes;
    loop {
        let mut improvement: Option<(&str, usize)> = None;
        for candidate in &candidates {
            if chosen.contains(candidate) {
                continue;
            }
            let mut trial = chosen.clone();
            trial.push(candidate);
            let bytes = encoded_len(paths, &trial);
            let better = improvement.is_none_or(|(_, best)| bytes < best);
            if bytes < best_bytes && better {
                improvement = Some((candidate, bytes));
            }
        }
        match improvement {
            Some((candidate, bytes)) => {
                chosen.push(candidate);
                best_bytes = bytes;
            }
            None => break,
        }
    }

    if chosen.is_empty() {
        return (Vec::new(), verbatim);
    }
    let (roots, folded) = encode_with_roots(paths, &chosen);
    (roots, folded)
}

/// Serialized size of the path table plus root table under a root set (am4a).
/// `+ 4` covers the `"r":` key and its separator in the envelope.
fn encoded_len(paths: &std::collections::BTreeMap<String, String>, roots: &[&str]) -> usize {
    let (roots, folded) = encode_with_roots(paths, roots);
    let folded = serde_json::to_string(&folded)
        .map(|text| text.len())
        .unwrap_or(usize::MAX);
    let roots = serde_json::to_string(&roots)
        .map(|text| text.len())
        .unwrap_or(usize::MAX);
    folded.saturating_add(roots).saturating_add(4)
}

/// Encode a path table against a root set, longest matching root per path.
/// Roots that end up unused are dropped so the table never carries dead weight.
fn encode_with_roots(
    paths: &std::collections::BTreeMap<String, String>,
    roots: &[&str],
) -> (Vec<String>, serde_json::Value) {
    let mut ordered: Vec<&str> = roots.to_vec();
    ordered.sort_by(|left, right| right.len().cmp(&left.len()).then(left.cmp(right)));

    let mut used: Vec<String> = Vec::new();
    let mut folded = serde_json::Map::new();
    for (id, path) in paths {
        match ordered.iter().find(|root| path.starts_with(**root)) {
            Some(root) => {
                let index = match used.iter().position(|existing| existing == *root) {
                    Some(index) => index,
                    None => {
                        used.push((*root).to_owned());
                        used.len() - 1
                    }
                };
                folded.insert(id.clone(), serde_json::json!([index, &path[root.len()..]]));
            }
            None => {
                folded.insert(id.clone(), serde_json::Value::String(path.clone()));
            }
        }
    }
    (used, serde_json::Value::Object(folded))
}

fn compact_kind(kind: HitKind) -> &'static str {
    match kind {
        HitKind::Asgrep => "x",
        HitKind::Def => "d",
        HitKind::Caller => "c",
        HitKind::Graph => "g",
        HitKind::Anchor => "a",
        HitKind::Import => "i",
        HitKind::Pattern => "p",
        HitKind::Embed => "e",
    }
}

fn compact_signal(signal: ast_sgrep_core::search::HitSignal) -> &'static str {
    match signal {
        ast_sgrep_core::search::HitSignal::Exact => "x",
        ast_sgrep_core::search::HitSignal::Structural => "t",
        ast_sgrep_core::search::HitSignal::Semantic => "m",
    }
}

fn utf8_prefix(input: &str, max_bytes: usize) -> (&str, bool) {
    if input.len() <= max_bytes {
        return (input, false);
    }
    let mut end = max_bytes.min(input.len());
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    (&input[..end], true)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut reversed = [0_u8; 13];
    let mut cursor = reversed.len();
    loop {
        cursor -= 1;
        reversed[cursor] = DIGITS[(value % 36) as usize];
        value /= 36;
        if value == 0 {
            break;
        }
    }
    String::from_utf8(reversed[cursor..].to_vec()).expect("base36 is ASCII")
}

fn preview_line(excerpt: &str) -> String {
    let line = excerpt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() <= PREVIEW_MAX_CHARS {
        line.to_string()
    } else {
        format!(
            "{}…",
            line.chars().take(PREVIEW_MAX_CHARS).collect::<String>()
        )
    }
}

/// Why a search returned nothing (6a3i).
///
/// These four cases demand four different next moves, and a bare empty result
/// makes them indistinguishable -- which pushes an agent into speculative
/// retries that cost far more than the search did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissReason {
    /// Nothing is indexed yet.
    EmptyIndex,
    /// Candidates existed but every one was filtered out.
    FiltersExcludedAll,
    /// A channel the query needed could not run.
    ChannelUnavailable,
    /// The index is populated and unfiltered: the term simply is not there.
    NoMatch,
}

impl MissReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmptyIndex => "empty_index",
            Self::FiltersExcludedAll => "filters_excluded_all",
            Self::ChannelUnavailable => "channel_unavailable",
            Self::NoMatch => "no_match",
        }
    }

    /// One actionable next step. Deliberately one, not a menu.
    pub fn next_step(self, first_filter: Option<&str>) -> String {
        match self {
            Self::EmptyIndex => "index this project, then search again".to_owned(),
            Self::FiltersExcludedAll => match first_filter {
                Some(filter) => format!("drop the {filter} filter"),
                None => "widen the search scope".to_owned(),
            },
            Self::ChannelUnavailable => {
                "retry on another channel, or repair the unavailable one".to_owned()
            }
            Self::NoMatch => "try a shorter or more distinctive term".to_owned(),
        }
    }
}

/// Everything known about a zero-hit search (6a3i).
#[derive(Debug, Default, Clone)]
pub struct MissContext {
    /// Channels actually executed.
    pub tried: Vec<String>,
    /// Channels that could not run.
    pub unavailable: Vec<String>,
    /// Effective filters, so the agent can see what scoped it out.
    pub scope: Vec<(String, String)>,
    /// Indexed file count when known; `None` means not consulted.
    pub indexed_files: Option<usize>,
}

impl MissContext {
    /// Classify the miss from the signals available. Order matters: an empty
    /// index explains everything else, and filters explain a miss before a
    /// genuine absence does.
    pub fn reason(&self) -> MissReason {
        if self.indexed_files == Some(0) {
            return MissReason::EmptyIndex;
        }
        if !self.unavailable.is_empty() {
            return MissReason::ChannelUnavailable;
        }
        if !self.scope.is_empty() {
            return MissReason::FiltersExcludedAll;
        }
        MissReason::NoMatch
    }
}

/// Compact diagnostic envelope for a zero-hit search (6a3i).
///
/// A miss should be the cheapest response the engine can produce, and still say
/// enough that the agent's next move is obvious rather than a guess.
pub fn to_compact_miss_json(query: &str, context: &MissContext) -> serde_json::Value {
    let reason = context.reason();
    let first_filter = context.scope.first().map(|(name, _)| name.as_str());
    let mut envelope = serde_json::json!({
        "v": 1,
        "q": query,
        "h": [],
        "zn": 0,
        "why": reason.as_str(),
        "next": reason.next_step(first_filter),
    });
    if !context.tried.is_empty() {
        envelope["tried"] = serde_json::Value::from(context.tried.clone());
    }
    if !context.unavailable.is_empty() {
        envelope["down"] = serde_json::Value::from(context.unavailable.clone());
    }
    if !context.scope.is_empty() {
        envelope["scope"] = serde_json::Value::Object(
            context
                .scope
                .iter()
                .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
                .collect(),
        );
    }
    envelope
}

/// Resolve the `p` table of a compact envelope into plain `id -> path` pairs.
///
/// Handles both encodings: a verbatim string entry, and the folded
/// `[root_index, suffix]` form introduced with the `r` root table (am4a).
/// Entries that reference an unknown root are skipped rather than guessed.
pub fn resolve_compact_paths(envelope: &serde_json::Value) -> Vec<(String, String)> {
    let roots: Vec<&str> = envelope
        .get("r")
        .and_then(serde_json::Value::as_array)
        .map(|roots| roots.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let Some(paths) = envelope.get("p").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    paths
        .iter()
        .filter_map(|(id, entry)| {
            let path = match entry {
                serde_json::Value::String(path) => path.clone(),
                serde_json::Value::Array(parts) => {
                    let index = usize::try_from(parts.first()?.as_u64()?).ok()?;
                    format!("{}{}", roots.get(index)?, parts.get(1)?.as_str()?)
                }
                _ => return None,
            };
            Some((id.clone(), path))
        })
        .collect()
}

/// Compatibility module paths retained for downstream format adapters.
pub mod agent {
    pub use super::{to_agent_capsule_json, to_agent_json, to_compact_json, CompactBudget};
}
pub mod github {
    pub use super::to_github_json;
}
pub mod gitlab {
    pub use super::to_gitlab_json;
}
