//! Tests for the size hints `shell.rs` reads off a client's toplevel.
//!
//! Two layers, for two different questions:
//!
//! - [`hint_limit`]/[`clamp_hint`] are pure arithmetic and tested directly,
//!   one value under each bound, exactly at it, and one over.
//! - Whether `info_of` *uses* them -- and whether a clamped hint actually
//!   keeps the render path's `i32` arithmetic in range -- cannot be seen from
//!   a pure function. Those tests drive a real `wayland-client` toplevel
//!   through a real [`State`], the approach `dispatch/tests.rs` established,
//!   and then assert on what reached the core and on the focus-ring geometry
//!   the core's own arrangement produces.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket, which
//! nothing here connects to (the client is a socket pair) but which is created
//! either way.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use scoot_core::{Arrangement, Config, OutputId};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, DispatchError, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::*;
use crate::compositor::decorations::{Appearance, ring_rects};
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// The output every live test here hands the core, and the gap it lays out
/// with. Chosen to match the compositor's own `--tty` dev resolution rather
/// than anything the arithmetic depends on.
const OUTPUT: Rect = Rect::new(0, 0, 1600, 1000);
const GAP: i32 = 12;
/// `OUTPUT.inset(GAP)`, i.e. the cap a client's `min_size` gets clamped to.
const USABLE: Size = Size::new(1576, 976);

// -------------------------------------------------------------------------
// Pure arithmetic
// -------------------------------------------------------------------------

#[test]
fn the_limit_is_one_outputs_area_inset_by_the_gap() {
    assert_eq!(hint_limit([OUTPUT].into_iter(), GAP), USABLE);
}

#[test]
fn no_outputs_leave_no_room_for_any_hint() {
    assert_eq!(hint_limit(std::iter::empty(), GAP), Size::new(0, 0));
}

#[test]
fn a_gap_wider_than_the_output_leaves_a_zero_limit() {
    // `Rect::inset` floors at zero, so this is a limit of nothing rather
    // than a negative one -- which is what lets `clamp_hint` stay free of
    // `i32::clamp`'s panic.
    let limit = hint_limit([Rect::new(0, 0, 100, 100)].into_iter(), Config::MAX_GAP);
    assert_eq!(limit, Size::new(0, 0));
}

#[test]
fn each_axis_of_the_limit_comes_from_whichever_output_is_larger_on_it() {
    // A wide-and-short output beside a narrow-and-tall one: the bound has to
    // be no tighter than either, since nothing here knows which one a
    // brand-new toplevel will be placed on.
    let wide = Rect::new(0, 0, 1600, 600);
    let tall = Rect::new(1600, 0, 800, 1200);
    assert_eq!(
        hint_limit([wide, tall].into_iter(), 0),
        Size::new(1600, 1200)
    );
}

#[test]
fn a_hint_up_to_the_limit_passes_through_untouched() {
    for declared in [0, 1, USABLE.w - 1, USABLE.w] {
        let hint = Size::new(declared, declared.min(USABLE.h));
        assert_eq!(
            clamp_hint(hint, USABLE),
            hint,
            "a legitimate hint of {declared} was altered"
        );
    }
}

#[test]
fn a_hint_past_the_limit_is_capped_to_it() {
    for declared in [USABLE.w + 1, 100_000, i32::MAX] {
        assert_eq!(
            clamp_hint(Size::new(declared, declared), USABLE),
            USABLE,
            "a hint of {declared} was not capped"
        );
    }
}

#[test]
fn a_negative_hint_becomes_no_hint() {
    // `xdg_toplevel.set_min_size` takes raw `i32`s and the pinned Smithay rev
    // stores them unchecked, so this is reachable from any client.
    assert_eq!(clamp_hint(Size::new(-1, i32::MIN), USABLE), Size::new(0, 0));
}

#[test]
fn a_zero_limit_drops_every_hint() {
    assert_eq!(
        clamp_hint(Size::new(500, 400), Size::new(0, 0)),
        Size::new(0, 0)
    );
}

/// Per-axis, not per-output: a hint may be legitimate on one axis and absurd
/// on the other, and the capped axis must not drag the other one down.
#[test]
fn one_absurd_axis_does_not_affect_the_other() {
    assert_eq!(
        clamp_hint(Size::new(400, i32::MAX), USABLE),
        Size::new(400, USABLE.h)
    );
}

/// The ticket's chain, pinned at both ends: `hint_limit` caps to the output,
/// not to any absolute bound, so the bound that keeps 12(b)'s limit real
/// lives one layer up, in the CLI's `--width`/`--height` range check.
#[test]
fn the_limit_follows_the_output_so_only_the_flag_bound_bounds_it() {
    // A 2e9-wide area yields a ~2e9 limit that bounds nothing real. This is
    // the vacuous clamp the ticket diagnoses: reachable before the CLI
    // bound, unreachable after (anything past 65535 is refused at parse).
    assert_eq!(
        hint_limit(
            [Rect::new(0, 0, 2_000_000_000, 2_000_000_000)].into_iter(),
            GAP
        ),
        Size::new(1_999_999_976, 1_999_999_976)
    );
    // ...while the largest output the flags can still spell leaves a limit
    // every layout sum stays four orders of magnitude inside `i32` in (the
    // logical area can double the flag at the `[output] scale` floor of 0.5,
    // to 131070 a side, and the largest dimension-derived sum past that is
    // `available + gap` at ~141000 -- still ~15000x below `i32::MAX`).
    let limit = hint_limit([Rect::new(0, 0, 65535, 65535)].into_iter(), GAP);
    assert_eq!(limit, Size::new(65511, 65511));
    assert_eq!(
        clamp_hint(Size::new(i32::MAX, i32::MAX), limit),
        Size::new(65511, 65511)
    );
}

