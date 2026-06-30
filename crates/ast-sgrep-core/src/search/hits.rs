use std::collections::HashSet;

use super::types::SearchHit;

pub fn matches_lang(language: Option<&str>, filter: Option<&str>) -> bool {
    filter.is_none_or(|lang| language == Some(lang))
}

pub fn dedup_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for hit in hits {
        let key = (
            hit.kind.as_str(),
            hit.file.clone(),
            hit.line_start,
            hit.line_end,
            hit.symbol.clone(),
            hit.caller.clone(),
            hit.callee.clone(),
        );
        if seen.insert(key) {
            out.push(hit);
        }
    }
    out
}
