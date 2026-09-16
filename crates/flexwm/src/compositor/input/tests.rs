//! Tests for injected input.
//!
//! Two layers, for two different questions:
//!
//! - The small pure mappings ([`keysym_for_char`], [`keysym_named`],
//!   [`code`], [`clamp_to_extent`]) are arithmetic and lookup tables, tested
//!   directly.
//! - Whether [`State::type_text`] actually *delivers* the text asked of it
//!   cannot be seen from inside the compositor at all. The bug this module
//!   was extended for -- every shifted character silently arriving as its
//!   unshifted twin -- looked completely correct from the compositor side:
//!   the right keycode was pressed, a key event went out, and three rounds
//!   of hardware testing missed it because nobody typed a capital letter.
//!   So these drive a *real* `wayland-client` toplevel through a real
//!   [`State`], build the client's own `xkb::State` from the keymap the
//!   compositor sent it, and assert on the text that client decoded -- the
//!   same thing a terminal or a text field would have shown.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket, which
//! nothing here connects to (the client is a socket pair) but which is
//! created either way.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use flexwm_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::utils::Serial;
use wayland_client::protocol::{wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

// -------------------------------------------------------------------------
// Pure mappings
// -------------------------------------------------------------------------

#[test]
fn newline_and_friends_map_to_named_keys_not_utf32() {
    assert_eq!(keysym_for_char('\n'), Keysym::Return);
    assert_eq!(keysym_for_char('\r'), Keysym::Return);
    assert_eq!(keysym_for_char('\t'), Keysym::Tab);
}

#[test]
fn printable_ascii_falls_back_to_utf32_to_keysym() {
    // Untouched by the control-character special case, so this should
    // still go through the general xkbcommon mapping.
    assert_eq!(keysym_for_char('a'), xkb::utf32_to_keysym('a' as u32));
    assert_eq!(keysym_for_char('!'), xkb::utf32_to_keysym('!' as u32));
}

#[test]
fn keysym_named_accepts_exact_and_case_insensitive_names() {
    assert_eq!(keysym_named("Return"), Some(Keysym::Return));
    assert_eq!(keysym_named("return"), Some(Keysym::Return));
    assert_eq!(keysym_named("ctrl+shift+t"), None);
    assert_eq!(keysym_named("not-a-real-key"), None);
}

#[test]
fn button_codes_match_linux_input_event_codes() {
    assert_eq!(code(PointerButton::Left), BTN_LEFT);
    assert_eq!(code(PointerButton::Right), BTN_RIGHT);
    assert_eq!(code(PointerButton::Middle), BTN_MIDDLE);
}

#[test]
fn clamp_to_extent_keeps_values_inside_the_output() {
    assert_eq!(clamp_to_extent(-5.0, 800), 0.0);
    assert_eq!(clamp_to_extent(5.0, 800), 5.0);
    assert_eq!(clamp_to_extent(900.0, 800), 799.0);
    // No output yet: everything clamps to the origin.
    assert_eq!(clamp_to_extent(50.0, 0), 0.0);
}

#[test]
fn modifier_keysyms_are_the_left_variant() {
    assert_eq!(modifier_keysym(Modifier::Ctrl), Keysym::Control_L);
    assert_eq!(modifier_keysym(Modifier::Shift), Keysym::Shift_L);
    assert_eq!(modifier_keysym(Modifier::Alt), Keysym::Alt_L);
    assert_eq!(modifier_keysym(Modifier::Super), Keysym::Super_L);
}

// -------------------------------------------------------------------------
// A live compositor and a live client with a real keyboard
// -------------------------------------------------------------------------

/// The headless output these run on. Nothing here looks at a pixel; it only
/// has to be big enough to be a legitimate output.
const CANVAS: i32 = 200;

/// One instruction for the client thread.
enum Step {
    /// Map an `xdg_toplevel`, which is what flexwm hands keyboard focus to.
    MapWindow,
    /// Report everything the client's own `wl_keyboard` has decoded so far.
    Report,
}

/// What the client's `wl_keyboard` actually received -- the only evidence
/// that settles "did `type_text` deliver this string", since every wrong
/// answer this ever gave looked right from the compositor's side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Typed {
    /// The text the client decoded, from the keymap the compositor sent it
    /// and the modifier state the compositor told it about -- exactly what a
    /// terminal would have put on screen.
    text: String,
    /// `wl_keyboard.key` events seen, press and release alike. A modifier
    /// held around a character is a real key press of its own, so this is
    /// what tells "held Shift" apart from "sent a different keysym".
    keys: u32,
    /// Whether the client holds keyboard focus. Without it every assertion
    /// below would pass vacuously on an empty string.
    focused: bool,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    /// Created from the seat's `Capabilities` event, so the client never
    /// asks for a keyboard the compositor didn't advertise.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The client's own xkb state, compiled from the keymap fd the
    /// compositor sent over `wl_keyboard.keymap` -- i.e. the real decoding
    /// path every toolkit uses, not a second copy of the compositor's.
    xkb: Option<xkb::State>,
    typed: Typed,
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
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()))
            }
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(5), qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for TestClient {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
            && client.keyboard.is_none()
        {
            client.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                // SAFETY: the fd is the one the compositor just sent for
                // exactly this purpose, and it is mapped copy-on-write and
                // read-only by `new_from_fd` itself (the v7+ requirement).
                let keymap = unsafe {
                    xkb::Keymap::new_from_fd(
                        &context,
                        fd,
                        size as usize,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                };
                client.xkb = keymap
                    .expect("the keymap fd should be readable")
                    .map(|keymap| xkb::State::new(&keymap));
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                // A client tracks modifiers *only* from this event -- never
                // by feeding key events to its own xkb state -- which is
                // what makes the decoding below a test of what the
                // compositor said rather than of what it meant.
                if let Some(state) = client.xkb.as_mut() {
                    state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(state),
                ..
            } => {
                client.typed.keys += 1;
                if state == wl_keyboard::KeyState::Pressed
                    && let Some(xkb_state) = client.xkb.as_ref()
                {
                    // Wayland carries evdev keycodes; xkb's are those plus 8.
                    let text = xkb_state.key_get_utf8(Keycode::new(key + 8));
                    client.typed.text.push_str(&text);
                }
            }
            wl_keyboard::Event::Enter { .. } => client.typed.focused = true,
            wl_keyboard::Event::Leave { .. } => client.typed.focused = false,
            _ => {}
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

/// Runs the client half: binds the globals, then executes whatever steps the
/// test sends, acknowledging each one once the compositor has seen it.
fn run_client(
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<Typed>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    // Twice: the first round trip binds the seat, the second delivers the
    // seat's capabilities -- and the `wl_keyboard` is only created from
    // those, so the keymap cannot arrive before it.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
    let mut windows: Vec<wl_surface::WlSurface> = Vec::new();

    while let Ok(step) = steps.recv() {
        match step {
            Step::MapWindow => {
                let surface = compositor.create_surface(&qh, ());
                let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
                let _toplevel = xdg.get_toplevel(&qh, ());
                surface.commit();
                windows.push(surface);
            }
            Step::Report => {}
        }
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(client.typed.clone()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time -- the shape `layer_shell/tests.rs` established.
struct Fixture {
    event_loop: EventLoop<'static, State>,
    state: State,
    steps: Option<Sender<Step>>,
    acks: Receiver<Typed>,
    client: Option<JoinHandle<Result<(), String>>>,
    /// Who the scripted client is, compositor-side. Needed because an input
    /// serial is only evidence for the client the event went *to* (see
    /// `interaction.rs`), so the tests below have to name it.
    client_id: ClientId,
    /// A second client that connects and does nothing else -- no surfaces, no
    /// focus, never sent an input event. What a background client trying to
    /// spend someone else's serial looks like.
    bystander_id: ClientId,
    /// The bystander's own end of its socket, held so it stays connected.
    #[allow(dead_code)]
    bystander: UnixStream,
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
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        let client_id = state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client")
            .id();
        let (bystander_end, bystander) = UnixStream::pair().expect("a socket pair");
        let bystander_id = state
            .display_handle
            .insert_client(bystander_end, Arc::new(ClientState::default()))
            .expect("an inserted client")
            .id();

        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let handle = thread::spawn(move || run_client(client_end, step_rx, ack_tx));

        let mut fixture = Self {
            event_loop,
            state,
            steps: Some(step_tx),
            acks: ack_rx,
            client: Some(handle),
            client_id,
            bystander_id,
            bystander,
        };
        fixture.run(Step::MapWindow);
        // Asserted here, once, rather than in every test: without focus the
        // client receives nothing and every assertion below would pass
        // against an empty string.
        assert!(
            fixture.run(Step::Report).focused,
            "the mapped window should hold keyboard focus before anything is typed"
        );
        fixture
    }

    /// Runs one client step to completion, then lets the compositor settle.
    fn run(&mut self, step: Step) -> Typed {
        self.steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
        let acks = std::mem::replace(&mut self.acks, channel().1);
        let report = self.wait_for(&acks);
        self.acks = acks;
        self.settle();
        report
    }

    /// Dispatches until `channel` produces a value. A client that died
    /// instead of answering is reported with its own error rather than as a
    /// ten-second timeout -- the channel disconnecting is exactly that case.
    fn wait_for(&mut self, channel: &Receiver<Typed>) -> Typed {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match channel.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => {
                    let outcome = self
                        .client
                        .take()
                        .map(|handle| handle.join().expect("the client thread"));
                    panic!("the client stopped instead of answering: {outcome:?}");
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the client; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
    }

    /// A few dispatch cycles with nothing outstanding, then an explicit
    /// flush: key events queued by `type_text` have nothing else that would
    /// push them out (the display source only flushes when the *client*
    /// writes), and `event_loop.dispatch` is not the compositor's own run
    /// loop, which is what flushes in production.
    fn settle(&mut self) {
        for _ in 0..10 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let _ = self.state.display_handle.flush_clients();
    }

    /// Types a string the way `flexwm msg type` does, then reports what the
    /// client made of it.
    fn type_text(&mut self, text: &str) -> Typed {
        self.state.type_text(text).expect("the text is typable");
        self.settle();
        self.run(Step::Report)
    }

    /// Presses a combination the way `flexwm msg key` does, then reports
    /// what the client made of it. A refusal is returned rather than
    /// asserted away: half these tests are about exactly which names get
    /// refused.
    fn press(&mut self, combo: &str) -> Result<Typed, String> {
        let combo: KeyCombo = combo.parse().expect("a parsable key combination");
        self.state.press(&combo)?;
        self.settle();
        Ok(self.run(Step::Report))
    }
}

impl Drop for Fixture {
    /// Closes the step channel, which is what ends [`run_client`]'s loop.
    /// Deliberately does not join while unwinding: a compositor-side panic
    /// leaves the client blocked in a roundtrip whose answer never comes,
    /// and joining there turns a failing test into a hung one.
    fn drop(&mut self) {
        drop(self.steps.take());
        if std::thread::panicking() {
            return;
        }
        if let Some(handle) = self.client.take() {
            let _ = handle.join();
        }
    }
}

/// The reported bug, end to end: `flexwm msg type "AbC xyz"` used to deliver
/// `abc xyz`.
#[test]
fn mixed_case_text_reaches_the_client_exactly_as_asked_for() {
    let mut fixture = Fixture::new();
    let typed = fixture.type_text("AbC xyz");
    assert_eq!(typed.text, "AbC xyz");
}

/// Every character class the old level-0 check got wrong. `!`, `_`, `?`,
/// `~`, `:` and `|` each used to arrive as `1`, `-`, `/`, backtick, `;` and
/// backslash -- close enough to look like a typo rather than a bug.
#[test]
fn shifted_punctuation_reaches_the_client_exactly_as_asked_for() {
    let mut fixture = Fixture::new();
    let expected = "!\"#$%&()*+:<>?@^_{|}~";
    let typed = fixture.type_text(expected);
    assert_eq!(typed.text, expected);
}

/// The other half of the fix: nothing about unshifted text changed. Worth
/// its own assertion because the obvious wrong fix -- hold Shift whenever
/// the keysym is not at level 0 of the *first* key that carries it -- would
/// break a plain shell command line while passing the tests above.
#[test]
fn unshifted_text_still_reaches_the_client_untouched() {
    let mut fixture = Fixture::new();
    let expected = "ls -la /tmp/dir; echo 'one two'";
    let typed = fixture.type_text(expected);
    assert_eq!(typed.text, expected);
}

/// A shifted character costs a real modifier key press, not a different
/// keysym: two key events for `a` (down, up) and four for `A` (Shift down,
/// key down, key up, Shift up). This is what a client sees from a human
/// typing, and what an application watching raw keys (a game, a terminal in
/// raw mode) needs to agree with the text.
#[test]
fn a_shifted_character_is_delivered_as_a_held_modifier_key() {
    let mut fixture = Fixture::new();
    let lower = fixture.type_text("a");
    assert_eq!((lower.text.as_str(), lower.keys), ("a", 2));
    let upper = fixture.type_text("A");
    assert_eq!((upper.text.as_str(), upper.keys), ("aA", 6));
}

/// `\n` is a `Return` press, not a character with a level -- the control
/// characters `keysym_for_char` special-cases have to keep working through
/// the new lookup.
#[test]
fn control_characters_still_arrive_as_their_named_keys() {
    let mut fixture = Fixture::new();
    let typed = fixture.type_text("Ab\n");
    assert_eq!(typed.text, "Ab\r", "Return decodes as a carriage return");
    // Shift + A (4), b (2), Return (2).
    assert_eq!(typed.keys, 8);
}

/// Empty input is a no-op, not an error and not a stray key.
#[test]
fn typing_nothing_sends_nothing() {
    let mut fixture = Fixture::new();
    let typed = fixture.type_text("");
    assert_eq!(
        typed,
        Typed {
            text: String::new(),
            keys: 0,
            focused: true,
        }
    );
}

/// A character the layout cannot produce is an error, and the characters
/// before it were really typed -- the documented, pre-existing behaviour of
/// this call, asserted so a future change to it is a deliberate one.
#[test]
fn an_untypable_character_errors_and_leaves_the_text_before_it_typed() {
    let mut fixture = Fixture::new();
    let error = fixture
        .state
        .type_text("Hi é")
        .expect_err("`é` is not on a US layout");
    assert!(
        error.contains('é'),
        "the error should name the character: {error}"
    );
    fixture.settle();
    let typed = fixture.run(Step::Report);
    assert_eq!(typed.text, "Hi ");
}

// -------------------------------------------------------------------------
// `press`: exactly the combination named, or a refusal
// -------------------------------------------------------------------------

/// The same bug class as the one above, at `type_text`'s neighbour:
/// `flexwm msg key exclam` used to press the `1` key with nothing held and
/// deliver `1`, because the key lookup scanned every level while `press`
/// holds only what its caller named. Measured on a real client: `exclam`,
/// `at`, `asciitilde`, `underscore`, `question`, `colon`, `bar` and
/// `braceleft` all arrived as the character *below* the one named.
#[test]
fn a_key_named_above_the_unmodified_level_is_refused_instead_of_typing_another() {
    let mut fixture = Fixture::new();
    for name in [
        "exclam",
        "at",
        "asciitilde",
        "underscore",
        "question",
        "colon",
        "bar",
        "braceleft",
        "A",
    ] {
        let error = fixture
            .press(name)
            .expect_err("this name is only above level 0 on a US layout");
        assert!(
            error.contains(name),
            "the refusal should name the key asked for: {error}"
        );
        assert!(
            error.contains("type"),
            "the refusal should point at the call that can type it: {error}"
        );
    }
    let typed = fixture.run(Step::Report);
    assert_eq!(
        (typed.text.as_str(), typed.keys),
        ("", 0),
        "a refused combination must not send any key at all"
    );
}

/// The other half: what the refusal tells the caller to write instead
/// really does deliver the character, as four key events -- the modifier's
/// own press and release around the key's.
#[test]
fn naming_the_modifier_explicitly_delivers_the_character_the_refusal_named() {
    let mut fixture = Fixture::new();
    let typed = fixture
        .press("shift+1")
        .expect("`shift+1` is pressable on a US layout");
    assert_eq!(typed.text, "!");
    assert_eq!(typed.keys, 4, "Shift down, `1` down, `1` up, Shift up");
}

/// Ordinary names are untouched by the stricter lookup -- the level-0 scan
/// finds the same keys the old one did for everything an agent actually
/// sends.
#[test]
fn an_unmodified_name_still_presses_its_own_key() {
    let mut fixture = Fixture::new();
    let typed = fixture.press("h").expect("`h` is on a US layout");
    assert_eq!((typed.text.as_str(), typed.keys), ("h", 2));
    let typed = fixture.press("Return").expect("`Return` is on a US layout");
    assert_eq!((typed.text.as_str(), typed.keys), ("h\r", 4));
}

/// A name no key carries at any level is a different failure from one
/// that's out of reach, and says so -- the fix for each is different.
#[test]
fn a_name_absent_from_the_layout_is_refused_as_absent() {
    let mut fixture = Fixture::new();
    let error = fixture
        .press("ssharp")
        .expect_err("`ssharp` is not on a US layout");
    assert!(
        error.contains("no key for `ssharp`"),
        "an absent key should say so plainly: {error}"
    );
    let error = fixture
        .press("not-a-real-key")
        .expect_err("that is not a keysym name at all");
    assert!(
        error.contains("unknown key"),
        "an unknown name should say so plainly: {error}"
    );
}

/// `KeyCombo`'s modifiers are a list, so a client that builds one by
/// concatenation can legally repeat a modifier any number of times. Each
/// distinct one is pressed once: repeats neither reach the keymap again nor
/// overrun the fixed-size buffer the resolved keys live in.
#[test]
fn a_repeated_modifier_is_resolved_and_pressed_once() {
    let mut fixture = Fixture::new();
    let mut combo = "shift+".repeat(1000);
    combo.push('1');
    let typed = fixture.press(&combo).expect("a repeated modifier is legal");
    assert_eq!(typed.text, "!");
    assert_eq!(typed.keys, 4, "one Shift press, not a thousand");
}

// -------------------------------------------------------------------------
// Which events count as the user asking for something, and whose they are
// -------------------------------------------------------------------------
//
// The ring itself is tested in `interaction.rs`; these pin the other half --
// *which* of this module's three `SERIAL_COUNTER` call sites feed it, and
// which client each recorded event is attributed to. What reads the answer is
// `activation.rs`'s focus-stealing gate, so a motion serial quietly becoming
// an interaction, or an event landing under the wrong client, would each hand
// a client a key to the keyboard it never earned.

/// Any key press or release: the key an agent injects, the key a user
/// presses and the key `nested_dispatch` forwards all come through here.
#[test]
fn a_key_press_and_its_release_are_both_recorded_for_the_focused_client() {
    let mut fixture = Fixture::new();
    let code = Keycode::new(28 + 8); // evdev `Return`, +8 for the xkb offset.
    let client = fixture.client_id.clone();

    fixture.state.key(code, KeyState::Pressed);
    let (press, press_client) = fixture
        .state
        .interaction_serials
        .latest()
        .expect("the press recorded an event");
    fixture.state.key(code, KeyState::Released);
    let (release, release_client) = fixture
        .state
        .interaction_serials
        .latest()
        .expect("the release recorded an event");

    assert_ne!(press, release, "both went out with the same serial");
    assert_eq!(press_client, client, "recorded under the wrong client");
    assert_eq!(release_client, client, "recorded under the wrong client");
    // Both, because which one a client mints an activation token from is the
    // client's own choice -- see `interaction.rs`.
    assert!(fixture.state.interaction_serials.contains(press, &client));
    assert!(fixture.state.interaction_serials.contains(release, &client));
}

/// The pointer's other half: a click is an interaction, and so is letting go
/// of it (a GTK button activates on release) -- but only when the click
/// reaches somebody.
///
/// This client's toplevel never attaches a buffer, so it has no bounding box
/// and `Space::element_under` can never find it: every click here lands on
/// bare desktop, with no pointer focus, and must therefore be recorded as
/// nothing at all rather than against whoever happens to hold the keyboard.
/// (The click that *does* reach a surface is in `activation/tests.rs`, whose
/// client paints real pixels.)
#[test]
fn a_button_that_reaches_no_surface_is_recorded_as_nothing() {
    let mut fixture = Fixture::new();
    let code = Keycode::new(28 + 8);
    fixture.state.key(code, KeyState::Pressed);
    let before = fixture.state.interaction_serials.latest();
    assert!(before.is_some(), "the keypress should have been recorded");

    fixture.state.pointer_move(10.0, 10.0);
    assert!(
        fixture
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .current_focus()
            .is_none(),
        "something is under the pointer after all; this test proves nothing"
    );
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);

    assert_eq!(
        fixture.state.interaction_serials.latest(),
        before,
        "a click nobody received was recorded anyway"
    );
}

/// The serial is evidence for the client that *received* the event and for
/// nobody else, which is what stops a client from guessing its way to one:
/// `SERIAL_COUNTER` is process-global and a client can read a live value out
/// of it for free (an `xdg_surface.configure` serial comes from the same
/// counter), so a check on the number alone would be brute-forceable.
#[test]
fn an_event_is_not_evidence_for_a_client_that_never_received_it() {
    let mut fixture = Fixture::new();
    let code = Keycode::new(28 + 8);
    let bystander = fixture.bystander_id.clone();

    fixture.state.key(code, KeyState::Pressed);
    let (press, _) = fixture
        .state
        .interaction_serials
        .latest()
        .expect("the press recorded an event");

    assert!(
        !fixture
            .state
            .interaction_serials
            .contains(press, &bystander),
        "a client that was never focused can spend the focused client's serial"
    );
}

/// An event nobody receives is evidence for nobody: with no keyboard focus
/// there is no client to attribute the keypress to, so nothing is recorded
/// rather than something being recorded against an arbitrary client.
#[test]
fn a_key_with_nothing_focused_records_nothing() {
    let mut fixture = Fixture::new();
    let code = Keycode::new(28 + 8);
    let client = fixture.client_id.clone();
    fixture.state.key(code, KeyState::Pressed);
    let before = fixture.state.interaction_serials.latest();

    // Takes the keyboard away from the only client there is.
    let keyboard = fixture.state.seat.get_keyboard().expect("a keyboard");
    let serial = SERIAL_COUNTER.next_serial();
    keyboard.set_focus(&mut fixture.state, None, serial);
    fixture.state.key(code, KeyState::Released);

    assert_eq!(
        fixture.state.interaction_serials.latest(),
        before,
        "a key nobody received was still recorded"
    );
    let (_, ring) = before.expect("the first press was recorded");
    assert_eq!(ring, client);
}

/// Motion is the one that must not count: it is continuous and passive, and
/// `refresh_pointer_focus` synthesizes it with no user involvement at all.
///
/// Asserted by bracketing the move between two recorded key events rather
/// than by predicting the motion serial: `SERIAL_COUNTER` is process-global
/// and every other test running in parallel draws from it, so the only thing
/// known about the motion's serial is that it lies strictly between these
/// two. Nothing in that whole window may be in the ring.
#[test]
fn pointer_motion_is_never_recorded_as_an_interaction() {
    let mut fixture = Fixture::new();
    let code = Keycode::new(28 + 8);
    let client = fixture.client_id.clone();

    fixture.state.key(code, KeyState::Pressed);
    fixture.state.key(code, KeyState::Released);
    let (before, _) = fixture
        .state
        .interaction_serials
        .latest()
        .expect("an event before the move");
    let before = u32::from(before);

    fixture.state.pointer_move(10.0, 10.0);

    fixture.state.key(code, KeyState::Pressed);
    let (after, _) = fixture
        .state
        .interaction_serials
        .latest()
        .expect("an event after the move");
    let after = u32::from(after);

    assert!(after > before + 1, "the move issued no serial of its own");
    let mut raw = before + 1;
    while raw < after {
        assert!(
            !fixture
                .state
                .interaction_serials
                .contains(Serial::from(raw), &client),
            "serial {raw}, issued between two key events, is in the ring; \
             pointer motion (or another passive source) is being recorded"
        );
        raw += 1;
    }
    assert!(
        fixture
            .state
            .interaction_serials
            .contains(Serial::from(after), &client)
    );
}
