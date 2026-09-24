//! `--nested` presenting by dma-buf end to end, in process: a real nested
//! scoot (`Host`, `init_on`, the frame path) connected over a socket pair to
//! a second, real scoot acting as its host on its own thread -- the same
//! outer-scoot-as-host arrangement `scripts/nested-dmabuf-bench.sh` runs
//! live, without needing a second process.
//!
//! What it pins that the live run cannot on the dev VM: the host *refusing*
//! a buffer. No host there refuses one (an outer scoot under either renderer,
//! and one forced onto Mesa's software device, all import what scoot
//! allocates), so the mid-session fallback is driven here by a host with no
//! render target, which refuses every import through the asynchronous
//! `create` -- a `failed` event, exactly as a host that cannot import would
//! send.
//!
//! Needs a DRM device GBM can allocate on and a GLES renderer on it; where
//! the nested scoot's startup negotiation finds none (CI, a GPU-less
//! container) the tests say so and assert nothing, like
//! `render/tests/host_copy.rs`.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use scoot_core::Action;
use wayland_client::Connection;

use crate::cli::RendererKind;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::nested::{self, Host};
use crate::compositor::state::ClientState;
use crate::compositor::test_support::{Harness, capture_logs, pixel};

const NESTED_START: i32 = 64;
const HOST_CANVAS: i32 = 160;
const PATIENCE: Duration = Duration::from_secs(10);
/// The one WARN a fallback logs.
const FALLBACK_WARN: &str = "presenting to the host by read-back into wl_shm from now on";

/// Every test here takes `dmabuf/tests.rs`'s mapping lock: the buffers it
/// allocates and imports may be mapped into this process, and under `cargo
/// test` (one process, tests as threads) a mapping appearing or vanishing
/// inside one of those suites' before/after windows would fail them. Free
/// under nextest.
fn exclusive_mappings() -> std::sync::MutexGuard<'static, ()> {
    crate::compositor::dmabuf::tests::exclusive_mappings()
}

enum Command {
    /// Draw a host frame; answer its pixels and the nested window's centre.
    Look,
    /// Step the host's column width: a host-driven resize of the nested
    /// window.
    Resize,
}

enum Reply {
    Looked {
        pixels: Vec<u8>,
        centre: Option<(i32, i32)>,
    },
    Resized,
}

/// The host: an outer scoot on its own thread, dispatching until told to
/// stop. With `refuse`, its render target is taken away before the nested
/// scoot connects, so every dma-buf it is offered is refused (`failed`) --
/// while its feedback, advertised at startup from the renderer it had, still
/// names its device.
fn host(
    server_end: UnixStream,
    refuse: bool,
    commands: Receiver<Command>,
    replies: Sender<Reply>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let appearance = Appearance {
            focus_ring_width: 0,
            background_color: Color::new(0.0, 0.0, 1.0, 1.0),
            ..Appearance::default()
        };
        let mut host: Harness<(), ()> =
            Harness::headless_on(appearance, HOST_CANVAS, RendererKind::Gles);
        if refuse {
            drop(host.state.take_primary_backend());
        }
        host.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("the nested scoot connects");
        loop {
            match commands.try_recv() {
                Ok(Command::Look) => {
                    let pixels = host.render();
                    let centre = host.state.space.elements().next().and_then(|window| {
                        host.state.space.element_bbox(window).map(|bbox| {
                            (bbox.loc.x + bbox.size.w / 2, bbox.loc.y + bbox.size.h / 2)
                        })
                    });
                    if replies.send(Reply::Looked { pixels, centre }).is_err() {
                        return;
                    }
                }
                Ok(Command::Resize) => {
                    host.state.act(Action::CycleColumnWidth);
                    host.settle();
                    if replies.send(Reply::Resized).is_err() {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => host.settle(),
                Err(TryRecvError::Disconnected) => return,
            }
        }
    })
}

