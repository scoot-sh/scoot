//! Tests for `text-input-v3` and `input-method-v2`.
//!
//! An input-method popup cannot be conjured from the compositor side: it only
//! exists once a real `zwp_input_method_v2` has been bound, a real
//! `zwp_text_input_v3` has been enabled on a focused surface, and the input
//! method has asked for a popup against it. So the live tests here drive one
//! `wayland-client` connection that plays *both* halves -- the application
//! with the text field and the input method composing into it -- through a
//! real [`State`], the approach `shell/tests.rs` established.
//!
//! One connection rather than two is deliberate and not a shortcut: Smithay
//! requires the text input and the focused surface to belong to the same
//! client (`text_input_handle.rs` discards a request "for unfocused client"
//! by comparing client ids), and a second connection would only add a second
//! focus to keep in step without testing anything this module owns.
//!
//! [`State::parent_geometry`] is the one piece with an answer that does not
//! need any of that, so it is asked directly.
//!
//! Like every other live-[`State`] test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use scoot_core::{Config, Rect, WindowId};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::reexports::wayland_server::{Client, Display};
use wayland_client::protocol::{wl_compositor, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2, zwp_input_method_v2, zwp_input_popup_surface_v2,
};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

const OUTPUT: Rect = Rect::new(0, 0, 1600, 1000);
const PATIENCE: Duration = Duration::from_secs(10);

/// One instruction for the client thread.
enum Step {
    /// `zwp_text_input_v3.enable` + `commit`: the application says its text
    /// field is ready, which is what activates the input method.
    EnableTextInput,
    /// `zwp_text_input_v3.disable` + `commit`: the field went away.
    DisableTextInput,
    /// `zwp_input_method_v2.get_input_popup_surface`: the input method asks
    /// for a candidate window. Reports the popup surface's protocol id.
    CreatePopup,
    /// Move the text cursor within the field, which the popup follows.
    SetCursorRectangle { x: i32, y: i32, w: i32, h: i32 },
}

/// Both halves of the IME pair, plus enough of a toolkit to map a toplevel.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    /// `None` is what `both_globals_are_advertised` fails on.
    text_input_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    input_method_manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    /// Whether the compositor told the input method it is now active, i.e.
    /// whether the two halves actually met.
    activated: bool,
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
            client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_seat::WlSeat::interface().name {
            client.seat = Some(registry.bind(name, version.min(5), qh, ()));
        } else if interface == zwp_text_input_manager_v3::ZwpTextInputManagerV3::interface().name {
            client.text_input_manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface
            == zwp_input_method_manager_v2::ZwpInputMethodManagerV2::interface().name
        {
            client.input_method_manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_input_method_v2::ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => client.activated = true,
            zwp_input_method_v2::Event::Deactivate => client.activated = false,
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

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClient {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3);
wayland_client::delegate_noop!(TestClient: ignore zwp_text_input_v3::ZwpTextInputV3);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_input_method_manager_v2::ZwpInputMethodManagerV2
);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2
);

/// What the client reports back once it is set up.
struct Ready {
    /// The toplevel's `wl_surface` protocol id, so the compositor side can
    /// name the window the popup should be parented to.
    window_surface: u32,
}

