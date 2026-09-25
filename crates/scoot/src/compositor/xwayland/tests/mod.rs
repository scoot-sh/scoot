//! Tests for opt-in XWayland: the server (Phase 1), window mapping (Phase 2)
//! and the focus gate (Phase 3) -- see `super`, and
//! `docs/backlog/protocols/xwayland-support.md`.
//!
//! Two halves, split by what the machine provides:
//!
//! - **Hermetic** (every machine, both feature flavours): the opt-in
//!   resolution, the `DISPLAY` value, the `State::spawn` plumbing (a live
//!   child process, no X server), and a real Wayland client's registry
//!   listing proving neither XWayland global leaks to ordinary clients.
//! - **Live** (only where the `Xwayland` binary is on `PATH`, and only in
//!   `xwayland`-feature builds): server start, `READY`, `DISPLAY` in the
//!   state, a mid-session server kill the session survives, and -- in
//!   [`mapping`], [`focus`] and [`lock`] -- real X clients mapping windows
//!   into the layout, asking for focus, and meeting the lock. Where the binary is
//!   absent these take the fallback branch instead -- asserting the loud
//!   Wayland-only outcome, the way `dmabuf`'s tests assert their own
//!   absence branch -- and print the same `skipped --` line, never a
//!   silent pass.
//!
//! The live half needs a writable `$XDG_RUNTIME_DIR` for the same reason
//! every other compositor test does (`State::new` binds a real Wayland
//! listening socket).

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

use super::{DISPLAY_ENV, display_value, resolve};
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;
#[cfg(feature = "xwayland")]
use crate::compositor::test_support::capture_logs;

#[test]
fn resolve_is_an_or_with_off_as_the_agreement() {
    // The flag can only say yes, so either side saying yes means yes, and
    // only both silent means off (see `resolve`).
    assert!(!resolve(false, false));
    assert!(resolve(true, false));
    assert!(resolve(false, true));
    assert!(resolve(true, true));
}

/// X strings are cut at their first NUL before they go anywhere a Wayland
/// C string could carry them (see `manage::x11_text`); one without a NUL is
/// untouched.
#[cfg(feature = "xwayland")]
#[test]
fn x11_text_is_cut_at_the_first_nul() {
    use super::manage::x11_text;

    assert_eq!(x11_text("plain".to_owned()), "plain");
    assert_eq!(x11_text("evil\0title".to_owned()), "evil");
    assert_eq!(x11_text("\0".to_owned()), "");
    assert_eq!(x11_text(String::new()), "");
}

#[test]
fn display_value_is_the_local_colon_form() {
    // What X clients expect for a local server (the spike measured
    // `DISPLAY=:6`); the abstract socket makes it reachable.
    assert_eq!(display_value(6), ":6");
    assert_eq!(display_value(0), ":0");
}

/// Polls `marker` until the spawned child writes it, handing back what it
/// wrote. (`test_support::wait_for_marker` removes the file on success,
/// which is exactly what the `DISPLAY` test then wants to read.)
fn read_marker(marker: &std::path::Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match std::fs::read_to_string(marker) {
            Ok(text) => {
                let _ = std::fs::remove_file(marker);
                return text;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => panic!("the spawned child never wrote its marker: {error}"),
        }
    }
}

/// A live compositor with no backend: the server-only tests need no framebuffer,
/// and [`Harness::bare`] is what the wire-only suites use.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn unstarted() -> Self {
        Harness::bare(Appearance::default())
    }
}

/// What the test asks its registry-listing client to do.
#[derive(Debug)]
enum Step {
    /// Round-trip again and report the globals seen now (a global created
    /// after connect is announced to existing registries).
    Relist,
}

/// What the listing client answers.
#[derive(Debug)]
enum Ack {
    Globals(Vec<String>),
}

