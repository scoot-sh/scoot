//! Hiding the pointer after inactivity under a fullscreen window
//! (`[appearance] cursor_hide_after_ms`).
//!
//! On a CRTC with no cursor plane, a visible pointer denies every
//! fullscreen window a primary-direct attempt, so the session can hide its
//! own pointer while a fullscreen window covers the output the pointer is
//! on. These tests drive it headlessly through a real client: map a
//! window, fullscreen it, park the pointer on it, and advance a synthetic
//! clock through `update_cursor_hide` / `note_cursor_hide_timeout` rather
//! than sleeping for the configured delay.

use std::fs;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use scoot_core::Action;
use scoot_ipc::PointerButton;
use smithay::backend::input::{TabletToolCapabilities, TabletToolDescriptor, TabletToolType};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use xdg_positioner::{Anchor, Gravity};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{self, Harness};

mod bench;

/// The headless canvas: one 200x200 output at the origin.
const CANVAS: i32 = 200;
/// The pointer's parking spot: the middle of the output, over the
/// fullscreen window wherever one covers it.
const CENTER: (f64, f64) = (100.0, 100.0);
/// The configured hide delay every test uses.
const TIMEOUT: Duration = Duration::from_millis(2_000);
/// The window's color, as BGRA bytes: unmistakable against the white
/// cursor and the dark background alike.
const WINDOW_BGRA: [u8; 4] = [0x20, 0x20, 0xE0, 0xFF];

/// A cursor pixel: the fallback arrow's fill is white, and (101, 103) is
/// inside the fill (x < y, past the 1px outline) with the hotspot at the
/// pointer.
const CURSOR_AT: (i32, i32) = (101, 103);
const CURSOR_BGRA: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// Steps the test client performs.
enum Step {
    /// Map a toplevel and draw `WINDOW_BGRA` at its configured size.
    MapWindow,
    /// `set_fullscreen` on the `window`-th toplevel and ack the answer.
    SetFullscreen { window: usize },
    /// `unset_fullscreen` on the `window`-th toplevel and ack the answer.
    UnsetFullscreen { window: usize },
    /// Destroy the `window`-th toplevel's objects outright.
    DestroyWindow { window: usize },
    /// Take an `ext_session_lock_v1`, wait for the compositor's `locked`
    /// (an unlock before confirmation is a protocol error), and hold it.
    Lock,
    /// Release the held lock.
    Unlock,
    /// Open an `xdg_popup` on the `window`-th toplevel, over the pointer.
    MakePopup { window: usize },
    /// Destroy the popup again.
    DestroyPopup,
}

enum Ack {
    Done,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    pending: Vec<(i32, i32)>,
    configures: Vec<Vec<(u32, i32, i32)>>,
    acked: Vec<Option<u32>>,
    /// Per popup, by creation order: the `xdg_surface.configure` serials
    /// waiting to be acked.
    popup_configures: Vec<Vec<u32>>,
    /// `locked` events seen, cumulative: what the Lock step waits for
    /// before answering, since `unlock_and_destroy` is a protocol error on
    /// a lock the compositor has not confirmed yet.
    locked: u32,
    /// ...and `finished`, which is how a refusal arrives. Never expected
    /// here (nothing else locks), but waited on all the same: hanging to
    /// `PATIENCE` on a refusal would say nothing, while the count tells
    /// which of the two arrived.
    finished: u32,
    lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
}

struct Index(usize);

/// Userdata for popup `xdg_surface`s: their configures are tracked
/// separately from the toplevels', which carry sizes alongside.
struct PIndex(usize);

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
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "ext_session_lock_manager_v1" => {
                client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
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

impl Dispatch<xdg_toplevel::XdgToplevel, Index> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &Index,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event
            && let Some(pending) = client.pending.get_mut(index.0)
        {
            *pending = (width, height);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, Index> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &Index,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(pending) = client.pending.get(index.0).copied()
            && let Some(seen) = client.configures.get_mut(index.0)
        {
            seen.push((serial, pending.0, pending.1));
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, PIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &PIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(seen) = client.popup_configures.get_mut(index.0)
        {
            seen.push(serial);
        }
    }
}

wayland_client::delegate_noop!(
    TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1
);

/// `locked`/`finished` are counted, not ignored: the Lock step waits for
/// one of the two (the protocol: "the compositor must send either the
/// locked or finished event"), so an unlock never races the confirmation
/// -- unlocking an unconfirmed lock is `InvalidUnlock`, which kills the
/// client and fails the test as a confusing disconnect under load.
impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => client.locked += 1,
            ext_session_lock_v1::Event::Finished => client.finished += 1,
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);

