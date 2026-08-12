use super::*;

#[test]
fn every_benchmark_case_has_a_specific_identity_oracle() {
    for case in DEFAULT_SUITE.iter().chain(SELF_SUITE) {
        let expected = benchmark_expectation(case)
            .unwrap_or_else(|| panic!("{} has no identity oracle", case.name));
        assert!(
            expected.is_specific(),
            "{} has no identity oracle",
            case.name
        );
        assert!(expected.max_rank > 0, "{} has a vacuous rank", case.name);
    }
}

#[test]
fn percentile_99_empty_samples_returns_zero_without_panic() {
    assert_eq!(percentile_99(Vec::new()), 0);
}

#[test]
fn percentile_99_single_sample_is_that_value() {
    assert_eq!(percentile_99(vec![42]), 42);
}

#[test]
fn percentile_99_nonempty_is_near_top_of_sorted() {
    let samples: Vec<u64> = (1..=100).collect();
    // p99 of 1..=100 is the 99th percentile index → 99 after sort.
    assert_eq!(percentile_99(samples), 99);
}
