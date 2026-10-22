//! Portal global shortcuts on the physical input path: what a person (or
//! `scoot msg key`, which funnels through the same [`State::key`]) presses.
//!
//! The pins, on a live headless `State` with no client at all (interception
//! needs no focus, which is the whole point of a global shortcut):
//!
//! 1. a bound chord's press emits exactly one `ShortcutActivated` with the
//!    press-time timestamp, and its release exactly one `ShortcutDeactivated`
//!    with a timestamp no earlier -- both intercepted, never forwarded;
//! 2. a registered-but-unbound shortcut's preferred chord forwards whole and
//!    sends nothing (the hint never auto-binds);
//! 3. a chord a live compositor bind holds stays listed but conflicted and
//!    never fires: the compositor wins (here `super+h`, which the press
//!    still runs as a focus move);
//! 4. an unregister between press and release keeps the release suppressed:
//!    release routing reads the press-time identity filed in
//!    `global_suppressed`, never the live table -- the client never saw the
//!    press, so it must not see a lone release either. The pair still
//!    closes with `Deactivated` for the stored identity;
//! 5. a reload that rebinds a held chord keeps its release suppressed too.
//!
//! The virtual-keyboard half lives in `virtual_input/tests.rs` (same grab,
//! the `[virtual_input] binds` gate); the register/bind/list/unregister
//! wire surface lives in `ipc/tests/global_shortcuts.rs`.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use scoot_core::Config;
use scoot_ipc::{EventKind, KeyCombo, Request, Response, ShortcutInfo, decode};
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::super::State;
use super::super::decorations::Appearance;
use super::super::headless;
use super::super::keybindings::Keybindings;
use super::KeyOutcome;

const CANVAS: i32 = 200;

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

    /// Registers one shortcut as `conn`, answering `Shortcuts`.
    fn register(&mut self, conn: u64, app: &str, id: &str) {
        self.state.current_conn = conn;
        match self.state.handle_request(Request::ShortcutRegister {
            app_id: app.into(),
            shortcut_id: id.into(),
            preferred_trigger: None,
            description: None,
        }) {
            Response::Shortcuts { .. } => {}
            other => panic!("register answers Shortcuts, got {other:?}"),
        }
        self.state.current_conn = 0;
    }

    /// Binds one registered shortcut to `combo`, answering `Shortcuts`.
    fn bind(&mut self, app: &str, id: &str, combo: &str) {
        let keys: KeyCombo = combo.parse().expect("a parsable chord");
        match self.state.handle_request(Request::ShortcutBind {
            app_id: app.into(),
            shortcut_id: id.into(),
            combo: Some(keys),
        }) {
            Response::Shortcuts { .. } => {}
            other => panic!("bind answers Shortcuts, got {other:?}"),
        }
    }

    fn shortcuts(&mut self) -> Vec<ShortcutInfo> {
        match self.state.handle_request(Request::Shortcuts) {
            Response::Shortcuts { shortcuts } => shortcuts,
            other => panic!("shortcuts answers Shortcuts, got {other:?}"),
        }
    }

    /// Presses `combo` the way a real key event arrives: every modifier in
    /// layout order, then the main key. Returns the main key's outcome plus
    /// every keycode pressed (modifiers first), for the release half.
    fn press(&mut self, combo: &str) -> (Vec<Keycode>, KeyOutcome) {
        let keys: KeyCombo = combo.parse().expect("a parsable chord");
        let (code, held) = self
            .state
            .resolve_combo(&keys)
            .unwrap_or_else(|error| panic!("`{combo}` is pressable on this layout: {error}"));
        let mut codes: Vec<Keycode> = held.as_slice().to_vec();
        for &modifier in &codes.clone() {
            self.state.key(modifier, KeyState::Pressed);
        }
        let outcome = self.state.key(code, KeyState::Pressed);
        codes.push(code);
        (codes, outcome)
    }

    /// Releases `codes` in reverse press order (main key first). Returns
    /// the main key's outcome.
    fn release(&mut self, codes: &[Keycode]) -> KeyOutcome {
        let mut outcome = KeyOutcome::default();
        for &code in codes.iter().rev() {
            let step = self.state.key(code, KeyState::Released);
            if code == *codes.last().expect("a main key") {
                outcome = step;
            }
        }
        outcome
    }
}

