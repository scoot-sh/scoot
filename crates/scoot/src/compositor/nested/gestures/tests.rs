//! The host-gesture translation: the pure rules ([`Open`], [`Plan`]) on
//! plain values, then the whole path over real sockets -- a headless scoot
//! as the host (it advertises `zwp_pointer_gestures_v1` v3 with
//! `client_gestures`), a nested scoot connected to it through
//! `nested::init_on`, and a real `wayland-client` inside the nested scoot
//! hearing what the host's gestures became.
//!
//! The host's events are sent through Smithay's `PointerHandle` directly
//! (not scoot's entry points), so the host's own `[gestures]` claim on a
//! three-finger swipe cannot swallow what the test means to send.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use smithay::backend::input::InputTime;
use smithay::input::pointer::{
    GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
    GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
};
use smithay::utils::SERIAL_COUNTER;
use wayland_client::Connection;

use super::{HOLD_VERSION, HostGesture, Open, Plan};
use crate::compositor::decorations::Appearance;
use crate::compositor::gesture::Route;
use crate::compositor::gesture::client::init_client_gestures;
use crate::compositor::gesture::client::tests::{Ev, Step, run_client};
use crate::compositor::nested::init_on;
use crate::compositor::state::ClientState;
use crate::compositor::test_support::Harness;

// ---- the pure rules ----

use HostGesture::*;

fn swipe_begin() -> HostGesture {
    SwipeBegin {
        fingers: 4,
        time: 1,
    }
}
fn swipe_update() -> HostGesture {
    SwipeUpdate {
        dx: 1.0,
        dy: 2.0,
        time: 2,
    }
}
fn swipe_end() -> HostGesture {
    SwipeEnd {
        cancelled: false,
        time: 3,
    }
}
fn pinch_update() -> HostGesture {
    PinchUpdate {
        dx: 1.0,
        dy: 2.0,
        scale: 1.5,
        rotation: 3.0,
        time: 2,
    }
}

#[test]
fn an_update_or_end_without_a_begin_is_not_admitted() {
    let mut open = Open::default();
    for event in [
        swipe_update(),
        swipe_end(),
        pinch_update(),
        PinchEnd {
            cancelled: false,
            time: 3,
        },
        HoldEnd {
            cancelled: false,
            time: 3,
        },
    ] {
        assert!(!open.admit(&event), "{event:?} followed no begin");
    }
    assert_eq!(open, Open::default(), "a dropped event opened something");
}

#[test]
fn a_gesture_is_admitted_from_its_begin_to_its_end_and_not_after() {
    let mut open = Open::default();
    assert!(open.admit(&swipe_begin()));
    assert!(open.admit(&swipe_update()));
    assert!(open.admit(&swipe_end()));
    assert!(!open.admit(&swipe_update()), "an update after the end");
    assert!(!open.admit(&swipe_end()), "a second end");
}

#[test]
fn each_kind_is_tracked_on_its_own() {
    let mut open = Open::default();
    assert!(open.admit(&HoldBegin {
        fingers: 2,
        time: 1
    }));
    assert!(!open.admit(&swipe_end()), "a hold is not a swipe");
    assert!(!open.admit(&pinch_update()), "a hold is not a pinch");
    assert!(open.admit(&HoldEnd {
        cancelled: true,
        time: 2
    }));
}

#[test]
fn a_begin_over_an_open_gesture_is_passed_on_for_the_entry_point_to_close() {
    let mut open = Open::default();
    assert!(open.admit(&swipe_begin()));
    assert!(open.admit(&swipe_begin()));
    assert!(open.admit(&swipe_end()));
    assert!(!open.admit(&swipe_end()));
}

#[test]
fn what_is_open_is_cancelled_and_nothing_else() {
    let mut open = Open::default();
    open.admit(&swipe_begin());
    open.admit(&HoldBegin {
        fingers: 2,
        time: 1,
    });
    let cancels: Vec<_> = open.cancels(9).collect();
    assert_eq!(
        cancels,
        vec![
            SwipeEnd {
                cancelled: true,
                time: 9
            },
            HoldEnd {
                cancelled: true,
                time: 9
            },
        ]
    );
    assert_eq!(Open::default().cancels(9).count(), 0);
}

