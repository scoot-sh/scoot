//! A keypress under an IME keyboard grab is still the focused window's interaction.
//!
//! The decision this pins: while an input method holds the keyboard
//! (`zwp_input_method_v2.grab_keyboard`), `State::key` keeps recording the
//! press against the focused window's client -- the window the user is typing
//! into, which is exactly who the activation gate means to credit -- rather
//! than recording nothing (option (a) in
//! `docs/backlog/resolved/interaction-serial-ime-grab-done.md`) or crediting the
//! IME (option (b)). The divergence the ticket names is real -- the pinned
//! Smithay rev's `InputMethodKeyboardGrab::input` sends the key to the IME's
//! own grab object and never touches the seat's focus, so the focused client
//! receives nothing -- but the outcome is the intended one, and the test
//! below fails if recording is ever skipped under a grab.
//!
//! Two clients, the way the real setup works: client 0 maps the windows (this
//! suite's own script, untouched), while a second one is the input method --
//! an IME is always somebody else's client. That is what makes the client
//! assertions non-vacuous: crediting the grab's client (option (b)) would
//! record under a *different* id than the focused window's, and recording
//! nothing (option (a)) would leave the ring where the pre-grab press left
//! it.
//!
//! The grab is a real one, not a flag set by hand: the IME client binds
//! `zwp_input_method_manager_v2` and calls `grab_keyboard` through the
//! protocol, and the test asserts `is_grabbed()` on the seat plus that both
//! halves of the keypress actually reached the IME's grab object -- while the
//! serial they carried is spendable by the focused window's client, and by
//! nobody else, for an activation token.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat as ServerSeat;
use smithay::utils::Serial;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2, zwp_input_method_manager_v2, zwp_input_method_v2,
};

use super::*;
use crate::compositor::test_support::wait_for;

/// The IME end: binds the input-method manager, takes the keyboard grab the
/// way fcitx5 or ibus would, and counts the key events diverted to it.
#[derive(Default)]
struct ImeClient {
    manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    /// Every `wl_seat` the compositor advertises with the name it sent back,
    /// so this picks the compositor's own seat rather than the inert extra
    /// one [`drive`] advertises.
    seats: Vec<(wl_seat::WlSeat, Option<String>)>,
    /// `key` events seen on the grab object, presses and releases alike.
    grab_keys: u32,
}