/// Subscribes a fresh connection to `kinds`: what `Request::Subscribe`
/// does once the connection loop hands it over. The reader carries a
/// timeout, so a missing event fails instead of hanging the suite.
fn subscribed(state: &mut State, kinds: Vec<EventKind>) -> BufReader<UnixStream> {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let response = state.subscribe(11, server, kinds);
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

/// Pin 1: a bound chord's press emits exactly one `Activated` and its
/// release exactly one `Deactivated`, both intercepted, timestamps ordered.
#[test]
fn a_bound_chord_press_emits_activated_and_release_emits_deactivated() {
    let mut fixture = Fixture::new();
    fixture.register(7, "org.example", "toggle");
    fixture.bind("org.example", "toggle", "super+ctrl+r");
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::GlobalShortcut]);

    let (codes, pressed) = fixture.press("super+ctrl+r");
    assert!(
        pressed.intercepted,
        "a bound global chord must be intercepted, not forwarded"
    );
    let (app, id, pressed_ts) = match next_event(&mut reader) {
        Response::ShortcutActivated {
            app_id,
            shortcut_id,
            timestamp_ms,
        } => (app_id, shortcut_id, timestamp_ms),
        other => panic!("the press emits Activated, got {other:?}"),
    };
    assert_eq!((app.as_str(), id.as_str()), ("org.example", "toggle"));
    assert_silent(&mut reader);

    let released = fixture.release(&codes);
    assert!(
        released.intercepted,
        "the matching release must be intercepted too"
    );
    match next_event(&mut reader) {
        Response::ShortcutDeactivated {
            app_id,
            shortcut_id,
            timestamp_ms,
        } => {
            assert_eq!(app_id, "org.example");
            assert_eq!(shortcut_id, "toggle");
            assert!(
                timestamp_ms >= pressed_ts,
                "the release timestamp ({timestamp_ms}) predates the press ({pressed_ts})"
            );
        }
        other => panic!("the release emits Deactivated, got {other:?}"),
    }
    assert_silent(&mut reader);
    assert!(
        fixture.state.global_suppressed.is_empty(),
        "the intercepted release must clear the suppression entry"
    );
}

/// Pin 2: a registered-but-unbound shortcut's preferred chord forwards
/// whole and sends nothing -- the hint never auto-binds.
#[test]
fn an_unbound_shortcut_forwards_its_preferred_chord_silently() {
    let mut fixture = Fixture::new();
    fixture.state.current_conn = 7;
    match fixture.state.handle_request(Request::ShortcutRegister {
        app_id: "org.example".into(),
        shortcut_id: "toggle".into(),
        preferred_trigger: Some("super+ctrl+r".into()),
        description: None,
    }) {
        Response::Shortcuts { shortcuts } => {
            assert_eq!(shortcuts.len(), 1);
            assert_eq!(shortcuts[0].bound, None);
            assert_eq!(
                shortcuts[0].preferred_trigger.as_deref(),
                Some("super+ctrl+r")
            );
        }
        other => panic!("register answers Shortcuts, got {other:?}"),
    }
    fixture.state.current_conn = 0;
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::GlobalShortcut]);

    let (codes, pressed) = fixture.press("super+ctrl+r");
    assert!(
        !pressed.intercepted,
        "an unbound shortcut must not swallow its hint chord"
    );
    let released = fixture.release(&codes);
    assert!(
        !released.intercepted,
        "a forwarded press's release must forward too"
    );
    assert_silent(&mut reader);
}

/// Pin 3: a chord a live compositor bind holds stays listed but conflicted
/// and never fires -- the compositor wins.
#[test]
fn a_chord_a_compositor_bind_holds_stays_listed_but_conflicted_and_never_fires() {
    let mut fixture = Fixture::new();
    fixture.register(7, "org.example", "toggle");
    fixture.bind("org.example", "toggle", "super+h");
    let listed = fixture.shortcuts();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].bound.as_deref(), Some("super+h"));
    assert!(
        listed[0].conflicted,
        "super+h is a default compositor bind, so this test proves nothing unless flagged"
    );
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::GlobalShortcut]);

    let (codes, pressed) = fixture.press("super+h");
    assert!(
        pressed.intercepted,
        "the compositor bind still intercepts its own chord"
    );
    assert_silent(&mut reader);
    let released = fixture.release(&codes);
    assert!(
        released.intercepted,
        "the compositor bind swallows its release"
    );
    assert_silent(&mut reader);
}

