//! Single-pass fusion dedup: interned u32 keys instead of cloned String HashMap keys.
//!
//! Identity matches historical `dedup_hits` (file + span + symbol/caller/callee
//! Options). Channel kind is evidence, not identity (vh65).

use super::types::{assign_hit_confidence, merge_channel_evidence, SearchHit};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DedupKey {
    file: u32,
    line_start: u32,
    line_end: u32,
    symbol: u32,
    caller: u32,
    callee: u32,
    // PASS 131 (130A-F4): the matched-node byte span joins the key. The
    // dedup's rationale is collapsing the SAME node surfaced by several
    // channels — two hits with equal (file, lines, span). Two INDEPENDENT
    // same-line hits (`q(1); q(2);` under `q($A)`) carry different spans
    // and are distinct sg rows. Span-less rows (`None`) keep the exact
    // pre-131 keying (the index lane has no byte columns — registered
    // residual, CNR §45).
    byte_span: Option<(usize, usize)>,
}

struct Interner {
    ids: HashMap<Box<str>, u32>,
}

impl Interner {
    fn new(cap: usize) -> Self {
        Self {
            ids: HashMap::with_capacity(cap),
        }
    }

    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = self.ids.len() as u32;
        self.ids.insert(s.into(), id);
        id
    }

    /// Preserve `Option` identity: `None` is not `Some("")`.
    fn intern_opt(&mut self, s: Option<&str>) -> u32 {
        match s {
            None => 0,
            Some(s) => self.intern(s).saturating_add(1),
        }
    }
}

/// Fuse channel hits for the same location. Best-score and contributor merge
/// stay in `merge_channel_evidence`.
pub fn dedup_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut intern = Interner::new(hits.len());
    let mut best: Vec<SearchHit> = Vec::with_capacity(hits.len());
    let mut positions: HashMap<DedupKey, usize> = HashMap::with_capacity(hits.len());
    for hit in hits {
        let key = DedupKey {
            file: intern.intern(&hit.file),
            line_start: hit.line_start,
            line_end: hit.line_end,
            symbol: intern.intern_opt(hit.symbol.as_deref()),
            caller: intern.intern_opt(hit.caller.as_deref()),
            callee: intern.intern_opt(hit.callee.as_deref()),
            byte_span: hit.byte_span,
        };
        if let Some(&index) = positions.get(&key) {
            merge_channel_evidence(&mut best[index], hit);
        } else {
            positions.insert(key, best.len());
            best.push(hit);
        }
    }
    assign_hit_confidence(&mut best);
    best
}
