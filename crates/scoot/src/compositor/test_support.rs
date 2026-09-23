//! The harness the real-`wayland-client` test suites share.
//!
//! Several modules here test what a *client* was actually told, or what
//! actually reached the framebuffer, by driving a real `wayland-client`
//! connection over a real `UnixStream` pair through a real [`State`]. That is
//! deliberate and stays that way -- a test asserting on which enum variant a
//! render path chose would pass against a version that drew the wrong pixels.
//! What it costs is boilerplate: an event loop, a `State`, a socket pair, a
//! client thread, and a dispatch-until-it-answers loop, per suite.
//!
//! That boilerplate was written out independently in five files before this
//! module existed, near byte-for-byte, so a fix to any of it had to be
//! rediscovered five times (see
//! `docs/backlog/resolved/large-test-file-organization-done.md`). It lives
//! here now. What stays per-suite is what genuinely differs: each suite's own
//! `Step`/`Ack` vocabulary, its `TestClient` and the `Dispatch` impls for the
//! protocol it exercises, and its buffer and colour choices.
//!
//! The intended shape for a suite is a type alias plus an inherent impl:
//!
//! ```ignore
//! type Fixture = Harness<Step, Ack>;
//!
//! impl Fixture {
//!     fn new() -> Self {
//!         let mut fixture = Harness::headless(appearance(), CANVAS);
//!         fixture.spawn(run_client);
//!         fixture
//!     }
//! }
//! ```
//!
//! which keeps every call site reading `fixture.run(...)`. Inherent impls on a
//! concrete instantiation are legal because this is all one crate; two suites
//! cannot collide because their `Step` types differ. They *can* collide with
//! the generic impl below, so nothing here is named `new`.
//!
//! Like the suites that use it, this needs a writable `$XDG_RUNTIME_DIR`:
//! [`State::new`] binds a real wayland listening socket, which nothing
//! connects to (clients are inserted as socket pairs) but which is created
//! either way.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use scoot_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::{Client, Display};
use wayland_client::EventQueue;
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};

use crate::cli::RendererKind;
use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// How long the compositor side will dispatch before deciding the client it
/// is waiting on is never going to answer. Generous: a debug build under a
/// VM. It exists so a regression fails in seconds rather than hanging the
/// suite forever, not as a timing assertion.
const PATIENCE: Duration = Duration::from_secs(10);

/// ...and the same, for a client waiting on the compositor.
///
/// A *deadline*, not a number of round trips, and the two are not
/// interchangeable: a compositor answer that waits on a frame tick
/// (`headless::FRAME_INTERVAL`, 16ms) is many round trips away from a
/// compositor being dispatched on another thread, so a round-trip count can
/// give up before the frame it is waiting for could possibly have happened.
const CLIENT_PATIENCE: Duration = Duration::from_secs(5);

