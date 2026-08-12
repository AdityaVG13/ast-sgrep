use super::*;
use std::io::Cursor;

#[test]
fn read_utf8_capped_accepts_at_limit() {
    let data = "a".repeat(32);
    let got = read_utf8_capped(Cursor::new(data.as_bytes()), 32).expect("ok");
    assert_eq!(got, data);
}

#[test]
fn read_utf8_capped_rejects_over_limit_without_reading_all() {
    // Reader yields more than max; take() stops at max+1 so we never grow unboundedly.
    let data = vec![b'x'; 10_000];
    let err = read_utf8_capped(Cursor::new(data), 64).expect_err("oversize");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("exceeds max"), "{err}");
}

#[test]
fn raw_machine_detects_codemode_batch_without_json_flag() {
    let args = ["asgrep", "codemode-batch", "req.json"]
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    assert!(raw_machine_output_requested(&args));
}

#[test]
fn raw_machine_still_false_for_plain_search() {
    let args = ["asgrep", "search", "auth", "."]
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    assert!(!raw_machine_output_requested(&args));
}

#[test]
fn write_line_treats_broken_pipe_as_success() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    write_line(&mut Broken, "payload").expect("BrokenPipe must not fail agents");
}

#[test]
fn write_line_propagates_other_io_errors() {
    struct Fail;
    impl Write for Fail {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "nope"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let err = write_line(&mut Fail, "x").expect_err("other errors must propagate");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
}
