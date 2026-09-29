use std::io::{self, Write};

use super::write_to;

struct Fail(io::ErrorKind);

impl Write for Fail {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(self.0.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_closed_reader_is_quiet_and_other_errors_pass_through() {
    assert!(write_to(Fail(io::ErrorKind::BrokenPipe), b"x").is_ok());
    assert_eq!(
        write_to(Fail(io::ErrorKind::PermissionDenied), b"x")
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn everything_is_written() {
    let mut out = Vec::new();
    write_to(&mut out, b"abc\n").unwrap();
    assert_eq!(out, b"abc\n");
}