/// The renderer every test in the crate builds its `State` with, from
/// `SCOOT_TEST_RENDERER` (`pixman` -- the default -- or `gles`).
///
/// This is how the pixel-readback suites become the regression net for a
/// *second* renderer instead of just the first: the same tests, the same
/// assertions, run a second time with `SCOOT_TEST_RENDERER=gles`, and any
/// pixel the two renderers disagree about fails a real test rather than
/// going unnoticed. One env var rather than a parameterised `Harness::new`
/// per suite because the suites assert on pixels, not on renderers -- a
/// GLES-run variant of each would be the same assertions copied twice.
///
/// Public to the crate because a dozen suites still build their `State` by
/// hand rather than through [`Harness`] (they predate it), and a run that
/// covered only the harness-based ones would be a partial claim.
///
/// Both failure modes are loud on purpose. An unrecognised value panics
/// rather than falling back, and a `gles` run on a machine where GLES cannot
/// be built panics inside `headless::init` with the renderer's own error --
/// because a run that quietly used pixman would report a clean pass for a
/// renderer it never touched.
///
/// # Known: seven `dmabuf` tests fail under `gles` on software EGL
///
/// Every pixel-readback suite passes byte-identically under both renderers,
/// and so does every other test in the crate (the reload suite's renderer
/// refusal names whichever renderer is *not* running, so it refuses under
/// either). The exception is `dmabuf::tests`' seven *import* tests, and it is a
/// property of the machine, not of this change: those tests synthesise a
/// dma-buf from a memfd through `/dev/udmabuf`, which pixman imports by
/// mmapping it, while GLES must hand it to the driver -- and Mesa's
/// `kms_swrast` answers `eglCreateImageKHR: createImageFromDmaBufs failed`
/// (`EGL_BAD_ALLOC`) for a udmabuf-backed import.
///
/// It is the buffer's **provenance** that is refused, not its format: both
/// advertised formats are present with `LINEAR` among that display's 76
/// import formats. So stage 4's renderer-derived advertisement would *not*
/// fix these seven -- it would name the same two formats. Do not read them
/// as blocked on stage 4; they are blocked on `kms_swrast` accepting a
/// udmabuf, or on the tests allocating through GBM instead.
pub(crate) fn test_renderer() -> RendererKind {
    static RENDERER: OnceLock<RendererKind> = OnceLock::new();
    *RENDERER.get_or_init(|| match std::env::var("SCOOT_TEST_RENDERER") {
        Ok(name) => RendererKind::parse(&name).unwrap_or_else(|| {
            panic!("SCOOT_TEST_RENDERER must be `pixman` or `gles`, not `{name}`")
        }),
        Err(_) => RendererKind::default(),
    })
}

/// A live compositor on its own event loop, plus the client threads scripted
/// against it.
///
/// `S` is the suite's step vocabulary (what a test tells its client to do) and
/// `A` what the client answers with. Both are per-suite on purpose: the
/// variants encode what *this* protocol's test script can perform, and one
/// shared enum of every protocol's steps would be a worse abstraction than the
/// duplication it removed.
pub(crate) struct Harness<S, A> {
    pub(crate) event_loop: EventLoop<'static, State>,
    pub(crate) state: State,
    /// The framebuffer size [`Harness::pixels`] reads back, or `None` when the
    /// harness was built without a backend.
    canvas: Option<i32>,
    clients: Vec<ClientHandle<S, A>>,
}

/// One connected client thread and the channels driving it.
struct ClientHandle<S, A> {
    /// The compositor's own handle on the client, for resolving a protocol id
    /// the client reported back into the server-side resource it names.
    client: Client,
    /// `None` once the test has disconnected this client on purpose. Dropping
    /// it is what ends the client script's `steps.recv()` loop.
    steps: Option<Sender<S>>,
    acks: Receiver<A>,
    /// `None` once the thread has been joined -- by a disconnect, or by the
    /// diagnosis of a client that died mid-step.
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl<S, A> Harness<S, A> {
    /// A compositor with no backend at all: nothing renders, and
    /// [`Harness::pixels`] would panic. For suites that assert on the wire, or
    /// that render offscreen themselves.
    pub(crate) fn bare(appearance: Appearance) -> Self {
        Self::build(appearance, None, 1.0)
    }

    /// A compositor with a real headless backend rendering into a
    /// `canvas`-square framebuffer, which [`Harness::render`] draws and reads
    /// back with a real renderer -- `PixmanRenderer`, or `GlesRenderer`
    /// under `SCOOT_TEST_RENDERER=gles` (see [`test_renderer`]).
    pub(crate) fn headless(appearance: Appearance, canvas: i32) -> Self {
        Self::build(appearance, Some(canvas), 1.0)
    }

    /// The same at an output scale other than 1.0: the framebuffer stays
    /// `canvas` physical pixels square while the core arranges in
    /// `canvas / scale` (rounded up) logical ones -- the shape a fractional
    /// `[output] scale` session renders at. Clients that size their buffers
    /// from `wp_fractional_scale_v1` + `wp_viewporter` draw the way real
    /// toolkits do at that scale; clients that don't are upscaled from
    /// scale-1 buffers, exactly like a scale-unaware client on a real
    /// fractional session.
    pub(crate) fn headless_scaled(appearance: Appearance, canvas: i32, scale: f64) -> Self {
        Self::build(appearance, Some(canvas), scale)
    }

    fn build(appearance: Appearance, canvas: Option<i32>, scale: f64) -> Self {
        let renderer = test_renderer();
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            appearance,
            scale,
            renderer,
        )
        .expect("a compositor state with a wayland socket");
        if let Some(canvas) = canvas {
            headless::init(&mut state, canvas, canvas).expect("a headless backend");
            // What was asked for is not evidence of what was built (see
            // `Backend::renderer`): this is what makes a `gles` run of the
            // suite a claim about GLES rather than about whatever the
            // compositor fell back to.
            let primary = state.outputs.primary_id().expect("an output");
            assert_eq!(
                state
                    .backends
                    .get(&primary)
                    .expect("a headless backend")
                    .renderer(),
                renderer,
                "the harness did not get the renderer it asked for"
            );
        }
        Self {
            event_loop,
            state,
            canvas,
            clients: Vec::new(),
        }
    }

