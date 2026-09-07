pub fn backoff_attempt(attempt: u32) -> u64 {
    1u64 << attempt.min(16)
}
