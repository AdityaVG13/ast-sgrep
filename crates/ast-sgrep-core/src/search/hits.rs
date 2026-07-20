
use super::types::SearchHit; pub fn matches_lang(language: Option<&str>, filter: Option<&str>) -> bool {
    filter.is_none_or(|lang| language == Some(lang))
} pub fn dedup_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut best: Vec<SearchHit> = Vec::with_capacity(hits.len()); let mut positions: std::collections::HashMap<_, usize> = std::collections::HashMap::new(); for hit in hits {
        let key = (
            hit.kind.as_str(), hit.file.clone(), hit.line_start, hit.line_end, hit.symbol.clone(), hit.caller.clone(), hit.callee.clone(),
        ); if let Some(&index) = positions.get(&key) {
            if hit.score > best[index].score {
                best[index] = hit;
            }
        } else {
            positions.insert(key, best.len()); best.push(hit);
        }
    } best
}