/// Popup events are all server-to-client notices the script never acts on
/// (configures arrive on the popup's `xdg_surface`, acked there; the
/// destroy is scripted, not announced).
impl Dispatch<xdg_popup::XdgPopup, PIndex> for TestClient {
    fn event(
        _: &mut Self,
        _: &xdg_popup::XdgPopup,
        _: xdg_popup::Event,
        _: &PIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// One toplevel the script made.
struct Toplevel {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

/// One popup the script made: held for the run, since dropping its
/// objects dismisses it.
struct PopupParts {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    popup: xdg_popup::XdgPopup,
}

fn wait_for_popup_configure(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    index: usize,
    seen: usize,
) -> Result<u32, String> {
    test_support::wait_for(queue, client, "a popup configure", |client| {
        let all = client.popup_configures.get(index)?;
        (all.len() > seen).then(|| all.last().copied()).flatten()
    })
}

/// Ack the popup's configure and draw it solid, so it maps and takes the
/// hit test where it lands.
fn draw_popup(
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    surface: &wl_surface::WlSurface,
    xdg: &xdg_surface::XdgSurface,
    serial: u32,
) -> Result<(), String> {
    xdg.ack_configure(serial);
    let (width, height) = (40, 40);
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-cursor-hide-popup", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = WINDOW_BGRA.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, width, height);
    surface.commit();
    Ok(())
}

fn wait_for_configure(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    window: usize,
    seen: usize,
) -> Result<(u32, i32, i32), String> {
    test_support::wait_for(queue, client, "a toplevel configure", |client| {
        let all = client.configures.get(window)?;
        (all.len() > seen).then(|| all.last().copied()).flatten()
    })
}

#[allow(clippy::too_many_arguments)]
fn ack_and_draw(
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    window: &Toplevel,
    index: usize,
    serial: u32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    if client.acked.get(index).copied().flatten() != Some(serial) {
        window.xdg.ack_configure(serial);
        if let Some(slot) = client.acked.get_mut(index) {
            *slot = Some(serial);
        }
    }
    let width = if width > 0 { width } else { 40 };
    let height = if height > 0 { height } else { 40 };
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-cursor-hide-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = WINDOW_BGRA.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    window.surface.attach(Some(&buffer), 0, 0);
    window.surface.damage(0, 0, width, height);
    window.surface.commit();
    Ok(())
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;

    let mut windows: Vec<Toplevel> = Vec::new();
    let mut popups: Vec<PopupParts> = Vec::new();
    while let Ok(step) = steps.recv() {
        match step {
            Step::MapWindow => {
                let index = windows.len();
                client.pending.push((0, 0));
                client.configures.push(Vec::new());
                client.acked.push(None);
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Index(index));
                let toplevel = xdg.get_toplevel(&qh, Index(index));
                surface.commit();
                let (serial, width, height) =
                    wait_for_configure(&mut queue, &mut client, index, 0)?;
                let window = Toplevel {
                    surface,
                    xdg,
                    toplevel,
                };
                ack_and_draw(
                    &mut client,
                    &qh,
                    &shm,
                    &window,
                    index,
                    serial,
                    width,
                    height,
                )?;
                windows.push(window);
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
            Step::SetFullscreen { window } => {
                let seen = client.configures[window].len();
                windows[window].toplevel.set_fullscreen(None);
                let (serial, width, height) =
                    wait_for_configure(&mut queue, &mut client, window, seen)?;
                ack_and_draw(
                    &mut client,
                    &qh,
                    &shm,
                    &windows[window],
                    window,
                    serial,
                    width,
                    height,
                )?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
            Step::UnsetFullscreen { window } => {
                let seen = client.configures[window].len();
                windows[window].toplevel.unset_fullscreen();
                let (serial, width, height) =
                    wait_for_configure(&mut queue, &mut client, window, seen)?;
                ack_and_draw(
                    &mut client,
                    &qh,
                    &shm,
                    &windows[window],
                    window,
                    serial,
                    width,
                    height,
                )?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
            Step::DestroyWindow { window } => {
                let window = &windows[window];
                window.toplevel.destroy();
                window.xdg.destroy();
                window.surface.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
            Step::Lock => {
                let seen = client.locked + client.finished;
                let manager = client
                    .lock_manager
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1")?;
                let lock = manager.lock(&qh, ());
                client.lock = Some(lock);
                // Exactly one of the two must arrive before anything may
                // unlock: waiting (while dispatching, so the confirming
                // blanked frame gets to present) is what keeps this step
                // load-proof, where a bare round trip confirmed only when
                // the tick happened to run inside it.
                test_support::wait_for(&mut queue, &mut client, "locked or finished", |client| {
                    (client.locked + client.finished > seen).then_some(())
                })?;
            }
            Step::Unlock => {
                let lock = client.lock.take().ok_or("no lock held")?;
                lock.unlock_and_destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
            Step::MakePopup { window } => {
                let index = popups.len();
                client.popup_configures.push(Vec::new());
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, PIndex(index));
                // A 40x40 menu at (80, 80) in the parent's geometry,
                // growing right and down over the parked pointer (100, 100).
                let positioner = wm_base.create_positioner(&qh, ());
                positioner.set_size(40, 40);
                positioner.set_anchor_rect(80, 80, 1, 1);
                positioner.set_anchor(Anchor::TopLeft);
                positioner.set_gravity(Gravity::BottomRight);
                let popup =
                    xdg.get_popup(Some(&windows[window].xdg), &positioner, &qh, PIndex(index));
                surface.commit();
                let serial = wait_for_popup_configure(&mut queue, &mut client, index, 0)?;
                draw_popup(&qh, &shm, &surface, &xdg, serial)?;
                popups.push(PopupParts {
                    surface,
                    xdg,
                    popup,
                });
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
            Step::DestroyPopup => {
                let popup = popups.pop().ok_or("no popup open")?;
                popup.popup.destroy();
                popup.xdg.destroy();
                popup.surface.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
            }
        }
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// A headless compositor with the hide delay set, one connected client,
    /// and headless frames drawing the cursor the way every `--tty` frame
    /// on the dumb tier does.
    fn hide_after() -> Self {
        let appearance = Appearance {
            cursor_hide_after_ms: TIMEOUT.as_millis() as u64,
            ..Appearance::default()
        };
        let mut fixture = Harness::headless(appearance, CANVAS);
        fixture.state.frame_cursor_for_test = Some(true);
        fixture.spawn(run_client);
        fixture
    }

    /// Map one window and cover the output with it.
    fn cover_output(&mut self) {
        self.run(Step::MapWindow);
        self.run(Step::SetFullscreen { window: 0 });
        self.state.pointer_move(CENTER.0, CENTER.1);
    }

    /// Park the pointer on the focused window's drawn pixels: ten pixels
    /// inside its space bounding box's origin, which the committed buffer
    /// covers (a client that drew short of its configure only takes input
    /// where it drew). Returns the integer parking spot, for pixel
    /// assertions.
    fn park_on_focused_window(&mut self) -> (i32, i32) {
        let id = self.state.world.focused_window().expect("a focused window");
        let window = self.state.window(id).expect("the focused window");
        let bbox = self
            .state
            .space
            .element_bbox(window)
            .expect("the window is placed");
        let at = (bbox.loc.x + 10, bbox.loc.y + 10);
        self.state.pointer_move(f64::from(at.0), f64::from(at.1));
        at
    }

    /// Whether the compositor currently hides its pointer for inactivity.
    fn hidden(&self) -> bool {
        self.state.cursor_idle_hidden
    }

    /// Arm the hide at `now`, then fire the timer at `now + TIMEOUT`.
    fn hide_at(&mut self, now: Instant) {
        self.state.update_cursor_hide(now);
        assert!(!self.hidden(), "arming must not hide at once");
        self.state.note_cursor_hide_timeout(now + TIMEOUT);
    }

    /// The BGRA pixel at `at` of a freshly rendered primary-output frame.
    fn pixel_at(&mut self, at: (i32, i32)) -> [u8; 4] {
        let pixels = self.render();
        let i = ((at.1 * CANVAS + at.0) * 4) as usize;
        pixels[i..i + 4].try_into().expect("inside the frame")
    }
}

#[test]
fn hide_fires_after_timeout_with_a_synthetic_clock() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    let now = Instant::now();
    fixture.hide_at(now);
    assert!(fixture.hidden(), "the timer firing at the deadline hides");
}

#[test]
fn firing_before_the_deadline_hides_nothing_and_re_arms() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    let now = Instant::now();
    fixture.state.update_cursor_hide(now);
    fixture
        .state
        .note_cursor_hide_timeout(now + TIMEOUT - Duration::from_millis(1));
    assert!(!fixture.hidden(), "an early firing must not hide");
    assert!(
        fixture.state.cursor_hide_timer_live,
        "the early firing re-arms for the remainder"
    );
    fixture.state.note_cursor_hide_timeout(now + TIMEOUT);
    assert!(fixture.hidden(), "the deadline still hides afterwards");
}

#[test]
fn motion_during_the_timeout_race_pushes_the_deadline() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    let now = Instant::now();
    fixture.state.update_cursor_hide(now);
    // A motion just before the deadline: the firing scheduled for `now +
    // TIMEOUT` must see the pushed deadline and re-arm instead of hiding.
    fixture.state.pointer_move(CENTER.0 + 1.0, CENTER.1);
    fixture.state.note_cursor_hide_timeout(now + TIMEOUT);
    assert!(!fixture.hidden(), "motion won the race: no hide");
    assert!(
        fixture.state.cursor_hide_timer_live,
        "the timer re-armed for the pushed deadline"
    );
}

#[test]
fn motion_reshows_and_marks_a_frame() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.hide_at(Instant::now());
    assert!(fixture.hidden());
    fixture.state.needs_render = false;
    fixture.state.pointer_move(CENTER.0 + 5.0, CENTER.1);
    assert!(!fixture.hidden(), "motion shows the pointer again");
    assert!(
        fixture.state.needs_render,
        "the reshown pointer needs its frame"
    );
}

#[test]
fn button_and_scroll_reshow() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.hide_at(Instant::now());
    fixture.state.pointer_button(PointerButton::Left, true);
    assert!(!fixture.hidden(), "a press shows the pointer again");
    fixture.hide_at(Instant::now());
    fixture.state.pointer_button(PointerButton::Left, false);
    assert!(!fixture.hidden(), "a release shows the pointer again");
    fixture.hide_at(Instant::now());
    fixture.state.scroll(0.0, 1.0);
    assert!(!fixture.hidden(), "a scroll shows the pointer again");
}

#[test]
fn the_reshown_pointer_is_in_the_frame() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    assert_eq!(
        fixture.pixel_at(CURSOR_AT),
        CURSOR_BGRA,
        "the parked pointer draws before the hide"
    );
    fixture.hide_at(Instant::now());
    assert_eq!(
        fixture.pixel_at(CURSOR_AT),
        WINDOW_BGRA,
        "a hidden pointer leaves the window's own pixels"
    );
    fixture.state.pointer_move(CENTER.0, CENTER.1);
    assert_eq!(
        fixture.pixel_at(CURSOR_AT),
        CURSOR_BGRA,
        "the next motion draws the pointer again, so that frame composites"
    );
}

#[test]
fn no_fullscreen_window_never_hides() {
    let mut fixture = Fixture::hide_after();
    fixture.run(Step::MapWindow);
    // Over the tiled window's own pixels, not the bare desktop: the
    // configure may size it short of the output (and the test client draws
    // a 40x40 fallback at a zero size), so the output center is not
    // reliably on the window.
    let at = fixture.park_on_focused_window();
    let now = Instant::now();
    fixture.state.update_cursor_hide(now);
    fixture.state.note_cursor_hide_timeout(now + TIMEOUT);
    assert!(!fixture.hidden(), "a tiled window is not a cover");
    assert_eq!(
        fixture.pixel_at((at.0 + 1, at.1 + 3)),
        CURSOR_BGRA,
        "the pointer keeps drawing over the tiled window"
    );
}

#[test]
fn a_pointer_on_an_uncovered_output_never_hides() {
    let mut fixture = Fixture::hide_after();
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.cover_output();
    // A second, tiled window on the second output: windows open on the
    // pointer's output, so move there first, then park on its pixels.
    fixture.state.pointer_move(f64::from(CANVAS) + 10.0, 10.0);
    fixture.run(Step::MapWindow);
    let at = fixture.park_on_focused_window();
    let now = Instant::now();
    fixture.state.update_cursor_hide(now);
    assert!(
        fixture.state.cursor_hide_deadline.is_none(),
        "no cover under the pointer arms nothing"
    );
    fixture.state.note_cursor_hide_timeout(now + TIMEOUT);
    assert!(!fixture.hidden(), "only the covered output hides");
    // The second output's own framebuffer: the primary's never draws a
    // pointer parked on the other screen.
    let second = fixture
        .state
        .outputs
        .iter_with_ids()
        .nth(1)
        .map(|(id, _)| id)
        .expect("a second output");
    fixture.state.request_render();
    fixture.state.render();
    let pixels = fixture.pixels_of(second);
    let local = (at.0 + 1 - CANVAS, at.1 + 3);
    let i = ((local.1 * CANVAS + local.0) * 4) as usize;
    assert_eq!(
        pixels[i..i + 4],
        CURSOR_BGRA,
        "the pointer keeps drawing on the uncovered output"
    );
}

#[test]
fn leaving_fullscreen_disarms() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.hide_at(Instant::now());
    fixture.run(Step::UnsetFullscreen { window: 0 });
    assert!(!fixture.hidden(), "un-fullscreen shows the pointer again");
    assert!(
        fixture.state.cursor_hide_deadline.is_none(),
        "and disarms the timer"
    );
}

#[test]
fn closing_the_covering_window_disarms() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.hide_at(Instant::now());
    fixture.run(Step::DestroyWindow { window: 0 });
    assert!(
        !fixture.hidden(),
        "closing the cover shows the pointer again"
    );
    assert!(
        fixture.state.cursor_hide_deadline.is_none(),
        "and disarms the timer"
    );
}

