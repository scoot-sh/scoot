//! What a client's `wl_keyboard` is told when the window holding keyboard
//! focus is destroyed -- the connection staying up.
//!
//! The risk is a client left believing it still has focus (a `focused` flag
//! that never clears, a key repeat still armed) after the compositor dropped
//! focus without saying so. These drive a *real* `wayland-client` connection
//! through a real [`State`] and assert on the exact `enter`/`leave` sequence
//! the client received, plus the compositor's own idea of who holds focus.
//!
//! The whole `xdg_toplevel`, `xdg_surface`, `wl_surface` chain destroyed in
//! one burst is covered as well as the role object alone: the client has
//! already destroyed its `wl_surface` proxy by the time the compositor acts,
//! and the `leave` naming it must still arrive (it does: the server sends it
//! while handling the destroy, before the object id is retired; the test
//! client matches it through the proxy it kept).
//!
//! Windows are bare `xdg_toplevel`s with no buffer, as in
//! `foreign_toplevel/tests.rs`; a mapped toplevel takes focus without one.
//!
//! Like the other client-driven suites these need a writable
//! `$XDG_RUNTIME_DIR` ([`State::new`] binds a real listening socket).

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

const CANVAS: i32 = 200;

/// One `wl_keyboard` event, keyed by the creation order of the surface it
/// names (`u32::MAX` for a surface the client never made).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Enter(u32),
    Leave(u32),
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Every surface ever created, kept after destruction so a late event can
    /// still be matched to its position.
    surfaces: Vec<wl_surface::WlSurface>,
    configure: Vec<Option<u32>>,
    log: Vec<Seen>,
}

