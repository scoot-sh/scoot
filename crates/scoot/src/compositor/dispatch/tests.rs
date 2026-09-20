//! Regression tests for the parent module's two `wl_shm` pool-size guards.
//!
//! These deliberately drive a *real* `wayland-client` connection through a
//! real [`State`]'s real dispatch path rather than unit-testing the guards'
//! predicates in isolation: what is being guarded against is upstream code
//! that only runs when the generated `Dispatch` glue forwards to it, so a
//! test that never goes through that glue couldn't observe it at all. The
//! size cap's *accepted* cases matter for the same reason -- "the client was
//! not disconnected" is only meaningful if Smithay really did map the pool.
//!
//! Both tests need a writable `$XDG_RUNTIME_DIR`, since [`State::new`] binds
//! a real listening socket. That holds anywhere this crate is built (it is
//! Linux-only, and the dev VM's ssh sessions get one from logind); the
//! clients here don't use that socket -- they're inserted directly as
//! socket pairs -- but `State::new` creates it either way.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use scoot_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1;

use super::MAX_SHM_POOL_BYTES;
use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::shm_pools::MAX_POOLS_PER_CLIENT;
use crate::compositor::state::ClientState;
use crate::compositor::wl_buffers::MAX_BUFFERS_PER_CLIENT;

/// The framebuffer the fixture's render target is built at. Small on purpose:
/// nothing in this suite reads a pixel, it exists only so the session has a
/// renderer at all (see [`drive_both`]).
const CANVAS: i32 = 32;

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
    /// How the offending client's own run ended -- `Ok` if every request it
    /// made was accepted, the protocol error it provoked otherwise.
    resize: Result<(), DispatchError>,
    /// How many globals a *second*, independent client saw afterwards, and
    /// whether its own (valid) pool resize went through. This is the
    /// assertion that actually matters: the offending client must be the
    /// only casualty.
    survivor: Result<usize, DispatchError>,
    /// How many pools the compositor still counts across all clients once
    /// both runs are done. The live-count tests assert the bookkeeping
    /// drains (or holds exactly what is still alive), which only the
    /// compositor side can state.
    pools: usize,
    /// Same, for `wl_buffer`s. The buffer-count tests below read this the
    /// way the pool tests read `pools`.
    buffers: usize,
}

/// Binds `wl_shm`, makes a one-byte pool and asks for `size`.
///
/// The backing file stays one byte however large `size` is: `mmap`/`mremap`
/// past the end of a file is legal (only *touching* those pages faults, which
/// is what Smithay's SIGBUS handler is for), so this exercises the size
/// arithmetic without committing the memory.
fn resize_pool_to(stream: UnixStream, size: i32) -> Result<(), DispatchError> {
    let conn = Connection::from_socket(stream).expect("a client connection");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client)?;

    let shm = client.shm.clone().expect("the wl_shm global");
    let fd = rustix::fs::memfd_create("scoot-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    rustix::fs::ftruncate(&fd, 1).expect("a one-byte pool file");
    let pool = shm.create_pool(fd.as_fd(), 1, &qh, ());
    queue.roundtrip(&mut client)?;

    pool.resize(size);
    queue.roundtrip(&mut client)?;
    Ok(())
}

/// Binds `wl_shm` and asks for a pool of `size` bytes straight away -- the
/// other half of the cap, since `create_pool`'s size reaches the same `mmap`
/// `resize`'s does.
fn create_pool_of(stream: UnixStream, size: i32) -> Result<(), DispatchError> {
    let conn = Connection::from_socket(stream).expect("a client connection");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client)?;

    let shm = client.shm.clone().expect("the wl_shm global");
    let fd = rustix::fs::memfd_create("scoot-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    rustix::fs::ftruncate(&fd, 1).expect("a one-byte pool file");
    let _pool = shm.create_pool(fd.as_fd(), size, &qh, ());
    queue.roundtrip(&mut client)?;
    Ok(())
}

/// Serialises the fd-flood tests below (and the pool-count floods above,
/// plus the icon-buffer flood in `toplevel_icon/tests.rs`, which shares
/// this lock through `super::tests`).
///
/// `cargo test` runs tests in threads of one process, sharing one fd
/// table (`RLIMIT_NOFILE` 1024 on the dev VM). One flood holds ~512
/// server fds plus its client memfds at peak -- two such floods on two
/// threads exhaust the table for every test in the process, including
/// unrelated ones opening a single memfd. The floods each take a second
/// or two; everything else runs unserialised.
pub(in crate::compositor) static FD_FLOOD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Holds the [`FD_FLOOD_LOCK`] for one flood test. Recovers from poisoning
/// so one flood's panic fails that test, not every later flood with a
/// confusing lock error.
pub(in crate::compositor) fn hold_flood_lock() -> std::sync::MutexGuard<'static, ()> {
    FD_FLOOD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    let fd = rustix::fs::memfd_create("scoot-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    rustix::fs::ftruncate(&fd, 4096).expect("a one-page pool file");
    let pool = shm.create_pool(fd.as_fd(), 4096, &qh, ());
    queue.roundtrip(&mut client)?;

    rustix::fs::ftruncate(&fd, 8192).expect("a two-page pool file");
    pool.resize(8192);
    queue.roundtrip(&mut client)?;
    Ok(client.globals)
}

/// Stands up a real compositor, lets `offender` do whatever it likes on its
/// own connection, then lets a second, innocent client try to use it.
fn drive(
    offender: impl FnOnce(UnixStream) -> Result<(), DispatchError> + Send + 'static,
) -> Report {
    let (resize, survivor, pools, buffers) = drive_both(|offender_stream, survivor| {
        (offender(offender_stream), healthy_client(survivor))
    });
    Report {
        resize,
        survivor,
        pools,
        buffers,
    }
}

/// The two-connection form: one closure owns *both* client ends, so the
/// first connection can stay up -- holding its pools -- while the second
/// runs. That is what the isolation test needs; everything else goes
/// through [`drive`].
fn drive_both<A, B>(
    act: impl FnOnce(UnixStream, UnixStream) -> (A, B) + Send + 'static,
) -> (A, B, usize, usize)
where
    A: Send + 'static,
    B: Send + 'static,
{
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
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
    // A real render target, because three of the tests below reach the
    // `zwp_linux_dmabuf_v1` global and that global now exists only where a
    // renderer does: it is created by `headless::init` from the renderer whose
    // importable formats it advertises (see `dmabuf.rs`). Nothing else here
    // needs the output or the framebuffer -- these tests are about `wl_shm`
    // pools and the per-client buffer budget -- but building it is what makes
    // this fixture the same shape as a real session.
    crate::compositor::headless::init(&mut state, CANVAS, CANVAS)
        .expect("a headless backend behind the dispatch fixture");

    // Both clients are inserted up front, as socket pairs: that skips the
    // listening socket (and so any dependence on which name it got) while
    // still going through exactly the same per-client dispatch as a real
    // connection.
    let (offender_server, offender_stream) = UnixStream::pair().expect("a socket pair");
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
        let outcome = act(offender_stream, survivor);
        flag.store(true, Ordering::Release);
        outcome
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
    let (first, second) = clients.join().expect("the client thread");
    // Drain any disconnect cleanup the client thread's return queued: its
    // sockets are closed (the thread joined), but the destroyed hooks that
    // release the count only run in a dispatch. Without this, the pools read
    // below races cleanup -- usually winning, sometimes not. Zero-timeout
    // polls, so a settled loop costs microseconds; 50 passes is far more
    // than two disconnects need.
    for _ in 0..50 {
        event_loop
            .dispatch(Some(Duration::ZERO), &mut state)
            .expect("a compositor dispatch");
    }
    // Read after the drain, so every dispatch the clients provoked --
    // including disconnect cleanup -- has run.
    let pools = state.shm_pools.pools_in_flight();
    let buffers = state.wl_buffers.buffers_in_flight();
    (first, second, pools, buffers)
}