/// Binds both halves, maps one toplevel, then runs whatever steps arrive.
///
/// The input method is created *before* the toplevel maps, deliberately:
/// Smithay only sends `zwp_text_input_v3.enter` when an input method instance
/// already exists at the moment keyboard focus lands, so an IME bound
/// afterwards would never see this focus.
fn run_client(
    stream: UnixStream,
    ready: Sender<Ready>,
    steps: Receiver<Step>,
    acks: Sender<Option<u32>>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let seat = client.seat.clone().ok_or("no wl_seat")?;
    let text_inputs = client
        .text_input_manager
        .clone()
        .ok_or("no zwp_text_input_manager_v3")?;
    let input_methods = client
        .input_method_manager
        .clone()
        .ok_or("no zwp_input_method_manager_v2")?;

    let input_method = input_methods.get_input_method(&seat, &qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_title("ime-probe".to_string());
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let text_input = text_inputs.get_text_input(&seat, &qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    ready
        .send(Ready {
            window_surface: surface.id().protocol_id(),
        })
        .map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut reported = None;
        match step {
            Step::EnableTextInput => {
                text_input.enable();
                text_input.commit();
                // Reported back so a test can assert the compositor actually
                // told the input method it is now active, rather than
                // inferring it from a later side effect.
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                reported = Some(u32::from(client.activated));
            }
            Step::DisableTextInput => {
                text_input.disable();
                text_input.commit();
            }
            Step::CreatePopup => {
                let popup = compositor.create_surface(&qh, ());
                input_method.get_input_popup_surface(&popup, &qh, ());
                reported = Some(popup.id().protocol_id());
            }
            Step::SetCursorRectangle { x, y, w, h } => {
                text_input.set_cursor_rectangle(x, y, w, h);
                text_input.commit();
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(reported).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with one client that is both the application and the
/// input method.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    client: Client,
    steps: Option<Sender<Step>>,
    acks: Receiver<Option<u32>>,
    thread: Option<thread::JoinHandle<Result<(), String>>>,
    window_surface: u32,
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
        // The real headless backend rather than a bare `OutputAdded` into the
        // core: a layer surface needs an actual `Output` in `State::output`
        // and a layer map to be mapped into (`layer_shell.rs` returns early
        // without one), and `parent_geometry`'s layer branch reads the same
        // output. `layer_shell/tests.rs` stands its compositor up the same
        // way, for the same reason.
        crate::compositor::headless::init(&mut state, OUTPUT.w, OUTPUT.h)
            .expect("a headless backend");

        let (server, client_end) = UnixStream::pair().expect("a socket pair");
        let client: Client = state
            .display_handle
            .insert_client(server, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (ready_tx, ready_rx) = channel();
        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let handle = thread::spawn(move || run_client(client_end, ready_tx, step_rx, ack_tx));

        let mut fixture = Self {
            event_loop,
            state,
            client,
            steps: Some(step_tx),
            acks: ack_rx,
            thread: Some(handle),
            window_surface: 0,
        };
        let ready = fixture.wait_for(&ready_rx, "the client's setup report");
        fixture.window_surface = ready.window_surface;
        assert!(
            fixture.state.focus.is_some(),
            "the client's toplevel never got focus, so no text field can be focused either"
        );
        fixture
    }

    fn pump(&mut self) {
        self.event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut self.state)
            .expect("a compositor dispatch");
    }

    fn wait_for<T>(&mut self, channel: &Receiver<T>, what: &str) -> T {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Ok(value) = channel.try_recv() {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the client thread stopped or the compositor did"
            );
            self.pump();
        }
    }

    /// Runs one client step, returning whatever protocol id it reported.
    fn run(&mut self, step: Step) -> Option<u32> {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        let reported = self.wait_for(&acks, "a client step acknowledgement");
        self.acks = acks;
        reported
    }

    fn server_surface(&self, protocol_id: u32) -> ServerSurface {
        self.client
            .object_from_protocol_id(&self.state.display_handle, protocol_id)
            .expect("the client's surface")
    }

    /// Whether `popup` is tracked as a popup of the client's toplevel, which
    /// is what makes it render (see the module doc).
    fn popup_is_on_the_window(&self, popup: u32) -> bool {
        let window = self.server_surface(self.window_surface);
        let popup = self.server_surface(popup);
        PopupManager::popups_for_surface(&window).any(|(kind, _)| *kind.wl_surface() == popup)
    }

    fn window(&self) -> WindowId {
        self.state.focus.expect("a focused window")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.steps = None;
        if let Some(handle) = self.thread.take() {
            let deadline = Instant::now() + PATIENCE;
            while !handle.is_finished() && Instant::now() < deadline {
                let _ = self
                    .event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state);
            }
            if let Ok(Err(error)) = handle.join() {
                // Not an assert: a panicking `Drop` while another assertion
                // is already unwinding aborts the process and hides it.
                eprintln!("the test client failed: {error}");
            }
        }
    }
}

#[test]
fn both_globals_are_advertised() {
    // `foot` prints "text input interface not implemented by compositor; IME
    // will be disabled" on the first of these missing. The client above fails
    // with "no zwp_text_input_manager_v3" / "no zwp_input_method_manager_v2"
    // before reporting ready, so reaching a fixture at all is the assertion.
    let _fixture = Fixture::new();
}

#[test]
fn an_enabled_text_field_activates_the_input_method() {
    // The two halves meeting, which every popup test below depends on: with
    // no `activate` the input method has no field to compose into and
    // `get_input_popup_surface` would never reach `new_popup`.
    let mut fixture = Fixture::new();
    assert_eq!(
        fixture.run(Step::EnableTextInput),
        Some(1),
        "the input method was never sent `activate` for the focused text field"
    );
    let popup = fixture.run(Step::CreatePopup).expect("a popup surface id");
    assert!(
        fixture
            .state
            .popups
            .find_popup(&fixture.server_surface(popup))
            .is_some(),
        "the input method's popup was never tracked, so the two halves did not meet"
    );
}

#[test]
fn an_input_method_popup_is_tracked_against_the_focused_window() {
    // What actually makes it render: a `Window`'s render elements draw
    // `PopupManager::popups_for_surface`, so a popup tracked anywhere else --
    // or not at all -- is a candidate window that never appears.
    let mut fixture = Fixture::new();
    fixture.run(Step::EnableTextInput);
    let popup = fixture.run(Step::CreatePopup).expect("a popup surface id");
    assert!(
        fixture.popup_is_on_the_window(popup),
        "the popup is not among the focused window's popups"
    );
}

#[test]
fn every_popup_transition_asks_for_the_frame_that_shows_it() {
    // Nothing commits on the popup's own paths -- it is created, moved or
    // re-parented without the client touching a buffer -- so without an
    // explicit `request_render` in each handler the candidate window would
    // sit invisible (or keep being drawn after dismissal) until some
    // unrelated event happened to mark the screen dirty.
    //
    // The three handlers are called *directly* rather than driven through
    // the client, deliberately: `needs_render` is cleared again by the very
    // next frame the compositor draws, so observing it after a client round
    // trip (which pumps the loop, and therefore renders) is a race. What
    // the client *can* observe -- that the popup is tracked and untracked --
    // is covered by the round-trip tests above.
    let mut fixture = Fixture::new();
    fixture.run(Step::EnableTextInput);
    let popup_id = fixture.run(Step::CreatePopup).expect("a popup surface id");
    let surface = fixture.server_surface(popup_id);
    let Some(PopupKind::InputMethod(popup)) = fixture.state.popups.find_popup(&surface) else {
        panic!("the input-method popup is not tracked, so there is nothing to transition");
    };

    for (what, call) in [
        (
            "new_popup",
            Box::new(|state: &mut State, popup: PopupSurface| state.new_popup(popup))
                as Box<dyn Fn(&mut State, PopupSurface)>,
        ),
        (
            "popup_repositioned",
            Box::new(|state: &mut State, popup: PopupSurface| state.popup_repositioned(popup)),
        ),
        (
            "dismiss_popup",
            Box::new(|state: &mut State, popup: PopupSurface| state.dismiss_popup(popup)),
        ),
    ] {
        fixture.state.needs_render = false;
        call(&mut fixture.state, popup.clone());
        assert!(fixture.state.needs_render, "{what} did not ask for a frame");
    }
}

#[test]
fn disabling_the_text_field_dismisses_the_popup() {
    // The other end of the lifecycle: the field goes away, the input method
    // is deactivated, and the candidate window must stop being drawn -- a
    // popup left tracked would keep rendering over whatever has focus next.
    let mut fixture = Fixture::new();
    fixture.run(Step::EnableTextInput);
    let popup = fixture.run(Step::CreatePopup).expect("a popup surface id");
    assert!(fixture.popup_is_on_the_window(popup));

    fixture.run(Step::DisableTextInput);
    assert!(
        !fixture.popup_is_on_the_window(popup),
        "the popup is still tracked against the window after the field was disabled"
    );
}

#[test]
fn moving_the_text_cursor_moves_the_popup_with_it() {
    // `set_cursor_rectangle` is how the application says where its caret is,
    // and it is what the candidate window is placed against. Asserted on the
    // popup's own rectangle rather than on a rendered frame, because that is
    // the value the render path reads per frame.
    let mut fixture = Fixture::new();
    fixture.run(Step::EnableTextInput);
    let popup_id = fixture.run(Step::CreatePopup).expect("a popup surface id");
    fixture.run(Step::SetCursorRectangle {
        x: 40,
        y: 60,
        w: 2,
        h: 18,
    });

    let surface = fixture.server_surface(popup_id);
    let Some(PopupKind::InputMethod(popup)) = fixture.state.popups.find_popup(&surface) else {
        panic!("the input-method popup is not tracked");
    };
    assert_eq!(
        popup.text_input_rectangle(),
        Rectangle::new((40, 60).into(), (2, 18).into()),
        "the caret rectangle never reached the popup"
    );
}

// -------------------------------------------------------------------------
// `parent_geometry`
// -------------------------------------------------------------------------

#[test]
fn a_windows_parent_geometry_is_the_windows_own() {
    // What the input method places its candidate window against. An empty
    // rectangle here would put every popup at the output's origin instead of
    // beside the text cursor.
    let fixture = Fixture::new();
    let surface = fixture.server_surface(fixture.window_surface);
    let expected = fixture
        .state
        .window(fixture.window())
        .expect("the focused window")
        .geometry();
    assert_eq!(fixture.state.parent_geometry(&surface), expected);
}

#[test]
fn an_unknown_surface_has_no_parent_geometry() {
    // A text field on a surface this compositor does not lay out -- a
    // session-lock surface's password box, most concretely. The popup is
    // placed at the origin rather than the lookup panicking or the IME being
    // refused a popup at all.
    let mut fixture = Fixture::new();
    let popup = fixture.run(Step::CreatePopup).expect("a popup surface id");
    let surface = fixture.server_surface(popup);
    assert_eq!(
        fixture.state.parent_geometry(&surface),
        Rectangle::default()
    );
}

// -------------------------------------------------------------------------
// `parent_geometry` for a layer surface
// -------------------------------------------------------------------------

/// A launcher-shaped layer surface: a centred panel, i.e. one whose position
/// on the output is *not* the origin. That is the whole point of the test
/// below -- the bug it guards is invisible at (0, 0).
const PANEL: (i32, i32) = (400, 200);

/// Maps one centred layer surface with a real committed buffer and reports
/// its `wl_surface`'s protocol id.
fn run_layer_client(stream: UnixStream, ready: Sender<u32>) -> Result<Connection, String> {
    use std::io::Write;
    use std::os::fd::AsFd;
    use wayland_client::protocol::{wl_buffer, wl_shm, wl_shm_pool};
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1, zwlr_layer_surface_v1,
    };

    #[derive(Default)]
    struct LayerClient {
        compositor: Option<wl_compositor::WlCompositor>,
        shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
        shm: Option<wl_shm::WlShm>,
    }
    impl Dispatch<wl_registry::WlRegistry, ()> for LayerClient {
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
                client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
            } else if interface == zwlr_layer_shell_v1::ZwlrLayerShellV1::interface().name {
                client.shell = Some(registry.bind(name, version.min(1), qh, ()));
            } else if interface == wl_shm::WlShm::interface().name {
                client.shm = Some(registry.bind(name, version.min(1), qh, ()));
            }
        }
    }
    impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for LayerClient {
        fn event(
            _: &mut Self,
            layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
            event: zwlr_layer_surface_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
                layer.ack_configure(serial);
            }
        }
    }
    wayland_client::delegate_noop!(LayerClient: ignore wl_compositor::WlCompositor);
    wayland_client::delegate_noop!(LayerClient: ignore wl_surface::WlSurface);
    wayland_client::delegate_noop!(LayerClient: ignore wl_shm::WlShm);
    wayland_client::delegate_noop!(LayerClient: ignore wl_shm_pool::WlShmPool);
    wayland_client::delegate_noop!(LayerClient: ignore wl_buffer::WlBuffer);
    wayland_client::delegate_noop!(LayerClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = LayerClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shell = client.shell.clone().ok_or("no zwlr_layer_shell_v1")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;

    let surface = compositor.create_surface(&qh, ());
    let layer = shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "ime-parent-probe".to_string(),
        &qh,
        (),
    );
    // No anchors: `LayerMap::arrange` centres it, which is what puts it at a
    // non-zero position on the output.
    layer.set_size(PANEL.0 as u32, PANEL.1 as u32);
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    // A real buffer, because `LayerSurface::geometry` reads the committed
    // surface view -- without one it is the default rectangle and the
    // assertion below could not tell the two candidate answers apart.
    let (w, h) = PANEL;
    let stride = w * 4;
    let len = (stride * h) as usize;
    let fd = rustix::fs::memfd_create("scoot-ime-layer", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len]).expect("a pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, &qh, ());
    let buffer = pool.create_buffer(0, w, h, stride, wl_shm::Format::Argb8888, &qh, ());
    pool.destroy();
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, w, h);
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    ready
        .send(surface.id().protocol_id())
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

