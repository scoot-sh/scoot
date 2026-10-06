//! Unit tests for the two decisions `--nested` makes about a host configure.
//!
//! [`Host`](super::Host) itself needs a live host compositor to construct, so
//! neither entry point (`apply_first_configure`, `apply_resize`) nor
//! `present`'s buffer handling can be driven from here -- the buffer pool's
//! own logic is tested in `buffers.rs`, where it needs no connection, and the
//! two entry points are pinned live by the `--nested` resize run recorded on
//! the PR. What *is* testable is what this module deliberately keeps as free
//! functions over plain values: which of the two entry points a configure
//! goes to, and which proposed sizes are acted on at all.

use super::{ConfigureAction, PendingResize, configure_action, usable_size};
use crate::cli::MAX_OUTPUT_DIMENSION;

const STARTED_AT: (i32, i32) = (1280, 800);

#[test]
fn the_first_configure_is_the_first_configure_even_at_the_starting_size() {
    // A host that proposes exactly what scoot asked for still has to build
    // the render target: nothing exists yet. Returning `Nothing` here would
    // leave the surface unconfigured forever, and xdg-shell forbids attaching
    // a buffer until it is -- a window that never shows anything.
    assert_eq!(
        configure_action(false, STARTED_AT, STARTED_AT),
        ConfigureAction::FirstConfigure
    );
}

#[test]
fn the_first_configure_at_a_different_size_is_still_the_first_configure() {
    assert_eq!(
        configure_action(false, (1920, 1080), STARTED_AT),
        ConfigureAction::FirstConfigure
    );
}

#[test]
fn a_later_configure_at_a_new_size_resizes() {
    // Issue #144: this is the case that used to return early.
    assert_eq!(
        configure_action(true, (1920, 1080), STARTED_AT),
        ConfigureAction::Resize
    );
}

#[test]
fn one_axis_moving_is_enough_to_resize() {
    assert_eq!(
        configure_action(true, (1280, 801), STARTED_AT),
        ConfigureAction::Resize
    );
    assert_eq!(
        configure_action(true, (1281, 800), STARTED_AT),
        ConfigureAction::Resize
    );
}

#[test]
fn a_later_configure_at_the_same_size_does_nothing() {
    // Load-bearing, not an optimisation: hosts re-send a configure on every
    // state change that is not a resize (activation, maximize, a tiling-edge
    // update), so without this every focus change in the host would throw
    // away a working render target and buffer pool to build an identical
    // pair, and redraw the whole frame into it.
    assert_eq!(
        configure_action(true, STARTED_AT, STARTED_AT),
        ConfigureAction::Nothing
    );
}

#[test]
fn an_ordinary_size_is_usable() {
    assert_eq!(usable_size(1280, 800), Some((1280, 800)));
    assert_eq!(usable_size(1, 1), Some((1, 1)));
    assert_eq!(
        usable_size(MAX_OUTPUT_DIMENSION, MAX_OUTPUT_DIMENSION),
        Some((MAX_OUTPUT_DIMENSION, MAX_OUTPUT_DIMENSION))
    );
}

#[test]
fn a_zero_axis_is_the_hosts_you_choose_and_is_not_usable() {
    // xdg-shell's way of saying "pick that dimension yourself". Dropping the
    // whole proposal keeps the size scoot is already at; the alternative
    // (half-applying it) is not what this has ever done.
    assert_eq!(usable_size(0, 0), None);
    assert_eq!(usable_size(0, 800), None);
    assert_eq!(usable_size(1280, 0), None);
}

#[test]
fn a_negative_axis_is_not_usable() {
    // Nothing a correct host sends, but the wire carries `int`s and a
    // `width as usize` on a negative one is how a buffer pool asks for
    // sixteen exabytes.
    assert_eq!(usable_size(-1, 800), None);
    assert_eq!(usable_size(1280, -1), None);
    assert_eq!(usable_size(i32::MIN, i32::MIN), None);
}

#[test]
fn an_axis_past_the_output_bound_is_not_usable() {
    // The same `1..=65535` `--width`/`--height` are parsed into: DRM reports
    // a mode axis in a `u16`, so a bigger one is not a mode any client could
    // believe. Refused rather than clamped, matching `cli::dimension`.
    assert_eq!(usable_size(MAX_OUTPUT_DIMENSION + 1, 800), None);
    assert_eq!(usable_size(1280, MAX_OUTPUT_DIMENSION + 1), None);
    assert_eq!(usable_size(i32::MAX, i32::MAX), None);
}

