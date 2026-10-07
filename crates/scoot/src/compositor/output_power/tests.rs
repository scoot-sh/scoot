//! Tests for `zwlr_output_power_manager_v1` and the IPC `output-power` request.
//!
//! These drive *real* `wayland-client` connections -- binding the manager
//! and flipping modes exactly as `wlopm` does -- through a real [`State`]
//! with a real headless backend. What is under test is what the client
//! observes (`mode`, `failed`, survival vs. protocol error) plus what the
//! compositor still holds afterwards, which a handler-level test cannot see.
//!
//! The client runs on its own thread while the test pumps the compositor;
//! each test is one linear script reporting back one result. See
//! `gamma_control/tests.rs` for the shared shape.
//!
//! Two cases have no test here, deliberately:
//!
//! - `get_output_power` naming an unknown output: unreachable from a real
//!   client. `wl_output` objects are unforgeable -- the only one that can be
//!   named is the compositor's own, which always validates -- so the
//!   `failed` branch is defence for a removed output, not a reachable path
//!   for a live one. Removal itself *is* tested (`removing_the_output...`).
//! - Applying to real hardware: headless has no panel, so everything here
//!   asserts accept/retire semantics. The `--tty` DPMS path is exercised on
//!   the dev VM (see the PR description), not in this file.
//!
//! These need a writable `$XDG_RUNTIME_DIR` for the same reason the other
//! compositor tests do.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use scoot_core::Config;
use scoot_ipc::{Request, Response};
use smithay::reexports::calloop::EventLoop;
/// The server half's `WEnum`: the client and server bindings are generated
/// into different crates (scoot's own `wayland-protocols-wlr` vs. Smithay's
/// re-export), so their `Mode` types never meet -- only the raw values do,
/// the way `gamma_control/tests.rs` compares sizes rather than enums.
use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_v1 as server_power;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::WEnum as ServerWEnum;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1, zwlr_output_power_v1,
};

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// How long a client script may take before the compositor counts as not
/// answering. Generous: a debug build on a VM.
const PATIENCE: Duration = Duration::from_secs(20);

/// A live compositor with a real headless backend, serving client threads
/// that each run one script to completion.
struct Harness {
    event_loop: EventLoop<'static, State>,
    state: State,
}

impl Harness {
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
            crate::compositor::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, 200, 200).expect("a headless backend");
        Self { event_loop, state }
    }

    /// Connects one client over a socket pair and runs `script` on its
    /// thread.
    fn run_client(
        &mut self,
        script: impl FnOnce(ClientConn) -> Result<String, String> + Send + 'static,
    ) -> JoinHandle<Result<String, String>> {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        self.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");
        std::thread::spawn(move || {
            let conn = ClientConn::new(client_end)?;
            script(conn)
        })
    }

    /// Pumps the compositor until the client thread reports back, failing the
    /// test if the compositor stops serving first.
    fn wait_for(&mut self, handle: JoinHandle<Result<String, String>>) -> Result<String, String> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if handle.is_finished() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the client; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
        handle.join().expect("the client thread")
    }

    /// Dispatches until nothing more arrives, so destructions the client
    /// caused (an explicit destroy, or its disconnect) have run their
    /// server-side hooks before the test inspects [`State`].
    fn settle(&mut self) {
        for _ in 0..20 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }
}

/// The client end of one connection: the socket, queue and dispatch state.
struct ClientConn {
    queue: wayland_client::EventQueue<Client>,
    client: Client,
}

