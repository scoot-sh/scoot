//! What workspace-occupancy tracking costs on the window-churn path.
//!
//! The ticket's bound in numbers: every `apply()` ends in
//! `refresh_workspaces`, which now also re-reads the occupancy snapshots --
//! so the churn path (a window opening, closing or moving workspaces) pays
//! for the snapshot work whether or not anything subscribes, and a
//! subscriber pays for the send. Four arms, each timed as a whole and
//! reported per operation:
//!
//! - **steady, unsubscribed**: `apply()` with nothing changed -- the gate
//!   itself (one walk of the subscriber list, nothing built). Compare
//!   against `main` for the regression check.
//! - **steady, subscribed**: the same, with a `workspace` subscriber
//!   attached -- one snapshot build and compare per output per call.
//! - **churn, unsubscribed**: open a window and close it again per round
//!   (two `apply()`s, each re-marking) -- the gate plus the mark work,
//!   with nobody to send to.
//! - **churn, subscribed**: the same with a subscriber that reads -- mark,
//!   tick-flush and one small event per round, the production shape under
//!   a bar watching occupancy.
//!
//! The subscriber arms also run one size up (`BIG_WINDOWS`), so the
//! per-call walk shows its scaling rather than a single point. Run by
//! hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot workspace_snapshot_cost -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use scoot_core::{Event as CoreEvent, WindowId, WindowInfo};
use scoot_ipc::EventKind;

use super::{RUNS, RoundedFixture};

/// `apply()` calls per timed run in the steady arms.
const CALLS: u32 = 20_000;

/// Open/close rounds per timed run in the churn arms. Far fewer than
/// [`CALLS`]: each round is two full `apply()`s plus a tick-flush and an
/// event write in the subscribed arm.
const CHURN_ROUNDS: u32 = 2_000;

/// Windows in the big scene, so the per-call walk shows its scaling.
const BIG_WINDOWS: usize = 64;

/// A scene with `windows` core-only windows (see the module doc: no client
/// surfaces, so no texture cost -- what is timed is the arrangement and
/// the snapshot work on top of it, not pixman).
fn scene(windows: usize) -> RoundedFixture {
    // `rounded_scene` maps real clients; the occupancy path only reads the
    // core, so core-only windows time the same path without client cost.
    // Rebuilt here rather than reusing `scene()`, which fixes the count.
    let mut fixture = RoundedFixture::headless(
        crate::compositor::decorations::Appearance::default(),
        super::CANVAS,
    );
    for index in 0..windows as u64 {
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(index + 1),
            info: WindowInfo {
                app_id: "bench".to_string(),
                title: "bench".to_string(),
                hints: Default::default(),
                parent: None,
            },
            output: None,
            focus: true,
        });
    }
    fixture.state.apply();
    fixture
}

/// Attaches a `workspace` subscriber that reads: the production shape
/// under a bar watching occupancy. Hands back the client end, which the
/// caller drains after each flush so no backpressure ever builds.
fn live_subscriber(fixture: &mut RoundedFixture) -> UnixStream {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let response = fixture
        .state
        .subscribe(7, server, vec![EventKind::Workspace]);
    assert!(matches!(response, scoot_ipc::Response::Subscribed { .. }));
    client.set_nonblocking(true).expect("non-blocking");
    client
}

/// Empties whatever the subscriber was sent: what a reading bar does
/// every tick, so the bench never measures a stuffing socket.
fn drain(client: &UnixStream) {
    let mut client = client;
    let mut chunk = [0u8; 16 * 1024];
    while let Ok(count) = client.read(&mut chunk) {
        if count == 0 {
            break;
        }
    }
}

fn time_steady(fixture: &mut RoundedFixture, calls: u32) -> Duration {
    let started = Instant::now();
    for _ in 0..calls {
        fixture.state.apply();
    }
    started.elapsed()
}

