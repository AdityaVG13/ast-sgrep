use super::should_prune_missing_files;
#[test]
fn walk_error_prevents_pruning_from_incomplete_seen_paths() {
    assert!(!should_prune_missing_files(true));
    assert!(should_prune_missing_files(false));
}