// -- the coalescing queue -------------------------------------------------

#[test]
fn an_empty_queue_drains_to_nothing() {
    let mut queued = PendingResize::default();
    assert_eq!(queued.take_if_changed(STARTED_AT), None);
}

#[test]
fn a_queued_resize_drains_once_at_its_size() {
    // The drain half of the coalescing: one queued configure becomes one
    // applied resize, and the queue is consumed whether or not it differed.
    let mut queued = PendingResize::default();
    queued.queue((1920, 1080));
    assert_eq!(queued.take_if_changed(STARTED_AT), Some((1920, 1080)));
    assert_eq!(queued.take_if_changed((1920, 1080)), None);
}

#[test]
fn queueing_overwrites_so_only_the_latest_size_is_ever_applied() {
    // The queue half: a drag's configure-per-pixel-step collapses to the
    // latest size, so a frame tick rebuilds once, not once per step.
    let mut queued = PendingResize::default();
    for width in [1281, 1400, 1600, 1900] {
        queued.queue((width, 800));
    }
    assert_eq!(queued.take_if_changed(STARTED_AT), Some((1900, 800)));
    assert_eq!(queued.take_if_changed((1900, 800)), None);
}

#[test]
fn a_queue_that_comes_back_to_the_current_size_drains_to_nothing() {
    // A drag that returns to where it started within one frame must not
    // rebuild anything: the size on screen is already the queued one.
    let mut queued = PendingResize::default();
    queued.queue((1900, 800));
    queued.queue(STARTED_AT);
    assert_eq!(queued.take_if_changed(STARTED_AT), None);
}

// -------------------------------------------------------------------------
// Host loss (`HostSource`)
// -------------------------------------------------------------------------

use std::io::Read as _;
use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wayland_client::Connection;

use super::{HostLoss, STARTUP_TIMEOUT, init_on};
use crate::compositor::decorations::Appearance;
use crate::compositor::state::ClientState;
use crate::compositor::test_support::{Harness, capture_logs};

/// How long the test waits for the host thread at each step. Generous: a
/// loaded CI box dispatches slowly, and only a true wedge may fail it.
const HOST_PATIENCE: Duration = Duration::from_secs(10);

/// The host: a headless scoot on its own thread, dispatching until told to
/// die -- the in-process shape of a parent compositor going away mid-session
/// (see `docs/backlog/resolved/nested-ipc-socket-refuses-done.md`). Dying means
/// dropping the whole harness, which closes the server end of the socket
/// pair the nested session is connected through.
fn host(
    server_end: UnixStream,
    ready: Sender<()>,
    die: Receiver<()>,
    died: Sender<()>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut host: Harness<(), ()> = Harness::headless(Appearance::default(), 64);
        host.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("the nested session connects");
        // The nested side blocks in a registry round trip inside `init_on`,
        // so this has to arrive before it can proceed.
        ready.send(()).expect("the test waits for the host");
        loop {
            if die.try_recv().is_ok() {
                break;
            }
            host.settle();
        }
        drop(host);
        died.send(()).expect("the test waits for the death");
    })
}