/// Asserts the offender was refused with `code` on `interface`, and on the
/// code rather than the message so the test stays valid whichever side -- a
/// guard here or Smithay's own handler -- answered.
fn assert_shm_protocol_error(
    result: &Result<(), DispatchError>,
    interface: &str,
    code: wl_shm::Error,
) {
    match result {
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
            assert_eq!(
                error.code, code as u32,
                "wrong protocol error code: {error:?}"
            );
            assert_eq!(
                error.object_interface, interface,
                "the error was posted on the wrong object: {error:?}"
            );
        }
        Err(other) => panic!("expected a protocol error, got {other:?}"),
        Ok(()) => panic!("the pool size was accepted"),
    }
}

/// Every refusal has to leave the compositor serving everyone else -- which is
/// the whole point of a protocol error over a panic.
fn assert_survivor_still_served(report: &Report) {
    let globals = *report
        .survivor
        .as_ref()
        .expect("a second client must still be served after the first misbehaved");
    assert!(globals > 0, "the second client saw no globals at all");
}

// -- the resize(0) crash (item 7) ----------------------------------------

#[test]
fn a_zero_sized_pool_resize_kills_only_the_client_that_asked() {
    let report = drive(|stream| resize_pool_to(stream, 0));
    assert_shm_protocol_error(&report.resize, "wl_shm_pool", wl_shm::Error::InvalidFd);
    assert_survivor_still_served(&report);
}

#[test]
fn a_negative_pool_resize_kills_only_the_client_that_asked() {
    // Negative sizes never reached the panic upstream (`-1i32 as usize`
    // sign-extends to a huge non-zero value), so this one passed before the
    // guard existed too -- it's here to prove the guard didn't change that
    // outcome while fixing zero.
    let report = drive(|stream| resize_pool_to(stream, -1));
    assert_shm_protocol_error(&report.resize, "wl_shm_pool", wl_shm::Error::InvalidFd);
    assert_survivor_still_served(&report);
}

// -- the size cap, at both requests that reach the same mmap -------------

#[test]
fn a_resize_up_to_the_cap_is_accepted() {
    // One under and exactly at: the cap must not refuse the largest pool it
    // claims to allow, and "accepted" here means Smithay really did remap it.
    for size in [MAX_SHM_POOL_BYTES - 1, MAX_SHM_POOL_BYTES] {
        let report = drive(move |stream| resize_pool_to(stream, size));
        if let Err(error) = &report.resize {
            panic!("a {size}-byte pool resize was refused: {error:?}");
        }
        assert_survivor_still_served(&report);
    }
}

#[test]
fn a_resize_past_the_cap_kills_only_the_client_that_asked() {
    // One over, and the extreme the backlog entry was written about.
    for size in [MAX_SHM_POOL_BYTES + 1, i32::MAX] {
        let report = drive(move |stream| resize_pool_to(stream, size));
        assert_shm_protocol_error(&report.resize, "wl_shm_pool", wl_shm::Error::InvalidFd);
        assert_survivor_still_served(&report);
    }
}

#[test]
fn creating_a_pool_up_to_the_cap_is_accepted() {
    for size in [MAX_SHM_POOL_BYTES - 1, MAX_SHM_POOL_BYTES] {
        let report = drive(move |stream| create_pool_of(stream, size));
        if let Err(error) = &report.resize {
            panic!("a {size}-byte pool was refused at creation: {error:?}");
        }
        assert_survivor_still_served(&report);
    }
}

/// The half a cap on `resize` alone would miss entirely: `create_pool` takes
/// the same size straight to the same `mmap`.
///
/// `InvalidStride`, not `InvalidFd`, because that is the code upstream's own
/// `create_pool` uses for a size it refuses -- and it is posted on `wl_shm`,
/// not on the pool, since the pool object is never initialized (see the parent
/// module's doc on why that is safe).
#[test]
fn creating_a_pool_past_the_cap_kills_only_the_client_that_asked() {
    for size in [MAX_SHM_POOL_BYTES + 1, i32::MAX] {
        let report = drive(move |stream| create_pool_of(stream, size));
        assert_shm_protocol_error(&report.resize, "wl_shm", wl_shm::Error::InvalidStride);
        assert_survivor_still_served(&report);
    }
}

/// A refused `create_pool` leaves an uninitialized object id behind, and the
/// client has a second request already on the wire behind it. Nothing may
/// dispatch that second request (`UninitObjectData::request` is a `panic!`) --
/// the kill has to stop it first. This is the case that would take the whole
/// compositor down if `post_error` did not `kill` synchronously.
#[test]
fn a_pipelined_request_after_a_refused_pool_does_not_reach_the_dead_object() {
    let report = drive(|stream| {
        let conn = Connection::from_socket(stream).expect("a client connection");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = TestClient::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client)?;

        let shm = client.shm.clone().expect("the wl_shm global");
        let fd = rustix::fs::memfd_create("scoot-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
            .expect("a memfd");
        rustix::fs::ftruncate(&fd, 1).expect("a one-byte pool file");
        // Both requests queued before a single flush, so they arrive in one
        // `write()` and the compositor reads them in one go.
        let pool = shm.create_pool(fd.as_fd(), i32::MAX, &qh, ());
        pool.resize(4096);
        queue.roundtrip(&mut client)?;
        Ok(())
    });
    assert_shm_protocol_error(&report.resize, "wl_shm", wl_shm::Error::InvalidStride);
    assert_survivor_still_served(&report);
}

// -- the per-client live-pool count (`shm_pools.rs`) -----------------------

/// One client connection that can hold pools open across round trips.
///
/// The free helpers above open one pool and drop the connection, which is
/// enough for size-cap tests -- but the count tests need a connection that
/// stays up while pools accumulate, so creation, destruction and
/// disconnect are separate steps on one held connection.
struct PoolClient {
    /// Held so the socket stays open: dropping the connection is the
    /// disconnect half of the drain test, and it must happen exactly when
    /// the test says, not when a helper returns.
    _conn: Connection,
    queue: EventQueue<TestClient>,
    client: TestClient,
    shm: wl_shm::WlShm,
    pools: Vec<wl_shm_pool::WlShmPool>,
    /// The backing files, kept alive until the pools that reference them
    /// are flushed: a destroyed fd before its `create_pool` goes out would
    /// hand Smithay an already-closed file.
    _fds: Vec<OwnedFd>,
}