#[test]
fn switching_workspace_disarms() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.hide_at(Instant::now());
    fixture.state.act(Action::FocusWorkspaceIndex(1));
    fixture.settle();
    let primary = fixture.state.outputs.primary_id().expect("an output");
    assert!(
        fixture.state.world.fullscreen_on(primary).is_none(),
        "the cover stays on the old workspace"
    );
    assert!(!fixture.hidden(), "the switch shows the pointer again");
}

#[test]
fn locking_disarms_and_never_hides() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.hide_at(Instant::now());
    fixture.run(Step::Lock);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the test really locked the session"
    );
    assert!(!fixture.hidden(), "locking shows the pointer again");
    let now = Instant::now();
    fixture.state.update_cursor_hide(now);
    fixture.state.note_cursor_hide_timeout(now + TIMEOUT);
    assert!(!fixture.hidden(), "the lock screen keeps its pointer");
}

#[test]
fn unlocking_re_arms_over_a_cover() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.run(Step::Lock);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the test really locked the session before unlocking it"
    );
    fixture.run(Step::Unlock);
    assert!(
        !fixture.state.session_lock.is_locked(),
        "the test really unlocked the session"
    );
    let now = Instant::now();
    assert!(
        fixture.state.cursor_hide_deadline.is_some(),
        "unlocking re-arms over the still-covering window"
    );
    fixture.state.note_cursor_hide_timeout(now + TIMEOUT);
    assert!(fixture.hidden(), "the cover hides again after unlock");
}

