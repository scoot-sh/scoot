//! Tests for `wlr-output-management-unstable-v1`.
//!
//! These drive a *real* `wayland-client` connection -- binding
//! `zwlr_output_manager_v1` and reacting to its events the way `wlr-randr` and
//! a shell's Display page do -- through a real [`State`], and assert on **the
//! exact sequence of events the client received**, in order.
//!
//! That is the point of every one of them. What can go wrong with an
//! advertisement protocol is a client being told the wrong thing, or told it in
//! the wrong order, or not told at all: a `current_mode` naming a mode object
//! the client never got, an event a v1 client cannot decode, a `done` that
//! never comes, a head that keeps reporting a mode the output no longer has.
//! Each of those is a sensible-looking call sequence on the compositor side and
//! a settings page showing a screen that does not exist.
//!
//! One test here is load-bearing beyond its own assertions:
//! [`head_state_agrees_with_wl_output`] binds `wl_output` and the manager on
//! the same connection and compares them field by field. This protocol must not
//! become a second source of truth about the output, and that is the executable
//! form of it.
//!
//! Like the other client-driven suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real wayland listening socket,
//! which nothing here connects to (clients are inserted as socket pairs) but
//! which is created either way.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1;
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_v1::{
    self, ZwlrOutputConfigurationV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_head_v1::{
    self, AdaptiveSyncState, ZwlrOutputHeadV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_manager_v1::{
    self, ZwlrOutputManagerV1,
};
use wayland_protocols_wlr::output_management::v1::client::zwlr_output_mode_v1::{
    self, ZwlrOutputModeV1,
};

use crate::compositor::decorations::Appearance;
use crate::compositor::headless::{self, OUTPUT_NAME};
use crate::compositor::test_support::Harness;

/// The framebuffer these tests render into, and so the output's mode. Nothing
/// here reads a pixel; the backend exists so the compositor has a real output,
/// as it does in a session.
const CANVAS: i32 = 200;

/// A second size to resize the output to, for the mode-change tests. Different
/// from [`CANVAS`] in both dimensions so a partially-applied resize cannot pass.
const RESIZED: i32 = 300;

/// The refresh rate `headless.rs`'s `set_mode` hard-codes, in mHz.
const REFRESH: i32 = 60_000;

/// One protocol event, as the client saw it.
///
/// Heads are keyed by the order the client was told about them and modes by the
/// order they were created, so an expectation can be written out literally.
/// `PartialEq` but not `Eq`: [`Seen::Scale`] carries the protocol's `fixed`
/// value as the `f64` a client receives.
#[derive(Clone, Debug, PartialEq)]
enum Seen {
    /// `zwlr_output_manager_v1.head`
    Head(u32),
    Name(u32, String),
    Description(u32, String),
    Make(u32, String),
    Model(u32, String),
    SerialNumber(u32, String),
    PhysicalSize(u32, i32, i32),
    /// `zwlr_output_head_v1.mode`, keyed by head then by the mode's own key.
    Mode(u32, u32),
    ModeSize(u32, i32, i32),
    ModeRefresh(u32, i32),
    ModePreferred(u32),
    ModeFinished(u32),
    Enabled(u32, i32),
    CurrentMode(u32, u32),
    Position(u32, i32, i32),
    Transform(u32, WEnum<wl_output::Transform>),
    Scale(u32, f64),
    AdaptiveSync(u32, WEnum<AdaptiveSyncState>),
    HeadFinished(u32),
    /// `zwlr_output_manager_v1.done`, carrying its serial.
    Done(u32),
    ManagerFinished,
    ConfigurationSucceeded,
    ConfigurationFailed,
    ConfigurationCancelled,
}

/// The whole burst one head is announced with at version 4, in the order this
/// compositor sends it.
///
/// `head` and `mode` are the keys the client will have assigned; `modes` is the
/// output's mode list, newest last, and `current` the index in it of the mode
/// in use (which is also the preferred one, since `set_mode` sets both).
fn announced(head: u32, first_mode: u32, modes: &[i32], current: usize, version: u32) -> Vec<Seen> {
    let mut seen = vec![
        Seen::Head(head),
        Seen::Name(head, OUTPUT_NAME.to_string()),
        // `make - model - name`, which is how Smithay builds an output's
        // description (verified against the pinned rev's `Output::new`).
        Seen::Description(head, format!("scoot - {OUTPUT_NAME} - {OUTPUT_NAME}")),
    ];
    // No `physical_size`: scoot's is (0, 0). No `serial_number`: scoot's is a
    // placeholder. See the module doc for both.
    if version >= 2 {
        seen.push(Seen::Make(head, "scoot".to_string()));
        seen.push(Seen::Model(head, OUTPUT_NAME.to_string()));
    }
    for (index, size) in modes.iter().enumerate() {
        let key = first_mode + index as u32;
        seen.push(Seen::Mode(head, key));
        seen.push(Seen::ModeSize(key, *size, *size));
        seen.push(Seen::ModeRefresh(key, REFRESH));
        if index == current {
            seen.push(Seen::ModePreferred(key));
        }
    }
    seen.push(Seen::Enabled(head, 1));
    seen.push(Seen::CurrentMode(head, first_mode + current as u32));
    seen.push(Seen::Position(head, 0, 0));
    seen.push(Seen::Transform(
        head,
        WEnum::Value(wl_output::Transform::Normal),
    ));
    seen.push(Seen::Scale(head, 1.0));
    if version >= 4 {
        seen.push(Seen::AdaptiveSync(
            head,
            WEnum::Value(AdaptiveSyncState::Disabled),
        ));
    }
    seen
}

/// One `wl_output.mode` event, as the client saw it: the size plus which of
/// the two defined flag bits were set. Recorded for *every* event rather than
/// snapshotted like [`OutputRecord::current_mode`], so a test can tell what an
/// already-bound client was told by a resize apart from what a later bind saw.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WlMode {
    width: i32,
    height: i32,
    refresh: i32,
    current: bool,
    preferred: bool,
}

/// What `wl_output` itself said, for the cross-check.
#[derive(Clone, Debug, Default, PartialEq)]
struct OutputRecord {
    name: Option<String>,
    description: Option<String>,
    make: Option<String>,
    model: Option<String>,
    position: Option<(i32, i32)>,
    physical_size: Option<(i32, i32)>,
    transform: Option<WEnum<wl_output::Transform>>,
    scale: Option<i32>,
    /// `(width, height, refresh)` of whichever mode carried the `current` flag.
    current_mode: Option<(i32, i32, i32)>,
    /// Every `mode` event in arrival order, with its flags.
    modes: Vec<WlMode>,
}

#[derive(Default)]
struct TestClient {
    /// Bound on demand so a test can choose the version, and so a test can bind
    /// before the output exists.
    manager_name: Option<(u32, u32)>,
    output_name: Option<(u32, u32)>,
    managers: Vec<ZwlrOutputManagerV1>,
    heads: Vec<ZwlrOutputHeadV1>,
    modes: Vec<ZwlrOutputModeV1>,
    /// The serial of the most recent `done`, for `create_configuration`.
    serial: u32,
    /// Every event since the last [`Step::TakeLog`], in arrival order.
    log: Vec<Seen>,
    output: OutputRecord,
    /// The configuration built by the last apply/test step, kept so a later
    /// step can send another request on it.
    configuration: Option<ZwlrOutputConfigurationV1>,
}

impl TestClient {
    fn head_key(&self, head: &ZwlrOutputHeadV1) -> u32 {
        key_of(&self.heads, head)
    }

    fn mode_key(&self, mode: &ZwlrOutputModeV1) -> u32 {
        key_of(&self.modes, mode)
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
        _registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
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
            "zwlr_output_manager_v1" => client.manager_name = Some((name, version)),
            "wl_output" => client.output_name = Some((name, version)),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _manager: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                client.heads.push(head);
                let key = u32::try_from(client.heads.len() - 1).expect("few heads");
                client.log.push(Seen::Head(key));
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                client.serial = serial;
                client.log.push(Seen::Done(serial));
            }
            zwlr_output_manager_v1::Event::Finished => client.log.push(Seen::ManagerFinished),
            _ => {}
        }
    }

    event_created_child!(TestClient, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = client.head_key(head);
        let seen = match event {
            zwlr_output_head_v1::Event::Name { name } => Seen::Name(key, name),
            zwlr_output_head_v1::Event::Description { description } => {
                Seen::Description(key, description)
            }
            zwlr_output_head_v1::Event::Make { make } => Seen::Make(key, make),
            zwlr_output_head_v1::Event::Model { model } => Seen::Model(key, model),
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                Seen::SerialNumber(key, serial_number)
            }
            zwlr_output_head_v1::Event::PhysicalSize { width, height } => {
                Seen::PhysicalSize(key, width, height)
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                client.modes.push(mode);
                let mode_key = u32::try_from(client.modes.len() - 1).expect("few modes");
                Seen::Mode(key, mode_key)
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => Seen::Enabled(key, enabled),
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                Seen::CurrentMode(key, client.mode_key(&mode))
            }
            zwlr_output_head_v1::Event::Position { x, y } => Seen::Position(key, x, y),
            zwlr_output_head_v1::Event::Transform { transform } => Seen::Transform(key, transform),
            zwlr_output_head_v1::Event::Scale { scale } => Seen::Scale(key, scale),
            zwlr_output_head_v1::Event::AdaptiveSync { state } => Seen::AdaptiveSync(key, state),
            zwlr_output_head_v1::Event::Finished => Seen::HeadFinished(key),
            _ => return,
        };
        client.log.push(seen);
    }

    event_created_child!(TestClient, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = client.mode_key(mode);
        let seen = match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                Seen::ModeSize(key, width, height)
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => Seen::ModeRefresh(key, refresh),
            zwlr_output_mode_v1::Event::Preferred => Seen::ModePreferred(key),
            zwlr_output_mode_v1::Event::Finished => Seen::ModeFinished(key),
            _ => return,
        };
        client.log.push(seen);
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _configuration: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let seen = match event {
            zwlr_output_configuration_v1::Event::Succeeded => Seen::ConfigurationSucceeded,
            zwlr_output_configuration_v1::Event::Failed => Seen::ConfigurationFailed,
            zwlr_output_configuration_v1::Event::Cancelled => Seen::ConfigurationCancelled,
            _ => return,
        };
        client.log.push(seen);
    }
}