impl PoolClient {
    fn connect(stream: UnixStream) -> Result<Self, DispatchError> {
        let conn = Connection::from_socket(stream).expect("a client connection");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = TestClient::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client)?;
        let shm = client.shm.clone().expect("the wl_shm global");
        Ok(Self {
            _conn: conn,
            queue,
            client,
            shm,
            pools: Vec::new(),
            _fds: Vec::new(),
        })
    }

    /// Opens one pool per size in a single batch and flushes once, holding
    /// every pool (and fd) open. Small sizes on purpose: the count is
    /// size-agnostic, and 4 KiB mappings keep a 129-pool flood cheap.
    fn create_many(&mut self, sizes: &[i32]) -> Result<(), DispatchError> {
        let qh = self.queue.handle();
        for size in sizes {
            let fd = rustix::fs::memfd_create("scoot-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
                .expect("a memfd");
            rustix::fs::ftruncate(&fd, 1).expect("a one-byte pool file");
            self.pools
                .push(self.shm.create_pool(fd.as_fd(), *size, &qh, ()));
            self._fds.push(fd);
        }
        self.queue.roundtrip(&mut self.client)?;
        Ok(())
    }

    /// Destroys every held pool and flushes, then forgets them client-side.
    fn destroy_all(&mut self) -> Result<(), DispatchError> {
        for pool in self.pools.drain(..) {
            pool.destroy();
        }
        self._fds.clear();
        self.queue.roundtrip(&mut self.client)?;
        Ok(())
    }
}

/// Over the live-pool cap, the excess `create_pool` is refused with the
/// same `InvalidStride` on `wl_shm` the per-pool cap uses -- and the kill
/// that carries it drains the dead client's pools: the compositor counts
/// zero afterwards, and the survivor never noticed.
#[test]
fn creating_pools_past_the_count_cap_kills_only_the_client_that_asked() {
    let _flood = hold_flood_lock();
    let report = drive(|stream| {
        let mut pools = PoolClient::connect(stream)?;
        pools.create_many(&vec![4096; MAX_POOLS_PER_CLIENT as usize + 1])?;
        Ok(())
    });
    assert_shm_protocol_error(&report.resize, "wl_shm", wl_shm::Error::InvalidStride);
    assert_survivor_still_served(&report);
    assert_eq!(
        report.pools, 0,
        "the killed client's pools must drain with it, or the count leaks per kill"
    );
}

/// Destroying pools reopens headroom: a full cap's worth created, all
/// destroyed, one more created -- the 129th lifetime pool succeeds iff
/// every destroy released its unit.
#[test]
fn destroying_pools_reopens_headroom() {
    let _flood = hold_flood_lock();
    let report = drive(|stream| {
        let mut pools = PoolClient::connect(stream)?;
        pools.create_many(&vec![4096; MAX_POOLS_PER_CLIENT as usize])?;
        pools.destroy_all()?;
        pools.create_many(&[4096])?;
        Ok(())
    });
    if let Err(error) = &report.resize {
        panic!("a pool created after destroying a full cap was refused: {error:?}");
    }
    assert_survivor_still_served(&report);
}

/// The anti-shared-table property: one client sitting exactly at the cap
/// must not deny a second client its first pool. Both runs succeed on their
/// own connections; a global count would refuse the second.
#[test]
fn a_second_client_is_unaffected_by_the_first_clients_full_cap() {
    let _flood = hold_flood_lock();
    let (first, second, _, _) = drive_both(|greedy_stream, survivor_stream| {
        let mut greedy = PoolClient::connect(greedy_stream)
            .expect("the greedy client connects and binds wl_shm");
        let first = greedy.create_many(&vec![4096; MAX_POOLS_PER_CLIENT as usize]);
        // `greedy` stays alive -- and its pools open -- while the second
        // client runs: that overlap is the whole assertion.
        let second = healthy_client(survivor_stream);
        (first, second)
    });
    if let Err(error) = &first {
        panic!("filling exactly to the cap was refused: {error:?}");
    }
    let globals = second
        .as_ref()
        .expect("a second client's first pool was refused while another sat at the cap");
    assert!(*globals > 0, "the second client saw no globals at all");
}

/// Both bounds compose: a pool past the per-pool cap is refused by that cap
/// without consuming a unit of this one -- the compositor still counts
/// zero, so a client that keeps overshooting cannot wedge its own budget.
#[test]
fn an_oversized_create_consumes_no_count_budget() {
    let report = drive(|stream| create_pool_of(stream, MAX_SHM_POOL_BYTES + 1));
    assert_shm_protocol_error(&report.resize, "wl_shm", wl_shm::Error::InvalidStride);
    assert_survivor_still_served(&report);
    assert_eq!(
        report.pools, 0,
        "a pool refused by the per-pool cap must not be counted"
    );
}

/// Same for the size upstream refuses: a zero-size `create_pool` dies with
/// upstream's `InvalidStride` and consumes nothing. Without the `size <= 0`
/// carve-out each of these would leak one unit for a client with no live
/// objects left to drain it.
#[test]
fn a_zero_size_create_consumes_no_count_budget() {
    let report = drive(|stream| create_pool_of(stream, 0));
    assert_shm_protocol_error(&report.resize, "wl_shm", wl_shm::Error::InvalidStride);
    assert_survivor_still_served(&report);
    assert_eq!(
        report.pools, 0,
        "a pool refused by upstream must not be counted"
    );
}

/// Same for a valid size on an unmappable fd: Smithay's own `mmap` fails
/// (`InvalidFd`, client killed, pool never created), so claiming one would
/// leak a unit no destruction could release -- the never-initialized object
/// keeps `UninitObjectData`, whose `destroyed` is a no-op that never reaches
/// the blanket hook. `/dev/null` cannot be mapped `SHARED`, deterministically.
#[test]
fn an_unmappable_fd_create_consumes_no_count_budget() {
    let report = drive(|stream| {
        let conn = Connection::from_socket(stream).expect("a client connection");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = TestClient::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client)?;

        let shm = client.shm.clone().expect("the wl_shm global");
        let null = std::fs::File::open("/dev/null").expect("/dev/null exists");
        let _pool = shm.create_pool(null.as_fd(), 4096, &qh, ());
        queue.roundtrip(&mut client)?;
        Ok(())
    });
    assert_shm_protocol_error(&report.resize, "wl_shm", wl_shm::Error::InvalidFd);
    assert_survivor_still_served(&report);
    assert_eq!(
        report.pools, 0,
        "a pool Smithay never created must not be counted"
    );
}

/// Disconnecting with live pools drains the count to zero: the destruction
/// hook runs for every object in cleanup, so no entry outlives the client
/// it names.
#[test]
fn disconnecting_with_live_pools_drains_the_count() {
    let report = drive(|stream| {
        let mut pools = PoolClient::connect(stream)?;
        pools.create_many(&[4096, 4096, 4096])?;
        // No destroy: dropping `pools` disconnects with three pools live.
        Ok(())
    });
    if let Err(error) = &report.resize {
        panic!("creating three small pools was refused: {error:?}");
    }
    assert_survivor_still_served(&report);
    assert_eq!(report.pools, 0, "live pools must drain on disconnect");
}

