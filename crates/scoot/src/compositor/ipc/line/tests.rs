//! Tests for reading request lines off a socket that may hand over half of
//! one: the line cap, the end-of-stream cases, and `Incomplete`.

use std::collections::VecDeque;
use std::io::{Cursor, Read};

use scoot_ipc::{Request, decode, encode};

use super::*;

/// Reads with no per-wakeup read budget, for the tests that are about what a
/// line is rather than about how much one wakeup may take. The budget itself is
/// tested separately, at the bottom of this file.
fn unbounded<R: Read>(reader: &mut Lines<R>, limit: usize) -> LineRead {
    let mut reads = u32::MAX;
    reader.next(limit, &mut reads)
}

/// Reads with a deliberately tiny buffer, so every line of any length
/// crosses several `fill_buf`/`consume` rounds.
fn tiny_reader(bytes: &[u8]) -> Lines<Cursor<Vec<u8>>> {
    Lines::with_capacity(4, Cursor::new(bytes.to_vec()))
}

#[test]
fn reads_one_line_at_a_time_leaving_the_rest() {
    let mut reader = tiny_reader(b"{\"type\":\"version\"}\n{\"type\":\"windows\"}\n");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(reader.line(), b"{\"type\":\"version\"}\n");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(reader.line(), b"{\"type\":\"windows\"}\n");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Eof);
    assert!(reader.line().is_empty());
}

#[test]
fn a_last_line_without_a_newline_is_still_a_request() {
    // What `read_line` does (`Ok(n > 0)`), kept: a client that writes one
    // request and shuts its write half down still gets an answer. Note this
    // is a real end of stream, not a non-blocking "nothing yet" -- those are
    // distinct cases now, and confusing them would either lose that last
    // request or answer a half-written one.
    let mut reader = tiny_reader(b"{\"type\":\"version\"}");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(reader.line(), b"{\"type\":\"version\"}");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Eof);
}

#[test]
fn a_client_that_says_nothing_at_all_is_just_eof() {
    // Connect and disconnect without a byte: not an error, nothing to reply
    // to, close the connection.
    let mut reader = tiny_reader(b"");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Eof);
    assert!(reader.line().is_empty());
}

#[test]
fn an_empty_line_is_read_rather_than_mistaken_for_eof() {
    let mut reader = tiny_reader(b"\n\n");
    for _ in 0..2 {
        assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
        assert_eq!(reader.line(), b"\n");
    }
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Eof);
}

#[test]
fn the_limit_is_the_whole_line_newline_included() {
    // One under: 8 bytes plus the newline is exactly 9.
    let mut reader = tiny_reader(b"aaaaaaaa\n");
    assert_eq!(unbounded(&mut reader, 10), LineRead::Line);
    assert_eq!(reader.line().len(), 9);
    // Exactly at the limit: accepted.
    let mut reader = tiny_reader(b"aaaaaaaaa\n");
    assert_eq!(unbounded(&mut reader, 10), LineRead::Line);
    assert_eq!(reader.line().len(), 10);
    // One over: refused, and the buffer never grew past the limit.
    let mut reader = tiny_reader(b"aaaaaaaaaa\n");
    assert_eq!(unbounded(&mut reader, 10), LineRead::TooLong);
    assert!(
        reader.line().len() <= 10,
        "buffer grew to {}",
        reader.line().len()
    );
}

#[test]
fn the_limit_applies_to_each_line_not_to_the_connection() {
    // Three maximum-length requests in a row are three valid requests, not
    // one connection that has used up a budget.
    let mut reader = tiny_reader(b"aaaaa\naaaaa\naaaaa\n");
    for _ in 0..3 {
        assert_eq!(unbounded(&mut reader, 6), LineRead::Line);
        assert_eq!(reader.line(), b"aaaaa\n");
    }
}

#[test]
fn a_shorter_line_after_a_longer_one_leaves_no_leftovers() {
    // The buffer is reused across requests; if it were not cleared once per
    // line, the tail of the previous line would corrupt the next one's JSON.
    let mut reader = tiny_reader(b"aaaaaaaaaaaa\nbb\n");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(reader.line(), b"bb\n");
}

#[test]
fn an_endless_stream_with_no_newline_stops_at_the_limit() {
    // The finding the cap closed: before it, this read never returned and the
    // buffer grew until the machine gave out. `io::repeat` is an infinite
    // source with no newline in it, so this test hangs forever if the bound
    // is ever lost.
    let mut reader = Lines::new(std::io::repeat(b'x'));
    assert_eq!(unbounded(&mut reader, 4096), LineRead::TooLong);
    assert!(
        reader.line().len() <= 4096,
        "buffer grew to {}",
        reader.line().len()
    );
}

#[test]
fn a_failing_read_closes_rather_than_looping() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("gone"))
        }
    }
    let mut reader = Lines::new(Broken);
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Failed);
}

