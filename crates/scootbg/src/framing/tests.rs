use std::io::{self, Read};

use super::{Line, LineBuffer};

/// Hands out its data in fixed-size pieces, then EOF: a socket delivering
/// a request split across reads.
struct Trickle<'a> {
    data: &'a [u8],
    step: usize,
}

impl Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.step.min(buf.len()).min(self.data.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

/// Reads everything, collecting lines (as owned strings) as they complete,
/// then the unterminated rest.
fn collect(data: &[u8], step: usize, max: usize) -> Vec<Result<String, ()>> {
    let mut source = Trickle { data, step };
    let mut lines = LineBuffer::new(max);
    let mut scratch = [0u8; 4096];
    let mut out = Vec::new();
    loop {
        while let Some(line) = lines.next_line() {
            out.push(match line {
                Line::Complete(bytes) => Ok(String::from_utf8(bytes.to_vec()).unwrap()),
                Line::TooLong => Err(()),
            });
            if out.last() == Some(&Err(())) {
                return out;
            }
        }
        if lines.read_from(&mut source, &mut scratch).unwrap() == 0 {
            break;
        }
    }
    if let Some(line) = lines.take_rest() {
        out.push(match line {
            Line::Complete(bytes) => Ok(String::from_utf8(bytes.to_vec()).unwrap()),
            Line::TooLong => Err(()),
        });
    }
    out
}

fn ok(lines: &[&str]) -> Vec<Result<String, ()>> {
    lines.iter().map(|l| Ok((*l).to_owned())).collect()
}

#[test]
fn lines_come_out_whole_however_the_reads_split_them() {
    let data = b"first\nsecond line\n\nfourth\n";
    let expected = ok(&["first", "second line", "", "fourth"]);
    for step in [1, 2, 3, 5, 7, 64, 4096] {
        assert_eq!(collect(data, step, 64), expected, "step {step}");
    }
}

#[test]
fn an_unterminated_last_line_is_kept_for_the_end() {
    assert_eq!(collect(b"a\nb", 1, 64), ok(&["a", "b"]));
    assert_eq!(collect(b"only", 3, 64), ok(&["only"]));
    assert_eq!(collect(b"", 3, 64), ok(&[]));
}

#[test]
fn a_line_of_exactly_max_is_accepted() {
    let line = "x".repeat(10);
    let data = format!("{line}\n{line}");
    for step in [1, 4, 11, 100] {
        assert_eq!(collect(data.as_bytes(), step, 10), ok(&[&line, &line]));
    }
}

#[test]
fn a_line_over_max_is_refused_before_it_ends() {
    // 11 bytes, then more that would never fit: refused at byte 11, not
    // after reading everything.
    let mut data = vec![b'x'; 11];
    data.extend(std::iter::repeat_n(b'y', 1 << 20));
    for step in [1, 4, 11, 4096] {
        assert_eq!(collect(&data, step, 10), vec![Err(())], "step {step}");
    }
    // A complete line over max, arriving in one read, is refused too.
    assert_eq!(
        collect(b"ok\nxxxxxxxxxxxx\nnext\n", 4096, 10),
        vec![Ok("ok".to_owned()), Err(())]
    );
    // And an overlong unterminated tail at the end of the stream.
    let mut buffer = LineBuffer::new(4);
    let mut scratch = [0u8; 16];
    let mut source = Trickle {
        data: b"abcde",
        step: 5,
    };
    assert_eq!(buffer.read_from(&mut source, &mut scratch).unwrap(), 5);
    assert_eq!(buffer.next_line(), Some(Line::TooLong));
}

#[test]
fn the_buffer_never_grows_past_max_plus_one() {
    let mut buffer = LineBuffer::new(100);
    let mut scratch = [0u8; 4096];
    let data = "a line of request\n".repeat(10_000);
    let mut source = Trickle {
        data: data.as_bytes(),
        step: 4096,
    };
    let mut count = 0;
    loop {
        while let Some(line) = buffer.next_line() {
            assert_eq!(line, Line::Complete(b"a line of request"));
            count += 1;
        }
        assert!(buffer.buf.len() <= 101, "grew to {}", buffer.buf.len());
        if buffer.read_from(&mut source, &mut scratch).unwrap() == 0 {
            break;
        }
    }
    assert_eq!(count, 10_000);
    assert!(
        buffer.buf.capacity() <= 256,
        "capacity {}",
        buffer.buf.capacity()
    );
}

#[test]
fn reading_into_a_full_buffer_is_an_error_not_a_panic() {
    let mut buffer = LineBuffer::new(3);
    let mut scratch = [0u8; 8];
    let mut source = Trickle {
        data: b"abcdefgh",
        step: 8,
    };
    assert_eq!(buffer.read_from(&mut source, &mut scratch).unwrap(), 4);
    // Not drained: the next read has no room.
    assert!(buffer.read_from(&mut source, &mut scratch).is_err());
    // An empty scratch buffer is refused the same way.
    let mut empty = LineBuffer::new(3);
    assert!(empty.read_from(&mut source, &mut []).is_err());
}

#[test]
fn a_slow_client_costs_linear_scanning() {
    // One byte at a time up to the limit: each byte is looked at once.
    // Not a timing test; it checks the bookkeeping that makes it linear.
    let mut buffer = LineBuffer::new(1000);
    let mut scratch = [0u8; 1];
    let data = vec![b'z'; 999];
    let mut source = Trickle {
        data: &data,
        step: 1,
    };
    for i in 1..=999 {
        assert_eq!(buffer.read_from(&mut source, &mut scratch).unwrap(), 1);
        assert_eq!(buffer.next_line(), None);
        assert_eq!(buffer.scanned, i);
    }
}

/// A line at the limit never makes the buffer larger than `max + 1`, so
/// with the real 64 KiB limit it stays below the allocator's 128 KiB
/// own-mapping threshold.
#[test]
fn capacity_is_capped_at_max_plus_one() {
    let max = crate::protocol::MAX_REQUEST_LINE;
    let mut buffer = LineBuffer::new(max);
    let mut scratch = [0u8; 4096];
    let data = vec![b'x'; max + 10];
    let mut source = Trickle {
        data: &data,
        step: 4096,
    };
    loop {
        if buffer.next_line() == Some(Line::TooLong) {
            break;
        }
        assert!(buffer.read_from(&mut source, &mut scratch).unwrap() > 0);
    }
    assert!(
        buffer.buf.capacity() <= max + 1,
        "{}",
        buffer.buf.capacity()
    );
    assert!(buffer.buf.capacity() < scootbg_mem::alloc::THRESHOLD);
}
