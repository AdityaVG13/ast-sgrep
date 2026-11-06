//! C# using-directive lane.

use super::*;
use crate::extract::node_text;
use std::collections::BTreeMap;
use tree_sitter::Node;

pub(crate) fn cs_using_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::CsUsing {
        global_prefix,
        keyword_run,
        target,
        target_suffix,
        alias_type,
    } = template
    else {
        return None;
    };
    // Candidate-side demarcation: parse the directive's own shape —
    // optional `global`, the `using` keyword, the `static`/`unsafe` keyword
    // run, then a name or `name = type` alias. The reference walks the AST
    // where comments are trivia, so the SCANS see the comment-stripped
    // clause text (and the captures bind trivia-free slot texts) while the
    // emitted span/MATCH keep the original bytes. The demarcation skips
    // are CLASS-STRICT — the CANDIDATE-side class is is_sg_cs_gap_junk
    // (the parse skips full Unicode White_Space + FEFF + control junk
    // here), while the PATTERN-side class of record (directive_template)
    // stays is_sg_cs_trivia.
    let raw = node_text(node, source)?;
    let text = strip_comment_spans(raw);
    let mut rest = text.trim().trim_start_matches(is_sg_cs_gap_junk);
    let mut had_global = false;
    if let Some(after) = rest.strip_prefix("global") {
        if after.starts_with(is_sg_cs_gap_junk) {
            had_global = true;
            rest = after.trim_start_matches(is_sg_cs_gap_junk);
        }
    }
    let after_using = rest.strip_prefix("using")?;
    if !after_using.starts_with(is_sg_cs_gap_junk) {
        return None;
    }
    let mut rest = after_using.trim_start_matches(is_sg_cs_gap_junk);
    let mut had_keywords: Vec<CsUsingKeyword> = Vec::new();
    loop {
        let mut consumed = false;
        for keyword in [CsUsingKeyword::Static, CsUsingKeyword::Unsafe] {
            if let Some(after) = rest.strip_prefix(keyword.spell()) {
                if after.starts_with(is_sg_cs_gap_junk) {
                    had_keywords.push(keyword);
                    rest = after.trim_start_matches(is_sg_cs_gap_junk);
                    consumed = true;
                    break;
                }
            }
        }
        if !consumed {
            break;
        }
    }
    // `global` is demand-only; the keyword run must EQUAL the pattern's.
    if *global_prefix && !had_global {
        return None;
    }
    if *keyword_run != had_keywords {
        return None;
    }
    let inner = rest
        .trim()
        .trim_start_matches(is_sg_cs_gap_junk)
        .strip_suffix(';')?
        .trim_end_matches(is_sg_cs_gap_junk);
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), raw.to_string());
    match alias_type {
        Some(alias) => {
            let eq_at = top_level_equals(inner)?;
            let (lhs, rhs) = (
                inner[..eq_at].trim_matches(is_sg_cs_gap_junk),
                inner[eq_at + 1..].trim_matches(is_sg_cs_gap_junk),
            );
            if lhs.is_empty() || rhs.is_empty() {
                return None;
            }
            target.bind_or_match(&mut captures, lhs)?;
            alias.bind_or_match(&mut captures, rhs)?;
        }
        None => {
            // The plain face: one name token. The `=` alias spelling and
            // any decorated run never reach here (the run equality and
            // the trivia guard refuse them). Junk glued inside ONE name
            // token refuses. The law is PER-TOKEN-GAP — junk at an
            // identifier↔`.` boundary is skipped by the parse and the
            // qualified name BINDS with the junk RETAINED in the capture.
            // Segment law: split the top-level `.`, trim each segment's
            // EDGE junk runs, refuse a segment whose INTERIOR carries junk
            // or that empties out; the capture binds the raw
            // (junk-retained) section text.
            if inner.is_empty() || top_level_equals(inner).is_some() {
                return None;
            }
            let cand_segs: Vec<&str> = inner.split('.').collect();
            if cand_segs.iter().any(|seg| {
                let core = seg.trim_matches(is_sg_cs_gap_junk);
                core.is_empty() || core.chars().any(is_sg_cs_gap_junk)
            }) {
                return None;
            }
            // A LITERAL qualified-name pattern obeys the same per-segment law
            // — the reference skips boundary junk and comment gaps on the
            // literal face too, while glue INSIDE one segment refuses. Meta
            // targets keep the junk-retained capture.
            match (target, target_suffix) {
                (LaneName::Meta(meta), Some(suffix)) => {
                    let mut qualifier = inner;
                    for literal in suffix.rsplit('.') {
                        let (head, tail) = qualifier.rsplit_once('.')?;
                        if tail.trim_matches(is_sg_cs_gap_junk) != literal {
                            return None;
                        }
                        qualifier = head.trim_end_matches(is_sg_cs_gap_junk);
                    }
                    meta.bind(&mut captures, qualifier)?;
                }
                (LaneName::Meta(_), None) => {
                    target.bind_or_match(&mut captures, inner)?;
                }
                (LaneName::Literal(literal), _) => {
                    let lit_segs: Vec<&str> = literal.split('.').collect();
                    if lit_segs.len() != cand_segs.len() {
                        return None;
                    }
                    for (seg, lit) in cand_segs.iter().zip(lit_segs.iter()) {
                        if seg.trim_matches(is_sg_cs_gap_junk) != *lit {
                            return None;
                        }
                    }
                }
            }
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}
