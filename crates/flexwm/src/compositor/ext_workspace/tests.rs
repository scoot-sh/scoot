//! Tests for `ext-workspace-v1`.
//!
//! These drive a *real* `wayland-client` connection -- binding
//! `ext_workspace_manager_v1` and reacting to its events the way a bar's
//! workspace module does -- through a real [`State`] with a real `headless`
//! backend, and assert on **the exact sequence of events the client
//! received**, in order.
//!
//! That is the whole point, and why none of these calls a handler directly:
//! everything that can go wrong with this protocol is a question about what
//! reached the client and in what order. A `removed` sent before the group let
//! go of the workspace, a second `state` event undoing the first inside one
//! batch, a `done` with nothing before it, a handle created for a workspace
//! that already had one -- every one of those is a correct-looking call
//! sequence on the compositor side and a bar that draws the wrong thing.
//!
//! Windows are created as bare `xdg_toplevel`s with no buffer: what changes
//! the workspace list is a window *existing* (`XdgShellHandler::new_toplevel`
//! reaches the core immediately), so nothing here needs `wl_shm` at all.
//!
//! Like the other integration-style tests in this crate, these need a
//! writable `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening
//! socket, which nothing here connects to (the client is inserted as a socket
//! pair) but which is created either way.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use flexwm_core::{Action, Config, Horizontal, Vertical};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_group_handle_v1::{
    self, ExtWorkspaceGroupHandleV1,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::{
    self, ExtWorkspaceHandleV1,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_manager_v1::{
    self, ExtWorkspaceManagerV1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// The framebuffer these tests render into. Nothing here reads a pixel; it
/// only has to be a valid size for the headless backend.
const CANVAS: i32 = 200;

/// The `active` bit of `ext_workspace_handle_v1.state`, and the empty set.
const ACTIVE: u32 = 1;
const INACTIVE: u32 = 0;

/// One protocol event, as the client saw it. Handles and groups are named by
/// the order the client was told about them, so an expectation can be written
/// out literally.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    /// `ext_workspace_manager_v1.workspace_group`
    Group(u32),
    /// `ext_workspace_group_handle_v1.capabilities`, as raw bits.
    GroupCapabilities(u32, u32),
    OutputEnter(u32),
    OutputLeave(u32),
    GroupRemoved(u32),
    /// `ext_workspace_manager_v1.workspace`
    Workspace(u32),
    Name(u32, String),
    /// The `coordinates` array, decoded back out of its native-endian bytes.
    Coordinates(u32, Vec<u32>),
    /// `ext_workspace_handle_v1.capabilities`, as raw bits.
    Capabilities(u32, u32),
    /// `ext_workspace_handle_v1.state`, as raw bits.
    State(u32, u32),
    WorkspaceEnter(u32, u32),
    WorkspaceLeave(u32, u32),
    Removed(u32),
    /// `ext_workspace_handle_v1.id`, which flexwm never sends -- recorded so
    /// that a test asserting on its absence would actually see one.
    Id(u32, String),
    Done(u32),
    Finished(u32),
}

/// The full burst for one workspace being created, in the order this
/// compositor sends it.
fn created(key: u32, group: u32, name: &str, coordinate: u32, active: bool) -> Vec<Seen> {
    vec![
        Seen::Workspace(key),
        Seen::Name(key, name.to_string()),
        Seen::Coordinates(key, vec![coordinate]),
        // `activate` and nothing else: see the module doc on capabilities.
        Seen::Capabilities(key, 1),
        Seen::State(key, if active { ACTIVE } else { INACTIVE }),
        Seen::WorkspaceEnter(group, key),
    ]
}

#[derive(Default)]
struct TestClient {
    /// Registry names, bound on demand so a test can control the order in
    /// which the client binds the output and the workspace manager.
    output_name: Option<(u32, u32)>,
    manager_name: Option<(u32, u32)>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    outputs: Vec<wl_output::WlOutput>,
    managers: Vec<ExtWorkspaceManagerV1>,
    groups: Vec<ExtWorkspaceGroupHandleV1>,
    handles: Vec<ExtWorkspaceHandleV1>,
    /// Every event since the last [`Step::TakeLog`], in arrival order.
    log: Vec<Seen>,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`.
    window_serials: Vec<Option<u32>>,
}

impl TestClient {
    fn manager_key(&self, manager: &ExtWorkspaceManagerV1) -> u32 {
        key_of(&self.managers, manager)
    }

    fn group_key(&self, group: &ExtWorkspaceGroupHandleV1) -> u32 {
        key_of(&self.groups, group)
    }

    fn handle_key(&self, handle: &ExtWorkspaceHandleV1) -> u32 {
        key_of(&self.handles, handle)
    }
}

/// The position of `proxy` in `known`, or `u32::MAX` for something the client
/// was never told about -- which shows up as a mismatch in an assertion
/// rather than as a panic in a dispatch handler.
fn key_of<P: Proxy + PartialEq>(known: &[P], proxy: &P) -> u32 {
    known
        .iter()
        .position(|other| other == proxy)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(u32::MAX)
}

/// Decodes a `coordinates` array the way the protocol defines it: an array of
/// `uint`s in the host's byte order.
fn decode_coordinates(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
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
            // Not bound here: which of these a client binds first is the
            // server's choice of registry order in real life, and one test
            // is specifically about binding them the awkward way round.
            "wl_output" => client.output_name = Some((name, version)),
            "ext_workspace_manager_v1" => client.manager_name = Some((name, version)),
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        manager: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let manager_key = client.manager_key(manager);
        match event {
            ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group } => {
                client.groups.push(workspace_group);
                let key = u32::try_from(client.groups.len() - 1).expect("few groups");
                client.log.push(Seen::Group(key));
            }
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                client.handles.push(workspace);
                let key = u32::try_from(client.handles.len() - 1).expect("few workspaces");
                client.log.push(Seen::Workspace(key));
            }
            ext_workspace_manager_v1::Event::Done => client.log.push(Seen::Done(manager_key)),
            ext_workspace_manager_v1::Event::Finished => {
                client.log.push(Seen::Finished(manager_key));
            }
            _ => {}
        }
    }

    event_created_child!(TestClient, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        group: &ExtWorkspaceGroupHandleV1,
        event: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let group_key = client.group_key(group);
        let seen = match event {
            ext_workspace_group_handle_v1::Event::Capabilities { capabilities } => {
                Seen::GroupCapabilities(group_key, bits(capabilities, |c| c.bits()))
            }
            ext_workspace_group_handle_v1::Event::OutputEnter { .. } => {
                Seen::OutputEnter(group_key)
            }
            ext_workspace_group_handle_v1::Event::OutputLeave { .. } => {
                Seen::OutputLeave(group_key)
            }
            ext_workspace_group_handle_v1::Event::WorkspaceEnter { workspace } => {
                Seen::WorkspaceEnter(group_key, client.handle_key(&workspace))
            }
            ext_workspace_group_handle_v1::Event::WorkspaceLeave { workspace } => {
                Seen::WorkspaceLeave(group_key, client.handle_key(&workspace))
            }
            ext_workspace_group_handle_v1::Event::Removed => Seen::GroupRemoved(group_key),
            _ => return,
        };
        client.log.push(seen);
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        handle: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = client.handle_key(handle);
        let seen = match event {
            ext_workspace_handle_v1::Event::Id { id } => Seen::Id(key, id),
            ext_workspace_handle_v1::Event::Name { name } => Seen::Name(key, name),
            ext_workspace_handle_v1::Event::Coordinates { coordinates } => {
                Seen::Coordinates(key, decode_coordinates(&coordinates))
            }
            ext_workspace_handle_v1::Event::Capabilities { capabilities } => {
                Seen::Capabilities(key, bits(capabilities, |c| c.bits()))
            }
            ext_workspace_handle_v1::Event::State { state } => {
                Seen::State(key, bits(state, |s| s.bits()))
            }
            ext_workspace_handle_v1::Event::Removed => Seen::Removed(key),
            _ => return,
        };
        client.log.push(seen);
    }
}

/// The raw bits behind a bitfield argument, whether or not this client's
/// generated bindings recognise every bit in it.
fn bits<T>(value: WEnum<T>, known: impl Fn(T) -> u32) -> u32 {
    match value {
        WEnum::Value(value) => known(value),
        WEnum::Unknown(raw) => raw,
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

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);

/// One instruction for the client thread.
enum Step {
    /// Bind `wl_output`. Deliberately separate from the registry pass so a
    /// test can bind it before or after the workspace manager.
    BindOutput,
    /// Bind another `ext_workspace_manager_v1`.
    BindManager,
    /// Create an `xdg_toplevel` (and ack its configure). No buffer: what
    /// changes the workspace list is the window existing.
    MapWindow,
    /// Destroy the `index`-th window.
    CloseWindow(usize),
    /// `activate` on the `index`-th workspace handle -- staged, not
    /// committed.
    Activate(usize),
    /// The requests flexwm never advertises a capability for, which it must
    /// therefore ignore without disconnecting anyone.
    Deactivate(usize),
    RemoveWorkspace(usize),
    AssignWorkspace {
        workspace: usize,
        group: usize,
    },
    CreateWorkspace(usize),
    /// `destroy` on the `index`-th workspace handle, while it is still a live
    /// workspace.
    DestroyHandle(usize),
    /// `commit` on the `index`-th manager.
    Commit(usize),
    /// `stop` on the `index`-th manager.
    Stop(usize),
    /// Hand back (and clear) everything seen so far.
    TakeLog,
}

enum Ack {
    Done,
    Log(Vec<Seen>),
}

/// Runs the client half: binds what it needs, then executes whatever steps
/// the test sends, acknowledging each one once the compositor has seen it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let manager_global = client
        .manager_name
        .ok_or("no ext_workspace_manager_v1 -- the global is missing")?;
    let output_global = client.output_name.ok_or("no wl_output")?;
    let mut windows: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    )> = Vec::new();

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match step {
            Step::BindOutput => {
                let output: wl_output::WlOutput =
                    registry.bind(output_global.0, output_global.1.min(4), &qh, ());
                client.outputs.push(output);
            }
            Step::BindManager => {
                let manager: ExtWorkspaceManagerV1 =
                    registry.bind(manager_global.0, manager_global.1.min(1), &qh, ());
                client.managers.push(manager);
            }
            Step::MapWindow => {
                let surface = compositor.create_surface(&qh, ());
                let index = client.window_serials.len();
                client.window_serials.push(None);
                let xdg = wm_base.get_xdg_surface(&surface, &qh, WindowIndex(index));
                let toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                for _ in 0..50 {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                    if let Some(serial) = client.window_serials[index] {
                        xdg.ack_configure(serial);
                        break;
                    }
                }
                windows.push((surface, xdg, toplevel));
            }
            Step::CloseWindow(index) => {
                // In this order, or the compositor answers with a protocol
                // error: an `xdg_surface` may only be destroyed after its
                // role object.
                let (surface, xdg, toplevel) = windows.get(index).ok_or("no such window")?.clone();
                toplevel.destroy();
                xdg.destroy();
                surface.destroy();
            }
            Step::Activate(index) => handle(&client, index)?.activate(),
            Step::Deactivate(index) => handle(&client, index)?.deactivate(),
            Step::RemoveWorkspace(index) => handle(&client, index)?.remove(),
            Step::AssignWorkspace { workspace, group } => {
                let group = client.groups.get(group).ok_or("no such group")?.clone();
                handle(&client, workspace)?.assign(&group);
            }
            Step::CreateWorkspace(index) => {
                let group = client.groups.get(index).ok_or("no such group")?;
                group.create_workspace("scratch".into());
            }
            Step::DestroyHandle(index) => handle(&client, index)?.destroy(),
            Step::Commit(index) => manager(&client, index)?.commit(),
            Step::Stop(index) => manager(&client, index)?.stop(),
            Step::TakeLog => outcome = Ack::Log(std::mem::take(&mut client.log)),
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        if let Ack::Log(log) = &mut outcome {
            // Anything that arrived during the round trip above belongs to
            // this batch too: the step before a `TakeLog` may have provoked
            // events that were still in flight when it was acknowledged.
            log.extend(std::mem::take(&mut client.log));
        }
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn handle(client: &TestClient, index: usize) -> Result<ExtWorkspaceHandleV1, String> {
    client
        .handles
        .get(index)
        .cloned()
        .ok_or_else(|| format!("no workspace handle {index}"))
}

fn manager(client: &TestClient, index: usize) -> Result<ExtWorkspaceManagerV1, String> {
    client
        .managers
        .get(index)
        .cloned()
        .ok_or_else(|| format!("no workspace manager {index}"))
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time -- the same shape `layer_shell/tests.rs` uses.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    clients: Vec<ClientHandle>,
}

/// One connected client thread and the channels driving it.
struct ClientHandle {
    /// `None` once the test has disconnected this client on purpose.
    steps: Option<Sender<Step>>,
    acks: Receiver<Ack>,
    thread: Option<JoinHandle<Result<(), String>>>,
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
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        let mut fixture = Self {
            event_loop,
            state,
            clients: Vec::new(),
        };
        fixture.spawn_client();
        fixture
    }

    /// Connects another, independent client. Its steps are addressed by the
    /// index this returns.
    fn spawn_client(&mut self) -> usize {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        self.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let thread = thread::spawn(move || run_client(client_end, step_rx, ack_tx));
        self.clients.push(ClientHandle {
            steps: Some(step_tx),
            acks: ack_rx,
            thread: Some(thread),
        });
        self.clients.len() - 1
    }

    /// A fixture whose client has bound the output and then the manager --
    /// the ordinary case every test that isn't about binding order starts
    /// from, with the initial burst already drained.
    fn bound() -> Self {
        let mut fixture = Self::new();
        fixture.run(Step::BindOutput);
        fixture.run(Step::BindManager);
        fixture.take_log();
        fixture
    }

    fn run(&mut self, step: Step) -> Ack {
        self.run_on(0, step)
    }

    fn run_on(&mut self, client: usize, step: Step) -> Ack {
        self.clients[client]
            .steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.clients[client].acks, channel().1);
        let ack = self.wait_for(client, &acks, "a client step acknowledgement");
        self.clients[client].acks = acks;
        self.settle();
        ack
    }

    /// Everything the client has seen since the last call.
    fn take_log(&mut self) -> Vec<Seen> {
        self.take_log_on(0)
    }

    fn take_log_on(&mut self, client: usize) -> Vec<Seen> {
        match self.run_on(client, Step::TakeLog) {
            Ack::Log(log) => log,
            Ack::Done => panic!("the client answered a log request with nothing"),
        }
    }

    fn wait_for<T>(&mut self, client: usize, channel: &Receiver<T>, what: &str) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match channel.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => {
                    let outcome = self.clients[client]
                        .thread
                        .take()
                        .map(|handle| handle.join().expect("the client thread"));
                    panic!("client {client} stopped while waiting for {what}: {outcome:?}");
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    /// A few dispatch cycles with nothing outstanding, so in-flight protocol
    /// traffic in both directions has been processed.
    fn settle(&mut self) {
        for _ in 0..10 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let _ = self.state.display_handle.flush_clients();
    }

    /// Applies an action compositor-side, the way a keybinding or an IPC
    /// request would.
    fn act(&mut self, action: Action) {
        self.state.act(action);
        self.settle();
    }

    /// What the core says right now, as `(count, active)`.
    fn workspaces(&self) -> (usize, usize) {
        let workspaces = self
            .state
            .world
            .workspaces(headless::OUTPUT_ID)
            .expect("the output exists");
        (workspaces.count, workspaces.active)
    }

    /// How many managers the compositor is keeping in step.
    fn registered_managers(&self) -> usize {
        self.state.ext_workspace.managers.len()
    }

    /// Disconnects a client and waits for the compositor to notice.
    fn disconnect_client(&mut self, client: usize) {
        drop(self.clients[client].steps.take());
        if let Some(handle) = self.clients[client].thread.take() {
            handle
                .join()
                .expect("the client thread")
                .expect("the client ran cleanly");
        }
        self.settle();
    }
}

// -- what a client is told when it binds ---------------------------------

#[test]
fn binding_describes_every_workspace_and_ends_in_one_done() {
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindManager);
    let mut expected = vec![
        Seen::Group(0),
        Seen::GroupCapabilities(0, 0),
        Seen::OutputEnter(0),
    ];
    // One workspace on a fresh compositor: the trailing empty one, active.
    expected.extend(created(0, 0, "1", 0, true));
    expected.push(Seen::Done(0));
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn a_wl_output_bound_after_the_manager_still_enters_the_group() {
    // Registry order is the server's choice, so a bar may well bind the
    // manager first. Without the `output_bound` hook this group would name no
    // outputs at all, and a bar filtering workspaces by output would show
    // none.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager);
    let log = fixture.take_log();
    assert!(
        !log.contains(&Seen::OutputEnter(0)),
        "the client has no wl_output yet: {log:?}"
    );

    fixture.run(Step::BindOutput);
    assert_eq!(fixture.take_log(), &[Seen::OutputEnter(0), Seen::Done(0)]);
}

#[test]
fn no_workspace_is_given_a_stable_id() {
    // Deliberate, not an omission: flexwm's workspaces are positions that the
    // next window closing can renumber, and this protocol's `id` is for
    // workspaces stable enough for a client to store preferences against.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert!(
        !log.iter().any(|seen| matches!(seen, Seen::Id(..))),
        "an id was sent: {log:?}"
    );
}

// -- keeping a client in step --------------------------------------------

#[test]
fn opening_a_window_adds_the_next_workspace_in_one_batch() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    assert_eq!(fixture.workspaces(), (2, 0));

    // The window landed on workspace 1, which was the trailing empty one, so
    // a new trailing empty one appears behind it. Nothing about workspace 1
    // changed, so it is not restated.
    let mut expected = created(1, 0, "2", 1, false);
    expected.push(Seen::Done(0));
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn switching_workspaces_restates_exactly_two_handles() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.act(Action::FocusWorkspace(Vertical::Down));
    assert_eq!(fixture.workspaces(), (2, 1));
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::State(0, INACTIVE),
            Seen::State(1, ACTIVE),
            Seen::Done(0),
        ]
    );
}

#[test]
fn a_change_that_leaves_the_workspaces_alone_says_nothing() {
    // `State::apply` runs for far more than workspace changes -- every window
    // open, every layout action, every retitle. A bar must not be woken for
    // any of them.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.act(Action::FocusColumn(Horizontal::Left));
    fixture.act(Action::MoveColumn(Horizontal::Right));
    fixture.act(Action::CycleColumnWidth);
    assert_eq!(fixture.take_log(), &[], "no done either -- nothing changed");
}

#[test]
fn a_workspace_leaves_its_group_before_it_is_removed() {
    // The protocol is explicit: a compositor "must only remove a workspace
    // not currently belonging to any workspace_group", so `workspace_leave`
    // has to come first -- and both have to be inside the same `done`.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (2, 1));

    fixture.run(Step::CloseWindow(0));
    // The window's workspace empties and is dropped; what was workspace 1 is
    // now workspace 0, and there is one workspace left.
    assert_eq!(fixture.workspaces(), (1, 0));
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::State(0, ACTIVE),
            Seen::WorkspaceLeave(0, 1),
            Seen::Removed(1),
            Seen::Done(0),
        ]
    );
}

#[test]
fn a_removed_workspace_never_gets_another_event() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CloseWindow(0));
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (1, 0));

    // Grow the list again: the new workspace must be a *new* handle, never
    // the removed one coming back to life.
    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert!(
        !log.iter().any(|seen| matches!(
            seen,
            Seen::State(1, _) | Seen::Name(1, _) | Seen::Coordinates(1, _)
        )),
        "the removed handle was spoken to again: {log:?}"
    );
    let mut expected = created(2, 0, "2", 1, false);
    expected.push(Seen::Done(0));
    assert_eq!(log, expected);
}

#[test]
fn a_client_that_destroyed_a_handle_keeps_getting_the_rest() {
    // Destroying a handle is the client's own business; it must not stop the
    // compositor sending events about the others, and it must not shift what
    // the remaining handles mean.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();
    fixture.run(Step::DestroyHandle(0));

    fixture.act(Action::FocusWorkspace(Vertical::Down));
    assert_eq!(fixture.workspaces(), (2, 1));
    // Handle 0 is gone, so only handle 1 is told anything -- and it is told
    // the right thing.
    assert_eq!(fixture.take_log(), &[Seen::State(1, ACTIVE), Seen::Done(0)]);
}

#[test]
fn a_second_manager_on_the_same_client_is_told_the_same_state() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    fixture.take_log();

    fixture.run(Step::BindManager);
    let mut expected = vec![
        Seen::Group(1),
        Seen::GroupCapabilities(1, 0),
        Seen::OutputEnter(1),
    ];
    expected.extend(created(2, 1, "1", 0, false));
    expected.extend(created(3, 1, "2", 1, true));
    expected.push(Seen::Done(1));
    assert_eq!(fixture.take_log(), expected);
    assert_eq!(fixture.registered_managers(), 2);

    // ...and from then on both are kept in step by the same diff.
    fixture.act(Action::FocusWorkspace(Vertical::Up));
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::State(1, INACTIVE),
            Seen::State(0, ACTIVE),
            Seen::Done(0),
            Seen::State(3, INACTIVE),
            Seen::State(2, ACTIVE),
            Seen::Done(1),
        ]
    );
}

// -- what a client may ask for -------------------------------------------

#[test]
fn an_activate_does_nothing_until_it_is_committed() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (2, 0));

    fixture.run(Step::Activate(1));
    assert_eq!(
        fixture.workspaces(),
        (2, 0),
        "activate is staged, not applied"
    );
    assert_eq!(fixture.take_log(), &[]);

    fixture.run(Step::Commit(0));
    assert_eq!(fixture.workspaces(), (2, 1));
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::State(0, INACTIVE),
            Seen::State(1, ACTIVE),
            Seen::Done(0),
        ]
    );
}

#[test]
fn the_last_activate_before_a_commit_is_the_one_that_happens() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    fixture.run(Step::MapWindow);
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (3, 1));

    fixture.run(Step::Activate(2));
    fixture.run(Step::Activate(0));
    fixture.run(Step::Commit(0));
    assert_eq!(fixture.workspaces(), (3, 0));
    // One batch, not two: the intermediate request never reached the layout,
    // so nothing ever said workspace 2 was active.
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::State(1, INACTIVE),
            Seen::State(0, ACTIVE),
            Seen::Done(0),
        ]
    );
}

#[test]
fn committing_an_activate_for_the_active_workspace_changes_nothing() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    for _ in 0..5 {
        fixture.run(Step::Activate(0));
        fixture.run(Step::Commit(0));
    }
    assert_eq!(fixture.workspaces(), (2, 0));
    assert_eq!(fixture.take_log(), &[], "a no-op must not re-announce");
}

#[test]
fn a_bare_commit_with_nothing_staged_is_harmless() {
    let mut fixture = Fixture::bound();
    for _ in 0..3 {
        fixture.run(Step::Commit(0));
    }
    assert_eq!(fixture.workspaces(), (1, 0));
    assert_eq!(fixture.take_log(), &[]);
}

#[test]
fn an_activate_for_a_workspace_that_vanished_before_the_commit_is_ignored() {
    // The race the protocol's own batching creates: the client acts on the
    // list it last saw, and the compositor decides against the list it has.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (2, 0));

    fixture.run(Step::Activate(1));
    // The window goes away, so workspace 1 does too -- while an `activate`
    // for it is still staged.
    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.workspaces(), (1, 0));
    fixture.run(Step::Commit(0));
    assert_eq!(fixture.workspaces(), (1, 0));

    assert_eq!(
        fixture.take_log(),
        &[Seen::WorkspaceLeave(0, 1), Seen::Removed(1), Seen::Done(0),],
        "only the removal, and nothing from the stale activate"
    );
}

#[test]
fn an_activate_on_a_removed_handle_is_ignored() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    fixture.run(Step::MapWindow);
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (3, 1));

    // Close the second window: workspace 2 goes away, and handle 2 with it.
    fixture.run(Step::CloseWindow(1));
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (2, 1));

    // The client still holds the removed handle (it has not destroyed it
    // yet) and asks it to activate. The protocol says an object is inert
    // after `removed`: nothing may happen.
    fixture.run(Step::Activate(2));
    fixture.run(Step::Commit(0));
    assert_eq!(fixture.workspaces(), (2, 1));
    assert_eq!(fixture.take_log(), &[]);

    // ...and a live handle still works afterwards, so this is "ignored", not
    // "wedged".
    fixture.run(Step::Activate(0));
    fixture.run(Step::Commit(0));
    assert_eq!(fixture.workspaces(), (2, 0));
}

#[test]
fn requests_flexwm_advertises_no_capability_for_are_ignored() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();
    assert_eq!(fixture.workspaces(), (2, 0));

    fixture.run(Step::Deactivate(0));
    fixture.run(Step::RemoveWorkspace(1));
    fixture.run(Step::AssignWorkspace {
        workspace: 1,
        group: 0,
    });
    fixture.run(Step::CreateWorkspace(0));
    fixture.run(Step::Commit(0));

    // Nothing happened, nobody was disconnected, and the client is still
    // being served -- the protocol's own answer for a request whose
    // capability is not advertised.
    assert_eq!(fixture.workspaces(), (2, 0));
    assert_eq!(fixture.take_log(), &[]);
    fixture.run(Step::Activate(1));
    fixture.run(Step::Commit(0));
    assert_eq!(fixture.workspaces(), (2, 1));
}

// -- lifecycle -------------------------------------------------------------

#[test]
fn stop_is_answered_with_finished_and_ends_the_updates() {
    let mut fixture = Fixture::bound();
    assert_eq!(fixture.registered_managers(), 1);

    fixture.run(Step::Stop(0));
    assert_eq!(fixture.take_log(), &[Seen::Finished(0)]);
    assert_eq!(fixture.registered_managers(), 0);

    // No further events, and the handles it was given are inert: an
    // `activate` on one is ignored rather than honoured or fatal.
    fixture.run(Step::MapWindow);
    assert_eq!(fixture.workspaces(), (2, 0));
    assert_eq!(fixture.take_log(), &[]);
    fixture.run(Step::Activate(0));
    assert_eq!(fixture.workspaces(), (2, 0));
}

#[test]
fn two_clients_are_kept_in_step_independently() {
    let mut fixture = Fixture::bound();
    let second = fixture.spawn_client();
    fixture.run_on(second, Step::BindManager);
    // The second client binds its `wl_output` *after* its manager, so the
    // compositor's `output_bound` hook has to answer it -- and answer it
    // only to that client. Sending an event carrying one client's object to
    // another client is a panic inside wayland-backend, i.e. the whole
    // compositor, which is why this is a cross-client test and not a second
    // manager on the same connection.
    fixture.run_on(second, Step::BindOutput);
    fixture.take_log();
    fixture.take_log_on(second);
    assert_eq!(fixture.registered_managers(), 2);

    fixture.run(Step::MapWindow);
    let first_log = fixture.take_log();
    let second_log = fixture.take_log_on(second);
    // Same change, same events, each against its own handle numbering -- the
    // second client's first workspace handle is its key 0, not 1.
    let mut expected_first = created(1, 0, "2", 1, false);
    expected_first.push(Seen::Done(0));
    assert_eq!(first_log, expected_first);
    let mut expected_second = created(1, 0, "2", 1, false);
    expected_second.push(Seen::Done(0));
    assert_eq!(second_log, expected_second);

    // One client stopping leaves the other exactly as it was.
    fixture.run_on(second, Step::Stop(0));
    assert_eq!(fixture.registered_managers(), 1);
    fixture.take_log_on(second);
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::State(0, INACTIVE),
            Seen::State(1, ACTIVE),
            Seen::Done(0),
        ]
    );
    assert_eq!(fixture.take_log_on(second), &[]);
}

#[test]
fn one_client_disconnecting_does_not_disturb_another() {
    let mut fixture = Fixture::bound();
    let second = fixture.spawn_client();
    fixture.run_on(second, Step::BindOutput);
    fixture.run_on(second, Step::BindManager);
    fixture.run_on(second, Step::MapWindow);
    fixture.take_log();
    fixture.take_log_on(second);
    assert_eq!(fixture.registered_managers(), 2);
    assert_eq!(fixture.workspaces(), (2, 0));

    // The second client owned the window, so its disconnect also shrinks the
    // workspace list -- which the survivor must be told about, in full,
    // while the dead client's manager is being dropped in the same pass.
    fixture.disconnect_client(second);
    assert_eq!(fixture.registered_managers(), 1);
    assert_eq!(fixture.workspaces(), (1, 0));
    assert_eq!(
        fixture.take_log(),
        &[Seen::WorkspaceLeave(0, 1), Seen::Removed(1), Seen::Done(0),]
    );
}

#[test]
fn a_client_going_away_stops_being_tracked() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    assert_eq!(fixture.registered_managers(), 1);

    fixture.disconnect_client(0);
    assert_eq!(
        fixture.registered_managers(),
        0,
        "a dead client's manager must not be walked on every refresh"
    );
    // The client's window went with it, which is also the proof that the
    // disconnect was fully processed: the workspace it was on is gone, and
    // publishing that to a manager that no longer exists did not panic.
    assert_eq!(fixture.workspaces(), (1, 0));

    // ...and the compositor keeps working with nobody listening.
    fixture.state.act(Action::FocusWorkspace(Vertical::Down));
    fixture.settle();
    assert_eq!(
        fixture.workspaces(),
        (1, 0),
        "one workspace has nowhere to go"
    );
}
