//! Tests for `ext-foreign-toplevel-list-v1`.
//!
//! These drive a *real* `wayland-client` connection -- binding
//! `ext_foreign_toplevel_list_v1` and reacting to its events the way a
//! taskbar's window list does -- through a real [`State`], and assert on
//! **the exact sequence of events the client received**, in order.
//!
//! That is the point of every one of them: what can go wrong with an
//! enumeration protocol is a client being told the wrong thing, or told it in
//! the wrong order, or not told at all. A `done` with nothing before it, a
//! `title` on a handle already `closed`, a window announced twice, a handle
//! left behind after its window went -- each is a perfectly sensible-looking
//! call sequence on the compositor side and a taskbar showing windows that do
//! not exist.
//!
//! Windows here are bare `xdg_toplevel`s with no buffer, for the same reason
//! `ext_workspace/tests.rs` uses them: what puts a window in scoot's lists is
//! the toplevel *existing* (see this protocol's own module doc on what "a
//! toplevel" means here), so nothing needs `wl_shm`.
//!
//! Like the other client-driven suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here connects to (clients are inserted as socket pairs) but
//! which is created either way.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

/// The framebuffer these tests render into. Nothing here reads a pixel; the
/// backend exists so the compositor has a real output, as it does in a
/// session.
const CANVAS: i32 = 200;

/// One protocol event, as the client saw it.
///
/// Toplevel handles are keyed by the order the client was told about them and
/// lists by the order it bound them, so an expectation can be written out
/// literally.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    /// `ext_foreign_toplevel_list_v1.toplevel`
    Toplevel(u32),
    Identifier(u32, String),
    Title(u32, String),
    AppId(u32, String),
    Done(u32),
    Closed(u32),
    /// `ext_foreign_toplevel_list_v1.finished`, keyed by list.
    Finished(u32),
}

/// The whole burst for one toplevel being announced, in the order this
/// compositor sends it.
fn announced(key: u32, identifier: &str, title: &str, app_id: &str) -> Vec<Seen> {
    vec![
        Seen::Toplevel(key),
        Seen::Identifier(key, identifier.to_string()),
        Seen::Title(key, title.to_string()),
        Seen::AppId(key, app_id.to_string()),
        Seen::Done(key),
    ]
}

#[derive(Default)]
struct TestClient {
    /// Bound on demand so a test can control whether the list is bound before
    /// or after a window exists.
    list_name: Option<(u32, u32)>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    locks: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    lists: Vec<ExtForeignToplevelListV1>,
    handles: Vec<ExtForeignToplevelHandleV1>,
    /// Every event since the last [`Step::TakeLog`], in arrival order.
    log: Vec<Seen>,
    /// The serial of each toplevel's latest unacked `xdg_surface.configure`.
    window_serials: Vec<Option<u32>>,
}

impl TestClient {
    fn list_key(&self, list: &ExtForeignToplevelListV1) -> u32 {
        key_of(&self.lists, list)
    }

    fn handle_key(&self, handle: &ExtForeignToplevelHandleV1) -> u32 {
        key_of(&self.handles, handle)
    }
}

/// The position of `proxy` in `known`, or `u32::MAX` for something the client
/// was never told about -- which shows up as a mismatch in an assertion rather
/// than as a panic in a dispatch handler.
fn key_of<P: Proxy + PartialEq>(known: &[P], proxy: &P) -> u32 {
    known
        .iter()
        .position(|other| other == proxy)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(u32::MAX)
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
            "ext_foreign_toplevel_list_v1" => client.list_name = Some((name, version)),
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "ext_session_lock_manager_v1" => {
                client.locks = Some(registry.bind(name, version.min(1), qh, ()))
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        list: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                client.handles.push(toplevel);
                let key = u32::try_from(client.handles.len() - 1).expect("few toplevels");
                client.log.push(Seen::Toplevel(key));
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {
                let key = client.list_key(list);
                client.log.push(Seen::Finished(key));
            }
            _ => {}
        }
    }

    event_created_child!(TestClient, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = client.handle_key(handle);
        let seen = match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                Seen::Identifier(key, identifier)
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => Seen::Title(key, title),
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => Seen::AppId(key, app_id),
            ext_foreign_toplevel_handle_v1::Event::Done => Seen::Done(key),
            ext_foreign_toplevel_handle_v1::Event::Closed => Seen::Closed(key),
            _ => return,
        };
        client.log.push(seen);
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
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(TestClient: ignore ext_session_lock_v1::ExtSessionLockV1);

