use ast_sgrep_lang::{detect_language, Language};
use std::path::Path;

const NAME_ALIASES: &[(&str, &str)] = &[
    ("rust", "rust"),
    ("typescript", "typescript"),
    ("javascript", "javascript"),
    ("python", "python"),
    ("golang", "go"),
    ("csharp", "csharp"),
    ("c#", "csharp"),
    ("c-sharp", "csharp"),
    ("ruby", "ruby"),
    ("c++", "cpp"),
    ("kotlin", "kotlin"),
    ("TypeScript", "typescript"),
];

#[test]
fn every_source_extension_canonicalizes_and_detects() {
    for (ext, lang) in Language::SOURCE_EXTENSIONS {
        assert_eq!(
            Language::canonical_filter(Some(ext)).as_deref(),
            Some(lang.as_str()),
            "extension {ext}"
        );
        let rel = format!("n.{ext}");
        let path = Path::new(&rel);
        assert_eq!(
            detect_language(path, None),
            Some(*lang),
            "detect_language({ext})"
        );
        assert_eq!(Language::from_extension(ext), Some(*lang));
        assert_eq!(
            Language::from_extension(&ext.to_ascii_uppercase()),
            Some(*lang)
        );
    }
}

#[test]
fn stored_ids_and_name_aliases_parse() {
    for lang in Language::all() {
        assert_eq!(Language::parse(lang.as_str()), Some(*lang));
    }
    for (raw, stored) in NAME_ALIASES {
        assert_eq!(
            Language::canonical_filter(Some(raw)).as_deref(),
            Some(*stored),
            "alias {raw}"
        );
    }
}

#[test]
fn aliases_map_to_stored_ids() {
    assert_eq!(
        Language::canonical_filter(Some("ts")).as_deref(),
        Some("typescript")
    );
    assert_eq!(
        Language::canonical_filter(Some("hpp")).as_deref(),
        Some("cpp")
    );
    assert_eq!(Language::canonical_filter(Some("h")).as_deref(), Some("c"));
    assert_eq!(
        Language::canonical_filter(Some("c#")).as_deref(),
        Some("csharp")
    );
}

#[test]
fn blank_is_no_filter() {
    assert_eq!(Language::canonical_filter(None), None);
    assert_eq!(Language::canonical_filter(Some("")), None);
    assert_eq!(Language::canonical_filter(Some("  ")), None);
}

#[test]
fn unknown_labels_lowercase() {
    assert_eq!(
        Language::canonical_filter(Some("Fortran")).as_deref(),
        Some("fortran")
    );
}