    /// Connects another client over a socket pair and runs `script` against it
    /// on its own thread, returning the index its steps are addressed by.
    ///
    /// A socket pair rather than the listening socket: identical per-client
    /// dispatch, and no dependence on which socket name the compositor got.
    ///
    /// More than one client is not a nicety -- taking over an abandoned lock,
    /// or checking that two subscribers stay in step, is by definition
    /// something a *different* connection does.
    pub(crate) fn spawn<F>(&mut self, script: F) -> usize
    where
        F: FnOnce(UnixStream, Receiver<S>, Sender<A>) -> Result<(), String> + Send + 'static,
        S: Send + 'static,
        A: Send + 'static,
    {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        let client = self
            .state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let thread = thread::spawn(move || script(client_end, step_rx, ack_tx));
        self.clients.push(ClientHandle {
            client,
            steps: Some(step_tx),
            acks: ack_rx,
            thread: Some(thread),
        });
        self.clients.len() - 1
    }

    /// The compositor's own handle on a connected client, for turning a
    /// protocol id the client reported back into a server-side resource.
    pub(crate) fn client(&self, index: usize) -> &Client {
        &self.clients[index].client
    }

    /// Runs one step on client 0 to completion, then lets the compositor
    /// settle so anything the step provoked (a configure, a re-layout) has
    /// happened before the test looks.
    pub(crate) fn run(&mut self, step: S) -> A {
        self.run_on(0, step)
    }

    /// The same, naming the client.
    pub(crate) fn run_on(&mut self, index: usize, step: S) -> A {
        self.send_step(index, step);
        let ack = self.wait_for_ack(index);
        self.settle();
        ack
    }

    /// Sends a step *without* waiting for it to be acknowledged.
    ///
    /// For the handful of cases that need the compositor observed while a
    /// request is still outstanding -- a `lock` whose answer must not have
    /// arrived yet. Every other caller wants [`Harness::run_on`].
    pub(crate) fn send_step(&mut self, index: usize, step: S) {
        self.clients[index]
            .steps
            .as_ref()
            .expect("the step channel")
            .send(step)
            .expect("the client thread is still running");
    }

