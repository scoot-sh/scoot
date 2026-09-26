//! Printing that never panics: `println!`/`eprintln!` panic when the
//! stream is closed, and a panic aborts under the release profile.

use std::fmt;
use std::io::{self, Write};

#[cfg(test)]
mod tests;

/// Writes `text` to stdout. A reader that went away (`scootbg query |
/// head -c0`) is a quiet success, as for any Unix tool.
pub fn print(text: &str) -> io::Result<()> {
    write_to(io::stdout().lock(), text.as_bytes())
}

/// One line to stderr; infallible, because if stderr is gone too there is
/// nowhere left to report to.
pub fn warn(message: fmt::Arguments<'_>) {
    let mut err = io::stderr().lock();
    let _ = err.write_fmt(message);
    let _ = err.write_all(b"\n");
}

fn write_to(mut out: impl Write, bytes: &[u8]) -> io::Result<()> {
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}
