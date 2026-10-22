//! The portal-global-shortcut wire surface: register, bind, list and
//! unregister over IPC, plus the lifecycle and validation rules around
//! them. No keypresses here -- the grab itself is pinned in
//! `input/global_tests.rs` (physical path) and `virtual_input/tests.rs`
//! (virtual path). Like every other live-`State` test module here, these
//! need a writable `$XDG_RUNTIME_DIR`: [`State::new`] binds a real
//! listening socket.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use scoot_core::Config;
use scoot_ipc::{EventKind, KeyCombo, Request, Response, decode};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;

const CANVAS: i32 = 200;

/// A live compositor with a real headless backend and no client.
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

    /// Runs one request as `conn` (the registering owner).
    fn request_as(&mut self, conn: u64, request: Request) -> Response {
        self.state.current_conn = conn;
        let response = self.state.handle_request(request);
        self.state.current_conn = 0;
        response
    }

    fn register(
        &mut self,
        conn: u64,
        app: &str,
        id: &str,
        trigger: Option<&str>,
    ) -> Vec<scoot_ipc::ShortcutInfo> {
        match self.request_as(
            conn,
            Request::ShortcutRegister {
                app_id: app.into(),
                shortcut_id: id.into(),
                preferred_trigger: trigger.map(str::to_owned),
                description: None,
            },
        ) {
            Response::Shortcuts { shortcuts } => shortcuts,
            other => panic!("register answers Shortcuts, got {other:?}"),
        }
    }

    fn bind(&mut self, app: &str, id: &str, combo: Option<&str>) -> Response {
        let combo = combo.map(|raw| {
            raw.parse::<KeyCombo>()
                .unwrap_or_else(|_| panic!("`{raw}` parses"))
        });
        self.state.handle_request(Request::ShortcutBind {
            app_id: app.into(),
            shortcut_id: id.into(),
            combo,
        })
    }

    fn list(&mut self) -> Vec<scoot_ipc::ShortcutInfo> {
        match self.state.handle_request(Request::Shortcuts) {
            Response::Shortcuts { shortcuts } => shortcuts,
            other => panic!("shortcuts answers Shortcuts, got {other:?}"),
        }
    }
}

/// Subscribes a fresh connection to `kinds`. The reader carries a timeout,
/// so a missing event fails instead of hanging the suite. `conn` is the
/// subscription's id: subscribing twice under one id replaces the first
/// (see `State::subscribe`), so each subscriber here takes its own.
fn subscribed(state: &mut State, conn: u64, kinds: Vec<EventKind>) -> BufReader<UnixStream> {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let response = state.subscribe(conn, server, kinds);
    assert!(
        matches!(response, Response::Subscribed { .. }),
        "subscribing answers subscribed, got {response:?}"
    );
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout, so a missing event fails instead of hanging the suite");
    BufReader::new(client)
}

fn next_event(reader: &mut BufReader<UnixStream>) -> Response {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("the event arrived within the timeout");
    decode(&line).expect("a decodable event")
}

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

fn error(response: Response) -> String {
    match response {
        Response::Error { message } => message,
        other => panic!("expected an error, got {other:?}"),
    }
}

/// Register stores the hint and lists the shortcut unbound; bind assigns
/// the user chord; both emit `ShortcutsChanged` to subscribers.
#[test]
fn register_bind_and_list_round_trip_with_a_changed_event_each_time() {
    let mut fixture = Fixture::new();
    let mut reader = subscribed(&mut fixture.state, 11, vec![EventKind::GlobalShortcut]);

    let listed = fixture.register(7, "org.example", "toggle", Some("super+ctrl+r"));
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].app_id, "org.example");
    assert_eq!(listed[0].shortcut_id, "toggle");
    assert_eq!(listed[0].preferred_trigger.as_deref(), Some("super+ctrl+r"));
    assert_eq!(listed[0].bound, None);
    assert!(!listed[0].conflicted);
    assert!(
        matches!(next_event(&mut reader), Response::ShortcutsChanged),
        "register emits ShortcutsChanged"
    );

    match fixture.bind("org.example", "toggle", Some("super+ctrl+r")) {
        Response::Shortcuts { shortcuts } => {
            assert_eq!(shortcuts.len(), 1);
            assert_eq!(shortcuts[0].bound.as_deref(), Some("super+ctrl+r"));
            assert!(!shortcuts[0].conflicted);
        }
        other => panic!("bind answers Shortcuts, got {other:?}"),
    }
    assert!(
        matches!(next_event(&mut reader), Response::ShortcutsChanged),
        "bind emits ShortcutsChanged"
    );

    // `reset` (a null combo) clears the chord but keeps the registration.
    match fixture.bind("org.example", "toggle", None) {
        Response::Shortcuts { shortcuts } => {
            assert_eq!(shortcuts.len(), 1);
            assert_eq!(shortcuts[0].bound, None);
        }
        other => panic!("reset answers Shortcuts, got {other:?}"),
    }
    assert!(
        matches!(next_event(&mut reader), Response::ShortcutsChanged),
        "reset emits ShortcutsChanged"
    );
    assert_silent(&mut reader);
}