#[test]
fn the_plan_follows_the_host_version_and_the_client_global() {
    // Swipe is scoot's own: always. Pinch and hold exist only for clients.
    for version in 1..=3 {
        let off = Plan::new(version, false);
        assert!(
            off.swipe && !off.pinch && !off.hold,
            "v{version} off: {off:?}"
        );
    }
    // v1 and v2 hosts have no `get_hold_gesture`: asking is a protocol error.
    for version in [1, 2] {
        let on = Plan::new(version, true);
        assert!(on.swipe && on.pinch && !on.hold, "v{version} on: {on:?}");
    }
    assert!(Plan::new(HOLD_VERSION, true).hold);
    assert_eq!(HOLD_VERSION, 3);
}

// ---- the whole path ----

const PATIENCE: Duration = Duration::from_secs(10);
const CANVAS: i32 = 400;

type HostJob = Box<dyn FnOnce(&mut Harness<(), ()>) + Send>;

/// The host: a headless scoot on its own thread (`State` is `!Send`), doing
/// the jobs it is sent between dispatches.
fn host_thread(
    server_end: UnixStream,
    offers_gestures: bool,
    ready: Sender<()>,
    jobs: Receiver<HostJob>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut host: Harness<(), ()> = Harness::headless(Appearance::default(), CANVAS);
        init_client_gestures(&mut host.state, offers_gestures);
        host.state
            .display_handle
            .insert_client(server_end, std::sync::Arc::new(ClientState::default()))
            .expect("the nested session connects");
        ready.send(()).expect("the test waits for the host");
        loop {
            match jobs.recv_timeout(Duration::from_millis(2)) {
                Ok(job) => job(&mut host),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            host.settle();
        }
    })
}

