use super::*;

#[test]
fn boolish_accepts_common_truthy_spellings() {
    for value in ["1", "true", "TRUE", "yes", "on", " Yes "] {
        assert!(is_boolish_true(value), "{value}");
    }
    for value in ["0", "false", "no", "off", "", "2", "maybe"] {
        assert!(!is_boolish_true(value), "{value}");
    }
}