/// A registry-listing client: binds the registry, sweeps the globals it is
/// shown, reports the first sweep out-of-band (so the test can assert on a
/// session that never takes a step), then answers `Relist` steps.
struct Lister {
    globals: Vec<String>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Lister {
    fn event(
        lister: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { interface, .. } = event {
            lister.globals.push(interface);
        }
    }
}

fn lister(
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<Ack>,
    initial: Sender<Vec<String>>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut lister = Lister {
        globals: Vec::new(),
    };
    // Globals arrive on bind; three round trips is the settled shape (one
    // is not provably enough -- the compositor answers across dispatches).
    for _ in 0..3 {
        queue.roundtrip(&mut lister).map_err(|e| e.to_string())?;
    }
    initial
        .send(lister.globals.clone())
        .map_err(|e| e.to_string())?;
    while let Ok(step) = steps.recv() {
        match step {
            Step::Relist => {
                for _ in 0..3 {
                    queue.roundtrip(&mut lister).map_err(|e| e.to_string())?;
                }
                acks.send(Ack::Globals(lister.globals.clone()))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Neither XWayland global is ever visible to an ordinary client -- not
/// before the server starts (the shell global's `can_view` admits only
/// XWayland's own client; the grab manager does not exist yet), and -- in
/// the live test below -- not after either.
#[test]
fn ordinary_clients_see_no_xwayland_globals() {
    let mut fixture = Fixture::unstarted();
    let (initial_tx, initial_rx) = channel();
    fixture.spawn(|stream, steps, acks| lister(stream, steps, acks, initial_tx));
    let initial: Vec<String> = fixture.wait_for(0, &initial_rx, "the initial registry listing");
    for global in ["xwayland_shell_v1", "zwp_xwayland_keyboard_grab_manager_v1"] {
        assert!(
            !initial.iter().any(|seen| seen == global),
            "an ordinary client sees {global} on a session that never started XWayland: {initial:?}"
        );
    }
    // A re-list reports the same registry: the step protocol itself is
    // exercised in both feature flavours (keeping `Relist`/`Globals` live
    // everywhere), and a global appearing only after connect would show.
    let Ack::Globals(relisted) = fixture.run_on(0, Step::Relist);
    assert_eq!(
        relisted, initial,
        "the registry changed between list and re-list with nothing happening"
    );
}

/// `DISPLAY` reaches spawned children while the server is believed live,
/// and children inherit (clobber nothing) when it is not.
///
/// Hermetic: `xdisplay` is set directly -- no server binary needed -- and
/// the child is `sh`, which exists everywhere.
#[test]
fn display_reaches_spawned_children_only_while_live() {
    // Live: the child sees exactly the session's display.
    let mut fixture = Fixture::unstarted();
    fixture.state.xdisplay = Some(9);
    let marker = crate::compositor::test_support::marker_path("xwayland-display");
    let marker_arg = marker.to_string_lossy().into_owned();
    assert!(
        fixture.state.spawn(&[
            "sh".to_owned(),
            "-c".to_owned(),
            format!("echo ${DISPLAY_ENV} > {marker_arg}")
        ]),
        "a `sh` spawn should start"
    );
    let seen = read_marker(&marker);
    assert_eq!(
        seen.trim(),
        ":9",
        "the spawned child did not see the session's DISPLAY"
    );

    // Not live: the child inherits whatever the process has -- which is
    // what "no clobber" means, pinned without touching the process-global
    // environment (a read, never a write, so this stays safe under `cargo
    // test`'s shared process too).
    let mut fixture = Fixture::unstarted();
    assert!(fixture.state.xdisplay.is_none());
    let marker = crate::compositor::test_support::marker_path("xwayland-noclobber");
    let marker_arg = marker.to_string_lossy().into_owned();
    assert!(
        fixture.state.spawn(&[
            "sh".to_owned(),
            "-c".to_owned(),
            format!("echo ${{{DISPLAY_ENV}:-unset}} > {marker_arg}")
        ]),
        "a `sh` spawn should start"
    );
    let seen = read_marker(&marker);
    let expected = std::env::var(DISPLAY_ENV).unwrap_or_else(|_| "unset".to_owned());
    assert_eq!(
        seen.trim(),
        expected,
        "a spawn with no live X server did not inherit DISPLAY untouched"
    );
}

// -- Live half (xwayland-feature builds with the binary on PATH) ----------

/// Whether the `Xwayland` binary resolves on this machine's `PATH` -- a
/// walk, never an exec, so probing has no side effects.
#[cfg(feature = "xwayland")]
fn xwayland_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|dir| dir.join("Xwayland").is_file() || dir.join("Xwayland.exe").is_file())
    })
}

/// How long to wait for the server to become ready, or for X events to
/// arrive. Generous: a debug build exec'ing a real server on a VM.
#[cfg(feature = "xwayland")]
const XWAYLAND_PATIENCE: Duration = Duration::from_secs(20);

/// Dispatches until `ready` holds, or fails loudly on the deadline.
#[cfg(feature = "xwayland")]
fn wait_until<S, A>(
    fixture: &mut Harness<S, A>,
    what: &str,
    mut ready: impl FnMut(&Harness<S, A>) -> bool,
) {
    let deadline = Instant::now() + XWAYLAND_PATIENCE;
    while !ready(fixture) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; the X server never got there"
        );
        fixture.settle();
    }
}

