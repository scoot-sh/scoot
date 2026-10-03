//! The fixture the live suites share: a headless compositor with a real
//! output, XWayland started and its window manager attached, one X client
//! connection, and the Wayland [`peer`](super::peer) as client 0.

use scoot_core::{Placement, WindowId};
use smithay::desktop::Window;
use smithay::wayland::xdg_activation::XdgActivationToken;
use x11rb::protocol::xproto::Window as XWindow;

use super::peer::{Ack, Step, peer};
use super::x11::{XClient, eventually};
use super::{wait_until, xwayland_on_path};
use crate::compositor::State;
use crate::compositor::config::XwaylandFractional;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::test_support::Harness;
use crate::compositor::xwayland::SpawnedPid;

/// The framebuffer, square.
pub(super) const CANVAS: i32 = 400;

/// Colors as the X server takes them (`0xRRGGBB` background pixels) and as
/// the BGRA framebuffer holds them.
pub(super) const RED: u32 = 0x00ff_0000;
pub(super) const RED_BGRA: [u8; 4] = [0x00, 0x00, 0xff, 0xff];
pub(super) const BLUE: u32 = 0x0000_00ff;
pub(super) const BLUE_BGRA: [u8; 4] = [0xff, 0x00, 0x00, 0xff];
pub(super) const PEER_BGRA: [u8; 4] = [0x20, 0xe0, 0x20, 0xff];

pub(super) type Fixture = Harness<Step, Ack>;

pub(super) struct Live {
    pub(super) fixture: Fixture,
    pub(super) x: XClient,
    pub(super) display: u32,
}

/// No ring and a flat background, so a pixel test reads a window's own
/// color wherever it samples inside it.
fn appearance() -> Appearance {
    Appearance {
        focus_ring_width: 0,
        corner_radius: 0,
        background_color: Color::new(0.1, 0.1, 0.1, 1.0),
        ..Appearance::default()
    }
}

/// The live fixture, or `None` (with the suite's `skipped --` line) where
/// the machine has no `Xwayland`.
pub(super) fn live(test: &str) -> Option<Live> {
    live_with(test, appearance())
}

/// [`live`] with its own `appearance` -- for the one suite that is about
/// the ring and the rounded clip.
pub(super) fn live_with(test: &str, appearance: Appearance) -> Option<Live> {
    live_built(test, appearance, 1, Shape::default())
}

/// [`live`] on an output at `[output] scale = scale`: [`CANVAS`] physical
/// pixels square, `CANVAS / scale` logical -- for the scale suite.
pub(super) fn live_scaled(test: &str, scale: f64) -> Option<Live> {
    live_built(
        test,
        appearance(),
        1,
        Shape {
            scale,
            ..Shape::default()
        },
    )
}

/// What a fixture's one output (or first of several) looks like, what
/// `[xwayland] fractional` the session runs with, and -- for the benchmark
/// that compares X scales on one output scale only -- the client scale
/// XWayland is given in place of the one scoot derives. `extra` is more
/// outputs, `(width, height)` in physical pixels, each added to the right
/// of the last before XWayland starts -- for the suite about layouts too
/// wide for X at their integer scale.
pub(super) struct Shape {
    pub(super) canvas: i32,
    pub(super) scale: f64,
    pub(super) fractional: XwaylandFractional,
    pub(super) client_scale: Option<f64>,
    pub(super) extra: &'static [(i32, i32)],
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            canvas: CANVAS,
            scale: 1.0,
            fractional: XwaylandFractional::Sharp,
            client_scale: None,
            extra: &[],
        }
    }
}

/// [`live`] with its output shaped by `shape`.
pub(super) fn live_shaped(test: &str, shape: Shape) -> Option<Live> {
    live_built(test, appearance(), 1, shape)
}

/// [`live`] with `outputs` headless outputs side by side, each [`CANVAS`]
/// square, all created before the peer connects (so its registry lists
/// every one, in id order) -- for the per-output suites.
pub(super) fn live_on(test: &str, outputs: i32) -> Option<Live> {
    live_built(test, appearance(), outputs, Shape::default())
}

