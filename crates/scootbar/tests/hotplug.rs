//! Outputs coming and going, on a headless sway: the only compositor here
//! that can add and remove outputs at runtime (`swaymsg create_output`,
//! `output X unplug`), and the second layer-shell compositor scootbar is
//! checked on. What the bar did is read from sway: each output's active
//! workspace rectangle (the output less its exclusive zones) and
//! wlr-screencopy pixels.
//!
//! Skipped without sway (see `common`); `SCOOTBAR_REQUIRE_SWAY` makes that a
//! failure.

mod common;

use std::time::{Duration, Instant};

use common::{Reaper, Session, open_fds, rgb, shm_mappings, wakeups};
use serde_json::Value;

const BAR: &str = "#20c030";

/// Sway's output names, sorted.
fn names(session: &Session) -> Vec<String> {
    let mut names: Vec<String> = session
        .sway_usable()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names
}

/// Whether output `name` has the bar: its workspace starts 28 below its
/// top (the default height), and its top-left pixel is the bar's color.
fn has_bar(session: &Session, name: &str, usable: &Value) -> bool {
    usable["y"].as_i64() == Some(28) && session.screencopy(name).at(0, 0) == rgb(BAR)
}

/// Waits until every output sway has shows the bar, and returns their
/// names.
fn wait_all_bars(session: &Session, bar: &mut Reaper, what: &str) -> Vec<String> {
    session.wait_for(&mut bar.0, what, |session| {
        let outputs = session.sway_usable();
        outputs
            .iter()
            .all(|(name, usable)| has_bar(session, name, usable))
            .then(|| outputs.into_iter().map(|(name, _)| name).collect())
    })
}

/// A new headless output; returns its name.
fn plug(session: &Session) -> String {
    let before = names(session);
    session.swaymsg(&["create_output"]);
    let deadline = Instant::now() + common::PATIENCE;
    loop {
        if let Some(new) = names(session).into_iter().find(|n| !before.contains(n)) {
            return new;
        }
        assert!(
            Instant::now() < deadline,
            "sway never listed the new output"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn unplug(session: &Session, name: &str) {
    session.swaymsg(&["output", name, "unplug"]);
}

/// The daemon's fd count once steady: the same on four reads 100 ms apart.
fn settled_fds(pid: u32) -> usize {
    let deadline = Instant::now() + common::PATIENCE;
    let mut count = open_fds(pid);
    let mut stable = 0;
    while stable < 3 {
        std::thread::sleep(Duration::from_millis(100));
        let now = open_fds(pid);
        if now == count {
            stable += 1;
        } else {
            count = now;
            stable = 0;
        }
        assert!(Instant::now() < deadline, "the fd count never settled");
    }
    count
}

/// At most the double buffer per output is mapped, and at least one.
fn assert_buffers(pid: u32, outputs: usize) {
    let mapped = shm_mappings(pid);
    assert!(
        (outputs..=2 * outputs).contains(&mapped),
        "{mapped} shm buffers mapped for {outputs} outputs"
    );
}

#[test]
fn a_bar_comes_and_goes_with_its_output() {
    let Some(session) = Session::sway("plug", 1) else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    let first = wait_all_bars(&session, &mut bar, "a bar on the first output");
    assert_eq!(first.len(), 1);
    let pid = bar.0.id();
    let fds = settled_fds(pid);

    let second = plug(&session);
    let both = wait_all_bars(&session, &mut bar, "a bar on the plugged output");
    assert_eq!(both.len(), 2, "{both:?}");

    // The first one goes: the other keeps its bar, the daemon keeps going.
    unplug(&session, &first[0]);
    let left = wait_all_bars(&session, &mut bar, "the other bar kept");
    assert_eq!(left, std::slice::from_ref(&second));
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );

    // Down to none, then one again.
    unplug(&session, &second);
    session.wait_for(&mut bar.0, "no outputs", |session| {
        names(session).is_empty().then_some(())
    });
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
    plug(&session);
    wait_all_bars(&session, &mut bar, "a bar after zero outputs");
    // One output, as at the start: what the gone ones held is released.
    assert_eq!(settled_fds(pid), fds, "fds leaked across the replugs");
    assert_buffers(pid, 1);
    assert!(
        session.bar_stderr().is_empty(),
        "unexpected stderr: {}",
        session.bar_stderr()
    );
}

#[test]
fn zero_outputs_at_start_waits_without_waking() {
    let Some(session) = Session::sway("zero", 0) else {
        return;
    };
    assert!(names(&session).is_empty());
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    // Past its start-up (a connect and a round trip), then idle.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
    let pid = bar.0.id();
    let before = wakeups(pid);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(wakeups(pid), before, "woke up with no outputs");
    plug(&session);
    wait_all_bars(&session, &mut bar, "a bar on the first output plugged");
}

#[test]
fn a_hotplug_storm_leaves_one_bar_per_output_and_no_leak() {
    let Some(session) = Session::sway("storm", 1) else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR]));
    let first = wait_all_bars(&session, &mut bar, "a bar on the first output");
    let pid = bar.0.id();
    let fds = settled_fds(pid);
    // Outputs plugged and unplugged back to back, without waiting for the
    // bar: most are removed before their settle round trip comes back,
    // mid-configure or with a buffer just committed, which is the point.
    for _ in 0..10 {
        let name = plug(&session);
        unplug(&session, &name);
    }
    // And the first one out from under its bar, then back.
    unplug(&session, &first[0]);
    let last = plug(&session);
    let bars = wait_all_bars(&session, &mut bar, "one bar after the storm");
    assert_eq!(bars, [last]);
    assert_eq!(settled_fds(pid), fds, "fds leaked across the storm");
    assert_buffers(pid, 1);
    assert!(
        bar.0.try_wait().unwrap().is_none(),
        "{}",
        session.bar_stderr()
    );
}