/// One instruction for the client thread.
enum Step {
    /// Bind another `ext_foreign_toplevel_list_v1`.
    BindList,
    /// Create an `xdg_toplevel` (and ack its configure), with no title, no app
    /// id and no buffer.
    MapWindow,
    /// The same, with the title and app id set before the first commit -- what
    /// a real toolkit does.
    MapDescribedWindow {
        app_id: String,
        title: String,
    },
    /// Destroy the `index`-th window.
    CloseWindow(usize),
    SetTitle(usize, String),
    SetAppId(usize, String),
    /// `destroy` on the `index`-th toplevel handle, while its window is still
    /// open.
    DestroyHandle(usize),
    /// `stop` on the `index`-th list.
    Stop(usize),
    /// `destroy` on the `index`-th list, keeping every handle it made.
    DestroyList(usize),
    /// Open and immediately destroy `count` toplevels, in one burst, without
    /// waiting for anything in between.
    ChurnWindows(usize),
    /// Take the session lock, without ever creating a lock surface.
    LockSession,
    /// Hand back (and clear) everything seen so far.
    TakeLog,
}

enum Ack {
    Done,
    Log(Vec<Seen>),
}

/// Runs the client half: binds what it needs, then executes whatever steps the
/// test sends, acknowledging each one once the compositor has seen it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let list_global = client
        .list_name
        .ok_or("no ext_foreign_toplevel_list_v1 -- the global is missing")?;
    let mut windows: Vec<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    )> = Vec::new();
    // Held so the lock is not released the moment it is taken.
    let mut lock: Option<ext_session_lock_v1::ExtSessionLockV1> = None;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match step {
            Step::BindList => {
                let list: ExtForeignToplevelListV1 =
                    registry.bind(list_global.0, list_global.1.min(1), &qh, ());
                client.lists.push(list);
            }
            Step::MapWindow => windows.push(map_window(
                &compositor,
                &wm_base,
                &qh,
                &mut queue,
                &mut client,
                None,
            )?),
            Step::MapDescribedWindow { app_id, title } => windows.push(map_window(
                &compositor,
                &wm_base,
                &qh,
                &mut queue,
                &mut client,
                Some((app_id, title)),
            )?),
            Step::CloseWindow(index) => {
                // In this order, or the compositor answers with a protocol
                // error: an `xdg_surface` may only be destroyed after its role
                // object.
                let (surface, xdg, toplevel) = windows.get(index).ok_or("no such window")?.clone();
                toplevel.destroy();
                xdg.destroy();
                surface.destroy();
            }
            Step::SetTitle(index, title) => windows
                .get(index)
                .ok_or("no such window")?
                .2
                .set_title(title),
            Step::SetAppId(index, app_id) => windows
                .get(index)
                .ok_or("no such window")?
                .2
                .set_app_id(app_id),
            Step::DestroyHandle(index) => client
                .handles
                .get(index)
                .ok_or("no such toplevel handle")?
                .destroy(),
            Step::Stop(index) => client.lists.get(index).ok_or("no such list")?.stop(),
            Step::DestroyList(index) => client.lists.get(index).ok_or("no such list")?.destroy(),
            Step::ChurnWindows(count) => {
                // No configure ack and no wait: the point is the compositor
                // seeing creation and destruction at the rate a client can
                // write them, not a well-behaved window.
                for _ in 0..count {
                    let surface = compositor.create_surface(&qh, ());
                    let index = client.window_serials.len();
                    client.window_serials.push(None);
                    let xdg = wm_base.get_xdg_surface(&surface, &qh, WindowIndex(index));
                    let toplevel = xdg.get_toplevel(&qh, ());
                    surface.commit();
                    toplevel.destroy();
                    xdg.destroy();
                    surface.destroy();
                }
            }
            Step::LockSession => {
                let manager = client
                    .locks
                    .clone()
                    .ok_or("no ext_session_lock_manager_v1")?;
                lock = Some(manager.lock(&qh, ()));
            }
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
    // Keeps the lock alive for the whole script rather than dropping it at the
    // first step boundary.
    drop(lock);
    Ok(())
}