/// A `resize` neither claims nor frees: it grows the pool it names. 130
/// successive grows stay at a count of one -- if resizes ever counted,
/// this trips the cap and fails.
#[test]
fn resizing_a_pool_leaves_the_count_alone() {
    let report = drive(|stream| {
        let conn = Connection::from_socket(stream).expect("a client connection");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = TestClient::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client)?;

        let shm = client.shm.clone().expect("the wl_shm global");
        let fd = rustix::fs::memfd_create("scoot-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
            .expect("a memfd");
        rustix::fs::ftruncate(&fd, 4096).expect("a one-page pool file");
        let pool = shm.create_pool(fd.as_fd(), 4096, &qh, ());
        queue.roundtrip(&mut client)?;

        // The healthy pattern, 130 times: widen the backing first, then
        // grow the mapping into it. Each grow is far under the per-pool
        // cap, so only a miscounted resize could refuse one.
        let mut size = 4096i32;
        for _ in 0..130 {
            size += 1024;
            rustix::fs::ftruncate(&fd, size as u64).expect("a wider pool file");
            pool.resize(size);
            queue.roundtrip(&mut client)?;
        }
        Ok(())
    });
    if let Err(error) = &report.resize {
        panic!("growing one pool 130 times was refused: {error:?}");
    }
    assert_survivor_still_served(&report);
    assert_eq!(report.pools, 0, "the grown pool must drain on disconnect");
}

// -- the per-client live-buffer count (`wl_buffers.rs`) ---------------------

/// The client end of one buffer-count test connection: binds `wl_shm`, the
/// dmabuf global and the single-pixel manager up front, so one connection
/// can exercise all three `wl_buffer` factories.
#[derive(Default)]
struct BufferDispatch {
    shm: Option<wl_shm::WlShm>,
    dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    single_pixel: Option<wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for BufferDispatch {
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
            if interface == wl_shm::WlShm::interface().name {
                client.shm = Some(registry.bind(name, version.min(1), qh, ()));
            } else if interface == zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1::interface().name {
                client.dmabuf = Some(registry.bind(name, version, qh, ()));
            } else if interface
                == wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1::interface().name
            {
                client.single_pixel = Some(registry.bind(name, version.min(1), qh, ()));
            }
        }
    }
}

wayland_client::delegate_noop!(BufferDispatch: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(BufferDispatch: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(BufferDispatch: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(BufferDispatch: ignore zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1);
wayland_client::delegate_noop!(BufferDispatch: ignore zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1);
wayland_client::delegate_noop!(BufferDispatch: ignore wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1);

/// One client connection that can hold buffers open across round trips.
///
/// Pools are 4 KiB (the count is size-agnostic) and buffers are 1x1
/// `Argb8888` at staggered offsets, so hundreds of iterations stay cheap
/// while passing Smithay's own parameter validation -- a buffer that fails
/// validation kills the client, which is a different test below.
struct BufferClient {
    /// Held so the socket stays open: dropping the connection is the
    /// disconnect half of the drain test, and it must happen exactly when
    /// the test says, not when a helper returns.
    _conn: Connection,
    queue: EventQueue<BufferDispatch>,
    dispatch: BufferDispatch,
    buffers: Vec<wl_buffer::WlBuffer>,
    /// The backing files, kept alive until the pools that reference them
    /// are flushed: a destroyed fd before its `create_pool` goes out would
    /// hand Smithay an already-closed file.
    _fds: Vec<OwnedFd>,
}

impl BufferClient {
    fn connect(stream: UnixStream) -> Result<Self, DispatchError> {
        let conn = Connection::from_socket(stream).expect("a client connection");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut dispatch = BufferDispatch::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut dispatch)?;
        // The dmabuf and single-pixel globals, like the registry itself,
        // can arrive after the first batch.
        queue.roundtrip(&mut dispatch)?;
        if dispatch.shm.is_none() {
            panic!("the wl_shm global was never announced");
        }
        Ok(Self {
            _conn: conn,
            queue,
            dispatch,
            buffers: Vec::new(),
            _fds: Vec::new(),
        })
    }

    fn roundtrip(&mut self) -> Result<(), DispatchError> {
        self.queue.roundtrip(&mut self.dispatch)?;
        Ok(())
    }

    /// Opens one 4 KiB pool and holds it (and its fd) open.
    fn create_pool(&mut self) -> Result<wl_shm_pool::WlShmPool, DispatchError> {
        let qh = self.queue.handle();
        let shm = self.dispatch.shm.clone().expect("the wl_shm global");
        let fd = rustix::fs::memfd_create("scoot-buffer-test", rustix::fs::MemfdFlags::CLOEXEC)
            .expect("a memfd");
        rustix::fs::ftruncate(&fd, 4096).expect("a one-page pool file");
        let pool = shm.create_pool(fd.as_fd(), 4096, &qh, ());
        self._fds.push(fd);
        Ok(pool)
    }

    /// Creates one 1x1 buffer from `pool` at `offset`, held open.
    fn create_buffer(
        &mut self,
        pool: &wl_shm_pool::WlShmPool,
        offset: i32,
    ) -> Result<(), DispatchError> {
        let qh = self.queue.handle();
        self.buffers
            .push(pool.create_buffer(offset, 1, 1, 4, wl_shm::Format::Argb8888, &qh, ()));
        Ok(())
    }

    /// One bypass iteration -- `create_pool`, `create_buffer`, destroy the
    /// pool -- queued without flushing, so a full-cap flood is one batch.
    /// The buffer stays alive: that retention is what the guard counts.
    fn bypass_once(&mut self) {
        let pool = self.create_pool().expect("a pool for the bypass");
        self.create_buffer(&pool, 0)
            .expect("a buffer for the bypass");
        pool.destroy();
    }

    /// Flushes whatever is queued and drops the backing files flushed with
    /// it. A flood that held every memfd to the end would need 513 client
    /// fds next to its 512 retained server ones -- past `RLIMIT_NOFILE` on
    /// its own. Flushing in chunks keeps the client peak at the chunk size;
    /// the server already has its own dup of each fd (plus the mapping the
    /// retained buffer keeps), so closing the client's copy changes
    /// nothing the count observes.
    fn flush(&mut self) -> Result<(), DispatchError> {
        self.roundtrip()?;
        self._fds.clear();
        Ok(())
    }

    /// Destroys every held buffer and flushes, then forgets them
    /// client-side.
    fn destroy_all_buffers(&mut self) -> Result<(), DispatchError> {
        for buffer in self.buffers.drain(..) {
            buffer.destroy();
        }
        self.roundtrip()
    }
}

/// Asserts the offender was refused with a bare numeric `code` on
/// `interface` -- for factories whose protocol defines no error enum (the
/// single-pixel manager), where there is no named variant to assert.
fn assert_raw_protocol_error(result: &Result<(), DispatchError>, interface: &str, code: u32) {
    match result {
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
            assert_eq!(error.code, code, "wrong protocol error code: {error:?}");
            assert_eq!(
                error.object_interface, interface,
                "the error was posted on the wrong object: {error:?}"
            );
        }
        Err(other) => panic!("expected a protocol error, got {other:?}"),
        Ok(()) => panic!("the buffer creation was accepted"),
    }
}

/// `assert_raw_protocol_error` plus a cause pin on the server's message.
///
/// Budget and pressure refusals post the same code on the same object on
/// every factory here (7 on the dmabuf params, 0 on the single-pixel
/// manager), so code+interface cannot tell "the shared budget said no"
/// from "the pressured table said no" -- only the message proves the
/// kill came from the bound under test rather than the ceiling. The same
/// pin shape the icon half uses for its 513rd refusal, where its twins
/// likewise share code+object.
fn assert_raw_protocol_error_with_message(
    result: &Result<(), DispatchError>,
    interface: &str,
    code: u32,
    message_contains: &str,
) {
    match result {
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
            assert_eq!(error.code, code, "wrong protocol error code: {error:?}");
            assert_eq!(
                error.object_interface, interface,
                "the error was posted on the wrong object: {error:?}"
            );
            assert!(
                error.message.contains(message_contains),
                "the refusal came from the wrong cause (budget and fd-pressure share code+object): {error:?}"
            );
        }
        Err(other) => panic!("expected a protocol error, got {other:?}"),
        Ok(()) => panic!("the buffer creation was accepted"),
    }
}