struct Rig {
    nested: Harness<Step, crate::compositor::gesture::client::tests::Ack>,
    jobs: Option<Sender<HostJob>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Closing the channel ends the host's loop.
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Sends `event` to the pointer focus of `host` the way the host's own
/// libinput arms would, bypassing its `[gestures]` routing.
fn inject(host: &mut Harness<(), ()>, event: HostGesture) {
    let Some(pointer) = host.state.seat.get_pointer() else {
        return;
    };
    let state = &mut host.state;
    let serial = || SERIAL_COUNTER.next_serial();
    let t = InputTime::from_millis;
    match event {
        SwipeBegin { fingers, time } => pointer.gesture_swipe_begin(
            state,
            &GestureSwipeBeginEvent {
                serial: serial(),
                time: t(time),
                fingers,
            },
        ),
        SwipeUpdate { dx, dy, time } => pointer.gesture_swipe_update(
            state,
            &GestureSwipeUpdateEvent {
                time: t(time),
                delta: (dx, dy).into(),
            },
        ),
        SwipeEnd { cancelled, time } => pointer.gesture_swipe_end(
            state,
            &GestureSwipeEndEvent {
                serial: serial(),
                time: t(time),
                cancelled,
            },
        ),
        PinchBegin { fingers, time } => pointer.gesture_pinch_begin(
            state,
            &GesturePinchBeginEvent {
                serial: serial(),
                time: t(time),
                fingers,
            },
        ),
        PinchUpdate {
            dx,
            dy,
            scale,
            rotation,
            time,
        } => pointer.gesture_pinch_update(
            state,
            &GesturePinchUpdateEvent {
                time: t(time),
                delta: (dx, dy).into(),
                scale,
                rotation,
            },
        ),
        PinchEnd { cancelled, time } => pointer.gesture_pinch_end(
            state,
            &GesturePinchEndEvent {
                serial: serial(),
                time: t(time),
                cancelled,
            },
        ),
        HoldBegin { fingers, time } => pointer.gesture_hold_begin(
            state,
            &GestureHoldBeginEvent {
                serial: serial(),
                time: t(time),
                fingers,
            },
        ),
        HoldEnd { cancelled, time } => pointer.gesture_hold_end(
            state,
            &GestureHoldEndEvent {
                serial: serial(),
                time: t(time),
                cancelled,
            },
        ),
    }
}

impl Rig {
    /// A host that does or does not offer the global, and a nested scoot
    /// (`client_gestures` as given) with one mapped, painted client that
    /// the host pointer is on.
    fn new(host_offers: bool, client_gestures: bool) -> Self {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        let (ready_tx, ready_rx) = channel();
        let (job_tx, job_rx) = channel();
        let thread = host_thread(server_end, host_offers, ready_tx, job_rx);
        ready_rx.recv_timeout(PATIENCE).expect("the host comes up");

        let mut nested = Harness::headless(Appearance::default(), CANVAS);
        init_client_gestures(&mut nested.state, client_gestures);
        let conn = Connection::from_socket(client_end).expect("a host connection");
        let handle = nested.event_loop.handle();
        init_on(handle, &mut nested.state, conn, CANVAS, CANVAS).expect("the nested backend");
        let mut rig = Self {
            nested,
            jobs: Some(job_tx),
            thread: Some(thread),
        };

        // The first configure builds the render target; a frame then maps
        // scoot's window in the host.
        rig.until("the host to map the nested window", |rig| {
            rig.nested.state.request_render();
            rig.nested.state.render();
            rig.host(|h| h.state.world.arrange().placements.len()) == 1
        });
        rig.until("the nested seat to get the host pointer", |rig| {
            rig.point_host((1.0, 1.0), 0);
            rig.nested
                .state
                .host
                .as_ref()
                .is_some_and(|h| h.pointer().is_some())
        });
        assert_eq!(rig.nested.spawn(run_client), 0);
        rig.nested.run_on(0, Step::Map);
        rig.until("the host pointer to be on the nested client", |rig| {
            let middle = rig.client_middle();
            rig.point_host(middle, 1);
            rig.nested.run_on(0, Step::Report).inside
        });
        rig
    }

    /// Runs `f` on the host thread and waits for its answer.
    fn host<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Harness<(), ()>) -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = channel();
        self.jobs
            .as_ref()
            .expect("the host is up")
            .send(Box::new(move |h| {
                let _ = tx.send(f(h));
            }))
            .expect("the host thread is running");
        rx.recv_timeout(PATIENCE).expect("the host answers")
    }

    /// Parks the host pointer so that it lands on `(x, y)` of the nested
    /// session (the host window's origin plus that); `nudge` shifts it a
    /// pixel so a second call is a motion.
    fn point_host(&self, (x, y): (f64, f64), nudge: i32) {
        self.host(move |h| {
            let rect = h.state.world.arrange().placements[0].rect;
            h.state.pointer_move(
                f64::from(rect.x) + x + f64::from(nudge),
                f64::from(rect.y) + y,
            );
        });
    }

    /// The middle of the nested client's window, in nested coordinates.
    fn client_middle(&self) -> (f64, f64) {
        let rect = self.nested.state.world.arrange().placements[0].rect;
        (
            f64::from(rect.x + rect.w / 2),
            f64::from(rect.y + rect.h / 2),
        )
    }

    fn send(&self, events: impl IntoIterator<Item = HostGesture> + Send + 'static) {
        self.host(move |h| events.into_iter().for_each(|e| inject(h, e)));
    }

    /// Dispatches the nested scoot until `done`, which may poke the host.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: nested pointer {:?}, nested windows {:?}",
                self.nested
                    .state
                    .seat
                    .get_pointer()
                    .map(|p| p.current_location()),
                self.nested
                    .state
                    .world
                    .arrange()
                    .placements
                    .iter()
                    .map(|p| p.rect)
                    .collect::<Vec<_>>(),
            );
            self.nested.settle();
        }
    }

    /// Everything the nested client heard once the host's events (sent
    /// before this call) have all been through: the sentinel hold, sent last,
    /// is the marker, and is not part of the answer.
    fn heard(&mut self) -> Vec<Ev> {
        self.heard_by(
            [
                HoldBegin {
                    fingers: 9,
                    time: 99,
                },
                HoldEnd {
                    cancelled: false,
                    time: 99,
                },
            ],
            [Ev::HoldBegin(9), Ev::HoldEnd(false)],
        )
    }

    /// The same with a nine-finger swipe as the marker, for a host that
    /// cannot send a hold (version 2 or older).
    fn heard_without_hold(&mut self) -> Vec<Ev> {
        self.heard_by(
            [
                SwipeBegin {
                    fingers: 9,
                    time: 99,
                },
                SwipeEnd {
                    cancelled: false,
                    time: 99,
                },
            ],
            [Ev::SwipeBegin(9), Ev::SwipeEnd(false)],
        )
    }

    fn heard_by(&mut self, marker: [HostGesture; 2], expect: [Ev; 2]) -> Vec<Ev> {
        self.send(marker);
        let mut heard = Vec::new();
        self.until("the marker to arrive", |rig| {
            heard.extend(rig.nested.run_on(0, Step::Report).events);
            heard.ends_with(&expect)
        });
        heard.truncate(heard.len() - 2);
        heard
    }
}

