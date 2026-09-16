//! Tests for the outbound queue: partial writes, the order replies go out in,
//! and what the high-water mark actually measures.

use std::io::{self, Write};

use super::*;

/// A writer that accepts at most `room` bytes per call and then answers
/// `WouldBlock`, the way a socket does once the peer has stopped reading.
struct Throttled {
    /// What actually went out, in order.
    accepted: Vec<u8>,
    /// How much more this will take before blocking. Set to zero to stand in
    /// for a full socket.
    room: usize,
}

impl Throttled {
    fn with_room(room: usize) -> Self {
        Self {
            accepted: Vec::new(),
            room,
        }
    }
}

impl Write for Throttled {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.room == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(self.room);
        self.accepted.extend_from_slice(&bytes[..count]);
        self.room -= count;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_reply_the_socket_takes_whole_queues_nothing() {
    // The common case, and the one that must stay allocation-free beyond the
    // encoded line itself.
    let mut socket = Throttled::with_room(64);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "{\"type\":\"ok\"}\n".to_string())
        .expect("writes");
    assert!(outbound.is_empty());
    assert_eq!(socket.accepted, b"{\"type\":\"ok\"}\n");
}

#[test]
fn a_reply_that_does_not_fit_is_queued_and_finished_later() {
    let mut socket = Throttled::with_room(4);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "hello\n".to_string())
        .expect("writes what it can");
    assert_eq!(socket.accepted, b"hell");
    assert_eq!(outbound.pending(), 2, "the rest must be kept, not dropped");

    // Still nothing doing: a flush against a full socket is not an error and
    // must not lose anything either.
    outbound.flush(&mut socket).expect("blocks, not fails");
    assert_eq!(outbound.pending(), 2);

    socket.room = 64;
    outbound.flush(&mut socket).expect("finishes");
    assert!(outbound.is_empty());
    assert_eq!(socket.accepted, b"hello\n", "the reply must arrive intact");
}

#[test]
fn a_second_reply_queues_behind_the_first_rather_than_overtaking_it() {
    // The framing invariant. Two responses interleaved on the wire are not
    // late, they are undecodable -- and this is reachable whenever a reply
    // blocks partway and another request is already buffered.
    let mut socket = Throttled::with_room(2);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "aaaa\n".to_string())
        .expect("writes what it can");
    outbound
        .send(&mut socket, "bbbb\n".to_string())
        .expect("queues");
    assert_eq!(socket.accepted, b"aa");
    assert_eq!(outbound.pending(), 8);

    socket.room = 64;
    outbound.flush(&mut socket).expect("finishes");
    assert!(outbound.is_empty());
    assert_eq!(socket.accepted, b"aaaa\nbbbb\n");
}

#[test]
fn a_socket_that_takes_nothing_at_all_keeps_the_whole_reply() {
    let mut socket = Throttled::with_room(0);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "abcdef\n".to_string())
        .expect("queues it all");
    assert_eq!(outbound.pending(), 7);
    assert!(socket.accepted.is_empty());

    socket.room = 3;
    outbound.flush(&mut socket).expect("writes what it can");
    assert_eq!(outbound.pending(), 4);
    socket.room = 64;
    outbound.flush(&mut socket).expect("finishes");
    assert!(outbound.is_empty());
    assert_eq!(socket.accepted, b"abcdef\n");
}

#[test]
fn queueing_many_replies_does_not_carry_the_sent_prefix_along() {
    // A connection that queues reply after reply compacts as it goes, so the
    // buffer tracks what is *left*, not everything it has ever written.
    let mut socket = Throttled::with_room(3);
    let mut outbound = Outbound::default();
    for _ in 0..10 {
        outbound
            .send(&mut socket, "xxxx\n".to_string())
            .expect("queues");
        socket.room += 3;
    }
    // 50 bytes written in total, 3 taken up front plus 3 per later round.
    assert_eq!(socket.accepted.len(), 30);
    assert_eq!(outbound.pending(), 20);
    socket.room = 1024;
    outbound.flush(&mut socket).expect("finishes");
    assert_eq!(socket.accepted, "xxxx\n".repeat(10).as_bytes());
}

