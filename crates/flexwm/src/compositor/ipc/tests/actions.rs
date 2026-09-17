//! Focusing over IPC spends a clicked `on_demand` layer surface's keyboard.
//!
//! The bug this pins: `Request::Action` called `act` for every IPC action
//! with no `clicked_layer` clear anywhere on the path, so an agent that
//! clicked an `on_demand` panel (taking the keyboard), then drove focus with
//! `focus-window-id` and injected keystrokes, got every keystroke delivered
//! to the panel instead -- with `flexwm msg windows` reporting the window as
//! focused the whole time, so there was nothing to detect the mismatch from.
//! Unlike an `xdg-activation-v1` launcher, nothing here unmaps itself a
//! moment later to self-correct, which is why this half is the higher
//! severity one.
//!
//! Like `foreign_toplevel_management`'s `taskbar_holding_the_keyboard` and
//! `activation`'s `keyboard` suite, this runs a real mapped `on_demand`
//! taskbar and a real pointer click through the input path (the only writer
//! of `clicked_layer`), then focuses over IPC, asserting on the seat's
//! actual keyboard focus surface. The weak shape (hand-setting
//! `clicked_layer` with no real layer surface) passes either way and proves
//! nothing: without the clear, `layer_keyboard_focus` re-derives the
//! still-mapped taskbar and every covered assertion below fails.
//!
//! The predicate under test is the whole focus family -- every action whose
//! purpose is moving window focus spends the click -- plus the boundary: a
//! layout action must leave a deliberate keyboard placement alone.

use std::os::fd::AsFd;
use std::sync::mpsc::{Receiver, Sender};

use flexwm_core::WindowId;
use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

/// The framebuffer the headless backend renders into. Nothing here reads a
/// pixel; the backend exists so the compositor has a real output with a real
/// layer map.
const CANVAS: i32 = 200;

/// How wide and tall the `on_demand` taskbar is, anchored to the
/// bottom-right corner -- the shape
/// `foreign_toplevel_management/tests` uses, for the same reason: the layout
/// puts columns from the left edge, so the corner stays outside any window.
const TASKBAR: u32 = 60;

/// A point inside that taskbar, for a click that really goes through the
/// pointer.
const TASKBAR_POINT: (f64, f64) = (CANVAS as f64 - 20.0, CANVAS as f64 - 20.0);

/// One instruction for the client thread. The script runs start to finish on
/// its own; the test drives focus from the compositor side, over IPC.
enum Step {
    /// Hand back everything seen so far (unused -- kept so the harness has a
    /// step type at all).
    TakeLog,
}

enum Ack {
    Ready,
    Done,
}