// -------------------------------------------------------------------------
// A live compositor and a live client
// -------------------------------------------------------------------------

/// The client end of one test connection: just enough of a toolkit to map a
/// toplevel and declare a minimum size.
#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
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
        // Version 1 of each is all this needs (`create_surface`,
        // `get_xdg_surface`, `set_min_size`, `set_title` are all v1), so ask
        // for exactly that and stay independent of what the compositor
        // advertises.
        if interface == wl_compositor::WlCompositor::interface().name {
            client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
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
        // Nothing in scoot pings today, but a client that ignores one is a
        // client that can be killed for it.
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

/// Maps a toplevel declaring `min` as its minimum size, and returns the live
/// connection so the caller can keep it -- and therefore the window -- alive
/// while it asserts.
fn declare_min_size(stream: UnixStream, min: (i32, i32)) -> Result<Connection, DispatchError> {
    let conn = Connection::from_socket(stream).expect("a client connection");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client)?;

    let compositor = client.compositor.clone().expect("the wl_compositor global");
    let wm_base = client.wm_base.clone().expect("the xdg_wm_base global");
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());

    toplevel.set_min_size(min.0, min.1);
    // `set_min_size` is double-buffered -- the pinned Smithay rev writes it to
    // the *pending* `SurfaceCachedState` -- so it is not the value `info_of`
    // reads (`current()`) until a commit applies it.
    surface.commit();
    queue.roundtrip(&mut client)?;

    // And a commit is not what makes the compositor re-read hints: `info_of`
    // runs from `add_window` (on `get_toplevel`, before any commit could have
    // applied anything) and from `refresh_window`, whose only callers are
    // `app_id_changed`/`title_changed`. So retitling is what publishes the
    // hint -- which is also the sequence a real toolkit produces, every
    // terminal retitling itself after it maps.
    toplevel.set_title("min-size-probe".to_string());
    queue.roundtrip(&mut client)?;
    Ok(conn)
}

/// What one live run observed on the compositor side.
struct Observed {
    /// What the core ended up holding for the window.
    info: WindowInfo,
    /// And where it decided to put it, which is what the render path (see
    /// [`ring_rects`]) then does plain `i32` arithmetic on.
    arrangement: Arrangement,
}

/// Stands up a real compositor with one output, lets one client map a
/// toplevel declaring `min`, and reports what the core made of it.
fn drive(min: (i32, i32)) -> Observed {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    let display: Display<State> = Display::new().expect("a wayland display");
    let mut state = State::new(
        &mut event_loop,
        display,
        Config {
            gap: GAP,
            ..Config::default()
        },
        Keybindings::default(),
        Appearance::default(),
        1.0,
    )
    .expect("a compositor state with a wayland socket");
    // The core needs an output for there to be a usable area to clamp
    // against. Added directly rather than through `headless::init`, which
    // would also build a renderer and a render target nothing here draws to.
    state.world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: OUTPUT,
    });

    let (server, client) = UnixStream::pair().expect("a socket pair");
    state
        .display_handle
        .insert_client(server, Arc::new(ClientState::default()))
        .expect("an inserted client");

    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    let clients = thread::spawn(move || {
        let result = declare_min_size(client, min);
        flag.store(true, Ordering::Release);
        result
    });

    // The client blocks on its roundtrips, so the compositor has to be
    // dispatched from here until it is done. The deadline only exists so a
    // regression hangs the test for ten seconds instead of forever.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !finished.load(Ordering::Acquire) && Instant::now() < deadline {
        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut state)
            .expect("a compositor dispatch");
    }
    assert!(
        finished.load(Ordering::Acquire),
        "the client thread never finished; the compositor stopped serving it"
    );
    // Held, not dropped: closing the connection would disconnect the client,
    // and the next dispatch would take its window back out of the core before
    // anything below could look at it.
    let _connection = clients
        .join()
        .expect("the client thread")
        .expect("the client's requests were all accepted");

    let windows = state.world.windows();
    let (_, info) = windows.first().expect("the toplevel reached the core");
    Observed {
        info: (*info).clone(),
        arrangement: state.world.arrange(),
    }
}

#[test]
fn a_modest_minimum_size_reaches_the_core_unchanged() {
    // Keeps the test below honest: it proves this harness really does carry a
    // client's `min_size` through to the core, so a capped result there is the
    // clamp at work and not a hint that never arrived.
    let observed = drive((400, 300));
    assert_eq!(observed.info.hints.min, Size::new(400, 300));
}

#[test]
fn an_enormous_minimum_size_is_capped_at_the_outputs_usable_area() {
    let observed = drive((i32::MAX, i32::MAX));
    assert_eq!(observed.info.hints.min, USABLE);
}

/// The consequence the clamp exists for. An unclamped `i32::MAX` minimum
/// becomes a column that wide, and two plain `i32` adds downstream overflow on
/// it -- `World::place_workspace`'s `x + width` first (verified: with the
/// clamp disabled this test panics in `arrange.rs`, before it ever reaches a
/// ring), then `ring_rects`'s `rect.w + 2 * width`. In a debug build either
/// one is a panic, i.e. the whole compositor and every client's unsaved state.
#[test]
fn an_enormous_minimum_size_cannot_overflow_the_focus_ring_arithmetic() {
    let observed = drive((i32::MAX, i32::MAX));
    let placement = observed
        .arrangement
        .placements
        .first()
        .expect("a placement for the mapped window");
    assert_eq!(
        placement.rect.w, USABLE.w,
        "the column is wider than the screen"
    );
    let rects = ring_rects(placement.rect, 3, OUTPUT);
    assert!(
        rects.left.is_some() && rects.right.is_some(),
        "a window filling the usable area should still have side rings: {rects:?}"
    );
}
