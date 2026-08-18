use super::*;
use std::io::{BufReader, Cursor, Write};

#[test]
fn rejects_oversized_files() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    tmp.write_all(&[b'a'; 64]).unwrap();
    tmp.flush().unwrap();
    let err = read_text_capped(tmp.path(), 32).unwrap_err();
    assert!(err.to_string().contains("index cap"), "{err}");
}

#[test]
fn rejects_non_regular_files() {
    let tmp = tempfile::tempdir().unwrap();
    let err = read_text_capped(tmp.path(), 32).unwrap_err();
    assert!(err.to_string().contains("not a regular file"), "{err}");
}

#[test]
fn oversized_line_is_drained_before_next_record() {
    let input = [vec![b'x'; 17], b"\n{\"type\":\"end\"}\n".to_vec()].concat();
    let mut reader = BufReader::with_capacity(3, Cursor::new(input));
    assert!(matches!(
        read_bounded_line(&mut reader, 16).unwrap(),
        Some(BoundedLine::TooLong)
    ));
    let Some(BoundedLine::Line(next)) = read_bounded_line(&mut reader, 16).unwrap() else {
        panic!("valid record after oversized line must remain readable");
    };
    assert_eq!(next, br#"{"type":"end"}"#);
}

#[cfg(unix)]
#[test]
fn root_handle_refuses_symlinked_path_components() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.rs"), "outside").unwrap();
    let handle = RootDir::open(root.path()).unwrap();

    symlink(outside.path(), root.path().join("escape")).unwrap();
    assert!(handle
        .read_text_capped(Path::new("escape/secret.rs"), 1024)
        .is_err());

    symlink(
        outside.path().join("secret.rs"),
        root.path().join("leaf.rs"),
    )
    .unwrap();
    assert!(handle.read_text_capped(Path::new("leaf.rs"), 1024).is_err());
}
