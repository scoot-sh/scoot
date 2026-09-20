//! Stdout/stderr writes for the client (`scootctl`, and the `scoot msg`
//! alias through it) that survive a closed pipe.
//!
//! Rust ignores SIGPIPE process-wide, so a `println!` to a closed stdout
//! panics (`failed printing to stdout: Broken pipe`, exit 101) instead of
//! dying quietly the way standard Unix tools do. Every client-binary write
//! to stdout goes through [`print_line`], [`write_str`] or [`write_bytes`]
//! so a vanished reader maps to a quiet success (exit 0) rather than a
//! panic; every stderr write goes through [`warn`], which never fails at
//! all. A vanished *socket* peer stays an ordinary error, which is why
//! EPIPE is mapped only here, at the stdio edge, and never inside
//! `scoot_ipc::Client`.
//!
//! Deliberately *not* a restored SIGPIPE disposition: the `scoot` binary
//! also hosts the compositor, which must never die to a signal because one
//! IPC peer stopped reading, and the disposition is process-wide -- placing
//! it on a client-only path would leave a future refactor one move away
//! from handing the compositor a crash-on-disconnect. Per-write handling
//! keeps the two halves' failure modes separate with safe code and no new
//! dependencies (the alternative needs `unsafe` to set a process-wide
//! disposition for what is really a per-write concern).
//!
//! Quiet exit 0 (rather than death-by-SIGPIPE's 141) follows the
//! established Rust CLI convention (ripgrep, fd, bat): the truncated
//! consumer got what it wanted, so success is the truthful status, and a
//! `| head` pipeline stays green under `set -o pipefail`.

use std::fmt;
use std::io::{self, Write};

/// Writes one `\n`-terminated line to stdout; see the module doc for the
/// EPIPE contract. Locks stdout once for the line and the flush (this is a
/// one-shot CLI, not a hot path, but there is no reason to lock twice).
pub fn print_line(line: &str) -> io::Result<()> {
    print_to(io::stdout().lock(), line)
}

/// Writes pre-terminated text (e.g. `--help`) to stdout; same EPIPE
/// contract as [`print_line`].
pub fn write_str(text: &str) -> io::Result<()> {
    write_to(io::stdout().lock(), text.as_bytes())
}

/// Writes raw bytes (e.g. a screenshot PNG) to stdout; same EPIPE contract
/// as [`print_line`]. Flushes explicitly rather than relying on the
/// runtime's exit-time flush, so the bytes are on the wire even if a later
/// refactor takes an early `process::exit` somewhere down this path.
pub fn write_bytes(bytes: &[u8]) -> io::Result<()> {
    write_to(io::stdout().lock(), bytes)
}

/// An advisory stderr line that never fails the process: if stderr itself
/// is closed there is nowhere to report to, and panicking (what
/// `eprintln!` does on EPIPE) would turn a warning into a crash. Takes
/// `Arguments` so callers pay no allocation for a line nobody may read.
pub fn warn(message: fmt::Arguments<'_>) {
    let mut err = io::stderr().lock();
    let _ = err.write_fmt(message);
    let _ = err.write_all(b"\n");
}

/// Maps an output-write result to the process outcome: `BrokenPipe` (the
/// reader went away) is success, anything else passes through. Split out
/// from [`print_to`] so the mapping unit-tests without a real pipe.
fn settle(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

fn print_to(mut out: impl Write, line: &str) -> io::Result<()> {
    settle(write_to_inner(&mut out, line.as_bytes(), true))
}

fn write_to(mut out: impl Write, bytes: &[u8]) -> io::Result<()> {
    settle(write_to_inner(&mut out, bytes, false))
}

fn write_to_inner(out: &mut impl Write, bytes: &[u8], newline: bool) -> io::Result<()> {
    out.write_all(bytes)?;
    if newline {
        out.write_all(b"\n")?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    /// A writer scripted to fail, so the mapping tests don't need a pipe.
    struct Fail(io::ErrorKind);

    impl Write for Fail {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                self.0,
                format!("scripted {:?}", bytes.len()),
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_is_quiet_success_and_other_errors_pass_through() {
        assert!(print_to(Fail(io::ErrorKind::BrokenPipe), "hello").is_ok());
        assert!(write_to(Fail(io::ErrorKind::BrokenPipe), b"hello").is_ok());
        let error = print_to(Fail(io::ErrorKind::PermissionDenied), "hello").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let error = write_to(Fail(io::ErrorKind::Other), b"hello").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn lines_gain_exactly_one_newline_and_everything_flushes() {
        #[derive(Default)]
        struct Record {
            bytes: Vec<u8>,
            flushes: usize,
        }

        impl Write for Record {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }

        let mut out = Record::default();
        print_to(&mut out, "hello").unwrap();
        assert_eq!(out.bytes, b"hello\n");
        assert_eq!(out.flushes, 1);

        let mut out = Record::default();
        write_to(&mut out, b"raw").unwrap();
        assert_eq!(out.bytes, b"raw");
        assert_eq!(out.flushes, 1);
    }

    /// A real EPIPE through a real closed socket, not a scripted error
    /// kind: the write end's peer is dropped, the payload (1 MiB) dwarfs
    /// any socket buffer, and SIGPIPE stays ignored the way the Rust
    /// runtime leaves it, so the kernel must answer EPIPE.
    #[test]
    fn a_closed_peer_maps_to_ok() {
        let (reader, writer) = UnixStream::pair().unwrap();
        drop(reader);
        assert!(write_to(writer, &vec![b'x'; 1 << 20]).is_ok());
    }
}