#[test]
fn a_host_swipe_pinch_and_hold_reach_the_nested_client() {
    let mut rig = Rig::new(true, true);
    rig.send([
        SwipeBegin {
            fingers: 4,
            time: 10,
        },
        SwipeUpdate {
            dx: 3.0,
            dy: -1.5,
            time: 11,
        },
        SwipeEnd {
            cancelled: false,
            time: 12,
        },
        PinchBegin {
            fingers: 2,
            time: 20,
        },
        PinchUpdate {
            dx: 1.0,
            dy: -2.0,
            scale: 1.5,
            rotation: 10.0,
            time: 21,
        },
        PinchEnd {
            cancelled: false,
            time: 22,
        },
        HoldBegin {
            fingers: 3,
            time: 30,
        },
        HoldEnd {
            cancelled: true,
            time: 31,
        },
    ]);
    assert_eq!(
        rig.heard(),
        vec![
            Ev::SwipeBegin(4),
            Ev::SwipeUpdate(3.0, -1.5),
            Ev::SwipeEnd(false),
            Ev::PinchBegin(2),
            Ev::PinchUpdate(1.0, -2.0, 1.5, 10.0),
            Ev::PinchEnd(false),
            Ev::HoldBegin(3),
            Ev::HoldEnd(true),
        ]
    );
}

#[test]
fn the_nested_session_asked_the_host_for_what_it_uses() {
    let rig = Rig::new(true, true);
    let host = rig.nested.state.host.as_ref().expect("a nested host");
    assert_eq!(host.gestures.bound(), Some(3));
    assert_eq!(
        host.gestures.plan_made(),
        Some(Plan {
            swipe: true,
            pinch: true,
            hold: true
        })
    );
    // Without the client global, swipe only: scoot's own is all that is used.
    let rig = Rig::new(true, false);
    let host = rig.nested.state.host.as_ref().expect("a nested host");
    assert_eq!(
        host.gestures.plan_made(),
        Some(Plan {
            swipe: true,
            pinch: false,
            hold: false
        })
    );
}

#[test]
fn a_host_without_the_global_is_a_quiet_no_op() {
    let mut rig = Rig::new(false, true);
    let host = rig.nested.state.host.as_ref().expect("a nested host");
    assert_eq!(host.gestures.bound(), None, "bound a global the host lacks");
    assert_eq!(host.gestures.plan_made(), None);
    // The session works: the sentinel the host sends is simply never heard,
    // and nothing else is either.
    rig.send([
        SwipeBegin {
            fingers: 4,
            time: 1,
        },
        SwipeEnd {
            cancelled: false,
            time: 2,
        },
    ]);
    rig.nested.settle();
    assert!(rig.nested.run_on(0, Step::Report).events.is_empty());
}

