use super::*;

#[test]
fn clear_all_meta_whitelist_matches_sql() {
    for key in CLEAR_ALL_META_WHITELIST {
        assert!(
            CLEAR_ALL_SQL.contains(&format!("'{key}'")),
            "CLEAR_ALL_SQL must list whitelist key {key}"
        );
    }
}
