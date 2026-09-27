//! A measurement, run by hand (`#[ignore]`d; ~15 minutes at 50 a case): how an X drag
//! source's motion and the window manager's proxy unmap reach the source
//! when the drag's first motion onto another X window moves the proxy out
//! of the way. Its numbers are recorded in
//! `docs/backlog/protocols/xwayland-x-drag-first-motion-race.md`.
//!
//! scoot flushes the unmap to the X server before it flushes the motion to
//! XWayland, but the X server does not keep that order: it handles input
//! before requests that arrive in the same wakeup, and flushes a core
//! `MotionNotify` at once (it is "critical output"), so the source can see
//! the motion before the proxy's `UnmapNotify`. Whether that matters depends
//! on how the source picks its target, so the source here does it two ways:
//! asking the server on the motion (`QueryPointer`, as Qt and Chromium do),
//! or reading a map-state cache it keeps from root `SubstructureNotify`
//! events, at once on the motion (an eager form of GTK's `GdkWindowCache`).
//!
//! What this cannot represent: the source reacts in the test's thread
//! between compositor dispatches, and XWayland runs on the same four cores
//! as the test, so the gaps are this machine's; and GTK itself resolves in
//! a low-priority idle after its pending X events, and gets XI2 motion,
//! which the X server does not flush early -- the live `mousepad` runs are
//! what speak for GTK.
//!
//! ```sh
//! SCOOT_REQUIRE_XWAYLAND=1 FIRST_MOTION_N=50 cargo test -p scoot --features xwayland \
//!     --bin scoot first_motion_release_measurement -- --ignored --nocapture --test-threads=1
//! ```

use std::time::{Duration, Instant};

use scoot_ipc::PointerButton;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _, EventMask, Window as XWindow,
};

use super::dnd::{press_at, taken_over};
use super::drop::{centre, grabbed, visible};
use super::live::{BLUE, Live, RED, live};
use super::x11::{Props, XClient};
use super::xdnd::{PROXY_NAME, XDND_VERSION, packed};

/// How the source picks the window under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolve {
    /// Asks the server when it handles the motion.
    Query,
    /// Reads its event-fed cache of the proxy's map state.
    Cache,
}

/// When the release comes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    /// With the motion, in one input batch and one flush.
    SameStep,
    /// After the source has handled the motion and scoot has settled.
    Settled,
}

/// Where the drag is before its one motion onto the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    /// Just started, the source having named its types to the proxy.
    Announced,
    /// Just started, the source having said nothing to the proxy -- what
    /// GTK and Qt do (see `first_motion.rs`).
    Unannounced,
    /// After one motion within the source's own window: the motion onto
    /// the target maps the proxy back and unmaps it again.
    ViaOwnWindow,
}

#[derive(Clone, Copy)]
struct Case {
    start: Start,
    order: Order,
    resolve: Resolve,
}

/// One drag, as the source saw it.
#[derive(Default)]
struct Outcome {
    /// The proxy was still mapped as far as the source's events had said
    /// when the motion onto the target came.
    stale_at_motion: bool,
    found_proxy: bool,
    dropped_on_proxy: bool,
    target_got_drop: bool,
    grabbed_after: bool,
    proxy_alive_after: bool,
}

/// The source's side of the drag.
struct Source {
    owner: XWindow,
    proxy: XWindow,
    proxy_mapped: bool,
    dest: Option<XWindow>,
}

impl XClient {
    fn select(&self, window: XWindow, mask: EventMask) {
        self.conn
            .change_window_attributes(window, &ChangeWindowAttributesAux::new().event_mask(mask))
            .expect("an attributes request")
            .check()
            .expect("the X server accepted the event mask");
    }

    /// `xdnd_send`, but a window gone by then is an answer, not a panic: a
    /// source's message to the proxy can land after the drag ended.
    fn xdnd_try_send(&self, window: XWindow, kind: &str, data: [u32; 5]) -> bool {
        let kind = self.atom(kind);
        self.conn
            .send_event(
                false,
                window,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, window, kind, data),
            )
            .expect("a send request")
            .check()
            .is_ok()
    }

    fn exists(&self, window: XWindow) -> bool {
        self.conn
            .get_window_attributes(window)
            .expect("an attributes request")
            .reply()
            .is_ok()
    }
}