    /// Dispatches until `index` acknowledges its outstanding step.
    pub(crate) fn wait_for_ack(&mut self, index: usize) -> A {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.clients[index].acks.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => {
                    self.client_died(index, "a client step acknowledgement")
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for client {index}; the compositor stopped serving"
            );
            self.dispatch_once();
        }
    }

    /// Dispatches until `channel` produces a value, for whatever a client
    /// reports outside the step protocol -- the ids of the surfaces it made
    /// before the first step, say.
    pub(crate) fn wait_for<T>(&mut self, index: usize, channel: &Receiver<T>, what: &str) -> T {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match channel.try_recv() {
                Ok(value) => return value,
                Err(TryRecvError::Disconnected) => self.client_died(index, what),
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the compositor stopped serving"
            );
            self.dispatch_once();
        }
    }

    /// A client that died instead of answering is reported with *its own*
    /// error -- the protocol error it provoked, usually -- rather than as a
    /// timeout, which costs [`PATIENCE`] and says nothing.
    ///
    /// A disconnected channel is evidence the client is on its way out, not
    /// proof it already has: a client that reports something over a
    /// secondary channel and then drops it while still running its main step
    /// loop looks the same from here. So this waits for the thread itself,
    /// dispatched rather than blocked on, and says so distinctly if the
    /// thread never actually finishes -- see [`Harness::wait_for_thread`].
    fn client_died(&mut self, index: usize, what: &str) -> ! {
        let Some(handle) = self.clients[index].thread.take() else {
            panic!("client {index} stopped while waiting for {what}, and was already joined");
        };
        if !self.wait_for_thread(&handle) {
            panic!(
                "client {index}'s channel disconnected while waiting for {what}, but its \
                 thread never finished -- it is likely still running, not dead"
            );
        }
        let outcome = handle.join().expect("the client thread");
        panic!("client {index} stopped while waiting for {what}: {outcome:?}");
    }

    /// Sends client 0 a step it is expected *not* to survive -- a request the
    /// compositor answers with a protocol error -- and dispatches until the
    /// client thread has gone, handing back its own error.
    ///
    /// The signal is the ack channel's sender dropping: the client script
    /// returns its error instead of acknowledging the step, which disconnects
    /// the receiver here.
    pub(crate) fn run_expecting_disconnect(&mut self, step: S) -> String {
        self.send_step(0, step);
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.clients[0].acks.try_recv() {
                Ok(_) => panic!("the client survived a request that should have been refused"),
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for client 0 to be disconnected"
            );
            self.dispatch_once();
        }
        self.join_disconnected()
    }

    /// Sends client 0 a step that may or may not be refused, and dispatches
    /// until the client either answers it or is gone: its answer, or its own
    /// error.
    ///
    /// For a test that has to look at the compositor *whatever* came of the
    /// step before it asserts on how the step ended -- drawing a frame after
    /// an attack, where the attack, if it got through, is what crashes.
    /// [`Harness::run_expecting_disconnect`] would fail the test on the
    /// surviving client first, and never get as far as the crash.
    pub(crate) fn run_or_disconnect(&mut self, step: S) -> Result<A, String> {
        self.send_step(0, step);
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.clients[0].acks.try_recv() {
                Ok(ack) => return Ok(ack),
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for client 0 to answer or be disconnected"
            );
            self.dispatch_once();
        }
        Err(self.join_disconnected())
    }

    /// Joins client 0, whose ack channel has just disconnected, and hands
    /// back the error it stopped with.
    fn join_disconnected(&mut self) -> String {
        let Some(handle) = self.clients[0].thread.take() else {
            panic!("client 0 disconnected as expected, but was already joined");
        };
        assert!(
            self.wait_for_thread(&handle),
            "client 0's ack channel disconnected, but its thread never finished"
        );
        let error = handle
            .join()
            .expect("the client thread")
            .expect_err("the client should have stopped with the protocol error it provoked");
        self.settle();
        error
    }

    /// Disconnects a client and waits for the compositor to notice -- "the
    /// client died", as far as the compositor can tell.
    pub(crate) fn disconnect(&mut self, index: usize) {
        drop(self.clients[index].steps.take());
        if let Some(handle) = self.clients[index].thread.take() {
            assert!(
                self.wait_for_thread(&handle),
                "client {index} never finished after being disconnected"
            );
            handle
                .join()
                .expect("the client thread")
                .expect("the client ran cleanly");
        }
        self.settle();
    }

    /// Dispatches until `handle` finishes, or gives up after [`PATIENCE`] and
    /// says so by returning `false`.
    ///
    /// Shared by every caller that needs to join a client thread outside the
    /// normal step/ack protocol ([`Harness::client_died`],
    /// [`Harness::disconnect`], [`Harness::run_expecting_disconnect`], and
    /// this type's own `Drop` impl): a plain `handle.join()` blocks the
    /// calling thread
    /// outright, and a thread legitimately still running (parked on its own
    /// step channel, say, rather than actually finished) would hang the
    /// caller forever instead of letting it fail loudly. Dispatching while
    /// waiting is also what lets a join that depends on the compositor
    /// processing something first (a disconnect notification, a flush)
    /// actually happen.
    fn wait_for_thread(&mut self, handle: &JoinHandle<Result<(), String>>) -> bool {
        let deadline = Instant::now() + PATIENCE;
        while !handle.is_finished() && Instant::now() < deadline {
            let _ = self
                .event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state);
        }
        handle.is_finished()
    }

    /// A few dispatch cycles with nothing outstanding, so in-flight protocol
    /// traffic in both directions has been processed.
    pub(crate) fn settle(&mut self) {
        for _ in 0..10 {
            self.event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut self.state)
                .expect("a compositor dispatch");
        }
        let _ = self.state.display_handle.flush_clients();
    }

    /// Dispatches for `duration` without asking for anything, so the frame
    /// timer gets to run on its own -- the only way to test that the
    /// compositor redraws *by itself*.
    pub(crate) fn tick(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            self.dispatch_once();
        }
    }

    fn dispatch_once(&mut self) {
        self.event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut self.state)
            .expect("a compositor dispatch");
    }

    /// Renders a frame and hands back its raw BGRA pixels.
    pub(crate) fn render(&mut self) -> Vec<u8> {
        self.state.request_render();
        self.state.render();
        self.pixels()
    }

    /// Reads the primary output's framebuffer back *without* rendering first
    /// -- what a screenshot would see of whatever is already there, which is
    /// how a test asserts on the frame the compositor drew by itself.
    pub(crate) fn pixels(&mut self) -> Vec<u8> {
        self.pixels_of(
            self.state
                .outputs
                .primary_id()
                .expect("a headless harness has an output"),
        )
    }

    /// Reads output `id`'s framebuffer back without rendering first.
    pub(crate) fn pixels_of(&mut self, id: scoot_core::OutputId) -> Vec<u8> {
        let canvas = self.canvas.expect("a headless harness has a framebuffer");
        let backend = self.state.backends.get_mut(&id).expect("a backend");
        assert_eq!(
            backend.size(),
            (canvas, canvas),
            "the harness reads the whole framebuffer back, so its size must be the canvas"
        );
        backend
            .capture(<[u8]>::to_vec)
            .expect("a framebuffer readback")
    }
}