impl ClientConn {
    fn new(stream: UnixStream) -> Result<Self, String> {
        let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = Client::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        Ok(Self { queue, client })
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        self.queue
            .roundtrip(&mut self.client)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Everything a power client binds or observes.
#[derive(Default)]
struct Client {
    manager: Option<zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1>,
    /// Every `wl_output` global, in advertisement order (creation order).
    outputs: Vec<wl_output::WlOutput>,
    /// One slot per power object created, in creation order: every `mode`
    /// seen on it, and whether it has been told `failed`.
    modes: Vec<Vec<WEnum<zwlr_output_power_v1::Mode>>>,
    failed: Vec<bool>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    locked: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Client {
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
            "zwlr_output_power_manager_v1" => {
                client.manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_output" => client
                .outputs
                .push(registry.bind(name, version.min(4), qh, ())),
            "ext_session_lock_manager_v1" => {
                client.lock_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
        _: zwlr_output_power_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Which power object in creation order an event belongs to: the manager
/// hands over the object first and its `mode` after.
struct ControlIndex(usize);

impl Dispatch<zwlr_output_power_v1::ZwlrOutputPowerV1, ControlIndex> for Client {
    fn event(
        client: &mut Self,
        _: &zwlr_output_power_v1::ZwlrOutputPowerV1,
        event: zwlr_output_power_v1::Event,
        index: &ControlIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_power_v1::Event::Mode { mode } => {
                if let Some(slot) = client.modes.get_mut(index.0) {
                    slot.push(mode);
                }
            }
            zwlr_output_power_v1::Event::Failed => {
                if let Some(slot) = client.failed.get_mut(index.0) {
                    *slot = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_manager_v1::ExtSessionLockManagerV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        _: ext_session_lock_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, ext_session_lock_v1::Event::Locked) {
            client.locked = true;
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_output::WlOutput);

/// Rounds the client's queue until `ready` sees what the test is waiting for.
///
/// Wall-clock deadline, not a roundtrip count: under CPU contention each
/// roundtrip returns fast with nothing new while the event the test waits
/// for (e.g. another client's `set_mode`) has not been sent yet, so a
/// fixed count burns out before it arrives. Generous: a debug build on a
/// loaded box.
const WAIT_FOR_EVENT: Duration = Duration::from_secs(10);

fn wait_for_event(
    conn: &mut ClientConn,
    what: &str,
    mut ready: impl FnMut(&Client) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + WAIT_FOR_EVENT;
    loop {
        conn.roundtrip()?;
        if ready(&conn.client) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("the compositor never sent {what}"));
        }
    }
}

/// Binds the manager and the first output, or says which one is missing.
/// Every single-output power test starts here.
fn manager_and_output(
    conn: &ClientConn,
) -> Result<
    (
        zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
        wl_output::WlOutput,
    ),
    String,
> {
    let manager = conn
        .client
        .manager
        .clone()
        .ok_or("no zwlr_output_power_manager_v1 -- the global is missing")?;
    let output = conn.client.outputs.first().cloned().ok_or("no wl_output")?;
    Ok((manager, output))
}

fn mode_value(mode: &WEnum<zwlr_output_power_v1::Mode>) -> u32 {
    match mode {
        WEnum::Value(value) => *value as u32,
        WEnum::Unknown(raw) => *raw,
    }
}

fn is_off(mode: &WEnum<zwlr_output_power_v1::Mode>) -> bool {
    mode_value(mode) == server_power::Mode::Off as u32
}

fn is_on(mode: &WEnum<zwlr_output_power_v1::Mode>) -> bool {
    mode_value(mode) == server_power::Mode::On as u32
}

/// Creates one power object and waits for its initial `mode` event.
fn create_control(
    conn: &mut ClientConn,
    manager: &zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    output: &wl_output::WlOutput,
) -> Result<zwlr_output_power_v1::ZwlrOutputPowerV1, String> {
    let qh = conn.queue.handle();
    let index = conn.client.modes.len();
    conn.client.modes.push(Vec::new());
    conn.client.failed.push(false);
    let control = manager.get_output_power(output, &qh, ControlIndex(index));
    wait_for_event(conn, "the initial mode", |client| {
        !client.modes[index].is_empty()
    })?;
    Ok(control)
}

#[test]
fn the_initial_mode_event_is_on() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let (manager, output) = manager_and_output(&conn)?;
        let _control = create_control(&mut conn, &manager, &output)?;
        let modes = &conn.client.modes[0];
        if modes.len() != 1 || !is_on(&modes[0]) {
            return Err(format!("expected one initial on-mode, saw {modes:?}"));
        }
        Ok("on".into())
    });
    assert_eq!(harness.wait_for(handle).expect("the client"), "on");
}

#[test]
fn set_mode_off_reaches_every_holder() {
    let mut harness = Harness::new();
    // Two clients hold a control for the same output; the first one turns
    // it off. Both must hear it -- the broadcast, not just the asker. That
    // both hear it also proves there is no exclusivity transfer: had the
    // second creation failed the first (gamma's shape), the first's
    // `set_mode` would have landed on a stale object and nobody would hear
    // anything.
    let first = harness.run_client(|mut conn| {
        let (manager, output) = manager_and_output(&conn)?;
        let control = create_control(&mut conn, &manager, &output)?;
        control.set_mode(zwlr_output_power_v1::Mode::Off);
        wait_for_event(&mut conn, "the off mode", |client| {
            client.modes[0].iter().any(is_off)
        })?;
        Ok("first heard off".into())
    });
    let second = harness.run_client(|mut conn| {
        let (manager, output) = manager_and_output(&conn)?;
        let _control = create_control(&mut conn, &manager, &output)?;
        wait_for_event(&mut conn, "the off mode", |client| {
            client.modes[0].iter().any(is_off)
        })?;
        // No exclusivity: the second control is still live (exactly one
        // holder per client, both counted server-side).
        Ok("second heard off".into())
    });
    assert_eq!(
        harness.wait_for(first).expect("the first client"),
        "first heard off"
    );
    assert_eq!(
        harness.wait_for(second).expect("the second client"),
        "second heard off"
    );
    harness.settle();
    assert_eq!(
        harness.state.output_power.live_control_count(),
        0,
        "both clients are gone, so both controls are forgotten"
    );
}

#[test]
fn set_mode_on_restores_and_rebroadcasts() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let (manager, output) = manager_and_output(&conn)?;
        let control = create_control(&mut conn, &manager, &output)?;
        control.set_mode(zwlr_output_power_v1::Mode::Off);
        wait_for_event(&mut conn, "the off mode", |client| {
            client.modes[0].iter().any(is_off)
        })?;
        control.set_mode(zwlr_output_power_v1::Mode::On);
        wait_for_event(&mut conn, "the on mode again", |client| {
            client.modes[0].len() >= 3
        })?;
        let modes = &conn.client.modes[0];
        if !is_on(modes.last().expect("a last mode")) {
            return Err(format!("expected the last mode to be on, saw {modes:?}"));
        }
        Ok("on again".into())
    });
    assert_eq!(harness.wait_for(handle).expect("the client"), "on again");
}

#[test]
fn an_unknown_mode_maps_to_the_invalid_mode_refusal() {
    use super::mode_to_on;

    assert_eq!(
        mode_to_on(ServerWEnum::Value(server_power::Mode::Off)),
        Some(false)
    );
    assert_eq!(
        mode_to_on(ServerWEnum::Value(server_power::Mode::On)),
        Some(true)
    );
    // What a raw client sends for a mode this version does not define: the
    // only refusal path `set_mode` has, pinned here because no generated
    // client binding can deliver it (an out-of-range value panics
    // client-side before it reaches the wire).
    assert_eq!(mode_to_on(ServerWEnum::Unknown(7)), None);
}

#[test]
fn destroying_a_control_keeps_the_state() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let (manager, output) = manager_and_output(&conn)?;
        let control = create_control(&mut conn, &manager, &output)?;
        control.set_mode(zwlr_output_power_v1::Mode::Off);
        wait_for_event(&mut conn, "the off mode", |client| {
            client.modes[0].iter().any(is_off)
        })?;
        control.destroy();
        conn.roundtrip()?;
        // A fresh object for the same output reports the kept state, not on.
        let _control = create_control(&mut conn, &manager, &output)?;
        let modes = &conn.client.modes[1];
        if modes.len() != 1 || !is_off(&modes[0]) {
            return Err(format!("expected one initial off-mode, saw {modes:?}"));
        }
        Ok("still off".into())
    });
    assert_eq!(harness.wait_for(handle).expect("the client"), "still off");
}

