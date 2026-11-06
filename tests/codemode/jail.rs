//! F8: one jail function — traversal-rejection matrix.
//!
//! INTENT: every per-call path that must stay under the session workspace
//! resolves through a single `jail_join` primitive holding the strictest
//! semantics of the former copies: decoded-component `..` rejection (a
//! `sub/../../x` that resolves back inside is still rejected), symlink-aware
//! containment, UTF-8 canonical paths, and an explicit existence policy per
//! caller contract (strict for roots/reads, missing-leaf-tolerant for
//! targeted reindex). Backslash spellings follow platform component rules
//! (literal names on Unix, separators on Windows).
//! KILLS: detector-coverage loss, symlink escape, non-UTF8 canon confusion,
//! nested-dir traversal, resolve-inside `..` leniency in any single copy.

use ast_sgrep_codemode::jail::{jail_join, JailKind, JailResolve};

fn kind_of(root: &std::path::Path, raw: &str, resolve: JailResolve) -> JailKind {
    jail_join(root, raw, resolve).expect_err("must reject").kind
}

fn tmp_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn dotdot_components_reject_before_touching_disk() {
    let root = tmp_root();
    // None of these need to exist: the `..` pre-reject fires first.
    for raw in [
        "../escape.rs",
        "sub/../../escape.rs",
        "a/b/../../../x",
        "..",
        "sub/..",
        "/../etc",
    ] {
        assert_eq!(
            kind_of(root.path(), raw, JailResolve::Strict),
            JailKind::Traversal,
            "raw={raw:?}"
        );
    }
}

#[test]
fn backslash_spelling_follows_platform_component_rules() {
    // On Unix a backslash is an ordinary filename byte, so `literal\..\name`
    // is one literal name and must resolve (pinned end-to-end by
    // read_refs_preserve_literal_backslashes). On Windows the same spelling
    // is a real traversal and must reject.
    let root = tmp_root();
    #[cfg(unix)]
    {
        std::fs::write(root.path().join("literal\\..\\name.ts"), "ok\n").expect("write");
        let got = jail_join(root.path(), "literal\\..\\name.ts", JailResolve::Strict)
            .expect("literal backslash name must resolve on unix");
        assert!(got.ends_with("literal\\..\\name.ts"), "got={got:?}");
    }
    #[cfg(windows)]
    {
        assert_eq!(
            kind_of(root.path(), "literal\\..\\name.ts", JailResolve::Strict),
            JailKind::Traversal
        );
    }
}

#[test]
fn absolute_outside_and_symlink_escape_are_outside() {
    let root = tmp_root();
    let outside = tmp_root();
    std::fs::write(outside.path().join("evil.txt"), "x\n").expect("write");

    // Absolute path outside the root.
    let abs = outside.path().join("evil.txt");
    assert_eq!(
        kind_of(root.path(), abs.to_str().unwrap(), JailResolve::Strict),
        JailKind::Outside
    );
    // `..`-free relative path that escapes via a symlink still fails
    // containment AFTER canonicalization.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), root.path().join("link"))
            .expect("symlink");
        assert_eq!(
            kind_of(root.path(), "link/evil.txt", JailResolve::Strict),
            JailKind::Outside
        );
        // Negative control: a symlink pointing inside resolves fine.
        let inner = root.path().join("inner");
        std::fs::create_dir(&inner).expect("mkdir");
        std::fs::write(inner.join("f.txt"), "ok\n").expect("write");
        std::os::unix::fs::symlink(&inner, root.path().join("link2")).expect("symlink");
        let got = jail_join(root.path(), "link2/f.txt", JailResolve::Strict).expect("inside");
        assert!(got.ends_with("inner/f.txt"), "got={got:?}");
    }
}

#[test]
fn benign_dots_resolve_and_missing_is_unresolvable() {
    let root = tmp_root();
    for name in ["a..b", "..foo", "foo.."] {
        std::fs::write(root.path().join(name), "ok\n").expect("write");
        let got = jail_join(root.path(), name, JailResolve::Strict)
            .expect("benign dots must resolve");
        assert!(got.ends_with(name), "got={got:?}");
    }
    // Strict policy: the target must exist (root overrides, reads).
    assert_eq!(
        kind_of(root.path(), "missing.txt", JailResolve::Strict),
        JailKind::Unresolvable
    );
}

#[test]
fn nested_inside_paths_resolve_to_contained_canonical() {
    let root = tmp_root();
    let dir = root.path().join("a");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("b.txt"), "ok\n").expect("write");
    let got = jail_join(root.path(), "a/b.txt", JailResolve::Strict).expect("inside");
    assert!(got.is_absolute(), "got={got:?}");
    let base = root.path().canonicalize().expect("canon root");
    assert!(got.starts_with(&base), "got={got:?} base={base:?}");
    assert!(got.ends_with("a/b.txt"), "got={got:?}");
    // Empty raw preserves root_arg's contract: no override means the root.
    let empty = jail_join(root.path(), "", JailResolve::Strict).expect("empty");
    assert_eq!(empty, base);
}

#[test]
fn lenient_leaf_keeps_missing_leaf_identity_inside_root() {
    // index_repo targeted paths stay missing-tolerant: a not-yet-existing
    // (or deleted) leaf keeps its identity instead of erroring, while
    // `..` and containment still reject.
    let root = tmp_root();
    let got = jail_join(root.path(), "newdir/newfile.txt", JailResolve::LenientLeaf)
        .expect("lenient leaf");
    assert!(got.ends_with("newdir/newfile.txt"), "got={got:?}");
    assert_eq!(
        kind_of(root.path(), "../escape.rs", JailResolve::LenientLeaf),
        JailKind::Traversal
    );
    let outside = tmp_root();
    let abs = outside.path().join("evil.txt");
    assert_eq!(
        kind_of(root.path(), abs.to_str().unwrap(), JailResolve::LenientLeaf),
        JailKind::Outside
    );
}

// macOS (APFS) rejects non-UTF-8 filenames at the syscall level, so this
// arm is only exercisable where the filesystem accepts arbitrary bytes.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_canon_rejects_fail_closed() {
    // Bytes that are not valid UTF-8 can still name real files; the jail
    // must reject the non-UTF-8 canonical path instead of letting it
    // through (strictest copy's semantics).
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let parent = tmp_root();
    let raw_dir = OsString::from_vec(b"r\xffoot".to_vec());
    let non_utf8_root = parent.path().join(&raw_dir);
    std::fs::create_dir(&non_utf8_root).expect("mkdir");
    std::fs::write(non_utf8_root.join("f.txt"), "ok\n").expect("write");
    assert_eq!(
        kind_of(&non_utf8_root, "f.txt", JailResolve::Strict),
        JailKind::NonUtf8
    );
}