impl Source {
    /// Handles one of the source's events the way a toolkit's XDND source
    /// does. Answers whether it was the release.
    fn handle(&mut self, live: &Live, resolve: Resolve, event: &XEvent, out: &mut Outcome) -> bool {
        match event {
            XEvent::UnmapNotify(e) if e.window == self.proxy => self.proxy_mapped = false,
            XEvent::MapNotify(e) if e.window == self.proxy => self.proxy_mapped = true,
            XEvent::MotionNotify(motion) => {
                out.stale_at_motion = self.proxy_mapped;
                let under = match resolve {
                    Resolve::Cache if self.proxy_mapped => self.proxy,
                    _ => live.x.pointer().2,
                };
                out.found_proxy = under == self.proxy;
                if self.dest != Some(under) {
                    if let Some(old) = self.dest {
                        live.x
                            .xdnd_try_send(old, "XdndLeave", [self.owner, 0, 0, 0, 0]);
                    }
                    let utf8 = live.x.atom("UTF8_STRING");
                    live.x.xdnd_try_send(
                        under,
                        "XdndEnter",
                        [self.owner, XDND_VERSION << 24, utf8, 0, 0],
                    );
                    self.dest = Some(under);
                }
                let copy = live.x.atom("XdndActionCopy");
                let at = packed(motion.root_x, motion.root_y);
                live.x.xdnd_try_send(
                    under,
                    "XdndPosition",
                    [self.owner, 0, at, x11rb::CURRENT_TIME, copy],
                );
            }
            XEvent::ButtonRelease(_) => {
                if let Some(dest) = self.dest {
                    out.dropped_on_proxy = dest == self.proxy;
                    live.x.xdnd_try_send(
                        dest,
                        "XdndDrop",
                        [self.owner, 0, x11rb::CURRENT_TIME, 0, 0],
                    );
                }
                return true;
            }
            _ => {}
        }
        false
    }