#[test]
fn removing_the_output_fails_its_controls_and_forgets_the_state() {
    let mut harness = Harness::new();
    let second =
        headless::add_output(&mut harness.state, "SECOND", 200, 200).expect("a second output");
    // The client controls the second output (advertisement order is
    // creation order), parks on it, and reports the `failed`.
    let handle = harness.run_client(|mut conn| {
        let manager = conn
            .client
            .manager
            .clone()
            .ok_or("no zwlr_output_power_manager_v1")?;
        // The second output arrives over the registry after connect; wait
        // on a wall-clock deadline for the same reason `wait_for_event`
        // does (a fixed roundtrip count burns out under load).
        let deadline = Instant::now() + WAIT_FOR_EVENT;
        while conn.client.outputs.len() < 2 {
            conn.roundtrip()?;
            if Instant::now() >= deadline {
                break;
            }
        }
        let output = conn
            .client
            .outputs
            .get(1)
            .cloned()
            .ok_or("no second wl_output")?;
        let _control = create_control(&mut conn, &manager, &output)?;
        wait_for_event(&mut conn, "failed", |client| client.failed[0])?;
        Ok("failed".into())
    });
    // Wait for the control to exist, then power the output off and remove
    // it from the test thread.
    let deadline = Instant::now() + PATIENCE;
    loop {
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
        if harness.state.output_power.live_control_count() == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the client never created its control"
        );
    }
    let response = harness.state.handle_request(Request::OutputPower {
        output: Some(second.0),
        powered: false,
    });
    assert!(
        matches!(response, Response::Ok { .. }),
        "powering off the second output: {response:?}"
    );
    harness.state.remove_output(second);
    assert_eq!(harness.wait_for(handle).expect("the client"), "failed");
    harness.settle();
    assert_eq!(
        harness.state.output_power.live_control_count(),
        0,
        "removal fails every control for the output"
    );
    // A replugged monitor comes back on under a fresh id.
    let fresh =
        headless::add_output(&mut harness.state, "SECOND", 200, 200).expect("a replugged output");
    assert_ne!(fresh, second, "ids are never reused");
    let response = harness.state.handle_request(Request::Outputs);
    let Response::Outputs { outputs } = response else {
        panic!("outputs must answer: {response:?}");
    };
    assert!(
        outputs.iter().all(|output| output.powered),
        "a replugged output starts on: {outputs:?}"
    );
}