impl Dispatch<wl_output::WlOutput, ()> for TestClient {
    fn event(
        client: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let record = &mut client.output;
        match event {
            wl_output::Event::Geometry {
                x,
                y,
                physical_width,
                physical_height,
                make,
                model,
                transform,
                ..
            } => {
                record.position = Some((x, y));
                record.physical_size = Some((physical_width, physical_height));
                record.make = Some(make);
                record.model = Some(model);
                record.transform = Some(transform);
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                refresh,
            } => {
                let (current, preferred) = flags.into_result().map_or((false, false), |flags| {
                    (
                        flags.contains(wl_output::Mode::Current),
                        flags.contains(wl_output::Mode::Preferred),
                    )
                });
                record.modes.push(WlMode {
                    width,
                    height,
                    refresh,
                    current,
                    preferred,
                });
                if current {
                    record.current_mode = Some((width, height, refresh));
                }
            }
            wl_output::Event::Scale { factor } => record.scale = Some(factor),
            wl_output::Event::Name { name } => record.name = Some(name),
            wl_output::Event::Description { description } => record.description = Some(description),
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore ZwlrOutputConfigurationHeadV1);

/// One instruction for the client thread.
enum Step {
    /// Bind another `zwlr_output_manager_v1`, at `version`.
    BindManager(u32),
    /// Bind `wl_output` at version 4, so the two can be cross-checked.
    BindOutput,
    /// `stop` on the `index`-th manager.
    Stop(usize),
    /// `release` on the `index`-th head (version 3+).
    ReleaseHead(usize),
    /// `release` on every mode object the client holds (version 3+).
    ReleaseModes,
    /// Build a configuration on manager 0 naming head 0, describe it, and
    /// `apply` it.
    Apply,
    /// The same, ending in `test` instead.
    Test,
    /// Send `enable_head` again on the configuration the last [`Step::Apply`]
    /// or [`Step::Test`] built -- which the protocol makes a fatal error.
    ReuseConfiguration,
    /// Hand back (and clear) everything seen so far.
    TakeLog,
    /// Hand back what `wl_output` reported.
    TakeOutput,
}

enum Ack {
    Done,
    Log(Vec<Seen>),
    Output(Box<OutputRecord>),
}

/// Runs the client half: executes whatever steps the test sends, acknowledging
/// each one once the compositor has seen it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let manager_global = client
        .manager_name
        .ok_or("no zwlr_output_manager_v1 -- the global is missing")?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let mut outcome = Ack::Done;
        match step {
            Step::BindManager(version) => {
                let manager: ZwlrOutputManagerV1 =
                    registry.bind(manager_global.0, manager_global.1.min(version), &qh, ());
                client.managers.push(manager);
            }
            Step::BindOutput => {
                let (name, version) = client.output_name.ok_or("no wl_output")?;
                let _: wl_output::WlOutput = registry.bind(name, version.min(4), &qh, ());
            }
            Step::Stop(index) => client.managers.get(index).ok_or("no such manager")?.stop(),
            Step::ReleaseHead(index) => client.heads.get(index).ok_or("no such head")?.release(),
            Step::ReleaseModes => {
                for mode in &client.modes {
                    mode.release();
                }
            }
            Step::Apply | Step::Test => {
                let manager = client.managers.first().ok_or("no manager")?.clone();
                let head = client.heads.first().ok_or("no head")?.clone();
                let mode = client.modes.first().ok_or("no mode")?.clone();
                let configuration = manager.create_configuration(client.serial, &qh, ());
                let configured = configuration.enable_head(&head, &qh, ());
                configured.set_mode(&mode);
                configured.set_position(10, 20);
                configured.set_scale(2.0);
                if matches!(step, Step::Apply) {
                    configuration.apply();
                } else {
                    configuration.test();
                }
                client.configuration = Some(configuration);
            }
            Step::ReuseConfiguration => {
                let configuration = client
                    .configuration
                    .clone()
                    .ok_or("no configuration was built")?;
                let head = client.heads.first().ok_or("no head")?.clone();
                let _ = configuration.enable_head(&head, &qh, ());
            }
            Step::TakeLog => outcome = Ack::Log(std::mem::take(&mut client.log)),
            Step::TakeOutput => outcome = Ack::Output(Box::new(client.output.clone())),
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match &mut outcome {
            // Anything that arrived during the round trip above belongs to this
            // batch too: the step before a `TakeLog` may have provoked events
            // that were still in flight when it was acknowledged.
            Ack::Log(log) => log.extend(std::mem::take(&mut client.log)),
            Ack::Output(record) => **record = client.output.clone(),
            Ack::Done => {}
        }
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
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

    /// The same, with no backend and so no output at all.
    fn without_output() -> Self {
        let mut fixture = Harness::bare(Appearance::default());
        fixture.spawn(run_client);
        fixture
    }

    /// A fixture whose client has bound the manager at `version`, with the
    /// initial burst already drained.
    fn bound(version: u32) -> Self {
        let mut fixture = Self::new();
        fixture.run(Step::BindManager(version));
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
            _ => panic!("the client answered a log request with nothing"),
        }
    }

    fn take_output(&mut self) -> OutputRecord {
        match self.run(Step::TakeOutput) {
            Ack::Output(record) => *record,
            _ => panic!("the client answered an output request with nothing"),
        }
    }

    /// How many managers the compositor is keeping.
    fn tracked(&self) -> usize {
        self.state.output_management.managers.len()
    }
}

// -- what a client is told -----------------------------------------------

#[test]
fn binding_announces_the_one_output_as_a_head() {
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager(4));