#[test]
fn a_three_finger_host_swipe_is_scoots_and_the_client_hears_none_of_it() {
    // `client_gestures` off in the nested session: scoot's own swipe works
    // all the same, as on `--tty`.
    let mut rig = Rig::new(true, false);
    rig.send([SwipeBegin {
        fingers: 3,
        time: 10,
    }]);
    rig.until("the nested swipe to be claimed", |rig| {
        rig.nested.state.gestures.route == Route::Claimed
    });
    rig.send([
        SwipeUpdate {
            dx: -30.0,
            dy: 0.5,
            time: 11,
        },
        SwipeEnd {
            cancelled: false,
            time: 12,
        },
    ]);
    rig.until("the nested swipe to end", |rig| {
        rig.nested.state.gestures.route == Route::Idle
    });
    // Nothing reached the client, with no global in the nested session.
    assert!(rig.nested.run_on(0, Step::Report).events.is_empty());
}

#[test]
fn the_host_pointer_going_away_cancels_the_gesture_and_it_comes_back_clean() {
    let mut rig = Rig::new(true, true);
    rig.send([
        SwipeBegin {
            fingers: 4,
            time: 10,
        },
        PinchBegin {
            fingers: 2,
            time: 11,
        },
        HoldBegin {
            fingers: 3,
            time: 12,
        },
    ]);
    // Not `heard()`: its sentinel hold would close the open one.
    let mut opened = Vec::new();
    rig.until("the nested client to hear the begins", |rig| {
        opened.extend(rig.nested.run_on(0, Step::Report).events);
        opened.len() >= 3
    });
    assert_eq!(
        opened,
        vec![Ev::SwipeBegin(4), Ev::PinchBegin(2), Ev::HoldBegin(3)]
    );

    // The host seat loses its pointer in the middle of all three.
    rig.host(|h| h.state.seat.remove_pointer());
    let mut heard = Vec::new();
    rig.until("the nested client to hear the cancels", |rig| {
        heard.extend(rig.nested.run_on(0, Step::Report).events);
        heard.len() >= 3
    });
    assert_eq!(
        heard,
        vec![Ev::SwipeEnd(true), Ev::PinchEnd(true), Ev::HoldEnd(true)]
    );
    let host = rig.nested.state.host.as_ref().expect("a nested host");
    assert!(host.pointer().is_none() && host.gestures.plan_made().is_none());

    // And it can come back: new pointer, new objects, gestures flow again.
    rig.host(|h| {
        h.state.seat.add_pointer();
    });
    rig.until("the nested seat to get the new pointer", |rig| {
        let middle = rig.client_middle();
        rig.point_host(middle, 0);
        rig.point_host(middle, 1);
        rig.nested
            .state
            .host
            .as_ref()
            .is_some_and(|h| h.pointer().is_some() && h.gestures.plan_made().is_some())
    });
    rig.send([
        SwipeBegin {
            fingers: 4,
            time: 20,
        },
        SwipeEnd {
            cancelled: false,
            time: 21,
        },
    ]);
    assert_eq!(rig.heard(), vec![Ev::SwipeBegin(4), Ev::SwipeEnd(false)]);
}

#[test]
fn a_cancelled_end_is_passed_on_cancelled() {
    let mut rig = Rig::new(true, true);
    rig.send([
        PinchBegin {
            fingers: 2,
            time: 10,
        },
        PinchEnd {
            cancelled: true,
            time: 11,
        },
        SwipeBegin {
            fingers: 4,
            time: 12,
        },
        SwipeEnd {
            cancelled: true,
            time: 13,
        },
    ]);
    assert_eq!(
        rig.heard(),
        vec![
            Ev::PinchBegin(2),
            Ev::PinchEnd(true),
            Ev::SwipeBegin(4),
            Ev::SwipeEnd(true)
        ]
    );
}

