//! Tests for the shared per-client bind budget.
//!
//! These drive *real* `wayland-client` connections -- binding each of the
//! four capped globals the way a shell does -- through a real [`State`] with
//! a real `headless` backend, and assert on **which binds were answered with
//! `finished`**, plus the compositor-side count underneath.
//!
//! That pairing is the point: the wire says what the client was told, and
//! the count says the bookkeeping is exact -- a test asserting only the wire
//! would pass against a version that leaked every released bind internally,
//! and one asserting only the count would pass against a version that told
//! the client nothing. No windows are opened anywhere here: the budgeted
//! thing is binds, and the per-protocol suites already pin what an allowed
//! bind announces with a session behind it.
//!
//! Like the other client-driven suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here connects to (clients are inserted as socket pairs) but
//! which is created either way.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1;
use wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::ExtWorkspaceHandleV1;
use wayland_protocols::ext::workspace::v1::client::ext_workspace_manager_v1::{
    self, ExtWorkspaceManagerV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self as client_manager, ZwlrForeignToplevelManagerV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_head_v1::{
    self, ZwlrOutputHeadV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_manager_v1::{
    self, ZwlrOutputManagerV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_mode_v1::ZwlrOutputModeV1;

use super::MAX_BINDS_PER_CLIENT;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

type Fixture = Harness<Step, Ack>;

/// The framebuffer these tests render into. Nothing here reads a pixel; the
/// backend exists so the compositor has a real output, as it does in a
/// session.
const CANVAS: i32 = 200;

/// One refusal-relevant event, as the client saw it. Managers and lists are
/// keyed by the order the client bound them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    WsDone(u32),
    WsFinished(u32),
    ListFinished(u32),
    WlrFinished(u32),
    OutputDone(u32),
    OutputFinished(u32),
}

/// One instruction for the client thread: bind another of a kind, `stop` the
/// `n`-th bound one of a kind, bare-`destroy` the `n`-th workspace manager
/// with no `stop` before it, or do a stop/destroy and a bind back to back
/// with no round trip in between (the dead-but-unpruned shapes).
enum Step {
    BindWs,
    BindList,
    BindWlr,
    BindOutput,
    StopWs(usize),
    StopList(usize),
    StopWlr(usize),
    StopOutput(usize),
    StopWsAndBindWs(usize),
    DestroyListAndBindList(usize),
    TakeLog,
}

enum Ack {
    Done,
    Log(Vec<Seen>),
}

#[derive(Default)]
struct TestClient {
    /// Registry names, bound on demand so a test controls exactly how many
    /// binds of each kind its client holds.
    ws_name: Option<(u32, u32)>,
    list_name: Option<(u32, u32)>,
    wlr_name: Option<(u32, u32)>,
    output_name: Option<(u32, u32)>,
    ws: Vec<ExtWorkspaceManagerV1>,
    lists: Vec<ExtForeignToplevelListV1>,
    wlr: Vec<ZwlrForeignToplevelManagerV1>,
    outputs: Vec<ZwlrOutputManagerV1>,
    /// Every refusal-relevant event since the last [`Step::TakeLog`].
    log: Vec<Seen>,
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
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
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
            "ext_workspace_manager_v1" => client.ws_name = Some((name, version)),
            "ext_foreign_toplevel_list_v1" => client.list_name = Some((name, version)),
            "zwlr_foreign_toplevel_manager_v1" => client.wlr_name = Some((name, version)),
            "zwlr_output_manager_v1" => client.output_name = Some((name, version)),
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
        let key = key_of(&client.ws, manager);
        match event {
            ext_workspace_manager_v1::Event::Done => client.log.push(Seen::WsDone(key)),
            ext_workspace_manager_v1::Event::Finished => client.log.push(Seen::WsFinished(key)),
            _ => {}
        }
    }

    event_created_child!(TestClient, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
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
        if let ext_foreign_toplevel_list_v1::Event::Finished = event {
            client
                .log
                .push(Seen::ListFinished(key_of(&client.lists, list)));
        }
    }

    event_created_child!(TestClient, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        manager: &ZwlrForeignToplevelManagerV1,
        event: client_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let client_manager::Event::Finished = event {
            client
                .log
                .push(Seen::WlrFinished(key_of(&client.wlr, manager)));
        }
    }

    event_created_child!(TestClient, ZwlrForeignToplevelManagerV1, [
        client_manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputManagerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        manager: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = key_of(&client.outputs, manager);
        match event {
            zwlr_output_manager_v1::Event::Done { .. } => client.log.push(Seen::OutputDone(key)),
            zwlr_output_manager_v1::Event::Finished => client.log.push(Seen::OutputFinished(key)),
            _ => {}
        }
    }

    event_created_child!(TestClient, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputHeadV1,
        _: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(TestClient, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

wayland_client::delegate_noop!(TestClient: ignore ExtWorkspaceGroupHandleV1);
wayland_client::delegate_noop!(TestClient: ignore ExtWorkspaceHandleV1);
wayland_client::delegate_noop!(TestClient: ignore ExtForeignToplevelHandleV1);
wayland_client::delegate_noop!(TestClient: ignore ZwlrForeignToplevelHandleV1);
wayland_client::delegate_noop!(TestClient: ignore ZwlrOutputModeV1);

/// Runs the client half: learns the globals, then executes whatever steps the
/// test sends, acknowledging each one once the compositor has answered it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let (ws_name, ws_version) = client.ws_name.ok_or("no ext_workspace_manager_v1")?;
    let (list_name, list_version) = client.list_name.ok_or("no ext_foreign_toplevel_list_v1")?;
    let (wlr_name, wlr_version) = client
        .wlr_name
        .ok_or("no zwlr_foreign_toplevel_manager_v1")?;
    let (output_name, output_version) = client.output_name.ok_or("no zwlr_output_manager_v1")?;

    for step in steps {
        match step {
            Step::BindWs => {
                client
                    .ws
                    .push(registry.bind(ws_name, ws_version.min(1), &qh, ()));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::BindList => {
                client
                    .lists
                    .push(registry.bind(list_name, list_version.min(1), &qh, ()));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::BindWlr => {
                client
                    .wlr
                    .push(registry.bind(wlr_name, wlr_version.min(3), &qh, ()));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::BindOutput => {
                client
                    .outputs
                    .push(registry.bind(output_name, output_version.min(4), &qh, ()));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::StopWs(index) => {
                client.ws[index].stop();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::StopList(index) => {
                client.lists[index].stop();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::StopWlr(index) => {
                client.wlr[index].stop();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::StopOutput(index) => {
                client.outputs[index].stop();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::StopWsAndBindWs(index) => {
                // No round trip between the two: both land in one dispatch
                // batch, so the rebind must see the synchronously freed slot.
                client.ws[index].stop();
                client
                    .ws
                    .push(registry.bind(ws_name, ws_version.min(1), &qh, ()));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::DestroyListAndBindList(index) => {
                // Same, but the free is only visible at post-batch cleanup:
                // the rebind briefly counts both generations.
                client.lists[index].destroy();
                client
                    .lists
                    .push(registry.bind(list_name, list_version.min(1), &qh, ()));
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Done).map_err(|e| e.to_string())?;
            }
            Step::TakeLog => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let log = std::mem::take(&mut client.log);
                acks.send(Ack::Log(log)).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

impl Fixture {
    fn new() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// Everything client `index` has seen since the last call.
    fn take_log_on(&mut self, index: usize) -> Vec<Seen> {
        match self.run_on(index, Step::TakeLog) {
            Ack::Log(log) => log,
            Ack::Done => panic!("the client answered a log request with nothing"),
        }
    }

    fn take_log(&mut self) -> Vec<Seen> {
        self.take_log_on(0)
    }

    /// What the compositor thinks client `index` holds.
    fn held(&self, index: usize) -> usize {
        self.state.bind_budget.count(&self.client(index).id())
    }
}

// -- one refusal test per global -------------------------------------------

/// Fills the whole shared budget with workspace managers, then binds one
/// more: the ninth is answered `done` + `finished` and nothing else, and the
/// client survives it.
#[test]
fn ninth_workspace_manager_is_finished_not_killed() {
    let mut fixture = Fixture::new();
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run(Step::BindWs);
    }
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
    fixture.run(Step::BindWs);
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::WsFinished(MAX_BINDS_PER_CLIENT)),
        "the ninth bind was not finished: {log:?}"
    );
    assert!(
        log.iter()
            .all(|seen| !matches!(seen, Seen::WsFinished(key) if *key < MAX_BINDS_PER_CLIENT)),
        "an allowed bind was finished: {log:?}"
    );
    // Still connected: stopping one frees its slot synchronously, and the
    // rebind succeeds.
    fixture.run(Step::StopWs(0));
    fixture.run(Step::BindWs);
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::WsDone(MAX_BINDS_PER_CLIENT + 1)),
        "the rebind after a stop did not announce: {log:?}"
    );
    assert!(
        !log.contains(&Seen::WsFinished(MAX_BINDS_PER_CLIENT + 1)),
        "the rebind after a stop was finished: {log:?}"
    );
}

/// Same, for the ext list: the ninth bind is answered `finished` (a plain
/// event here, not a destructor -- the client destroys the object itself).
#[test]
fn ninth_toplevel_list_is_finished_not_killed() {
    let mut fixture = Fixture::new();
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run(Step::BindList);
    }
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
    fixture.run(Step::BindList);
    let log = fixture.take_log();
    assert_eq!(log, vec![Seen::ListFinished(MAX_BINDS_PER_CLIENT)]);
    // Still connected, and the slot frees on `stop` (whose own `finished`
    // is the only one in the log afterwards).
    fixture.run(Step::StopList(0));
    fixture.run(Step::BindList);
    assert_eq!(fixture.take_log(), vec![Seen::ListFinished(0)]);
}

/// Same, for the wlr manager.
#[test]
fn ninth_wlr_manager_is_finished_not_killed() {
    let mut fixture = Fixture::new();
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run(Step::BindWlr);
    }
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
    fixture.run(Step::BindWlr);
    let log = fixture.take_log();
    assert_eq!(log, vec![Seen::WlrFinished(MAX_BINDS_PER_CLIENT)]);
    fixture.run(Step::StopWlr(0));
    fixture.run(Step::BindWlr);
    assert_eq!(fixture.take_log(), vec![Seen::WlrFinished(0)]);
}

/// Same, for the output manager: `done` + `finished`, like the existing
/// mid-announce give-up.
#[test]
fn ninth_output_manager_is_finished_not_killed() {
    let mut fixture = Fixture::new();
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run(Step::BindOutput);
    }
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
    fixture.run(Step::BindOutput);
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::OutputDone(MAX_BINDS_PER_CLIENT)),
        "the ninth bind was not closed with done: {log:?}"
    );
    assert!(
        log.contains(&Seen::OutputFinished(MAX_BINDS_PER_CLIENT)),
        "the ninth bind was not finished: {log:?}"
    );
    fixture.run(Step::StopOutput(0));
    fixture.run(Step::BindOutput);
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::OutputDone(MAX_BINDS_PER_CLIENT + 1)),
        "the rebind after a stop did not announce: {log:?}"
    );
    assert!(
        !log.contains(&Seen::OutputFinished(MAX_BINDS_PER_CLIENT + 1)),
        "the rebind after a stop was finished: {log:?}"
    );
}

// -- the shared shape ------------------------------------------------------

/// The budget is shared, not per global: eight binds in any mixture fill it,
/// and the ninth -- of any kind -- is refused.
#[test]
fn the_budget_is_shared_across_globals() {
    let mut fixture = Fixture::new();
    for _ in 0..2 {
        fixture.run(Step::BindWs);
        fixture.run(Step::BindList);
        fixture.run(Step::BindWlr);
        fixture.run(Step::BindOutput);
    }
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
    fixture.take_log();
    // One of each kind past a full budget: every one refused.
    fixture.run(Step::BindWs);
    fixture.run(Step::BindList);
    fixture.run(Step::BindWlr);
    fixture.run(Step::BindOutput);
    let log = fixture.take_log();
    assert_eq!(
        log,
        vec![
            Seen::WsDone(2),
            Seen::WsFinished(2),
            Seen::ListFinished(2),
            Seen::WlrFinished(2),
            Seen::OutputDone(2),
            Seen::OutputFinished(2),
        ]
    );
}

/// Per client, not global: a greedy client past its budget denies nothing to
/// a second one.
#[test]
fn a_greedy_client_does_not_deny_a_second_client() {
    let mut fixture = Fixture::new();
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run(Step::BindWlr);
    }
    fixture.run(Step::BindWlr);
    assert_eq!(
        fixture.take_log(),
        vec![Seen::WlrFinished(MAX_BINDS_PER_CLIENT)]
    );
    fixture.spawn(run_client);
    fixture.run_on(1, Step::BindWs);
    fixture.run_on(1, Step::BindList);
    fixture.run_on(1, Step::BindWlr);
    fixture.run_on(1, Step::BindOutput);
    let log = fixture.take_log_on(1);
    assert!(
        log.contains(&Seen::WsDone(0)),
        "the second client's workspace manager did not announce: {log:?}"
    );
    assert!(
        log.contains(&Seen::OutputDone(0)),
        "the second client's output manager did not announce: {log:?}"
    );
    assert!(
        !log.iter().any(|seen| matches!(
            seen,
            Seen::WsFinished(_)
                | Seen::ListFinished(_)
                | Seen::WlrFinished(_)
                | Seen::OutputFinished(_)
        )),
        "the second client was refused for the first client's greed: {log:?}"
    );
    assert_eq!(fixture.held(1), 4);
}

/// A disconnect releases everything: the count drains to zero, and a new
/// client can fill the whole budget again.
#[test]
fn disconnecting_releases_every_bind() {
    let mut fixture = Fixture::new();
    for _ in 0..2 {
        fixture.run(Step::BindWs);
        fixture.run(Step::BindList);
        fixture.run(Step::BindWlr);
        fixture.run(Step::BindOutput);
    }
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
    fixture.disconnect(0);
    assert_eq!(fixture.held(0), 0);
    fixture.spawn(run_client);
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run_on(1, Step::BindWs);
    }
    assert_eq!(
        fixture.take_log_on(1),
        (0..MAX_BINDS_PER_CLIENT)
            .map(Seen::WsDone)
            .collect::<Vec<_>>()
    );
    assert_eq!(fixture.held(1), MAX_BINDS_PER_CLIENT as usize);
}

// -- the legitimate floor --------------------------------------------------

/// What a shell binds at startup fits with room to spare: one of every kind,
/// each answered with its announcement rather than a refusal.
#[test]
fn a_shell_binding_every_global_fits() {
    let mut fixture = Fixture::new();
    fixture.run(Step::BindWs);
    fixture.run(Step::BindList);
    fixture.run(Step::BindWlr);
    fixture.run(Step::BindOutput);
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::WsDone(0)),
        "the workspace manager did not announce: {log:?}"
    );
    assert!(
        log.contains(&Seen::OutputDone(0)),
        "the output manager did not announce: {log:?}"
    );
    assert!(
        !log.iter().any(|seen| matches!(
            seen,
            Seen::WsFinished(_)
                | Seen::ListFinished(_)
                | Seen::WlrFinished(_)
                | Seen::OutputFinished(_)
        )),
        "a legitimate bind was refused: {log:?}"
    );
    assert_eq!(fixture.held(0), 4);
}

