//! A window is drawn, and takes input, only on the output it is placed on.
//!
//! Every test here runs two side-by-side outputs through a real
//! `wayland-client` connection and asserts on what a user would see and
//! where their pointer lands: framebuffer bytes of each output, and which
//! surface the pointer entered -- never on which list a window was filtered
//! out of. Where a test probes a pixel and a pointer position, it probes the
//! *same* coordinates, so "drawn there" and "clicked there" cannot drift
//! apart without a failure.
//!
//! The geometry, on the 200-square canvas with the default 12px gap: the
//! first output spans `x = 0..200`, the second `200..400`. Two half-width
//! windows on the first output sit at `12..94` and `106..188`. The second
//! output's scenes scroll its left column part-way off its left edge, so
//! that column's placement rect crosses `x = 200` and covers the first
//! output's right window and right-hand gap -- the pixels that used to show,
//! and take clicks for, a window that belongs to the other screen.
//!
//! Like every live-`State` suite here, these need a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::{Action, Horizontal, OutputId, Rect, WindowId};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};

use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::headless;
use crate::compositor::test_support::{self, Harness, wait_for};

/// Each output's framebuffer, square.
const CANVAS: i32 = 200;
/// `Config::default`'s gap, which the harness's `State` lays out with.
const GAP: i32 = 12;
/// A row every window, popup and ring side crosses.
const ROW: i32 = 100;

// Colours, as the BGRA bytes an `Argb8888` buffer holds them in: distinct in
// every channel from each other, and built from channel values (0, 1 and
// n/255) both renderers turn into the same byte.
const FIRST_A_BGRA: [u8; 4] = [0x20, 0xE0, 0x20, 0xFF];
const FIRST_B_BGRA: [u8; 4] = [0xE0, 0xE0, 0x20, 0xFF];
const LEFT_BGRA: [u8; 4] = [0x20, 0x20, 0xE0, 0xFF];
const RIGHT_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];
const POPUP_BGRA: [u8; 4] = [0x20, 0xE0, 0xE0, 0xFF];
const ACTIVE_RING_BGRA: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];
const INACTIVE_RING_BGRA: [u8; 4] = [0xFF, 0xFF, 0x00, 0xFF];
const BACKGROUND_BGRA: [u8; 4] = [0x56, 0x34, 0x12, 0xFF];

fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: 3,
        focus_ring_active_color: Color::new(1.0, 0.0, 1.0, 1.0),
        focus_ring_inactive_color: Color::new(0.0, 1.0, 1.0, 1.0),
        background_color: Color::new(0.07058824, 0.20392157, 0.3372549, 1.0),
        ..Appearance::default()
    }
}

enum Step {
    /// Create a toplevel, commit without a buffer, then ack the configure
    /// that answers and draw `color` at the size it names.
    MapWindow { color: [u8; 4] },
    /// Ack the newest configure and draw `color` at the size it names.
    Draw { window: usize, color: [u8; 4] },
    /// `xdg_toplevel.set_fullscreen` with no output, then wait for the
    /// configure that answers. Does not ack it.
    SetFullscreen { window: usize },
    /// Map a no-grab popup on the `window`-th toplevel whose top-*right*
    /// corner sits at `(x, y)` in the parent's window geometry -- so it grows
    /// leftwards from there, `w` x `h`, with no constraint adjustment.
    PopupLeftOf {
        window: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    /// Report which of this client's surfaces the pointer last entered.
    ReportPointer,
}

enum Ack {
    Done,
    Pointer(Option<Entered>),
}

/// A surface the pointer entered, by the order the script created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entered {
    Window(usize),
    Popup(usize),
    Other,
}