/// The client end: two bare windows and one `on_demand` taskbar with a real
/// buffer, the way a shell panel looks to the compositor.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`.
    window_serials: Vec<Option<u32>>,
    /// The size the compositor configured the taskbar at, once it has.
    taskbar_size: Option<(u32, u32)>,
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
        if interface == wl_compositor::WlCompositor::interface().name {
            client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(3), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == zwlr_layer_shell_v1::ZwlrLayerShellV1::interface().name {
            client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
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

/// A window's own index in creation order, so its configure can be matched
/// back to it.
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
            && let Some(slot) = client.window_serials.get_mut(index.0)
        {
            *slot = Some(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, WindowIndex> for TestClient {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: xdg_toplevel::Event,
        _: &WindowIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for TestClient {
    /// Acks the configure and records the size it carried, which is the only
    /// thing the taskbar needs from the compositor before it can draw.
    fn event(
        client: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer.ack_configure(serial);
            client.taskbar_size = Some((width, height));
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// Creates one bare `xdg_toplevel` and acks the configure that comes back.
///
/// Bare, like `foreign_toplevel_management/tests` uses them: what puts a
/// window in flexwm's model is the toplevel existing, and nothing here clicks
/// a window or reads a pixel, so no `wl_shm` is needed for them.
fn map_window(
    compositor: &wl_compositor::WlCompositor,
    wm_base: &xdg_wm_base::XdgWmBase,
    qh: &QueueHandle<TestClient>,
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
) -> Result<
    (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    ),
    String,
> {
    let surface = compositor.create_surface(qh, ());
    let index = client.window_serials.len();
    client.window_serials.push(None);
    let xdg = wm_base.get_xdg_surface(&surface, qh, WindowIndex(index));
    let toplevel = xdg.get_toplevel(qh, WindowIndex(index));
    surface.commit();
    let serial = wait_for(queue, client, "a toplevel configure", |client| {
        client.window_serials[index]
    })?;
    xdg.ack_configure(serial);
    Ok((surface, xdg, toplevel))
}

/// A `width`x`height` opaque `wl_buffer` over a real memfd -- the same path
/// any toolkit takes, and what a layer surface needs before it counts as
/// mapped.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("flexwm-ipc-actions-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len])
        .map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

/// Maps two windows and the taskbar, reports readiness, and parks until the
/// test disconnects it -- returning early would unmap the taskbar and spend
/// the click under test from underneath it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    // Held so the windows stay mapped: dropping a role object would destroy
    // the window it stands for.
    let windows = vec![
        map_window(&compositor, &wm_base, &qh, &mut queue, &mut client)?,
        map_window(&compositor, &wm_base, &qh, &mut queue, &mut client)?,
    ];

    let shell = client
        .layer_shell
        .clone()
        .ok_or("no zwlr_layer_shell_v1 -- the global is missing")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let surface = compositor.create_surface(&qh, ());
    let layer = shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "flexwm-ipc-test-taskbar".into(),
        &qh,
        (),
    );
    layer.set_anchor(zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Right);
    layer.set_size(TASKBAR, TASKBAR);
    layer.set_exclusive_zone(0);
    // The whole point: a surface that takes the keyboard when it is clicked,
    // and keeps it until something takes it away -- which is what a
    // Quickshell panel is.
    layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand);
    surface.commit();
    let (width, height) = wait_for(&mut queue, &mut client, "a taskbar configure", |client| {
        client.taskbar_size
    })?;
    let buffer = solid_buffer(&shm, &qh, width as i32, height as i32)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, width as i32, height as i32);
    surface.commit();
    // Held so everything stays mapped for the whole test.
    let _mapped = (windows, surface, layer, buffer);

    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::Ready).map_err(|e| e.to_string())?;
    // One step may already be queued (see `drive`); swallow steps until the
    // test disconnects rather than acting on any of them.
    while let Ok(Step::TakeLog) = steps.recv() {
        acks.send(Ack::Done).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one connected client
/// holding two windows and a taskbar.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn drive() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        let Ack::Ready = fixture.run(Step::TakeLog) else {
            panic!("the client never mapped its windows and taskbar");
        };
        // Mapping the second window focused it, and mapping the taskbar
        // never steals anything.
        assert_eq!(
            fixture.state.focus,
            Some(WindowId(2)),
            "the second window should have focus before anything is clicked"
        );
        fixture
    }

    /// A left click at a point, press and release, the way a user makes one.
    /// `State::clicked_layer` is only ever written by a click that really
    /// went through the pointer.
    fn click(&mut self, x: f64, y: f64) {
        self.state.pointer_move(x, y);
        self.state
            .pointer_button(flexwm_ipc::PointerButton::Left, true);
        self.state
            .pointer_button(flexwm_ipc::PointerButton::Left, false);
        self.settle();
    }

    /// The `wl_surface` the seat's keyboard focus is actually on, which is
    /// the only unambiguous answer to "where do keystrokes go".
    fn keyboard_surface(&self) -> Option<WlSurface> {
        self.state
            .seat
            .get_keyboard()
            .expect("a keyboard")
            .current_focus()
    }

    /// The `wl_surface` of the clicked layer surface itself, so the test can
    /// assert the keyboard is on exactly that surface rather than merely off
    /// the window.
    fn clicked_surface(&self) -> WlSurface {
        self.state
            .clicked_layer
            .as_ref()
            .expect("a clicked layer surface")
            .wl_surface()
            .clone()
    }

    /// The `wl_surface` of window `id`'s toplevel.
    fn window_surface(&self, id: WindowId) -> WlSurface {
        self.state
            .windows
            .get(&id)
            .and_then(Window::toplevel)
            .expect("a live window")
            .wl_surface()
            .clone()
    }

    /// Clicks the taskbar and asserts the click really landed: the keyboard
    /// is on exactly the clicked surface, and window focus did not move.
    fn click_taskbar(&mut self) {
        let focus_before = self.state.focus;
        self.click(TASKBAR_POINT.0, TASKBAR_POINT.1);
        assert!(
            self.state.clicked_layer.is_some(),
            "the click never reached the taskbar -- check TASKBAR_POINT against the layout"
        );
        assert!(
            self.state.keyboard_on_layer,
            "an on_demand layer surface should hold the keyboard once clicked"
        );
        assert_eq!(
            self.keyboard_surface(),
            Some(self.clicked_surface()),
            "the keyboard is not on the taskbar the click landed on"
        );
        assert_eq!(
            self.state.focus, focus_before,
            "clicking a bar must not move *window* focus"
        );
    }

    /// The keyboard must be on whatever window focus reports, once a focus
    /// action has spent the click -- or on nothing, when focus itself is
    /// empty (stepping off the last workspace is still a focus gesture, and
    /// spending the click there is what keeps the two from disagreeing).
    fn assert_keyboard_follows_focus(&self, what: &str) {
        let expected = self.state.focus.map(|id| self.window_surface(id));
        assert_eq!(
            self.keyboard_surface(),
            expected,
            "{what}: the keyboard did not follow window focus"
        );
        assert!(
            self.state.clicked_layer.is_none(),
            "{what}: the taskbar's click was not spent"
        );
    }
}

#[test]
fn focusing_over_ipc_takes_the_keyboard_back_from_a_clicked_taskbar() {
    // Every action whose purpose is moving window focus spends the click a
    // real pointer click left on the taskbar. Without the clear in
    // `handle_request`, each of these moves the reported focus while the
    // keyboard stays on the still-mapped panel -- the mismatch this test
    // exists for.
    let mut fixture = Fixture::drive();
    let focused = fixture.state.focus.expect("a focused window");
    let other = *fixture
        .state
        .windows
        .keys()
        .find(|id| **id != focused)
        .expect("two windows");
    let covered = [
        flexwm_ipc::Action::FocusWindowId { id: other.0 },
        flexwm_ipc::Action::FocusColumn {
            direction: flexwm_ipc::Horizontal::Left,
        },
        flexwm_ipc::Action::FocusWindow {
            direction: flexwm_ipc::Vertical::Down,
        },
        flexwm_ipc::Action::FocusWorkspace {
            direction: flexwm_ipc::Vertical::Down,
        },
        flexwm_ipc::Action::FocusWorkspaceIndex { index: 0 },
    ];
    for (n, action) in covered.into_iter().enumerate() {
        fixture.click_taskbar();
        let response = fixture.state.handle_request(Request::Action(action));
        assert!(
            matches!(response, Response::Ok { locked: false }),
            "focus action {n} was not served"
        );
        fixture.assert_keyboard_follows_focus(&format!("focus action {n}"));
    }
}

#[test]
fn already_focused_actions_spend_the_click_without_an_apply() {
    // The no-op half of the fast path `handle_request` shares with
    // `ext_workspace.rs` and `wlr_toplevel_activate`: the target is already
    // there, so there is no `act` -- no arrange, no configure per window, no
    // render -- but the click is still spent and the keyboard half still
    // runs. `needs_render` is the deterministic witness, the way
    // `activating_the_window_that_is_already_focused_does_no_work_at_all`
    // uses it: it is set by `apply`'s `request_render` and by nothing else
    // on this path, and nothing dispatches between the reset and the
    // assertion. Without the fast path each of these runs a full apply and
    // the `!needs_render` below fails -- which is the fail-first pin for
    // this half.
    let mut fixture = Fixture::drive();
    let focused = fixture.state.focus.expect("a focused window");
    let output = fixture
        .state
        .world
        .focused_output()
        .expect("a focused output");
    let active = fixture
        .state
        .world
        .workspaces(output)
        .expect("a workspace list")
        .active;
    assert_eq!(
        active, 0,
        "the Up step below is only a no-op from the first workspace"
    );
    let noop = [
        flexwm_ipc::Action::FocusWindowId { id: focused.0 },
        flexwm_ipc::Action::FocusWorkspaceIndex { index: active },
        flexwm_ipc::Action::FocusWorkspace {
            direction: flexwm_ipc::Vertical::Up,
        },
    ];
    for (n, action) in noop.into_iter().enumerate() {
        fixture.click_taskbar();
        fixture.state.needs_render = false;
        let response = fixture.state.handle_request(Request::Action(action));
        assert!(
            matches!(response, Response::Ok { locked: false }),
            "no-op focus action {n} was not served"
        );
        assert_eq!(
            fixture.state.focus,
            Some(focused),
            "a no-op focus action {n} moved window focus"
        );
        fixture.assert_keyboard_follows_focus(&format!("no-op focus action {n}"));
        assert!(
            !fixture.state.needs_render,
            "no-op focus action {n} ran a full apply"
        );
    }
}

#[test]
fn real_focus_moves_over_ipc_still_apply() {
    // The contrast the test above needs: the same assertions, but for moves
    // that really go somewhere -- these must still run the full `apply`.
    // Passes with and without the fast path, by design: it pins what the
    // no-op split must *not* swallow.
    let mut fixture = Fixture::drive();
    let focused = fixture.state.focus.expect("a focused window");
    let other = *fixture
        .state
        .windows
        .keys()
        .find(|id| **id != focused)
        .expect("two windows");

    // Across windows: focus, keyboard and layout all move.
    fixture.click_taskbar();
    fixture.state.needs_render = false;
    let response =
        fixture
            .state
            .handle_request(Request::Action(flexwm_ipc::Action::FocusWindowId {
                id: other.0,
            }));
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the focus action was not served"
    );
    assert_eq!(fixture.state.focus, Some(other));
    fixture.assert_keyboard_follows_focus("a real window-focus move");
    assert!(
        fixture.state.needs_render,
        "a real window-focus move laid nothing out"
    );

    // Across workspaces, onto the trailing empty one: focus legitimately
    // empties, and the keyboard follows it to nothing.
    fixture.click_taskbar();
    fixture.state.needs_render = false;
    let response =
        fixture
            .state
            .handle_request(Request::Action(flexwm_ipc::Action::FocusWorkspace {
                direction: flexwm_ipc::Vertical::Down,
            }));
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the workspace step was not served"
    );
    assert_eq!(
        fixture.state.focus, None,
        "stepping onto the empty workspace should empty focus"
    );
    fixture.assert_keyboard_follows_focus("a real workspace move");
    assert!(
        fixture.state.needs_render,
        "a real workspace move laid nothing out"
    );
}

