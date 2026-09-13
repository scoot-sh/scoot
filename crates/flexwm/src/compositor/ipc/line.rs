//! Reading request lines off a non-blocking socket without letting a client
//! choose how much memory the compositor spends on it, and without ever
//! parking the event-loop thread.
//!
//! [`std::io::BufRead::read_line`] grows its `String` until it finds a `\n`
//! or the peer goes away, so a connected client that streams bytes and never
//! sends a newline can drive the compositor's memory wherever it likes. This
//! module is the same read, with a ceiling -- and with the other half of that
//! problem closed too: the socket is non-blocking, so a line that has only
//! half arrived yields [`LineRead::Incomplete`] and the event loop gets its
//! thread back, instead of the read sitting in `unix_stream_read_generic`
//! with every other client, wayland dispatch and input waiting behind it.
//!
//! The ceiling lives here and not in `flexwm-ipc`'s shared codec on purpose:
//! that codec also reads *responses*, and a `Response::Screenshot` is a
//! whole base64'd PNG -- legitimately megabytes, and legitimately unbounded
//! from the client's point of view since the compositor chose the size. A
//! `Request` has no such case; the largest real one is `Request::Type`'s
//! text.

use std::io::{self, BufRead, BufReader, Read};

#[cfg(test)]
mod tests;

/// The most bytes one request line may occupy, newline included.
///
/// Generous on purpose: the biggest legitimate request is `Request::Type`
/// with a pasted block of text, and JSON escaping can inflate that several
/// times over. Nothing a real client sends comes near a mebibyte, and a
/// client that does is not going to be helped by a larger number.
pub(super) const MAX_REQUEST_BYTES: usize = 1 << 20;

/// How a call to [`Lines::next`] ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LineRead {
    /// A whole line is in the buffer ([`Lines::line`]). The trailing `\n` is
    /// included when there was one; a final line the peer ended by closing
    /// its write half instead is also reported here, matching `read_line`'s
    /// own behavior (`Ok(n > 0)`), so a client that sends a request and shuts
    /// down its write half still gets an answer.
    Line,
    /// Nothing more has arrived yet, and what has is not a whole line. Not an
    /// error and not an end: the bytes read so far stay in the buffer and the
    /// next call picks the same line up where this one left off. The caller
    /// hands the thread back to the event loop, which wakes it again when the
    /// socket has more to read.
    Incomplete,
    /// The peer closed cleanly with nothing buffered: no more requests.
    Eof,
    /// No `\n` appeared within the limit, across however many reads it took
    /// to get there. The buffer holds whatever was accepted before the limit,
    /// which is not a request and never will be -- there is no way to
    /// resynchronize mid-line, so the caller closes the connection.
    TooLong,
    /// The read itself failed.
    Failed,
}

/// A socket (or any reader) being consumed one `\n`-terminated request line
/// at a time, with one line buffer reused for all of them.
///
/// Owns both halves of that job rather than leaving the buffer to the caller,
/// because the two are coupled in a way that is easy to get wrong: a line
/// split across several non-blocking reads must *keep* what has arrived so
/// far, so the buffer cannot simply be cleared at the top of every read --
/// it has to be cleared exactly once per line, after the caller has consumed
/// it. [`Lines::next`] is the only thing that touches either, so that pairing
/// cannot drift.
pub(super) struct Lines<R> {
    reader: BufReader<R>,
    /// The line being assembled: a whole one after [`LineRead::Line`], a
    /// prefix after [`LineRead::Incomplete`], empty otherwise.
    line: Vec<u8>,
    /// Whether `line` holds a line the caller has already been handed, i.e.
    /// whether the next call starts a new one.
    ///
    /// Not inferred from the buffer's contents, because neither candidate
    /// works: "ends in `\n`" misses the last line of a peer that closed
    /// without one (the next call would hand the same request back forever),
    /// and "is empty" misses an empty line, which is a real -- if invalid --
    /// request the caller has to answer.
    finished: bool,
}