/// Losing the host stops the session without failing the loop.
///
/// Before `HostSource`, the dead host connection surfaced as an event-loop
/// error (`other error during loop operation: ...Broken pipe`), which killed
/// `EventLoop::run` and with it the session -- while the control socket's
/// file stayed behind with no listener, so every later `scoot msg` answered
/// `Connection refused` against a compositor that was no longer there. Now
/// the loss is named in the log, the loop stops cleanly, and `State`
/// carries the loss for `compositor::run` to report honestly.
#[test]
fn losing_the_host_stops_the_session_without_failing_the_loop() {
    let (_, logs) = capture_logs(|| {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        let (ready_tx, ready_rx) = channel();
        let (die_tx, die_rx) = channel();
        let (died_tx, died_rx) = channel();
        let thread = host(server_end, ready_tx, die_rx, died_tx);
        ready_rx
            .recv_timeout(HOST_PATIENCE)
            .expect("the host comes up");

        let mut nested: Harness<(), ()> = Harness::headless(Appearance::default(), 64);
        let conn = Connection::from_socket(client_end).expect("a host connection");
        let handle = nested.event_loop.handle();
        init_on(handle, &mut nested.state, conn, 64, 64).expect("the nested backend comes up");
        assert!(
            !nested
                .state
                .host_loss
                .as_ref()
                .expect("a nested session names its loss flag")
                .lost(),
            "nothing is lost while the host is alive"
        );

        // The parent goes away. Queuing a frame first makes sure there is
        // host-bound traffic in flight, so the loss surfaces through the
        // flush path too rather than only through the read end.
        die_tx.send(()).expect("the host dies on request");
        died_rx
            .recv_timeout(HOST_PATIENCE)
            .expect("the host stays dead");
        thread.join().expect("the host thread ends");
        nested.state.request_render();
        nested.state.render();

        // Every dispatch stays `Ok`: the loss is mapped, never loop-fatal.
        // (Before the mapping, the first dispatch surfacing the dead
        // connection returned `Err`, which is what this asserts against.)
        let deadline = Instant::now() + HOST_PATIENCE;
        loop {
            let dispatched = nested
                .event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut nested.state);
            assert!(
                dispatched.is_ok(),
                "a dead host must not fail the loop: {dispatched:?}"
            );
            if nested
                .state
                .host_loss
                .as_ref()
                .expect("a nested session names its loss flag")
                .lost()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the loss never surfaced within {HOST_PATIENCE:?}"
            );
        }
    });
    assert!(
        logs.contains("lost the connection to the host compositor"),
        "the loss is named in the log:\n{logs}"
    );
    assert!(
        !logs.contains("other error during loop operation"),
        "no calloop-internal jargon escapes:\n{logs}"
    );
}

/// The flag itself: unset at rest, set once marked. Same-thread either way
/// (marked in dispatch, read after the loop), so one mark is visible
/// immediately -- no eventual-consistency window for the loop to outrun.
#[test]
fn host_loss_marks_once_and_stays() {
    let loss = HostLoss::default();
    assert!(!loss.lost());
    assert!(loss.mark(), "the first mark is the one that logs");
    assert!(!loss.mark(), "a second death must not log again");
    assert!(loss.lost());
}

// -------------------------------------------------------------------------
// Startup bound (a host that accepts but never answers)
// -------------------------------------------------------------------------