#[test]
fn unknown_window_id_is_not_a_noop() {
    // The edge `focus_action_is_noop` is written around: an id that names no
    // window can never equal `State::focus`, so it stays on the full path
    // and keeps whatever handling `act` gives it today (which currently
    // still runs an `apply`). Focus does not move, but the click -- this is
    // still a focus-family action -- is spent, and the keyboard is
    // re-derived onto the window that kept focus.
    let mut fixture = Fixture::drive();
    let focused = fixture.state.focus.expect("a focused window");

    fixture.click_taskbar();
    fixture.state.needs_render = false;
    let response =
        fixture
            .state
            .handle_request(Request::Action(flexwm_ipc::Action::FocusWindowId {
                id: 999,
            }));
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the unknown-id action was not served"
    );
    assert_eq!(
        fixture.state.focus,
        Some(focused),
        "an unknown window id moved window focus"
    );
    fixture.assert_keyboard_follows_focus("an unknown window id");
    assert!(
        fixture.state.needs_render,
        "an unknown window id skipped the apply it has always run"
    );
}

#[test]
fn out_of_range_workspace_index_is_not_a_noop() {
    // Same edge one list over: no workspace 99 exists, and `active < count`
    // always, so the index can never compare equal -- full path, `apply`
    // included, active workspace unchanged.
    let mut fixture = Fixture::drive();
    let output = fixture
        .state
        .world
        .focused_output()
        .expect("a focused output");
    let before = fixture
        .state
        .world
        .workspaces(output)
        .expect("a workspace list");

    fixture.click_taskbar();
    fixture.state.needs_render = false;
    let response =
        fixture
            .state
            .handle_request(Request::Action(flexwm_ipc::Action::FocusWorkspaceIndex {
                index: 99,
            }));
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the out-of-range action was not served"
    );
    assert_eq!(
        fixture.state.world.workspaces(output),
        Some(before),
        "an out-of-range workspace index switched workspaces"
    );
    assert!(
        fixture.state.needs_render,
        "an out-of-range workspace index skipped the apply it has always run"
    );
}