/// The server starts where the binary exists -- and where it does not, the
/// attempt fails loudly as a returned `Err` (never a panic) while the
/// session keeps serving. One test, two branches: the machine's `PATH`
/// picks which behaviour is correct here, and both are asserted, so this
/// is coverage either way, not a skip.
#[cfg(feature = "xwayland")]
#[test]
fn server_starts_or_falls_back_loudly() {
    use super::StartError;

    if !xwayland_on_path() {
        eprintln!(
            "server_starts_or_falls_back_loudly: skipped live half -- no Xwayland binary on PATH"
        );
        let mut fixture = Fixture::unstarted();
        let handle = fixture.state.loop_handle.clone();
        match super::start(handle, &mut fixture.state) {
            Err(StartError::Spawn(error)) => {
                let message = error.to_string();
                assert!(
                    message.to_lowercase().contains("not found")
                        || message.to_lowercase().contains("no such file"),
                    "the spawn failure should name the missing binary: {message}"
                );
                assert!(fixture.state.xdisplay.is_none());
                assert!(fixture.state.xwm.is_none());
            }
            Err(error @ StartError::Insert(_)) => {
                panic!("a missing binary must fail as Spawn, not as a loop error: {error}")
            }
            Ok(display) => {
                panic!("starting XWayland with no binary on PATH unexpectedly worked: :{display}")
            }
        }
        // The session underneath is unharmed: it still spawns and still
        // dispatches (the Wayland-only half of the fallback contract).
        assert!(fixture.state.spawn(&["true".to_owned()]));
        fixture.settle();
        return;
    }
    let mut fixture = Fixture::unstarted();
    let handle = fixture.state.loop_handle.clone();
    let display = super::start(handle, &mut fixture.state).expect("XWayland should start");
    // The number is known synchronously, before READY -- the first
    // spawned child already gates on it.
    assert_eq!(fixture.state.xdisplay, Some(display));
    wait_until(&mut fixture, "XWayland READY", |fixture| {
        fixture.state.xwm.is_some()
    });
    // The READY number agrees with the synchronous one (one lock, one
    // number -- a rapid restart landing elsewhere would show here).
    assert_eq!(fixture.state.xdisplay, Some(display));
    assert!(fixture.state.xwayland_grab.is_some());
    // ... and the session underneath is a working compositor, not just a
    // process that did not crash: it still spawns.
    assert!(fixture.state.spawn(&["true".to_owned()]));
}

/// Neither XWayland global becomes visible to an ordinary client once a
/// live server has started, either: the shell global's `can_view` admits
/// only XWayland's own client, and the grab manager the same (both verified
/// against the pinned Smithay source). The listing client predates the
/// start, so this also proves new globals are (not) announced to existing
/// registries correctly. (Phase 1's `no_x_window_enters_the_core` asserted
/// this beside the mapping boundary; the boundary itself is gone -- X
/// windows map now, see [`mapping`] -- and this half stays.)
#[cfg(feature = "xwayland")]
#[test]
fn the_globals_stay_invisible_on_a_live_server() {
    if !xwayland_on_path() {
        eprintln!(
            "the_globals_stay_invisible_on_a_live_server: skipped -- no Xwayland binary on PATH"
        );
        return;
    }
    let mut fixture = Fixture::unstarted();
    let (initial_tx, initial_rx) = channel();
    fixture.spawn(|stream, steps, acks| lister(stream, steps, acks, initial_tx));
    let _: Vec<String> = fixture.wait_for(0, &initial_rx, "the initial registry listing");
    let handle = fixture.state.loop_handle.clone();
    super::start(handle, &mut fixture.state).expect("XWayland should start");
    wait_until(&mut fixture, "XWayland READY", |fixture| {
        fixture.state.xwm.is_some()
    });
    let Ack::Globals(post) = fixture.run_on(0, Step::Relist);
    for global in ["xwayland_shell_v1", "zwp_xwayland_keyboard_grab_manager_v1"] {
        assert!(
            !post.iter().any(|seen| seen == global),
            "an ordinary client sees {global} on a live XWayland session: {post:?}"
        );
    }
}