/// Pin 4: an unregister between press and release keeps the release
/// suppressed -- release routing reads the press-time identity filed in
/// `global_suppressed`, never the live table. The pair still closes with
/// `Deactivated` for the stored identity.
#[test]
fn an_unregister_mid_hold_keeps_the_release_suppressed() {
    let mut fixture = Fixture::new();
    fixture.register(7, "org.example", "toggle");
    fixture.bind("org.example", "toggle", "super+ctrl+r");
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::GlobalShortcut]);

    let (codes, pressed) = fixture.press("super+ctrl+r");
    assert!(pressed.intercepted, "the press must intercept");
    assert!(
        matches!(next_event(&mut reader), Response::ShortcutActivated { .. }),
        "the press emits Activated"
    );

    match fixture.state.handle_request(Request::ShortcutUnregister {
        app_id: "org.example".into(),
        shortcut_id: "toggle".into(),
    }) {
        Response::Shortcuts { shortcuts } => assert!(shortcuts.is_empty()),
        other => panic!("unregister answers Shortcuts, got {other:?}"),
    }
    match next_event(&mut reader) {
        Response::ShortcutsChanged => {}
        other => panic!("unregister emits ShortcutsChanged, got {other:?}"),
    }

    let released = fixture.release(&codes);
    assert!(
        released.intercepted,
        "the release was forwarded as a lone release the client never pressed"
    );
    match next_event(&mut reader) {
        Response::ShortcutDeactivated {
            app_id,
            shortcut_id,
            ..
        } => {
            assert_eq!(app_id, "org.example");
            assert_eq!(shortcut_id, "toggle");
        }
        other => panic!("the release still closes the pair, got {other:?}"),
    }
    assert_silent(&mut reader);
}

/// Pin 5: a reload that rebinds a held chord keeps its release suppressed
/// too -- same routing reason as pin 4, through the config path instead.
#[test]
fn a_reload_that_rebinds_a_held_chord_keeps_its_release_suppressed() {
    use std::fs;

    let mut fixture = Fixture::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "[global_shortcuts]\n\"org.example:toggle\" = \"super+ctrl+r\"\n",
    )
    .expect("a config file");
    fixture.state.config_path = Some(path.clone());
    // Load the file the way startup would: the chord then applies at
    // register time, like the `[binds]` half of the same reload.
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { .. } => {}
        other => panic!("the initial reload was not served: {other:?}"),
    }
    fixture.register(7, "org.example", "toggle");
    assert_eq!(
        fixture.shortcuts()[0].bound.as_deref(),
        Some("super+ctrl+r"),
        "the file's chord applies at register time"
    );
    let mut reader = subscribed(&mut fixture.state, vec![EventKind::GlobalShortcut]);

    let (codes, pressed) = fixture.press("super+ctrl+r");
    assert!(pressed.intercepted, "the press must intercept");
    assert!(
        matches!(next_event(&mut reader), Response::ShortcutActivated { .. }),
        "the press emits Activated"
    );

    fs::write(
        &path,
        "[global_shortcuts]\n\"org.example:toggle\" = \"super+ctrl+z\"\n",
    )
    .expect("a rewritten config file");
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => assert!(
            applied.contains(&"global_shortcuts".to_owned()),
            "the chord move reports global_shortcuts as applied, got {applied:?}"
        ),
        other => panic!("the rebinding reload was not served: {other:?}"),
    }
    // The reload re-resolves the registration against the new file.
    assert_eq!(
        fixture.shortcuts()[0].bound.as_deref(),
        Some("super+ctrl+z"),
        "the reload moved the chord"
    );
    match next_event(&mut reader) {
        Response::ShortcutsChanged => {}
        other => panic!("the reload emits ShortcutsChanged, got {other:?}"),
    }

    let released = fixture.release(&codes);
    assert!(
        released.intercepted,
        "the release was forwarded as a lone release the client never pressed"
    );
    assert!(
        matches!(
            next_event(&mut reader),
            Response::ShortcutDeactivated { .. }
        ),
        "the release still closes the pair"
    );
    assert_silent(&mut reader);
}
