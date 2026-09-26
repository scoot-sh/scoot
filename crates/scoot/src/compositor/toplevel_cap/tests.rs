//! Wire tests for the per-client toplevel cap (see `toplevel_cap.rs`).
//!
//! A real `wayland-client` opens real `xdg_toplevel`s on a real headless
//! compositor -- no buffers, no commits, the way a hostile client would: a
//! toplevel enters the core at `get_toplevel`, before the client has drawn
//! anything. Every test asserts on the count beside the wire: the client
//! that went past the cap is disconnected with `wl_display.no_memory`,
//! everything it held is released, and other clients are still served.
//!
//! Nothing here names `toplevel_cap::MAX_TOPLEVELS_PER_CLIENT` (the cap is
//! spelled out as [`CAP`]), so this file compiles against the code before
//! it, which is how its tests were watched failing first.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use wayland_client::protocol::{wl_callback, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::MAX_TOPLEVELS_PER_CLIENT;
use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// The most live toplevels one client may hold --
/// `toplevel_cap::MAX_TOPLEVELS_PER_CLIENT`, spelled out so this file does
/// not depend on it (see the module doc).
const CAP: u32 = 128;

/// `wl_display.error.no_memory`.
const NO_MEMORY: u32 = 2;

enum Step {
    /// Open `count` toplevels and keep them: no buffers, no commits.
    Open { count: u32 },
    /// Destroy every toplevel this client still holds.
    CloseAll,
}

enum Ack {
    Done,
}

type Fixture = Harness<Step, Ack>;

/// A headless compositor with one client connected.
fn start() -> Fixture {
    let mut fixture = Harness::headless_on(Appearance::default(), 64, RendererKind::Pixman);
    fixture.spawn(run_client);
    fixture
}

impl Fixture {
    fn open(&mut self, index: usize, count: u32) {
        match self.run_on(index, Step::Open { count }) {
            Ack::Done => {}
        }
    }

    fn live(&self, index: usize) -> u32 {
        self.state.toplevel_cap.live_for(&self.client(index).id())
    }
}

// ---------------------------------------------------------------------------
// The bound
// ---------------------------------------------------------------------------

/// The client opening past the cap is disconnected with
/// `wl_display.no_memory`, and everything it held -- its count and its
/// windows -- goes with it.
#[test]
fn a_toplevel_past_the_cap_disconnects_with_no_memory() {
    const BOUND: u32 = CAP;
    assert_eq!(
        MAX_TOPLEVELS_PER_CLIENT, BOUND,
        "the test's spelled-out cap drifted from the code's"
    );
    let mut fixture = start();
    fixture.open(0, BOUND);
    assert_eq!(fixture.live(0), BOUND);
    assert_eq!(fixture.state.toplevel_cap.in_flight(), BOUND);
    let error = fixture.run_expecting_disconnect(Step::Open { count: 1 });
    assert!(
        error.contains("wl_display")
            && error.contains(&format!("Protocol error {NO_MEMORY}"))
            && error.contains(&format!("at most {BOUND}")),
        "{error}"
    );
    assert_eq!(
        fixture.state.toplevel_cap.in_flight(),
        0,
        "the killed client's count drained"
    );
    assert!(
        fixture.state.world.windows().is_empty(),
        "and its windows left the core"
    );
}

/// Exactly the bound is allowed: the 128th toplevel maps and lives.
#[test]
fn exactly_the_bound_is_allowed() {
    let mut fixture = start();
    fixture.open(0, CAP);
    assert_eq!(fixture.live(0), CAP);
    assert_eq!(fixture.state.world.windows().len(), CAP as usize);
}

/// Per client: one client at its bound does not stop another from opening,
/// and one client's closes never touch another's count.
#[test]
fn the_bound_is_per_client() {
    let mut fixture = start();
    fixture.spawn(run_client);
    fixture.open(0, CAP);
    fixture.open(1, 5);
    assert_eq!(fixture.live(0), CAP, "the first client kept its count");
    assert_eq!(fixture.live(1), 5, "the second client opened beside it");
    assert_eq!(fixture.state.toplevel_cap.in_flight(), CAP + 5);
    match fixture.run_on(1, Step::CloseAll) {
        Ack::Done => {}
    }
    assert_eq!(fixture.live(1), 0);
    assert_eq!(
        fixture.live(0),
        CAP,
        "the other's closes left this count alone"
    );
}