impl TestClient {
    fn key_of(&self, surface: &wl_surface::WlSurface) -> u32 {
        self.surfaces
            .iter()
            .position(|other| other == surface)
            .and_then(|index| u32::try_from(index).ok())
            .unwrap_or(u32::MAX)
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
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for TestClient {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
            && client.keyboard.is_none()
        {
            client.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                let key = client.key_of(&surface);
                client.log.push(Seen::Enter(key));
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                let key = client.key_of(&surface);
                client.log.push(Seen::Leave(key));
            }
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

struct WindowIndex(usize);

impl Dispatch<xdg_surface::XdgSurface, WindowIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &WindowIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(slot) = client.configure.get_mut(index.0)
        {
            *slot = Some(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);

enum Step {
    /// Map an `xdg_toplevel` and ack its configure.
    MapWindow,
    /// Destroy only the `xdg_toplevel` of window `n`, leaving its
    /// `xdg_surface` and `wl_surface` alive.
    DestroyToplevel(usize),
    /// Destroy the `xdg_toplevel` and `xdg_surface`, keeping the `wl_surface`
    /// (legal: a plain surface with no role object).
    DestroyRole(usize),
    /// Destroy the whole chain in order, in one burst, then round-trip.
    DestroyAll(usize),
    /// Destroy the rest of window `n`'s chain after an earlier
    /// [`Step::DestroyToplevel`].
    DestroyRemainder(usize),
    /// Hand back (and clear) every keyboard event seen so far.
    TakeLog,
}

enum Ack {
    Done,
    Log(Vec<Seen>),
}

type Window = (
    wl_surface::WlSurface,
    xdg_surface::XdgSurface,
    xdg_toplevel::XdgToplevel,
);

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    // Twice: the keyboard is only created from the seat's capabilities.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    client.keyboard.as_ref().ok_or("no wl_keyboard")?;
    let mut windows: Vec<Window> = Vec::new();

    while let Ok(step) = steps.recv() {
        let mut outcome = Ack::Done;
        match step {
            Step::MapWindow => {
                let surface = compositor.create_surface(&qh, ());
                client.surfaces.push(surface.clone());
                let index = client.configure.len();
                client.configure.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, WindowIndex(index));
                let toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                let serial = wait_for(&mut queue, &mut client, "a toplevel configure", |c| {
                    c.configure[index]
                })?;
                xdg.ack_configure(serial);
                windows.push((surface, xdg, toplevel));
            }
            Step::DestroyToplevel(n) => windows.get(n).ok_or("no such window")?.2.destroy(),
            Step::DestroyRole(n) => {
                let (_, xdg, toplevel) = windows.get(n).ok_or("no such window")?;
                toplevel.destroy();
                xdg.destroy();
            }
            Step::DestroyAll(n) => {
                let (surface, xdg, toplevel) = windows.get(n).ok_or("no such window")?;
                toplevel.destroy();
                xdg.destroy();
                surface.destroy();
            }
            Step::DestroyRemainder(n) => {
                let (surface, xdg, _) = windows.get(n).ok_or("no such window")?;
                xdg.destroy();
                surface.destroy();
            }
            Step::TakeLog => outcome = Ack::Log(std::mem::take(&mut client.log)),
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        if let Ack::Log(log) = &mut outcome {
            log.extend(std::mem::take(&mut client.log));
        }
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn new() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    fn take_log_on(&mut self, client: usize) -> Vec<Seen> {
        match self.run_on(client, Step::TakeLog) {
            Ack::Log(log) => log,
            Ack::Done => panic!("the client answered a log request with nothing"),
        }
    }

    /// Whether the compositor's seat keyboard has a focus at all.
    fn focused(&self) -> bool {
        self.state
            .seat
            .get_keyboard()
            .expect("a keyboard")
            .current_focus()
            .is_some()
    }
}

#[test]
fn the_only_window_gets_a_leave_when_its_toplevel_is_destroyed() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert_eq!(fixture.take_log_on(0), [Seen::Enter(0)]);
    assert!(fixture.focused());

    fixture.run(Step::DestroyToplevel(0));
    assert_eq!(fixture.take_log_on(0), [Seen::Leave(0)]);
    assert!(
        !fixture.focused(),
        "focus must be dropped, not left dangling"
    );

    // The rest of the chain going away afterwards is quiet.
    fixture.run(Step::DestroyRemainder(0));
    assert_eq!(fixture.take_log_on(0), []);
}

#[test]
fn the_only_window_gets_a_leave_when_the_role_objects_go_but_the_surface_stays() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.take_log_on(0);

    fixture.run(Step::DestroyRole(0));
    assert_eq!(fixture.take_log_on(0), [Seen::Leave(0)]);
    assert!(!fixture.focused());
}

#[test]
fn the_only_window_gets_a_leave_when_the_whole_chain_is_destroyed_at_once() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.take_log_on(0);

    fixture.run(Step::DestroyAll(0));
    assert_eq!(fixture.take_log_on(0), [Seen::Leave(0)]);
    assert!(!fixture.focused(), "focus must not outlive the surface");
}

#[test]
fn focus_moves_with_a_leave_then_an_enter_when_another_window_exists() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.take_log_on(0),
        [Seen::Enter(0), Seen::Leave(0), Seen::Enter(1)],
        "the newest window takes focus"
    );

    fixture.run(Step::DestroyToplevel(1));
    assert_eq!(
        fixture.take_log_on(0),
        [Seen::Leave(1), Seen::Enter(0)],
        "leave for the destroyed window, then enter for the survivor"
    );
    assert!(fixture.focused());
}

#[test]
fn focus_moves_with_a_leave_then_an_enter_when_the_whole_chain_is_destroyed_at_once() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.take_log_on(0);

    fixture.run(Step::DestroyAll(1));
    assert_eq!(fixture.take_log_on(0), [Seen::Leave(1), Seen::Enter(0)]);
    assert!(fixture.focused());
}

#[test]
fn a_leave_reaches_the_client_that_lost_focus_and_an_enter_the_one_that_gained_it() {
    let mut fixture = Fixture::new();
    let other = fixture.spawn(run_client);
    fixture.run(Step::MapWindow);
    fixture.run_on(other, Step::MapWindow);
    assert_eq!(fixture.take_log_on(0), [Seen::Enter(0), Seen::Leave(0)]);
    assert_eq!(fixture.take_log_on(other), [Seen::Enter(0)]);

    fixture.run_on(other, Step::DestroyToplevel(0));
    assert_eq!(fixture.take_log_on(other), [Seen::Leave(0)]);
    assert_eq!(fixture.take_log_on(0), [Seen::Enter(0)]);
    assert!(fixture.focused());
}
