use super::{lock_clear_on_poison, query_embed_cache};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn query_embed_cache_poison_recovers_fail_closed() {
    let cache = query_embed_cache();
    {
        let mut guard = lock_clear_on_poison(cache, |map| map.clear());
        guard.insert("probe".into(), vec![1.0]);
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _guard = cache.lock().unwrap();
        panic!("intentional query-embed cache poison");
    }));
    assert!(cache.is_poisoned(), "setup: lock should be poisoned");
    let guard = lock_clear_on_poison(cache, |map| map.clear());
    assert!(!cache.is_poisoned(), "clear_poison after recover");
    assert!(
        guard.is_empty(),
        "poison must clear untrusted entries before reuse"
    );
}