#[test]
fn a_zero_timeout_is_a_disabled_feature() {
    let appearance = Appearance::default();
    assert_eq!(appearance.cursor_hide_after_ms, 0, "off unless configured");
    let mut fixture = Harness::headless(appearance, CANVAS);
    fixture.state.frame_cursor_for_test = Some(true);
    fixture.spawn(run_client);
    fixture.cover_output();
    let now = Instant::now();
    fixture.state.update_cursor_hide(now);
    assert!(
        fixture.state.cursor_hide_deadline.is_none(),
        "zero arms nothing"
    );
    assert!(
        !fixture.state.cursor_hide_timer_live,
        "zero inserts no timer"
    );
    fixture.state.note_cursor_hide_timeout(now + TIMEOUT);
    assert!(!fixture.hidden(), "zero never hides");
    fixture.state.pointer_move(CENTER.0 + 1.0, CENTER.1);
    assert!(
        !fixture.state.cursor_hide_timer_live,
        "motion inserts no timer while off"
    );
}

/// The pen every barrel-button test drives. It never enters proximity,
/// so the tool half stays silent and the test witnesses only the
/// activity reset.
fn pen() -> TabletToolDescriptor {
    TabletToolDescriptor {
        tool_type: TabletToolType::Pen,
        hardware_serial: 42,
        hardware_id_wacom: 0,
        capabilities: TabletToolCapabilities::PRESSURE,
    }
}

