//! The history's tests: buckets, tolerance, atomicity.

use super::*;

fn load_text(lines: &[&str]) -> Frecency {
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    Frecency::from_bytes(&bytes)
}

#[test]
fn recency_buckets_decay_with_age() {
    let now = 1_800_000_000;
    let history = load_text(&[
        "day1\t1\t1799990000",
        "day10\t1\t1799136000",
        "day30\t1\t1797408000",
        "day80\t1\t1793088000",
        "day400\t1\t1700000000",
    ]);
    let (w1, w10, w30, w80, w400) = (
        history.weight("day1", now),
        history.weight("day10", now),
        history.weight("day30", now),
        history.weight("day80", now),
        history.weight("day400", now),
    );
    assert!(w1 > w10 && w10 > w30 && w30 > w80 && w80 > w400);
    assert_eq!((w1, w10, w30, w80, w400), (100, 70, 50, 30, 10));
}

#[test]
fn count_multiplies_the_bucket() {
    let now = 1_800_000_000;
    let history = load_text(&["a\t1\t1799990000", "b\t7\t1799990000"]);
    assert_eq!(history.weight("a", now), 100);
    assert_eq!(history.weight("b", now), 700);
    assert_eq!(history.weight("missing", now), 0);
}

#[test]
fn future_timestamps_count_as_most_recent() {
    let now = 1_800_000_000;
    let history = load_text(&["skewed\t3\t1900000000"]);
    assert_eq!(history.weight("skewed", now), 300);
}

#[test]
fn malformed_lines_are_skipped_and_first_id_wins() {
    let history = load_text(&[
        "good\t2\t1799990000",
        "no-tabs-here",
        "bad-count\tx\t1799990000",
        "bad-epoch\t2\tyesterday",
        "empty-id\t\t", // empty id
        "\t1\t1799990000",
        "good\t9\t1799990000", // duplicate: first wins
        "trailing-junk\t1\t1799990000\textra",
        "also-good\t1\t1799990000",
    ]);
    assert_eq!(
        history.get("good"),
        Some(Record {
            count: 2,
            last: 1_799_990_000
        })
    );
    assert_eq!(
        history.get("also-good"),
        Some(Record {
            count: 1,
            last: 1_799_990_000
        })
    );
    // A fourth tab poisons the epoch: skipped.
    assert_eq!(history.get("trailing-junk"), None);
    assert_eq!(history.len(), 2);
}

#[test]
fn non_utf8_and_huge_inputs_never_panic() {
    let hostile: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    let history = Frecency::from_bytes(&hostile);
    assert!(history.len() <= MAX_ENTRIES);
    // Past the read cap, the file is ignored wholesale.
    let big = vec![b'a'; READ_CAP + 1];
    assert_eq!(Frecency::from_bytes(&big).len(), 0);
    let empty = Frecency::from_bytes(b"");
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.weight("x", 0), 0);
}

#[test]
fn round_trip_keeps_records_and_caps_entries() {
    let mut history = Frecency::empty();
    let dir = tempfile::tempdir().unwrap();
    let now = 1_800_000_000;
    for i in 0..MAX_ENTRIES + 50 {
        assert!(history.record(Some(dir.path()), &format!("app-{i}"), now));
    }
    let reloaded = load(Some(dir.path()));
    assert_eq!(reloaded.len(), MAX_ENTRIES);
    let bytes = std::fs::read(history_path(Some(dir.path())).unwrap()).unwrap();
    assert!(bytes.len() <= MAX_FILE);
}

#[test]
fn record_bumps_count_and_recency() {
    let dir = tempfile::tempdir().unwrap();
    let mut history = load(Some(dir.path()));
    assert!(history.record(Some(dir.path()), "firefox", 1_000));
    assert!(history.record(Some(dir.path()), "firefox", 2_000));
    let reloaded = load(Some(dir.path()));
    assert_eq!(
        reloaded.get("firefox"),
        Some(Record {
            count: 2,
            last: 2_000
        })
    );
}

#[test]
fn hostile_ids_are_refused_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut history = load(Some(dir.path()));
    assert!(!history.record(Some(dir.path()), "", 1_000));
    assert!(!history.record(Some(dir.path()), "a\nb", 1_000));
    assert!(!history.record(Some(dir.path()), "a\rb", 1_000));
    assert!(!history.record(Some(dir.path()), &"x".repeat(MAX_ID + 1), 1_000));
    assert!(history_path(None).is_none());
    assert!(!history.record(None, "firefox", 1_000));
    assert_eq!(load(None).len(), 0);
}

#[test]
fn concurrent_writers_leave_one_whole_file() {
    use std::sync::{Arc, Barrier};
    let dir = tempfile::tempdir().unwrap();
    let dir = Arc::new(dir);
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for writer in 0..8 {
        let (dir, barrier) = (Arc::clone(&dir), Arc::clone(&barrier));
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for i in 0..50 {
                let mut history = load(Some(dir.path()));
                history.record(Some(dir.path()), &format!("w{writer}-{i}"), 1_000);
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    // Whatever won, the file parses: bounded, well-formed lines.
    let bytes = std::fs::read(history_path(Some(dir.path())).unwrap()).unwrap();
    assert!(bytes.len() <= MAX_FILE);
    let reloaded = Frecency::from_bytes(&bytes);
    assert!(reloaded.len() <= MAX_ENTRIES);
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let text = std::str::from_utf8(line).unwrap();
        assert_eq!(text.split('\t').count(), 3, "{text}");
    }
}
