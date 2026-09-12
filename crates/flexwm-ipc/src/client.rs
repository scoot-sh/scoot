//! A blocking client: enough for scripts, agents and `flexwm msg`.

use std::io::{self, BufReader};
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::codec::{read_message_buffered, write_message};
use crate::request::Request;
use crate::response::Response;
use crate::socket::socket_path;

#[derive(Debug)]
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    /// Reused across requests instead of allocating a fresh String for every
    /// reply -- a long-lived Client (an agent driving a whole session) can
    /// make many requests, not just one.
    line: String,
}

impl Client {
    pub fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        UnixStream::connect(path).and_then(Self::from_stream)
    }

    /// Connects to the socket named by `FLEXWM_SOCKET`, or the default one in
    /// `XDG_RUNTIME_DIR`.
    pub fn connect_default() -> io::Result<Self> {
        let path = socket_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "neither FLEXWM_SOCKET nor XDG_RUNTIME_DIR is set",
            )
        })?;
        Self::connect(path)
    }

    pub fn from_stream(stream: UnixStream) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            line: String::new(),
        })
    }

    pub fn request(&mut self, request: &Request) -> io::Result<Response> {
        write_message(&mut self.writer, request)?;
        read_message_buffered(&mut self.reader, &mut self.line)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "server closed the connection")
        })
    }
}