#[test]
fn barrel_button_reshows() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    fixture.hide_at(Instant::now());
    assert!(fixture.hidden());
    fixture.state.tablet_button(&pen(), 0, true);
    assert!(!fixture.hidden(), "a barrel press shows the pointer again");
    fixture.hide_at(Instant::now());
    fixture.state.tablet_button(&pen(), 0, false);
    assert!(
        !fixture.hidden(),
        "a barrel release shows the pointer again"
    );
}

#[test]
fn retime_shortens_an_armed_delay() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    let t0 = Instant::now();
    fixture.state.update_cursor_hide(t0);
    // Already armed by the parking motion's own activity reset, under the
    // configured delay; `update` does not move an armed deadline.
    assert!(
        fixture.state.cursor_hide_deadline.is_some(),
        "armed over the cover"
    );
    // Lengthening first, so the shorten below proves the armed deadline
    // moves rather than the test arming fresh.
    fixture.state.appearance.cursor_hide_after_ms = 10_000;
    fixture.state.retime_cursor_hide(t0);
    assert_eq!(
        fixture.state.cursor_hide_deadline,
        Some(t0 + Duration::from_secs(10)),
        "lengthening pushes the armed deadline out"
    );
    fixture.state.appearance.cursor_hide_after_ms = 1_000;
    fixture.state.retime_cursor_hide(t0);
    assert_eq!(
        fixture.state.cursor_hide_deadline,
        Some(t0 + Duration::from_secs(1)),
        "shortening pulls the armed deadline in"
    );
    fixture
        .state
        .note_cursor_hide_timeout(t0 + Duration::from_secs(1));
    assert!(fixture.hidden(), "the shortened delay hides");
}