// -- the dead-but-unpruned shapes ------------------------------------------

/// `stop` frees its slot synchronously: a stop and a bind in one batch
/// succeed even at the cap.
#[test]
fn stop_and_rebind_in_one_batch_succeeds_at_the_cap() {
    let mut fixture = Fixture::new();
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run(Step::BindWs);
    }
    fixture.take_log();
    fixture.run(Step::StopWsAndBindWs(0));
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::WsFinished(0)),
        "the stop was not answered: {log:?}"
    );
    assert!(
        log.contains(&Seen::WsDone(MAX_BINDS_PER_CLIENT)),
        "the same-batch rebind did not announce: {log:?}"
    );
    assert!(
        !log.iter()
            .any(|seen| matches!(seen, Seen::WsFinished(key) if *key == MAX_BINDS_PER_CLIENT)),
        "the same-batch rebind was refused: {log:?}"
    );
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
}

/// A bare destroy frees its slot inline: a destructor request runs its
/// `destroyed` hook in the same dispatch (not at post-batch cleanup), so a
/// destroy-and-rebind in one batch succeeds even at the cap -- the same
/// promptness `stop` has, through a different mechanism.
#[test]
fn destroy_and_rebind_in_one_batch_succeeds_at_the_cap() {
    let mut fixture = Fixture::new();
    for _ in 0..MAX_BINDS_PER_CLIENT {
        fixture.run(Step::BindList);
    }
    fixture.take_log();
    fixture.run(Step::DestroyListAndBindList(0));
    // Neither a refusal for the rebind nor anything else: the destroy freed
    // its slot before the bind was counted, and an allowed bind with no
    // windows behind it announces nothing.
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
    // And the count is exact afterwards: one stop plus one bind move it by
    // exactly one.
    fixture.run(Step::StopList(1));
    fixture.run(Step::BindList);
    assert_eq!(fixture.take_log(), vec![Seen::ListFinished(1)]);
    assert_eq!(fixture.held(0), MAX_BINDS_PER_CLIENT as usize);
}