/// Creates one `xdg_toplevel`, optionally describing it first, and acks the
/// configure that comes back.
fn map_window(
    compositor: &wl_compositor::WlCompositor,
    wm_base: &xdg_wm_base::XdgWmBase,
    qh: &QueueHandle<TestClient>,
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    described: Option<(String, String)>,
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
    let toplevel = xdg.get_toplevel(qh, ());
    if let Some((app_id, title)) = described {
        toplevel.set_app_id(app_id);
        toplevel.set_title(title);
    }
    surface.commit();
    let serial = wait_for(queue, client, "a toplevel configure", |client| {
        client.window_serials[index]
    })?;
    xdg.ack_configure(serial);
    Ok((surface, xdg, toplevel))
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time. See [`crate::compositor::test_support`] for
/// everything that is not specific to this protocol.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn new() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// A fixture whose client has bound the list, with the (empty) initial
    /// burst already drained.
    fn bound() -> Self {
        let mut fixture = Self::new();
        fixture.run(Step::BindList);
        fixture.take_log();
        fixture
    }

    /// Everything client 0 has seen since the last call.
    fn take_log(&mut self) -> Vec<Seen> {
        self.take_log_on(0)
    }

    fn take_log_on(&mut self, client: usize) -> Vec<Seen> {
        match self.run_on(client, Step::TakeLog) {
            Ack::Log(log) => log,
            Ack::Done => panic!("the client answered a log request with nothing"),
        }
    }

    /// The identifier the compositor would have sent for the `n`-th window it
    /// created, derived from its own state rather than from what the client
    /// reported -- so an expectation is checked against both ends.
    fn identifier_of(&self, window: u64) -> String {
        identifier(&self.state.foreign_toplevels.generation, WindowId(window))
    }

    /// How many windows the compositor is keeping a handle for.
    fn tracked(&self) -> usize {
        self.state.foreign_toplevels.toplevels.len()
    }
}

// -- what a client is told -----------------------------------------------

#[test]
fn binding_with_no_windows_announces_nothing() {
    // The zero-window case: a bar started before anything else in the session
    // binds this global with an empty desktop behind it, and must simply be
    // told nothing -- not an empty `done`, which belongs to a *handle*.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindList);
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.tracked(), 0);
}

#[test]
fn a_window_opened_after_binding_is_announced() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.take_log(),
        announced(0, &fixture.identifier_of(1), "", ""),
    );
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn a_window_that_already_exists_is_announced_at_bind() {
    // Registry order is the server's choice and a bar may well start after the
    // session is full of windows, so binding has to describe the world as it
    // already is.
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::BindList);
    let mut expected = announced(0, &fixture.identifier_of(1), "", "");
    expected.extend(announced(1, &fixture.identifier_of(2), "", ""));
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn a_window_created_with_a_title_reports_it_in_its_own_batch() {
    // What every real toolkit does: `get_toplevel`, then `set_app_id` and
    // `set_title`, then commit. The handle exists from `get_toplevel`, so the
    // description arrives as later batches rather than in the first one --
    // which is exactly what `done` is for, and why a client draws on `done`
    // rather than on each event.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapDescribedWindow {
        app_id: "org.scoot.Probe".to_string(),
        title: "a window".to_string(),
    });
    let mut expected = announced(0, &fixture.identifier_of(1), "", "");
    expected.extend([
        Seen::AppId(0, "org.scoot.Probe".to_string()),
        Seen::Done(0),
        Seen::Title(0, "a window".to_string()),
        Seen::Done(0),
    ]);
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn a_title_change_is_one_title_event_and_one_done() {
    // The app id is *not* re-sent: Smithay drops a `send_app_id` that would
    // repeat the current value, which is what lets this module publish both
    // fields on every change without comparing anything itself.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::SetTitle(0, "renamed".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "renamed".to_string()), Seen::Done(0)],
    );
}

#[test]
fn an_app_id_change_is_one_app_id_event_and_one_done() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::SetAppId(0, "org.scoot.Probe".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::AppId(0, "org.scoot.Probe".to_string()), Seen::Done(0),],
    );
}

#[test]
fn setting_the_same_title_again_tells_the_list_nothing() {
    // A terminal that re-sets the title it already has (every prompt, for some
    // shells) must not turn into wire traffic for every bar in the session.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::SetTitle(0, "steady".to_string()));
    fixture.take_log();

    fixture.run(Step::SetTitle(0, "steady".to_string()));
    fixture.run(Step::SetTitle(0, "steady".to_string()));
    assert_eq!(fixture.take_log(), Vec::new());
}

#[test]
fn closing_a_window_closes_its_handle() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), vec![Seen::Closed(0)]);
    assert_eq!(fixture.tracked(), 0, "a handle outlived its window");
}