/// A nested scoot connected to a fresh host, or `None` (said on stderr) when
/// this machine cannot present by dma-buf at all.
struct Pair {
    nested: Harness<(), ()>,
    commands: Sender<Command>,
    replies: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Pair {
    fn drop(&mut self) {
        // Closing the channel ends the host's loop.
        let (dead, _) = channel();
        self.commands = dead;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn pair(test: &str, refuse: bool) -> Option<Pair> {
    let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
    let (commands, host_commands) = channel();
    let (host_replies, replies) = channel();
    let thread = host(server_end, refuse, host_commands, host_replies);
    let appearance = Appearance {
        focus_ring_width: 0,
        background_color: Color::new(1.0, 0.0, 0.0, 1.0),
        ..Appearance::default()
    };
    let mut nested: Harness<(), ()> =
        Harness::headless_on(appearance, NESTED_START, RendererKind::Gles);
    let conn = Connection::from_socket(client_end).expect("a host connection");
    let handle = nested.event_loop.handle();
    nested::init_on(handle, &mut nested.state, conn, NESTED_START, NESTED_START)
        .expect("the nested backend comes up");
    let pair = Pair {
        nested,
        commands,
        replies,
        thread: Some(thread),
    };
    if !host_of(&pair.nested).may_present_dmabuf_for_test() {
        eprintln!("{test}: skipped -- this machine cannot present to a host by dma-buf");
        return None;
    }
    Some(pair)
}

fn host_of(nested: &Harness<(), ()>) -> &Host {
    nested.state.host.as_ref().expect("a nested session")
}

impl Pair {
    /// One nested frame, then the host's view of it.
    fn frame_and_look(&mut self) -> (Vec<u8>, Option<(i32, i32)>) {
        self.nested.settle();
        self.nested.state.request_render();
        self.nested.state.render();
        self.nested.settle();
        self.commands
            .send(Command::Look)
            .expect("the host is alive");
        match self
            .replies
            .recv_timeout(PATIENCE)
            .expect("the host answers")
        {
            Reply::Looked { pixels, centre } => (pixels, centre),
            Reply::Resized => unreachable!("asked to look"),
        }
    }

    /// The nested scoot's own pixel at the centre of its frame, and its size.
    fn nested_centre(&mut self) -> ([u8; 4], (i32, i32)) {
        let id = self.nested.state.outputs.primary_id().expect("an output");
        let backend = self.nested.state.backends.get_mut(&id).expect("a backend");
        let (width, height) = backend.size();
        let bytes = backend.capture(<[u8]>::to_vec).expect("a capture");
        (pixel(&bytes, width, width / 2, height / 2), (width, height))
    }

    /// Frames until the host shows, at the nested window's centre, the
    /// nested scoot's own centre pixel -- i.e. until a frame has reached the
    /// host and is on its screen. Panics after [`PATIENCE`].
    fn until_the_host_shows_the_frame(&mut self) -> (i32, i32) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let (pixels, centre) = self.frame_and_look();
            let (ours, size) = self.nested_centre();
            if let Some((x, y)) = centre
                && x < HOST_CANVAS
                && y < HOST_CANVAS
                && pixel(&pixels, HOST_CANVAS, x, y) == ours
                && host_of(&self.nested).is_configured()
            {
                return size;
            }
            assert!(
                Instant::now() < deadline,
                "the host never showed the nested frame (presenter {})",
                host_of(&self.nested).presenter_for_test()
            );
        }
    }
}

/// Runs `test` against a fresh pair, with the nested scoot's whole life --
/// built, driven, dropped -- inside [`capture_logs`], and answers what it
/// returned and the logs; `None` (said on stderr) where this machine cannot
/// present by dma-buf. The whole life, not just the driving: the renderer
/// creates tracing spans when it is built and enters them when it is
/// dropped, and a span created under one subscriber and entered under
/// another panics in `tracing-subscriber`'s registry -- which `cargo test`
/// reached (one process, a global subscriber some other test set) and
/// nextest never did.
fn with_pair<T>(
    name: &str,
    refuse: bool,
    test: impl FnOnce(&mut Pair) -> T,
) -> Option<(T, String)> {
    let (answer, logs) = capture_logs(|| {
        let mut pair = pair(name, refuse)?;
        Some(test(&mut pair))
    });
    answer.map(|answer| (answer, logs))
}

/// The whole path against a host that imports: negotiated at startup, the
/// frame on the host's screen, by dma-buf throughout -- and still so after
/// the host resizes the window, which replaces the chain and hands the
/// first frame at the new size over once the host has created it.
#[test]
fn frames_reach_a_host_by_dma_buf_and_follow_its_resize() {
    let _mappings = exclusive_mappings();
    let Some(((), logs)) = with_pair(
        "frames_reach_a_host_by_dma_buf_and_follow_its_resize",
        false,
        |pair| {
            let before = pair.until_the_host_shows_the_frame();
            assert_eq!(host_of(&pair.nested).presenter_for_test(), "dmabuf");

            pair.commands
                .send(Command::Resize)
                .expect("the host is alive");
            assert!(matches!(
                pair.replies.recv_timeout(PATIENCE),
                Ok(Reply::Resized)
            ));
            let deadline = Instant::now() + PATIENCE;
            loop {
                let after = pair.until_the_host_shows_the_frame();
                if after != before {
                    break;
                }
                assert!(Instant::now() < deadline, "the nested window never resized");
            }
            assert_eq!(
                host_of(&pair.nested).presenter_for_test(),
                "dmabuf",
                "a resize keeps the dma-buf path"
            );
        },
    ) else {
        return;
    };
    assert!(
        !logs.contains(FALLBACK_WARN),
        "nothing refused, nothing fell back:\n{logs}"
    );
    // The first frame at a new chain's size is drawn before the host has
    // created that chain; it goes out when the host does, as drawn.
    assert!(
        logs.contains("handed the owed frame to the host without redrawing it"),
        "a new chain's first frame is handed over, not drawn again:\n{logs}"
    );
}

/// A host that refuses the buffers: one WARN, read-back for the rest of the
/// session, and frames keep going out -- never a stranded window, never a
/// second WARN however many refusals arrive (there is one per buffer).
#[test]
fn a_host_refusing_the_buffers_moves_the_session_to_read_back_once() {
    let _mappings = exclusive_mappings();
    let Some(((presenter, still_open), logs)) = with_pair(
        "a_host_refusing_the_buffers_moves_the_session_to_read_back_once",
        true,
        |pair| {
            let deadline = Instant::now() + PATIENCE;
            while host_of(&pair.nested).presenter_for_test() != "shm" {
                assert!(
                    Instant::now() < deadline,
                    "the refusal never moved the session to read-back (presenter {})",
                    host_of(&pair.nested).presenter_for_test()
                );
                pair.nested.settle();
                pair.nested.state.request_render();
                pair.nested.state.render();
            }
            // More frames after the switch: they go out by read-back, and
            // the refusals still in flight for the abandoned chain change
            // nothing.
            for _ in 0..5 {
                pair.nested.settle();
                pair.nested.state.request_render();
                pair.nested.state.render();
            }
            let host = host_of(&pair.nested);
            (
                host.presenter_for_test(),
                host.may_present_dmabuf_for_test(),
            )
        },
    ) else {
        return;
    };
    assert_eq!(presenter, "shm");
    assert!(!still_open, "a refusal is for the rest of the session");
    assert_eq!(
        logs.matches(FALLBACK_WARN).count(),
        1,
        "exactly one WARN for the fallback:\n{logs}"
    );
}

/// A chain that cannot grow when the host holds its only buffer -- GBM out
/// of memory, or the fd table exhausted -- must not freeze the window: no
/// `release` or `created` would ever come to hand the waiting frame over.
/// One WARN, read-back for the rest of the session, and the next frame
/// reaches the host that way.
#[test]
fn a_chain_that_cannot_grow_moves_the_session_to_read_back_once() {
    let _mappings = exclusive_mappings();
    let Some((still_open, logs)) = with_pair(
        "a_chain_that_cannot_grow_moves_the_session_to_read_back_once",
        false,
        |pair| {
            let before = pair.until_the_host_shows_the_frame();
            assert_eq!(host_of(&pair.nested).presenter_for_test(), "dmabuf");
            pair.nested
                .state
                .host
                .as_mut()
                .expect("a nested session")
                .fail_growth_for_test();
            // A resize starts a new chain with one buffer: its first frame
            // is handed over into it, and the next finds it held and must
            // grow.
            pair.commands
                .send(Command::Resize)
                .expect("the host is alive");
            assert!(matches!(
                pair.replies.recv_timeout(PATIENCE),
                Ok(Reply::Resized)
            ));
            let deadline = Instant::now() + PATIENCE;
            while host_of(&pair.nested).presenter_for_test() != "shm" {
                assert!(
                    Instant::now() < deadline,
                    "a failed growth never moved the session to read-back (presenter {})",
                    host_of(&pair.nested).presenter_for_test()
                );
                pair.frame_and_look();
            }
            // And frames keep reaching the host, now by read-back, at the
            // new size.
            let after = pair.until_the_host_shows_the_frame();
            assert_ne!(after, before, "the resize was followed");
            host_of(&pair.nested).may_present_dmabuf_for_test()
        },
    ) else {
        return;
    };
    assert!(
        !still_open,
        "a failed growth is for the rest of the session"
    );
    assert_eq!(
        logs.matches(FALLBACK_WARN).count(),
        1,
        "exactly one WARN for the fallback:\n{logs}"
    );
    assert!(
        logs.contains("could not add a host buffer: injected growth failure"),
        "the WARN names the cause:\n{logs}"
    );
}
