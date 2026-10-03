//! The keyboard layout query and event: what a bar indicator reads.
//!
//! The pins, on a live headless `State` with no client at all (a group
//! toggle moves the seat's xkb state whether or not any client has focus,
//! which is the whole reason no Wayland protocol can report it to a bar):
//!
//! 1. the `keyboard` query answers the active group's index and the
//!    keymap's name for it on a multi-layout keymap (`us,ru`);
//! 2. pressing the group toggle (Caps Lock under `grp:caps_toggle`) emits
//!    exactly one `keyboard_changed` event with the new index and name --
//!    and toggling back emits group 0 again;
//! 3. typing without a group change sends nothing, and neither does the
//!    toggle's own release: at most one event per key, none without a
//!    change (the "coalesced" half);
//! 4. rapid successive toggles emit exactly one event per change -- no
//!    duplicates, no dropped changes, alternating indices;
//! 5. with no subscriber a toggle costs no keymap read at all (the recorded
//!    group stays behind), and subscribing refreshes the record without
//!    emitting;
//! 6. a keyboard subscriber that never reads is dropped past the same
//!    high-water mark as every other subscriber, stalling nothing.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use scoot_core::Config;
use scoot_ipc::{EventKind, Request, Response, decode};
use smithay::backend::input::KeyState;
use smithay::input::keyboard::{Keycode, XkbConfig};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::set_sndbuf;
use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;

/// The canvas the headless backend renders into. Nothing here reads a
/// pixel; the backend exists so the `State` is a whole session, the way
/// `input/tests.rs`'s fixture builds one.
const CANVAS: i32 = 200;

/// Caps Lock as an xkb keycode (evdev `KEY_CAPSLOCK` 58, plus the 8 every
/// xkb keymap is built with). Under `grp:caps_toggle` this is the group
/// toggle: each press flips the effective group, each release changes
/// nothing.
const CAPS_LOCK: u32 = 58 + 8;

/// A live compositor with a real headless backend and no client: group
/// changes need no focus, so none is mapped.
struct Fixture {
    _event_loop: EventLoop<'static, State>,
    state: State,
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
            crate::compositor::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");
        Self {
            _event_loop: event_loop,
            state,
        }
    }

    /// Replaces the seat keyboard's keymap -- the way a session started
    /// with e.g. `XKB_DEFAULT_LAYOUT=us,ru` and
    /// `XKB_DEFAULT_OPTIONS=grp:caps_toggle` would have compiled it at
    /// startup.
    fn set_keymap(&mut self, layout: &str, options: Option<&str>) {
        let keyboard = self.state.seat.get_keyboard().expect("a keyboard");
        keyboard
            .set_xkb_config(
                &mut self.state,
                XkbConfig {
                    rules: "",
                    model: "",
                    layout,
                    variant: "",
                    options: options.map(str::to_owned),
                },
            )
            .expect("xkeyboard-config should compile the layout");
    }

    /// Presses and releases Caps Lock the way a real key event would arrive:
    /// through [`State::key`], the one funnel every keyboard source
    /// reaches. Under `grp:caps_toggle` the press flips the group and the
    /// release changes nothing.
    fn toggle(&mut self) {
        self.state.key(Keycode::new(CAPS_LOCK), KeyState::Pressed);
        self.state.key(Keycode::new(CAPS_LOCK), KeyState::Released);
    }
}

/// Subscribes a fresh connection to `kinds`, answering its end: what a
/// `Request::Subscribe` does once the connection loop hands it over. The
/// reader carries a timeout, so a missing event fails instead of hanging
/// the suite.
fn subscribed(state: &mut State, kinds: Vec<EventKind>) -> BufReader<UnixStream> {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let response = state.subscribe(7, server, kinds);
    assert!(
        matches!(response, Response::Subscribed { .. }),
        "subscribing answers subscribed, got {response:?}"
    );
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout, so a missing event fails instead of hanging the suite");
    BufReader::new(client)
}

