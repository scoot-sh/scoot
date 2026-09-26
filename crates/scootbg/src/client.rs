//! The client half: one request to the running daemon, one reply.

use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::paths::{self, PathError};
use crate::protocol::Request;

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
    /// The daemon answered with an error.
    Daemon(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paths(error) => write!(f, "{error}"),
            Self::NotRunning { display, socket } => write!(
                f,
                "no scootbg daemon is running for {display} (nothing listens on {}); \
                 start one with `scootbg daemon`",
                socket.display()
            ),
            Self::Io { socket, error } => write!(f, "{}: {error}", socket.display()),
            Self::NoReply => write!(f, "the daemon closed the connection without replying"),
            Self::BadReply(why) => write!(f, "unreadable reply from the daemon: {why}"),
            Self::Daemon(message) => write!(f, "daemon: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Sends `request` and returns the reply line as the daemon sent it
/// (newline included). An error reply is an `Err`. For `kill`, returns only
/// once the daemon has closed the connection, which it does after removing
/// its socket and releasing its lock.
pub fn send(request: Request) -> Result<String, Error> {
    let paths = paths::from_env().map_err(Error::Paths)?;
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
                display: paths.display,
                socket: paths.socket,
            });
        }
        Err(error) => return Err(io_error(error)),
    };
    match exchange(&mut stream, request) {
        // The daemon closed without answering, and its socket is gone: it
        // was already stopping (another `kill`) when this request
        // arrived. For `kill`, that is the outcome asked for.
        Err(Error::NoReply) if request == Request::Kill && !paths.socket.exists() => {
            Ok(String::new())
        }
        Err(Error::Io { error, .. }) => Err(io_error(error)),
        other => other,
    }
}

/// One request and its reply over a connected stream. A connection the
/// daemon closes early (EOF, EPIPE, ECONNRESET) is [`Error::NoReply`].
fn exchange(stream: &mut UnixStream, request: Request) -> Result<String, Error> {
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
    if request == Request::Kill {
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