/// Starting against a host that accepts the connection but never answers
/// fails loud within `STARTUP_TIMEOUT` instead of wedging before `scoot is
/// up` with no IPC socket and nothing for a supervisor to act on.
///
/// The silent end is a socket pair whose server half is held open but never
/// dispatched: the registry roundtrip gets no answer, the same shape as the
/// restarted Selkies host from the #467 review (listening without
/// dispatching).
///
/// The scenario runs in a FRESH CHILD PROCESS (this test binary re-executed
/// with `SCOOT_SILENT_CHILD` set), not a fork and not a thread: `State` is
/// `!Send` so the scenario cannot move to a thread, and forking the
/// shared-process `cargo test` runner duplicates every concurrently-running
/// fixture's sockets and Smithay state into the child -- that shape broke
/// unrelated `layer_shell` teardown tests in the same run, deterministically
/// (see the report). A re-executed process shares nothing but the binary:
/// the child builds its own harness single-threaded, holds its own silent
/// server with no thread behind it, and exits with a code. The parent only
/// manages the subprocess and bounds the pre-fix hang with its own deadline,
/// so before the bound the test fails instead of hanging nextest.
#[test]
fn starting_against_a_silent_host_fails_loud_within_the_bound() {
    /// The parent's deadline: the child's whole `STARTUP_TIMEOUT` plus
    /// slack for a loaded box to re-exec, build a harness and time out.
    /// Well under nextest's 60 s slow mark either way: ~10 s after the fix,
    /// 30 s on a pre-fix wedge.
    const SILENT_PARENT_PATIENT: Duration = Duration::from_secs(30);
    assert!(
        SILENT_PARENT_PATIENT > STARTUP_TIMEOUT,
        "the parent must outwait the child's bound"
    );
    // libtest names tests by module path *without* the crate prefix
    // (`compositor::nested::tests::...`, as a failure list shows), while
    // `module_path!()` keeps it (`scoot::compositor::...`), so the crate
    // segment is stripped for the `--exact` filter.
    let child_path = match module_path!().split_once("::") {
        Some((_, rest)) => format!("{rest}::silent_child_entry"),
        None => format!("{}::silent_child_entry", module_path!()),
    };
    let mut child = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", &child_path, "--nocapture"])
        .env("SCOOT_SILENT_CHILD", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the silent-host child starts");
    let deadline = Instant::now() + SILENT_PARENT_PATIENT;
    let (code, stdout) = loop {
        match child
            .try_wait()
            .expect("the silent-host child is waited on")
        {
            Some(status) => {
                // The child's output is tiny (a libtest summary); draining
                // it after the exit cannot deadlock.
                let mut stdout = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_string(&mut stdout);
                }
                break (status.code(), stdout);
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "starting against a silent host wedged past \
                         {SILENT_PARENT_PATIENT:?}: the startup wait is unbounded"
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    assert_eq!(
        code,
        Some(SILENT_CHILD_PASS),
        "starting against a silent host must fail loud naming the host (child exit {code:?})"
    );
    // Guards a filter typo: `--exact` with no match runs zero tests and
    // still exits 0, which the exit code alone would pass vacuously. (The
    // libtest `1 passed` summary cannot serve here: the entry exits via
    // `process::exit`, which never prints it -- so the entry prints its own
    // marker instead.)
    assert!(
        stdout.contains("SILENT_CHILD_PASS"),
        "the silent-host child ran no scenario; its filter names nothing:\n{stdout}"
    );
}

/// What [`silent_child_entry`] exits with: the scenario behaved (loud,
/// host-naming `Err` within the bound).
const SILENT_CHILD_PASS: i32 = 0;

/// The child's half of
/// [`starting_against_a_silent_host_fails_loud_within_the_bound`], run only
/// when this binary is re-executed with `SCOOT_SILENT_CHILD` set (a direct
/// `cargo test`/`nextest` run of this test is a no-op pass: the real check
/// is the parent above spawning it as a subprocess).
///
/// Single-threaded, with its own harness and its own silent server end held
/// open but never dispatched -- no thread behind the server, so the
/// registry roundtrip simply gets no answer. Exits with
/// [`SILENT_CHILD_PASS`] iff `init_on` fails loud naming the host;
/// any other outcome (including an `Ok` against a host that said nothing)
/// exits nonzero, and a pre-fix build never exits at all (the parent kills
/// it past its deadline).
#[test]
fn silent_child_entry() {
    if std::env::var("SCOOT_SILENT_CHILD").is_err() {
        return;
    }
    let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
    // Held open, never dispatched: closing every server copy would fail the
    // client fast with EPIPE instead of the silence this pins.
    let _held = server_end;
    let mut nested: Harness<(), ()> = Harness::headless(Appearance::default(), 64);
    let conn = match Connection::from_socket(client_end) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("silent child: could not wrap the host socket: {error}");
            std::process::exit(1);
        }
    };
    let handle = nested.event_loop.handle();
    let started = Instant::now();
    match init_on(handle, &mut nested.state, conn, 64, 64) {
        Err(error) => {
            let message = error.to_string();
            if !message.contains("host") {
                eprintln!("silent child: the startup failure does not name the host: {message}");
                std::process::exit(1);
            }
            if started.elapsed() > STARTUP_TIMEOUT + Duration::from_secs(10) {
                eprintln!("silent child: the startup bound took far longer than itself");
                std::process::exit(1);
            }
            // The marker the parent's filter-typo guard looks for (libtest
            // prints no summary past `process::exit`, so the entry says so
            // itself; flushed -- a piped stdout is block-buffered and
            // `process::exit` flushes nothing).
            println!("SILENT_CHILD_PASS");
            let _ = std::io::stdout().flush();
            std::process::exit(SILENT_CHILD_PASS);
        }
        Ok(()) => {
            eprintln!("silent child: starting against a silent host succeeded");
            std::process::exit(1);
        }
    }
}

// -------------------------------------------------------------------------
// Scroll buffering (`PendingAxis`)
// -------------------------------------------------------------------------

use super::PendingAxis;
use smithay::backend::input::{AxisRelativeDirection, AxisSource, InputTime};

/// One host frame's finger scroll keeps its source, value and stop: the
/// three events the host sent separately become one client frame, which is
/// the whole point of buffering until `Frame`.
#[test]
fn a_host_finger_frame_keeps_its_source_value_and_stop() {
    let mut pending = PendingAxis::default();
    pending.push_source(AxisSource::Finger);
    pending.push_axis(false, 12.0);
    let frame = pending
        .finish(InputTime::from_millis(1))
        .expect("a finger scroll is worth sending");
    assert_eq!(frame.source, Some(AxisSource::Finger));
    assert_eq!(frame.axis, (0.0, 12.0));
    assert_eq!(frame.stop, (false, false));

    let mut pending = PendingAxis::default();
    pending.push_source(AxisSource::Finger);
    pending.push_stop(false);
    let frame = pending
        .finish(InputTime::from_millis(2))
        .expect("a stop is worth sending");
    assert_eq!(frame.stop, (false, true));
    assert_eq!(frame.axis, (0.0, 0.0));
}

/// `value120` wins over `discrete`: a host that sent both for one click
/// (Smithay's own server never does -- it sends one or the other by client
/// version -- but a foreign host might) must not double every wheel click.
#[test]
fn value120_wins_over_discrete_in_either_order() {
    for first_value120 in [true, false] {
        let mut pending = PendingAxis::default();
        pending.push_source(AxisSource::Wheel);
        if first_value120 {
            pending.push_value120(false, 120);
            pending.push_discrete(false, 1);
        } else {
            pending.push_discrete(false, 1);
            pending.push_value120(false, 120);
        }
        let frame = pending
            .finish(InputTime::from_millis(1))
            .expect("a wheel click is worth sending");
        assert_eq!(
            frame.v120,
            Some((0, 120)),
            "counted twice when value120 arrived {}",
            if first_value120 { "first" } else { "second" }
        );
    }
}

/// `discrete` alone scales to 120ths, so pre-v8 hosts still deliver steps.
#[test]
fn discrete_alone_scales_to_120ths() {
    let mut pending = PendingAxis::default();
    pending.push_source(AxisSource::Wheel);
    pending.push_discrete(false, -1);
    let frame = pending
        .finish(InputTime::from_millis(1))
        .expect("a discrete click is worth sending");
    assert_eq!(frame.v120, Some((0, -120)));
}

/// An empty buffer finishes to nothing, and finishing resets: a frame is
/// never forwarded twice, and one malformed host frame cannot poison the
/// next. A source alone (motion that never arrived) finishes to nothing
/// too -- a bare source is not a scroll.
#[test]
fn an_empty_buffer_finishes_to_nothing_and_resets() {
    let mut pending = PendingAxis::default();
    assert!(pending.finish(InputTime::from_millis(1)).is_none());
    pending.push_source(AxisSource::Finger);
    assert!(
        pending.finish(InputTime::from_millis(2)).is_none(),
        "a source with no motion is not a scroll"
    );
    pending.push_source(AxisSource::Finger);
    pending.push_axis(true, 5.0);
    let frame = pending
        .finish(InputTime::from_millis(3))
        .expect("motion after resets still sends");
    assert_eq!(frame.axis, (5.0, 0.0));
    assert!(pending.finish(InputTime::from_millis(4)).is_none());
}

/// Two `axis` events for one direction in one frame add up, the way
/// `AxisFrame::value` accumulates -- and the natural-scroll direction
/// rides along untouched.
#[test]
fn axes_accumulate_and_carry_their_direction() {
    let mut pending = PendingAxis::default();
    pending.push_source(AxisSource::Continuous);
    pending.push_axis(false, 4.0);
    pending.push_axis(false, 6.0);
    pending.push_direction(false, AxisRelativeDirection::Inverted);
    let frame = pending
        .finish(InputTime::from_millis(1))
        .expect("accumulated motion is a scroll");
    assert_eq!(frame.axis, (0.0, 10.0));
    assert_eq!(
        frame.relative_direction,
        (
            AxisRelativeDirection::Identical,
            AxisRelativeDirection::Inverted
        )
    );
}