/// Register is an idempotent upsert: re-registering moves ownership to the
/// newest connection, so a stale first registrant's disconnect cannot drop
/// a live shortcut -- and the live owner's disconnect does.
#[test]
fn re_register_moves_ownership_and_only_the_live_owner_clears_it() {
    let mut fixture = Fixture::new();
    fixture.register(7, "org.example", "toggle", None);
    fixture.register(8, "org.example", "toggle", None);

    fixture.state.drop_global_shortcuts_for(7);
    assert_eq!(
        fixture.list().len(),
        1,
        "the stale owner's disconnect must not drop a re-registered shortcut"
    );
    fixture.state.drop_global_shortcuts_for(8);
    assert!(
        fixture.list().is_empty(),
        "the live owner's disconnect clears its shortcuts"
    );
}

/// A dead registrant's chords go quiet: dropping its connection clears the
/// entries and emits `ShortcutsChanged`.
#[test]
fn a_dead_registrant_clears_its_shortcuts_with_a_changed_event() {
    let mut fixture = Fixture::new();
    let mut reader = subscribed(&mut fixture.state, 11, vec![EventKind::GlobalShortcut]);
    fixture.register(9, "org.example", "one", None);
    fixture.register(9, "org.example", "two", None);
    assert!(matches!(
        next_event(&mut reader),
        Response::ShortcutsChanged
    ));
    assert!(matches!(
        next_event(&mut reader),
        Response::ShortcutsChanged
    ));
    assert_eq!(fixture.list().len(), 2);

    fixture.state.drop_global_shortcuts_for(9);
    assert!(
        fixture.list().is_empty(),
        "the disconnect clears every entry the connection owned"
    );
    assert!(
        matches!(next_event(&mut reader), Response::ShortcutsChanged),
        "the cleanup emits ShortcutsChanged"
    );
    assert_silent(&mut reader);

    // And a connection that owned nothing changes nothing (no event).
    fixture.state.drop_global_shortcuts_for(9);
    assert_silent(&mut reader);
}

/// Malformed ids, unknown ids and bad chords are all loud refusals, and a
/// refusal changes nothing (in particular it emits no `ShortcutsChanged`).
#[test]
fn malformed_unknown_and_bad_chords_are_loud_refusals() {
    let mut fixture = Fixture::new();
    let mut reader = subscribed(&mut fixture.state, 11, vec![EventKind::GlobalShortcut]);

    for (app, id) in [
        ("", "toggle"),
        ("org.example", ""),
        ("has space", "toggle"),
        ("org.example", "has space"),
        ("org/example", "toggle"),
    ] {
        let message = error(fixture.request_as(
            7,
            Request::ShortcutRegister {
                app_id: app.into(),
                shortcut_id: id.into(),
                preferred_trigger: None,
                description: None,
            },
        ));
        assert!(
            message.contains("app_id and shortcut_id"),
            "`{app}:{id}` refused with the id rule, got: {message}"
        );
    }
    assert_silent(&mut reader);

    let message = error(fixture.state.handle_request(Request::ShortcutUnregister {
        app_id: "org.nope".into(),
        shortcut_id: "nothing".into(),
    }));
    assert!(message.contains("no such shortcut"), "got: {message}");

    let message = error(fixture.bind("org.nope", "nothing", Some("super+r")));
    assert!(message.contains("no such shortcut"), "got: {message}");

    fixture.register(7, "org.example", "toggle", None);
    assert!(matches!(
        next_event(&mut reader),
        Response::ShortcutsChanged
    ));
    // A chord no layout can press: refused, nothing bound, no event.
    let message = error(fixture.bind("org.example", "toggle", Some("super+notakey")));
    assert!(message.contains("unknown key"), "got: {message}");
    assert_eq!(fixture.list()[0].bound, None);
    assert_silent(&mut reader);
}

/// A config-file chord applies at register time: the file owns assignment,
/// the app's hint never does.
#[test]
fn a_config_chord_applies_at_register_time() {
    let mut fixture = Fixture::new();
    fixture.state.global_shortcuts.config.insert(
        ("org.example".into(), "toggle".into()),
        "super+ctrl+r".into(),
    );
    fixture.state.global_shortcuts.apply_config();
    let listed = fixture.register(7, "org.example", "toggle", Some("super+alt+r"));
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].bound.as_deref(),
        Some("super+ctrl+r"),
        "the file's chord wins over the app's hint"
    );
}

