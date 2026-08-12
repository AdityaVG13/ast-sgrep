use super::*;
use std::io::Write;

#[test]
fn maps_existing_file() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    tmp.write_all(b"hello-mmap").unwrap();
    tmp.flush().unwrap();
    let file = File::open(tmp.path()).unwrap();
    let map = map_readonly(&file).unwrap();
    assert_eq!(&map[..], b"hello-mmap");
}
