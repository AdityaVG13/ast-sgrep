use crate::search::{HitKind, SearchHit};

/// Format a search hit as a line per PRD spec.
pub fn format_hit_line(hit: &SearchHit) -> String {
    match hit.kind {
        HitKind::Asgrep => {
            format!(
                "ASGREP: {}:{}-{}: {}",
                hit.file, hit.line_start, hit.line_end, hit.excerpt
            )
        }
        HitKind::Def => {
            format!(
                "DEF: {}: {} span={}..{} | {}",
                hit.file,
                hit.symbol.as_deref().unwrap_or("?"),
                hit.line_start,
                hit.line_end,
                truncate_excerpt(&hit.excerpt, 120)
            )
        }
        HitKind::Caller => {
            format!(
                "CALLER: {}: {} -> {}",
                hit.file,
                hit.caller.as_deref().unwrap_or("?"),
                hit.callee.as_deref().unwrap_or("?")
            )
        }
        HitKind::Graph => {
            format!(
                "GRAPH: {}: {} calls {}",
                hit.file,
                hit.caller.as_deref().unwrap_or("?"),
                hit.callee.as_deref().unwrap_or("?")
            )
        }
        HitKind::Anchor => {
            format!(
                "ANCHOR: {}:{}-{}: {}",
                hit.file,
                hit.line_start,
                hit.line_end,
                truncate_excerpt(&hit.excerpt, 120)
            )
        }
        HitKind::Import => {
            format!(
                "IMPORT: {}:{}: {}",
                hit.file, hit.line_start, hit.excerpt
            )
        }
        HitKind::Pattern => {
            format!(
                "PATTERN: {}:{}-{}: {}",
                hit.file,
                hit.line_start,
                hit.line_end,
                truncate_excerpt(&hit.excerpt, 120)
            )
        }
        HitKind::Embed => {
            format!(
                "EMBED: {}:{}-{}: {}",
                hit.file,
                hit.line_start,
                hit.line_end,
                truncate_excerpt(&hit.excerpt, 120)
            )
        }
    }
}

fn truncate_excerpt(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