/// The X server keeps running under session lock: locking changes
/// nothing about the server itself (it must -- lock is not logout). What
/// the lock does to X *windows* -- blanking them, refusing them input -- is
/// [`lock`]'s. Uses the shared [`Locker`] client: the lock is real, not a
/// flag.
#[cfg(feature = "xwayland")]
#[test]
fn the_server_survives_session_lock() {
    use crate::compositor::test_support::locker;

    if !xwayland_on_path() {
        eprintln!("the_server_survives_session_lock: skipped -- no Xwayland binary on PATH");
        return;
    }
    let mut fixture: Harness<(), ()> = Harness::headless(Appearance::default(), 200);
    let handle = fixture.state.loop_handle.clone();
    let display = super::start(handle, &mut fixture.state).expect("XWayland should start");
    wait_until(&mut fixture, "XWayland READY", |fixture| {
        fixture.state.xwm.is_some()
    });

    fixture.spawn(locker);
    fixture.run(());
    assert!(
        fixture.state.session_lock.is_locked(),
        "the test locker should hold the session locked"
    );
    // Still there, still believed live, still serving the loop.
    assert!(fixture.state.xwm.is_some());
    assert_eq!(fixture.state.xdisplay, Some(display));
    assert!(fixture.state.spawn(&["true".to_owned()]));
    fixture.settle();
}

/// A mid-session server death is loud and the session survives it -- then
/// a restart comes back on a (possibly new) display with no stale-socket
/// wedge.
///
/// The kill goes by display number (a bracketed `pkill -f` pattern naming
/// `Xwayland :N`), which names exactly the server this test started: the
/// lock scan guarantees no two live servers share a number.
#[cfg(feature = "xwayland")]
#[test]
fn a_dead_server_is_loud_and_the_session_survives() {
    if !xwayland_on_path() {
        eprintln!(
            "a_dead_server_is_loud_and_the_session_survives: skipped -- no Xwayland binary on PATH"
        );
        return;
    }
    let status = std::process::Command::new("pkill")
        .arg("--version")
        .output();
    if status.is_err() {
        eprintln!(
            "a_dead_server_is_loud_and_the_session_survives: skipped -- no pkill on PATH to drive the kill half"
        );
        return;
    }
    // The whole session inside one capture, not just the kill: the XWM's
    // span is made at `start` and entered on every X event, and a span made
    // outside a capture panics the registry when entered inside one under
    // `cargo test`'s shared process (see `capture_logs`).
    let ((), logs) = capture_logs(|| {
        let mut fixture = Fixture::unstarted();
        let handle = fixture.state.loop_handle.clone();
        let display = super::start(handle, &mut fixture.state).expect("XWayland should start");
        wait_until(&mut fixture, "XWayland READY", |fixture| {
            fixture.state.xwm.is_some()
        });
        // A managed window and an override-redirect one, so the death has
        // something to sweep: a dead server sends no unmap for either.
        let x = x11::XClient::connect(display);
        let managed = x.map(&x11::Props::new(0x00ff_0000));
        let mut overlay = x11::Props::new(0x0000_00ff);
        overlay.override_redirect = true;
        let overlay = x.map(&overlay);
        x11::eventually(
            &mut fixture,
            "both windows reaching the compositor",
            |fixture| {
                live::id_of_xid(&fixture.state, managed).is_some()
                    && fixture
                        .state
                        .x11_unmanaged
                        .iter()
                        .any(|known| known.window_id() == overlay)
            },
        );

        // The bracket keeps `pkill -f` from matching this very command:
        // the pattern text contains `[X]wayland`, which the regex does
        // not match, while the server's `Xwayland :N` does. (Learned the
        // hard way: an un-bracketed pattern suicided the invoking shell
        // during manual verification.)
        let killed = std::process::Command::new("pkill")
            .args(["-f", &format!("[X]wayland :{display}( |$)")])
            .status()
            .expect("pkill runs");
        assert!(killed.success(), "pkill should find the test's server");
        // The death surfaces on dispatch: either the XWayland source's
        // Error or the XWM's `disconnected`. Both fire within a few
        // dispatches of the kill -- the drain below is bounded (not a
        // probe: the death paths deliberately change no state the loop
        // could watch, see `xwayland/mod.rs`'s staleness note), and the
        // assertion after reads the captured logs.
        for _ in 0..200 {
            fixture.settle();
        }
        // The session underneath still works: it spawns, and dispatches
        // fine around the corpse.
        assert!(fixture.state.spawn(&["true".to_owned()]));
        fixture.settle();
        // Swept: no empty column, no stale taskbar entry, nothing drawn or
        // hit-tested from the dead server.
        assert!(
            fixture.state.windows.is_empty(),
            "a dead server's window stayed in the layout: {:?}",
            fixture.state.windows.keys()
        );
        assert!(
            fixture.state.x11_unmanaged.is_empty(),
            "a dead server's override-redirect window stayed on the draw list"
        );
        drop(x);

        // And a restart comes back: a fresh server, a fresh READY, no wedge
        // from the killed one's sockets (the lock scan moves on where the
        // dead one's locks linger).
        let handle = fixture.state.loop_handle.clone();
        let display = super::start(handle, &mut fixture.state).expect("a restart should start");
        wait_until(&mut fixture, "XWayland READY again", |fixture| {
            fixture.state.xwm.is_some() && fixture.state.xdisplay == Some(display)
        });
    });
    assert!(
        logs.contains("continuing Wayland-only") || logs.contains("connection lost"),
        "the server died but the session never logged it loudly: {logs}"
    );
}

