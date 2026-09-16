//! Tests for `xdg-toplevel-icon-v1`.
//!
//! Everything here is about what a *client* can put on a surface and what
//! comes back out of [`State::icon_name_of`], which is double-buffered state
//! Smithay owns -- there is no pure function in the middle to test. So these
//! drive a real `wayland-client` connection through a real [`State`], the
//! approach `shell/tests.rs` established, and then ask the compositor what it
//! would report to a bar or an agent.
//!
//! The commit boundary is the point of most of them: an icon is pending until
//! the surface commits, and a compositor that reported the pending one would
//! show a bar an icon the client has not published yet.
//!
//! Like every other live-[`State`] test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use flexwm_core::{Config, Event, OutputId, Rect};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols::xdg::toplevel_icon::v1::client::{
    xdg_toplevel_icon_manager_v1, xdg_toplevel_icon_v1,
};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

const OUTPUT: Rect = Rect::new(0, 0, 1600, 1000);
const PATIENCE: Duration = Duration::from_secs(10);

/// One instruction for the client thread. Steps are shipped one at a time
/// because what is under test is what the compositor sees *between* them.
enum Step {
    /// Build an icon with this name and attach it, without committing the
    /// surface afterwards.
    AttachIcon { name: String },
    /// Commit the toplevel's surface, publishing whatever is pending.
    Commit,
    /// `set_icon(toplevel, None)`: take the icon away again.
    ClearIcon,
    /// `set_name` on an icon that has already been assigned to a toplevel.
    /// A protocol error for the client -- and, unguarded, a compositor
    /// panic; see `dispatch.rs`.
    RenameAssignedIcon,
    /// `add_buffer` on an already-assigned icon: the same trap, other arm.
    AddBufferToAssignedIcon,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    icons: Option<xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1>,
    shm: Option<wl_shm::WlShm>,
    /// Every `icon_size` the manager advertised at bind time.
    advertised_sizes: Vec<i32>,
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
        } else if interface
            == xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1::interface().name
        {
            client.icons = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1,
        event: xdg_toplevel_icon_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel_icon_manager_v1::Event::IconSize { size } = event {
            client.advertised_sizes.push(size);
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
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel_icon_v1::XdgToplevelIconV1);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);