/// Headroom for the fd-flood tests below: the `dispatch` half of the
/// treatment `toplevel_icon/tests.rs::ensure_flood_headroom` owns the
/// other half of (see
/// `docs/backlog/resolved/icon-buffer-budget-fd-pressure-flake-done.md`
/// for the mechanism, and
/// `docs/backlog/resolved/dispatch-flood-fd-pressure-flake-done.md` for
/// this half).
///
/// The 512-retaining fills sit near the process-wide fd-pressure
/// boundary by design, so under `cargo test` -- one process, one fd
/// table shared with every neighbour thread -- a few neighbour-held fds
/// are enough to land the refusal mid-fill instead of after it. Raising
/// the ceiling moves the boundary out of reach instead of sampling
/// around it. Same safe direction as the icon half: `free = soft - used`
/// only grows, so parallel neighbours see fewer pressure verdicts, never
/// more -- and the suite's own pressure pins drive hand-built tables or
/// a forked child's own copy of the limit, never the live process table.
///
/// `retained` is what the caller's fill keeps server-side: 512 for the
/// shm-bypass fills (one fd+mapping per retained buffer), 0 for the
/// single-pixel flood, which holds no fd at all and trips only on a
/// neighbour-pressured table -- hence its smaller need, and hence why it
/// still takes the lock: without it, a concurrent flood's ~512 fds
/// overlapping its microsecond grace-crossing is its whole exposure.
///
/// Must run under [`hold_flood_lock`], taken on the test thread *before*
/// `drive` (which every caller does): the lock is what keeps another
/// flood's ~512 fds from appearing between this check and the fill, and
/// taking it on the test thread is what keeps a failed check a fast
/// panic here rather than a stranded client thread and the 10s dispatch
/// deadline. What can still move is non-flood neighbours,
/// covered by `NEIGHBOUR_SLACK` -- and if they ever outgrow even that,
/// the failure is a loud panic here, never a fill reddened with the
/// wrong cause. Every failure is fast and names the numbers -- never a
/// skip, which would stop guarding the bound these floods exist to pin.
fn ensure_dispatch_flood_headroom(retained: u64) {
    use crate::compositor::fd_pressure::{RESERVE_FDS, table};

    /// Ceiling the floods run under: the same 4096 the icon half raises
    /// to, so one number covers the whole suite and a flood here never
    /// re-pressures a flood there.
    const TARGET_SOFT: u64 = 4096;
    /// Fds non-flood neighbours may transiently hold between the check
    /// and the fill. Generous on purpose: an ordinary fixture holds a
    /// socket pair, an event loop and a handful of memfds -- tens, not
    /// hundreds.
    const NEIGHBOUR_SLACK: u64 = 256;

    // SAFETY: `getrlimit` writes exactly one `struct rlimit` through a
    // live pointer to one, and returns nonzero on failure without
    // touching it (the same shape `fd_pressure` already uses).
    let mut limits: libc::rlimit = unsafe { std::mem::zeroed() };
    assert!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } == 0,
        "cannot read RLIMIT_NOFILE, so the flood's headroom is unknowable; refusing to run it blind"
    );
    let soft = limits.rlim_cur as u64;
    let hard = limits.rlim_max as u64;
    // Only ever raise, never lower -- and never past the hard limit.
    // (`hard` may be `RLIM_INFINITY`, which `min` folds away.)
    let goal = TARGET_SOFT.min(hard);
    if soft < goal {
        let raised = libc::rlimit {
            rlim_cur: goal as libc::rlim_t,
            rlim_max: limits.rlim_max,
        };
        // SAFETY: a plain value copy through a live pointer; on failure
        // the limit is unchanged.
        assert!(
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } == 0,
            "cannot raise RLIMIT_NOFILE to {goal} (soft {soft}, hard {hard}); the flood does not fit this table deterministically"
        );
    }
    let need_free = retained + RESERVE_FDS + NEIGHBOUR_SLACK;
    if let Some(observed) = table() {
        assert!(
            observed.free() >= need_free,
            "no deterministic headroom for the dispatch flood: table is {}/{} used/soft, \
             need {need_free} free ({retained} retained + {RESERVE_FDS} reserve + {NEIGHBOUR_SLACK} neighbour slack); \
             raise this process's RLIMIT_NOFILE hard limit",
            observed.used,
            observed.soft,
        );
    }
    // `table()` is `None` where there is no observable guard:
    // `/proc/self/fd` unreadable (macOS), an infinite table, or a table
    // below `MIN_TABLE_FDS` -- and every enforcement site fails open on
    // `None`, so pressure cannot fire. (A table physically too small for
    // the flood fails the flood itself with its own error, not with a
    // pressure refusal.)
}

/// The bypass loop the ticket is about: `create_pool` / `create_buffer` /
/// `destroy_pool` returns the live-pool count to zero every iteration while
/// the retained fd+mapping count grows. Past the buffer cap the excess
/// `create_buffer` is refused with `InvalidStride` on the pool -- the kill
/// that carries it drains the dead client's 512 buffers, and the survivor
/// never noticed.
///
/// Pre-fix this runs unbounded (513 live buffers, no refusal); the loop is
/// bounded in-test at one past the cap so it cannot exhaust the machine it
/// runs on.
#[test]
fn retaining_a_buffer_past_its_pool_trips_the_buffer_cap() {
    let report = drive(|stream| {
        let _flood = hold_flood_lock();
        let mut buffers = BufferClient::connect(stream)?;
        for i in 0..=MAX_BUFFERS_PER_CLIENT {
            buffers.bypass_once();
            if i % 64 == 63 {
                buffers.flush()?;
            }
        }
        buffers.roundtrip()
    });
    // Message pin, not the code-only `assert_shm_protocol_error`, on purpose:
    // the fd-pressure twin of this refusal posts the identical
    // `InvalidStride` on the identical pool object (see `reject_excess_buffer`),
    // so code+interface cannot tell "the 512-cap said no" from "the pressured
    // table said no" -- proven by running this test under
    // `prlimit --nofile=650:650`, where the kill lands at ~grace-129 with the
    // pressure cause and the code-only assertion still passes green. Only the
    // budget message proves the kill came from the bound under test (the
    // pressure text carries no `maximum of … live buffers`). The helper itself
    // is unchanged: its "stays valid whichever side answered" stance stands
    // for its other users, where no twin shares code+object.
    assert_raw_protocol_error_with_message(
        &report.resize,
        "wl_shm_pool",
        wl_shm::Error::InvalidStride as u32,
        &format!("maximum of {MAX_BUFFERS_PER_CLIENT} live buffers"),
    );
    assert_survivor_still_served(&report);
    assert_eq!(
        report.buffers, 0,
        "the killed client's buffers must drain with it, or the count leaks per kill"
    );
    assert_eq!(
        report.pools, 0,
        "every bypass pool was destroyed before the kill; none may be counted"
    );
}

