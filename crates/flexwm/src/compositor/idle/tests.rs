//! Tests for idle detection and inhibition.
//!
//! Like every other live-`State` test module here, these drive a real
//! `wayland-client` through a real [`State`] over a socket pair and assert
//! on what the client was actually sent -- `idled`/`resumed` events, not a
//! flag in the compositor. The timers behind those events are real calloop
//! timers (50-100ms, not mocked), so these take about a second each; the
//! waits poll rather than sleep, and every wait has a deadline that fails
//! the test instead of hanging it.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket, which
//! nothing here connects to (the client is a socket pair) but which is
//! created either way.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use flexwm_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::protocol::{wl_compositor, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1, zwp_idle_inhibitor_v1,
};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// What the client can be asked to do, in order. Idle events arrive
/// asynchronously (a calloop timer on the compositor side), so most steps
/// only arrange state; [`Fixture::idle_counts`] polls for what arrived.
enum Step {
    /// Watch the seat for idleness with a `timeout_ms` quiet period --
    /// `get_idle_notification`, which honors inhibitors.
    Watch { timeout_ms: u32 },
    /// The same via `get_input_idle_notification`, which ignores them.
    WatchInput { timeout_ms: u32 },
    /// Report how many `idled`/`resumed` events have arrived so far.
    ReportIdle,
    /// Create a bare `wl_surface` to hang an inhibitor on. No role, no
    /// buffer, never mapped: inhibition is about the surface existing,
    /// not about it showing anything (see `idle.rs`'s scoping note).
    CreateSurface,
    /// Inhibit idle on the surface [`Step::CreateSurface`] made. Kept
    /// alive client-side, the way a video player holds its inhibitor.
    Inhibit,
    /// Destroy the most recent inhibitor (`zwp_idle_inhibitor_v1.destroy`).
    Uninhibit,
    /// Destroy the surface the inhibitor hangs on without destroying the
    /// inhibitor first -- the disconnect-without-cleanup path, where only
    /// the compositor's surface-destroyed hook can release the hold.
    DestroySurface,
}

