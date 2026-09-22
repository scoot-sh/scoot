//! Tests for the opt-in XWayland skeleton (see `super`, and
//! `docs/backlog/protocols/xwayland-support.md` Phase 1).
//!
//! Two halves, split by what the machine provides:
//!
//! - **Hermetic** (every machine, both feature flavours): the opt-in
//!   resolution, the `DISPLAY` value, the `State::spawn` plumbing (a live
//!   child process, no X server), and a real Wayland client's registry
//!   listing proving neither XWayland global leaks to ordinary clients.
//! - **Live** (only where the `Xwayland` binary is on `PATH`, and only in
//!   `xwayland`-feature builds): server start, `READY`, `DISPLAY` in the
//!   state, the Phase-1 mapping boundary against a real X client, and a
//!   mid-session server kill the session survives. Where the binary is
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

#[test]
fn resolve_is_an_or_with_off_as_the_agreement() {
    // The flag can only say yes, so either side saying yes means yes, and
    // only both silent means off (see `resolve`).
    assert!(!resolve(false, false));
    assert!(resolve(true, false));
    assert!(resolve(false, true));
    assert!(resolve(true, true));
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

/// A live compositor with no backend: the skeleton needs no framebuffer,
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

/// Runs `f` with compositor logs captured, handing back what it returned
/// plus everything logged on this thread while it ran.
///
/// Scoped (`with_default`), not global: every dispatch this test drives
/// runs on this thread, which is where `READY`, the XWM callbacks and the
/// death signals all fire -- and a scoped subscriber cannot collide with
/// another test's under `cargo test` the way a second global default
/// would.
#[cfg(feature = "xwayland")]
fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("the log buffer")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer = Buffer::default();
    let factory = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || factory.clone())
        .with_ansi(false)
        // INFO is the compositor's own production default (see
        // `init_logging`), but these tests assert on `debug!` handler
        // lines -- the refusals are the designed answer, not an anomaly,
        // so they log below the production floor.
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let result = tracing::subscriber::with_default(subscriber, f);
    let logs = String::from_utf8(buffer.0.lock().expect("the log buffer").clone())
        .expect("compositor logs are UTF-8");
    (result, logs)
}