    // The whole burst, in order, closed by exactly one `done`. Serial 1, not 0:
    // `headless::init` announced the head before this client connected, which
    // is the one refresh that has happened.
    let mut expected = announced(0, 0, &[CANVAS], 0, 4);
    expected.push(Seen::Done(1));
    assert_eq!(fixture.take_log(), expected);
    assert_eq!(fixture.tracked(), 1, "the manager should be registered");
}

#[test]
fn head_state_agrees_with_wl_output() {
    // The one test that pins this protocol to the same source of truth as
    // `wl_output`: both are bound on the same connection and compared field by
    // field. A regression that made this module cache or recompute anything
    // instead of reading the `Output` shows up here.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindManager(4));
    let log = fixture.take_log();
    let output = fixture.take_output();

    let mut name = None;
    let mut description = None;
    let mut make = None;
    let mut model = None;
    let mut position = None;
    let mut transform = None;
    let mut scale = None;
    let mut current_mode = None;
    let mut sizes = Vec::new();
    for seen in &log {
        match seen {
            Seen::Name(_, value) => name = Some(value.clone()),
            Seen::Description(_, value) => description = Some(value.clone()),
            Seen::Make(_, value) => make = Some(value.clone()),
            Seen::Model(_, value) => model = Some(value.clone()),
            Seen::Position(_, x, y) => position = Some((*x, *y)),
            Seen::Transform(_, value) => transform = Some(*value),
            Seen::Scale(_, value) => scale = Some(*value),
            Seen::CurrentMode(_, key) => current_mode = Some(*key),
            Seen::ModeSize(key, w, h) => sizes.push((*key, *w, *h)),
            _ => {}
        }
    }

    assert_eq!(name, output.name, "name");
    assert_eq!(description, output.description, "description");
    assert_eq!(make, output.make, "make");
    assert_eq!(model, output.model, "model");
    assert_eq!(position, output.position, "position");
    assert_eq!(transform, output.transform, "transform");
    // `wl_output.scale` is an integer, this protocol's a `fixed` -- so the
    // relationship is `ceil`, not equality (see `output_scale.rs`). At the
    // default scale both are 1, and this states which of the two is the
    // rounding.
    let scale = scale.expect("a scale event");
    assert_eq!(
        Some(scale.ceil() as i32),
        output.scale,
        "wl_output.scale should be ceil() of the head scale"
    );
    let key = current_mode.expect("a current_mode event");
    let (_, width, height) = sizes
        .iter()
        .copied()
        .find(|(mode, _, _)| *mode == key)
        .expect("a size for the current mode");
    assert_eq!(
        Some((width, height, REFRESH)),
        output.current_mode,
        "the current mode"
    );
    // Neither event is sent, and that agrees with `wl_output` reporting (0, 0):
    // scoot knows no physical size. Asserted so a future backend that learns
    // one has to update both sides, not just this protocol.
    assert_eq!(
        output.physical_size,
        Some((0, 0)),
        "wl_output physical size"
    );
    assert!(
        !log.iter()
            .any(|seen| matches!(seen, Seen::PhysicalSize(..) | Seen::SerialNumber(..))),
        "physical_size and serial_number should not be sent: {log:?}"
    );
}