#[test]
fn ipc_output_power_drives_the_same_state() {
    use std::sync::mpsc::channel;

    let mut harness = Harness::new();
    // The client reports its initial mode over the channel, so the test
    // thread's IPC never races it: an off that landed before the initial
    // event would read as the initial one.
    let (tx, rx) = channel::<()>();
    let handle = harness.run_client(move |mut conn| {
        let (manager, output) = manager_and_output(&conn)?;
        let _control = create_control(&mut conn, &manager, &output)?;
        tx.send(()).map_err(|e| e.to_string())?;
        // Off over IPC: the protocol holder hears it all the same (scoot
        // itself changing the mode broadcasts like any set_mode).
        wait_for_event(&mut conn, "the IPC off mode", |client| {
            client.modes[0].iter().any(is_off)
        })?;
        // ...and back on again.
        wait_for_event(&mut conn, "the IPC on mode", |client| {
            client.modes[0].iter().any(is_on)
        })?;
        Ok("heard both".into())
    });
    // Pump until the client has its initial mode, then drive IPC.
    let deadline = Instant::now() + PATIENCE;
    loop {
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
        if rx.try_recv().is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the client never got its initial mode"
        );
    }
    let response = harness.state.handle_request(Request::OutputPower {
        output: Some(1),
        powered: false,
    });
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    // While off, the outputs snapshot says so and screenshots refuse.
    let response = harness.state.handle_request(Request::Outputs);
    let Response::Outputs { outputs } = response else {
        panic!("outputs must answer: {response:?}");
    };
    assert_eq!(outputs.len(), 1);
    assert!(!outputs[0].powered, "outputs reports the off state");
    assert!(
        harness
            .state
            .screenshot_refusal(Some(1))
            .is_some_and(|message| message.contains("powered off")),
        "a powered-off output refuses screenshots"
    );
    let response = harness.state.handle_request(Request::OutputPower {
        output: None,
        powered: true,
    });
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    assert_eq!(harness.wait_for(handle).expect("the client"), "heard both");
    harness.settle();
    let response = harness.state.handle_request(Request::Outputs);
    let Response::Outputs { outputs } = response else {
        panic!("outputs must answer: {response:?}");
    };
    assert!(outputs[0].powered, "all-on powers it back on");
    assert!(
        harness.state.screenshot_refusal(Some(1)).is_none(),
        "screenshots work again once on"
    );
}

#[test]
fn an_unknown_output_id_is_refused() {
    let mut harness = Harness::new();
    let response = harness.state.handle_request(Request::OutputPower {
        output: Some(99),
        powered: false,
    });
    let Response::Error { message } = response else {
        panic!("an unknown output id must be refused, saw: {response:?}");
    };
    assert!(
        message.contains("no such output"),
        "the refusal names the problem: {message}"
    );
}

#[test]
fn a_repeat_set_is_a_silent_noop() {
    use std::sync::mpsc::channel;

    let mut harness = Harness::new();
    let (tx, rx) = channel::<()>();
    let handle = harness.run_client(move |mut conn| {
        let (manager, output) = manager_and_output(&conn)?;
        let _control = create_control(&mut conn, &manager, &output)?;
        tx.send(()).map_err(|e| e.to_string())?;
        wait_for_event(&mut conn, "the off mode", |client| {
            client.modes[0].iter().any(is_off)
        })?;
        // Settle past any straggler, then count: exactly initial + one.
        for _ in 0..10 {
            conn.roundtrip()?;
        }
        Ok(format!("{} modes", conn.client.modes[0].len()))
    });
    let deadline = Instant::now() + PATIENCE;
    loop {
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
        if rx.try_recv().is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the client never got its initial mode"
        );
    }
    for _ in 0..3 {
        let response = harness.state.handle_request(Request::OutputPower {
            output: Some(1),
            powered: false,
        });
        assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    }
    assert_eq!(harness.wait_for(handle).expect("the client"), "2 modes");
}

