//! Regression tests for the `wl_shm_pool.resize` guard in the parent module.
//!
//! These deliberately drive a *real* `wayland-client` connection through a
//! real [`State`]'s real dispatch path rather than unit-testing the guard's
//! predicate in isolation: the bug being guarded against is upstream code
//! that only runs when the generated `Dispatch` glue forwards to it, so a
//! test that never goes through that glue couldn't observe it at all.
//!
//! Both tests need a writable `$XDG_RUNTIME_DIR`, since [`State::new`] binds
//! a real listening socket. That holds anywhere this crate is built (it is
//! Linux-only, and the dev VM's ssh sessions get one from logind); the
//! clients here don't use that socket -- they're inserted directly as
//! socket pairs -- but `State::new` creates it either way.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use flexwm_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, DispatchError, Proxy, QueueHandle};

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// The client end of one test connection.
#[derive(Default)]
struct TestClient {
    shm: Option<wl_shm::WlShm>,
    globals: usize,
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            client.globals += 1;
            if interface == wl_shm::WlShm::interface().name {
                client.shm = Some(registry.bind(name, version.min(1), qh, ()));
            }
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);

/// What one run of [`drive`] observed, from the client side.
struct Report {
    /// How the `wl_shm_pool.resize(size)` attempt ended.
    resize: Result<(), DispatchError>,
    /// How many globals a *second*, independent client saw afterwards, and
    /// whether its own (valid) pool resize went through. This is the
    /// assertion that actually matters: the offending client must be the
    /// only casualty.
    survivor: Result<usize, DispatchError>,
}

/// Binds `wl_shm`, makes a one-byte pool and asks for `size`.
fn resize_pool_to(stream: UnixStream, size: i32) -> Result<(), DispatchError> {
    let conn = Connection::from_socket(stream).expect("a client connection");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client)?;

    let shm = client.shm.clone().expect("the wl_shm global");
    let fd = rustix::fs::memfd_create("flexwm-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    rustix::fs::ftruncate(&fd, 1).expect("a one-byte pool file");
    let pool = shm.create_pool(fd.as_fd(), 1, &qh, ());
    queue.roundtrip(&mut client)?;

    pool.resize(size);
    queue.roundtrip(&mut client)?;
    Ok(())
}

/// A well-behaved client: connects, binds `wl_shm`, and grows a pool the way
/// a real toolkit does on its first resize. Proves both that the compositor
/// is still serving and that the guard didn't break valid resizes.
fn healthy_client(stream: UnixStream) -> Result<usize, DispatchError> {
    let conn = Connection::from_socket(stream).expect("a client connection");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client)?;

    let shm = client.shm.clone().expect("the wl_shm global");
    let fd = rustix::fs::memfd_create("flexwm-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    rustix::fs::ftruncate(&fd, 4096).expect("a one-page pool file");
    let pool = shm.create_pool(fd.as_fd(), 4096, &qh, ());
    queue.roundtrip(&mut client)?;

    rustix::fs::ftruncate(&fd, 8192).expect("a two-page pool file");
    pool.resize(8192);
    queue.roundtrip(&mut client)?;
    Ok(client.globals)
}

/// Stands up a real compositor, lets one client ask for `resize(size)`, then
/// lets a second, innocent client try to use it.
fn drive(size: i32) -> Report {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    let display: Display<State> = Display::new().expect("a wayland display");
    let mut state = State::new(
        &mut event_loop,
        display,
        Config::default(),
        Keybindings::default(),
        Appearance::default(),
    );

    // Both clients are inserted up front, as socket pairs: that skips the
    // listening socket (and so any dependence on which name it got) while
    // still going through exactly the same per-client dispatch as a real
    // connection.
    let (offender_server, offender) = UnixStream::pair().expect("a socket pair");
    let (survivor_server, survivor) = UnixStream::pair().expect("a socket pair");
    for stream in [offender_server, survivor_server] {
        state
            .display_handle
            .insert_client(stream, Arc::new(ClientState::default()))
            .expect("an inserted client");
    }

    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    let clients = thread::spawn(move || {
        let report = Report {
            resize: resize_pool_to(offender, size),
            survivor: healthy_client(survivor),
        };
        flag.store(true, Ordering::Release);
        report
    });

    // The clients block on their roundtrips, so the compositor has to be
    // dispatched from here until they're done. The deadline only exists so a
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
    clients.join().expect("the client thread")
}

/// `wl_shm::Error::InvalidFd` is what upstream posts on both resize failure
/// paths, so asserting on the code (not the message) keeps this test valid
/// whether the guard or Smithay's own code answered.
fn assert_shm_pool_protocol_error(result: Result<(), DispatchError>) {
    match result {
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
            assert_eq!(
                error.code,
                wl_shm::Error::InvalidFd as u32,
                "wrong protocol error code: {error:?}"
            );
            assert_eq!(
                error.object_interface, "wl_shm_pool",
                "the error was posted on the wrong object: {error:?}"
            );
        }
        Err(other) => panic!("expected a protocol error, got {other:?}"),
        Ok(()) => panic!("the pool resize was accepted"),
    }
}

#[test]
fn a_zero_sized_pool_resize_kills_only_the_client_that_asked() {
    let report = drive(0);
    assert_shm_pool_protocol_error(report.resize);
    let globals = report
        .survivor
        .expect("a second client must still be served after the first misbehaved");
    assert!(globals > 0, "the second client saw no globals at all");
}

#[test]
fn a_negative_pool_resize_kills_only_the_client_that_asked() {
    // Negative sizes never reached the panic upstream (`-1i32 as usize`
    // sign-extends to a huge non-zero value), so this one passed before the
    // guard existed too -- it's here to prove the guard didn't change that
    // outcome while fixing zero.
    let report = drive(-1);
    assert_shm_pool_protocol_error(report.resize);
    let globals = report
        .survivor
        .expect("a second client must still be served after the first misbehaved");
    assert!(globals > 0, "the second client saw no globals at all");
}