#[test]
fn a_version_one_client_hears_only_version_one_events() {
    // wayland-backend does not check an event's `since` on the way out, so an
    // ungated `make` would reach a v1 client as an opcode it cannot decode --
    // which shows up as the client dying, not as a wrong value.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager(1));

    let mut expected = announced(0, 0, &[CANVAS], 0, 1);
    expected.push(Seen::Done(1));
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn a_version_two_client_hears_make_and_model_but_not_adaptive_sync() {
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager(2));

    let mut expected = announced(0, 0, &[CANVAS], 0, 2);
    expected.push(Seen::Done(1));
    assert_eq!(fixture.take_log(), expected);
}

#[test]
fn binding_before_the_output_exists_announces_no_head() {
    // Not reachable in a session -- `headless::init_named` runs before the
    // event loop -- but it is the zero-output case, and the honest answer is a
    // bare `done` rather than silence a client would wait on forever.
    let mut fixture = Fixture::without_output();
    fixture.run(Step::BindManager(4));
    assert_eq!(fixture.take_log(), vec![Seen::Done(0)]);

    // ...and the head arrives in its own batch once there is an output.
    headless::init(&mut fixture.state, CANVAS, CANVAS).expect("a headless backend");
    fixture.settle();
    let mut expected = announced(0, 0, &[CANVAS], 0, 4);
    expected.push(Seen::Done(1));
    assert_eq!(fixture.take_log(), expected);
}

