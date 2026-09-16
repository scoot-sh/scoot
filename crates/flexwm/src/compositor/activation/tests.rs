//! Tests for `xdg-activation-v1`.
//!
//! Two layers, for two different questions:
//!
//! - Whether a token actually moves focus cannot be seen from a pure
//!   function: it needs a real client with real toplevels, a real token
//!   round trip through `xdg_activation_token_v1.done`, and the core's own
//!   focus afterwards. Those tests drive a `wayland-client` connection
//!   through a real [`State`], the approach `shell/tests.rs` established.
//!   They also cover the serial gate end to end -- the client really does
//!   call `set_serial`, or really does not -- because that is the one rule
//!   an outside client can exercise from the protocol alone.
//! - The rest of the policy is reached by calling the handler directly:
//!   [`TOKEN_LIFETIME`] needs a token already too old to have been created
//!   by a live client inside a test, [`MAX_TOKENS`] needs the table filled
//!   without waiting for 64 protocol round trips, and the individual serial
//!   refusals (no serial, another seat's serial, a serial that has scrolled
//!   out of the recent history) need token data a cooperating client would
//!   never build.
//!
//! Like every other live-[`State`] test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

use flexwm_core::{Config, Event, OutputId, Rect, WindowId};
use flexwm_ipc::PointerButton;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat as ServerSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::reexports::wayland_server::{Client, Display};
use smithay::utils::Serial;
use wayland_client::protocol::{wl_compositor, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::input::interaction;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// The one output every test here gives the core, so there is somewhere for
/// windows to be arranged.
const OUTPUT: Rect = Rect::new(0, 0, 1600, 1000);

/// How long a live test will dispatch before deciding the compositor stopped
/// serving its client. Generous: a debug build under a VM.
const PATIENCE: Duration = Duration::from_secs(10);

/// The name [`State::new`] gives the one seat this compositor has, which is
/// how the test client picks it out of the registry.
const SEAT: &str = "flexwm";

/// A second seat [`drive`] advertises, so a token naming a seat this
/// compositor does not own can be built at all. Inert: no keyboard, no
/// pointer, and nothing in [`State`] refers to it.
const OTHER_SEAT: &str = "flexwm-test-other";

/// evdev's `KEY_ENTER`, plus the offset every xkb keymap is built with.
///
/// Pressed bare, so it cannot match a default keybinding -- every one of
/// those holds Super (see `Keybindings::default`) -- which
/// [`press_a_key`] asserts rather than assumes.
const RETURN: u32 = 28 + 8;

// -------------------------------------------------------------------------
// The live half
// -------------------------------------------------------------------------

/// The client end of one test connection: enough of a toolkit to map
/// toplevels and to redeem an activation token against one of them.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    /// `None` here is what `the_activation_global_is_advertised` fails on.
    activation: Option<xdg_activation_v1::XdgActivationV1>,
    /// Every `wl_seat` the compositor advertises, each with the name it sent
    /// back. A real launcher has exactly one to choose from; this client has
    /// two (see [`OTHER_SEAT`]) and has to name the right one, which is what
    /// makes the compositor-side seat check testable at all.
    seats: Vec<(wl_seat::WlSeat, Option<String>)>,
    /// The token string the compositor sent back, once `commit` produced one.
    /// Sent whether or not the compositor accepted the token -- a refusal is
    /// invisible from here, which is the protocol's own shape (see
    /// `token_created`).
    token: Option<String>,
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
        // Version 1 of each is all this needs, so ask for exactly that and
        // stay independent of what the compositor advertises.
        if interface == wl_compositor::WlCompositor::interface().name {
            client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_activation_v1::XdgActivationV1::interface().name {
            client.activation = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_seat::WlSeat::interface().name {
            // Version 2, unlike everything else here, for one reason: the
            // `name` event arrived in 2, and it is how this client tells the
            // compositor's own seat from the extra one without depending on
            // the order globals happen to be advertised in.
            client
                .seats
                .push((registry.bind(name, version.min(2), qh, ()), None));
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
        _: &QueueHandle<Self>,
    ) {
        // Capabilities are ignored on purpose: nothing here takes a keyboard
        // or a pointer from the seat, it only names it in `set_serial`.
        if let wl_seat::Event::Name { name } = event
            && let Some(entry) = client.seats.iter_mut().find(|(bound, _)| bound == seat)
        {
            entry.1 = Some(name);
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            client.token = Some(token);
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
wayland_client::delegate_noop!(TestClient: ignore xdg_activation_v1::XdgActivationV1);

/// What one live run reports back to the compositor side.
struct Run {
    /// Kept so the client's surfaces -- and therefore the compositor's
    /// windows -- outlive the assertions.
    #[allow(dead_code)]
    connection: Connection,
    /// The protocol id of the first toplevel's `wl_surface`, so the
    /// compositor side can name the window that should have been activated.
    first_surface: u32,
    /// The protocol id of a plain `wl_surface` with no role at all -- what a
    /// layer surface, a popup or a cursor surface looks like to this
    /// handler, which takes any `wl_surface` the protocol hands it.
    bare_surface: u32,
    /// The protocol ids of every `wl_seat` the client bound, in no
    /// particular order: the compositor side resolves each one to tell which
    /// is its own (see [`seats`]).
    seats: Vec<u32>,
    /// Whether the client found `xdg_activation_v1` in the registry.
    activation_advertised: bool,
}

/// What a client attaches to the tokens it mints.
#[derive(Clone, Copy, Debug)]
enum Claim {
    /// The serial of a real key press the compositor just issued -- what a
    /// launcher minting a token inside its own key handler has, and the only
    /// thing this compositor accepts.
    RealKeyPress,
    /// `set_serial` never called. Legal in the protocol (it is a request,
    /// not a constructor argument) and exactly what a background client
    /// trying to take focus has to offer.
    Nothing,
    /// A number no key or button event ever carried: a stale serial, or a
    /// guessed one.
    Fabricated,
}

/// Maps two toplevels, then -- unless `activate` says otherwise -- takes a
/// token and redeems it against the *first* one, which mapping the second
/// took focus away from.
///
/// Every token minted here carries `serial` (on the compositor's own seat)
/// unless it is `None`, which is the client never calling `set_serial` at
/// all.
///
/// `spare_tokens` are created and committed but never redeemed, which is how
/// the cap test fills the table through the real protocol path.
fn map_two_and_activate_first(
    stream: UnixStream,
    activate: bool,
    spare_tokens: usize,
    serial: Option<u32>,
) -> Result<Run, String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;

    let map = |title: &str| -> Result<wl_surface::WlSurface, String> {
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_title(title.to_string());
        surface.commit();
        Ok(surface)
    };
    let first = map("first")?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let _second = map("second")?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    // No xdg role, no buffer: a surface the compositor tracks as a surface
    // and as nothing else.
    let bare = compositor.create_surface(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let activation = client.activation.clone().ok_or("no xdg_activation_v1")?;
    // A serial is only meaningful against the seat that issued it, so name
    // the compositor's own rather than whichever seat came first.
    let seat = client
        .seats
        .iter()
        .find(|(_, name)| name.as_deref() == Some(SEAT))
        .map(|(seat, _)| seat.clone())
        .ok_or_else(|| format!("no wl_seat named `{SEAT}`"))?;
    let mint = |app_id: &str| {
        let token = activation.get_activation_token(&qh, ());
        if let Some(serial) = serial {
            token.set_serial(serial, &seat);
        }
        token.set_app_id(app_id.to_string());
        token.commit();
    };

    for _ in 0..spare_tokens {
        mint("spare");
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    }

    if activate {
        mint("flexwm-activation-test");
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        // Sent even for a token the compositor refused: the client cannot
        // tell, so this is what a refused activation really looks like on
        // the wire.
        let token = client.token.clone().ok_or("the compositor sent no token")?;
        activation.activate(token, &first);
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    }

    Ok(Run {
        connection: conn,
        first_surface: first.id().protocol_id(),
        bare_surface: bare.id().protocol_id(),
        seats: client
            .seats
            .iter()
            .map(|(seat, _)| seat.id().protocol_id())
            .collect(),
        // The client would have failed above with "no xdg_activation_v1" if
        // it were missing, so reaching here means it was found. Recorded
        // explicitly so the dedicated test below asserts on the registry
        // rather than on a side effect.
        activation_advertised: true,
    })
}

/// Stands up a real compositor with one output, runs the client script above
/// against it, and hands back both halves for the caller to assert on.
///
/// Two things happen before the client is let in, both so the serial gate in
/// `token_created` is exercised rather than accidentally short-circuited: a
/// second, inert seat is advertised (see [`OTHER_SEAT`]), and one real key
/// press and release go through the seat, which is the user interaction a
/// legitimate token is minted from. `claim` decides what the client then
/// attaches to its tokens.
fn drive(
    activate: bool,
    spare_tokens: usize,
    claim: Claim,
) -> (EventLoop<'static, State>, State, Client, Run) {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
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
    // Added directly rather than through `headless::init`, which would also
    // build a renderer and a render target nothing here draws to.
    state.world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: OUTPUT,
    });
    // Created before the client connects so it reaches the client's
    // registry, and kept alive by `seat_state`'s own list of seats rather
    // than by this handle.
    state
        .seat_state
        .new_wl_seat(&state.display_handle, OTHER_SEAT);
    let (press, _release) = press_a_key(&mut state);
    let claimed = match claim {
        Claim::RealKeyPress => Some(u32::from(press)),
        Claim::Nothing => None,
        // Far past anything this test issues, so it cannot collide with a
        // serial that really was recorded: the ring holds two entries here,
        // both from the press above.
        Claim::Fabricated => Some(u32::from(press).wrapping_add(1_000)),
    };

    let (server, client_end) = UnixStream::pair().expect("a socket pair");
    let client: Client = state
        .display_handle
        .insert_client(server, Arc::new(ClientState::default()))
        .expect("an inserted client");

    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    let thread = thread::spawn(move || {
        let result = map_two_and_activate_first(client_end, activate, spare_tokens, claimed);
        flag.store(true, Ordering::Release);
        result
    });

    // The client blocks on its roundtrips, so the compositor has to be
    // dispatched from here until it is done. The deadline only exists so a
    // regression fails in seconds instead of hanging forever.
    let deadline = Instant::now() + PATIENCE;
    while !finished.load(Ordering::Acquire) && Instant::now() < deadline {
        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut state)
            .expect("a compositor dispatch");
    }
    assert!(
        finished.load(Ordering::Acquire),
        "the client thread never finished; the compositor stopped serving it"
    );
    let run = thread
        .join()
        .expect("the client thread did not panic")
        .expect("the client script ran");
    (event_loop, state, client, run)
}

/// Presses and releases one key on the compositor's own seat, reporting the
/// serials the press and the release went out with.
///
/// The only way a test can learn a serial this compositor really issued:
/// `SERIAL_COUNTER` is process-global and shared with every other test
/// running in parallel, so the values cannot be predicted, only observed.
fn press_a_key(state: &mut State) -> (Serial, Serial) {
    let outcome = state.key(Keycode::new(RETURN), KeyState::Pressed);
    assert!(
        !outcome.intercepted,
        "a keybinding took the seeding keypress; it would have run an action \
         in the middle of a test that is not about keybindings"
    );
    let press = state
        .interaction_serials
        .latest()
        .expect("the press recorded a serial");
    state.key(Keycode::new(RETURN), KeyState::Released);
    let release = state
        .interaction_serials
        .latest()
        .expect("the release recorded a serial");
    (press, release)
}

/// The same, with the pointer: presses and releases the left button.
fn click(state: &mut State) -> (Serial, Serial) {
    state.pointer_button(PointerButton::Left, true);
    let press = state
        .interaction_serials
        .latest()
        .expect("the press recorded a serial");
    state.pointer_button(PointerButton::Left, false);
    let release = state
        .interaction_serials
        .latest()
        .expect("the release recorded a serial");
    (press, release)
}

/// The client's `wl_seat` resources, split into the compositor's own and the
/// other one -- resolved the same way `token_created` does it, rather than
/// by the order the client bound them in.
fn seats(state: &State, client: &Client, run: &Run) -> (ServerSeat, ServerSeat) {
    let mut ours = None;
    let mut other = None;
    for &id in &run.seats {
        let resource: ServerSeat = client
            .object_from_protocol_id(&state.display_handle, id)
            .expect("the client's seat");
        if Seat::<State>::from_resource(&resource).is_some_and(|named| named == state.seat) {
            ours = Some(resource);
        } else {
            other = Some(resource);
        }
    }
    (
        ours.expect("the compositor's own seat"),
        other.expect("the second seat"),
    )
}

/// A token whose data claims `serial`, and which is otherwise as fresh as
/// one a client just committed.
fn token_claiming(
    serial: Option<(Serial, ServerSeat)>,
) -> (XdgActivationToken, XdgActivationTokenData) {
    let data = XdgActivationTokenData {
        serial,
        ..XdgActivationTokenData::default()
    };
    (XdgActivationToken::from("test-token".to_string()), data)
}

/// The [`WindowId`] of the window whose toplevel is the client's surface
/// `protocol_id`.
fn window_of(state: &State, client: &Client, protocol_id: u32) -> WindowId {
    let surface: ServerSurface = client
        .object_from_protocol_id(&state.display_handle, protocol_id)
        .expect("the client's surface");
    state.id_of(&surface).expect("a window for that surface")
}

#[test]
fn the_activation_global_is_advertised() {
    // The global itself, not its effect: `foot` prints "compositor does not
    // implement XDG activation" on exactly this, and a client that cannot
    // find it never gets as far as asking for a token.
    let (_loop, _state, _client, run) = drive(false, 0, Claim::RealKeyPress);
    assert!(run.activation_advertised, "xdg_activation_v1 is missing");
}

#[test]
fn mapping_a_second_window_takes_focus_from_the_first() {
    // Not a test of this module, but what makes the one below non-vacuous:
    // if focus were already on the first window, an activation that did
    // nothing at all would pass.
    let (_loop, state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&state, &client, run.first_surface);
    assert_ne!(
        state.focus,
        Some(first),
        "the first window still has focus, so activating it would prove nothing"
    );
}

#[test]
fn a_fresh_token_activates_the_window_it_names() {
    // The whole legitimate flow, end to end and through the real protocol:
    // a key press happens, the client mints a token naming that press's
    // serial on this compositor's seat, and redeems it. This is what
    // `fuzzel` handing `foot` an `XDG_ACTIVATION_TOKEN` does.
    let (_loop, state, client, run) = drive(true, 0, Claim::RealKeyPress);
    let first = window_of(&state, &client, run.first_surface);
    assert_eq!(
        state.focus,
        Some(first),
        "the activated window did not get focus"
    );
}

#[test]
fn a_token_that_names_no_input_event_never_moves_focus() {
    // The focus-steal case this gate exists for, driven by a real client:
    // `set_serial` is optional in the protocol, so a background client can
    // simply not call it. The token is milliseconds old and the table is
    // empty, so neither other bound refuses it.
    let (_loop, state, client, run) = drive(true, 0, Claim::Nothing);
    let first = window_of(&state, &client, run.first_surface);
    assert_ne!(
        state.focus,
        Some(first),
        "a token with no input serial still moved focus"
    );
}

#[test]
fn a_token_naming_a_serial_no_input_event_carried_never_moves_focus() {
    // The same client, one step cleverer: it calls `set_serial` with the
    // right seat and a made-up number rather than omitting it.
    let (_loop, state, client, run) = drive(true, 0, Claim::Fabricated);
    let first = window_of(&state, &client, run.first_surface);
    assert_ne!(
        state.focus,
        Some(first),
        "a token with a fabricated serial still moved focus"
    );
}

#[test]
fn a_redeemed_token_cannot_be_spent_twice() {
    // One token, one activation: `request_activation` removes it whether or
    // not it honored it, so the same user action cannot be replayed into
    // focus later.
    let (_loop, state, client, run) = drive(true, 0, Claim::RealKeyPress);
    // Asserted first so the count below cannot pass vacuously: a token
    // refused at creation would also leave an empty table.
    let first = window_of(&state, &client, run.first_surface);
    assert_eq!(
        state.focus,
        Some(first),
        "the token was never honored, so its removal proves nothing"
    );
    assert_eq!(
        state.xdg_activation.tokens().count(),
        0,
        "the redeemed token is still outstanding"
    );
}

#[test]
fn outstanding_tokens_are_bounded() {
    // `get_activation_token` is unauthenticated and unlimited, and nothing
    // upstream prunes what it hands out -- see the module doc. The client
    // here commits well past the cap through the real protocol path, every
    // one of them properly serialled, so they are all otherwise acceptable;
    // the table must stop growing rather than track every one.
    let (_loop, state, _client, _run) = drive(false, MAX_TOKENS + 8, Claim::RealKeyPress);
    assert_eq!(
        state.xdg_activation.tokens().count(),
        MAX_TOKENS,
        "the token table grew past its cap"
    );
}

#[test]
fn refused_tokens_never_reach_the_table_at_all() {
    // The other half of the cap: a client looping on unserialled tokens is
    // refused before the table is touched, so it cannot evict anything or
    // occupy a slot a legitimate launcher needs.
    let (_loop, state, _client, _run) = drive(false, MAX_TOKENS + 8, Claim::Nothing);
    assert_eq!(
        state.xdg_activation.tokens().count(),
        0,
        "refused tokens were tracked anyway"
    );
}

// -------------------------------------------------------------------------
// The policy bounds, called directly
// -------------------------------------------------------------------------

/// A token whose data claims it was created `age` ago.
fn token_aged(age: Duration) -> (XdgActivationToken, XdgActivationTokenData) {
    let data = XdgActivationTokenData {
        timestamp: Instant::now() - age,
        ..XdgActivationTokenData::default()
    };
    (XdgActivationToken::from("test-token".to_string()), data)
}

#[test]
fn an_expired_token_does_not_move_focus() {
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&state, &client, run.first_surface);
    let focus_before = state.focus;
    assert_ne!(focus_before, Some(first));

    let surface: ServerSurface = client
        .object_from_protocol_id(&state.display_handle, run.first_surface)
        .expect("the client's surface");
    let (token, data) = token_aged(TOKEN_LIFETIME + Duration::from_secs(1));
    state.request_activation(token, data, surface);

    assert_eq!(
        state.focus, focus_before,
        "a token past TOKEN_LIFETIME still moved focus"
    );
}

#[test]
fn a_token_just_inside_the_lifetime_still_works() {
    // The other side of the bound, so the test above is checking an edge
    // rather than a handler that never activates anything.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&state, &client, run.first_surface);

    let surface: ServerSurface = client
        .object_from_protocol_id(&state.display_handle, run.first_surface)
        .expect("the client's surface");
    let (token, data) = token_aged(TOKEN_LIFETIME - Duration::from_secs(1));
    state.request_activation(token, data, surface);

    assert_eq!(state.focus, Some(first));
}

#[test]
fn activating_a_surface_that_is_not_a_window_changes_nothing() {
    // Layer surfaces, popups and cursor surfaces all reach this handler the
    // same way a toplevel does -- the protocol takes any `wl_surface`. None
    // of them is somewhere focus can go, and the lookup returning `None`
    // must be a no-op rather than clearing whatever had focus.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let focus_before = state.focus;
    assert!(focus_before.is_some(), "nothing had focus to begin with");

    let surface: ServerSurface = client
        .object_from_protocol_id(&state.display_handle, run.bare_surface)
        .expect("the client's role-less surface");
    assert!(
        state.id_of(&surface).is_none(),
        "the role-less surface is a window after all; this test proves nothing"
    );
    let (token, data) = token_aged(Duration::from_secs(0));
    state.request_activation(token, data, surface);

    assert_eq!(
        state.focus, focus_before,
        "activating a non-window surface changed focus"
    );
}

// -------------------------------------------------------------------------
// The serial gate, called directly
// -------------------------------------------------------------------------
//
// `token_created`'s answer is a `bool` a client can never see, so these ask
// it straight rather than inferring it from focus. The live tests above
// cover the two shapes a cooperating client can actually produce; these
// cover the ones it cannot -- another seat's serial, and a serial that was
// real once and has since scrolled out of the recent history.

#[test]
fn a_token_carrying_either_half_of_a_key_press_is_accepted() {
    // Both, because which one a client mints from is its own choice, and
    // `State::press` sends the release before any client could answer the
    // press: a gate that only knew one of them would refuse every token
    // `flexwm msg key` ever leads to.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&state, &client, &run);
    let (press, release) = press_a_key(&mut state);

    let (token, data) = token_claiming(Some((press, ours.clone())));
    assert!(
        state.token_created(token, data),
        "a token naming the press serial was refused"
    );
    let (token, data) = token_claiming(Some((release, ours)));
    assert!(
        state.token_created(token, data),
        "a token naming the release serial was refused"
    );
}

#[test]
fn a_token_carrying_either_half_of_a_click_is_accepted() {
    // The pointer's own half of the same rule: a launcher entry activated
    // with the mouse mints from a button event, and a GTK button fires on
    // the release.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&state, &client, &run);
    let (press, release) = click(&mut state);

    let (token, data) = token_claiming(Some((press, ours.clone())));
    assert!(
        state.token_created(token, data),
        "a token naming the button press serial was refused"
    );
    let (token, data) = token_claiming(Some((release, ours)));
    assert!(
        state.token_created(token, data),
        "a token naming the button release serial was refused"
    );
}

#[test]
fn two_tokens_from_different_recent_interactions_are_both_accepted() {
    // More than one token can be outstanding at once, each minted from a
    // different real interaction -- a launcher's token still being redeemed
    // by a cold-starting app while the user clicks something else. Only one
    // of those serials can be the *latest*; both are recent.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&state, &client, &run);
    let (older, _) = press_a_key(&mut state);
    let (newer, _) = click(&mut state);

    let (token, data) = token_claiming(Some((older, ours.clone())));
    assert!(
        state.token_created(token, data),
        "the older of two recent interactions was refused"
    );
    let (token, data) = token_claiming(Some((newer, ours)));
    assert!(
        state.token_created(token, data),
        "the newer of two recent interactions was refused"
    );
}

#[test]
fn a_token_that_names_no_input_event_is_refused() {
    let (_loop, mut state, _client, _run) = drive(false, 0, Claim::RealKeyPress);
    let (token, data) = token_claiming(None);
    assert!(
        !state.token_created(token, data),
        "a token with no serial at all was accepted"
    );
}

#[test]
fn a_token_naming_a_seat_this_compositor_does_not_own_is_refused() {
    // A real serial, on a real seat -- just not the seat that issued it. One
    // seat is all flexwm has today, so this is the rule holding the line for
    // a future that has more than one rather than one being enforced daily.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let (_ours, other) = seats(&state, &client, &run);
    let (press, _release) = press_a_key(&mut state);

    let (token, data) = token_claiming(Some((press, other)));
    assert!(
        !state.token_created(token, data),
        "a token naming another seat was accepted"
    );
}

#[test]
fn a_token_whose_serial_no_input_event_carried_is_refused() {
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&state, &client, &run);
    let (press, _release) = press_a_key(&mut state);

    // Far enough past a real one that nothing in this test could have
    // issued it, which is what a guessed or stale serial looks like.
    let fabricated = Serial::from(u32::from(press).wrapping_add(1_000));
    let (token, data) = token_claiming(Some((fabricated, ours)));
    assert!(
        !state.token_created(token, data),
        "a token with a fabricated serial was accepted"
    );
}