/// What the client reports back once a step is done.
enum Ack {
    Done,
    /// [`Step::ReportIdle`]'s answer: cumulative `idled`, then `resumed`.
    Idle {
        idled: u32,
        resumed: u32,
    },
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    seat: Option<wl_seat::WlSeat>,
    idle_notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    inhibit_manager: Option<zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1>,
    idled: u32,
    resumed: u32,
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            "ext_idle_notifier_v1" => {
                client.idle_notifier = Some(registry.bind(name, version.min(2), qh, ()))
            }
            "zwp_idle_inhibit_manager_v1" => {
                client.inhibit_manager = Some(registry.bind(name, version.min(1), qh, ()))
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => client.idled += 1,
            ext_idle_notification_v1::Event::Resumed => client.resumed += 1,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(TestClient: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);
wayland_client::delegate_noop!(TestClient: ignore zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1);
wayland_client::delegate_noop!(TestClient: ignore zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1);

/// Runs the client half: binds the globals, then executes whatever steps
/// the test sends.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let seat = client.seat.clone().ok_or("no wl_seat")?;
    let notifier = client
        .idle_notifier
        .clone()
        .ok_or("no ext_idle_notifier_v1 -- the global is missing")?;
    let inhibit_manager = client
        .inhibit_manager
        .clone()
        .ok_or("no zwp_idle_inhibit_manager_v1 -- the global is missing")?;

    // Surfaces to inhibit on, and inhibitors held, both kept alive so a
    // late event lands on a live proxy rather than killing the
    // connection.
    let mut surfaces: Vec<wl_surface::WlSurface> = Vec::new();
    let mut inhibitors: Vec<zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match &step {
            Step::Watch { timeout_ms } => {
                notifier.get_idle_notification(*timeout_ms, &seat, &qh, ());
            }
            Step::WatchInput { timeout_ms } => {
                notifier.get_input_idle_notification(*timeout_ms, &seat, &qh, ());
            }
            Step::ReportIdle => {
                outcome = Ack::Idle {
                    idled: client.idled,
                    resumed: client.resumed,
                };
            }
            Step::CreateSurface => {
                surfaces.push(compositor.create_surface(&qh, ()));
            }
            Step::Inhibit => {
                let surface = surfaces.last().ok_or("no surface to inhibit on")?;
                inhibitors.push(inhibit_manager.create_inhibitor(surface, &qh, ()));
            }
            Step::Uninhibit => {
                inhibitors.pop().ok_or("no inhibitor to release")?.destroy();
            }
            Step::DestroySurface => {
                surfaces.pop().ok_or("no surface to destroy")?.destroy();
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one connected
/// client, scripted a step at a time -- the shape `layer_shell/tests.rs`
/// established.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    steps: Option<Sender<Step>>,
    acks: Receiver<Ack>,
    client: Option<JoinHandle<Result<(), String>>>,
}

impl Fixture {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, 200, 200).expect("a headless backend");

        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let handle = thread::spawn(move || run_client(client_end, step_rx, ack_tx));

        Self {
            event_loop,
            state,
            steps: Some(step_tx),
            acks: ack_rx,
            client: Some(handle),
        }
    }

    /// Runs one client step to completion, then lets the compositor
    /// settle so anything the step provoked has happened before the test
    /// looks.
    fn run(&mut self, step: Step) -> Ack {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        let ack = self.wait_for(&acks, "a client step acknowledgement");
        self.acks = acks;
        self.settle();
        ack
    }

    /// Dispatches until `channel` produces a value. A client that died
    /// instead of answering is reported with its own error rather than
    /// as a timeout: the channel disconnecting is exactly that case.
    fn wait_for<T>(&mut self, channel: &Receiver<T>, what: &str) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match channel.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => {
                    let outcome = self
                        .client
                        .take()
                        .map(|handle| handle.join().expect("the client thread"));
                    panic!("the client stopped while waiting for {what}: {outcome:?}");
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    /// A few dispatch cycles with nothing outstanding, so in-flight
    /// protocol traffic in both directions has been processed -- and, at
    /// ~10ms a call, the poll quantum the idle waits below are built on.
    fn settle(&mut self) {
        for _ in 0..10 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let _ = self.state.display_handle.flush_clients();
    }

    /// The client's cumulative `(idled, resumed)` counts.
    fn idle_counts(&mut self) -> (u32, u32) {
        let Ack::Idle { idled, resumed } = self.run(Step::ReportIdle) else {
            panic!("the idle probe should report what the client saw");
        };
        (idled, resumed)
    }

    /// Polls until the client has seen a `resumed` for the latest input
    /// episode, returning the latest counts. Asserts exactly one resume:
    /// the protocol forbids two `resumed` without an `idled` between, and
    /// only one input episode happened since the last one. Deliberately
    /// does *not* pin `idled` exactly: the resume re-arms the timer
    /// synchronously, so a scheduling stall longer than the watch window
    /// may legitimately observe the next quiet window's `idled` before
    /// this poll runs -- pinning `(1, 1)` would flake on a loaded VM.
    /// Polls until the client has seen at least `idled` `idled` events
    /// (or `resumed` `resumed` ones when `resumed` is `Some`), failing on
    /// a deadline rather than hanging. Returns the latest counts.
    fn wait_for_idle(&mut self, idled: u32, resumed: Option<u32>) -> (u32, u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let counts = self.idle_counts();
            let resumed_ok = resumed.is_none_or(|want| counts.1 >= want);
            if counts.0 >= idled && resumed_ok {
                return counts;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for idled={idled} resumed={resumed:?}; saw {counts:?}"
            );
        }
    }

    /// Polls until the client has seen a `resumed` for the latest input
    /// episode, returning the latest counts. Asserts exactly one resume:
    /// the protocol forbids two `resumed` without an `idled` between, and
    /// only one input episode happened since the last one. Deliberately
    /// does *not* pin `idled` exactly: the resume re-arms the timer
    /// synchronously, so a scheduling stall longer than the watch window
    /// may legitimately observe the next quiet window's `idled` before
    /// this poll runs -- pinning `(1, 1)` would flake on a loaded VM.
    fn wait_for_resumed(&mut self, what: &str) -> (u32, u32) {
        let counts = self.wait_for_idle(1, Some(1));
        assert_eq!(
            counts.1, 1,
            "{what}: one input episode should resume the seat exactly once"
        );
        counts
    }

    /// Polls for `quiet` without the client's `idled` count moving past
    /// `idled` -- the assertion behind "inhibition holds". Fails on any
    /// movement, not on the deadline: the deadline only bounds the wait.
    fn assert_stays_below(&mut self, idled: u32, quiet: Duration) {
        let deadline = Instant::now() + quiet;
        loop {
            let (seen, _) = self.idle_counts();
            assert!(
                seen < idled,
                "the seat went idle while it should have been held awake"
            );
            if Instant::now() >= deadline {
                return;
            }
        }
    }

    /// Disconnects the client and waits for the compositor to notice.
    fn disconnect_client(&mut self) {
        drop(self.steps.take());
        if let Some(handle) = self.client.take() {
            handle
                .join()
                .expect("the client thread")
                .expect("the client ran cleanly");
        }
        self.settle();
    }
}

impl Drop for Fixture {
    /// Closes the step channel, which is what ends [`run_client`]'s loop.
    ///
    /// Deliberately does *not* join while unwinding: a compositor-side
    /// panic leaves the client thread blocked in a roundtrip whose answer
    /// will never come, and joining there turns a failing test into a
    /// hung one.
    fn drop(&mut self) {
        drop(self.steps.take());
        if std::thread::panicking() {
            return;
        }
        if let Some(handle) = self.client.take() {
            let _ = handle.join();
        }
    }
}

/// The watch timeout every test uses: long enough that scheduling jitter
/// on a loaded dev VM can't fire it early (nothing asserts "not yet
/// idle" inside the window), short enough that the suite stays fast.
const TIMEOUT_MS: u32 = 100;
/// How long "inhibition holds" waits with no `idled` arriving: three
/// full timeout windows, so a broken hold would have fired thrice over.
const HOLD_QUIET: Duration = Duration::from_millis(300);

/// A quiet seat goes idle once per watch, and the next input resumes it.
///
/// The core contract: `idled` arrives after the timeout with no input,
/// exactly once (the protocol forbids two `idled` without a `resumed`
/// between), and a pointer motion -- the same `State::pointer_move` every
/// input source funnels through -- produces exactly one `resumed`.
#[test]
fn a_quiet_seat_idles_and_input_resumes_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Watch {
        timeout_ms: TIMEOUT_MS,
    });

    assert_eq!(
        fixture.wait_for_idle(1, None),
        (1, 0),
        "one quiet window should idle the seat exactly once"
    );

    fixture.state.pointer_move(10.0, 10.0);
    fixture.wait_for_resumed("input after idling");

    // ...and the re-armed timer fires again: idling is a cycle, not a
    // one-shot.
    assert_eq!(
        fixture.wait_for_idle(2, None),
        (2, 1),
        "a second quiet window should idle the seat again"
    );
    fixture.disconnect_client();
}

