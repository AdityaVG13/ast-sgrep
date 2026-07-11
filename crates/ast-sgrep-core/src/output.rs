use crate::search::{HitKind, SearchHit};
pub fn format_hit_line(hit: &SearchHit) -> String {
    let f = &hit.file;
    let (ls, le) = (hit.line_start, hit.line_end);
    let trunc = |s: &str| truncate_excerpt(s, 120);
    match hit.kind {
        HitKind::Asgrep => format!("ASGREP: {f}:{ls}-{le}: {}", hit.excerpt),
        HitKind::Def => format!(
            "DEF: {f}: {} span={ls}..{le} | {}",
            hit.symbol.as_deref().unwrap_or("?"),
            trunc(&hit.excerpt)
        ),
        HitKind::Caller => format!(
            "CALLER: {f}: {} -> {}",
            hit.caller.as_deref().unwrap_or("?"),
            hit.callee.as_deref().unwrap_or("?")
        ),
        HitKind::Graph => format!(
            "GRAPH: {f}: {} calls {}",
            hit.caller.as_deref().unwrap_or("?"),
            hit.callee.as_deref().unwrap_or("?")
        ),
        HitKind::Anchor => format!("ANCHOR: {f}:{ls}-{le}: {}", trunc(&hit.excerpt)),
        HitKind::Import => format!("IMPORT: {f}:{ls}: {}", hit.excerpt),
        HitKind::Pattern => format!("PATTERN: {f}:{ls}-{le}: {}", trunc(&hit.excerpt)),
        HitKind::Embed => {
            let sym = hit
                .symbol
                .as_deref()
                .map(|s| format!("{s} | "))
                .unwrap_or_default();
            format!("EMBED: {f}:{ls}-{le}: {sym}{}", trunc(&hit.excerpt))
        }
    }
}
fn truncate_excerpt(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