#[test]
fn retime_to_zero_disarms() {
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    let t0 = Instant::now();
    fixture.state.update_cursor_hide(t0);
    assert!(fixture.state.cursor_hide_deadline.is_some());
    fixture.state.appearance.cursor_hide_after_ms = 0;
    fixture.state.retime_cursor_hide(t0);
    assert!(
        fixture.state.cursor_hide_deadline.is_none(),
        "disabling clears the armed deadline"
    );
    assert!(!fixture.hidden());
}

#[test]
fn reload_shortens_an_armed_delay() {
    // The reviewer's asked case end to end: armed under a 10 s file delay,
    // reloaded to 1 s, the hide fires about a second after the reload --
    // not ten seconds after the arm.
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "[appearance]\ncursor_hide_after_ms = 10000\n").expect("a config file");
    fixture.state.config_path = Some(path.clone());
    fixture.state.reload();
    assert_eq!(
        fixture.state.appearance.cursor_hide_after_ms, 10_000,
        "the reload applied the long delay"
    );
    assert!(
        fixture.state.cursor_hide_deadline.is_some(),
        "the reload armed under the long delay"
    );
    fs::write(&path, "[appearance]\ncursor_hide_after_ms = 1000\n").expect("a config file");
    let pre = Instant::now();
    fixture.state.reload();
    let post = Instant::now();
    assert_eq!(
        fixture.state.appearance.cursor_hide_after_ms, 1_000,
        "the reload applied the short delay"
    );
    let deadline = fixture
        .state
        .cursor_hide_deadline
        .expect("still armed after the reload");
    assert!(
        deadline >= pre + Duration::from_secs(1) && deadline <= post + Duration::from_secs(1),
        "the armed deadline moved to about a second after the reload, not ten after the arm: {deadline:?}"
    );
    fixture
        .state
        .note_cursor_hide_timeout(post + Duration::from_secs(1));
    assert!(fixture.hidden(), "the shortened delay hides");
}

#[test]
fn popup_dismiss_re_arms_after_a_mid_popup_firing() {
    // The open→dismiss gap with zero pointer motion: armed, a menu opens
    // over the pointer, the timer fires mid-menu (disarming without
    // hiding), the menu is dismissed -- and the cover hides on schedule
    // again instead of staying disarmed until the next motion.
    let mut fixture = Fixture::hide_after();
    fixture.cover_output();
    let t0 = Instant::now();
    fixture.state.update_cursor_hide(t0);
    fixture.run(Step::MakePopup { window: 0 });
    assert!(!fixture.hidden(), "nothing hides while the menu is up");
    fixture.state.note_cursor_hide_timeout(t0 + TIMEOUT);
    assert!(!fixture.hidden(), "a mid-menu firing hides nothing");
    assert!(
        fixture.state.cursor_hide_deadline.is_none(),
        "but it does disarm"
    );
    fixture.run(Step::DestroyPopup);
    assert!(
        fixture.state.cursor_hide_deadline.is_some(),
        "dismiss re-arms over the cover with no motion"
    );
    let t1 = Instant::now();
    fixture.state.note_cursor_hide_timeout(t1 + TIMEOUT);
    assert!(fixture.hidden(), "the re-armed delay hides");
}
