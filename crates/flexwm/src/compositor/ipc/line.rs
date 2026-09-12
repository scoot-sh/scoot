//! Reading one request line without letting a client choose how much memory
//! the compositor spends on it.
//!
//! [`std::io::BufRead::read_line`] grows its `String` until it finds a `\n`
//! or the peer goes away, so a connected client that streams bytes and never
//! sends a newline can drive the compositor's memory wherever it likes. This
//! module is the same read, with a ceiling.
//!
//! The ceiling lives here and not in `flexwm-ipc`'s shared codec on purpose:
//! that codec also reads *responses*, and a `Response::Screenshot` is a
//! whole base64'd PNG -- legitimately megabytes, and legitimately unbounded
//! from the client's point of view since the compositor chose the size. A
//! `Request` has no such case; the largest real one is `Request::Type`'s
//! text.

use std::io::{self, BufRead};

/// The most bytes one request line may occupy, newline included.
///
/// Generous on purpose: the biggest legitimate request is `Request::Type`
/// with a pasted block of text, and JSON escaping can inflate that several
/// times over. Nothing a real client sends comes near a mebibyte, and a
/// client that does is not going to be helped by a larger number.
pub(super) const MAX_REQUEST_BYTES: usize = 1 << 20;

/// How a call to [`read_line_bounded`] ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LineRead {
    /// A whole line is in the buffer. The trailing `\n` is included when
    /// there was one; a final line the peer ended by closing its side
    /// instead is also reported here, matching `read_line`'s own behavior
    /// (`Ok(n > 0)`), so a client that sends a request and shuts down its
    /// write half still gets an answer.
    Line,
    /// The peer closed cleanly with nothing buffered: no more requests.
    Eof,
    /// No `\n` appeared within [`MAX_REQUEST_BYTES`]. The buffer holds
    /// whatever was accepted before the limit, which is not a request and
    /// never will be -- there is no way to resynchronize mid-line, so the
    /// caller closes the connection.
    TooLong,
    /// The read itself failed.
    Failed,
}

/// Reads one `\n`-terminated line into `line`, refusing to grow it past
/// `limit` bytes.
///
/// `line` is cleared first and reused across calls, so a connection's buffer
/// capacity is paid for once rather than per request.
///
/// Bytes, not `String`: a line is only checked for UTF-8 once it is whole.
/// Validating incrementally is not possible here anyway, since a multi-byte
/// character can straddle two `fill_buf` chunks.
pub(super) fn read_line_bounded<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    limit: usize,
) -> LineRead {
    line.clear();
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            // Same retry `read_line` itself does: a signal interrupting the
            // read is not the client's doing and is not an error.
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return LineRead::Failed,
        };
        if available.is_empty() {
            return if line.is_empty() {
                LineRead::Eof
            } else {
                LineRead::Line
            };
        }

        // `position` over a byte slice, the same scan `read_until` does. The
        // chunk is whatever the BufReader holds, so this walks each byte of
        // the line exactly once across the whole call.
        let (used, complete) = match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => (index + 1, true),
            None => (available.len(), false),
        };
        // Checked before extending, not after: the point is that the
        // allocation never happens, so a `line.len()` test after the fact
        // would be too late. `used <= isize::MAX` (it is the length of a
        // live slice) and `line.len() <= limit`, so this cannot wrap.
        if line.len() + used > limit {
            return LineRead::TooLong;
        }
        line.extend_from_slice(&available[..used]);
        reader.consume(used);
        if complete {
            return LineRead::Line;
        }
    }
}