#[test]
fn rendering_skips_a_powered_off_output() {
    let mut harness = Harness::new();
    harness.state.request_render();
    harness.state.render();
    assert!(!harness.state.needs_render, "a drawn frame clears the flag");
    let response = harness.state.handle_request(Request::OutputPower {
        output: Some(1),
        powered: false,
    });
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    // The power change asked for a render; the frame draws nothing and
    // still clears the flag rather than spinning the timer on dark screens.
    assert!(harness.state.needs_render, "off asks for a render");
    harness.state.render();
    assert!(
        !harness.state.needs_render,
        "a frame with every output off clears the flag"
    );
}

#[test]
fn power_control_works_under_lock_and_survives_unlock() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let (manager, output) = manager_and_output(&conn)?;
        let _control = create_control(&mut conn, &manager, &output)?;
        let lock_manager = conn.client.lock_manager.clone().ok_or("no lock manager")?;
        let lock = lock_manager.lock(&qh, ());
        conn.roundtrip()?;
        // Hold the lock until the test thread has driven its half.
        for _ in 0..100 {
            conn.roundtrip()?;
            if conn.client.locked {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !conn.client.locked {
            return Err("the compositor never sent locked".into());
        }
        // Stay parked on the lock until the main thread says the IPC half
        // ran (two extra modes: off under lock, still off after unlock is
        // asserted server-side; here just wait for the on-broadcast).
        wait_for_event(&mut conn, "the on mode after unlock", |client| {
            client.modes[0].iter().any(is_on) && client.modes[0].len() >= 3
        })?;
        // Unlocking is its own request: `destroy` while locked is a
        // protocol error (Smithay refuses it), `unlock_and_destroy` ends
        // the session.
        lock.unlock_and_destroy();
        conn.roundtrip()?;
        Ok("locked, heard off, heard on".into())
    });
    // Wait for the lock to confirm (zero surfaces: the first blanked frame
    // confirms), rendering until it does.
    let deadline = Instant::now() + PATIENCE;
    while !harness.state.session_lock.is_locked() {
        assert!(Instant::now() < deadline, "the lock never confirmed");
        harness.state.render();
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
    }
    // Off under lock: session-level like gamma, never refused.
    let response = harness.state.handle_request(Request::OutputPower {
        output: Some(1),
        powered: false,
    });
    assert!(
        matches!(response, Response::Ok { locked: true }),
        "power control works under lock: {response:?}"
    );
    harness.state.render();
    harness.settle();
    // Unlock: destroy the client's lock object by ending its script half is
    // racy, so power back on first (still locked -- allowed), then let the
    // client destroy its lock and check the state survived.
    let response = harness.state.handle_request(Request::OutputPower {
        output: Some(1),
        powered: true,
    });
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    assert_eq!(
        harness.wait_for(handle).expect("the client"),
        "locked, heard off, heard on"
    );
    harness.settle();
    assert!(
        !harness.state.session_lock.is_locked(),
        "the client destroyed its lock"
    );
    let response = harness.state.handle_request(Request::Outputs);
    let Response::Outputs { outputs } = response else {
        panic!("outputs must answer: {response:?}");
    };
    assert!(outputs[0].powered, "the state survives the unlock");
}

#[test]
fn a_lock_confirms_while_its_output_is_off() {
    let mut harness = Harness::new();
    // Off first: the render loop skips the output from here on.
    let response = harness.state.handle_request(Request::OutputPower {
        output: Some(1),
        powered: false,
    });
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let lock_manager = conn.client.lock_manager.clone().ok_or("no lock manager")?;
        let _lock = lock_manager.lock(&qh, ());
        for _ in 0..100 {
            conn.roundtrip()?;
            if conn.client.locked {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !conn.client.locked {
            return Err("the compositor never sent locked".into());
        }
        Ok("locked while off".into())
    });
    // Pump and render: the off output records blanked without drawing
    // (dark is blank), so the lock confirms instead of hanging on the
    // fallback timeout.
    let deadline = Instant::now() + PATIENCE;
    while !harness.state.session_lock.is_locked() {
        assert!(Instant::now() < deadline, "the lock never confirmed");
        harness.state.render();
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
    }
    assert_eq!(
        harness.wait_for(handle).expect("the client"),
        "locked while off"
    );
}