#[test]
fn sway_reserves_the_margin_on_the_bars_edge_too() {
    let Some(session) = Session::sway("margin", 1) else {
        return;
    };
    let mut bar = Reaper(session.bar(&["--background", BAR, "--height", "20", "--margin", "8,4"]));
    // As on scoot (tests/bar.rs): the zone is the bar plus its edge's
    // margin, which the protocol says the compositor adds.
    // (The zone is reserved from the buffer-less first commit, so the
    // pixels are waited for as well.)
    let (usable, shot) = session.wait_for(&mut bar.0, "the zone reserved and drawn", |session| {
        let (name, usable) = session
            .sway_usable()
            .into_iter()
            .find(|(_, usable)| usable["y"].as_i64() == Some(28))?;
        let shot = session.screencopy(&name);
        (shot.at(4, 8) == rgb(BAR)).then_some((usable, shot))
    });
    assert_eq!(shot.at(4, 8), rgb(BAR), "the bar's corner");
    assert_ne!(shot.at(3, 8), rgb(BAR), "the left margin");
    assert_ne!(shot.at(4, 7), rgb(BAR), "the top margin");
    assert_ne!(shot.at(4, 28), rgb(BAR), "below the bar");
    assert_eq!(usable["height"].as_i64(), Some(i64::from(shot.height) - 28));
}

#[test]
fn side_margins_wider_than_the_output_still_give_a_bar() {
    let Some(session) = Session::sway("wide", 1) else {
        return;
    };
    // sway's headless output is 1280 wide: 2048 of side margins leave it
    // -768, which sway sends as the `uint` 4294966528. scootbar takes that
    // as "yours to choose" and draws the promised 1-pixel bar, rather than
    // refusing a multi-gigabyte buffer.
    let mut bar = Reaper(session.bar_with_env(
        &["--background", BAR, "--margin", "0,1024"],
        &[("WAYLAND_DEBUG", "1")],
    ));
    session.wait_for(&mut bar.0, "a 1-pixel bar drawn", |session| {
        let trace = session.bar_stderr();
        (trace.contains("configure, (") && common::created_buffers(&trace).last() == Some(&(1, 28)))
            .then_some(())
    });
    let trace = session.bar_stderr();
    assert!(
        trace.contains("4294966528"),
        "sway no longer sends the negative width; this test pins nothing: {trace}"
    );
    assert!(!trace.contains("cannot draw"), "{trace}");
    assert!(bar.0.try_wait().unwrap().is_none(), "{trace}");
    // It still reserves its height.
    let usable = session.sway_usable();
    assert_eq!(usable[0].1["y"].as_i64(), Some(28), "{usable:?}");
}