/// Closing windows hands the bound back: a client that filled it, closed
/// everything, and filled it again is served throughout.
#[test]
fn closed_windows_release_the_bound() {
    let mut fixture = start();
    fixture.open(0, CAP);
    match fixture.run(Step::CloseAll) {
        Ack::Done => {}
    }
    assert_eq!(fixture.live(0), 0);
    assert!(fixture.state.world.windows().is_empty());
    fixture.open(0, CAP);
    assert_eq!(fixture.live(0), CAP);
}

// ---------------------------------------------------------------------------
// The X half: the counter `map_x11_window` claims against, driven directly.
// (The wire half -- a real X client mapping past the cap -- is the live
// `xwayland/tests/cap.rs` suite: it needs a real XWayland server.)
//
// Gated on the feature with the counter itself: a default build carries no
// X code at all, and this section names nothing else.
// ---------------------------------------------------------------------------

#[cfg(feature = "xwayland")]
mod x11 {
    use scoot_core::WindowId;

    use super::super::{MAX_X11_TOPLEVELS_PER_CLIENT, X11ToplevelCap};

    /// Two X clients, as window-id client bits (see `xwayland/focus.rs`):
    /// the high bits differ, the low 21 do not matter here.
    const X_A: u32 = 0x0040_0000;
    const X_B: u32 = 0x0060_0000;

    /// The X cap is the xdg cap's number, spelled out so this file does not
    /// depend on it (like [`CAP`] above): the two bounds move together only
    /// by an explicit decision, never by sharing the constant.
    const X_CAP: u32 = 128;

    fn x_cap() -> X11ToplevelCap {
        assert_eq!(
            MAX_X11_TOPLEVELS_PER_CLIENT, X_CAP,
            "the test's spelled-out X cap drifted from the code's"
        );
        X11ToplevelCap::default()
    }

    /// Minting fresh core ids, the way `map_x11_window`'s `next_id` does.
    struct Ids(u64);

    impl Ids {
        fn next(&mut self) -> WindowId {
            self.0 += 1;
            WindowId(self.0)
        }
    }

    /// A full client is refused nothing silently: `admits` says no at
    /// exactly the bound, and the refusal claims nothing.
    #[test]
    fn the_x_bound_refuses_at_exactly_the_cap() {
        let mut cap = x_cap();
        let mut ids = Ids(0);
        for _ in 0..X_CAP {
            assert!(cap.admits(&X_A));
            cap.claim(X_A, ids.next());
        }
        assert_eq!(cap.live_for(&X_A), X_CAP);
        assert_eq!(cap.in_flight(), X_CAP);
        assert!(!cap.admits(&X_A), "the 129th window was admitted");
        assert_eq!(cap.live_for(&X_A), X_CAP, "the refusal claimed a unit");
    }

    /// Per X client: one client at its bound does not stop another, and one
    /// client's releases never touch another's count.
    #[test]
    fn the_x_bound_is_per_client() {
        let mut cap = x_cap();
        let mut ids = Ids(0);
        for _ in 0..X_CAP {
            cap.claim(X_A, ids.next());
        }
        for _ in 0..5 {
            assert!(cap.admits(&X_B));
            cap.claim(X_B, ids.next());
        }
        assert_eq!(cap.live_for(&X_A), X_CAP);
        assert_eq!(cap.live_for(&X_B), 5);
        assert_eq!(cap.in_flight(), X_CAP + 5);
    }

    /// Releasing hands the bound back: unmap (or death, or withdrawal --
    /// every one reaches `remove_window`) frees the unit, and a drained
    /// client leaves no entry behind.
    #[test]
    fn x_releases_free_the_bound_and_leave_no_entry() {
        let mut cap = x_cap();
        let mut ids = Ids(0);
        let mut held = Vec::new();
        for _ in 0..X_CAP {
            let id = ids.next();
            cap.claim(X_A, id);
            held.push(id);
        }
        assert!(!cap.admits(&X_A));
        cap.release(&held[0]);
        assert!(cap.admits(&X_A), "an unmap did not free the bound");
        assert_eq!(cap.live_for(&X_A), X_CAP - 1);
        for id in &held[1..] {
            cap.release(id);
        }
        assert_eq!(cap.live_for(&X_A), 0);
        assert_eq!(cap.in_flight(), 0, "a drained client left an entry behind");
    }