// -- what changes, and what does not -------------------------------------

#[test]
fn a_quiet_compositor_sends_nothing() {
    let mut fixture = Fixture::bound(4);
    // The refresh path is called from more than one place; calling it with
    // nothing changed must not produce a `done`, or a bar would redraw on every
    // window movement.
    fixture.state.refresh_output_heads();
    fixture.state.apply();
    fixture.settle();
    assert_eq!(fixture.take_log(), vec![]);
}

#[test]
fn a_resize_sends_the_new_mode_and_a_fresh_serial() {
    let mut fixture = Fixture::bound(4);
    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();

    // The old mode is *not* withdrawn: `Output::modes` only grows (see the
    // module doc), and `wl_output` keeps advertising it too. The new one is
    // introduced, becomes current, and is preferred -- `set_mode` sets both.
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::Mode(0, 1),
            Seen::ModeSize(1, RESIZED, RESIZED),
            Seen::ModeRefresh(1, REFRESH),
            Seen::ModePreferred(1),
            Seen::CurrentMode(0, 1),
            Seen::Done(2),
        ]
    );
}

#[test]
fn wl_output_tells_an_already_bound_client_the_resized_mode_is_preferred() {
    // `headless::set_mode` must mark the new mode preferred *before* Smithay's
    // `change_current_state` sends it to already-bound `wl_output` clients:
    // that call snapshots the preferred mode synchronously (verified against
    // the pinned rev's `wayland/output/mod.rs`), so the old order told a
    // client bound before the resize about the new mode with no `preferred`
    // bit, and nothing ever resent it. A client binding *after* the resize
    // gets the full state at bind time either way (pinned below).
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindManager(4));
    fixture.take_log();
    let before = fixture.take_output();
    assert!(
        before
            .modes
            .iter()
            .any(|mode| (mode.width, mode.height) == (CANVAS, CANVAS)
                && mode.current
                && mode.preferred),
        "the bind-time mode should arrive current and preferred: {before:?}"
    );

    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();

    // The management protocol agrees: its snapshot is taken after `set_mode`
    // has fully returned, so it was correct even before the fix -- asserting
    // both here pins the two protocols to each other across a resize.
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::Mode(0, 1),
            Seen::ModeSize(1, RESIZED, RESIZED),
            Seen::ModeRefresh(1, REFRESH),
            Seen::ModePreferred(1),
            Seen::CurrentMode(0, 1),
            Seen::Done(2),
        ]
    );
    let after = fixture.take_output();
    assert!(
        after.modes.contains(&WlMode {
            width: RESIZED,
            height: RESIZED,
            refresh: REFRESH,
            current: true,
            preferred: true,
        }),
        "the already-bound client should hear the resized mode as current \
         *and* preferred: {after:?}"
    );
}