fn live_built(test: &str, appearance: Appearance, outputs: i32, shape: Shape) -> Option<Live> {
    if !xwayland_on_path() {
        eprintln!("{test}: skipped -- no Xwayland binary on PATH");
        return None;
    }
    let mut fixture: Fixture = Harness::headless_scaled(appearance, shape.canvas, shape.scale);
    for index in 2..=outputs {
        crate::compositor::headless::add_output(
            &mut fixture.state,
            &format!("{}-{index}", crate::compositor::headless::OUTPUT_NAME),
            CANVAS,
            CANVAS,
        )
        .expect("another headless output");
    }
    for (index, &(width, height)) in shape.extra.iter().enumerate() {
        crate::compositor::headless::add_output(
            &mut fixture.state,
            &format!("wide-{index}"),
            width,
            height,
        )
        .expect("another headless output");
    }
    fixture.spawn(peer);
    let handle = fixture.state.loop_handle.clone();
    // Before the spawn, as `run` seeds it from the config file: what X
    // draws at a fractional scale for this whole session.
    fixture.state.xwayland_fractional = shape.fractional;
    let display = super::super::start(handle, &mut fixture.state).expect("XWayland should start");
    // Before the first dispatch, as `start` sets its own.
    if let Some(client_scale) = shape.client_scale
        && let Some(data) = fixture
            .state
            .xwayland_client
            .as_ref()
            .and_then(|client| client.get_data::<smithay::xwayland::XWaylandClientData>())
    {
        data.compositor_state.set_client_scale(client_scale);
    }
    wait_until(&mut fixture, "XWayland READY", |fixture| {
        fixture.state.xwm.is_some()
    });
    let x = XClient::connect(display);
    Some(Live {
        fixture,
        x,
        display,
    })
}

/// The core id of the managed window with X id `xid`.
pub(super) fn id_of_xid(state: &State, xid: XWindow) -> Option<WindowId> {
    state
        .windows
        .iter()
        .find(|(_, window)| {
            window
                .x11_surface()
                .is_some_and(|x11| x11.window_id() == xid)
        })
        .map(|(&id, _)| id)
}

impl Live {
    /// Waits until X window `xid` is managed, XWayland has paired it with
    /// its surface (so it can draw and take the keyboard), and it has drawn
    /// at the size scoot configured it to (so its input region is where its
    /// placement is), and returns its core id.
    pub(super) fn managed(&mut self, xid: XWindow) -> WindowId {
        eventually(
            &mut self.fixture,
            "the X window entering the layout and drawing at its size",
            |fixture| {
                id_of_xid(&fixture.state, xid).is_some_and(|id| {
                    fixture
                        .state
                        .window(id)
                        .and_then(Window::x11_surface)
                        .is_some_and(|x11| {
                            x11.wl_surface().is_some()
                                && x11.bbox().size == x11.last_configure().size
                        })
                })
            },
        );
        id_of_xid(&self.fixture.state, xid).expect("just waited for it")
    }

    /// Maps a Wayland window through the peer and returns its core id.
    pub(super) fn map_peer(&mut self, title: &'static str) -> WindowId {
        let before: Vec<WindowId> = self.fixture.state.windows.keys().copied().collect();
        assert!(matches!(
            self.fixture.run(Step::Map {
                title,
                color: PEER_BGRA
            }),
            Ack::Done
        ));
        self.fixture.settle();
        self.fixture
            .state
            .windows
            .keys()
            .copied()
            .find(|id| !before.contains(id))
            .expect("the peer's window entered the layout")
    }

    pub(super) fn placement(&self, id: WindowId) -> Placement {
        *self
            .fixture
            .state
            .world
            .arrange()
            .get(id)
            .expect("a placed window")
    }

    /// The taskbar's current list, `(title, app_id)` in announcement order.
    pub(super) fn taskbar(&mut self) -> Vec<(String, String)> {
        match self.fixture.run(Step::Toplevels) {
            Ack::Toplevels(list) => list,
            other => panic!("expected a toplevel list, got {other:?}"),
        }
    }

    /// The window the keyboard is on, as the seat says.
    pub(super) fn keyboard(&self) -> Option<crate::compositor::keyboard_focus::KeyboardFocus> {
        self.fixture
            .state
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
    }

    /// A spawn token bound the way `State::spawn` binds one while XWayland
    /// is live, with this test process as the launched app behind a
    /// wrapper: the token's spawn is the test's *parent* (tracked as a
    /// spawned child here; the live fixture installs no reaper to forget
    /// it), so this process -- every X connection it opens -- descends from
    /// it and may redeem the token by startup id. Not by process: its own
    /// pid is not a tracked spawn, so the process half of rule 2 cannot
    /// grant focus in the tests that use this.
    pub(super) fn launch_token(&mut self) -> XdgActivationToken {
        let token = self
            .fixture
            .state
            .mint_spawn_token("xprobe")
            .expect("a spawn token");
        let spawn = std::os::unix::process::parent_id();
        self.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .expect("just minted")
            .user_data
            .insert_if_missing_threadsafe(|| SpawnedPid(spawn));
        self.fixture.state.spawned_children.insert(spawn);
        token
    }

    /// A few settles, so X traffic in both directions has landed.
    pub(super) fn drain(&mut self) {
        for _ in 0..20 {
            self.fixture.settle();
        }
    }

    /// Renders a frame and returns the BGRA pixel at `(x, y)`.
    pub(super) fn pixel_at(&mut self, x: i32, y: i32) -> [u8; 4] {
        let pixels = self.fixture.render();
        crate::compositor::test_support::pixel(&pixels, CANVAS, x, y)
    }
}