    /// The release is idempotent: a refused window (never claimed), an
    /// `xdg_toplevel` (claimed elsewhere), and a double release all change
    /// nothing.
    #[test]
    fn x_release_is_idempotent() {
        let mut cap = x_cap();
        let mut ids = Ids(0);
        cap.release(&ids.next());
        assert_eq!(cap.in_flight(), 0, "an unknown id claimed a unit");
        let id = ids.next();
        cap.claim(X_A, id);
        cap.release(&id);
        cap.release(&id);
        assert_eq!(cap.live_for(&X_A), 0, "a double release went negative");
        assert_eq!(cap.in_flight(), 0);
    }
}

// ---------------------------------------------------------------------------
// The X unmanaged half: the counter `map_x11_unmanaged` claims against,
// driven directly. (The wire half -- a real X client mapping past the cap
// -- is the live `xwayland/tests/unmanaged_cap.rs` suite: it needs a real
// XWayland server.)
//
// Gated on the feature with the counter itself: a default build carries no
// X code at all, and this section names nothing else.
// ---------------------------------------------------------------------------

#[cfg(feature = "xwayland")]
mod x11_unmanaged {
    use super::super::{MAX_X11_UNMANAGED_PER_CLIENT, X11UnmanagedCap};

    /// Two X clients, as window-id client bits (see `xwayland/focus.rs`):
    /// the high bits differ, the low 21 do not matter here.
    const X_A: u32 = 0x0040_0000;
    const X_B: u32 = 0x0060_0000;

    /// The unmanaged cap is the managed cap's number, spelled out so this
    /// file does not depend on it (like [`X_CAP`]'s section above): the
    /// bounds move together only by an explicit decision, never by sharing
    /// the constant.
    const X_OR_CAP: u32 = 128;

    fn or_cap() -> X11UnmanagedCap {
        assert_eq!(
            MAX_X11_UNMANAGED_PER_CLIENT, X_OR_CAP,
            "the test's spelled-out unmanaged cap drifted from the code's"
        );
        X11UnmanagedCap::default()
    }

    /// Fresh X window ids for one client, the way the server mints them:
    /// the client bits fixed, the low bits rising.
    struct Xids(u32);

    impl Xids {
        fn next(&mut self, client: u32) -> u32 {
            self.0 += 1;
            client | self.0
        }
    }

    /// A full client is refused nothing silently: `admits` says no at
    /// exactly the bound, and the refusal claims nothing.
    #[test]
    fn the_unmanaged_bound_refuses_at_exactly_the_cap() {
        let mut cap = or_cap();
        let mut xids = Xids(0);
        for _ in 0..X_OR_CAP {
            assert!(cap.admits(&X_A));
            cap.claim(X_A, xids.next(X_A));
        }
        assert_eq!(cap.live_for(&X_A), X_OR_CAP);
        assert_eq!(cap.in_flight(), X_OR_CAP);
        assert!(!cap.admits(&X_A), "the 129th menu was admitted");
        assert_eq!(cap.live_for(&X_A), X_OR_CAP, "the refusal claimed a unit");
    }

    /// Per X client: one client at its bound does not stop another, and one
    /// client's releases never touch another's count. Managed windows never
    /// touch this count either -- the two caps share no unit -- which the
    /// live suite pins beside the real map paths.
    #[test]
    fn the_unmanaged_bound_is_per_client() {
        let mut cap = or_cap();
        let mut xids = Xids(0);
        for _ in 0..X_OR_CAP {
            cap.claim(X_A, xids.next(X_A));
        }
        for _ in 0..5 {
            assert!(cap.admits(&X_B));
            cap.claim(X_B, xids.next(X_B));
        }
        assert_eq!(cap.live_for(&X_A), X_OR_CAP);
        assert_eq!(cap.live_for(&X_B), 5);
        assert_eq!(cap.in_flight(), X_OR_CAP + 5);
    }