#[test]
fn wl_output_reports_preferred_to_a_client_bound_after_resize() {
    // Regression pin: the bind path sends the whole current state, preferred
    // bit included, so this passed before the fix too -- stated, not assumed.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager(4));
    fixture.take_log();
    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();
    fixture.take_log();

    fixture.run(Step::BindOutput);
    let output = fixture.take_output();
    assert!(
        output.modes.contains(&WlMode {
            width: RESIZED,
            height: RESIZED,
            refresh: REFRESH,
            current: true,
            preferred: true,
        }),
        "a client binding after the resize should see the mode as current \
         and preferred: {output:?}"
    );
}

#[test]
fn resizing_to_the_same_mode_keeps_it_current_and_preferred() {
    // Same-size edge: the preferred mode already names this mode, so even the
    // old order sent both bits here. Pins that the swap changes nothing about
    // the no-op resize both backends can produce (a hotplug re-probe landing
    // on the size it is already on never reaches `set_mode`, but a nested
    // host configuring exactly `--width`x`--height` does reach this shape at
    // startup... and is acked-and-ignored before it; this is the direct call).
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.take_output();

    fixture.state.resize_output(CANVAS, CANVAS);
    fixture.settle();
    let output = fixture.take_output();
    assert!(
        output
            .modes
            .last()
            .is_some_and(|mode| (mode.width, mode.height) == (CANVAS, CANVAS)
                && mode.current
                && mode.preferred),
        "a same-mode resize should still report the mode as current and \
         preferred: {output:?}"
    );
}

#[test]
fn resizing_back_to_a_known_mode_reuses_its_object() {
    // The protocol sends `mode` once per supported mode, so going back to a
    // size the output already knows must name the object the client already
    // has rather than introducing a second one for the same mode.
    let mut fixture = Fixture::bound(4);
    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();
    fixture.take_log();

    fixture.state.resize_output(CANVAS, CANVAS);
    fixture.settle();
    assert_eq!(
        fixture.take_log(),
        vec![Seen::CurrentMode(0, 0), Seen::Done(3)]
    );
}