/// Whether `window` is anywhere under `root` within `depth` levels: the
/// reparenting XWM moves managed windows into frames, so the root's direct
/// children never name them.
#[cfg(feature = "xwayland")]
fn tree_contains(
    conn: &impl x11rb::connection::Connection,
    root: x11rb::protocol::xproto::Window,
    window: x11rb::protocol::xproto::Window,
    depth: u8,
) -> bool {
    use x11rb::protocol::xproto::ConnectionExt as _;

    let Ok(tree) = conn.query_tree(root).map(|cookie| cookie.reply()) else {
        return false;
    };
    let Ok(tree) = tree else {
        return false;
    };
    tree.children.contains(&window)
        || (depth > 0
            && tree
                .children
                .iter()
                .any(|child| tree_contains(conn, *child, window, depth - 1)))
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

/// No X window enters the core: a real X client creates and maps windows
/// against the live server while the layout stays empty.
///
/// Non-vacuous by construction: the client proves X-side (its windows
/// exist in the server's tree, the override-redirect one viewable -- the
/// self-mapped path the XWM cannot prohibit) and the captured log proves
/// the requests reached this compositor's handlers (the map refusal, the
/// override-redirect notification). Both halves have to hold for the
/// assertions below to mean anything.
#[cfg(feature = "xwayland")]
#[test]
fn no_x_window_enters_the_core() {
    use x11rb::COPY_DEPTH_FROM_PARENT;
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::*;

    if !xwayland_on_path() {
        eprintln!("no_x_window_enters_the_core: skipped -- no Xwayland binary on PATH");
        return;
    }
    let mut fixture = Fixture::unstarted();
    // A registry-listing client from before the start, for the
    // still-invisible assertion at the end.
    let (initial_tx, initial_rx) = channel();
    fixture.spawn(|stream, steps, acks| lister(stream, steps, acks, initial_tx));
    let _: Vec<String> = fixture.wait_for(0, &initial_rx, "the initial registry listing");
    let handle = fixture.state.loop_handle.clone();
    let display = super::start(handle, &mut fixture.state).expect("XWayland should start");
    wait_until(&mut fixture, "XWayland READY", |fixture| {
        fixture.state.xwm.is_some()
    });

    let name = display_value(display);
    let (conn, screen_num) = x11rb::connect(Some(name.as_str())).expect("an X connection");
    let screen = &conn.setup().roots[screen_num];
    let window: Window = conn.generate_id().expect("an X window id");
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        window,
        screen.root,
        0,
        0,
        200,
        200,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new(),
    )
    .expect("an X window")
    .check()
    .expect("the X server accepted the window");
    conn.map_window(window)
        .expect("a map request")
        .check()
        .expect("the X server accepted the map");
    // Override-redirect: maps itself, which exercises the notification
    // half (the XWM cannot prohibit it) rather than the request half.
    let overlay: Window = conn.generate_id().expect("an override-redirect id");
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        overlay,
        screen.root,
        0,
        0,
        100,
        100,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new().override_redirect(1),
    )
    .expect("an override-redirect window")
    .check()
    .expect("the X server accepted the overlay");
    conn.map_window(overlay)
        .expect("an overlay map request")
        .check()
        .expect("the X server accepted the overlay map");
    conn.flush().expect("the requests hit the wire");

    // Drive until both windows exist X-side (proving the client dialogue
    // really happened against this server), capturing the log throughout
    // so the handler half is pinned too. The walk goes two levels: the XWM
    // is a reparenting manager, so a managed window moves out of the
    // root's children into a frame -- presence anywhere in the tree is the
    // proof, and the reparenting itself proves the manager processed it.
    let ((), logs) = capture_logs(|| {
        let deadline = Instant::now() + XWAYLAND_PATIENCE;
        loop {
            fixture.settle();
            if tree_contains(&conn, screen.root, window, 2)
                && tree_contains(&conn, screen.root, overlay, 2)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the X server never showed the test windows"
            );
        }
        // The dialogue needs a few more dispatches to reach the handlers
        // (X11 events arrive over the WM's own source, behind the tree
        // reply the client just read).
        for _ in 0..20 {
            fixture.settle();
        }
    });
    let overlay_attrs = conn
        .get_window_attributes(overlay)
        .expect("an attributes query")
        .reply()
        .expect("the attributes reply");
    assert_eq!(
        overlay_attrs.map_state,
        MapState::VIEWABLE,
        "the override-redirect window should have mapped itself X-side"
    );
    assert!(
        logs.contains("map request refused"),
        "the normal window's map request never reached the handler: {logs}"
    );
    assert!(
        logs.contains("mapped itself"),
        "the override-redirect notification never reached the handler: {logs}"
    );
    assert!(
        fixture.state.windows.is_empty(),
        "an X window entered the core: {:?}",
        fixture.state.windows.keys()
    );
    assert!(
        fixture.state.space.elements().next().is_none(),
        "an X window entered the space"
    );

    // The globals stay invisible after the server started, too: the shell
    // global's `can_view` admits only XWayland's own client, and the grab
    // manager the same (both verified against the pinned Smithay source).
    // The listing client predates the start, so this also proves new
    // globals are (not) announced to existing registries correctly.
    let Ack::Globals(post) = fixture.run_on(0, Step::Relist);
    for global in ["xwayland_shell_v1", "zwp_xwayland_keyboard_grab_manager_v1"] {
        assert!(
            !post.iter().any(|seen| seen == global),
            "an ordinary client sees {global} on a live XWayland session: {post:?}"
        );
    }
}

/// The X server keeps running under session lock: locking changes
/// nothing about it (it must -- lock is not logout), and in this phase
/// there is nothing to blank and no input to refuse, so the lock and the
/// server simply coexist. Uses the shared [`Locker`] client: the lock is
/// real, not a flag.
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
    let mut fixture = Fixture::unstarted();
    let handle = fixture.state.loop_handle.clone();
    let display = super::start(handle, &mut fixture.state).expect("XWayland should start");
    wait_until(&mut fixture, "XWayland READY", |fixture| {
        fixture.state.xwm.is_some()
    });

    let ((), logs) = capture_logs(|| {
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
        // could watch, see `xwayland.rs`'s staleness note), and the
        // assertion after reads the captured logs.
        for _ in 0..200 {
            fixture.settle();
        }
        // The session underneath still works: it spawns, and dispatches
        // fine around the corpse.
        assert!(fixture.state.spawn(&["true".to_owned()]));
        fixture.settle();
    });
    assert!(
        logs.contains("continuing Wayland-only") || logs.contains("connection lost"),
        "the server died but the session never logged it loudly: {logs}"
    );

    // And a restart comes back: a fresh server, a fresh READY, no wedge
    // from the killed one's sockets (the lock scan moves on where the
    // dead one's locks linger).
    let handle = fixture.state.loop_handle.clone();
    let display = super::start(handle, &mut fixture.state).expect("a restart should start");
    wait_until(&mut fixture, "XWayland READY again", |fixture| {
        fixture.state.xwm.is_some() && fixture.state.xdisplay == Some(display)
    });
}
