//! Splitting a child's output into lines with a hard bound on memory.
//!
//! A line longer than [`MAX_LINE`] bytes is **dropped, not truncated**: its
//! first bytes are discarded as they arrive and so is everything up to the
//! newline that ends it, so a chatty or hostile child holds at most
//! `MAX_LINE` bytes here however much it writes (and a text that is cut
//! would show a half-sentence as if it were whole). The count of dropped
//! lines is kept for one warning.

use super::super::payload::MAX_PAYLOAD;

/// The longest line kept, in bytes: what an update line may be.
pub const MAX_LINE: usize = MAX_PAYLOAD;

#[derive(Debug)]
pub struct Lines {
    /// The line so far, up to [`MAX_LINE`] bytes. Allocated once.
    buf: Vec<u8>,
    /// The line being read is already too long: bytes are discarded until
    /// its newline.
    overlong: bool,
    /// Lines dropped since [`Lines::take_dropped`].
    dropped: u32,
}

impl Default for Lines {
    fn default() -> Self {
        Self {
            buf: Vec::with_capacity(MAX_LINE),
            overlong: false,
            dropped: 0,
        }
    }
}

impl Lines {
    /// Feeds `chunk`, calling `line` for each line it completes (without
    /// its newline). A partial line is kept for the next chunk.
    pub fn feed(&mut self, mut chunk: &[u8], mut line: impl FnMut(&[u8])) {
        while !chunk.is_empty() {
            let end = chunk.iter().position(|&b| b == b'\n');
            let (part, rest) = match end {
                Some(at) => (&chunk[..at], Some(&chunk[at + 1..])),
                None => (chunk, None),
            };
            if self.overlong {
                // Discarding until the newline.
            } else if self.buf.len() + part.len() > MAX_LINE {
                self.overlong = true;
                self.dropped = self.dropped.saturating_add(1);
                self.buf.clear();
            } else {
                self.buf.extend_from_slice(part);
            }
            match rest {
                Some(rest) => {
                    // A newline: the line is whole, or was dropped.
                    if self.overlong {
                        self.overlong = false;
                    } else {
                        line(&self.buf);
                    }
                    self.buf.clear();
                    chunk = rest;
                }
                None => break,
            }
        }
    }

    /// The last line, if the child ended without a newline after it. A
    /// line cut short by exit that was being dropped stays dropped.
    pub fn finish(&mut self, mut line: impl FnMut(&[u8])) {
        if !self.overlong && !self.buf.is_empty() {
            line(&self.buf);
        }
        self.buf.clear();
        self.overlong = false;
    }

    /// How many lines were dropped for length since the last call.
    pub fn take_dropped(&mut self) -> u32 {
        std::mem::take(&mut self.dropped)
    }

    /// The bytes held now, for the bound's test.
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.buf.len()
    }

    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }
}
