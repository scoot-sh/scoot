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

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use flexwm_core::{Event, OutputId, Rect, WindowId};
use flexwm_ipc::PointerButton;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat as ServerSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::utils::Serial;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::input::interaction;
use crate::compositor::state::ClientState;
use crate::compositor::test_support::Harness;

/// The one output every test here gives the core, so there is somewhere for
/// windows to be arranged.
const OUTPUT: Rect = Rect::new(0, 0, 1600, 1000);

/// The name [`State::new`] gives the one seat this compositor has, which is
/// how the test client picks it out of the registry.
const SEAT: &str = "flexwm";

/// A second seat [`drive`] advertises, so a token naming a seat this
/// compositor does not own can be built at all. Inert: no keyboard, no
/// pointer, and nothing in [`State`] refers to it.
const OTHER_SEAT: &str = "flexwm-test-other";

/// The side, in pixels, of the buffer each toplevel paints. Small, and far
/// smaller than the column the core lays the window out in -- so a test that
/// wants the pointer over a window has to aim at the surface's own top-left
/// corner, not at the middle of its column.
const SURFACE: i32 = 64;

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
    /// Taken from the compositor's own seat once it advertises a keyboard.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// For the one buffer this client paints: a toplevel with no buffer has
    /// no size, so nothing the pointer can be *over* (see [`solid_buffer`]).
    shm: Option<wl_shm::WlShm>,
    /// The serial of the first key press this client was *sent*, which is the
    /// only serial it can honestly mint a token from. Learned over the
    /// protocol rather than handed in from the compositor side, because
    /// "did the client that received the event get to spend it" is exactly
    /// what the gate is made of.
    key_serial: Option<u32>,
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
        } else if interface == wl_shm::WlShm::interface().name {
            client.shm = Some(registry.bind(name, version.min(1), qh, ()));
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
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_seat::Event::Name { name } => {
                if let Some(entry) = client.seats.iter_mut().find(|(bound, _)| bound == seat) {
                    entry.1 = Some(name);
                }
            }
            // Only the compositor's own seat has any capability at all (the
            // second one is inert), so this needs no name check to pick the
            // right keyboard.
            wl_seat::Event::Capabilities {
                capabilities: WEnum::Value(capabilities),
            } if capabilities.contains(wl_seat::Capability::Keyboard)
                && client.keyboard.is_none() =>
            {
                client.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            _ => {}
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
        // The serial of the first key *press* this client is given, which is
        // the one a real launcher mints its token from -- fuzzel sets its
        // stored serial in exactly this handler and calls
        // `get_activation_token` from the binding that runs off it. The
        // keymap fd and everything else about the key are irrelevant here;
        // only the serial is.
        if let wl_keyboard::Event::Key {
            serial,
            state: WEnum::Value(wl_keyboard::KeyState::Pressed),
            ..
        } = event
            && client.key_serial.is_none()
        {
            client.key_serial = Some(serial);
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
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);

/// A `SURFACE`x`SURFACE` `wl_buffer` of opaque pixels, over a real memfd --
/// the same path any toolkit takes, and the shape `cursor/tests.rs` uses.
///
/// Needed for one reason: a toplevel that never attaches a buffer has an
/// empty bounding box, so `Space::element_under` can never find it and the
/// pointer can never be over anything. Without this the click half of the
/// gate would be tested against a pointer focused on nothing, which accepts
/// and asserts nothing.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
) -> Result<wl_buffer::WlBuffer, String> {
    let stride = SURFACE * 4;
    let len = (stride * SURFACE) as usize;
    let fd = rustix::fs::memfd_create("flexwm-activation-test", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0xffu8; len])
        .map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        SURFACE,
        SURFACE,
        stride,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Ok(buffer)
}

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