/// Which `xdg_surface` an event belongs to.
#[derive(Clone, Copy)]
enum Role {
    Window(usize),
    Popup(usize),
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_focus: Option<wl_surface::WlSurface>,
    /// Per toplevel: the size the pending `xdg_toplevel.configure` named.
    pending: Vec<(i32, i32)>,
    /// Per toplevel: every completed configure, `(serial, width, height)`.
    configures: Vec<Vec<(u32, i32, i32)>>,
    /// Per toplevel: the serial it acked last.
    acked: Vec<Option<u32>>,
    /// Per popup: the serial of its newest configure.
    popup_serials: Vec<Option<u32>>,
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
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
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
            && capabilities.contains(wl_seat::Capability::Pointer)
            && client.pointer.is_none()
        {
            client.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => client.pointer_focus = Some(surface),
            wl_pointer::Event::Leave { .. } => client.pointer_focus = None,
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

impl Dispatch<xdg_toplevel::XdgToplevel, Role> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let (xdg_toplevel::Event::Configure { width, height, .. }, Role::Window(index)) =
            (event, *role)
            && let Some(pending) = client.pending.get_mut(index)
        {
            *pending = (width, height);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, Role> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        role: &Role,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = event else {
            return;
        };
        match *role {
            Role::Window(index) => {
                if let Some(&(width, height)) = client.pending.get(index)
                    && let Some(seen) = client.configures.get_mut(index)
                {
                    seen.push((serial, width, height));
                }
            }
            Role::Popup(index) => {
                if let Some(slot) = client.popup_serials.get_mut(index) {
                    *slot = Some(serial);
                }
            }
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(TestClient: ignore xdg_popup::XdgPopup);

/// A `width`x`height` buffer of `color` over a real memfd. A zero size (a
/// configure that left the size to the client) draws 40 square.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> (wl_buffer::WlBuffer, i32, i32) {
    let width = if width > 0 { width } else { 40 };
    let height = if height > 0 { height } else { 40 };
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-output-clip-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    (buffer, width, height)
}

/// One toplevel the script made.
struct Toplevel {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

fn wait_for_configure(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    window: usize,
    seen: usize,
) -> Result<(), String> {
    wait_for(queue, client, "a toplevel configure", |client| {
        (client.configures.get(window)?.len() > seen).then_some(())
    })
}

/// Acks the newest configure (once) and draws `color` at its size.
fn draw(
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    window: &Toplevel,
    index: usize,
    color: [u8; 4],
) -> Result<(), String> {
    let (serial, width, height) = client
        .configures
        .get(index)
        .and_then(|all| all.last().copied())
        .ok_or("no configure to draw for")?;
    if client.acked.get(index).copied().flatten() != Some(serial) {
        window.xdg.ack_configure(serial);
        if let Some(slot) = client.acked.get_mut(index) {
            *slot = Some(serial);
        }
    }
    let (buffer, width, height) = solid_buffer(shm, qh, width, height, color);
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
    // Held for the run: a popup whose objects drop is dismissed.
    let mut popups: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_popup::XdgPopup,
    )> = Vec::new();
    while let Ok(step) = steps.recv() {
        let ack = match step {
            Step::MapWindow { color } => {
                let index = windows.len();
                client.pending.push((0, 0));
                client.configures.push(Vec::new());
                client.acked.push(None);
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Window(index));
                let toplevel = xdg.get_toplevel(&qh, Role::Window(index));
                surface.commit();
                wait_for_configure(&mut queue, &mut client, index, 0)?;
                let window = Toplevel {
                    surface,
                    xdg,
                    toplevel,
                };
                draw(&mut client, &qh, &shm, &window, index, color)?;
                windows.push(window);
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::Draw { window, color } => {
                draw(&mut client, &qh, &shm, &windows[window], window, color)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::SetFullscreen { window } => {
                let seen = client.configures[window].len();
                windows[window].toplevel.set_fullscreen(None);
                wait_for_configure(&mut queue, &mut client, window, seen)?;
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::PopupLeftOf { window, x, y, w, h } => {
                let index = popups.len();
                client.popup_serials.push(None);
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, Role::Popup(index));
                let positioner = wm_base.create_positioner(&qh, ());
                positioner.set_size(w, h);
                positioner.set_anchor_rect(x, y, 1, 1);
                positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
                positioner.set_gravity(xdg_positioner::Gravity::BottomLeft);
                let popup = xdg.get_popup(Some(&windows[window].xdg), &positioner, &qh, ());
                positioner.destroy();
                surface.commit();
                let serial = wait_for(&mut queue, &mut client, "a popup configure", |client| {
                    client.popup_serials[index]
                })?;
                xdg.ack_configure(serial);
                let (buffer, w, h) = solid_buffer(&shm, &qh, w, h, POPUP_BGRA);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, w, h);
                surface.commit();
                popups.push((surface, xdg, popup));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::ReportPointer => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let entered = client.pointer_focus.as_ref().map(|focus| {
                    if let Some(i) = windows.iter().position(|w| &w.surface == focus) {
                        Entered::Window(i)
                    } else if let Some(i) = popups.iter().position(|(s, _, _)| s == focus) {
                        Entered::Popup(i)
                    } else {
                        Entered::Other
                    }
                });
                Ack::Pointer(entered)
            }
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// Two outputs, one client, the pointer on the first output.
    fn two_outputs() -> Self {
        Self::two_outputs_with(appearance(), 1.0)
    }

    fn two_outputs_with(appearance: Appearance, scale: f64) -> Self {
        let mut fixture = Harness::headless_scaled(appearance, CANVAS, scale);
        headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
            .expect("a second output");
        fixture.settle();
        fixture.spawn(run_client);
        fixture
    }

    /// Parks the pointer in the middle of output `index` (1-based), which is
    /// where the next window opens (see `shell.rs`'s `add_window`).
    fn pointer_on(&mut self, index: i32) {
        let geometry = self.output_rect(index);
        self.state.pointer_move(
            f64::from(geometry.x + geometry.w / 2),
            f64::from(geometry.y + geometry.h / 2),
        );
        self.settle();
    }

    /// Output `index`'s (1-based) logical rectangle.
    fn output_rect(&self, index: i32) -> Rect {
        let output = self
            .state
            .outputs
            .get(OutputId(index as u64))
            .expect("an output");
        let geometry = self
            .state
            .space
            .output_geometry(output)
            .expect("a mapped output");
        Rect::new(
            geometry.loc.x,
            geometry.loc.y,
            geometry.size.w,
            geometry.size.h,
        )
    }

    fn map(&mut self, color: [u8; 4]) {
        assert!(matches!(self.run(Step::MapWindow { color }), Ack::Done));
    }

    fn done(&mut self, step: Step) {
        assert!(matches!(self.run(step), Ack::Done));
    }

    fn act(&mut self, action: Action) {
        self.state.act(action);
        self.settle();
    }

    /// The core id of the `index`-th window this client mapped (ids only
    /// increment, and this suite has one client).
    fn id(&self, index: usize) -> WindowId {
        let mut ids: Vec<WindowId> = self.state.windows.keys().copied().collect();
        ids.sort();
        ids[index]
    }

    fn rect_of(&self, index: usize) -> Rect {
        self.state
            .world
            .arrange()
            .get(self.id(index))
            .expect("a placed window")
            .rect
    }

    fn output_of(&self, index: usize) -> OutputId {
        self.state
            .world
            .arrange()
            .get(self.id(index))
            .expect("a placed window")
            .output
    }

    /// Draws every output and hands back output `index`'s (1-based) pixels.
    fn frame(&mut self, index: u64) -> Vec<u8> {
        self.state.request_render();
        self.state.render();
        self.pixels_of(OutputId(index))
    }

    fn pointer(&mut self) -> Option<Entered> {
        match self.run(Step::ReportPointer) {
            Ack::Pointer(entered) => entered,
            _ => panic!("expected a pointer report"),
        }
    }

    /// Moves the pointer to the global `(x, y)` and reports what it entered.
    fn point_at(&mut self, x: i32, y: i32) -> Option<Entered> {
        self.state.pointer_move(f64::from(x), f64::from(y));
        self.settle();
        self.pointer()
    }

    fn click(&mut self, x: i32, y: i32) {
        self.state.pointer_move(f64::from(x), f64::from(y));
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, true);
        self.state
            .pointer_button(scoot_ipc::PointerButton::Left, false);
        self.settle();
    }

    /// Two half-width windows on the first output, windows 0 and 1.
    fn first_output_pair(&mut self) {
        self.pointer_on(1);
        self.map(FIRST_A_BGRA);
        self.map(FIRST_B_BGRA);
        assert_eq!(
            self.rect_of(1).x,
            CANVAS / 2 + GAP / 2,
            "{:?}",
            self.rect_of(1)
        );
    }

    /// Two two-thirds windows on the second output (the next two indices),
    /// with the right one focused, so the left one's rect crosses onto the
    /// first output. Both are redrawn at their final sizes, the way a real
    /// client answers a resize. Hands back the left window's index.
    fn second_output_overhang(&mut self) -> usize {
        let left = self.state.windows.len();
        self.pointer_on(2);
        self.map(LEFT_BGRA);
        self.act(Action::SetColumnWidth(2));
        self.map(RIGHT_BGRA);
        self.act(Action::SetColumnWidth(2));
        self.act(Action::FocusColumn(Horizontal::Right));
        self.done(Step::Draw {
            window: left,
            color: LEFT_BGRA,
        });
        self.done(Step::Draw {
            window: left + 1,
            color: RIGHT_BGRA,
        });
        let rect = self.rect_of(left);
        assert_eq!(self.output_of(left), OutputId(2));
        assert!(
            rect.x < CANVAS && rect.right() > CANVAS,
            "the scene needs the left column across the shared edge: {rect:?}"
        );
        left
    }
}

fn pixel(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    test_support::pixel(pixels, CANVAS, x, y)
}

fn assert_absent(pixels: &[u8], color: [u8; 4], what: &str) {
    assert!(!test_support::contains(pixels, color), "{what}");
}

// ---------------------------------------------------------------------------
// A tiled column hanging over the shared edge
// ---------------------------------------------------------------------------

#[test]
fn an_overhanging_column_is_not_drawn_on_the_neighbouring_output() {
    let mut fixture = Fixture::two_outputs();
    let left = fixture.second_output_overhang();
    let rect = fixture.rect_of(left);

    let first = fixture.frame(1);
    assert_absent(
        &first,
        LEFT_BGRA,
        "the second output's column was drawn on the first output",
    );
    assert_absent(
        &first,
        INACTIVE_RING_BGRA,
        "the second output's ring was drawn on the first output",
    );
    assert!(
        first.chunks_exact(4).all(|pixel| pixel == BACKGROUND_BGRA),
        "the first output has no windows of its own, so it is background edge to edge"
    );

    // ...and it is still drawn on its own output, right up to the edge.
    let second = fixture.frame(2);
    assert_eq!(pixel(&second, 0, ROW), LEFT_BGRA);
    assert_eq!(pixel(&second, rect.right() - CANVAS - 1, ROW), LEFT_BGRA);
}

#[test]
fn a_click_on_the_overhang_reaches_the_first_outputs_own_window() {
    let mut fixture = Fixture::two_outputs();
    fixture.first_output_pair();
    let left = fixture.second_output_overhang();
    let own = fixture.rect_of(1);
    let rect = fixture.rect_of(left);
    // Inside the first output's right window, and under the overhang.
    let inside = (own.right() - 4, ROW);
    assert!(rect.x < inside.0, "{rect:?} does not cover {inside:?}");
    // The first output's right-hand gap, also under the overhang.
    let gap = (CANVAS - 3, ROW);

    let first = fixture.frame(1);
    assert_eq!(
        pixel(&first, inside.0, inside.1),
        FIRST_B_BGRA,
        "the overhang was drawn over the first output's window"
    );
    assert_eq!(pixel(&first, gap.0, gap.1), BACKGROUND_BGRA);
    assert_eq!(
        fixture.point_at(inside.0, inside.1),
        Some(Entered::Window(1)),
        "motion over the first output's window entered the other output's"
    );
    assert_eq!(
        fixture.point_at(gap.0, gap.1),
        None,
        "bare desktop on the first output entered the other output's window"
    );

    fixture.click(inside.0, inside.1);
    assert_eq!(
        fixture.state.focus,
        Some(fixture.id(1)),
        "a click on the first output's window focused the other output's"
    );
    // ...and the overhanging window still takes its own output's clicks.
    fixture.click(CANVAS + 4, ROW);
    assert_eq!(fixture.pointer(), Some(Entered::Window(left)));
    assert_eq!(fixture.state.focus, Some(fixture.id(left)));
}

// ---------------------------------------------------------------------------
// A fullscreen window its column is focused away from
// ---------------------------------------------------------------------------

#[test]
fn a_focused_away_fullscreen_window_stays_on_its_own_output() {
    let mut fixture = Fixture::two_outputs();
    fixture.first_output_pair();
    fixture.pointer_on(2);
    fixture.map(LEFT_BGRA);
    fixture.map(RIGHT_BGRA);
    let (full, other) = (2, 3);
    fixture.act(Action::FocusWindowId(fixture.id(full)));
    fixture.done(Step::SetFullscreen { window: full });
    fixture.done(Step::Draw {
        window: full,
        color: LEFT_BGRA,
    });
    assert_eq!(fixture.rect_of(full), fixture.output_rect(2));
    fixture.act(Action::FocusWindowId(fixture.id(other)));
    let rect = fixture.rect_of(full);
    assert!(fixture.state.world.is_fullscreen(fixture.id(full)));
    assert_eq!(rect.w, CANVAS, "it keeps its fullscreen size: {rect:?}");
    assert!(
        rect.x < CANVAS && rect.right() > CANVAS,
        "the scene needs it across the shared edge: {rect:?}"
    );

    let inside = (fixture.rect_of(1).x + 4, ROW);
    assert!(rect.x < inside.0, "{rect:?} does not cover {inside:?}");
    let first = fixture.frame(1);
    assert_absent(
        &first,
        LEFT_BGRA,
        "the focused-away fullscreen window was drawn on the first output",
    );
    assert_eq!(pixel(&first, inside.0, inside.1), FIRST_B_BGRA);
    assert_eq!(
        fixture.point_at(inside.0, inside.1),
        Some(Entered::Window(1)),
        "motion over the first output's window entered the fullscreen window"
    );
    assert_eq!(
        fixture.point_at(CANVAS - 3, ROW),
        None,
        "bare desktop on the first output entered the fullscreen window"
    );
    fixture.click(inside.0, inside.1);
    assert_eq!(
        fixture.state.focus,
        Some(fixture.id(1)),
        "a click on the first output's window focused the fullscreen window"
    );

    // On its own output it is drawn and pointed at as before.
    let second = fixture.frame(2);
    assert_eq!(pixel(&second, 0, ROW), LEFT_BGRA);
    assert_eq!(
        fixture.point_at(CANVAS + 2, ROW),
        Some(Entered::Window(full))
    );
}

// ---------------------------------------------------------------------------
// Popups belong to their parent's output
// ---------------------------------------------------------------------------

/// A menu opened near the shared edge is cut at that edge -- exactly as it
/// is at the outer edge of a single output -- rather than drawn over, and
/// taking the clicks of, the other output's windows. The part on its own
/// output is drawn and takes input as before.
#[test]
fn a_popup_crossing_the_shared_edge_is_cut_there() {
    let mut fixture = Fixture::two_outputs();
    fixture.first_output_pair();
    fixture.pointer_on(2);
    fixture.map(LEFT_BGRA);
    let parent = 2;
    let rect = fixture.rect_of(parent);
    assert_eq!(rect.x, CANVAS + GAP, "{rect:?}");
    // Grows left from the parent's own left edge, 60 wide: across the
    // second output's gap and onto the first output's gap and right window.
    let (y, w, h) = (20, 60, 40);
    fixture.done(Step::PopupLeftOf {
        window: parent,
        x: 0,
        y,
        w,
        h,
    });
    let row = rect.y + y + h / 2;
    let own_part = (CANVAS + GAP / 2, row);
    let on_window = (fixture.rect_of(1).right() - 4, row);
    let on_gap = (CANVAS - 3, row);
    assert!(
        rect.x - w < on_window.0,
        "the popup does not reach {on_window:?}"
    );

    let second = fixture.frame(2);
    assert_eq!(
        pixel(&second, own_part.0 - CANVAS, own_part.1),
        POPUP_BGRA,
        "the popup is drawn on its parent's output"
    );
    assert_eq!(
        fixture.point_at(own_part.0, own_part.1),
        Some(Entered::Popup(0)),
        "the popup takes the pointer on its parent's output"
    );

    let first = fixture.frame(1);
    assert_absent(
        &first,
        POPUP_BGRA,
        "the popup was drawn on the other output",
    );
    assert_eq!(pixel(&first, on_window.0, on_window.1), FIRST_B_BGRA);
    assert_eq!(pixel(&first, on_gap.0, on_gap.1), BACKGROUND_BGRA);
    assert_eq!(
        fixture.point_at(on_window.0, on_window.1),
        Some(Entered::Window(1)),
        "the first output's window lost the pointer to another output's popup"
    );
    assert_eq!(fixture.point_at(on_gap.0, on_gap.1), None);
}

// ---------------------------------------------------------------------------
// Decorations on outputs other than the first
// ---------------------------------------------------------------------------

/// The same lone window, mapped on the first output in one session and on
/// the second in another, composites the same pixels on the output it is
/// on -- ring, rounding and all -- and leaves the other output blank.
///
/// This is what a window's ring and its rounded clip being built in the
/// coordinates of the output they are drawn on means; built in global ones,
/// the second output's ring lands a whole output to the right of its
/// framebuffer, and its rounded clip cuts the wrong pixels.
fn same_window_on_either_output(appearance: Appearance, scale: f64) {
    let frames = |index: i32| {
        let mut fixture = Fixture::two_outputs_with(appearance.clone(), scale);
        fixture.pointer_on(index);
        fixture.map(LEFT_BGRA);
        assert_eq!(fixture.output_of(0), OutputId(index as u64));
        // Parked where no window is, so no pointer focus differs.
        fixture.pointer_on(3 - index);
        (fixture.frame(1), fixture.frame(2))
    };
    let (on_first, blank_second) = frames(1);
    let (blank_first, on_second) = frames(2);
    assert!(
        test_support::contains(&on_first, LEFT_BGRA),
        "the control: the window drew on the first output"
    );
    assert!(
        test_support::contains(&on_first, ACTIVE_RING_BGRA),
        "the control: the first output drew the focus ring"
    );
    assert!(
        on_second == on_first,
        "a window on the second output does not composite what the same window does on the \
         first (radius {}, scale {scale})",
        appearance.corner_radius
    );
    assert!(
        blank_first == blank_second,
        "the output with no window differs between the sessions"
    );
    assert_absent(
        &blank_first,
        LEFT_BGRA,
        "the window drew on the wrong output",
    );
    assert_absent(
        &blank_first,
        ACTIVE_RING_BGRA,
        "the ring drew on the wrong output",
    );
}

#[test]
fn the_second_output_draws_a_square_window_and_ring_like_the_first() {
    same_window_on_either_output(appearance(), 1.0);
}

#[test]
fn the_second_output_draws_a_rounded_window_and_ring_like_the_first() {
    same_window_on_either_output(
        Appearance {
            corner_radius: 12,
            ..appearance()
        },
        1.0,
    );
}

#[test]
fn the_second_output_draws_like_the_first_at_scale_two() {
    same_window_on_either_output(
        Appearance {
            corner_radius: 12,
            ..appearance()
        },
        2.0,
    );
}

#[test]
fn the_second_output_draws_like_the_first_at_a_fractional_scale() {
    for radius in [0, 12] {
        same_window_on_either_output(
            Appearance {
                corner_radius: radius,
                ..appearance()
            },
            1.5,
        );
    }
}

// ---------------------------------------------------------------------------
// Past every output
// ---------------------------------------------------------------------------

/// A column hanging off the far edge of the last output is drawn nowhere
/// there, so it takes no input there either -- the pointer can be moved
/// past the outputs (absolute motion is not clamped), and over no output
/// nothing is hit.
#[test]
fn a_column_hanging_past_the_last_output_takes_no_input_there() {
    let mut fixture = Fixture::two_outputs();
    let left = fixture.second_output_overhang();
    fixture.act(Action::FocusColumn(Horizontal::Left));
    let right = fixture.rect_of(left + 1);
    assert!(
        right.right() > 2 * CANVAS,
        "the scene needs the right column past the last output: {right:?}"
    );
    assert_eq!(
        fixture.point_at(2 * CANVAS - 4, ROW),
        Some(Entered::Window(left + 1)),
        "the control: on its own output the column takes the pointer"
    );
    assert_eq!(
        fixture.point_at(2 * CANVAS + 4, ROW),
        None,
        "past every output, a window nobody can see took the pointer"
    );
}

// ---------------------------------------------------------------------------
// Moving between outputs
// ---------------------------------------------------------------------------

/// A window moved to the other output is drawn and pointed at there, and
/// nowhere it used to be.
#[test]
fn a_window_moved_to_the_other_output_follows_there() {
    let mut fixture = Fixture::two_outputs();
    fixture.pointer_on(1);
    fixture.map(LEFT_BGRA);
    let before = fixture.rect_of(0);
    fixture.act(Action::MoveFocusedWindowToOutput(OutputId(2)));
    assert_eq!(fixture.output_of(0), OutputId(2));
    let after = fixture.rect_of(0);

    assert_absent(
        &fixture.frame(1),
        LEFT_BGRA,
        "the moved window was still drawn where it was",
    );
    assert_eq!(
        pixel(&fixture.frame(2), after.x - CANVAS + 4, ROW),
        LEFT_BGRA
    );
    assert_eq!(fixture.point_at(before.x + 4, ROW), None);
    assert_eq!(fixture.point_at(after.x + 4, ROW), Some(Entered::Window(0)));
}

// ---------------------------------------------------------------------------
// `to_output_local`
// ---------------------------------------------------------------------------

#[test]
fn local_coordinates_are_the_identity_on_an_output_at_the_origin() {
    let rect = Rect::new(12, 12, 82, 176);
    assert_eq!(
        super::to_output_local(rect, Rect::new(0, 0, CANVAS, CANVAS)),
        rect
    );
}

#[test]
fn local_coordinates_subtract_the_outputs_origin() {
    let output = Rect::new(CANVAS, 40, CANVAS, CANVAS);
    assert_eq!(
        super::to_output_local(Rect::new(150, 52, 117, 176), output),
        Rect::new(150 - CANVAS, 12, 117, 176),
        "a rect hanging over the output's left edge goes negative, not clamped"
    );
}

#[test]
fn local_coordinates_saturate_instead_of_wrapping() {
    let local = super::to_output_local(
        Rect::new(i32::MIN, i32::MAX, 10, 10),
        Rect::new(1, -1, CANVAS, CANVAS),
    );
    assert_eq!((local.x, local.y), (i32::MIN, i32::MAX));
}