impl<R: Read> Lines<R> {
    pub(super) fn new(reader: R) -> Self {
        Self::with_capacity(DEFAULT_CAPACITY, reader)
    }

    /// Sets the size of the read buffer between the socket and the line
    /// assembly above it. Visible for tests, which use a deliberately tiny
    /// one so every line crosses several `fill_buf`/`consume` rounds.
    pub(super) fn with_capacity(capacity: usize, reader: R) -> Self {
        Self {
            reader: BufReader::with_capacity(capacity, reader),
            line: Vec::new(),
            finished: false,
        }
    }

    /// The line the last [`LineRead::Line`] produced.
    pub(super) fn line(&self) -> &[u8] {
        &self.line
    }

    /// The reader underneath, for the caller that also needs to write to it.
    pub(super) fn socket(&self) -> &R {
        self.reader.get_ref()
    }

    /// Bytes already pulled out of the socket but not yet consumed as lines.
    ///
    /// The caller needs this to know whether it may stop reading: data sitting
    /// in *this* buffer has already left the kernel, so a level-triggered
    /// readiness source will not fire for it again. Stopping with a non-empty
    /// buffer strands whatever is in it until unrelated traffic happens to
    /// wake the connection.
    pub(super) fn buffered(&self) -> &[u8] {
        self.reader.buffer()
    }

    /// Reads the next line, refusing to grow the buffer past `limit` bytes.
    ///
    /// Bytes, not `String`: a line is only checked for UTF-8 once it is
    /// whole. Validating incrementally is not possible here anyway, since a
    /// multi-byte character can straddle two `fill_buf` chunks -- let alone
    /// two `read` calls seconds apart.
    pub(super) fn next(&mut self, limit: usize) -> LineRead {
        if self.finished {
            self.line.clear();
            self.finished = false;
        }
        loop {
            let available = match self.reader.fill_buf() {
                Ok(available) => available,
                // Same retry `read_line` itself does: a signal interrupting
                // the read is not the client's doing and is not an error.
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // The socket is non-blocking, so this is the ordinary "that
                // is all there is for now" answer, not a failure. Whatever
                // arrived stays in `self.line` for the next call.
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return LineRead::Incomplete;
                }
                Err(_) => return LineRead::Failed,
            };
            if available.is_empty() {
                // A real end of stream: `read` returned zero. Distinct from
                // `WouldBlock` above, so a half-written line whose client is
                // merely slow is never mistaken for one whose client is gone.
                return if self.line.is_empty() {
                    LineRead::Eof
                } else {
                    self.finish()
                };
            }

            // `position` over a byte slice, the same scan `read_until` does.
            // The chunk is whatever the BufReader holds, so this walks each
            // byte of the line exactly once across the whole connection.
            let (used, complete) = match available.iter().position(|byte| *byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (available.len(), false),
            };
            // Checked before extending, not after: the point is that the
            // allocation never happens, so a `line.len()` test after the fact
            // would be too late. `used <= isize::MAX` (it is the length of a
            // live slice) and `line.len() <= limit`, so this cannot wrap.
            //
            // `line` persists across `Incomplete` returns, so this is a limit
            // on the whole line however many reads it arrived in -- a client
            // dribbling one byte per wakeup reaches it just the same as one
            // sending a megabyte at once, it only takes longer.
            if self.line.len() + used > limit {
                return LineRead::TooLong;
            }
            self.line.extend_from_slice(&available[..used]);
            self.reader.consume(used);
            if complete {
                return self.finish();
            }
        }
    }

    /// Hands the assembled line to the caller, and remembers that the next
    /// call starts a new one.
    fn finish(&mut self) -> LineRead {
        self.finished = true;
        LineRead::Line
    }
}

/// What `BufReader::new` itself uses. Named here only so [`Lines::new`] and
/// [`Lines::with_capacity`] are visibly the same function with one argument
/// defaulted.
const DEFAULT_CAPACITY: usize = 8 * 1024;