impl<S, A> Drop for Harness<S, A> {
    /// Closes every step channel, which is what ends each client script's
    /// loop.
    ///
    /// Deliberately does *not* join while unwinding: a compositor-side panic
    /// leaves the client thread blocked in a round trip whose answer will
    /// never come (the server end of its socket outlives this `Drop`, so it
    /// sees no EOF either), and joining there turns a failing test into a hung
    /// one -- which is exactly what happened while writing an early version
    /// of `layer_shell`'s oversized-layer test, and cost the real bug
    /// `dispatch.rs`'s `reject_unrepresentable_layer_size` now guards against
    /// a diagnosis (the test hung instead of failing, on the client side of
    /// the very panic it was about to catch). The threads are released a
    /// moment later, when `state` -- and with it the server end of every
    /// socket -- drops.
    ///
    /// Otherwise each client is given [`PATIENCE`] to finish, *dispatched*
    /// rather than blocked on, because a client mid-round-trip only completes
    /// while the compositor runs. One that still has not finished is left
    /// detached rather than joined, for the same reason: a blocking join on a
    /// stuck thread would hang the suite instead of failing it.
    fn drop(&mut self) {
        for client in &mut self.clients {
            drop(client.steps.take());
        }
        if thread::panicking() {
            return;
        }
        for index in 0..self.clients.len() {
            let Some(handle) = self.clients[index].thread.take() else {
                continue;
            };
            if !self.wait_for_thread(&handle) {
                eprintln!("the test client {index} never finished");
                continue;
            }
            if let Ok(Err(error)) = handle.join() {
                // Printed, not asserted: a panicking `Drop` while another
                // assertion is already unwinding aborts the process and hides
                // the real failure.
                eprintln!("the test client {index} failed: {error}");
            }
        }
    }
}

