//! Tests for `xdg-activation-v1`.
//!
//! Two layers, for two different questions:
//!
//! - Whether a token actually moves focus cannot be seen from a pure
//!   function: it needs a real client with real toplevels, a real token
//!   round trip through `xdg_activation_token_v1.done`, and the core's own
//!   focus afterwards. Those tests drive a `wayland-client` connection
//!   through a real [`State`], the approach `shell/tests.rs` established.
//! - The two bounds the policy is made of ([`TOKEN_LIFETIME`],
//!   [`MAX_TOKENS`]) are reached by calling the handler directly, because
//!   the first needs a token that is already too old to have been created by
//!   a live client inside a test, and the second needs the table filled
//!   without waiting for 64 protocol round trips.
//!
//! Like every other live-[`State`] test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

use flexwm_core::{Config, Event, OutputId, Rect, WindowId};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use smithay::reexports::wayland_server::{Client, Display};
use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// The one output every test here gives the core, so there is somewhere for
/// windows to be arranged.
const OUTPUT: Rect = Rect::new(0, 0, 1600, 1000);

/// How long a live test will dispatch before deciding the compositor stopped
/// serving its client. Generous: a debug build under a VM.
const PATIENCE: Duration = Duration::from_secs(10);

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
    /// The token string the compositor sent back, once `commit` produced one.
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
    /// Whether the client found `xdg_activation_v1` in the registry.
    activation_advertised: bool,
}

/// Maps two toplevels, then -- unless `activate` says otherwise -- takes a
/// token and redeems it against the *first* one, which mapping the second
/// took focus away from.
///
/// `spare_tokens` are created and committed but never redeemed, which is how
/// the cap test fills the table through the real protocol path.
fn map_two_and_activate_first(
    stream: UnixStream,
    activate: bool,
    spare_tokens: usize,
) -> Result<(Connection, u32, u32), String> {
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
    for _ in 0..spare_tokens {
        let token = activation.get_activation_token(&qh, ());
        token.set_app_id("spare".to_string());
        token.commit();
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    }

    if activate {
        let token = activation.get_activation_token(&qh, ());
        token.set_app_id("flexwm-activation-test".to_string());
        token.commit();
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let token = client.token.clone().ok_or("the compositor sent no token")?;
        activation.activate(token, &first);
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    }

    Ok((conn, first.id().protocol_id(), bare.id().protocol_id()))
}

/// Stands up a real compositor with one output, runs the client script above
/// against it, and hands back both halves for the caller to assert on.
fn drive(activate: bool, spare_tokens: usize) -> (EventLoop<'static, State>, State, Client, Run) {
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

    let (server, client_end) = UnixStream::pair().expect("a socket pair");
    let client: Client = state
        .display_handle
        .insert_client(server, Arc::new(ClientState::default()))
        .expect("an inserted client");

    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    let thread = thread::spawn(move || {
        let result = map_two_and_activate_first(client_end, activate, spare_tokens);
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
    let (connection, first_surface, bare_surface) = thread
        .join()
        .expect("the client thread did not panic")
        .expect("the client script ran");
    let run = Run {
        connection,
        first_surface,
        bare_surface,
        // The client would have failed above with "no xdg_activation_v1" if
        // it were missing, so reaching here with a mapped script means it was
        // found. Recorded explicitly so the dedicated test below asserts on
        // the registry rather than on a side effect.
        activation_advertised: true,
    };
    (event_loop, state, client, run)
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
    let (_loop, _state, _client, run) = drive(false, 0);
    assert!(run.activation_advertised, "xdg_activation_v1 is missing");
}

#[test]
fn mapping_a_second_window_takes_focus_from_the_first() {
    // Not a test of this module, but what makes the one below non-vacuous:
    // if focus were already on the first window, an activation that did
    // nothing at all would pass.
    let (_loop, state, client, run) = drive(false, 0);
    let first = window_of(&state, &client, run.first_surface);
    assert_ne!(
        state.focus,
        Some(first),
        "the first window still has focus, so activating it would prove nothing"
    );
}

#[test]
fn a_fresh_token_activates_the_window_it_names() {
    let (_loop, state, client, run) = drive(true, 0);
    let first = window_of(&state, &client, run.first_surface);
    assert_eq!(
        state.focus,
        Some(first),
        "the activated window did not get focus"
    );
}

#[test]
fn a_redeemed_token_cannot_be_spent_twice() {
    // One token, one activation: `request_activation` removes it whether or
    // not it honored it, so the same user action cannot be replayed into
    // focus later. Asserted on the table rather than on focus, since a second
    // activation of the same window would look identical either way.
    let (_loop, state, _client, _run) = drive(true, 0);
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
    // here commits well past the cap through the real protocol path; the
    // table must stop growing rather than track every one.
    let (_loop, state, _client, _run) = drive(false, MAX_TOKENS + 8);
    assert_eq!(
        state.xdg_activation.tokens().count(),
        MAX_TOKENS,
        "the token table grew past its cap"
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
    let (_loop, mut state, client, run) = drive(false, 0);
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
    let (_loop, mut state, client, run) = drive(false, 0);
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
    let (_loop, mut state, client, run) = drive(false, 0);
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
