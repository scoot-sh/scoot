//! The client half: one request to the running daemon, one reply.
//!
//! (As scootbg's `client.rs`, for the bar's socket and protocol.)

use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use super::paths::{self, PathError, Paths};
use super::protocol::{DROPPED, Request};

/// How long to wait on a daemon that accepted the connection but does not
/// answer, before giving up rather than hanging a script.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The longest reply read. Replies are small; this only bounds a daemon
/// gone wrong.
const MAX_REPLY: u64 = 16 << 20;

#[derive(Debug)]
pub enum Error {
    Paths(PathError),
    /// Nothing listens on the socket.
    NotRunning {
        display: String,
        socket: PathBuf,
    },
    Io {
        socket: PathBuf,
        error: io::Error,
    },
    /// The daemon closed the connection without a reply.
    NoReply,
    BadReply(String),
    /// A streamed connection ended inside a line, so that line is not whole
    /// and was not passed on (the daemon drops a subscriber that does not
    /// read fast enough, which can cut the batch it was writing).
    Cut,
    /// The daemon said it was dropping this subscription (`{"type":"dropped"}`,
    /// the last line): events may have been missed.
    Dropped,
    /// The daemon answered with an error.
    Daemon(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paths(error) => write!(f, "{error}"),
            Self::NotRunning { display, socket } => write!(
                f,
                "no scootbar daemon is running for {display} (nothing listens on {}); \
                 start one with `scootbar daemon`",
                socket.display()
            ),
            Self::Io { socket, error } => write!(f, "{}: {error}", socket.display()),
            Self::NoReply => write!(f, "the daemon closed the connection without replying"),
            Self::BadReply(why) => write!(f, "unreadable reply from the daemon: {why}"),
            Self::Cut => write!(
                f,
                "the connection ended in the middle of a line, which was discarded \
                 (the daemon drops a subscriber that does not read fast enough): \
                 events may have been missed: subscribe again, then query"
            ),
            Self::Dropped => write!(
                f,
                "the daemon dropped this subscription (to admit another client, or \
                 because it was not read fast enough): events may have been missed: \
                 subscribe again, then query"
            ),
            Self::Daemon(message) => write!(f, "daemon: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Sends `request` and returns the reply line as the daemon sent it
/// (newline included). An error reply is an `Err`. For `kill`, returns only
/// once the daemon has closed the connection, which it does after removing
/// its socket and releasing its lock.
pub fn send(request: &Request<'_>) -> Result<String, Error> {
    let paths = paths::from_env().map_err(Error::Paths)?;
    send_to(&paths, request)
}

/// Sends `request` to the daemon at `paths`. Split out so tests can point
/// at a scratch socket without touching the process environment.
pub fn send_to(paths: &Paths, request: &Request<'_>) -> Result<String, Error> {
    let io_error = |error| Error::Io {
        socket: paths.socket.clone(),
        error,
    };
    let mut stream = match UnixStream::connect(&paths.socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(Error::NotRunning {
                display: paths.display.clone(),
                socket: paths.socket.clone(),
            });
        }
        Err(error) => return Err(io_error(error)),
    };
    match exchange(&mut stream, request) {
        // The daemon closed without answering, and its socket is gone: it
        // was already stopping (another `kill`) when this request
        // arrived. For `kill`, that is the outcome asked for.
        Err(Error::NoReply) if matches!(request, Request::Kill) && !paths.socket.exists() => {
            Ok(String::new())
        }
        Err(Error::Io { error, .. }) => Err(io_error(error)),
        other => other,
    }
}

/// One request and its reply over a connected stream. A connection the
/// daemon closes early (EOF, EPIPE, ECONNRESET) is [`Error::NoReply`].
fn exchange(stream: &mut UnixStream, request: &Request<'_>) -> Result<String, Error> {
    let io_error = |error: io::Error| {
        if matches!(
            error.kind(),
            io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::UnexpectedEof
        ) {
            Error::NoReply
        } else {
            Error::Io {
                socket: PathBuf::new(),
                error,
            }
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).map_err(io_error)?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io_error)?;
    stream
        .write_all(request.line().as_bytes())
        .map_err(io_error)?;

    let mut reader = BufReader::new((&*stream).take(MAX_REPLY));
    let mut line = String::new();
    if reader.read_line(&mut line).map_err(io_error)? == 0 {
        return Err(Error::NoReply);
    }
    let reply: serde_json::Value =
        serde_json::from_str(&line).map_err(|e| Error::BadReply(e.to_string()))?;
    if reply["type"] == "error" {
        let message = reply["message"].as_str().unwrap_or("(no message)");
        return Err(Error::Daemon(message.to_owned()));
    }
    if matches!(request, Request::Kill) {
        // Wait for the close: by then the socket file is gone and the
        // lock released, so a new daemon can start straight away. A reset
        // here is that same close.
        let mut rest = Vec::new();
        match reader.read_to_end(&mut rest) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    if !line.ends_with('\n') {
        line.push('\n');
    }
    Ok(line)
}

/// Sends `request` (a `subscribe`) and calls `each` with every line the daemon
/// sends, the `subscribed` reply first, until the daemon closes the
/// connection (`Ok`) or `each` returns `false`. An error reply to the
/// request is an `Err`. There is no read timeout: events come when they
/// come. `each` is only ever given whole lines: a connection that ends
/// inside one is [`Error::Cut`], and what it had sent of that line is dropped.
/// A `{"type":"dropped"}` line is given to `each` like any other and then
/// ends the stream as [`Error::Dropped`]. **`Ok` does not mean the daemon
/// went away**: a daemon that drops a subscriber it cannot write to (a full
/// socket) closes it with nothing more, which is a clean end too.
pub fn stream(request: &Request<'_>, each: impl FnMut(&str) -> bool) -> Result<(), Error> {
    let paths = paths::from_env().map_err(Error::Paths)?;
    stream_to(&paths, request, each)
}

/// [`stream`] to the daemon at `paths` (split out so tests can point at a
/// scratch socket without touching the process environment).
pub fn stream_to(
    paths: &Paths,
    request: &Request<'_>,
    mut each: impl FnMut(&str) -> bool,
) -> Result<(), Error> {
    let mut stream = match UnixStream::connect(&paths.socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(Error::NotRunning {
                display: paths.display.clone(),
                socket: paths.socket.clone(),
            });
        }
        Err(error) => {
            return Err(Error::Io {
                socket: paths.socket.clone(),
                error,
            });
        }
    };
    let io_error = |error| Error::Io {
        socket: paths.socket.clone(),
        error,
    };
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io_error)?;
    stream
        .write_all(request.line().as_bytes())
        .map_err(io_error)?;
    let mut reader = BufReader::new(&stream);
    let mut first = true;
    loop {
        let mut line = String::new();
        // One line, bounded: a daemon gone wrong cannot fill memory.
        match (&mut reader).take(MAX_REPLY).read_line(&mut line) {
            Ok(0) => {
                return if first { Err(Error::NoReply) } else { Ok(()) };
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof
                ) =>
            {
                // A reset after part of a line is a cut line, not an end.
                return if !line.is_empty() {
                    Err(Error::Cut)
                } else if first {
                    Err(Error::NoReply)
                } else {
                    Ok(())
                };
            }
            Err(error) => return Err(io_error(error)),
        }
        // Whole lines only. One that did not end in a newline is either cut
        // by the connection ending (the daemon drops a subscriber it cannot
        // write a whole batch to, which can leave a prefix of one) or past
        // the bound; printing either as if whole, then exiting 0, would hand
        // a script half an event.
        if !line.ends_with('\n') {
            return Err(if line.len() as u64 >= MAX_REPLY {
                Error::BadReply(format!("a line longer than {MAX_REPLY} bytes"))
            } else {
                Error::Cut
            });
        }
        if first {
            first = false;
            let reply: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| Error::BadReply(e.to_string()))?;
            if reply["type"] == "error" {
                let message = reply["message"].as_str().unwrap_or("(no message)");
                return Err(Error::Daemon(message.to_owned()));
            }
        }
        let dropped = line.as_bytes() == DROPPED;
        if !each(&line) {
            return Ok(());
        }
        if dropped {
            // Passed on first, so a script reading the lines sees it too.
            return Err(Error::Dropped);
        }
    }
}