/// What the client thread reports, in the order it reports it.
enum Ack {
    /// Both toplevels are up, one of them has the keyboard, and the client is
    /// now blocked waiting to be typed at.
    Mapped,
    /// The script ran to the end. Boxed because a `Run` owns a `Connection`
    /// and is far larger than the other variant.
    Done(Box<Run>),
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

/// Maps two toplevels, waits to be typed at, then -- unless `activate` says
/// otherwise -- takes a token and redeems it against the *first* one, which
/// mapping the second took focus away from.
///
/// The wait is the point, and it is what makes this the real flow rather than
/// a simulation of it: the client mints its token from the serial of a key
/// press *it was sent*, the way `fuzzel` does, instead of being handed a
/// number the compositor side picked. `mapped` tells the compositor when
/// there is a focused window to type into; the key press follows.
///
/// `spare_tokens` are created and committed but never redeemed, which is how
/// the cap test fills the table through the real protocol path.
fn map_two_and_activate_first(
    stream: UnixStream,
    activate: bool,
    spare_tokens: usize,
    claim: Claim,
    acks: Sender<Ack>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;

    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let map = |title: &str| -> Result<wl_surface::WlSurface, String> {
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_title(title.to_string());
        surface.commit();
        Ok(surface)
    };
    // Mapped in two commits, the way the protocol asks: the role-only commit
    // above, then pixels once the compositor's configure has been acked (the
    // `xdg_surface` handler above does that as the event arrives).
    let paint = |surface: &wl_surface::WlSurface| -> Result<(), String> {
        surface.attach(Some(&solid_buffer(&shm, &qh)?), 0, 0);
        surface.damage(0, 0, SURFACE, SURFACE);
        surface.commit();
        Ok(())
    };
    let first = map("first")?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    paint(&first)?;
    let second = map("second")?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    paint(&second)?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    // No xdg role, no buffer: a surface the compositor tracks as a surface
    // and as nothing else.
    let bare = compositor.create_surface(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    // Two windows are up and one of them has the keyboard, so there is
    // something for a keypress to reach. Dispatch until it arrives; the
    // compositor side's own deadline is what turns "it never comes" into a
    // failed assertion rather than a hang.
    acks.send(Ack::Mapped).map_err(|e| e.to_string())?;
    while client.key_serial.is_none() {
        queue
            .blocking_dispatch(&mut client)
            .map_err(|e| e.to_string())?;
    }
    let received = client.key_serial.ok_or("no key press reached the client")?;
    let serial = match claim {
        Claim::RealKeyPress => Some(received),
        Claim::Nothing => None,
        // Far past anything this test issues, so it cannot collide with a
        // serial that really was recorded -- what a guessed or stale number
        // looks like.
        Claim::Fabricated => Some(received.wrapping_add(1_000)),
    };

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

    acks.send(Ack::Done(Box::new(Run {
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
    })))
    .map_err(|e| e.to_string())
}

/// A live compositor with one output and one connected client. See
/// [`crate::compositor::test_support`] for everything that is not specific to
/// activation.
///
/// The client takes no steps -- its script runs start to finish on its own --
/// so the step type is `()`; everything it has to say comes back as an [`Ack`].
type Fixture = Harness<(), Ack>;

/// Stands up a real compositor with one output, runs the client script above
/// against it, and hands back both halves for the caller to assert on.
///
/// Two things happen around the client so that the gate in `token_created` is
/// exercised rather than accidentally short-circuited: a second, inert seat is
/// advertised before it connects (see [`OTHER_SEAT`]), and once its windows
/// are up a real key press and release go through the seat -- the user
/// interaction a legitimate token is minted from, delivered to the client
/// itself. `claim` decides what the client then attaches to its tokens.
fn drive(activate: bool, spare_tokens: usize, claim: Claim) -> (Fixture, Run) {
    // No backend: an output is added to the core directly rather than through
    // `headless::init`, which would also build a renderer and a render target
    // nothing here draws to.
    let mut fixture = Harness::bare(Appearance::default());
    fixture.state.world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: OUTPUT,
    });
    // Created before the client connects so it reaches the client's
    // registry, and kept alive by `seat_state`'s own list of seats rather
    // than by this handle.
    fixture
        .state
        .seat_state
        .new_wl_seat(&fixture.state.display_handle, OTHER_SEAT);
    fixture.spawn(move |stream, _steps, acks| {
        map_two_and_activate_first(stream, activate, spare_tokens, claim, acks)
    });

