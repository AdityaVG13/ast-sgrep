//! Query-local interned tables and the identity merge that uses them.
//!
//! `NameIntern` and `FileCsr` are dense indexes for one query: names become
//! u32 ids, file paths become CSR posting lists. `dedup_hits` collapses
//! channel hits that share (file, span, symbol/caller/callee). Ranking fusion
//! (RRF) lives in `crate::fusion`, not here.

use super::types::{assign_hit_confidence, merge_channel_evidence, SearchHit};

#[derive(Clone, Copy, PartialEq, Eq)]
struct DedupKey {
    file: u32,
    line_start: u32,
    line_end: u32,
    symbol: u32,
    caller: u32,
    callee: u32,
    // The matched-node byte span joins the key. The dedup's rationale is
    // collapsing the SAME node surfaced by several
    // channels — two hits with equal (file, lines, span). Two INDEPENDENT
    // same-line hits (`q(1); q(2);` under `q($A)`) carry different spans
    // and are distinct reference rows. Span-less rows (`None`) keep the
    // exact
    // pre-span keying (the index lane has no byte columns — a known
    // residual).
    byte_span: Option<(usize, usize)>,
}

/// Query-local name table: one byte arena, ids are first-seen indices.
/// Lookup is binary search over a name-sorted id side table.
pub(crate) struct NameIntern {
    bytes: Vec<u8>,
    offs: Vec<u32>,
    sorted: Vec<u32>,
}

impl NameIntern {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(cap.saturating_mul(8)),
            offs: vec![0],
            sorted: Vec::with_capacity(cap),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.offs.len() - 1
    }

    pub(crate) fn intern(&mut self, s: &str) -> u32 {
        match self
            .sorted
            .binary_search_by(|&id| self.get(id).cmp(s))
        {
            Ok(pos) => self.sorted[pos],
            Err(pos) => {
                let id = (self.offs.len() - 1) as u32;
                self.bytes.extend_from_slice(s.as_bytes());
                self.offs.push(self.bytes.len() as u32);
                self.sorted.insert(pos, id);
                id
            }
        }
    }

    pub(crate) fn intern_opt(&mut self, s: Option<&str>) -> u32 {
        match s {
            None => 0,
            Some(s) => self.intern(s).saturating_add(1),
        }
    }

    pub(crate) fn get(&self, id: u32) -> &str {
        let i = id as usize;
        if i + 1 >= self.offs.len() {
            return "";
        }
        let start = self.offs[i] as usize;
        let end = self.offs[i + 1] as usize;
        self.bytes
            .get(start..end)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or("")
    }
}

/// Fuse channel hits for the same location. Best-score and contributor merge
/// stay in `merge_channel_evidence`.
pub fn dedup_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut intern = NameIntern::new(hits.len());
    let mut best: Vec<SearchHit> = Vec::with_capacity(hits.len());
    let mut keys: Vec<DedupKey> = Vec::with_capacity(hits.len());
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
        if let Some(index) = keys.iter().position(|seen| *seen == key) {
            merge_channel_evidence(&mut best[index], hit);
        } else {
            keys.push(key);
            best.push(hit);
        }
    }
    assign_hit_confidence(&mut best);
    best
}

/// Sorted unique names + concatenated ids. `get` is binary search, not a hash.
pub(crate) struct FileCsr {
    names: Vec<Box<str>>,
    offsets: Vec<u32>,
    ids: Vec<u32>,
}

impl FileCsr {
    pub(crate) fn build(mut pairs: Vec<(&str, u32)>) -> Self {
        pairs.sort_unstable_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(&right.1)));
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        let mut ids = Vec::with_capacity(pairs.len());
        for (path, id) in pairs {
            if names.last().map(Box::as_ref) != Some(path) {
                names.push(Box::from(path));
                offsets.push(ids.len() as u32);
            }
            ids.push(id);
        }
        offsets.push(ids.len() as u32);
        Self {
            names,
            offsets,
            ids,
        }
    }

    pub(crate) fn get(&self, path: &str) -> &[u32] {
        let Ok(index) = self.names.binary_search_by(|name| name.as_ref().cmp(path)) else {
            return &[];
        };
        let start = self.offsets[index] as usize;
        let end = self.offsets[index + 1] as usize;
        &self.ids[start..end]
    }
}