impl Dispatch<wl_registry::WlRegistry, ()> for ImeClient {
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
        if interface == zwp_input_method_manager_v2::ZwpInputMethodManagerV2::interface().name {
            client.manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_seat::WlSeat::interface().name {
            // Version 2 for the `name` event, the way this suite's window
            // client binds its own seats.
            client
                .seats
                .push((registry.bind(name, version.min(2), qh, ()), None));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for ImeClient {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Name { name } = event
            && let Some(entry) = client.seats.iter_mut().find(|(bound, _)| bound == seat)
        {
            entry.1 = Some(name);
        }
    }
}

impl Dispatch<zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2, ()> for ImeClient {
    fn event(
        client: &mut Self,
        _: &zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
        event: zwp_input_method_keyboard_grab_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Counted, not decoded: the keymap fd and modifiers are irrelevant
        // here; what matters is that the keypress reached the grab instead
        // of the focused window.
        if let zwp_input_method_keyboard_grab_v2::Event::Key { .. } = event {
            client.grab_keys += 1;
        }
    }
}

wayland_client::delegate_noop!(ImeClient: ignore zwp_input_method_manager_v2::ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(ImeClient: ignore zwp_input_method_v2::ZwpInputMethodV2);

/// Grabs the seat's keyboard through the real protocol path, reports it, and
/// parks holding the grab -- returning early would release it -- answering
/// each step with how many diverted keys have arrived so far.
fn run_ime(stream: UnixStream, steps: Receiver<()>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = ImeClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let manager = client
        .manager
        .clone()
        .ok_or("no zwp_input_method_manager_v2 -- the global is missing")?;
    // The `name` events for the just-bound seats arrive a round trip or two
    // after the binds themselves, so wait for the compositor's own seat
    // rather than assuming the first round trip above already delivered it.
    let seat = wait_for(&mut queue, &mut client, "the compositor's seat", |client| {
        client
            .seats
            .iter()
            .find(|(_, name)| name.as_deref() == Some(SEAT))
            .map(|(seat, _)| seat.clone())
    })?;
    let method = manager.get_input_method(&seat, &qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    // Held so the grab stays active: dropping the object releases it, which
    // is exactly what must not happen while the keypress is recorded.
    let _grab = method.grab_keyboard(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::ImeGrabbed).map_err(|e| e.to_string())?;

    while steps.recv().is_ok() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(Ack::ImeKeys(client.grab_keys))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and two connected clients,
/// scripted as [`Harness`] describes. Client 0 maps the windows; client 1 is
/// the input method holding a real keyboard grab.
type Fixture = Harness<(), Ack>;

/// Two windows mapped by client 0, a real IME keyboard grab held by client 1,
/// then one key press and release through the input path. Returns the
/// compositor, the press serial and release serial, and who each half was
/// recorded for -- plus the seat the serials are meaningful against.
fn drive_grabbed() -> (Fixture, Run, Serial, Serial, ClientId, ClientId, ServerSeat) {
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    fixture.spawn(run_ime);

    let Ack::ImeGrabbed = fixture.wait_for_ack(1) else {
        panic!("the IME client never took its keyboard grab");
    };
    // The ack only proves the *client* finished its own round trip, not that
    // the compositor has dispatched the grab request yet.
    fixture.settle();
    assert!(
        fixture
            .state
            .seat
            .get_keyboard()
            .expect("a keyboard")
            .is_grabbed(),
        "the IME's grab_keyboard never took effect on the seat"
    );

    let window = fixture.client(0).id();
    let ime = fixture.client(1).id();
    assert_ne!(window, ime, "the IME is the window client after all");

    // Through `State::key`, the path every real and injected keypress takes:
    // the filter still runs, the key is still delivered -- to the grab -- and
    // the recording must still name the focused window.
    let (press, release, recorded_for) = press_a_key(&mut fixture);
    assert_eq!(
        recorded_for, window,
        "a keypress under an IME grab was not recorded for the focused window's client"
    );
    // Pushed out explicitly: the window client is blocked reading, the IME
    // answers on the next step, and the display source only flushes when a
    // client writes.
    let _ = fixture.state.display_handle.flush_clients();

    let (ours, _other) = seats(&fixture, &run);
    (fixture, run, press, release, window, ime, ours)
}

#[test]
fn a_key_under_an_ime_grab_mints_a_token_for_the_focused_window_and_nothing_else() {
    // The decided semantics, pinned: the focused window -- the one the user
    // is typing into, receiving these keystrokes as composed text -- can
    // spend this keypress for an activation token, while the IME that
    // actually received it cannot. With the option-(a) early return (skip
    // recording while grabbed) the `press_a_key` inside `drive_grabbed`
    // already fails, because the ring never moves; the assertions below say
    // what the recording that did happen means.
    let (mut fixture, _run, press, release, window, ime, ours) = drive_grabbed();

    assert!(
        fixture.state.interaction_serials.contains(press, &window),
        "the press under the grab is not spendable by the focused window"
    );
    assert!(
        fixture.state.interaction_serials.contains(release, &window),
        "the release under the grab is not spendable by the focused window"
    );
    assert!(
        !fixture.state.interaction_serials.contains(press, &ime),
        "the IME can spend a keypress the gate credits to the focused window"
    );

    let (token, data) = token_claiming(Some((press, ours.clone())), Some(window));
    assert!(
        fixture.state.token_created(token, data),
        "a token minted from the grabbed keypress was refused"
    );
    let (token, data) = token_claiming(Some((press, ours)), Some(ime));
    assert!(
        !fixture.state.token_created(token, data),
        "the IME minted a token from a keypress delivered to it but credited elsewhere"
    );

    // ... while both halves really did reach the IME and nobody else: the
    // grab is diverting delivery, only the recording stays with the window.
    let Ack::ImeKeys(keys) = fixture.run_on(1, ()) else {
        panic!("the IME client never reported its diverted keys");
    };
    assert_eq!(
        keys, 2,
        "the keypress under test did not reach the IME's grab object as a press and a release"
    );
}

#[test]
fn a_token_minted_under_an_ime_grab_still_activates_on_redemption() {
    // Creation and redemption together: the token the focused window mints
    // from the grabbed keypress is honored when redeemed against its first
    // window, moving focus there. `request_activation` checks no serial
    // itself -- the gate lives at creation -- so this is the creation check
    // above doing its job all the way to a focus move.
    let (mut fixture, run, press, _release, window, _ime, ours) = drive_grabbed();
    let first = window_of(&fixture, run.first_surface);
    assert_ne!(
        fixture.state.focus,
        Some(first),
        "the first window already had focus, so activating it would prove nothing"
    );

    let (token, data) = token_claiming(Some((press, ours)), Some(window));
    assert!(
        fixture.state.token_created(token.clone(), data.clone()),
        "a token minted from the grabbed keypress was refused"
    );
    let surface = surface_of(&fixture, run.first_surface);
    fixture.state.request_activation(token, data, surface);

    assert_eq!(
        fixture.state.focus,
        Some(first),
        "redeeming a token minted under the grab did not move focus"
    );
}
