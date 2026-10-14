//! Native static embedding engine: a trained token-embedding table baked
//! into the binary, pooled by plain mean over WordPiece tokens. No model
//! download, no ONNX runtime, no feature flag -- the engine ships with the
//! program and is always available.
//!
//! Weights are a pinned artifact (see `assets/README.md` for provenance):
//! `minishlab/potion-code-16M-v2` @ e9d2a44, MIT, 63,457 tokens x 256 dims,
//! f16. The tokenizer is a hand-rolled BERT WordPiece matching that model's
//! `tokenizer.json` (BertNormalizer: clean text, CJK spacing, lowercase,
//! strip accents; BertPreTokenizer: whitespace + punctuation isolation;
//! WordPiece greedy longest-match, `##` continuations, 100-char word cap).
//!
//! Invariants: embedding is deterministic across machines (table bytes are
//! in the binary), `[UNK]` tokens are dropped before pooling, and text with
//! no known tokens embeds to the zero vector. Startup cost is zero: the
//! vocab blob is binary-searched in place, never materialized into a map.

use anyhow::Result;

use crate::accent_table::ACCENT_BASE;
use crate::embedder::{CostHint, Embedder};

pub const STATIC_DIM: usize = 256;
pub const STATIC_MODEL_ID: &str = "static:potion-code-16M-v2:256";

const UNK_ID: u32 = 1;
const MAX_WORD_CHARS: usize = 100;
/// Longest token in the pinned vocab, in bytes (bounds WordPiece search).
const MAX_TOKEN_BYTES: usize = 65;

static WEIGHTS: &[u8] = include_bytes!("../assets/potion-code-16m-v2.f16.bin");
/// Layout: `[u32 n][u32 offsets (n+1)][u32 ids (n)][token bytes]`, tokens
/// sorted by byte order. Produced by `assets/convert_potion.py`.
static VOCAB: &[u8] = include_bytes!("../assets/potion-code-16m-v2.vocab.bin");

fn vocab_len() -> usize {
    u32::from_le_bytes(VOCAB[0..4].try_into().expect("vocab header")) as usize
}

fn token_at(idx: usize) -> &'static [u8] {
    let n = vocab_len();
    let off = |i: usize| -> usize {
        let p = 4 + i * 4;
        u32::from_le_bytes(VOCAB[p..p + 4].try_into().expect("vocab offset")) as usize
    };
    let bytes_start = 4 + (n + 1) * 4 + n * 4;
    &VOCAB[bytes_start + off(idx)..bytes_start + off(idx + 1)]
}

fn lookup(token: &[u8]) -> Option<u32> {
    let n = vocab_len();
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        match token_at(mid).cmp(token) {
            std::cmp::Ordering::Equal => {
                let p = 4 + (n + 1) * 4 + mid * 4;
                return Some(u32::from_le_bytes(
                    VOCAB[p..p + 4].try_into().expect("vocab id"),
                ));
            }
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
        }
    }
    None
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let mant = (bits & 0x3ff) as u32;
    let f = match exp {
        0 => {
            if mant == 0 {
                0
            } else {
                // Subnormal: shift mantissa up to the implicit leading 1.
                let mut e: i32 = -14;
                let mut m = mant;
                while m & 0x400 == 0 {
                    m <<= 1;
                    e -= 1;
                }
                ((e + 127) as u32) << 23 | ((m & 0x3ff) << 13)
            }
        }
        31 => 0x7f80_0000 | (mant << 13),
        _ => ((exp + 112) << 23) | (mant << 13),
    };
    f32::from_bits((sign << 31) | f)
}

fn is_cjk(cp: u32) -> bool {
    matches!(
        cp,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0xF900..=0xFAFF
            | 0x2F800..=0x2FA1F
    )
}

fn accent_base(cp: u32) -> Option<u32> {
    ACCENT_BASE
        .binary_search_by_key(&cp, |&(k, _)| k)
        .ok()
        .map(|i| ACCENT_BASE[i].1)
}