/// A window manager that cannot attach to a server that just became ready
/// -- the server's end gone between `READY` and `start_wm` -- leaves the
/// session Wayland-only with `DISPLAY` withdrawn: the WM-attach-failure arm
/// (`docs/backlog/resolved/xwayland-phase1-wm-failure-pin-done.md`).
///
/// Not the ticket's recipe, which does not hold: it had a rival X client
/// claim `SubstructureRedirect` on the root before the first dispatch, but
/// XWayland accepts *no* X client until the window manager owns `WM_S0`
/// (Smithay's `start_wm`, "No X11 clients are accepted before this"), so
/// the rival's connect blocks until the very attach it was meant to break
/// -- measured: the rival's `x11rb::connect` never returned, and nextest
/// killed the test at its 120s timeout. (And `start_wm` does not check its
/// own `ChangeWindowAttributes`, so a rival that somehow got in first would
/// not fail it either.) The failure `start_wm` does report is its
/// connection failing, which is what a server dying right after `READY`
/// looks like -- so that is what this drives, deterministically: the
/// server is spawned the way `start` spawns it, `READY` is recorded instead
/// of acted on, the recorded socket is shut down, and it is handed to
/// [`super::attach_window_manager`] -- the function `start`'s own callback
/// calls.
#[cfg(feature = "xwayland")]
#[test]
fn a_window_manager_that_cannot_attach_withdraws_the_display() {
    use std::cell::RefCell;
    use std::net::Shutdown;
    use std::process::Stdio;
    use std::rc::Rc;

    use smithay::xwayland::{XWayland, XWaylandEvent};

    if !xwayland_on_path() {
        eprintln!(
            "a_window_manager_that_cannot_attach_withdraws_the_display: skipped -- no Xwayland binary on PATH"
        );
        return;
    }
    let mut fixture = Fixture::unstarted();
    let display_handle = fixture.state.display_handle.clone();
    let (xwayland, client) = XWayland::spawn(
        &display_handle,
        None::<u32>,
        std::iter::empty::<(String, String)>(),
        std::iter::empty::<String>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    )
    .expect("XWayland should spawn");
    let display = xwayland.display_number();
    // What `start` stores synchronously, so the arm has something to
    // withdraw.
    fixture.state.xdisplay = Some(display);
    let ready: Rc<RefCell<Option<std::os::unix::net::UnixStream>>> = Rc::default();
    let seen = ready.clone();
    fixture
        .state
        .loop_handle
        .insert_source(xwayland, move |event, _, _| {
            if let XWaylandEvent::Ready { x11_socket, .. } = event {
                *seen.borrow_mut() = Some(x11_socket);
            }
        })
        .expect("a readiness source");
    wait_until(
        &mut fixture,
        "XWayland READY (recorded, not attached)",
        |_| ready.borrow().is_some(),
    );

    let x11_socket = ready
        .borrow_mut()
        .take()
        .expect("the recorded READY socket");
    x11_socket
        .shutdown(Shutdown::Both)
        .expect("the window manager's socket shuts down");
    let loop_handle = fixture.state.loop_handle.clone();
    let ((), logs) = capture_logs(|| {
        super::attach_window_manager(
            &mut fixture.state,
            loop_handle,
            &display_handle,
            x11_socket,
            client,
            display,
        );
    });
    assert!(
        fixture.state.xwm.is_none(),
        "a window manager attached over a dead connection"
    );
    assert_eq!(
        fixture.state.xdisplay, None,
        "DISPLAY was not withdrawn after the window manager failed to attach"
    );
    assert!(
        logs.contains("withdrawing DISPLAY"),
        "the failed attach was not logged loudly: {logs}"
    );
    // Wayland-only, and still serving.
    assert!(fixture.state.spawn(&["true".to_owned()]));
    fixture.settle();
}

#[cfg(feature = "xwayland")]
mod bench;
#[cfg(feature = "xwayland")]
mod focus;
#[cfg(feature = "xwayland")]
mod live;
#[cfg(feature = "xwayland")]
mod lock;
#[cfg(feature = "xwayland")]
mod mapping;
#[cfg(feature = "xwayland")]
mod peer;
#[cfg(all(feature = "xwayland", feature = "gpu-scanout"))]
mod scanout;
#[cfg(feature = "xwayland")]
mod x11;