/// Destroying a pool with live buffers keeps every buffer counted: five
/// buffers outliving their pool, then the rest of the budget filled the
/// bypass way -- the 512-live total succeeds iff the first five still
/// count, and the 513rd creation is refused.
#[test]
fn destroying_a_pool_with_live_buffers_keeps_every_buffer_counted() {
    let report = drive(|stream| {
        let _flood = hold_flood_lock();
        let mut buffers = BufferClient::connect(stream)?;
        let pool = buffers.create_pool()?;
        for i in 0..5 {
            buffers.create_buffer(&pool, i * 64)?;
        }
        buffers.roundtrip()?;
        pool.destroy();
        buffers.roundtrip()?;
        for i in 0..(MAX_BUFFERS_PER_CLIENT - 5) {
            buffers.bypass_once();
            if i % 64 == 63 {
                buffers.flush()?;
            }
        }
        buffers.roundtrip()?;
        // One more must be refused: 513 live with the first five retained.
        buffers.bypass_once();
        buffers.roundtrip()
    });
    // Message pin, not the code-only `assert_shm_protocol_error`, for the same
    // reason as the bypass-loop test above: pressure and budget refusals share
    // code+object on this path too, so only the budget message proves the
    // 513rd refusal came from the cap rather than a pressured table. The
    // helper's code-only stance stands for its other users; it is these two
    // sites that need the discrimination, not the helper that needs changing.
    assert_raw_protocol_error_with_message(
        &report.resize,
        "wl_shm_pool",
        wl_shm::Error::InvalidStride as u32,
        &format!("maximum of {MAX_BUFFERS_PER_CLIENT} live buffers"),
    );
    assert_survivor_still_served(&report);
    assert_eq!(report.buffers, 0, "the killed client's buffers must drain");
}

/// Legitimate shapes stay far from the cap: double-buffer churn (create a
/// third, destroy the oldest, twenty times) plus an 8-frame video-ish burst
/// held at once -- ten live at most against a 512 cap. Nothing refused.
#[test]
fn double_buffered_churn_and_a_burst_stay_far_from_the_cap() {
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        let pool = buffers.create_pool()?;
        buffers.create_buffer(&pool, 0)?;
        buffers.create_buffer(&pool, 64)?;
        for i in 0..20u32 {
            buffers.create_buffer(&pool, 128 + (i as i32) * 64)?;
            buffers.buffers.remove(0).destroy();
        }
        buffers.roundtrip()?;
        for i in 0..8u32 {
            buffers.create_buffer(&pool, 2048 + (i as i32) * 64)?;
        }
        buffers.roundtrip()?;
        buffers.destroy_all_buffers()?;
        Ok(())
    });
    if let Err(error) = &report.resize {
        panic!("a double-buffered client with an 8-frame burst was refused: {error:?}");
    }
    assert_survivor_still_served(&report);
    assert_eq!(report.buffers, 0, "destroyed buffers must release");
}

/// The anti-shared-table property for buffers: one client sitting exactly
/// at the cap (512 retained buffers, zero live pools -- the bypass shape at
/// rest) must not deny a second client its first buffer.
///
/// The 512-retaining fill runs under [`ensure_dispatch_flood_headroom`]:
/// without a raised ceiling the refusal lands mid-fill with the pressure
/// cause instead of after it (this test asserts the fill *succeeds*, so
/// there is no kill to discriminate -- the headroom half only, no message
/// pin).
#[test]
fn a_second_client_buffers_while_the_first_sits_at_the_cap() {
    // Lock and headroom on the test thread, before `drive_both`: a failed
    // check must panic here, fast -- inside the client thread it would
    // strand the dispatch loop on its 10s deadline instead.
    let _flood = hold_flood_lock();
    ensure_dispatch_flood_headroom(u64::from(MAX_BUFFERS_PER_CLIENT));
    let (first, second, _, _) = drive_both(|greedy_stream, survivor_stream| {
        let mut greedy = BufferClient::connect(greedy_stream).expect("the greedy client connects");
        for i in 0..MAX_BUFFERS_PER_CLIENT {
            greedy.bypass_once();
            if i % 64 == 63 {
                greedy.flush().expect("filling to the cap is served");
            }
        }
        let first = greedy.roundtrip();
        // `greedy` stays alive -- and its 512 buffers with it -- while the
        // second client runs: that overlap is the whole assertion.
        let second = (|| -> Result<(), DispatchError> {
            let mut survivor = BufferClient::connect(survivor_stream)?;
            let pool = survivor.create_pool()?;
            survivor.create_buffer(&pool, 0)?;
            survivor.roundtrip()
        })();
        (first, second)
    });
    if let Err(error) = &first {
        panic!("filling exactly to the cap was refused: {error:?}");
    }
    if let Err(error) = &second {
        panic!(
            "a second client's first buffer was refused while another sat at the cap: {error:?}"
        );
    }
}

/// A creation Smithay itself refuses still claims: the guard counts before
/// delegation, and a zero stride kills the client with `InvalidStride`
/// without ever initialising the buffer -- so no destruction hook can
/// release it. The unit sits on an entry whose client is already dead (one
/// per killing connection, never on a live budget); this pins that shape
/// rather than letting it drift unnoticed.
#[test]
fn a_failed_buffer_creation_leaves_only_a_dead_unit() {
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        let pool = buffers.create_pool()?;
        let qh = buffers.queue.handle();
        buffers
            .buffers
            .push(pool.create_buffer(0, 64, 64, 0, wl_shm::Format::Argb8888, &qh, ()));
        buffers.roundtrip()
    });
    assert_shm_protocol_error(&report.resize, "wl_shm_pool", wl_shm::Error::InvalidStride);
    assert_survivor_still_served(&report);
    assert_eq!(
        report.buffers, 1,
        "a creation Smithay refused must claim exactly one unit on its (dead) entry -- \
         zero would mean the guard stopped counting, more than one would mean it double-counts"
    );
}

/// Disconnecting with live buffers drains the count to zero: the
/// destruction hook runs for every object in cleanup -- including buffers
/// whose pool is already gone -- so no entry outlives the client it names.
#[test]
fn disconnecting_with_live_buffers_drains_the_count() {
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        let pool = buffers.create_pool()?;
        for i in 0..3 {
            buffers.create_buffer(&pool, i * 64)?;
        }
        buffers.roundtrip()?;
        // No destroy: dropping `buffers` disconnects with three buffers
        // (and their pool) live.
        Ok(())
    });
    if let Err(error) = &report.resize {
        panic!("creating three small buffers was refused: {error:?}");
    }
    assert_survivor_still_served(&report);
    assert_eq!(report.buffers, 0, "live buffers must drain on disconnect");
    assert_eq!(report.pools, 0, "the live pool must drain on disconnect");
}

