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

use flexwm_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle};

use super::MAX_SHM_POOL_BYTES;
use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::shm_pools::MAX_POOLS_PER_CLIENT;
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
    let fd = rustix::fs::memfd_create("flexwm-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
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
    let fd = rustix::fs::memfd_create("flexwm-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    rustix::fs::ftruncate(&fd, 1).expect("a one-byte pool file");
    let _pool = shm.create_pool(fd.as_fd(), size, &qh, ());
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

/// Stands up a real compositor, lets `offender` do whatever it likes on its
/// own connection, then lets a second, innocent client try to use it.
fn drive(
    offender: impl FnOnce(UnixStream) -> Result<(), DispatchError> + Send + 'static,
) -> Report {
    let (resize, survivor, pools) = drive_both(|offender_stream, survivor| {
        (offender(offender_stream), healthy_client(survivor))
    });
    Report {
        resize,
        survivor,
        pools,
    }
}

/// The two-connection form: one closure owns *both* client ends, so the
/// first connection can stay up -- holding its pools -- while the second
/// runs. That is what the isolation test needs; everything else goes
/// through [`drive`].
fn drive_both<A, B>(
    act: impl FnOnce(UnixStream, UnixStream) -> (A, B) + Send + 'static,
) -> (A, B, usize)
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
    )
    .expect("a compositor state with a wayland socket");

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
    (first, second, pools)
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
        let fd = rustix::fs::memfd_create("flexwm-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
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
            let fd = rustix::fs::memfd_create("flexwm-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
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
    let (first, second, _) = drive_both(|greedy_stream, survivor_stream| {
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
        let fd = rustix::fs::memfd_create("flexwm-shm-test", rustix::fs::MemfdFlags::CLOEXEC)
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
