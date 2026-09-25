//! `request_activation` spends a clicked `on_demand` layer surface's keyboard.
//!
//! The bug this pins: `request_activation` ended in
//! `act(FocusWindowId)` without first clearing `State::clicked_layer`, so a
//! launcher that stayed mapped kept every keystroke after handing window
//! focus away -- while the focus ring and `scoot msg windows` named the new
//! window. The weak shape of this test (hand-setting `clicked_layer` with no
//! real layer surface) passes either way and proves nothing; like
//! `foreign_toplevel_management`'s `taskbar_holding_the_keyboard`, this runs
//! a real mapped `on_demand` taskbar, a real pointer click through the input
//! path (the only writer of `clicked_layer`), then a real activation through
//! the token the client minted from a real key press, asserting on the seat's
//! actual keyboard focus surface.
//!
//! Two clients, the way the real gesture works: one maps the windows and
//! redeems the token (reusing this suite's own script, untouched), while a
//! second one is the taskbar -- a launcher is always somebody else's client.
//! The key press the token is minted from lands before the click, while the
//! second window still holds the keyboard; the click then moves the keyboard
//! to the taskbar, and the client's own activation moves window focus away.
//! With the `clicked_layer = None` line removed from `request_activation`
//! the final keyboard assertion fails, because `layer_keyboard_focus`
//! re-derives the still-mapped taskbar.
//!
//! A second test pins the lock gate's ordering the same way: taskbar clicked,
//! session locked (a third client holding `ext_session_lock_v1`, no lock
//! surface, like `foreign_toplevel_management`'s `LockSession` step), then a
//! direct `request_activation` that must be refused without spending the
//! click -- and the keyboard must still be on the taskbar after the unlock.
//! Moving the clear above the `is_locked()` check fails that test.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::*;
use crate::compositor::layer_shell::ABOVE_WINDOWS;
use crate::compositor::test_support::wait_for;
use smithay::desktop::{LayerSurface, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// The framebuffer the headless backend renders into. Nothing here reads a
/// pixel; the backend exists so the compositor has a real output with a real
/// layer map, which [`Harness::bare`] does not build.
const CANVAS: i32 = 200;

/// How wide and tall the `on_demand` taskbar is, anchored to the
/// bottom-right corner -- the shape
/// `foreign_toplevel_management/tests` uses, for the same reason: the layout
/// puts columns from the left edge, so the corner stays outside any window.
const TASKBAR: u32 = 60;

/// A point inside that taskbar, for a click that really goes through the
/// pointer.
const TASKBAR_POINT: (f64, f64) = (CANVAS as f64 - 20.0, CANVAS as f64 - 20.0);

/// A live compositor with a real headless backend and two connected clients,
/// scripted as [`Harness`] describes. Client 0 maps the windows and redeems
/// the token; client 1 is the taskbar.
type Fixture = Harness<(), Ack>;

/// The taskbar end: enough of a shell to map one `on_demand` layer surface
/// with a real `wl_shm` buffer behind it, which is what it takes to be in the
/// compositor's layer map and therefore eligible to hold the keyboard.
#[derive(Default)]
struct TaskbarClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    /// The size the compositor configured the taskbar at, once it has. A
    /// layer surface may only attach a buffer after acking a configure, and
    /// must draw at the size that configure carried.
    taskbar_size: Option<(u32, u32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for TaskbarClient {
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
            // Version 4: the taskbar reports damage with `damage_buffer`,
            // which does not exist at version 1.
            client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == zwlr_layer_shell_v1::ZwlrLayerShellV1::interface().name {
            client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for TaskbarClient {
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

wayland_client::delegate_noop!(TaskbarClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TaskbarClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TaskbarClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TaskbarClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TaskbarClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TaskbarClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// A `width`x`height` opaque `wl_buffer` over a real memfd -- the same path
/// any toolkit takes, and what a layer surface needs before it counts as
/// mapped.
fn taskbar_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TaskbarClient>,
    width: i32,
    height: i32,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create(
        "scoot-activation-keyboard-test",
        rustix::fs::MemfdFlags::CLOEXEC,
    )
    .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len])
        .map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

/// Maps the taskbar, reports it, and parks until the test disconnects it --
/// returning (and so unmapping the taskbar) as soon as the client thread ends
/// would spend the click under test from underneath it.
fn run_taskbar(stream: UnixStream, steps: Receiver<()>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TaskbarClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let shell = client
        .layer_shell
        .clone()
        .ok_or("no zwlr_layer_shell_v1 -- the global is missing")?;
    let surface = compositor.create_surface(&qh, ());
    let layer = shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "scoot-activation-test-taskbar".into(),
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
    // The first commit carries no buffer; the protocol requires that before
    // the first configure.
    surface.commit();
    let (width, height) = wait_for(&mut queue, &mut client, "a taskbar configure", |client| {
        client.taskbar_size
    })?;
    let buffer = taskbar_buffer(&shm, &qh, width as i32, height as i32)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, width as i32, height as i32);
    surface.commit();
    // Held so the taskbar stays mapped: dropping the role object or the
    // buffer would unmap it, which is exactly what must not happen while the
    // activation is trying to take the keyboard away from it.
    let _taskbar = (surface, layer, buffer);

    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::TaskbarMapped).map_err(|e| e.to_string())?;
    while steps.recv().is_ok() {}
    Ok(())
}

/// A left click at a point, press and release, the way a user makes one --
/// the same helper `foreign_toplevel_management/tests` uses, for the same
/// reason: `State::clicked_layer` is only ever written by a click that
/// really went through the pointer.
fn click(fixture: &mut Fixture, x: f64, y: f64) {
    fixture.state.pointer_move(x, y);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
}

/// Settles until the compositor has dispatched the taskbar's buffer commit,
/// so a click at [`TASKBAR_POINT`] really lands on it.
///
/// The ack only proves the *client* finished its own round trip, not that
/// the compositor has dispatched the buffer commit yet -- without this a
/// click can run against a layer map that does not have the taskbar's
/// surface in it yet and land on whatever is behind it instead. A single
/// `settle()` held until it didn't (a 2-in-80 trip under deliberately
/// abusive multi-process oversubscription, always at the `clicked_layer`
/// precondition, never past it -- settle-insufficiency in the test, not a
/// production race). The wait is on the exact hit test the click itself
/// uses (`State::layer_under`, the same `layer_hit` `focus_under_pointer`
/// clicks through), so it proves the click's precondition rather than
/// hoping a fixed dispatch count covers it; bounded so a taskbar that never
/// maps still fails loudly instead of hanging the suite.
fn settle_until_taskbar_hit(fixture: &mut Fixture) {
    for _ in 0..100 {
        fixture.settle();
        if fixture
            .state
            .layer_under(&ABOVE_WINDOWS, TASKBAR_POINT.into())
            .is_some()
        {
            return;
        }
    }
    panic!("the taskbar never became hittable -- check TASKBAR_POINT against the layout");
}

/// The `wl_surface` the seat's keyboard focus is actually on, which is the
/// only unambiguous answer to "where do keystrokes go".
fn keyboard_surface(fixture: &Fixture) -> Option<WlSurface> {
    fixture
        .state
        .seat
        .get_keyboard()
        .expect("a keyboard")
        .current_focus()
        .map(WlSurface::from)
}

/// The `wl_surface` of the clicked layer surface itself, so the
/// precondition can assert the keyboard is on exactly that surface rather
/// than merely off the window.
fn clicked_surface(fixture: &Fixture) -> WlSurface {
    fixture
        .state
        .clicked_layer
        .as_ref()
        .expect("a clicked layer surface")
        .wl_surface()
        .clone()
}

/// The `wl_surface` of window `id`'s toplevel.
fn window_surface(fixture: &Fixture, id: WindowId) -> WlSurface {
    fixture
        .state
        .windows
        .get(&id)
        .and_then(Window::toplevel)
        .expect("a live window")
        .wl_surface()
        .clone()
}

/// Two windows mapped by client 0, a taskbar mapped by client 1 and clicked
/// so it holds the keyboard, then the client's own activation of the first
/// window -- redeemed from the key press that landed while the second window
/// still had it. Returns the compositor, the client's report, and the window
/// focus as it was just before the activation landed, so the test can show
/// the activation moved something rather than re-asserting where it started.
fn drive_with_taskbar() -> (Fixture, Run, Option<WindowId>) {
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    fixture.spawn(|stream, _steps, acks| {
        map_two_and_activate_first(stream, true, 0, Claim::RealKeyPress, acks)
    });
    fixture.spawn(run_taskbar);

    let Ack::Mapped = fixture.wait_for_ack(0) else {
        panic!("the window client reported it was done before it reported being mapped");
    };
    let Ack::TaskbarMapped = fixture.wait_for_ack(1) else {
        panic!("the taskbar client never mapped its layer surface");
    };
    // Settle-until-hittable, not settle-once: the ack only proves the
    // *client* finished its own round trip, not that the compositor has
    // dispatched the buffer commit yet.
    settle_until_taskbar_hit(&mut fixture);
    // Mapping the second window focused it, and mapping the taskbar never
    // steals anything -- so this is the window the key press below reaches,
    // and the one the activation later has to move focus away from.
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "the second window should have focus before anything is clicked"
    );

    // While the second window still holds the keyboard: what the client
    // mints its token from, the way a launcher mints from inside its own
    // input handler.
    press_a_key(&mut fixture);
    // Then the launcher gesture itself: a real click on the taskbar, which
    // takes the keyboard without moving window focus. Pressed and asserted
    // synchronously, with no dispatch in between: the window client fires
    // its `activate` the moment the key press above reaches it, and any
    // dispatch before these assertions would let the compositor spend the
    // click under test -- exactly the production behavior the test below
    // goes on to pin -- before they run. That is a second, distinct
    // load-only trip at this same line (1-in-40 under the oversubscription
    // that caught the settle half, after the settle half had already proven
    // the click hittable on the same thread with no dispatch since), not a
    // missed click and not a production race: the click landed, the
    // activation spent it, and the assertion looked too late.
    fixture.state.pointer_move(TASKBAR_POINT.0, TASKBAR_POINT.1);
    fixture.state.pointer_button(PointerButton::Left, true);
    assert!(
        fixture.state.clicked_layer.is_some(),
        "the click never reached the taskbar -- check TASKBAR_POINT against the layout"
    );
    assert!(
        fixture.state.keyboard_on_layer,
        "an on_demand layer surface should hold the keyboard once clicked"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(clicked_surface(&fixture)),
        "the keyboard is not on the taskbar the click landed on"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "clicking a bar must not move *window* focus"
    );
    // Read before the release, not after the settle: the window client fires
    // its `activate` the moment the key press reaches it, and any dispatch
    // past this point -- the release's settle, the `Done` wait -- may already
    // have moved focus to the first window, which would make `focus_before`
    // lie about what the activation had to move away from. Same racing client
    // as the press-assert ordering above, third trip at the same test.
    let focus_before = fixture.state.focus;
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    // Flushed explicitly because the client is blocked reading rather than
    // writing, and the display source only pushes events out when the
    // *client* writes.
    let _ = fixture.state.display_handle.flush_clients();

    let Ack::Done(run) = fixture.wait_for_ack(0) else {
        panic!("the window client reported being mapped twice");
    };
    (fixture, *run, focus_before)
}

#[test]
fn an_activation_takes_the_keyboard_back_from_a_clicked_taskbar() {
    // A launcher that stays mapped: clicked (it takes the keyboard), then it
    // hands focus over with `xdg-activation-v1`. Window focus must move *and*
    // the keyboard must follow it -- without the `clicked_layer = None` line
    // in `request_activation`, `layer_keyboard_focus` re-derives the
    // still-mapped taskbar and only the first half happens, which is worse
    // than either end of it: the user cannot see where their typing is going.
    let (fixture, run, focus_before) = drive_with_taskbar();
    let first = window_of(&fixture, run.first_surface);
    assert_ne!(
        focus_before,
        Some(first),
        "the first window already had focus, so activating it would prove nothing"
    );

    assert_eq!(
        fixture.state.focus,
        Some(first),
        "the activated window did not get window focus"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(window_surface(&fixture, first)),
        "activating a window left the keyboard on the taskbar"
    );
    assert!(!fixture.state.keyboard_on_layer);
    assert!(
        fixture.state.clicked_layer.is_none(),
        "the taskbar's click was not spent"
    );
}

// -------------------------------------------------------------------------
// The lock gate: a refused activation must disturb nothing
// -------------------------------------------------------------------------

/// The window end for the lock test: two bare `xdg_toplevel`s, held for the
/// whole test. Bare, like `foreign_toplevel_management/tests` uses them:
/// what puts a window in scoot's model is the toplevel existing, and the
/// activation below is driven by a direct `request_activation` call -- the
/// serial gate lives at token creation, the ordering under test lives in
/// this handler's tail -- so no key press, no buffers, and no token round
/// trip are needed.
#[derive(Default)]
struct WindowsClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`.
    window_serials: Vec<Option<u32>>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for WindowsClient {
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
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, WindowIndex> for WindowsClient {
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

impl Dispatch<xdg_toplevel::XdgToplevel, WindowIndex> for WindowsClient {
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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WindowsClient {
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

wayland_client::delegate_noop!(WindowsClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(WindowsClient: ignore wl_surface::WlSurface);

/// A window's own index in creation order, so its configure can be matched
/// back to it.
struct WindowIndex(usize);

/// Maps one bare window and acks its configure, returning everything the
/// caller must hold: dropping a role object would destroy the window it
/// stands for.
fn map_bare_window(
    compositor: &wl_compositor::WlCompositor,
    wm_base: &xdg_wm_base::XdgWmBase,
    qh: &QueueHandle<WindowsClient>,
    queue: &mut wayland_client::EventQueue<WindowsClient>,
    client: &mut WindowsClient,
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

/// Maps two windows, reports them, and parks holding them until the test
/// disconnects it -- returning early would destroy the windows out from
/// under the assertions.
fn run_windows(stream: UnixStream, steps: Receiver<()>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = WindowsClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    // Held so the windows stay mapped: dropping a role object would destroy
    // the window it stands for.
    let windows = vec![
        map_bare_window(&compositor, &wm_base, &qh, &mut queue, &mut client)?,
        map_bare_window(&compositor, &wm_base, &qh, &mut queue, &mut client)?,
    ];
    let _windows = windows;

    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::WindowsMapped).map_err(|e| e.to_string())?;
    while steps.recv().is_ok() {}
    Ok(())
}

/// The lock end: takes the session lock the way a lock screen does --
/// `ext_session_lock_manager_v1.lock`, held for the whole test -- and maps
/// no lock surface, like `foreign_toplevel_management`'s `LockSession` step.
/// The one `()` this client ever receives means "unlock now", answered with
/// a real `unlock_and_destroy` rather than a disconnect, which would abandon
/// the lock instead of ending it.
#[derive(Default)]
struct LockerClient {
    locks: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    locked: u32,
    finished: u32,
}

impl Dispatch<wl_registry::WlRegistry, ()> for LockerClient {
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
        if interface == ext_session_lock_manager_v1::ExtSessionLockManagerV1::interface().name {
            client.locks = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for LockerClient {
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

wayland_client::delegate_noop!(LockerClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

fn run_locker(stream: UnixStream, steps: Receiver<()>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = LockerClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let manager = client
        .locks
        .clone()
        .ok_or("no ext_session_lock_manager_v1 -- the global is missing")?;
    let seen = client.locked + client.finished;
    // Held so the session stays locked: dropping the lock object ends the
    // lock, which is exactly what must not happen until the test unlocks.
    let lock = manager.lock(&qh, ());
    // Exactly one of the two must arrive, and the protocol says so in as
    // many words: "In response to the creation of this object the compositor
    // must send either the locked or finished event." The shape
    // `session_lock/tests` uses for its own `Lock` step.
    wait_for(&mut queue, &mut client, "locked or finished", |client| {
        (client.locked + client.finished > seen).then_some(())
    })?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::Locked).map_err(|e| e.to_string())?;

    steps.recv().map_err(|e| e.to_string())?;
    lock.unlock_and_destroy();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::Unlocked).map_err(|e| e.to_string())?;
    while steps.recv().is_ok() {}
    Ok(())
}

/// Two parked windows mapped by client 0, the taskbar mapped by client 1
/// and clicked so it holds the keyboard, then the session locked by client
/// 2. Returns the compositor and the clicked layer surface itself, so the
/// test can show the refusal preserved exactly that click.
fn drive_locked_with_taskbar() -> (Fixture, LayerSurface) {
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    fixture.spawn(run_windows);
    fixture.spawn(run_taskbar);
    let Ack::WindowsMapped = fixture.wait_for_ack(0) else {
        panic!("the window client never mapped its windows");
    };
    let Ack::TaskbarMapped = fixture.wait_for_ack(1) else {
        panic!("the taskbar client never mapped its layer surface");
    };
    // As in `drive_with_taskbar`: settle-until-hittable, not settle-once --
    // the ack only proves the *client* finished, not that the compositor
    // dispatched the commits yet.
    settle_until_taskbar_hit(&mut fixture);
    // The second window mapped last, so it has window focus -- the one the
    // refused activation below must leave alone.
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "the second window should have focus before anything is clicked"
    );

    // The launcher gesture: a real click on the taskbar, which takes the
    // keyboard without moving window focus.
    click(&mut fixture, TASKBAR_POINT.0, TASKBAR_POINT.1);
    assert!(
        fixture.state.clicked_layer.is_some(),
        "the click never reached the taskbar -- check TASKBAR_POINT against the layout"
    );
    assert!(
        fixture.state.keyboard_on_layer,
        "an on_demand layer surface should hold the keyboard once clicked"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(clicked_surface(&fixture)),
        "the keyboard is not on the taskbar the click landed on"
    );
    let taskbar = fixture
        .state
        .clicked_layer
        .clone()
        .expect("the taskbar holds the click");

    fixture.spawn(run_locker);
    let Ack::Locked = fixture.wait_for_ack(2) else {
        panic!("the locker client never took the session lock");
    };
    fixture.settle();
    assert!(fixture.state.session_lock.is_locked());
    // The precondition this test is named for: locking itself must not move
    // window focus, or the refusal below would exercise nothing.
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "locking the session moved window focus"
    );
    (fixture, taskbar)
}

#[test]
fn a_refused_activation_while_locked_spends_neither_focus_nor_the_taskbars_click() {
    // The lock gate's ordering, pinned: `request_activation` refuses before
    // touching anything, because what follows a honored request spends the
    // launcher's click -- and a refused request must not disturb anything, so
    // the session comes back as the user left it. Moving the
    // `clicked_layer = None` line above the `is_locked()` check keeps every
    // other test in this file green while silently spending the taskbar's
    // click here: on unlock the keyboard would be on the window instead of
    // the taskbar.
    //
    // Driven by a direct call, like the `token_aged` tests: the serial gate
    // lives at token creation, the ordering under test lives in this
    // handler's tail.
    let (mut fixture, taskbar) = drive_locked_with_taskbar();

    let surface = window_surface(&fixture, WindowId(1));
    let (token, data) = token_aged(Duration::from_secs(0));
    fixture.state.request_activation(token, data, surface);

    assert!(
        fixture.state.session_lock.is_locked(),
        "the session came unlocked on its own mid-test"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "a locked session's focus was moved by an activation"
    );
    assert_eq!(
        fixture.state.clicked_layer.as_ref(),
        Some(&taskbar),
        "a refused activate spent the taskbar's click anyway"
    );
    assert_ne!(
        keyboard_surface(&fixture),
        Some(window_surface(&fixture, WindowId(1))),
        "a locked session gave the keyboard to a window on a client's request"
    );
    // Deliberately not asserting the keyboard is still on the taskbar *while*
    // locked: with no lock surface mapped the seat has no focus at all, and
    // the lock guarantees no window does -- which is the half that matters
    // here. The click's survival is what brings the keyboard back below.

    // Unlock the way a lock screen does after auth, and the session must
    // come back as the user left it: same window focus, same click, and the
    // keyboard back on the taskbar it was clicked onto.
    let Ack::Unlocked = fixture.run_on(2, ()) else {
        panic!("the locker client never unlocked the session");
    };
    assert!(!fixture.state.session_lock.is_locked());
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "unlocking moved window focus"
    );
    assert_eq!(
        fixture.state.clicked_layer.as_ref(),
        Some(&taskbar),
        "the taskbar's click did not survive the lock"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(clicked_surface(&fixture)),
        "unlocking did not hand the keyboard back to the clicked taskbar"
    );
}