#[test]
fn two_managers_are_kept_in_step() {
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    fixture.run(Step::BindManager(4));
    fixture.run_on(1, Step::BindManager(4));
    fixture.take_log();
    fixture.take_log_on(1);
    assert_eq!(fixture.tracked(), 2);

    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();

    // The same batch, the same serial, on both connections -- the object keys
    // are per-client, so they agree too.
    let expected = vec![
        Seen::Mode(0, 1),
        Seen::ModeSize(1, RESIZED, RESIZED),
        Seen::ModeRefresh(1, REFRESH),
        Seen::ModePreferred(1),
        Seen::CurrentMode(0, 1),
        Seen::Done(2),
    ];
    assert_eq!(fixture.take_log(), expected);
    assert_eq!(fixture.take_log_on(1), expected);
}

// -- lifecycle -----------------------------------------------------------

#[test]
fn stop_is_answered_with_finished_and_ends_the_events() {
    let mut fixture = Fixture::bound(4);
    fixture.run(Step::Stop(0));
    assert_eq!(fixture.take_log(), vec![Seen::ManagerFinished]);
    assert_eq!(fixture.tracked(), 0, "a stopped manager should be dropped");

    // ...and a later change reaches nobody. The client's head and mode objects
    // are left alone, which is what the protocol's teardown sequence expects.
    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();
    assert_eq!(fixture.take_log(), vec![]);
}

#[test]
fn binding_and_immediately_stopping_leaks_nothing() {
    // A client that binds, reads the state once and leaves -- which is what
    // `wlr-randr` does, every time it runs. Nothing may accumulate across the
    // repeats.
    let mut fixture = Fixture::new();
    for round in 0..5 {
        fixture.run(Step::BindManager(4));
        assert_eq!(fixture.tracked(), 1, "round {round}");
        fixture.run(Step::Stop(round));
        assert_eq!(fixture.tracked(), 0, "round {round}");
    }
    fixture.take_log();
}

#[test]
fn a_disconnected_client_is_forgotten() {
    let mut fixture = Fixture::bound(4);
    assert_eq!(fixture.tracked(), 1);
    fixture.disconnect(0);
    assert_eq!(fixture.tracked(), 0, "a dead client's manager should go");

    // And the refresh path survives having nothing to send to.
    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();
}

#[test]
fn releasing_the_head_stops_its_updates() {
    let mut fixture = Fixture::bound(3);
    fixture.run(Step::ReleaseHead(0));
    fixture.take_log();

    // The manager is still registered -- `release` is about the head object,
    // not the manager -- so `done` still closes each batch, with nothing in it.
    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();
    assert_eq!(fixture.take_log(), vec![Seen::Done(2)]);
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn a_released_mode_is_never_named_as_current() {
    // Releasing a mode object means the client is done with it. Coming back to
    // that mode must not resurrect it, and must not introduce a second object
    // for the same mode either -- so the batch carries no `current_mode` at
    // all.
    let mut fixture = Fixture::bound(3);
    fixture.run(Step::ReleaseModes);
    fixture.state.resize_output(RESIZED, RESIZED);
    fixture.settle();
    fixture.take_log();

    fixture.state.resize_output(CANVAS, CANVAS);
    fixture.settle();
    assert_eq!(fixture.take_log(), vec![Seen::Done(3)]);
}

// -- the refused write half ----------------------------------------------

#[test]
fn apply_is_always_refused() {
    let mut fixture = Fixture::bound(4);
    fixture.run(Step::Apply);
    assert_eq!(fixture.take_log(), vec![Seen::ConfigurationFailed]);
    // And nothing moved: a refusal that silently changed the output would be
    // the worst of both answers.
    let output = fixture.state.output.clone().expect("the output");
    assert_eq!(
        output.current_mode().map(|mode| (mode.size.w, mode.size.h)),
        Some((CANVAS, CANVAS)),
        "the mode should be untouched"
    );
    assert_eq!(output.current_location().x, 0, "the position");
    assert_eq!(
        output.current_scale().fractional_scale(),
        1.0,
        "the scale should be untouched"
    );
}

#[test]
fn test_is_always_refused() {
    let mut fixture = Fixture::bound(4);
    fixture.run(Step::Test);
    assert_eq!(fixture.take_log(), vec![Seen::ConfigurationFailed]);
}

#[test]
fn reusing_a_configuration_is_a_protocol_error() {
    let mut fixture = Fixture::bound(4);
    fixture.run(Step::Apply);
    fixture.take_log();
    let error = fixture.run_expecting_disconnect(Step::ReuseConfiguration);
    assert!(
        error.contains("already been applied or tested"),
        "unexpected error: {error}"
    );
}