    /// Handles the source's events as they arrive, each at once, until the
    /// release (or, without `until_release`, the first motion).
    fn pump(&mut self, live: &Live, resolve: Resolve, out: &mut Outcome, until_release: bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            while let Some(event) = live.x.conn.poll_for_event().expect("an event poll") {
                let released = self.handle(live, resolve, &event, out);
                if released || (!until_release && matches!(event, XEvent::MotionNotify(_))) {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "the source never saw its event");
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    /// Handles whatever has arrived, into an outcome that is not kept.
    fn catch_up(&mut self, live: &Live, resolve: Resolve) {
        let mut ignored = Outcome::default();
        for event in live.x.drain() {
            self.handle(live, resolve, &event, &mut ignored);
        }
    }
}

/// One drag from the source window onto `to`.
fn one(
    live: &mut Live,
    from: scoot_core::Rect,
    (target, target_xid): (&XClient, XWindow),
    to: (f64, f64),
    case: Case,
) -> Outcome {
    let (sx, sy) = centre(from);
    // The pointer arrives, and only then the press: with both in one batch
    // XWayland lost the press when the pointer came from another X window
    // (seen while writing this; not investigated).
    live.fixture.state.pointer_move(sx, sy);
    live.drain();
    press_at(&mut live.fixture.state, from);
    live.drain();
    live.x.drain();
    let owner = live.x.take_selection("XdndSelection");
    live.drain();
    assert!(taken_over(&live.fixture.state), "the X drag did not start");
    let (x, y, proxy) = live.x.pointer();
    assert_eq!(live.x.name_of(proxy), PROXY_NAME, "no proxy at the start");
    let mut source = Source {
        owner,
        proxy,
        proxy_mapped: false,
        dest: None,
    };
    if case.start != Start::Unannounced {
        let (utf8, copy) = (live.x.atom("UTF8_STRING"), live.x.atom("XdndActionCopy"));
        live.x
            .xdnd_send(proxy, "XdndEnter", [owner, XDND_VERSION << 24, utf8, 0, 0]);
        let at = packed(x, y);
        live.x.xdnd_send(
            proxy,
            "XdndPosition",
            [owner, 0, at, x11rb::CURRENT_TIME, copy],
        );
        source.dest = Some(proxy);
        live.drain();
    }
    source.catch_up(live, case.resolve);
    assert!(source.proxy_mapped, "the source never saw the proxy map");
    if case.start == Start::ViaOwnWindow {
        live.fixture.state.pointer_move(sx + 5.0, sy + 5.0);
        let _ = live.fixture.state.display_handle.flush_clients();
        source.pump(live, case.resolve, &mut Outcome::default(), false);
        live.drain();
        source.catch_up(live, case.resolve);
    }
    target.drain();

    let mut out = Outcome::default();
    live.fixture.state.pointer_move(to.0, to.1);
    if case.order == Order::SameStep {
        live.fixture
            .state
            .pointer_button(PointerButton::Left, false);
    }
    // Where scoot's loop flushes (`post_dispatch`): after the input batch.
    let _ = live.fixture.state.display_handle.flush_clients();
    source.pump(live, case.resolve, &mut out, case.order == Order::SameStep);
    live.drain();
    if case.order == Order::Settled {
        live.fixture
            .state
            .pointer_button(PointerButton::Left, false);
        let _ = live.fixture.state.display_handle.flush_clients();
        source.pump(live, case.resolve, &mut out, true);
        live.drain();
    }
    let dropped = target.atom("XdndDrop");
    out.target_got_drop = target.drain().iter().any(|event| {
        matches!(event, XEvent::ClientMessage(m) if m.window == target_xid && m.type_ == dropped)
    });
    out.grabbed_after = grabbed(&live.fixture.state);
    out.proxy_alive_after = live.x.exists(proxy);
    live.x.drain();
    out
}

#[test]
#[ignore = "a measurement, run by hand: see the module doc"]
fn first_motion_release_measurement() {
    let Some(mut live) = live("first_motion_release_measurement") else {
        return;
    };
    let iterations: usize = std::env::var("FIRST_MOTION_N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(50);
    let source_xid = live.x.map(&Props::new(RED));
    let source_id = live.managed(source_xid);
    live.x.xdnd_aware(source_xid);
    // As a toolkit's window selects them: the press then gives the source
    // the X server's implicit grab, so its motion and release reach it
    // wherever the pointer is.
    live.x.select(
        source_xid,
        EventMask::KEY_PRESS
            | EventMask::FOCUS_CHANGE
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION,
    );
    live.x.select(live.x.root, EventMask::SUBSTRUCTURE_NOTIFY);
    let other = XClient::connect(live.display);
    let target_xid = other.map(&Props::new(BLUE));
    let target_id = live.managed(target_xid);
    other.xdnd_aware(target_xid);
    let (from, to) = (live.placement(source_id), live.placement(target_id));
    visible("drag source", &from);
    visible("drop target", &to);
    let (tx, ty) = centre(to.rect);
    for start in [Start::Announced, Start::Unannounced, Start::ViaOwnWindow] {
        for order in [Order::SameStep, Order::Settled] {
            for resolve in [Resolve::Query, Resolve::Cache] {
                let case = Case {
                    start,
                    order,
                    resolve,
                };
                let rows: Vec<Outcome> = (0..iterations)
                    .map(|i| {
                        // No two motions alike.
                        #[allow(clippy::cast_precision_loss)]
                        let at = (tx + (i % 7) as f64, ty + (i % 5) as f64);
                        one(&mut live, from.rect, (&other, target_xid), at, case)
                    })
                    .collect();
                let count = |f: fn(&Outcome) -> bool| rows.iter().filter(|o| f(o)).count();
                eprintln!(
                    "RESULT start={start:?} order={order:?} resolve={resolve:?} n={} \
                     stale_at_motion={} found_proxy={} dropped_on_proxy={} target_got_drop={} \
                     grabbed_after={} proxy_alive_after={}",
                    rows.len(),
                    count(|o| o.stale_at_motion),
                    count(|o| o.found_proxy),
                    count(|o| o.dropped_on_proxy),
                    count(|o| o.target_got_drop),
                    count(|o| o.grabbed_after),
                    count(|o| o.proxy_alive_after),
                );
            }
        }
    }
}
