//! The `/proc` parent walk: the parse against hostile `comm` fields and
//! truncated heads, and the walk against real processes.

use std::process::{Command, Stdio};

use super::{MAX_ANCESTRY_DEPTH, descends_from, parse_ppid};

#[test]
fn the_parent_pid_is_read_after_the_last_parenthesis() {
    assert_eq!(parse_ppid(b"123 (bash) S 45 123 123 0 -1"), Some(45));
    // A process may name itself anything: spaces, parentheses, even a fake
    // run of fields. The last `)` on the line is still `comm`'s own.
    assert_eq!(parse_ppid(b"123 (a b) S 45 1 1"), Some(45));
    assert_eq!(parse_ppid(b"123 (x) S 1 ) S 45 7 7"), Some(45));
    assert_eq!(parse_ppid(b"123 ()) S 45 7"), Some(45));
    assert_eq!(parse_ppid(b"1 (init) S 0 1 1"), Some(0));
}

#[test]
fn a_malformed_or_cut_short_head_is_no_answer() {
    for stat in [
        &b""[..],
        b"123 bash S 45 1",
        b"123 (bash)",
        b"123 (bash) S",
        b"123 (bash) S ",
        // Cut mid-number: "4" is not the parent, "45…" might be.
        b"123 (bash) S 4",
        b"123 (bash) S 45",
        b"123 (bash) SS 45 1",
        b"123 (bash)  S 45 1",
        b"123 (bash) S -1 1",
        b"123 (bash) S 4x5 1",
        b"123 (bash) S 99999999999 1",
    ] {
        assert_eq!(
            parse_ppid(stat),
            None,
            "{:?} parsed",
            String::from_utf8_lossy(stat)
        );
    }
}

#[test]
fn a_process_descends_from_itself_and_its_ancestors() {
    let me = std::process::id();
    let parent = std::os::unix::process::parent_id();
    assert!(descends_from(me, me));
    assert!(descends_from(me, parent));
    assert!(!descends_from(parent, me));
}

#[test]
fn a_child_descends_from_this_process_and_not_the_reverse() {
    let mut child = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sleep starts");
    let pid = child.id();
    let me = std::process::id();
    let outcome = (descends_from(pid, me), descends_from(me, pid));
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(outcome, (true, false));
}

#[test]
fn a_missing_process_descends_from_nothing_but_itself() {
    // Past the kernel's pid ceiling (2^22), so never a live process.
    let gone = u32::MAX - 1;
    assert!(!descends_from(gone, 1));
    assert!(!descends_from(gone, std::process::id()));
    // Identity needs no `/proc` read at all.
    assert!(descends_from(gone, gone));
}

#[test]
fn the_walk_stops_at_its_depth() {
    // A chain deeper than the bound: `sh` nesting `sh` past it, the
    // innermost a `sleep` this test can find by its parent.
    let depth = MAX_ANCESTRY_DEPTH + 2;
    let mut script = "exec sleep 30".to_owned();
    for _ in 0..depth {
        script = format!("sh -c '{}'; :", script.replace('\'', "'\\''"));
    }
    let mut outer = Command::new("sh")
        .args(["-c", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sh starts");
    let root = outer.id();
    // Walk down to the innermost process by parent pid.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut chain = vec![root];
    while chain.len() <= depth {
        let last = *chain.last().expect("non-empty");
        if let Some(child) = child_of(last) {
            chain.push(child);
        } else {
            assert!(
                std::time::Instant::now() < deadline,
                "the nested chain never reached {depth} levels: {chain:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let leaf = *chain.last().expect("non-empty");
    let within = chain[chain.len() - 1 - MAX_ANCESTRY_DEPTH];
    let beyond = chain[chain.len() - 2 - MAX_ANCESTRY_DEPTH];
    let outcome = (descends_from(leaf, within), descends_from(leaf, beyond));
    for pid in chain.iter().rev() {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let _ = outer.wait();
    assert_eq!(
        outcome,
        (true, false),
        "the walk did not stop at {MAX_ANCESTRY_DEPTH} links"
    );
}

/// A child of `pid`, found by scanning `/proc` for a process naming it as
/// its parent (`/proc/<pid>/task/<pid>/children` needs a kernel option).
fn child_of(pid: u32) -> Option<u32> {
    std::fs::read_dir("/proc").ok()?.find_map(|entry| {
        let candidate: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
        let stat = std::fs::read(format!("/proc/{candidate}/stat")).ok()?;
        (parse_ppid(&stat) == Some(pid)).then_some(candidate)
    })
}