#[test]
fn a_serial_from_before_the_recent_history_is_refused() {
    // The bound on how far back "recent" goes: the ring keeps a fixed
    // number of qualifying serials, and one pushed out of it is no longer
    // evidence of anything. A launcher mints and hands over its token in
    // milliseconds; this is a client that sat on a serial while the user
    // went on typing.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&state, &client, &run);
    let (press, _release) = press_a_key(&mut state);
    // Two events each, so this is twice the history's own length.
    for _ in 0..interaction::CAPACITY {
        press_a_key(&mut state);
    }

    let (token, data) = token_claiming(Some((press, ours)));
    assert!(
        !state.token_created(token, data),
        "a serial older than the whole recent history was still accepted"
    );
}

#[test]
fn a_session_that_has_seen_no_input_at_all_refuses_everything() {
    // The state a compositor starts in, and the one a client racing the
    // session's first keypress would find: nothing has been interacted
    // with, so no serial can be recent -- not even one this compositor
    // really did issue.
    let (_loop, mut state, client, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&state, &client, &run);
    let (press, _release) = press_a_key(&mut state);
    state.interaction_serials = interaction::Recent::default();

    let (token, data) = token_claiming(Some((press, ours)));
    assert!(
        !state.token_created(token, data),
        "a serial was accepted against an empty interaction history"
    );
}