/// A square, shm-backed `wl_buffer` -- what `xdg_toplevel_icon_v1.add_buffer`
/// requires, so that a rejected `add_buffer` is rejected for its timing and
/// not for its contents.
fn square_shm_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    size: i32,
) -> wl_buffer::WlBuffer {
    let stride = size * 4;
    let len = (stride * size) as usize;
    let fd = rustix::fs::memfd_create("flexwm-icon-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0u8; len]).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, size, size, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Maps one toplevel, reports what the manager advertised, then runs whatever
/// steps arrive, acknowledging each.
fn run_client(
    stream: UnixStream,
    report: Sender<Vec<i32>>,
    steps: Receiver<Step>,
    acks: Sender<()>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let icons = client
        .icons
        .clone()
        .ok_or("no xdg_toplevel_icon_manager_v1")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    // The manager's `icon_size` list arrives at bind time, followed by
    // `done`; the roundtrip above already delivered both.
    report
        .send(client.advertised_sizes.clone())
        .map_err(|e| e.to_string())?;

    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_title("icon-probe".to_string());
    surface.commit();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    // The most recently created icon, kept so a step can poke it after it has
    // been assigned.
    let mut last_icon: Option<xdg_toplevel_icon_v1::XdgToplevelIconV1> = None;
    while let Ok(step) = steps.recv() {
        match step {
            Step::AttachIcon { name } => {
                let icon = icons.create_icon(&qh, ());
                icon.set_name(name);
                icons.set_icon(&toplevel, Some(&icon));
                last_icon = Some(icon.clone());
                // Not destroyed here: the icon object stays the toplevel's
                // until it is replaced, and destroying it before the commit
                // would race the attachment.
            }
            Step::Commit => surface.commit(),
            Step::ClearIcon => icons.set_icon(&toplevel, None),
            Step::RenameAssignedIcon => {
                let icon = last_icon.as_ref().ok_or("no icon to rename")?;
                icon.set_name("renamed-after-assignment".to_string());
            }
            Step::AddBufferToAssignedIcon => {
                let icon = last_icon.as_ref().ok_or("no icon to add a buffer to")?;
                // A real, square, shm-backed buffer, so the only thing wrong
                // with the request is *when* it is sent.
                let buffer = square_shm_buffer(&shm, &qh, 16);
                icon.add_buffer(&buffer, 1);
            }
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with one client that has one mapped toplevel.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    steps: Option<Sender<Step>>,
    acks: Receiver<()>,
    client: Option<thread::JoinHandle<Result<(), String>>>,
    /// The `icon_size` values the manager sent the client at bind time.
    advertised_sizes: Vec<i32>,
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
        state.world.handle_event(Event::OutputAdded {
            id: OutputId(1),
            area: OUTPUT,
        });

        let (server, client_end) = UnixStream::pair().expect("a socket pair");
        state
            .display_handle
            .insert_client(server, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (report_tx, report_rx) = channel();
        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let handle = thread::spawn(move || run_client(client_end, report_tx, step_rx, ack_tx));

        let mut fixture = Self {
            event_loop,
            state,
            steps: Some(step_tx),
            acks: ack_rx,
            client: Some(handle),
            advertised_sizes: Vec::new(),
        };
        fixture.advertised_sizes = fixture.wait_for(&report_rx, "the advertised icon sizes");
        // The client maps its toplevel right after reporting; drive the loop
        // until the compositor has a window, so every test starts from one.
        let deadline = Instant::now() + PATIENCE;
        while fixture.state.windows.is_empty() {
            assert!(
                Instant::now() < deadline,
                "the client's toplevel never reached the compositor"
            );
            fixture.pump();
        }
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

    /// Sends a step without waiting for its acknowledgement -- for a step
    /// that kills the client, which then never acknowledges anything.
    fn send(&mut self, step: Step) {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
    }

    fn run(&mut self, step: Step) {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        self.wait_for(&acks, "a client step acknowledgement");
        self.acks = acks;
    }

    /// The one window this fixture's client mapped.
    fn window(&self) -> WindowId {
        *self
            .state
            .windows
            .keys()
            .next()
            .expect("the client's window")
    }

    fn icon(&self) -> Option<String> {
        self.state.icon_name_of(self.window())
    }

    /// The `icon` field the `windows` IPC request would report for the one
    /// window, through the same builder that answers it.
    fn icon_over_ipc(&self) -> Option<String> {
        let id = self.window();
        self.state
            .window_snapshots()
            .into_iter()
            .find(|snapshot| snapshot.id == id.0)
            .expect("the window is in the snapshot list")
            .icon
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.steps = None;
        if let Some(handle) = self.client.take() {
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
fn the_icon_manager_is_advertised() {
    // The global itself: `foot` prints "compositor does not implement the
    // xdg-toplevel-icon protocol" on exactly this, and the client above fails
    // with "no xdg_toplevel_icon_manager_v1" if it is missing -- which
    // `Fixture::new` then never gets a window out of.
    let _fixture = Fixture::new();
}

#[test]
fn no_icon_sizes_are_advertised() {
    // Deliberate, not an omission -- see `State`'s field doc. flexwm draws no
    // icon anywhere, so it has no size to prefer, and an empty list is the
    // protocol's own way to say so. This is that decision pinned down: if a
    // future change starts advertising sizes, it should be because something
    // in the compositor began drawing icons at them.
    let fixture = Fixture::new();
    assert!(
        fixture.advertised_sizes.is_empty(),
        "the manager advertised icon sizes: {:?}",
        fixture.advertised_sizes
    );
}

#[test]
fn a_window_starts_with_no_icon() {
    let fixture = Fixture::new();
    assert_eq!(fixture.icon(), None);
    assert_eq!(fixture.icon_over_ipc(), None);
}

#[test]
fn an_attached_icon_is_not_reported_until_the_surface_commits() {
    // The whole reason `icon_name_of` reads `current()` rather than
    // `pending()`. A bar shown the pending icon would be showing one the
    // client has not published -- and, if the client never commits, one it
    // never will.
    let mut fixture = Fixture::new();
    fixture.run(Step::AttachIcon {
        name: "org.flexwm.Probe".to_string(),
    });
    assert_eq!(
        fixture.icon(),
        None,
        "a pending icon was reported before its commit"
    );

    fixture.run(Step::Commit);
    assert_eq!(fixture.icon(), Some("org.flexwm.Probe".to_string()));
}

#[test]
fn a_replacement_icon_takes_over_on_its_own_commit() {
    let mut fixture = Fixture::new();
    fixture.run(Step::AttachIcon {
        name: "first".to_string(),
    });
    fixture.run(Step::Commit);
    assert_eq!(fixture.icon(), Some("first".to_string()));

    fixture.run(Step::AttachIcon {
        name: "second".to_string(),
    });
    assert_eq!(
        fixture.icon(),
        Some("first".to_string()),
        "the replacement was reported before its commit"
    );
    fixture.run(Step::Commit);
    assert_eq!(fixture.icon(), Some("second".to_string()));
}

#[test]
fn clearing_the_icon_takes_it_away_again() {
    let mut fixture = Fixture::new();
    fixture.run(Step::AttachIcon {
        name: "org.flexwm.Probe".to_string(),
    });
    fixture.run(Step::Commit);
    assert_eq!(fixture.icon(), Some("org.flexwm.Probe".to_string()));

    fixture.run(Step::ClearIcon);
    fixture.run(Step::Commit);
    assert_eq!(fixture.icon(), None, "a cleared icon is still reported");
}

#[test]
fn the_icon_reaches_the_ipc_window_list() {
    // The end an agent or a bar actually reads. Asserted through the same
    // snapshot builder the `windows` request answers with, so an icon wired
    // up in `icon_name_of` but never read by `ipc.rs` fails here.
    let mut fixture = Fixture::new();
    fixture.run(Step::AttachIcon {
        name: "org.flexwm.Probe".to_string(),
    });
    fixture.run(Step::Commit);
    assert_eq!(
        fixture.icon_over_ipc(),
        Some("org.flexwm.Probe".to_string())
    );
}

/// Reproduces the upstream trap this protocol brings with it: a client that
/// touches an icon *after* assigning it to a toplevel.
///
/// The protocol says that is a client error (`immutable`), and the client
/// being disconnected for it is correct and expected. What must not happen is
/// the compositor going down with it -- see `dispatch.rs`'s guard. The
/// assertion is therefore not about the client at all: it is that the
/// compositor is still alive and still serving afterwards.
fn survives_a_frozen_icon_request(step: Step) {
    let mut fixture = Fixture::new();
    fixture.run(Step::AttachIcon {
        name: "org.flexwm.Probe".to_string(),
    });
    fixture.run(Step::Commit);
    assert_eq!(fixture.icon(), Some("org.flexwm.Probe".to_string()));

    // The offending request. The client is killed for it, so its own thread
    // may fail from here on -- which is why this does not go through
    // `Fixture::run` (that waits for an acknowledgement the dead client will
    // never send).
    fixture.send(step);
    // Pump well past the point the request has been dispatched. A panic in
    // the compositor unwinds *here*, inside `dispatch`, and fails the test.
    for _ in 0..50 {
        fixture.pump();
    }

    // Still serving: a fresh client can still connect and be answered, which
    // is the property that actually matters to every *other* client in a real
    // session.
    let (server, client_end) = UnixStream::pair().expect("a socket pair");
    fixture
        .state
        .display_handle
        .insert_client(server, Arc::new(ClientState::default()))
        .expect("the compositor still accepts clients");
    drop(client_end);
    fixture.pump();
}

#[test]
fn renaming_an_assigned_icon_does_not_take_the_compositor_down() {
    survives_a_frozen_icon_request(Step::RenameAssignedIcon);
}

#[test]
fn adding_a_buffer_to_an_assigned_icon_does_not_take_the_compositor_down() {
    survives_a_frozen_icon_request(Step::AddBufferToAssignedIcon);
}
