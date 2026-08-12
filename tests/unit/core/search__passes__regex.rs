use super::regex_deadline;
use std::time::{Duration, Instant};

#[test]
fn unrepresentable_regex_budget_is_an_error_not_a_panic() {
    assert!(regex_deadline(Instant::now(), Duration::MAX).is_err());
}
