use super::*;

#[test]
fn percentile_handles_empty_and_single() {
    assert_eq!(percentile_us(&[], 50), 0);
    assert_eq!(percentile_us(&[10], 50), 10);
    assert_eq!(percentile_us(&[10], 95), 10);
}

#[test]
fn percentile_p95_near_tail() {
    let s: Vec<u64> = (1..=100).collect();
    assert_eq!(percentile_us(&s, 50), 50);
    assert_eq!(percentile_us(&s, 95), 95);
}

#[test]
fn summarize_accumulates() {
    let acc = SpanAcc {
        category: "index",
        evidence: "test",
        samples_us: vec![10, 20, 30, 40],
        sample_count: 4,
        cumulative_us: 100,
    };
    let s = summarize(&acc);
    assert_eq!(s.count, 4);
    assert_eq!(s.cumulative_us, 100);
    assert_eq!(s.p50_us, 20);
}

#[test]
fn summary_count_is_not_capped_with_percentile_samples() {
    let acc = SpanAcc {
        category: "index",
        evidence: "test",
        samples_us: vec![10; MAX_SAMPLES_PER_SPAN],
        sample_count: MAX_SAMPLES_PER_SPAN as u64 + 10,
        cumulative_us: (MAX_SAMPLES_PER_SPAN as u128 + 10) * 10,
    };
    let summary = summarize(&acc);
    assert_eq!(summary.count, MAX_SAMPLES_PER_SPAN as u64 + 10);
    assert_eq!(summary.p95_us, 10);
}

#[test]
fn disabled_span_is_noop() {
    // When flag is unset in the test process, Span/Run must not panic.
    // Do not force ENABLED: other tests may share the process.
    let _s = Span::start("test_span", "test", "unit");
    let _r = Run::start("test_run");
}
