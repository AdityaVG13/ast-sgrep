use super::Language;

#[test]
fn all_languages_round_trip_as_str_parse() {
    for &lang in Language::all() {
        assert_eq!(Language::parse(lang.as_str()), Some(lang));
        assert_eq!(Language::normalize_id(lang.as_str()), lang.as_str());
    }
    assert_eq!(Language::all().len(), 13);
}

#[test]
fn title_case_and_aliases_normalize_to_as_str() {
    assert_eq!(Language::normalize_id("Rust"), "rust");
    assert_eq!(Language::normalize_id("TypeScript"), "typescript");
    assert_eq!(Language::normalize_id("C#"), "csharp");
    assert_eq!(Language::normalize_id("CSharp"), "csharp");
    assert_eq!(Language::normalize_id("C++"), "cpp");
    assert_eq!(Language::normalize_id("Kotlin"), "kotlin");
    assert_eq!(Language::normalize_id("PHP"), "php");
    assert_eq!(Language::normalize_id("Swift"), "swift");
}
