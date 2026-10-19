//! The client half: one request to the running daemon, one reply.
//!
//! (As scootbar's `control/client.rs`, for the notifier's socket and
//! protocol.)

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
                "no scootnotify daemon is running for {display} (nothing listens on {}); \
                 start one with `scootnotify daemon`",
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
                "the daemon dropped this subscription: events may have been missed: \
                 subscribe again, then query"
            ),
            Self::Daemon(message) => write!(f, "daemon: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Sends `request` and returns the reply line as the daemon sent it
/// (newline included). An error reply is an `Err`.
pub fn send(request: &Request<'_>) -> Result<String, Error> {
    let paths = paths::from_env("scootnotify").map_err(Error::Paths)?;
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
    let paths = paths::from_env("scootnotify").map_err(Error::Paths)?;
    stream_to(&paths, request, each)
}

/// [`stream`] to the daemon at `paths` (split out so tests can point at a
/// scratch socket without touching the process environment).
pub fn stream_to(
    paths: &Paths,
    request: &Request<'_>,
    mut each: impl FnMut(&str) -> bool,
) -> Result<(), Error> {
    let mut subscription = subscribe_to(paths, request)?;
    loop {
        match subscription.read_line(None) {
            Ok(Some(line)) => {
                let dropped = line.as_bytes() == DROPPED;
                if !each(&line) {
                    return Ok(());
                }
                if dropped {
                    // Passed on first, so a script reading the lines sees it too.
                    return Err(Error::Dropped);
                }
            }
            // A clean end between lines (see `stream`): resubscribe and
            // query after any end.
            Ok(None) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

/// A subscribed connection: the `subscribed` reply already read, further
/// lines on demand. What `msg wait` holds while it watches for a match.
pub struct Subscription {
    reader: BufReader<UnixStream>,
    socket: PathBuf,
}

impl Subscription {
    /// Reads one line, waiting at most `timeout` (`None` waits forever).
    /// Returns `None` on a clean end between lines, or on a read timeout
    /// (the caller retries with what is left of its deadline). A cut line
    /// is [`Error::Cut`]; a `dropped` line is returned like any other, and
    /// the caller ends the wait as missed events.
    pub fn read_line(&mut self, timeout: Option<Duration>) -> Result<Option<String>, Error> {
        let io_error = |error| Error::Io {
            socket: self.socket.clone(),
            error,
        };
        // `None` blocks until a line comes: events arrive when they
        // arrive. `Some` bounds one attempt; a timeout is `Ok(None)`,
        // and the caller retries with what is left of its deadline.
        self.reader
            .get_mut()
            .set_read_timeout(timeout)
            .map_err(io_error)?;
        let mut line = String::new();
        // One line, bounded: a daemon gone wrong cannot fill memory.
        match (&mut self.reader).take(MAX_REPLY).read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Ok(None);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof
                ) =>
            {
                // A reset after part of a line is a cut line, not an end.
                return if !line.is_empty() {
                    Err(Error::Cut)
                } else {
                    Ok(None)
                };
            }
            Err(error) => return Err(io_error(error)),
        }
        // Whole lines only (see `stream`): a prefix is a cut, not an event.
        if !line.ends_with('\n') {
            return Err(if line.len() as u64 >= MAX_REPLY {
                Error::BadReply(format!("a line longer than {MAX_REPLY} bytes"))
            } else {
                Error::Cut
            });
        }
        Ok(Some(line))
    }
}

/// Subscribes at `paths`: connects, sends the `subscribe` request, and
/// reads its reply. An error reply is an `Err`, as is a daemon that
/// closes without answering.
pub fn subscribe_to(paths: &Paths, request: &Request<'_>) -> Result<Subscription, Error> {
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
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io_error)?;
    stream
        .write_all(request.line().as_bytes())
        .map_err(io_error)?;
    let mut subscription = Subscription {
        reader: BufReader::new(stream),
        socket: paths.socket.clone(),
    };
    // The reply, bounded: without it there is no subscription.
    match subscription.read_line(Some(TIMEOUT)) {
        Ok(Some(line)) => {
            let reply: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| Error::BadReply(e.to_string()))?;
            if reply["type"] == "error" {
                let message = reply["message"].as_str().unwrap_or("(no message)");
                return Err(Error::Daemon(message.to_owned()));
            }
            if reply["type"] != "subscribed" {
                return Err(Error::BadReply(format!(
                    "expected a `subscribed` reply: {line}"
                )));
            }
            Ok(subscription)
        }
        Ok(None) => Err(Error::NoReply),
        Err(error) => Err(error),
    }
}