#[test]
fn the_default_limit_is_generous_enough_for_a_real_request() {
    // A `Request::Type` carrying a big paste has to keep working; the cap is
    // there for a client with no newline in sight, not for a large one.
    let text = "x".repeat(200_000);
    let encoded = encode(&Request::Type { text }).expect("encodes");
    assert!(encoded.len() < MAX_REQUEST_BYTES);
    let mut reader = tiny_reader(encoded.as_bytes());
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert!(matches!(
        decode::<Request>(std::str::from_utf8(reader.line()).expect("utf-8")),
        Ok(Request::Type { .. })
    ));
}

// --- partial lines on a non-blocking socket -------------------------------

/// A reader that hands out `chunks` one call at a time and answers
/// `WouldBlock` in between, the way a non-blocking socket does when a client
/// is still writing.
struct Trickle {
    chunks: VecDeque<Vec<u8>>,
    /// Whether the next `read` is the gap between two chunks rather than a
    /// chunk. Starts `false`: the first chunk is already there.
    blocked: bool,
    /// What to do once the chunks run out: `true` for a peer that closed,
    /// `false` for one that is simply quiet.
    eof_at_end: bool,
}

impl Trickle {
    fn new<const N: usize>(chunks: [&[u8]; N], eof_at_end: bool) -> Lines<Self> {
        // Capacity 4 so a chunk can also straddle the buffer underneath.
        Lines::with_capacity(
            4,
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
                blocked: false,
                eof_at_end,
            },
        )
    }
}

impl Read for Trickle {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.blocked {
            self.blocked = false;
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let Some(chunk) = self.chunks.front_mut() else {
            return if self.eof_at_end {
                Ok(0)
            } else {
                Err(std::io::ErrorKind::WouldBlock.into())
            };
        };
        let count = out.len().min(chunk.len());
        out[..count].copy_from_slice(&chunk[..count]);
        chunk.drain(..count);
        if chunk.is_empty() {
            self.chunks.pop_front();
            self.blocked = true;
        }
        Ok(count)
    }
}

#[test]
fn a_line_split_across_reads_keeps_what_arrived_so_far() {
    // The whole point of `Incomplete`: the prefix must survive into the next
    // call. Losing it would silently corrupt the request rather than fail.
    let mut reader = Trickle::new([b"{\"type\":", b"\"vers", b"ion\"}\n"], false);
    assert_eq!(
        unbounded(&mut reader, MAX_REQUEST_BYTES),
        LineRead::Incomplete
    );
    assert_eq!(
        unbounded(&mut reader, MAX_REQUEST_BYTES),
        LineRead::Incomplete
    );
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(reader.line(), b"{\"type\":\"version\"}\n");
    assert!(matches!(
        decode::<Request>(std::str::from_utf8(reader.line()).expect("utf-8")),
        Ok(Request::Version)
    ));
    // And a quiet peer after a whole line is `Incomplete`, not `Eof`: there
    // is nothing to close, it just has nothing more to say yet.
    assert_eq!(
        unbounded(&mut reader, MAX_REQUEST_BYTES),
        LineRead::Incomplete
    );
    assert!(
        reader.line().is_empty(),
        "the consumed line was not cleared"
    );
}

#[test]
fn an_incomplete_line_is_not_mistaken_for_the_end_of_one() {
    // A peer that goes quiet mid-line has not sent a request. Returning
    // `Line` here would answer half a request -- reliably, on every chunked
    // write any agent makes.
    let mut reader = Trickle::new([b"{\"type\":\"vers"], false);
    assert_eq!(
        unbounded(&mut reader, MAX_REQUEST_BYTES),
        LineRead::Incomplete
    );
    assert_eq!(
        unbounded(&mut reader, MAX_REQUEST_BYTES),
        LineRead::Incomplete
    );
    assert_eq!(reader.line(), b"{\"type\":\"vers");
}

#[test]
fn a_peer_that_closes_mid_line_still_gets_that_line_answered() {
    // `Eof` only when `read` really returns zero, and only with nothing
    // buffered -- a half-line plus a closed peer is the last request, same as
    // before this change.
    let mut reader = Trickle::new([b"{\"type\":\"ver", b"sion\"}"], true);
    assert_eq!(
        unbounded(&mut reader, MAX_REQUEST_BYTES),
        LineRead::Incomplete
    );
    assert_eq!(
        unbounded(&mut reader, MAX_REQUEST_BYTES),
        LineRead::Incomplete
    );
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(reader.line(), b"{\"type\":\"version\"}");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Eof);
}

#[test]
fn the_limit_holds_across_however_many_reads_a_line_takes() {
    // The cap is per line, not per read: a client dribbling bytes one wakeup
    // at a time must reach the same limit as one sending them all at once,
    // or preserving the prefix would have handed it a way around the cap.
    let mut reader = Trickle::new([b"aaaa", b"aaaa", b"aaaa"], false);
    assert_eq!(unbounded(&mut reader, 10), LineRead::Incomplete);
    assert_eq!(unbounded(&mut reader, 10), LineRead::Incomplete);
    assert_eq!(unbounded(&mut reader, 10), LineRead::TooLong);
    assert!(
        reader.line().len() <= 10,
        "buffer grew to {}",
        reader.line().len()
    );
}

