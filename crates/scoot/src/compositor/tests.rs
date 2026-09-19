//! Tests for [`post_dispatch`], the event loop's own end-of-wakeup step.
//!
//! What is under test is *when a queued wayland message leaves the
//! compositor*, which no unit test of a predicate can observe: it needs a real
//! [`State`] with a real client whose outgoing buffer can hold something, and a
//! real `EventLoop` actually completing a dispatch cycle. So these drive
//! [`smithay::reexports::calloop::EventLoop::run`] -- the same call
//! [`run`](super::run) makes, with the same callback -- and stop it from inside
//! a source, which is the only way to return from a `run` whose timeout is
//! `None`.
//!
//! The two tests are a pair: the second one is what keeps the first
//! non-vacuous. A dispatch cycle on its own leaves the message queued (the bug
//! as it shipped); the same cycle with `post_dispatch` after it delivers it.
//!
//! Like `dispatch/tests.rs`, `cursor/tests.rs` and `ipc/connection/tests.rs`,
//! these need a writable `$XDG_RUNTIME_DIR`: [`State::new`] binds a real
//! wayland listening socket, which nothing here connects to but which is
//! created either way.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use scoot_core::Config;
use smithay::output::{Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::Display;

use super::decorations::Appearance;
use super::keybindings::Keybindings;
use super::state::ClientState;
use super::{State, post_dispatch};

/// How long the handshake may take before the compositor counts as not
/// answering. Generous: a debug build on a VM.
const PATIENCE: Duration = Duration::from_secs(20);

/// How many dispatch cycles count as "nothing more is coming". Each one costs
/// at most a millisecond with no source ready.
const SILENCE_ROUNDS: usize = 20;

/// One dispatch cycle, with a timeout small enough that a test waiting for
/// nothing does not pay for it.
fn pump(event_loop: &mut EventLoop<'static, State>, state: &mut State) {
    event_loop
        .dispatch(Some(Duration::from_millis(1)), state)
        .expect("a compositor dispatch");
}

/// Reads and discards whatever has arrived on `client`, returning how much that
/// was. Free rather than a method so [`Probe::new`] can use it before there is
/// a `Probe` to call it on.
fn drain(client: &mut UnixStream) -> usize {
    let mut chunk = [0u8; 4096];
    let mut total = 0;
    loop {
        match client.read(&mut chunk) {
            Ok(0) => panic!("the compositor disconnected the probe"),
            Ok(count) => total += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return total,
            Err(error) => panic!("the probe could not read: {error}"),
        }
    }
}

/// A real compositor with one wayland client that has a `wl_registry` bound,
/// and one message queued for it that nothing has flushed.
///
/// The message is a `wl_registry.global` from a newly created output, for the
/// same reason `ipc/connection/tests.rs` uses that trick for its own flush
/// test: it puts a message in exactly the position an injected keystroke's
/// `wl_keyboard.key` is in once `input::key` has queued it, without needing a
/// focused toplevel and a real toolkit to get there. What is under test is the
/// flush, not what filled the buffer. (That module's own probe is not reused
/// here: it is built around driving the loop with `dispatch`, and driving it
/// with `run` is the whole point of these tests.)
struct Probe {
    event_loop: EventLoop<'static, State>,
    state: State,
    /// The client end of the connection, non-blocking so a read can say
    /// "nothing arrived" instead of parking the single thread the compositor
    /// also runs on.
    client: UnixStream,
    /// Held only so its global stays alive -- dropping the `Output` would take
    /// the queued event's reason to exist with it.
    #[allow(dead_code)]
    output: Output,
}

impl Probe {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
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

        // A socket pair rather than the real listening socket: it skips any
        // dependence on which name that got, while still going through the
        // same per-client dispatch a real connection does.
        let (server, mut client) = UnixStream::pair().expect("a socket pair");
        state
            .display_handle
            .insert_client(server, Arc::new(ClientState::default()))
            .expect("an inserted wayland client");
        client
            .set_nonblocking(true)
            .expect("a non-blocking test client");

        // `wl_display.get_registry(new_id)`, on the wire: the object the
        // request is for, then the message's length and opcode packed into one
        // word, then the one argument. Native byte order, as wayland's wire
        // format is defined. Written by hand because a registry is all this
        // needs -- a protocol implementation on the client side would be more
        // code than the thing being tested.
        const WL_DISPLAY: u32 = 1;
        const GET_REGISTRY: u32 = 1;
        const REGISTRY_ID: u32 = 2;
        const LENGTH: u32 = 12;
        let mut request = [0u8; LENGTH as usize];
        request[..4].copy_from_slice(&WL_DISPLAY.to_ne_bytes());
        request[4..8].copy_from_slice(&((LENGTH << 16) | GET_REGISTRY).to_ne_bytes());
        request[8..].copy_from_slice(&REGISTRY_ID.to_ne_bytes());
        client
            .write_all(&request)
            .expect("the handshake is written");

        // The startup globals come back through the wayland display's own
        // event source, which flushes after dispatching (see `State::listen`)
        // -- so this loop is not relying on the behavior under test.
        let deadline = Instant::now() + PATIENCE;
        while drain(&mut client) == 0 {
            assert!(
                Instant::now() < deadline,
                "the compositor never answered the probe's get_registry"
            );
            pump(&mut event_loop, &mut state);
        }
        // Let the handshake finish completely and the loop settle, so anything
        // seen after this point was flushed by the cycle under test.
        for _ in 0..SILENCE_ROUNDS {
            pump(&mut event_loop, &mut state);
            drain(&mut client);
        }

        // Only now is something queued, and nothing has flushed since.
        let output = new_output("flush-probe");
        output.create_global::<State>(&state.display_handle);
        Self {
            event_loop,
            state,
            client,
            output,
        }
    }

    fn pump(&mut self) {
        pump(&mut self.event_loop, &mut self.state);
    }

    fn drain(&mut self) -> usize {
        drain(&mut self.client)
    }

    /// Runs the loop exactly as [`super::run`] does -- `run(None, ..,
    /// post_dispatch)` -- for one cycle, by arming a source that stops it.
    ///
    /// `Timer::immediate` is already due, so the `dispatch(None)` inside `run`
    /// returns straight away having run that callback; `run` then calls
    /// `post_dispatch` and only afterwards notices the stop signal
    /// (`loop_logic.rs`: `while !stop { dispatch(timeout)?; cb(data); }`). So
    /// the flush under test does happen, exactly once, and this returns.
    fn run_one_cycle(&mut self) {
        self.state
            .loop_handle
            .insert_source(Timer::immediate(), |_, _, state: &mut State| {
                state.loop_signal.stop();
                TimeoutAction::Drop
            })
            .expect("the stop timer");
        self.event_loop
            .run(None, &mut self.state, post_dispatch)
            .expect("the event loop runs");
    }
}