/// Round-trips a client's queue until the compositor has answered, or gives
/// up.
///
/// The client-side counterpart to [`Harness::wait_for_ack`]: one round trip is
/// not enough and cannot be made enough, because a configure is sent when the
/// compositor's layout says so, which may be a dispatch cycle or two after the
/// request that provoked it. See [`CLIENT_PATIENCE`] for why this is a
/// deadline rather than a round-trip count.
pub(crate) fn wait_for<T, C>(
    queue: &mut EventQueue<C>,
    client: &mut C,
    what: &str,
    ready: impl Fn(&C) -> Option<T>,
) -> Result<T, String> {
    let deadline = Instant::now() + CLIENT_PATIENCE;
    loop {
        queue.roundtrip(client).map_err(|e| e.to_string())?;
        if let Some(value) = ready(client) {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(format!("the compositor never sent {what}"));
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// Runs `f` with compositor logs captured, handing back what it returned
/// plus everything logged on this thread while it ran.
///
/// Scoped (`with_default`), not global: every dispatch a [`Harness`] drives
/// runs on the test's own thread, so everything the compositor logs while
/// handling a client lands here -- and a scoped subscriber cannot collide
/// with another test's under `cargo test` the way a second global default
/// would.
///
/// Wrap the whole test, the [`Harness`] included, when anything logged
/// inside might run under a span created before it: Smithay keeps some
/// (the keyboard's, for one), and under `cargo test` -- where another
/// thread's capture has marked every callsite interesting -- a span made
/// outside a capture gets the no-subscriber placeholder id, which
/// `tracing-subscriber`'s registry panics on when it is entered inside
/// one. nextest, one process per test, never shows it.
pub(crate) fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
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
        // `init_logging`), but some tests assert on `debug!` handler lines
        // (the XWayland refusals are the designed answer, not an anomaly,
        // so they log below the production floor).
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let result = tracing::subscriber::with_default(subscriber, f);
    let logs = String::from_utf8(buffer.0.lock().expect("the log buffer").clone())
        .expect("compositor logs are UTF-8");
    (result, logs)
}

/// A minimal session-lock client: binds `ext_session_lock_manager_v1`, takes
/// the lock, flushes the request onto the wire, acks, then parks holding the
/// lock object until the harness drops it.
///
/// Shared because lock-gated behavior needs a *real* lock in more than one
/// suite: `is_locked` is defined by a live lock object, not a settable flag,
/// so neither the SIGHUP-under-lock test nor the reload-under-lock policy
/// test can fake one. The script shape fits [`Harness::spawn`] directly
/// (`Harness<(), ()>`). Dropped, never unlocked, at the end -- an abandoned
/// lock stays locked by design, which is what those tests assert under.
pub(crate) struct Locker {
    manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for Locker {
    fn event(
        client: &mut Self,
        registry: &wayland_client::protocol::wl_registry::WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        qh: &wayland_client::QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface.as_str() == "ext_session_lock_manager_v1"
        {
            client.manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl wayland_client::Dispatch<ext_session_lock_manager_v1::ExtSessionLockManagerV1, ()> for Locker {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        _: ext_session_lock_manager_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for Locker {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        _: ext_session_lock_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

/// The [`Locker`] client as a [`Harness::spawn`] script: lock, flush, ack,
/// then park holding the lock until the harness ends the script.
pub(crate) fn locker(
    stream: UnixStream,
    steps: Receiver<()>,
    acks: Sender<()>,
) -> Result<(), String> {
    use wayland_client::Connection;
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut client = Locker { manager: None };
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let manager = client
        .manager
        .take()
        .ok_or("no ext_session_lock_manager_v1")?;
    let _lock = manager.lock(&qh, ());
    queue.flush().map_err(|e| e.to_string())?;
    acks.send(()).map_err(|e| e.to_string())?;
    // Parked: the lock object lives until the harness ends this script.
    while steps.recv().is_ok() {}
    Ok(())
}

/// Probes for the autostart spawn-delta policy: autostart entries are
/// whitespace-split action strings, so no `sh -c` probe survives them --
/// `spawn touch <path>` is the whole probe vocabulary, and the path must be
/// whitespace-free to be expressible at all (the same limitation
/// `docs/configuration.md` documents for every spawn entry). Requires a real
/// `touch`, like the suites that already spawn require a real `sh`.
///
/// A fresh `spawn touch` target: `temp_dir` joined with a space-free name
/// (asserted, not assumed -- a space would silently split the entry into two
/// arguments and the probe would touch the wrong path).
pub(crate) fn marker_path(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "scoot-reload-{}-{}-{tag}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock")
            .as_nanos()
    ));
    assert!(
        !path.to_string_lossy().contains(char::is_whitespace),
        "the marker path must survive the autostart whitespace split: {path:?}"
    );
    let _ = std::fs::remove_file(&path);
    path
}

/// The autostart entry text for a `spawn touch` of `path`.
pub(crate) fn touch_entry(path: &std::path::Path) -> String {
    format!("spawn touch {}", path.display())
}

/// `touch` forks before the reload that spawned it returns, so a spawn lands
/// in milliseconds; ten seconds of quiet is the proof it never ran.
pub(crate) fn wait_for_marker(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the spawned autostart entry never ran: {path:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = std::fs::remove_file(path);
}

/// Bounded absence: a full second of quiet after a reload that must not have
/// spawned. `touch` would have landed in milliseconds, so this is a proof,
/// not a hope -- but it is a bound, and it says so.
pub(crate) fn assert_marker_never_appears(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        assert!(
            !path.exists(),
            "an autostart entry ran that must not have: {path:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Reads one pixel out of a `width`-pixel-wide BGRA framebuffer.
pub(crate) fn pixel(pixels: &[u8], width: i32, x: i32, y: i32) -> [u8; 4] {
    let index = ((y * width + x) * 4) as usize;
    pixels[index..index + 4].try_into().expect("a BGRA pixel")
}

/// Whether any pixel of the frame is `color` -- position-independent, for
/// surfaces whose exact placement the test does not pin down.
pub(crate) fn contains(pixels: &[u8], color: [u8; 4]) -> bool {
    pixels.chunks_exact(4).any(|pixel| pixel == color)
}

/// Where the first `color` pixel is, scanning in row order.
///
/// For pointing at a surface whose placement the test does not pin down: a
/// popup lands wherever its positioner and the parent's geometry put it, and
/// hard-coding that coordinate would be re-deriving Smithay's own positioner
/// arithmetic in a test -- which would then pass or fail for reasons that
/// have nothing to do with what is under test.
pub(crate) fn find_color(pixels: &[u8], width: i32, color: [u8; 4]) -> Option<(f64, f64)> {
    let index = pixels.chunks_exact(4).position(|pixel| pixel == color)? as i32;
    Some(((index % width) as f64, (index / width) as f64))
}

/// Asserts one pixel of the frame is `expected`, naming what was being drawn.
pub(crate) fn assert_pixel(
    pixels: &[u8],
    width: i32,
    x: i32,
    y: i32,
    expected: [u8; 4],
    what: &str,
) {
    assert_eq!(
        pixel(pixels, width, x, y),
        expected,
        "{what}: wrong pixel at ({x}, {y})"
    );
}