#[test]
fn buffered_bytes_report_what_a_readiness_event_will_not_fire_for_again() {
    // `buffered()` is what lets the caller tell "nothing left" from "nothing
    // left *in the kernel*" -- stopping while this is non-empty strands those
    // bytes until unrelated traffic wakes the connection.
    let mut reader = Lines::new(Cursor::new(b"a\nb\nc\n".to_vec()));
    assert!(reader.buffered().is_empty(), "nothing read yet");
    assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    assert_eq!(reader.line(), b"a\n");
    assert_eq!(reader.buffered(), b"b\nc\n", "the rest came in one read");
    for _ in 0..2 {
        assert_eq!(unbounded(&mut reader, MAX_REQUEST_BYTES), LineRead::Line);
    }
    assert!(reader.buffered().is_empty());
}

// --- the per-wakeup read budget -------------------------------------------

#[test]
fn a_spent_read_budget_is_incomplete_with_the_buffer_left_alone() {
    // The fairness bound. With the budget spent, this must not go to the socket
    // again -- and must say so in a way the caller can yield on safely, which
    // means the read buffer has to be empty when it does (anything still in
    // there would be stranded: a level-triggered source does not report what
    // has already left the kernel).
    let mut reader = Lines::with_capacity(8, Cursor::new(b"aaa\nbbb\nccc\n".to_vec()));
    let mut reads = 1;
    // One read brings in 8 bytes: the first line, and most of the second.
    assert_eq!(reader.next(MAX_REQUEST_BYTES, &mut reads), LineRead::Line);
    assert_eq!(reader.line(), b"aaa\n");
    assert_eq!(reads, 0, "the one read was spent");
    assert_eq!(reader.buffered(), b"bbb\n", "and left the rest buffered");
    // The second line is already buffered, so it needs no read and is served.
    assert_eq!(reader.next(MAX_REQUEST_BYTES, &mut reads), LineRead::Line);
    assert_eq!(reader.line(), b"bbb\n");
    // The third needs one, and there is none left.
    assert!(reader.buffered().is_empty());
    assert_eq!(
        reader.next(MAX_REQUEST_BYTES, &mut reads),
        LineRead::Incomplete
    );
    // A fresh budget picks it up where it was left.
    reads = 1;
    assert_eq!(reader.next(MAX_REQUEST_BYTES, &mut reads), LineRead::Line);
    assert_eq!(reader.line(), b"ccc\n");
}

#[test]
fn a_line_spanning_a_chunk_boundary_needs_a_read_of_its_own() {
    // Why the budget is counted in reads and not in lines or bytes served: a
    // line that ends mid-chunk leaves a tail that can only be finished by
    // reading again, which is exactly how a caller looping on "something is
    // still buffered" ends up consuming a whole flood in one wakeup.
    let mut reader = Lines::with_capacity(4, Cursor::new(b"aaaaaa\n".to_vec()));
    let mut reads = 1;
    assert_eq!(
        reader.next(MAX_REQUEST_BYTES, &mut reads),
        LineRead::Incomplete
    );
    assert_eq!(reader.line(), b"aaaa", "the first chunk is kept");
    let mut reads = 1;
    assert_eq!(reader.next(MAX_REQUEST_BYTES, &mut reads), LineRead::Line);
    assert_eq!(reader.line(), b"aaaaaa\n");
}

#[test]
fn a_zero_budget_reads_nothing_at_all() {
    let mut reader = tiny_reader(b"aaa\n");
    let mut reads = 0;
    assert_eq!(
        reader.next(MAX_REQUEST_BYTES, &mut reads),
        LineRead::Incomplete
    );
    assert!(reader.line().is_empty(), "nothing may have been read");
    let mut reads = 4;
    assert_eq!(reader.next(MAX_REQUEST_BYTES, &mut reads), LineRead::Line);
}

#[test]
fn a_big_line_buffer_is_freed_rather_than_kept_for_the_connections_lifetime() {
    // Matches what `Outbound` does with a drained reply, and for the same
    // reason: one near-mebibyte request must not cost a megabyte per
    // connection for as long as that connection lives.
    let big = "x".repeat(64 * 1024);
    let mut reader = Lines::new(Cursor::new(format!("{big}\nsmall\n").into_bytes()));
    let mut reads = u32::MAX;
    assert_eq!(reader.next(MAX_REQUEST_BYTES, &mut reads), LineRead::Line);
    assert_eq!(reader.line().len(), big.len() + 1);
    assert_eq!(reader.next(MAX_REQUEST_BYTES, &mut reads), LineRead::Line);
    assert_eq!(reader.line(), b"small\n");
    assert!(
        reader.line_capacity() <= DEFAULT_CAPACITY,
        "a {}-byte line buffer outlived the line that needed it",
        reader.line_capacity()
    );
}