    // The client blocks on its roundtrips, so the compositor has to be
    // dispatched from here until it answers -- which is what `wait_for_ack`
    // does, with the deadline that turns "it never comes" into a failed
    // assertion rather than a hang.
    let Ack::Mapped = fixture.wait_for_ack(0) else {
        panic!("the client reported it was done before it reported being mapped");
    };
    // The client is up and focused, and is now waiting to be typed at. Flushed
    // explicitly afterwards because it is blocked reading rather than writing,
    // and the display source only pushes events out when the *client* writes.
    press_a_key(&mut fixture);
    let _ = fixture.state.display_handle.flush_clients();

    let Ack::Done(run) = fixture.wait_for_ack(0) else {
        panic!("the client reported being mapped twice");
    };
    (fixture, *run)
}

/// Presses and releases one key on the compositor's own seat, reporting the
/// serials the press and the release went out with, and who they went to.
///
/// The only way a test can learn a serial this compositor really issued:
/// `SERIAL_COUNTER` is process-global and shared with every other test
/// running in parallel, so the values cannot be predicted, only observed. The
/// recipient is asserted rather than returned per-event: both halves of one
/// press go to whoever holds the keyboard, and a test that found otherwise
/// would be testing a compositor that had stopped making sense.
fn press_a_key(fixture: &mut Fixture) -> (Serial, Serial, ClientId) {
    let state = &mut fixture.state;
    let before = state.interaction_serials.latest();
    let outcome = state.key(Keycode::new(RETURN), KeyState::Pressed);
    assert!(
        !outcome.intercepted,
        "a keybinding took the test keypress; it would have run an action \
         in the middle of a test that is not about keybindings"
    );
    let (press, client) = state
        .interaction_serials
        .latest()
        .expect("the press was recorded, so a client had the keyboard");
    // The same guard `click` carries, for the same reason: with no keyboard
    // focus nothing is recorded, and `latest()` then quietly returns whatever
    // came before -- which a test would go on to assert about.
    assert_ne!(
        Some((press, client.clone())),
        before,
        "the keypress recorded nothing: no client holds the keyboard"
    );
    state.key(Keycode::new(RETURN), KeyState::Released);
    let (release, released_to) = state
        .interaction_serials
        .latest()
        .expect("the release was recorded");
    assert_eq!(client, released_to, "the pair went to different clients");
    (press, release, client)
}

/// The same, with the pointer: presses and releases the left button over
/// whatever the pointer last entered.
///
/// Fails rather than reports if the click reached nobody -- a button event
/// with no pointer focus records nothing, and reading [`Recent::latest`] then
/// silently returns whatever came *before* the click, which is how the first
/// version of this test passed while testing a keypress.
fn click(fixture: &mut Fixture) -> (Serial, Serial, ClientId) {
    let state = &mut fixture.state;
    let before = state.interaction_serials.latest();
    state.pointer_button(PointerButton::Left, true);
    let (press, client) = state
        .interaction_serials
        .latest()
        .expect("the press was recorded, so the pointer was over a surface");
    assert_ne!(
        Some((press, client.clone())),
        before,
        "the click recorded nothing: the pointer is over no surface, so this \
         would have gone on to assert about the event before it"
    );
    state.pointer_button(PointerButton::Left, false);
    let (release, released_to) = state
        .interaction_serials
        .latest()
        .expect("the release was recorded");
    assert_eq!(client, released_to, "the pair went to different clients");
    (press, release, client)
}

