#![no_main]

use ast_sgrep_core::rank::{fuse_rrf, score_symbol, SCORE_EXACT_SYMBOL};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: (&str, &str, Vec<usize>)| {
    let (term, symbol, mut ranks) = data;
    let symbol_score = score_symbol(term, symbol);
    assert!(symbol_score.is_finite());
    assert!((0.0..=SCORE_EXACT_SYMBOL).contains(&symbol_score));

    let fused = fuse_rrf(&ranks, 60.0);
    assert!(fused.is_finite());
    assert!(fused >= 0.0);

    ranks.reverse();
    let reversed = fuse_rrf(&ranks, 60.0);
    let tolerance = f64::EPSILON * ranks.len().max(1) as f64;
    assert!((fused - reversed).abs() <= tolerance);
});
