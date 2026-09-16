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
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use flexwm_core::Config;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, ExportMem};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::utils::Rectangle;
use wayland_client::EventQueue;

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless::{self, Backend};
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
    /// `None` once the test has disconnected this client on purpose. Dropping
    /// it is what ends the client script's `steps.recv()` loop.
    steps: Option<Sender<S>>,
    acks: Receiver<A>,
    /// `None` once the thread has been joined -- by a disconnect, or by the
    /// diagnosis of a client that died mid-step.
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl<S, A> Harness<S, A> {
    /// A compositor with a real headless backend rendering into a
    /// `canvas`-square framebuffer, which [`Harness::render`] draws and reads
    /// back with the real `PixmanRenderer`.
    pub(crate) fn headless(appearance: Appearance, canvas: i32) -> Self {
        Self::build(appearance, Some(canvas))
    }

    fn build(appearance: Appearance, canvas: Option<i32>) -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            appearance,
            1.0,
        )
        .expect("a compositor state with a wayland socket");
        if let Some(canvas) = canvas {
            headless::init(&mut state, canvas, canvas).expect("a headless backend");
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
        self.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");

        let (step_tx, step_rx) = channel();
        let (ack_tx, ack_rx) = channel();
        let thread = thread::spawn(move || script(client_end, step_rx, ack_tx));
        self.clients.push(ClientHandle {
            steps: Some(step_tx),
            acks: ack_rx,
            thread: Some(thread),
        });
        self.clients.len() - 1
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

    /// A client that died instead of answering is reported with *its own*
    /// error -- the protocol error it provoked, usually -- rather than as a
    /// timeout, which costs [`PATIENCE`] and says nothing.
    fn client_died(&mut self, index: usize, what: &str) -> ! {
        let outcome = self.clients[index]
            .thread
            .take()
            .map(|handle| handle.join().expect("the client thread"));
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
        let error = self.clients[0]
            .thread
            .take()
            .map(|handle| handle.join().expect("the client thread"))
            .and_then(Result::err)
            .expect("the client should have stopped with the protocol error it provoked");
        self.settle();
        error
    }

    /// Disconnects a client and waits for the compositor to notice -- "the
    /// client died", as far as the compositor can tell.
    pub(crate) fn disconnect(&mut self, index: usize) {
        drop(self.clients[index].steps.take());
        if let Some(handle) = self.clients[index].thread.take() {
            handle
                .join()
                .expect("the client thread")
                .expect("the client ran cleanly");
        }
        self.settle();
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

    /// Reads the framebuffer back *without* rendering first -- what a
    /// screenshot would see of whatever is already there, which is how a test
    /// asserts on the frame the compositor drew by itself.
    pub(crate) fn pixels(&mut self) -> Vec<u8> {
        let canvas = self.canvas.expect("a headless harness has a framebuffer");
        let backend = self.state.backend.as_mut().expect("a backend");
        let Backend {
            renderer, image, ..
        } = backend;
        let framebuffer = renderer.bind(image).expect("a framebuffer");
        let region = Rectangle::from_size((canvas, canvas).into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
            .expect("a framebuffer readback");
        renderer
            .map_texture(&mapping)
            .expect("mapped pixels")
            .to_vec()
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
    /// one. The threads are released a moment later, when `state` -- and with
    /// it the server end of every socket -- drops.
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
            let deadline = Instant::now() + PATIENCE;
            while !handle.is_finished() && Instant::now() < deadline {
                let _ = self
                    .event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state);
            }
            if !handle.is_finished() {
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
