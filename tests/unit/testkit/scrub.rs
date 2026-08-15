use super::Scrubber;
use std::path::Path;

#[test]
fn version_scrub_leaves_schema_version_intact() {
    let input = r#"{"schema_version":"1.0.0","version":"1.4.0","tool":"asgrep"}"#;
    let out = Scrubber::machine_contract().apply(input);
    assert!(
        out.contains(r#""schema_version":"1.0.0""#),
        "schema_version must stay: {out}"
    );
    assert!(
        out.contains(r#""version": "<version>""#) || out.contains(r#""version":"<version>""#),
        "package version must scrub: {out}"
    );
}

#[test]
fn path_placeholders_unix_and_windows() {
    let unix = Scrubber::standard().apply("/Users/ada/src/lib.rs and /tmp/work/a");
    assert!(unix.contains("<HOME>/src/lib.rs"), "{unix}");
    assert!(unix.contains("<TMP>/work/a"), "{unix}");
    let win = Scrubber::standard().apply(r"C:\Users\ada\src\lib.rs");
    assert!(win.contains(r"<HOME>\src\lib.rs"), "{win}");
}

#[test]
fn standard_is_idempotent() {
    let s = Scrubber::standard();
    let input = "/Users/ada/x 0xdeadbeef 550e8400-e29b-41d4-a716-446655440000 2026-08-13T20:00:00Z";
    let once = s.apply(input);
    let twice = s.apply(&once);
    assert_eq!(once, twice);
}

#[test]
fn search_dump_replaces_root() {
    let root = Path::new("/tmp/proj");
    let out = Scrubber::search_dump(root).apply("/tmp/proj/src/main.rs");
    assert!(out.starts_with("<ROOT>"), "{out}");
    assert!(out.contains("src/main.rs"), "{out}");
}

#[test]
fn none_is_identity() {
    let raw = "/Users/ada/secret 1.4.0";
    assert_eq!(Scrubber::none().apply(raw), raw);
}

#[test]
fn doctor_and_status_match_standard() {
    let raw = "/tmp/x 0xabcdef";
    assert_eq!(
        Scrubber::doctor().apply(raw),
        Scrubber::standard().apply(raw)
    );
    assert_eq!(
        Scrubber::status().apply(raw),
        Scrubber::standard().apply(raw)
    );
}