/// The next message the server sent: one subscribed event.
fn next_event(reader: &mut BufReader<UnixStream>) -> Response {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("the event arrived within the timeout");
    decode(&line).expect("a decodable event")
}

/// Asserts the server sent nothing: the socket must be unreadable right
/// now. Runs the stream non-blocking for the check, then restores blocking
/// (and its timeout) for whatever the test reads next.
fn assert_silent(reader: &mut BufReader<UnixStream>) {
    reader
        .get_mut()
        .set_nonblocking(true)
        .expect("non-blocking for the silence check");
    let mut line = String::new();
    let result = reader.read_line(&mut line);
    reader
        .get_mut()
        .set_nonblocking(false)
        .expect("blocking again afterwards");
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        _ => panic!("expected silence, got {result:?} line {line:?}"),
    }
}

/// Pin 1: the query answers the active group's index and the keymap's name
/// for it, live off the keymap rather than any recorded copy.
#[test]
fn keyboard_query_reports_the_active_layout_name_and_index() {
    let mut fixture = Fixture::new();
    fixture.set_keymap("us,ru", Some("grp:caps_toggle"));
    // A toggle behind the query's back: the answer still follows the live
    // group, because the query reads the keymap and not the recorded
    // detector state (which skipped its read with no subscriber around).
    fixture.toggle();
    fixture.toggle();
    assert_eq!(
        fixture.state.last_keyboard_layout, 0,
        "no subscriber, no read"
    );
    match fixture.state.handle_request(Request::Keyboard) {
        Response::Keyboard(layout) => {
            assert_eq!(layout.index, 0);
            assert_eq!(layout.name, "English (US)");
        }
        other => panic!("the keyboard query answers its layout, got {other:?}"),
    }
}

/// Pin 2: one toggle is exactly one event, carrying the new group and its
/// name -- and toggling back reports group 0 again.
#[test]
fn a_group_toggle_emits_one_event_with_the_new_layout() {
    let mut fixture = Fixture::new();
    fixture.set_keymap("us,ru", Some("grp:caps_toggle"));
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Keyboard]);

    fixture.toggle();
    match next_event(&mut reader) {
        Response::KeyboardChanged(layout) => {
            assert_eq!(layout.index, 1);
            assert_eq!(layout.name, "Russian");
        }
        other => panic!("the toggle emits its new layout, got {other:?}"),
    }

    fixture.toggle();
    match next_event(&mut reader) {
        Response::KeyboardChanged(layout) => {
            assert_eq!(layout.index, 0);
            assert_eq!(layout.name, "English (US)");
        }
        other => panic!("toggling back emits group 0, got {other:?}"),
    }
}

/// Pin 3: typing on one layout sends nothing, and neither does a key that
/// leaves the group alone (the toggle's own release, inside every
/// `toggle()` above). One event per change, never per keypress.
#[test]
fn typing_without_a_group_change_sends_nothing() {
    let mut fixture = Fixture::new();
    fixture.set_keymap("us,ru", Some("grp:caps_toggle"));
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Keyboard]);

    fixture
        .state
        .type_text("hello")
        .expect("plain ASCII types on group 0");
    assert_silent(&mut reader);

    // A lone release changes nothing either: the press below already moved
    // the group (one event), and this release must not add a second.
    fixture
        .state
        .key(Keycode::new(CAPS_LOCK), KeyState::Pressed);
    assert!(
        matches!(next_event(&mut reader), Response::KeyboardChanged(_)),
        "the press moved the group and emits once"
    );
    fixture
        .state
        .key(Keycode::new(CAPS_LOCK), KeyState::Released);
    assert_silent(&mut reader);
}