#[test]
fn a_window_closing_leaves_the_others_alone() {
    // One window's `closed` must not be another's: the handles are keyed by
    // `WindowId`, and a taskbar that dropped the wrong row would be showing a
    // window that is gone and hiding one that is not.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::SetTitle(1, "survivor".to_string()));
    fixture.take_log();

    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), vec![Seen::Closed(0)]);

    fixture.run(Step::SetTitle(1, "still here".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(1, "still here".to_string()), Seen::Done(1)],
    );
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn a_late_bind_after_closing_the_first_of_several_sees_the_right_survivors() {
    // Regression guard for the hazard this module's own doc describes:
    // upstream's `remove_toplevel` computes a removal index over the
    // *upgradable* handles and removes that index from the full list, so a
    // dead entry earlier in the list would make it remove the wrong, still-
    // live one. Closing the *first* of several windows is exactly the shape
    // that would expose that -- if the removal ever went wrong, a client
    // binding afterwards would see the wrong pair of survivors (the closed
    // window still present, or a live one missing) rather than exactly the
    // two that are actually still open.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1
    fixture.run(Step::MapWindow); // window 2
    fixture.run(Step::MapWindow); // window 3
    fixture.take_log();

    fixture.run(Step::CloseWindow(0)); // closes window 1, the earliest entry
    fixture.take_log();

    fixture.run(Step::BindList);
    let log = fixture.take_log();
    let mut seen: Vec<String> = log
        .iter()
        .filter_map(|seen| match seen {
            Seen::Identifier(_, identifier) => Some(identifier.clone()),
            _ => None,
        })
        .collect();
    seen.sort();
    let mut expected = vec![fixture.identifier_of(2), fixture.identifier_of(3)];
    expected.sort();
    assert_eq!(
        seen, expected,
        "a fresh bind after closing the first window saw the wrong survivors"
    );
}

#[test]
fn a_reopened_window_gets_a_new_identifier() {
    // The protocol forbids reusing an identifier once a toplevel is gone.
    // scoot's window ids only ever increase, which is what makes that hold.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CloseWindow(0));
    fixture.take_log();

    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert_eq!(log, announced(1, &fixture.identifier_of(2), "", ""));
    assert_ne!(fixture.identifier_of(1), fixture.identifier_of(2));
}

// -- the bridge to scoot's own window list ------------------------------

#[test]
fn the_identifier_names_the_ipc_window_id() {
    // The one thing that makes enumeration actionable: this protocol has no
    // requests at all, so a client that wants to *focus* a window it found
    // here has to get from the identifier to `scoot msg action
    // focus-window-id N`. Asserted against the same snapshot builder the
    // `windows` request answers with, in both directions.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapDescribedWindow {
        app_id: "org.scoot.Probe".to_string(),
        title: "target".to_string(),
    });
    let log = fixture.take_log();
    let Some(Seen::Identifier(_, identifier)) = log
        .iter()
        .find(|seen| matches!(seen, Seen::Identifier(..)))
        .cloned()
    else {
        panic!("no identifier in {log:?}");
    };

    let snapshot = fixture
        .state
        .window_snapshots()
        .into_iter()
        .find(|snapshot| snapshot.title == "target")
        .expect("the window is in the IPC list");
    let (generation, id) = identifier
        .rsplit_once('-')
        .expect("an identifier is <generation>-<window id>");
    assert_eq!(id.parse::<u64>().expect("a decimal window id"), snapshot.id);
    assert_eq!(generation.len(), GENERATION_HEX);
}

#[test]
fn an_identifier_fits_the_protocol_bound() {
    // Smithay *asserts* on an identifier that is empty, non-ASCII or longer
    // than 32 bytes, and an assert in a compositor is every client's session.
    // The largest one this can ever build is the largest `u64`.
    let identifier = identifier("3f9c1e07", WindowId(u64::MAX));
    assert!(!identifier.is_empty());
    assert!(identifier.is_ascii());
    assert!(
        identifier.len() <= 32,
        "identifier {identifier} is {} bytes",
        identifier.len()
    );
    assert_eq!(identifier, "3f9c1e07-18446744073709551615");
}

#[test]
fn the_generation_prefix_is_eight_hex_digits() {
    // Fixed width is what keeps the bound above provable, and what lets a
    // client split the identifier at its last `-`.
    let generation = generation();
    assert_eq!(generation.len(), GENERATION_HEX);
    assert!(
        generation.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "generation {generation} is not hex"
    );
}

// -- more than one list, and more than one client ------------------------