#[test]
fn a_write_that_accepts_zero_bytes_without_blocking_is_an_error() {
    // Not something a socket does -- but if it did, retrying forever on the
    // event-loop thread would be a hang, so it has to come back as an error.
    struct Stuck;
    impl Write for Stuck {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Ok(0)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut outbound = Outbound::default();
    let error = outbound
        .send(&mut Stuck, "a\n".to_string())
        .expect_err("must not loop");
    assert_eq!(error.kind(), io::ErrorKind::WriteZero);
}

#[test]
fn a_failing_write_is_reported_rather_than_swallowed() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut outbound = Outbound::default();
    assert_eq!(
        outbound
            .send(&mut Broken, "a\n".to_string())
            .expect_err("the peer is gone")
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn an_interrupted_write_is_retried_rather_than_treated_as_full() {
    // A signal landing mid-write is not the client's doing; losing the rest of
    // the reply over it would be a silent truncation.
    struct Interrupting {
        accepted: Vec<u8>,
        interrupts: usize,
    }
    impl Write for Interrupting {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.interrupts > 0 {
                self.interrupts -= 1;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            self.accepted.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut socket = Interrupting {
        accepted: Vec::new(),
        interrupts: 3,
    };
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "hello\n".to_string())
        .expect("writes");
    assert!(outbound.is_empty());
    assert_eq!(socket.accepted, b"hello\n");
}

#[test]
fn the_high_water_mark_is_about_unsent_bytes_not_the_buffers_length() {
    // The distinction matters for exactly the case the mark exists to allow: a
    // multi-megabyte screenshot reply that is nearly all out must not go on
    // back-pressuring the connection, and the buffer still holds every byte of
    // it either way.
    let oversized = HIGH_WATER_BYTES + 10;
    let mut socket = Throttled::with_room(0);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "x".repeat(oversized))
        .expect("queues it all");
    assert!(outbound.over_high_water());

    // Everything but ten bytes goes out; the buffer is still `oversized` long.
    socket.room = oversized - 10;
    outbound.flush(&mut socket).expect("writes what it can");
    assert_eq!(outbound.pending(), 10);
    assert!(
        !outbound.over_high_water(),
        "a nearly-drained reply must not hold the connection back"
    );
}

#[test]
fn the_mark_is_exclusive_so_exactly_that_many_bytes_is_not_over_it() {
    let mut socket = Throttled::with_room(0);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "x".repeat(HIGH_WATER_BYTES))
        .expect("queues");
    assert_eq!(outbound.pending(), HIGH_WATER_BYTES);
    assert!(!outbound.over_high_water());
    outbound.send(&mut socket, "x".to_string()).expect("queues");
    assert!(outbound.over_high_water());
}

#[test]
fn a_drained_queue_frees_its_buffer_rather_than_keeping_the_capacity() {
    // A screenshot's worth of capacity held per connection for the rest of its
    // life is the cost this avoids; the next blocked reply hands its own
    // allocation over instead.
    let mut socket = Throttled::with_room(0);
    let mut outbound = Outbound::default();
    outbound
        .send(&mut socket, "x".repeat(1 << 20))
        .expect("queues");
    socket.room = usize::MAX;
    outbound.flush(&mut socket).expect("finishes");
    assert!(outbound.is_empty());
    assert_eq!(
        outbound.capacity(),
        0,
        "a megabyte of capacity must not outlive the reply that needed it"
    );
}

#[test]
fn what_has_gone_out_is_counted_across_replies_and_partial_writes() {
    // The write-stall deadline in `connection.rs` asks this and nothing else:
    // has anything left since the last look? So it has to count bytes the
    // socket took, wherever they were written from -- a reply that went
    // straight out, the tail of one that did not, and the next reply queued
    // behind it -- and it must not go backwards when the queue empties and
    // frees its buffer.
    let mut socket = Throttled::with_room(4);
    let mut outbound = Outbound::default();
    assert_eq!(outbound.total_sent(), 0, "nothing has gone out yet");

    outbound
        .send(&mut socket, "hello\n".to_string())
        .expect("writes what it can");
    assert_eq!(outbound.total_sent(), 4, "four bytes of six went out");

    // Blocked: a flush that moves nothing must not count anything.
    outbound.flush(&mut socket).expect("blocks, not fails");
    assert_eq!(outbound.total_sent(), 4);

    // A second reply queues behind the first, and one more byte of room lets
    // exactly one more byte out.
    socket.room = 1;
    outbound
        .send(&mut socket, "world\n".to_string())
        .expect("queues behind the tail");
    assert_eq!(outbound.total_sent(), 5);

    // Everything else goes, the queue empties and frees its buffer -- and the
    // count is still the total, not what the emptied buffer remembers.
    socket.room = usize::MAX;
    outbound.flush(&mut socket).expect("finishes");
    assert!(outbound.is_empty());
    assert_eq!(outbound.total_sent(), 12, "both replies, whole");
    assert_eq!(socket.accepted, b"hello\nworld\n");
}
