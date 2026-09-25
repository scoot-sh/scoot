//! The fixture the live suites share: a headless compositor with a real
//! output, XWayland started and its window manager attached, one X client
//! connection, and the Wayland [`peer`](super::peer) as client 0.

use scoot_core::{Placement, WindowId};
use smithay::desktop::Window;
use x11rb::protocol::xproto::Window as XWindow;

use super::peer::{Ack, Step, peer};
use super::x11::{XClient, eventually};
use super::{wait_until, xwayland_on_path};
use crate::compositor::State;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::test_support::Harness;

/// The framebuffer, square.
pub(super) const CANVAS: i32 = 400;

/// Colours as the X server takes them (`0xRRGGBB` background pixels) and as
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
/// colour wherever it samples inside it.
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
    if !xwayland_on_path() {
        eprintln!("{test}: skipped -- no Xwayland binary on PATH");
        return None;
    }
    let mut fixture: Fixture = Harness::headless(appearance, CANVAS);
    fixture.spawn(peer);
    let handle = fixture.state.loop_handle.clone();
    let display = super::super::start(handle, &mut fixture.state).expect("XWayland should start");
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