#[test]
fn two_lists_in_one_client_each_get_their_own_handle() {
    // A client may bind the global more than once (a shell with two widgets
    // watching windows). Each binding is its own object tree, and the same
    // window carries the same identifier on both -- which is what the
    // identifier is *for*, per the protocol: telling a client that two handles
    // are the same toplevel.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::BindList);
    let log = fixture.take_log();
    let mut expected = announced(0, &fixture.identifier_of(1), "", "");
    expected.extend(announced(1, &fixture.identifier_of(1), "", ""));
    assert_eq!(log, expected);

    // And both are kept in step afterwards. The two handles' events interleave
    // -- upstream sends one event to every instance before moving to the next
    // -- which is fine and is asserted as it happens rather than tidied away:
    // what the protocol requires is that each *handle* sees its `title` before
    // its `done`, not that two handles are served in turn.
    fixture.run(Step::SetTitle(0, "both".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::Title(0, "both".to_string()),
            Seen::Title(1, "both".to_string()),
            Seen::Done(0),
            Seen::Done(1),
        ],
    );
}

#[test]
fn two_clients_see_the_same_window_with_the_same_identifier() {
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    fixture.run_on(0, Step::BindList);
    fixture.run_on(1, Step::BindList);
    fixture.take_log();
    fixture.take_log_on(1);

    fixture.run_on(0, Step::MapWindow);
    let identifier = fixture.identifier_of(1);
    assert_eq!(fixture.take_log(), announced(0, &identifier, "", ""));
    assert_eq!(fixture.take_log_on(1), announced(0, &identifier, "", ""));

    // The window belongs to client 0; client 1 is told when it goes.
    fixture.run_on(0, Step::CloseWindow(0));
    assert_eq!(fixture.take_log_on(1), vec![Seen::Closed(0)]);
}

#[test]
fn a_client_that_disconnects_stops_being_watched() {
    // The ordinary end of a bar's life. Nothing may be left pointing at it,
    // and every other client's view must survive it.
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    fixture.run_on(0, Step::BindList);
    fixture.run_on(1, Step::BindList);
    fixture.run_on(1, Step::MapWindow);
    fixture.take_log();
    fixture.take_log_on(1);

    fixture.disconnect(0);

    fixture.run_on(1, Step::MapWindow);
    fixture.run_on(1, Step::CloseWindow(0));
    assert_eq!(
        fixture.take_log_on(1),
        vec![
            Seen::Toplevel(1),
            Seen::Identifier(1, fixture.identifier_of(2)),
            Seen::Title(1, String::new()),
            Seen::AppId(1, String::new()),
            Seen::Done(1),
            Seen::Closed(0),
        ],
    );
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn destroying_a_handle_early_costs_no_one_else_anything() {
    // Legal, and the protocol says so: a client may destroy a handle while its
    // window is still open (it just will not get another one for that
    // window). What must not happen is the compositor trying to send `closed`
    // on the dead object, or the *other* client losing its own event.
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    fixture.run_on(0, Step::BindList);
    fixture.run_on(1, Step::BindList);
    fixture.run_on(0, Step::MapWindow);
    fixture.take_log();
    fixture.take_log_on(1);

    fixture.run_on(0, Step::DestroyHandle(0));
    fixture.run_on(0, Step::SetTitle(0, "unheard".to_string()));
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "a destroyed handle was still being written to"
    );
    assert_eq!(
        fixture.take_log_on(1),
        vec![Seen::Title(0, "unheard".to_string()), Seen::Done(0)],
    );

    fixture.run_on(0, Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.take_log_on(1), vec![Seen::Closed(0)]);
    assert_eq!(fixture.tracked(), 0);
}

// -- stop ----------------------------------------------------------------

#[test]
fn stop_finishes_the_list_and_no_later_window_is_announced() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::Stop(0));
    assert_eq!(fixture.take_log(), vec![Seen::Finished(0)]);

    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "a stopped list was still being told about new windows"
    );
    // The window itself is still tracked -- `stop` is one client's choice, not
    // a change to the compositor.
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn stopping_or_destroying_a_list_twice_over_is_survivable() {
    // Neither is well-behaved -- the protocol's sequence is stop, wait for
    // `finished`, destroy -- and neither may take the compositor with it. A
    // second `stop` is answered a second time (`finished` is not a destructor
    // event, so the object is still there to answer), and `destroy` while
    // handles are still held leaves those handles working, which is the part
    // that would otherwise reach a dead list object on the next window.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::Stop(0));
    fixture.run(Step::Stop(0));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Finished(0), Seen::Finished(0)],
    );

    fixture.run(Step::DestroyList(0));
    fixture.run(Step::MapWindow);
    fixture.run(Step::SetTitle(0, "orphaned handle".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "orphaned handle".to_string()), Seen::Done(0)],
    );
    assert_eq!(fixture.tracked(), 2);

    // Still serving: a fresh list binds and sees both windows.
    fixture.run(Step::BindList);
    let log = fixture.take_log();
    assert_eq!(
        log.iter()
            .filter(|seen| matches!(seen, Seen::Toplevel(_)))
            .count(),
        2,
        "a fresh list did not see both windows: {log:?}"
    );
}