/// BertNormalizer equivalent: drop control/NUL/replacement chars, map
/// whitespace to spaces, space out CJK chars, lowercase, strip accents.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\0' | '\u{fffd}' => {}
            '\t' | '\n' | '\r' => out.push(' '),
            c if c.is_control() => {}
            c if c.is_whitespace() => out.push(' '),
            c if is_cjk(c as u32) => {
                out.push(' ');
                out.push(c);
                out.push(' ');
            }
            c => {
                for lc in c.to_lowercase() {
                    let cp = lc as u32;
                    if (0x0300..=0x036F).contains(&cp) {
                        // Combining diacritical mark from an already
                        // decomposed sequence: dropped like NFD+strip-Mn.
                        continue;
                    }
                    match accent_base(cp).and_then(char::from_u32) {
                        Some(base) => out.push(base),
                        None => out.push(lc),
                    }
                }
            }
        }
    }
    out
}

/// BERT punctuation test. Non-ASCII punctuation is approximated as
/// "not alphanumeric, not whitespace" (the reference tokenizers use the
/// Unicode P* categories); the difference only affects exotic symbols,
/// which are out-of-vocab either way.
fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace())
}

/// Greedy longest-match WordPiece on one pre-token. `None` = whole word is
/// `[UNK]` (unknown words contribute nothing to pooling).
fn wordpiece(word: &str, out: &mut Vec<u32>) {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() > MAX_WORD_CHARS {
        return;
    }
    let start_len = out.len();
    let mut start = 0;
    let mut cand = String::with_capacity(MAX_TOKEN_BYTES);
    while start < chars.len() {
        let mut end = chars.len();
        let mut found = None;
        while start < end {
            cand.clear();
            if start > 0 {
                cand.push_str("##");
            }
            cand.extend(&chars[start..end]);
            if cand.len() <= MAX_TOKEN_BYTES {
                if let Some(id) = lookup(cand.as_bytes()) {
                    found = Some(id);
                    break;
                }
            }
            end -= 1;
        }
        match found {
            Some(id) => {
                if id != UNK_ID {
                    out.push(id);
                }
                start = end;
            }
            None => {
                out.truncate(start_len);
                return;
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct StaticEmbedder;

impl StaticEmbedder {
    pub fn new() -> Self {
        Self
    }

    /// Token ids that actually get pooled for `text` (`[UNK]` dropped).
    pub fn token_ids(&self, text: &str) -> Vec<u32> {
        let normalized = normalize(text);
        let mut ids = Vec::new();
        for pre in normalized.split(' ') {
            if pre.is_empty() {
                continue;
            }
            // BertPreTokenizer: isolate each punctuation char.
            let mut run_start: Option<usize> = None;
            for (i, c) in pre.char_indices() {
                if is_punct(c) {
                    if let Some(s) = run_start.take() {
                        wordpiece(&pre[s..i], &mut ids);
                    }
                    wordpiece(&pre[i..i + c.len_utf8()], &mut ids);
                } else if run_start.is_none() {
                    run_start = Some(i);
                }
            }
            if let Some(s) = run_start {
                wordpiece(&pre[s..], &mut ids);
            }
        }
        ids
    }
}

impl Embedder for StaticEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                let ids = self.token_ids(text);
                let mut v = vec![0.0f32; STATIC_DIM];
                for id in &ids {
                    let row = (*id as usize) * STATIC_DIM * 2;
                    for (j, x) in v.iter_mut().enumerate() {
                        let p = row + j * 2;
                        *x += f16_to_f32(u16::from_le_bytes([WEIGHTS[p], WEIGHTS[p + 1]]));
                    }
                }
                if !ids.is_empty() {
                    let n = ids.len() as f32;
                    for x in &mut v {
                        *x /= n;
                    }
                }
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt() + 1e-32;
                for x in &mut v {
                    *x /= norm;
                }
                v
            })
            .collect())
    }

    fn dim(&self) -> usize {
        STATIC_DIM
    }

    fn model_id(&self) -> &str {
        STATIC_MODEL_ID
    }

    fn cost_hint(&self) -> CostHint {
        CostHint::LocalCheap
    }
}