fn new_output(name: &str) -> Output {
    Output::new(
        name.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "scoot".into(),
            model: name.into(),
            serial_number: "0".into(),
        },
    )
}

#[test]
fn a_queued_client_message_goes_out_at_the_end_of_the_dispatch_cycle() {
    // The fix, in the shape the bug had: something queued a wayland message
    // for a client, nothing else on the way out flushed it (no render, because
    // nothing marked the screen dirty; no IPC request, because the input came
    // from libinput), and the client is entitled to it now rather than
    // whenever an unrelated wakeup next happens to flush.
    let mut probe = Probe::new();
    probe.run_one_cycle();
    assert!(
        probe.drain() > 0,
        "the message queued before the dispatch cycle never reached the client"
    );
}

#[test]
fn a_dispatch_cycle_on_its_own_leaves_a_queued_message_unsent() {
    // The bug as it shipped, and what keeps the test above honest: without the
    // end-of-wakeup flush, dispatching does not deliver this message however
    // long the loop turns, because no source it wakes for is the one that
    // queued it. If this ever starts failing, something else began flushing
    // and the test above would pass whether `post_dispatch` ran or not.
    let mut probe = Probe::new();
    for _ in 0..SILENCE_ROUNDS {
        probe.pump();
        assert_eq!(
            probe.drain(),
            0,
            "a dispatch cycle with no post-dispatch flush delivered the message anyway"
        );
    }
}