/// A few single-pixel buffers are served normally: counted, released, and
/// nowhere near the cap. (The flood below proves they are counted at all;
/// this is the legitimate shape that must stay green.)
#[test]
fn a_few_single_pixel_buffers_are_accepted() {
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        let manager = buffers
            .dispatch
            .single_pixel
            .clone()
            .expect("the single-pixel manager global");
        let qh = buffers.queue.handle();
        for i in 0..3u32 {
            buffers.buffers.push(manager.create_u32_rgba_buffer(
                0xFF * i,
                0,
                0,
                0xFFFF_FFFF,
                &qh,
                (),
            ));
        }
        buffers.roundtrip()?;
        buffers.destroy_all_buffers()?;
        Ok(())
    });
    if let Err(error) = &report.resize {
        panic!("three single-pixel buffers were refused: {error:?}");
    }
    assert_survivor_still_served(&report);
    assert_eq!(
        report.buffers, 0,
        "destroyed single-pixel buffers must release"
    );
}

/// Single-pixel buffers count toward the same budget: 513 of them trip the
/// cap even though each holds no fd. Refused with a bare 0 on the manager
/// -- the interface defines no error enum -- killing only the flooder.
///
/// The fd-pressure twin of that refusal posts the identical 0 on the
/// identical manager object, so code+interface cannot discriminate them:
/// the assertion pins the budget cause by message (required here, not
/// optional), the same pin the icon half uses where its twins likewise
/// share code+object. The flood holds no fd itself, so its only pressure
/// exposure is a concurrent fd-holder overlapping its microsecond
/// grace-128 crossing -- which is why it takes the shared
/// `FD_FLOOD_LOCK` like every other flood (it did not, before this
/// treatment) and runs under [`ensure_dispatch_flood_headroom`].
#[test]
fn flooding_single_pixel_buffers_trips_the_same_cap() {
    // Lock and headroom on the test thread, before `drive`: a failed
    // check must panic here, fast -- inside the client thread it would
    // strand the dispatch loop on its 10s deadline instead.
    let _flood = hold_flood_lock();
    ensure_dispatch_flood_headroom(0);
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        let manager = buffers
            .dispatch
            .single_pixel
            .clone()
            .expect("the single-pixel manager global");
        let qh = buffers.queue.handle();
        for _ in 0..=MAX_BUFFERS_PER_CLIENT {
            buffers
                .buffers
                .push(manager.create_u32_rgba_buffer(0, 0, 0, 0xFFFF_FFFF, &qh, ()));
        }
        buffers.roundtrip()
    });
    assert_raw_protocol_error_with_message(
        &report.resize,
        "wp_single_pixel_buffer_manager_v1",
        0,
        &format!("maximum of {MAX_BUFFERS_PER_CLIENT} live buffers"),
    );
    assert_survivor_still_served(&report);
    assert_eq!(
        report.buffers, 0,
        "the killed client's single-pixel buffers must drain with it"
    );
}

/// A dmabuf `create_immed` past a full shm budget is refused by the guard
/// before Smithay ever validates it: the budget is shared across
/// factories, so the error is the guard's `InvalidWlBuffer` (7), not the
/// `InvalidFormat` (4) the garbage format below would otherwise earn. That
/// code difference is what proves the refusal came from the shared budget
/// rather than the import path.
///
/// The 512-buffer fill runs under [`ensure_dispatch_flood_headroom`]: it
/// sits near the fd-pressure boundary by design, and without a raised
/// ceiling the refusal lands mid-fill with the pressure cause instead of
/// after it with the budget one. The assertion pins the budget cause by
/// message for the same reason -- both causes post 7 on this object.
#[test]
fn a_dmabuf_immed_past_a_full_budget_is_refused_before_validation() {
    // Lock and headroom on the test thread, before `drive`: a failed
    // check must panic here, fast -- inside the client thread it would
    // strand the dispatch loop on its 10s deadline instead.
    let _flood = hold_flood_lock();
    ensure_dispatch_flood_headroom(u64::from(MAX_BUFFERS_PER_CLIENT));
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        for i in 0..MAX_BUFFERS_PER_CLIENT {
            buffers.bypass_once();
            if i % 64 == 63 {
                buffers.flush()?;
            }
        }
        buffers.roundtrip()?;
        let dmabuf = buffers.dispatch.dmabuf.clone().expect("the dmabuf global");
        let qh = buffers.queue.handle();
        let params = dmabuf.create_params(&qh, ());
        let size = 32i32;
        let stride = size * 4;
        let fd =
            rustix::fs::memfd_create("scoot-dmabuf-budget-test", rustix::fs::MemfdFlags::CLOEXEC)
                .expect("a memfd");
        rustix::fs::ftruncate(&fd, (stride * size) as u64).expect("a sized memfd");
        params.add(fd.as_fd(), 0, 0, stride as u32, 0, 0);
        buffers.buffers.push(params.create_immed(
            size,
            size,
            0xFFFF_FFFF,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &qh,
            (),
        ));
        buffers.roundtrip()
    });
    assert_raw_protocol_error_with_message(
        &report.resize,
        "zwp_linux_buffer_params_v1",
        7,
        &format!("maximum of {MAX_BUFFERS_PER_CLIENT} live buffers"),
    );
    assert_survivor_still_served(&report);
    assert_eq!(
        report.buffers, 0,
        "the killed client's shm budget must drain with it"
    );
}

/// The dmabuf async `create` claims like every other factory now that a
/// successful import really mints a `wl_buffer` on that path, so past a full
/// shm budget it is refused by the guard with the same shared-budget
/// `InvalidWlBuffer` (7) the `create_immed` case above earns.
///
/// This test used to assert the opposite -- `create` claiming nothing and
/// still being answered `failed` -- which was correct only while this
/// compositor refused every import. See `wl_buffers.rs`: uncounted, the
/// async path would be the one `wl_buffer` factory outside
/// `MAX_BUFFERS_PER_CLIENT` entirely.
///
/// The 512-buffer fill runs under [`ensure_dispatch_flood_headroom`]: it
/// sits near the fd-pressure boundary by design, and without a raised
/// ceiling the refusal lands mid-fill with the pressure cause instead of
/// after it with the budget one. The assertion pins the budget cause by
/// message for the same reason -- both causes post 7 on this object.
#[test]
fn a_dmabuf_create_past_a_full_budget_is_refused_by_the_shared_budget() {
    // Lock and headroom on the test thread, before `drive`: a failed
    // check must panic here, fast -- inside the client thread it would
    // strand the dispatch loop on its 10s deadline instead.
    let _flood = hold_flood_lock();
    ensure_dispatch_flood_headroom(u64::from(MAX_BUFFERS_PER_CLIENT));
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        for i in 0..MAX_BUFFERS_PER_CLIENT {
            buffers.bypass_once();
            if i % 64 == 63 {
                buffers.flush()?;
            }
        }
        buffers.roundtrip()?;
        let dmabuf = buffers.dispatch.dmabuf.clone().expect("the dmabuf global");
        let qh = buffers.queue.handle();
        let params = dmabuf.create_params(&qh, ());
        let size = 32i32;
        let stride = size * 4;
        let fd =
            rustix::fs::memfd_create("scoot-dmabuf-create-test", rustix::fs::MemfdFlags::CLOEXEC)
                .expect("a memfd");
        rustix::fs::ftruncate(&fd, (stride * size) as u64).expect("a sized memfd");
        params.add(fd.as_fd(), 0, 0, stride as u32, 0, 0);
        params.create(
            size,
            size,
            u32::from_ne_bytes(*b"XR24"),
            zwp_linux_buffer_params_v1::Flags::empty(),
        );
        buffers.roundtrip()
    });
    assert_raw_protocol_error_with_message(
        &report.resize,
        "zwp_linux_buffer_params_v1",
        7,
        &format!("maximum of {MAX_BUFFERS_PER_CLIENT} live buffers"),
    );
    assert_survivor_still_served(&report);
    assert_eq!(
        report.buffers, 0,
        "the killed client's shm budget must drain with it"
    );
}