/// Pin 4: rapid successive toggles emit exactly one event per change -- no
/// duplicates from the press/release pair, no dropped changes, alternating
/// groups in order.
#[test]
fn rapid_toggles_emit_exactly_one_event_per_change() {
    let mut fixture = Fixture::new();
    fixture.set_keymap("us,ru", Some("grp:caps_toggle"));
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Keyboard]);

    for _ in 0..5 {
        fixture.toggle();
    }
    let mut indices = Vec::new();
    for _ in 0..5 {
        match next_event(&mut reader) {
            Response::KeyboardChanged(layout) => indices.push(layout.index),
            other => panic!("each toggle emits its layout, got {other:?}"),
        }
    }
    assert_eq!(indices, vec![1, 0, 1, 0, 1]);
    assert_silent(&mut reader);
}

/// Pin 5: with no subscriber a toggle costs no keymap read -- the recorded
/// group stays behind -- and subscribing refreshes that record without
/// emitting, so the next real change emits exactly once.
#[test]
fn with_no_subscriber_a_toggle_costs_no_keymap_read() {
    let mut fixture = Fixture::new();
    fixture.set_keymap("us,ru", Some("grp:caps_toggle"));

    fixture.toggle();
    assert_eq!(
        fixture.state.last_keyboard_layout, 0,
        "the detector skipped its read with nobody subscribed"
    );

    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Keyboard]);
    assert_eq!(
        fixture.state.last_keyboard_layout, 1,
        "subscribing refreshes the record to the live group"
    );
    assert_silent(&mut reader);

    fixture.toggle();
    match next_event(&mut reader) {
        Response::KeyboardChanged(layout) => assert_eq!(layout.index, 0),
        other => panic!("the next real change emits exactly once, got {other:?}"),
    }
    assert_silent(&mut reader);
}

/// Pin 6: a keyboard subscriber that never reads is dropped past the same
/// high-water mark every subscriber observes, and the session carries on
/// without it. The socket is stuffed before subscribing, so every emission
/// queues; the queue passes 1 MiB after a few thousand small lines.
#[test]
fn a_keyboard_subscriber_that_never_reads_is_dropped() {
    let mut fixture = Fixture::new();
    fixture.set_keymap("us,ru", Some("grp:caps_toggle"));
    let (server, _client) = UnixStream::pair().expect("a socket pair");
    server.set_nonblocking(true).expect("non-blocking");
    set_sndbuf(&server, 1024);
    gorge(&server);
    let response = fixture
        .state
        .subscribe(7, server, vec![EventKind::Keyboard]);
    assert!(
        matches!(response, Response::Subscribed { .. }),
        "subscribing answers subscribed, got {response:?}"
    );

    let mut dropped_after = None;
    for n in 0..20_000u32 {
        fixture
            .state
            .emit_keyboard_changed(scoot_ipc::KeyboardLayout {
                name: "Russian".into(),
                index: n,
            });
        if fixture.state.subscribers.is_empty() {
            dropped_after = Some(n);
            break;
        }
    }
    let n = dropped_after.expect("a subscriber that never reads is dropped");
    assert!(
        n > 10,
        "dropped after {n} small lines: the first few must queue, not drop"
    );
    // And emission with nobody left is a no-op, not a panic.
    fixture.toggle();
}

/// Fills `stream`'s send buffer from its own end, so nothing more goes out
/// without a reader -- the state a client that stopped reading leaves
/// behind.
fn gorge(stream: &UnixStream) {
    let mut stream = stream;
    let chunk = [b'x'; 64 * 1024];
    for _ in 0..64 {
        match std::io::Write::write(&mut stream, &chunk) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("could not fill the socket: {error}"),
        }
    }
    panic!("the socket took 4 MiB without blocking; it is not stuffed");
}

/// An output-only subscriber never sees a layout event: filtering is by
/// kind, and the detector does not even read the keymap for one.
#[test]
fn an_output_only_subscriber_gets_no_layout_events() {
    let mut fixture = Fixture::new();
    fixture.set_keymap("us,ru", Some("grp:caps_toggle"));
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::Output]);

    fixture.toggle();
    assert_silent(&mut reader);
    assert_eq!(
        fixture.state.last_keyboard_layout, 0,
        "no keyboard subscriber, no keymap read"
    );
}