#[test]
fn a_gesture_the_host_ends_by_leaving_is_not_ended_twice() {
    // A conformant host sends the cancelled `end` itself when the pointer
    // leaves; the nested session must not add a second one.
    let mut rig = Rig::new(true, true);
    rig.send([SwipeBegin {
        fingers: 4,
        time: 10,
    }]);
    rig.host(|h| h.state.pointer_move(f64::from(CANVAS - 1), 1.0));
    let mut heard = Vec::new();
    rig.until("the cancel", |rig| {
        heard.extend(rig.nested.run_on(0, Step::Report).events);
        heard.len() >= 2
    });
    assert_eq!(heard, vec![Ev::SwipeBegin(4), Ev::SwipeEnd(true)]);
    rig.nested.settle();
    assert!(rig.nested.run_on(0, Step::Report).events.is_empty());
}

#[test]
fn a_gesture_the_host_never_ended_is_cancelled_when_its_pointer_leaves() {
    // A host that left without ending the gesture. The event is fed to the
    // nested session as if the host had begun it (the test's host would have
    // cancelled it itself), then the real host pointer leaves.
    let mut rig = Rig::new(true, true);
    rig.nested.state.host_gesture(SwipeBegin {
        fingers: 4,
        time: 1,
    });
    rig.host(|h| h.state.pointer_move(f64::from(CANVAS - 1), 1.0));
    let mut heard = Vec::new();
    rig.until("the cancel", |rig| {
        heard.extend(rig.nested.run_on(0, Step::Report).events);
        heard.len() >= 2
    });
    assert_eq!(heard, vec![Ev::SwipeBegin(4), Ev::SwipeEnd(true)]);
}

#[test]
fn a_version_2_host_gets_no_hold_request_and_the_session_carries_on() {
    // The only host a test can run is Smithay's, fixed at version 3, so the
    // nested side binds at most 2: from the client's end that is exactly an
    // older host. A `get_hold_gesture` on it would be a protocol error that
    // ends the connection, so the rest of the test only works if none is sent.
    super::BIND_CAP.with(|cap| cap.set(2));
    let mut rig = Rig::new(true, true);
    let host = rig.nested.state.host.as_ref().expect("a nested host");
    assert_eq!(host.gestures.bound(), Some(2));
    assert_eq!(
        host.gestures.plan_made(),
        Some(Plan {
            swipe: true,
            pinch: true,
            hold: false
        })
    );
    rig.send([
        SwipeBegin {
            fingers: 4,
            time: 10,
        },
        SwipeEnd {
            cancelled: false,
            time: 11,
        },
        PinchBegin {
            fingers: 2,
            time: 12,
        },
        PinchEnd {
            cancelled: false,
            time: 13,
        },
        HoldBegin {
            fingers: 3,
            time: 14,
        },
        HoldEnd {
            cancelled: false,
            time: 15,
        },
    ]);
    assert_eq!(
        rig.heard_without_hold(),
        vec![
            Ev::SwipeBegin(4),
            Ev::SwipeEnd(false),
            Ev::PinchBegin(2),
            Ev::PinchEnd(false),
        ],
        "swipe and pinch arrive; the hold has nowhere to go"
    );
}

#[test]
fn a_cancel_made_here_carries_the_hosts_clock() {
    let mut rig = Rig::new(true, true);
    let state = &mut rig.nested.state;
    state.host_gesture(PinchBegin {
        fingers: 2,
        time: 5_000_000,
    });
    state.host_gesture(PinchUpdate {
        dx: 0.0,
        dy: 0.0,
        scale: 1.0,
        rotation: 0.0,
        time: 5_000_040,
    });
    let host = state.host.as_mut().expect("a nested host");
    let open = host.gestures.take_open();
    assert_eq!(host.gestures.last_time, 5_000_040);
    assert_eq!(
        open.cancels(host.gestures.last_time).collect::<Vec<_>>(),
        vec![PinchEnd {
            cancelled: true,
            time: 5_000_040
        }]
    );
}