/// A dmabuf `create_immed` this compositor cannot import -- the "dma-buf" the
/// client offers is a plain memfd, which `DMA_BUF_IOCTL_SYNC` rejects with
/// `ENOTTY` under pixman and `eglCreateImageKHR` rejects under GLES -- kills
/// the client with `InvalidWlBuffer`, takes no one else down with it, and
/// leaves the buffer budget empty.
///
/// **The reason the import fails changed, and the assertions did not.** This
/// used to say "the harness builds a `State` with no backend at all, so there
/// is no renderer to import into". That stopped being how the fixture works
/// when the dmabuf global became renderer-derived: the global exists only
/// where a renderer does, so a fixture that can send `create_immed` at all
/// necessarily has one (see `drive_both`). The refusal is now the renderer's
/// own, which is the more realistic path and the one
/// `dmabuf/tests.rs::a_fake_dmabuf_over_a_plain_memfd_is_refused_not_trusted`
/// covers from the other side. What this test is *for* -- the kill, its blast
/// radius, and the budget drain -- is unchanged by that.
///
/// **What this no longer proves, deliberately stated rather than left to be
/// assumed:** the final zero used to be read as "Smithay initialised the
/// buffer object before running the import, else one phantom unit would be
/// left". That inference died when `dmabuf.rs`'s `refuse_import` started
/// handing the claimed unit back itself (it has to -- see `wl_buffers.rs`),
/// because the count now lands on zero either way. The error-before-init
/// shape is instead guarded by
/// `dmabuf/tests.rs::an_import_through_create_immed_is_not_a_client_kill`,
/// which asserts the count is exactly *one* on the accepted path, where
/// nothing releases it early. What is left here is still worth having, and is
/// what the assertions below now say: the kill, its blast radius, and the
/// drain.
#[test]
fn a_failed_dmabuf_import_kills_only_that_client_and_drains_its_budget() {
    let report = drive(|stream| {
        let mut buffers = BufferClient::connect(stream)?;
        let dmabuf = buffers.dispatch.dmabuf.clone().expect("the dmabuf global");
        let qh = buffers.queue.handle();
        let params = dmabuf.create_params(&qh, ());
        let size = 32i32;
        let stride = size * 4;
        let fd =
            rustix::fs::memfd_create("scoot-dmabuf-buffer-test", rustix::fs::MemfdFlags::CLOEXEC)
                .expect("a memfd");
        rustix::fs::ftruncate(&fd, (stride * size) as u64).expect("a sized memfd");
        params.add(fd.as_fd(), 0, 0, stride as u32, 0, 0);
        buffers.buffers.push(params.create_immed(
            size,
            size,
            u32::from_ne_bytes(*b"XR24"),
            zwp_linux_buffer_params_v1::Flags::empty(),
            &qh,
            (),
        ));
        buffers.roundtrip()
    });
    assert_raw_protocol_error(&report.resize, "zwp_linux_buffer_params_v1", 7);
    assert_survivor_still_served(&report);
    assert_eq!(
        report.buffers, 0,
        "the killed client's buffer budget must drain with it, whether the \
         unit came back through `refuse_import` or through disconnect cleanup"
    );
}

// --- `pressure_refusal_for` boundary pins -----------------------------------
//
// Filed from PR #123 review (`fd-pressure-grace-boundary-pins`): the shipped
// conjunction is correct as written, and these pin it rather than fixing it.
// An operator flip here (`>` to `>=`, `&&` to `||`) kills under-grace
// clients during pressure -- the exact catastrophe the grace exists to
// prevent -- so each operator direction gets its own failing-loud test.
//
// The pure conjunction is tested, not `pressure_refusal` itself: the table
// half reads the test process's own fd table, which no in-suite test can
// drive to pressure without starving its siblings. All four call sites (the
// pool claim and the three buffer claims) funnel through this predicate
// with one of the two grace constants below, so pinning the constants plus
// the predicate covers every site.

use super::pressure_refusal_for;
use crate::compositor::fd_pressure::{PRESSURE_GRACE_BUFFERS, PRESSURE_GRACE_POOLS};

#[test]
fn pressure_graces_are_128_buffers_and_64_pools() {
    // A silent grace change moves every boundary below; fail loudly here.
    assert_eq!(PRESSURE_GRACE_BUFFERS, 128);
    assert_eq!(PRESSURE_GRACE_POOLS, 64);
}

#[test]
fn at_grace_passes_even_under_pressure() {
    // `>` is exact, not `>=`: holding exactly the grace is never a refusal.
    // `live > grace` permits grace+1 units, so the permitted per-connection
    // fd maximum is 2 x (129 + 65 + 1) + 14 = 404, not the 400 "two at
    // grace" holds.
    for grace in [PRESSURE_GRACE_BUFFERS, PRESSURE_GRACE_POOLS] {
        assert!(
            !pressure_refusal_for(0, grace, true),
            "an empty client is never refused"
        );
        assert!(
            !pressure_refusal_for(grace - 1, grace, true),
            "one under grace passes under pressure"
        );
        assert!(
            !pressure_refusal_for(grace, grace, true),
            "at grace passes under pressure"
        );
    }
}

#[test]
fn one_past_grace_refuses_under_pressure() {
    // The 129th buffer / 65th pool is the first refusal -- one past grace,
    // not at it.
    assert!(
        pressure_refusal_for(PRESSURE_GRACE_BUFFERS + 1, PRESSURE_GRACE_BUFFERS, true),
        "129 live buffers refuse under pressure"
    );
    assert!(
        pressure_refusal_for(PRESSURE_GRACE_POOLS + 1, PRESSURE_GRACE_POOLS, true),
        "65 live pools refuse under pressure"
    );
}

#[test]
fn past_grace_without_pressure_passes() {
    // `&&` is exact, not `||`: over grace alone, with a calm table, never
    // refuses. (Past the per-connection 512/128 caps the cap path still
    // refuses -- this pins only the pressure half.)
    for grace in [PRESSURE_GRACE_BUFFERS, PRESSURE_GRACE_POOLS] {
        assert!(
            !pressure_refusal_for(grace + 1, grace, false),
            "past grace with a calm table passes"
        );
        assert!(
            !pressure_refusal_for(u32::MAX, grace, false),
            "any holding with a calm table passes"
        );
    }
}

#[test]
fn under_grace_with_pressure_passes() {
    // The other half of `&&` vs `||`: pressure alone, with the client under
    // grace, never refuses -- this is the innocent-client guarantee.
    for grace in [PRESSURE_GRACE_BUFFERS, PRESSURE_GRACE_POOLS] {
        for live in [0, 1, grace - 1, grace] {
            assert!(
                !pressure_refusal_for(live, grace, true),
                "live {live} under a {grace} grace passes under pressure"
            );
        }
    }
}