/// One open plus one close per round, each with its own `apply()` -- the
/// shape a window storm has. With a subscriber, each round also flushes
/// and drains, the way the tick does.
fn time_churn(
    fixture: &mut RoundedFixture,
    rounds: u32,
    next_id: &mut u64,
    reader: Option<&UnixStream>,
) -> Duration {
    let started = Instant::now();
    for _ in 0..rounds {
        *next_id += 1;
        let id = WindowId(*next_id);
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id,
            info: WindowInfo::default(),
            output: None,
            focus: true,
        });
        fixture.state.apply();
        fixture
            .state
            .world
            .handle_event(CoreEvent::WindowClosed { id });
        fixture.state.apply();
        if let Some(client) = reader {
            fixture.state.flush_workspace_events();
            drain(client);
        }
    }
    started.elapsed()
}

#[test]
#[ignore = "prints per-call timings for a human; asserts nothing"]
fn workspace_snapshot_cost() {
    for windows in [8, BIG_WINDOWS] {
        // Steady, unsubscribed: the gate alone. The before/after number.
        let mut fixture = scene(windows);
        time_steady(&mut fixture, CALLS / 10);
        let mut runs: Vec<Duration> = (0..RUNS)
            .map(|_| time_steady(&mut fixture, CALLS))
            .collect();
        runs.sort();
        println!(
            "apply() steady, {windows} windows, no subscriber: median {:?} per call (min {:?}, max {:?}; {} runs x {CALLS})",
            runs[runs.len() / 2] / CALLS,
            runs[0] / CALLS,
            runs[runs.len() - 1] / CALLS,
            runs.len(),
        );

        // Steady, subscribed: build plus compare per call, nothing changed.
        let mut fixture = scene(windows);
        let _client = live_subscriber(&mut fixture);
        time_steady(&mut fixture, CALLS / 10);
        let mut runs: Vec<Duration> = (0..RUNS)
            .map(|_| time_steady(&mut fixture, CALLS))
            .collect();
        runs.sort();
        println!(
            "apply() steady, {windows} windows, workspace subscriber: median {:?} per call (min {:?}, max {:?}; {} runs x {CALLS})",
            runs[runs.len() / 2] / CALLS,
            runs[0] / CALLS,
            runs[runs.len() - 1] / CALLS,
            runs.len(),
        );

        // Churn, unsubscribed: mark work with nobody to send to.
        let mut fixture = scene(windows);
        let mut next_id = 10_000;
        time_churn(&mut fixture, CHURN_ROUNDS / 10, &mut next_id, None);
        let mut runs: Vec<Duration> = (0..RUNS)
            .map(|_| time_churn(&mut fixture, CHURN_ROUNDS, &mut next_id, None))
            .collect();
        runs.sort();
        println!(
            "open+close churn, {windows} windows, no subscriber: median {:?} per round (min {:?}, max {:?}; {} runs x {CHURN_ROUNDS})",
            runs[runs.len() / 2] / CHURN_ROUNDS,
            runs[0] / CHURN_ROUNDS,
            runs[runs.len() - 1] / CHURN_ROUNDS,
            runs.len(),
        );

        // Churn, subscribed with a reader: mark, flush, one event per
        // round -- the production shape under churn.
        let mut fixture = scene(windows);
        let client = live_subscriber(&mut fixture);
        let mut next_id = 10_000;
        time_churn(&mut fixture, CHURN_ROUNDS / 10, &mut next_id, Some(&client));
        let mut runs: Vec<Duration> = (0..RUNS)
            .map(|_| time_churn(&mut fixture, CHURN_ROUNDS, &mut next_id, Some(&client)))
            .collect();
        runs.sort();
        println!(
            "open+close churn, {windows} windows, workspace subscriber reading: median {:?} per round (min {:?}, max {:?}; {} runs x {CHURN_ROUNDS})",
            runs[runs.len() / 2] / CHURN_ROUNDS,
            runs[0] / CHURN_ROUNDS,
            runs[runs.len() - 1] / CHURN_ROUNDS,
            runs.len(),
        );
    }
}