/// An inhibitor holds off `idled` until it is explicitly released, and
/// the seat idles on the next quiet window after that.
#[test]
fn an_inhibitor_holds_off_idle_until_released() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateSurface);
    fixture.run(Step::Inhibit);
    fixture.run(Step::Watch {
        timeout_ms: TIMEOUT_MS,
    });

    fixture.assert_stays_below(1, HOLD_QUIET);

    fixture.run(Step::Uninhibit);
    assert_eq!(
        fixture.wait_for_idle(1, None),
        (1, 0),
        "releasing the inhibitor should let the next quiet window idle"
    );
    fixture.disconnect_client();
}

/// Destroying the surface out from under a live inhibitor releases the
/// hold -- the client-disconnect-without-cleanup path, where only the
/// compositor's surface-destroyed hook can see the death.
///
/// The client surviving to report afterwards is itself an assertion:
/// the release must not take the connection with it.
#[test]
fn destroying_an_inhibitor_surface_releases_the_hold() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateSurface);
    fixture.run(Step::Inhibit);
    fixture.run(Step::Watch {
        timeout_ms: TIMEOUT_MS,
    });

    fixture.assert_stays_below(1, HOLD_QUIET);

    fixture.run(Step::DestroySurface);
    assert_eq!(
        fixture.wait_for_idle(1, None),
        (1, 0),
        "the dead surface must stop inhibiting once destroyed"
    );
    // ...and explicitly destroying the inhibitor afterwards is a no-op,
    // not a double-release: the destroyed hook already removed it. In
    // particular it must not re-arm a second `idled` on its own -- the
    // protocol forbids two `idled` without a `resumed` between, and no
    // input happened since the first.
    fixture.run(Step::Uninhibit);
    fixture.assert_stays_below(2, HOLD_QUIET);

    // The cycle still works after all of that: input resumes, quiet
    // idles again.
    fixture.state.pointer_move(10.0, 10.0);
    fixture.wait_for_resumed("input after the stale destroy");
    assert_eq!(
        fixture.wait_for_idle(2, None),
        (2, 1),
        "the seat should idle again on the next quiet window"
    );
    fixture.disconnect_client();
}

