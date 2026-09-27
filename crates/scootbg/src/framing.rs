//! Newline framing for the control socket, bounded.
//!
//! One [`LineBuffer`] per connection, reused for every line on it. Reads
//! append to it, complete lines are handed out as slices borrowed from it,
//! and consumed bytes are dropped by moving the unterminated remainder to
//! the front before the next read, so the buffer never grows past
//! `max + 1` bytes however much a client sends. A line longer than `max`
//! is reported as [`Line::TooLong`] as soon as `max + 1` bytes without a
//! newline have arrived, without waiting for the rest.
//!
//! Each byte is scanned for a newline once, however it is split across
//! reads, so a client trickling a long line a byte at a time costs linear
//! time, not quadratic.

use std::io::{self, Read};

#[cfg(test)]
mod tests;

#[derive(Debug, PartialEq, Eq)]
pub enum Line<'a> {
    /// One line, without its `\n`.
    Complete(&'a [u8]),
    /// More than `max` bytes arrived without a newline.
    TooLong,
}

#[derive(Debug)]
pub struct LineBuffer {
    buf: Vec<u8>,
    /// Where the next line starts; everything before it is consumed.
    start: usize,
    /// Bytes from `start` already known to hold no newline.
    scanned: usize,
    max: usize,
}

impl LineBuffer {
    /// An empty buffer for lines of at most `max` bytes. Allocates nothing
    /// until the first read.
    pub fn new(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            start: 0,
            scanned: 0,
            max,
        }
    }

    /// Reads once from `source` through `scratch`, appending what arrives.
    /// `Ok(0)` is the end of the stream. Call [`next_line`](Self::next_line)
    /// until it returns `None` before reading again: until then the buffer
    /// may already be full.
    pub fn read_from<R: Read>(&mut self, source: &mut R, scratch: &mut [u8]) -> io::Result<usize> {
        self.compact();
        let room = (self.max + 1).saturating_sub(self.buf.len());
        let Some(window) = scratch.get_mut(..room.min(scratch.len())) else {
            return Err(io::Error::other("line buffer scratch space"));
        };
        if window.is_empty() {
            // Only reachable if the caller read without draining lines
            // first: a full buffer has already been reported as TooLong.
            return Err(io::Error::other("line buffer is full"));
        }
        let n = source.read(window)?;
        // `n <= window.len()` by `Read`'s contract; `get` keeps a broken
        // reader from turning that into a panic.
        let Some(data) = window.get(..n) else {
            return Err(io::Error::other("reader returned more than it was given"));
        };
        // Grow by doubling, but never past `max + 1`: `extend` alone would
        // double 64 KiB to 128 KiB for a line near the limit, which is the
        // allocator's own-mapping threshold (an mmap/munmap per such
        // connection) and twice what the buffer can ever hold.
        let needed = self.buf.len() + data.len();
        if needed > self.buf.capacity() {
            let target = needed.max(self.buf.capacity() * 2).min(self.max + 1);
            self.buf.reserve_exact(target - self.buf.len());
        }
        self.buf.extend_from_slice(data);
        Ok(n)
    }

    /// The next complete line, if one has arrived.
    pub fn next_line(&mut self) -> Option<Line<'_>> {
        let pending = self.buf.get(self.start..)?;
        let unscanned = pending.get(self.scanned..)?;
        match unscanned.iter().position(|&b| b == b'\n') {
            Some(at) => {
                let len = self.scanned + at;
                let line_start = self.start;
                self.start += len + 1;
                self.scanned = 0;
                if len > self.max {
                    return Some(Line::TooLong);
                }
                self.buf
                    .get(line_start..line_start + len)
                    .map(Line::Complete)
            }
            None => {
                self.scanned = pending.len();
                (pending.len() > self.max).then_some(Line::TooLong)
            }
        }
    }

    /// At the end of the stream: the unterminated last line, if any bytes
    /// of one arrived. Consumes it.
    pub fn take_rest(&mut self) -> Option<Line<'_>> {
        let rest_start = self.start;
        let rest_len = self.buf.len().checked_sub(rest_start)?;
        if rest_len == 0 {
            return None;
        }
        self.start = self.buf.len();
        self.scanned = 0;
        if rest_len > self.max {
            return Some(Line::TooLong);
        }
        self.buf.get(rest_start..).map(Line::Complete)
    }

    /// Moves the unconsumed bytes to the front, keeping the allocation.
    fn compact(&mut self) {
        if self.start == 0 {
            return;
        }
        let start = self.start.min(self.buf.len());
        self.buf.copy_within(start.., 0);
        self.buf.truncate(self.buf.len() - start);
        self.start = 0;
    }
}
