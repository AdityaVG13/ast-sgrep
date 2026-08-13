//! Token-budgeted, AST-aware result rendering (bead ast-sgrep-tef-token-budget-m38g).
//!
//! Result COUNT is a poor proxy for token cost: five full functions can dwarf
//! twenty signatures. This module renders each hit at one of several detail
//! levels and chooses the mix that maximizes useful evidence under a token
//! budget, instead of truncating every excerpt to N lines.

use ast_sgrep_core::SearchHit;

/// A token unit is one UTF-8 byte.
///
/// Deliberately conservative and model-independent: a byte-fallback tokenizer
/// can always represent a byte with at most one token, so this ceiling cannot
/// understate a real tokenizer's count. It is not an estimate of any one
/// vendor's vocabulary.
pub type TokenUnits = usize;

/// How much of a result to render (m38g).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DetailLevel {
    /// Location and symbol only: enough to decide whether to expand.
    Metadata,
    /// Declaration signature plus the matched line.
    Signature,
    /// The smallest meaningful enclosing block, with gap markers.
    Block,
    /// The whole excerpt the index holds for this symbol.
    Full,
}

impl DetailLevel {
    pub const ALL: [Self; 4] = [Self::Metadata, Self::Signature, Self::Block, Self::Full];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Signature => "signature",
            Self::Block => "block",
            Self::Full => "full",
        }
    }
}

/// Output budget for one response (m38g).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputBudget {
    /// Total token units the rendered results may occupy.
    pub max_tokens: TokenUnits,
    /// Detail level a result gets before the budget is considered.
    pub default_detail: DetailLevel,
}

impl Default for OutputBudget {
    fn default() -> Self {
        Self {
            max_tokens: 900,
            default_detail: DetailLevel::Block,
        }
    }
}

/// One result rendered at a chosen detail level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedHit {
    pub detail: DetailLevel,
    pub body: String,
    pub cost: TokenUnits,
}

/// Marker for source omitted from an excerpt. Short on purpose: it is emitted
/// once per elided run, and its only job is to stop a reader assuming the lines
/// are contiguous.
pub const GAP_MARKER: &str = "…";

/// Render a hit at a given detail level (m38g).
///
/// Every level returns verifiable source text or nothing. No level generates a
/// summary, because a summary can describe behavior the code does not have.
pub fn render(hit: &SearchHit, detail: DetailLevel) -> RenderedHit {
    let body = match detail {
        DetailLevel::Metadata => String::new(),
        DetailLevel::Signature => signature_and_match(hit),
        DetailLevel::Block => ast_block(hit),
        DetailLevel::Full => hit.excerpt.trim_end().to_owned(),
    };
    let cost = body.len();
    RenderedHit { detail, body, cost }
}

/// Declaration signature plus the matched line (m38g).
fn signature_and_match(hit: &SearchHit) -> String {
    let lines: Vec<&str> = hit.excerpt.lines().collect();
    let Some(signature) = lines.iter().find(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let signature = signature.trim_end();
    // The most informative non-signature line is the first non-trivial body
    // line; a brace or a blank tells the reader nothing.
    let matched = lines
        .iter()
        .skip(1)
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && *line != "{" && *line != "}");
    match matched {
        Some(matched) => format!("{signature}\n{GAP_MARKER} {matched}"),
        None => signature.to_owned(),
    }
}

/// The smallest meaningful enclosing block, with gap markers (m38g).
///
/// Keeps the declaration, the block structure around the interesting lines, and
/// the closing delimiter, replacing omitted runs with a single gap marker so the
/// reader can never mistake the excerpt for contiguous source.
fn ast_block(hit: &SearchHit) -> String {
    let lines: Vec<&str> = hit.excerpt.lines().map(|line| line.trim_end()).collect();
    if lines.len() <= BLOCK_MAX_LINES {
        return lines.join("\n").trim_end().to_owned();
    }

    let mut keep = vec![false; lines.len()];
    // Always keep the declaration and the closing delimiter.
    if let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) {
        keep[first] = true;
    }
    if let Some(last) = lines.iter().rposition(|line| !line.trim().is_empty()) {
        keep[last] = true;
    }
    // Keep lines that open or close structure, and control-flow heads: these
    // are the shape of the function, which is what a reader is scanning for.
    let mut kept = keep.iter().filter(|k| **k).count();
    for (index, line) in lines.iter().enumerate() {
        if kept >= BLOCK_MAX_LINES {
            break;
        }
        if keep[index] {
            continue;
        }
        if is_structural_line(line) {
            keep[index] = true;
            kept += 1;
        }
    }

    let mut out = String::new();
    let mut gapped = false;
    for (index, line) in lines.iter().enumerate() {
        if keep[index] {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
            gapped = false;
        } else if !gapped {
            if !out.is_empty() {
                out.push('\n');
            }
            // Indent the marker like the line it replaces, so structure reads.
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push_str(&indent);
            out.push_str(GAP_MARKER);
            gapped = true;
        }
    }
    out.trim_end().to_owned()
}

const BLOCK_MAX_LINES: usize = 12;

/// Lines that carry block structure or control flow.
fn is_structural_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    const HEADS: [&str; 10] = [
        "if ", "for ", "while ", "match ", "return", "else", "try", "catch", "switch", "case ",
    ];
    HEADS.iter().any(|head| trimmed.starts_with(head))
        || trimmed.ends_with('{')
        || trimmed == "}"
        || trimmed.starts_with("} ")
}

/// Choose a detail level per hit so the whole response fits the budget (m38g).
///
/// Deterministic by construction: hits are processed in rank order, and every
/// comparison breaks ties by index, so the same input always yields the same
/// assignment. Ranked-first results are funded before later ones, then leftover
/// budget is spent upgrading results in rank order -- the density-greedy step.
pub fn select(hits: &[SearchHit], budget: OutputBudget) -> Vec<RenderedHit> {
    if hits.is_empty() {
        return Vec::new();
    }
    // Floor: every hit is at least addressable, so a budget can never hide a
    // result entirely -- it degrades detail instead of dropping evidence.
    let mut chosen: Vec<RenderedHit> = hits
        .iter()
        .map(|hit| render(hit, DetailLevel::Metadata))
        .collect();
    let mut spent: TokenUnits = chosen.iter().map(|r| r.cost).sum();

    // Upgrade in rank order, never exceeding the budget. A later result cannot
    // outbid an earlier one, which keeps the assignment stable and explainable.
    for target in [
        DetailLevel::Signature,
        DetailLevel::Block,
        DetailLevel::Full,
    ] {
        if target > budget.default_detail {
            break;
        }
        for (index, hit) in hits.iter().enumerate() {
            if chosen[index].detail >= target {
                continue;
            }
            let candidate = render(hit, target);
            let delta = candidate.cost.saturating_sub(chosen[index].cost);
            if spent + delta <= budget.max_tokens {
                spent += delta;
                chosen[index] = candidate;
            }
        }
    }
    chosen
}

/// Total token units a rendering plan occupies.
pub fn plan_cost(rendered: &[RenderedHit]) -> TokenUnits {
    rendered.iter().map(|r| r.cost).sum()
}