/// Inhibiting while already idle resumes the seat immediately -- a video
/// player starting playback over a sleeping screen wakes the timers,
/// and they stay held until the inhibitor goes.
#[test]
fn inhibiting_while_idle_resumes_and_holds() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateSurface);
    fixture.run(Step::Watch {
        timeout_ms: TIMEOUT_MS,
    });
    assert_eq!(fixture.wait_for_idle(1, None), (1, 0));

    fixture.run(Step::Inhibit);
    assert_eq!(
        fixture.wait_for_idle(1, Some(1)),
        (1, 1),
        "inhibiting an idle seat should resume it"
    );
    fixture.assert_stays_below(2, HOLD_QUIET);

    fixture.run(Step::Uninhibit);
    assert_eq!(fixture.wait_for_idle(2, None), (2, 1));
    fixture.disconnect_client();
}

/// `get_input_idle_notification` ignores inhibitors: input-driven idle
/// (what a locker with its own inhibit policy wants) still fires under
/// one.
#[test]
fn input_idle_notifications_ignore_inhibitors() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateSurface);
    fixture.run(Step::Inhibit);
    fixture.run(Step::WatchInput {
        timeout_ms: TIMEOUT_MS,
    });

    assert_eq!(
        fixture.wait_for_idle(1, None),
        (1, 0),
        "an input-idle watch should fire despite the inhibitor"
    );
    fixture.disconnect_client();
}

/// A zero-millisecond watch fires on the next dispatch, not never: the
/// timeout is "how long quiet", and an already-quiet seat is already
/// past it.
#[test]
fn a_zero_timeout_watch_fires_immediately() {
    let mut fixture = Fixture::new();
    fixture.run(Step::Watch { timeout_ms: 0 });

    assert_eq!(
        fixture.wait_for_idle(1, None),
        (1, 0),
        "a zero timeout should idle the quiet seat at once"
    );
    fixture.disconnect_client();
}

/// Inhibiting the same surface twice still releases with one destroy:
/// the set counts surfaces, not inhibitor objects, so a double-create
/// cannot wedge the hold on.
#[test]
fn a_duplicate_inhibit_releases_with_one_destroy() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateSurface);
    fixture.run(Step::Inhibit);
    fixture.run(Step::Inhibit);
    fixture.run(Step::Watch {
        timeout_ms: TIMEOUT_MS,
    });

    fixture.assert_stays_below(1, HOLD_QUIET);

    fixture.run(Step::Uninhibit);
    assert_eq!(
        fixture.wait_for_idle(1, None),
        (1, 0),
        "one destroy should release a doubly-inhibited surface"
    );
    fixture.disconnect_client();
}