#[test]
fn windows_opened_and_destroyed_at_full_rate_leave_nothing_behind() {
    // The maximum rate a client can actually reach: 200 toplevels created and
    // destroyed in one burst, with no configure ack and nothing waited for in
    // between -- so the compositor sees creation and destruction of the same
    // window in a single dispatch. What is under test is that each one is
    // announced and closed exactly once and that nothing is left tracked
    // afterwards, which is the leak this module's handle map could have.
    const CHURN: usize = 200;

    let mut fixture = Fixture::bound();
    fixture.run(Step::ChurnWindows(CHURN));
    let log = fixture.take_log();
    let toplevels = log
        .iter()
        .filter(|seen| matches!(seen, Seen::Toplevel(_)))
        .count();
    let closed = log
        .iter()
        .filter(|seen| matches!(seen, Seen::Closed(_)))
        .count();
    assert_eq!(toplevels, CHURN, "not every window was announced");
    assert_eq!(closed, CHURN, "not every window was closed");
    assert_eq!(fixture.tracked(), 0, "handles outlived their windows");
    assert!(
        fixture.state.windows.is_empty(),
        "the compositor kept windows the client destroyed"
    );

    // And the identifiers are all distinct, which is what the protocol
    // requires of a compositor that has just burned 200 of them.
    let mut identifiers: Vec<&String> = log
        .iter()
        .filter_map(|seen| match seen {
            Seen::Identifier(_, identifier) => Some(identifier),
            _ => None,
        })
        .collect();
    assert_eq!(identifiers.len(), CHURN);
    identifiers.sort();
    identifiers.dedup();
    assert_eq!(identifiers.len(), CHURN, "an identifier was reused");
}

#[test]
fn a_title_as_long_as_the_wire_allows_survives_the_round_trip() {
    // A client's title is its own to choose, and it reaches every watching
    // client verbatim. Nothing here truncates or validates it -- the wayland
    // message size is the only bound -- so this pins that a large-but-legal
    // one is forwarded whole rather than truncated, dropped, or turned into a
    // protocol error for the innocent client watching.
    let title = "t".repeat(3000);
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::SetTitle(0, title.clone()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, title.clone()), Seen::Done(0)],
    );
    // And the same string reaches the IPC window list, which is the other
    // consumer of the very same `WindowInfo`.
    let snapshot = fixture
        .state
        .window_snapshots()
        .into_iter()
        .next()
        .expect("the window is in the IPC list");
    assert_eq!(snapshot.title, title);
}

#[test]
fn a_handle_from_before_stop_still_reports_changes() {
    // `stop` says "no more *toplevel* events", not "forget the handles I
    // already have": the protocol's own teardown sequence is stop, wait for
    // `finished`, then destroy the handles -- which a client cannot do safely
    // if the compositor has already stopped telling it what they are doing.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Stop(0));
    fixture.take_log();

    fixture.run(Step::SetTitle(0, "after stop".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "after stop".to_string()), Seen::Done(0)],
    );

    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), vec![Seen::Closed(0)]);
}

// -- the session lock ----------------------------------------------------

#[test]
fn the_window_list_stays_live_while_the_session_is_locked() {
    // Deliberate, and the same answer `scoot msg windows` gives (see this
    // module's doc and `README.md`'s lock section): a process that can reach
    // this socket is inside the trust boundary already, and sending `closed`
    // for windows that did not close would be a lie a taskbar could not
    // recover from.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::LockSession);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the test never locked the session"
    );
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "locking the session churned the window list"
    );

    fixture.run(Step::SetTitle(0, "behind the lock".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "behind the lock".to_string()), Seen::Done(0),],
    );

    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.take_log(),
        announced(1, &fixture.identifier_of(2), "", ""),
    );
}