/// A second client that connects and does nothing at all -- no surfaces, no
/// focus, never sent an input event. What a background client trying to spend
/// someone else's serial looks like.
///
/// Its own end of the socket comes back with it: dropping that would
/// disconnect it before the assertion.
fn bystander(fixture: &mut Fixture) -> (ClientId, UnixStream) {
    let (server, ours) = UnixStream::pair().expect("a socket pair");
    let id = fixture
        .state
        .display_handle
        .insert_client(server, Arc::new(ClientState::default()))
        .expect("an inserted client")
        .id();
    (id, ours)
}

/// The client's `wl_seat` resources, split into the compositor's own and the
/// other one -- resolved the same way `token_created` does it, rather than
/// by the order the client bound them in.
fn seats(fixture: &Fixture, run: &Run) -> (ServerSeat, ServerSeat) {
    let mut ours = None;
    let mut other = None;
    for &id in &run.seats {
        let resource: ServerSeat = fixture
            .client(0)
            .object_from_protocol_id(&fixture.state.display_handle, id)
            .expect("the client's seat");
        if Seat::<State>::from_resource(&resource).is_some_and(|named| named == fixture.state.seat)
        {
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

/// A token whose data claims `serial` and comes from `client`, and which is
/// otherwise as fresh as one a client just committed.
///
/// `client` is what Smithay fills in from whoever sent the request, so a test
/// that passes someone else's id here is a client naming a serial that was
/// delivered elsewhere -- not something a client can do over the protocol,
/// which is exactly why it is checked from this side.
fn token_claiming(
    serial: Option<(Serial, ServerSeat)>,
    client: Option<ClientId>,
) -> (XdgActivationToken, XdgActivationTokenData) {
    let data = XdgActivationTokenData {
        client_id: client,
        serial,
        ..XdgActivationTokenData::default()
    };
    (XdgActivationToken::from("test-token".to_string()), data)
}

/// The client's own `wl_surface` resource for `protocol_id`.
fn surface_of(fixture: &Fixture, protocol_id: u32) -> ServerSurface {
    fixture
        .client(0)
        .object_from_protocol_id(&fixture.state.display_handle, protocol_id)
        .expect("the client's surface")
}

/// The [`WindowId`] of the window whose toplevel is the client's surface
/// `protocol_id`.
fn window_of(fixture: &Fixture, protocol_id: u32) -> WindowId {
    let surface = surface_of(fixture, protocol_id);
    fixture
        .state
        .id_of(&surface)
        .expect("a window for that surface")
}

#[test]
fn the_activation_global_is_advertised() {
    // The global itself, not its effect: `foot` prints "compositor does not
    // implement XDG activation" on exactly this, and a client that cannot
    // find it never gets as far as asking for a token.
    let (_fixture, run) = drive(false, 0, Claim::RealKeyPress);
    assert!(run.activation_advertised, "xdg_activation_v1 is missing");
}

#[test]
fn mapping_a_second_window_takes_focus_from_the_first() {
    // Not a test of this module, but what makes the one below non-vacuous:
    // if focus were already on the first window, an activation that did
    // nothing at all would pass.
    let (fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&fixture, run.first_surface);
    assert_ne!(
        fixture.state.focus,
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
    let (fixture, run) = drive(true, 0, Claim::RealKeyPress);
    let first = window_of(&fixture, run.first_surface);
    assert_eq!(
        fixture.state.focus,
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
    let (fixture, run) = drive(true, 0, Claim::Nothing);
    let first = window_of(&fixture, run.first_surface);
    assert_ne!(
        fixture.state.focus,
        Some(first),
        "a token with no input serial still moved focus"
    );
}

#[test]
fn a_token_naming_a_serial_no_input_event_carried_never_moves_focus() {
    // The same client, one step cleverer: it calls `set_serial` with the
    // right seat and a made-up number rather than omitting it.
    let (fixture, run) = drive(true, 0, Claim::Fabricated);
    let first = window_of(&fixture, run.first_surface);
    assert_ne!(
        fixture.state.focus,
        Some(first),
        "a token with a fabricated serial still moved focus"
    );
}

#[test]
fn a_redeemed_token_cannot_be_spent_twice() {
    // One token, one activation: `request_activation` removes it whether or
    // not it honored it, so the same user action cannot be replayed into
    // focus later.
    let (fixture, run) = drive(true, 0, Claim::RealKeyPress);
    // Asserted first so the count below cannot pass vacuously: a token
    // refused at creation would also leave an empty table.
    let first = window_of(&fixture, run.first_surface);
    assert_eq!(
        fixture.state.focus,
        Some(first),
        "the token was never honored, so its removal proves nothing"
    );
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
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
    let (fixture, _run) = drive(false, MAX_TOKENS + 8, Claim::RealKeyPress);
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        MAX_TOKENS,
        "the token table grew past its cap"
    );
}

#[test]
fn refused_tokens_never_reach_the_table_at_all() {
    // The other half of the cap: a client looping on unserialled tokens is
    // refused before the table is touched, so it cannot evict anything or
    // occupy a slot a legitimate launcher needs.
    let (fixture, _run) = drive(false, MAX_TOKENS + 8, Claim::Nothing);
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
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
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&fixture, run.first_surface);
    let focus_before = fixture.state.focus;
    assert_ne!(focus_before, Some(first));

    let surface = surface_of(&fixture, run.first_surface);
    let (token, data) = token_aged(TOKEN_LIFETIME + Duration::from_secs(1));
    fixture.state.request_activation(token, data, surface);

    assert_eq!(
        fixture.state.focus, focus_before,
        "a token past TOKEN_LIFETIME still moved focus"
    );
}

#[test]
fn a_token_just_inside_the_lifetime_still_works() {
    // The other side of the bound, so the test above is checking an edge
    // rather than a handler that never activates anything.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&fixture, run.first_surface);

    let surface = surface_of(&fixture, run.first_surface);
    let (token, data) = token_aged(TOKEN_LIFETIME - Duration::from_secs(1));
    fixture.state.request_activation(token, data, surface);

    assert_eq!(fixture.state.focus, Some(first));
}

#[test]
fn activating_a_surface_that_is_not_a_window_changes_nothing() {
    // Layer surfaces, popups and cursor surfaces all reach this handler the
    // same way a toplevel does -- the protocol takes any `wl_surface`. None
    // of them is somewhere focus can go, and the lookup returning `None`
    // must be a no-op rather than clearing whatever had focus.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let focus_before = fixture.state.focus;
    assert!(focus_before.is_some(), "nothing had focus to begin with");

    let surface = surface_of(&fixture, run.bare_surface);
    assert!(
        fixture.state.id_of(&surface).is_none(),
        "the role-less surface is a window after all; this test proves nothing"
    );
    let (token, data) = token_aged(Duration::from_secs(0));
    fixture.state.request_activation(token, data, surface);

    assert_eq!(
        fixture.state.focus, focus_before,
        "activating a non-window surface changed focus"
    );
}

// -------------------------------------------------------------------------
// The gate, called directly
// -------------------------------------------------------------------------
//
// `token_created`'s answer is a `bool` a client can never see, so these ask
// it straight rather than inferring it from focus. The live tests above cover
// the shapes a cooperating client can actually produce; these cover the ones
// it cannot -- another seat's serial, another *client's* serial, and a serial
// that was real once and has since scrolled out of the recent history.

#[test]
fn a_token_carrying_either_half_of_a_key_press_is_accepted() {
    // Both, because which one a client mints from is its own choice, and
    // `State::press` sends the release before any client could answer the
    // press: a gate that only knew one of them would refuse every token
    // `flexwm msg key` ever leads to.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    let (press, release, typed_at) = press_a_key(&mut fixture);

    let (token, data) = token_claiming(Some((press, ours.clone())), Some(typed_at.clone()));
    assert!(
        fixture.state.token_created(token, data),
        "a token naming the press serial was refused"
    );
    let (token, data) = token_claiming(Some((release, ours)), Some(typed_at));
    assert!(
        fixture.state.token_created(token, data),
        "a token naming the release serial was refused"
    );
}

#[test]
fn a_token_carrying_either_half_of_a_click_is_accepted() {
    // The pointer's own half of the same rule: a launcher entry activated
    // with the mouse mints from a button event, and a GTK button fires on
    // the release.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    // A button goes to whatever the pointer last entered, so put it over the
    // first window's painted area first -- which is `SURFACE` pixels square
    // at the window's own origin, not the whole column the core gave it.
    fixture.state.pointer_move(30.0, 30.0);
    let (press, release, clicked_on) = click(&mut fixture);

    let (token, data) = token_claiming(Some((press, ours.clone())), Some(clicked_on.clone()));
    assert!(
        fixture.state.token_created(token, data),
        "a token naming the button press serial was refused"
    );
    let (token, data) = token_claiming(Some((release, ours)), Some(clicked_on));
    assert!(
        fixture.state.token_created(token, data),
        "a token naming the button release serial was refused"
    );
}

#[test]
fn two_tokens_from_different_recent_interactions_are_both_accepted() {
    // More than one token can be outstanding at once, each minted from a
    // different real interaction -- a launcher's token still being redeemed
    // by a cold-starting app while the user clicks something else. Only one
    // of those serials can be the *latest*; both are recent.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    fixture.state.pointer_move(30.0, 30.0);
    let (older, _, typed_at) = press_a_key(&mut fixture);
    let (newer, _, clicked_on) = click(&mut fixture);
    assert_eq!(typed_at, clicked_on, "one client, two kinds of interaction");

    let (token, data) = token_claiming(Some((older, ours.clone())), Some(typed_at.clone()));
    assert!(
        fixture.state.token_created(token, data),
        "the older of two recent interactions was refused"
    );
    let (token, data) = token_claiming(Some((newer, ours)), Some(clicked_on));
    assert!(
        fixture.state.token_created(token, data),
        "the newer of two recent interactions was refused"
    );
}

#[test]
fn a_token_that_names_no_input_event_is_refused() {
    let (mut fixture, _run) = drive(false, 0, Claim::RealKeyPress);
    let (token, data) = token_claiming(None, Some(fixture.client(0).id()));
    assert!(
        !fixture.state.token_created(token, data),
        "a token with no serial at all was accepted"
    );
}

#[test]
fn a_token_naming_a_seat_this_compositor_does_not_own_is_refused() {
    // A real serial, on a real seat -- just not the seat that issued it. One
    // seat is all flexwm has today, so this is the rule holding the line for
    // a future that has more than one rather than one being enforced daily.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (_ours, other) = seats(&fixture, &run);
    let (press, _release, typed_at) = press_a_key(&mut fixture);

    let (token, data) = token_claiming(Some((press, other)), Some(typed_at));
    assert!(
        !fixture.state.token_created(token, data),
        "a token naming another seat was accepted"
    );
}

#[test]
fn a_token_whose_serial_no_input_event_carried_is_refused() {
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    let (press, _release, typed_at) = press_a_key(&mut fixture);

    // Far enough past a real one that nothing in this test could have
    // issued it, which is what a stale serial looks like.
    let fabricated = Serial::from(u32::from(press).wrapping_add(1_000));
    let (token, data) = token_claiming(Some((fabricated, ours)), Some(typed_at));
    assert!(
        !fixture.state.token_created(token, data),
        "a token with a fabricated serial was accepted"
    );
}

#[test]
fn a_serial_delivered_to_another_client_is_refused_however_right_the_number_is() {
    // The reason the ring stores who received each event. `SERIAL_COUNTER` is
    // process-global and a client can read a live value out of it for free --
    // an `xdg_surface.configure` serial comes from the same counter -- so a
    // value-only check would be brute-forceable by a client that received no
    // input at all: a few dozen guesses around an observed value, each costing
    // nothing because a refused token posts no error. Here the number is not
    // guessed but exactly right, and it still gets nowhere.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    let (press, release, typed_at) = press_a_key(&mut fixture);
    let (guesser, _socket) = bystander(&mut fixture);
    assert_ne!(guesser, typed_at, "the bystander is the focused client");

    for (serial, which) in [(press, "press"), (release, "release")] {
        let (token, data) = token_claiming(Some((serial, ours.clone())), Some(guesser.clone()));
        assert!(
            !fixture.state.token_created(token, data),
            "a client that never received the {which} spent its serial anyway"
        );
    }
    // ... and the client that really did receive it still can, so the refusal
    // above is about the recipient and not about the serial having gone stale.
    let (token, data) = token_claiming(Some((press, ours)), Some(typed_at));
    assert!(
        fixture.state.token_created(token, data),
        "the client the key actually went to was refused too"
    );
}

#[test]
fn a_token_with_no_requesting_client_is_refused() {
    // Not reachable over the protocol -- Smithay fills `client_id` in from
    // the sender -- but "no client" can never satisfy a rule about which
    // client received the event, and defaulting it open would be the whole
    // gate.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    let (press, _release, _typed_at) = press_a_key(&mut fixture);

    let (token, data) = token_claiming(Some((press, ours)), None);
    assert!(
        !fixture.state.token_created(token, data),
        "a token with no requesting client was accepted"
    );
}

#[test]
fn a_serial_from_before_the_recent_history_is_refused() {
    // The bound on how far back "recent" goes: the ring keeps a fixed
    // number of qualifying events, and one pushed out of it is no longer
    // evidence of anything. A launcher mints and hands over its token in
    // milliseconds; this is a client that sat on a serial while the user
    // went on typing.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    let (press, _release, typed_at) = press_a_key(&mut fixture);
    // Two events each, so this is twice the history's own length.
    for _ in 0..interaction::CAPACITY {
        press_a_key(&mut fixture);
    }

    let (token, data) = token_claiming(Some((press, ours)), Some(typed_at));
    assert!(
        !fixture.state.token_created(token, data),
        "a serial older than the whole recent history was still accepted"
    );
}

#[test]
fn an_interaction_from_hours_ago_is_refused_even_with_nothing_since() {
    // The ring only rotates when *newer* qualifying input arrives, and an
    // agent-driven session produces none: `Request::Action` goes straight to
    // `State::act`, and motion and scroll never qualify. Without the age
    // bound this morning's click would still be spendable tonight, against
    // whatever the agent had arranged since.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    let (press, _release, typed_at) = press_a_key(&mut fixture);
    fixture
        .state
        .interaction_serials
        .backdate(Duration::from_secs(8 * 3600));

    let (token, data) = token_claiming(Some((press, ours)), Some(typed_at));
    assert!(
        !fixture.state.token_created(token, data),
        "an interaction from hours ago was still good enough"
    );
}

#[test]
fn a_session_that_has_seen_no_input_at_all_refuses_everything() {
    // The fixture.state a compositor starts in, and the one a client racing the
    // session's first keypress would find: nothing has been interacted
    // with, so no serial can be recent -- not even one this compositor
    // really did issue, to the very client asking.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let (ours, _other) = seats(&fixture, &run);
    let (press, _release, typed_at) = press_a_key(&mut fixture);
    fixture.state.interaction_serials = interaction::Recent::default();

    let (token, data) = token_claiming(Some((press, ours)), Some(typed_at));
    assert!(
        !fixture.state.token_created(token, data),
        "a serial was accepted against an empty interaction history"
    );
}