#[test]
fn relative_steps_stay_on_the_full_path() {
    // The deliberate scope cut in `focus_action_is_noop`, pinned: a relative
    // step the core itself would no-op (left from the leftmost column, up
    // from the top of a one-window stack) still runs the full `apply`,
    // because no-op-ness there needs column/stack positions `World` does
    // not expose. If detection is ever added for these two variants, this
    // test and the helper's docs change together.
    let mut fixture = Fixture::drive();
    let focused = fixture.state.focus.expect("a focused window");
    let leftmost = *fixture.state.windows.keys().min().expect("two windows");
    // Park on the leftmost column first, so the Left step below is a core
    // no-op rather than a real move.
    if focused != leftmost {
        let response =
            fixture
                .state
                .handle_request(Request::Action(flexwm_ipc::Action::FocusWindowId {
                    id: leftmost.0,
                }));
        assert!(matches!(response, Response::Ok { .. }));
    }
    assert_eq!(fixture.state.focus, Some(leftmost));

    for (n, action) in [
        flexwm_ipc::Action::FocusColumn {
            direction: flexwm_ipc::Horizontal::Left,
        },
        flexwm_ipc::Action::FocusWindow {
            direction: flexwm_ipc::Vertical::Up,
        },
    ]
    .into_iter()
    .enumerate()
    {
        fixture.click_taskbar();
        fixture.state.needs_render = false;
        let response = fixture.state.handle_request(Request::Action(action));
        assert!(
            matches!(response, Response::Ok { locked: false }),
            "relative step {n} was not served"
        );
        assert_eq!(
            fixture.state.focus,
            Some(leftmost),
            "relative step {n} moved window focus"
        );
        fixture.assert_keyboard_follows_focus(&format!("relative step {n}"));
        assert!(
            fixture.state.needs_render,
            "relative step {n} skipped the apply it still runs"
        );
    }
}
#[test]
fn a_layout_action_over_ipc_leaves_a_clicked_taskbars_keyboard_alone() {
    // The boundary the predicate above draws: an action that changes
    // arrangement rather than where focus is reported to be must not spend a
    // deliberate keyboard placement. `CycleColumnWidth` never moves focus,
    // so the click -- and the keyboard it placed -- has to survive it.
    let mut fixture = Fixture::drive();
    let focus_before = fixture.state.focus;

    fixture.click_taskbar();
    let response = fixture
        .state
        .handle_request(Request::Action(flexwm_ipc::Action::CycleColumnWidth));
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the layout action was not served"
    );

    assert_eq!(
        fixture.state.focus, focus_before,
        "a layout action moved window focus"
    );
    assert_eq!(
        fixture.keyboard_surface(),
        Some(fixture.clicked_surface()),
        "a layout action ripped the keyboard out of the clicked taskbar"
    );
    assert!(
        fixture.state.clicked_layer.is_some(),
        "a layout action spent the taskbar's click"
    );
}