    /// Releasing hands the bound back: an unmap (or the destroy after it --
    /// both reach `unmap_x11_unmanaged`) frees the unit, and a drained
    /// client leaves no entry behind.
    #[test]
    fn unmanaged_releases_free_the_bound_and_leave_no_entry() {
        let mut cap = or_cap();
        let mut xids = Xids(0);
        let mut held = Vec::new();
        for _ in 0..X_OR_CAP {
            let xid = xids.next(X_A);
            cap.claim(X_A, xid);
            held.push(xid);
        }
        assert!(!cap.admits(&X_A));
        cap.release(&held[0]);
        assert!(cap.admits(&X_A), "an unmap did not free the bound");
        assert_eq!(cap.live_for(&X_A), X_OR_CAP - 1);
        for xid in &held[1..] {
            cap.release(xid);
        }
        assert_eq!(cap.live_for(&X_A), 0);
        assert_eq!(cap.in_flight(), 0, "a drained client left an entry behind");
    }

    /// The release is idempotent: a refused menu (never claimed) and a
    /// double release both change nothing.
    #[test]
    fn unmanaged_release_is_idempotent() {
        let mut cap = or_cap();
        cap.release(&X_A);
        assert_eq!(cap.in_flight(), 0, "an unknown id claimed a unit");
        let mut xids = Xids(0);
        let xid = xids.next(X_A);
        cap.claim(X_A, xid);
        cap.release(&xid);
        cap.release(&xid);
        assert_eq!(cap.live_for(&X_A), 0, "a double release went negative");
        assert_eq!(cap.in_flight(), 0);
    }

    /// The server's death drains everything at once: `clear` forgets both
    /// clients' claims, so a restarted server reusing window ids starts
    /// clean.
    #[test]
    fn the_server_dying_clears_the_whole_count() {
        let mut cap = or_cap();
        let mut xids = Xids(0);
        for _ in 0..7 {
            cap.claim(X_A, xids.next(X_A));
        }
        for _ in 0..3 {
            cap.claim(X_B, xids.next(X_B));
        }
        cap.clear();
        assert_eq!(cap.live_for(&X_A), 0);
        assert_eq!(cap.live_for(&X_B), 0);
        assert_eq!(cap.in_flight(), 0);
        assert!(cap.admits(&X_A));
    }
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    /// Whether the newest `wl_display.sync` has been answered.
    synced: bool,
}

struct Made {
    surfaces: Vec<wl_surface::WlSurface>,
    xdgs: Vec<xdg_surface::XdgSurface>,
    toplevels: Vec<xdg_toplevel::XdgToplevel>,
}

impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            client.synced = true;
        }
    }
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
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClient {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore xdg_surface::XdgSurface);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);

/// A flush of its own first, then the round trip's `sync` in a second
/// write, races the refusal: the compositor can kill the client and close
/// the socket in between, and the client then sees `EPIPE` on that second
/// write instead of the protocol error waiting in its receive buffer.
/// (The same recipe as `popup_parent`'s `sync`.)
fn sync(
    conn: &Connection,
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
) -> Result<(), String> {
    client.synced = false;
    conn.display().sync(&queue.handle(), ());
    loop {
        match conn.flush() {
            Ok(()) => break,
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    while !client.synced {
        queue.blocking_dispatch(client).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;

    let mut made = Made {
        surfaces: Vec::new(),
        xdgs: Vec::new(),
        toplevels: Vec::new(),
    };
    while let Ok(step) = steps.recv() {
        match step {
            Step::Open { count } => {
                for _ in 0..count {
                    let surface = compositor.create_surface(&qh, ());
                    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                    let toplevel = xdg.get_toplevel(&qh, ());
                    made.surfaces.push(surface);
                    made.xdgs.push(xdg);
                    made.toplevels.push(toplevel);
                }
                // One round trip after the whole burst, so a kill is
                // reported with how far it got -- and the kill, when it
                // comes, surfaces here as the protocol error, not an
                // `EPIPE` on a second write (see `sync`).
                sync(&conn, &mut queue, &mut client)?;
            }
            Step::CloseAll => {
                // Explicit destroys, child before parent: dropping a proxy
                // does not tell the server.
                for toplevel in made.toplevels.drain(..) {
                    toplevel.destroy();
                }
                for xdg in made.xdgs.drain(..) {
                    xdg.destroy();
                }
                for surface in made.surfaces.drain(..) {
                    surface.destroy();
                }
                sync(&conn, &mut queue, &mut client)?;
            }
        }
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}
