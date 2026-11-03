//! The compositor going away under a running bar, on a headless sway (the
//! only compositor here that can add and remove outputs at runtime): the
//! bar exits with status 1 (so `Restart=on-failure` restarts it against the
//! new compositor), says why, does it promptly, and leaves no `exec` child
//! behind. `tests/bar.rs` has the plain case (one output, nothing in
//! flight); these are the two that differ: nothing to draw at all, and the
//! connection dying with output events and a module child in flight.
//!
//! Skipped without sway (see `common`); `SCOOTBAR_REQUIRE_SWAY` makes that a
//! failure.

mod common;

use std::time::{Duration, Instant};

use common::{Reaper, Session};

const BAR: &str = "#20c030";
/// How long a bar may take to notice a dead compositor and exit. The loop
/// wakes on the hang-up at once; this is slack for a loaded machine, far
/// below the harness's 20 s patience.
const PROMPT: Duration = Duration::from_secs(3);

fn exits_one_promptly(session: &Session, bar: &mut Reaper) {
    let start = Instant::now();
    let status = common::wait_exit(&mut bar.0);
    let took = start.elapsed();
    let stderr = session.bar_stderr();
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("lost the connection to the compositor"),
        "{stderr}"
    );
    assert!(took < PROMPT, "took {took:?} to exit: {stderr}");
}

#[test]
fn a_bar_with_no_outputs_exits_when_the_compositor_goes() {
    let Some(mut session) = Session::sway(&common::scootbar_bin(), "gone-zero", 0) else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    // Past its start-up, idle with no surface: nothing to draw, and the
    // connection is all the loop is waiting on.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
    session.kill_compositor();
    exits_one_promptly(&session, &mut bar);
}

/// Whether a process runs with exactly this command line.
#[cfg(feature = "exec")]
fn running(cmdline: &str) -> bool {
    std::fs::read_dir("/proc").unwrap().flatten().any(|entry| {
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            return false;
        };
        let text: Vec<u8> = raw.iter().map(|&b| if b == 0 { b' ' } else { b }).collect();
        String::from_utf8_lossy(&text).trim_end() == cmdline
    })
}

#[cfg(feature = "exec")]
#[test]
fn the_compositor_going_mid_storm_ends_the_bar_and_its_command() {
    let Some(mut session) = Session::sway(&common::scootbar_bin(), "gone-storm", 1) else {
        return;
    };
    let script = "while :; do echo 5; sleep 1; done # gone-storm";
    let file = session.runtime_dir().join("bar.toml");
    std::fs::write(
        &file,
        format!(
            "right = [\"out\"]\n[colors]\nbackground = \"{BAR}\"\n\
             [exec.out]\ncommand = [\"sh\", \"-c\", \"{script}\"]\n"
        ),
    )
    .unwrap();
    let mut bar = Reaper(session.bar(&["--config", file.to_str().unwrap()]));
    let cmdline = format!("sh -c {script}");
    session.wait_for(&mut bar.0, "the command started", |_| {
        running(&cmdline).then_some(())
    });
    // Outputs coming and going back to back, and the compositor killed
    // with their events still on the wire.
    for _ in 0..10 {
        session.swaymsg(&["create_output"]);
    }
    for name in session
        .sway_usable()
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>()
    {
        session.swaymsg(&["output", &name, "unplug"]);
    }
    session.kill_compositor();
    exits_one_promptly(&session, &mut bar);
    let deadline = Instant::now() + Duration::from_secs(5);
    while running(&cmdline) {
        assert!(
            Instant::now() < deadline,
            "the command outlived the bar: it runs on, reparented"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