#[test]
fn a_layer_surfaces_parent_geometry_is_surface_local_not_its_place_on_the_output() {
    // The bug this pins: `LayerMap::layer_geometry` is the surface's rectangle
    // *plus its position on the output*, and the pinned rev's layer render
    // path (`space/wayland/layer.rs`) subtracts what `parent_geometry`
    // returns without adding the position back -- it has already placed the
    // surface there. Returning the output-positioned rectangle therefore
    // cancels the placement out and drops the IME's candidate window at the
    // raw surface-local caret, interpreted as output coordinates: for this
    // centred panel, ~(600, 400) away from the field it belongs to.
    //
    // Asserted against *both* candidate answers rather than just the right
    // one, so the test cannot pass by coincidence on an output where the two
    // happen to agree.
    let mut fixture = Fixture::new();

    let (server, client_end) = UnixStream::pair().expect("a socket pair");
    let client: Client = fixture
        .state
        .display_handle
        .insert_client(server, Arc::new(ClientState::default()))
        .expect("an inserted layer client");
    let (ready_tx, ready_rx) = channel();
    let layer_thread = thread::spawn(move || {
        let result = run_layer_client(client_end, ready_tx);
        if let Err(error) = &result {
            eprintln!("LAYER CLIENT FAILED: {error}");
        }
        result
    });
    let surface_id = fixture.wait_for(&ready_rx, "the layer client's surface id");

    let surface: ServerSurface = client
        .object_from_protocol_id(&fixture.state.display_handle, surface_id)
        .expect("the layer client's surface");

    let output = fixture.state.output.clone().expect("an output");
    let (positioned, local) = {
        let map = smithay::desktop::layer_map_for_output(&output);
        let layer = map
            .layer_for_surface(&surface, smithay::desktop::WindowSurfaceType::TOPLEVEL)
            .expect("the layer surface is mapped")
            .clone();
        (
            map.layer_geometry(&layer).expect("a geometry"),
            layer.geometry(),
        )
    };

    // The premise: this really is a surface whose position is not the origin,
    // so the two answers really do differ here.
    assert_ne!(
        positioned.loc, local.loc,
        "the layer surface sits at the origin, so this test cannot distinguish \
         the two answers -- it needs a centred panel"
    );

    let answer = fixture.state.parent_geometry(&surface);
    assert_eq!(
        answer, local,
        "parent_geometry must return the surface-local geometry"
    );
    assert_ne!(
        answer, positioned,
        "parent_geometry returned the output-positioned rectangle; the IME \
         popup would be placed {:?} away from its text field",
        positioned.loc
    );

    // Joined only now: the thread returns the live `Connection`, and dropping
    // that disconnects the client and unmaps the layer surface the
    // assertions above are about.
    drop(layer_thread.join().expect("the layer client thread"));
}