/// A `Shortcuts` subscription starts silent and only `GlobalShortcut`
/// subscribers hear the changed event.
#[test]
fn only_global_shortcut_subscribers_hear_shortcuts_changed() {
    let mut fixture = Fixture::new();
    let mut global = subscribed(&mut fixture.state, 11, vec![EventKind::GlobalShortcut]);
    let mut output = subscribed(&mut fixture.state, 12, vec![EventKind::Output]);

    fixture.register(7, "org.example", "toggle", None);
    assert!(
        matches!(next_event(&mut global), Response::ShortcutsChanged),
        "the GlobalShortcut subscriber hears it"
    );
    assert_silent(&mut output);
}

/// Caps and string limits are ordinary IPC refusals: an error answer, no
/// `ShortcutsChanged`, nothing stored.
#[test]
fn caps_and_string_limits_are_ordinary_ipc_refusals() {
    use crate::compositor::global_shortcuts::{
        MAX_DESCRIPTION_LEN, MAX_PREFERRED_TRIGGER_LEN, MAX_SHORTCUTS_PER_CONNECTION,
        MAX_SHORTCUTS_TOTAL,
    };
    let mut fixture = Fixture::new();
    let mut reader = subscribed(&mut fixture.state, 11, vec![EventKind::GlobalShortcut]);

    for long in [
        Request::ShortcutRegister {
            app_id: "org.example".into(),
            shortcut_id: "toggle".into(),
            preferred_trigger: Some("x".repeat(MAX_PREFERRED_TRIGGER_LEN + 1)),
            description: None,
        },
        Request::ShortcutRegister {
            app_id: "org.example".into(),
            shortcut_id: "toggle".into(),
            preferred_trigger: None,
            description: Some("y".repeat(MAX_DESCRIPTION_LEN + 1)),
        },
    ] {
        let message = error(fixture.request_as(7, long));
        assert!(message.contains("refused"), "got: {message}");
        assert!(fixture.list().is_empty());
        assert_silent(&mut reader);
    }

    for i in 0..MAX_SHORTCUTS_PER_CONNECTION {
        fixture.request_as(
            7,
            Request::ShortcutRegister {
                app_id: "org.example".into(),
                shortcut_id: format!("id-{i}"),
                preferred_trigger: None,
                description: None,
            },
        );
        assert!(matches!(
            next_event(&mut reader),
            Response::ShortcutsChanged
        ));
    }
    let message = error(fixture.request_as(
        7,
        Request::ShortcutRegister {
            app_id: "org.example".into(),
            shortcut_id: "one-more".into(),
            preferred_trigger: None,
            description: None,
        },
    ));
    assert!(message.contains("per connection"), "got: {message}");
    assert_eq!(fixture.list().len(), MAX_SHORTCUTS_PER_CONNECTION);
    assert_silent(&mut reader);

    // Re-registering an owned id is an upsert, not a new registration.
    match fixture.request_as(
        7,
        Request::ShortcutRegister {
            app_id: "org.example".into(),
            shortcut_id: "id-0".into(),
            preferred_trigger: Some("super+r".into()),
            description: None,
        },
    ) {
        Response::Shortcuts { .. } => {}
        other => panic!("a re-register answers Shortcuts, got {other:?}"),
    }
    assert!(matches!(
        next_event(&mut reader),
        Response::ShortcutsChanged
    ));
    assert_eq!(fixture.list().len(), MAX_SHORTCUTS_PER_CONNECTION);

    // The global cap: fill the table directly, then one more over IPC.
    fixture.state.global_shortcuts = Default::default();
    for i in 0..MAX_SHORTCUTS_TOTAL {
        fixture
            .state
            .global_shortcuts
            .register(
                format!("org.app{i}"),
                "rec".into(),
                None,
                None,
                1 + (i as u64 % 8),
            )
            .expect("up to the total cap");
    }
    let message = error(fixture.request_as(
        9,
        Request::ShortcutRegister {
            app_id: "org.overflow".into(),
            shortcut_id: "rec".into(),
            preferred_trigger: None,
            description: None,
        },
    ));
    assert!(
        message.contains(&MAX_SHORTCUTS_TOTAL.to_string()),
        "got: {message}"
    );
    assert_eq!(fixture.list().len(), MAX_SHORTCUTS_TOTAL);
    assert_silent(&mut reader);
}
