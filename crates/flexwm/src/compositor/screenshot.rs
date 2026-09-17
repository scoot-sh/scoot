//! Reading the framebuffer back out as a PNG, without stalling the event loop.
//!
//! A `Request::Screenshot` used to cost a full render, a framebuffer
//! read-back and a PNG encode, all on the compositor's only thread -- the one
//! that also runs wayland dispatch, input and every other IPC connection.
//! Each capture stalled all of that for its duration: ~12ms at 1600x1000 in a
//! release build. The per-connection rate limit and the connection cap bound
//! how *often* that stall could happen, not how long one lasted.
//!
//! What moved and what stayed, and why:
//!
//! - **Stays on the event-loop thread: `render()` and the read-back.**
//!   Both touch the renderer and its framebuffer, which belong to the
//!   compositor thread and are not `Send`. There is no way to hand them to
//!   another thread short of restructuring who owns the backend, which is
//!   out of scope. What stays is a memcpy-tempo operation -- render a frame
//!   that usually needs rendering anyway, copy the pixels out -- not the
//!   double-digit-millisecond stall.
//! - **Moves to a worker: the BGRA-to-RGBA swizzle, the PNG encode, and the
//!   reply's framing.** All three are pure CPU over owned bytes, touching
//!   nothing but their input. `encode_png` takes a byte slice and returns
//!   bytes or an error string; it borrows nothing from `State`. The JSON and
//!   base64 framing moves with it so the event-loop thread never touches PNG
//!   bytes at all -- a multi-megabyte screenshot's framed line is handed
//!   back ready to write.
//!
//! The worker is one dedicated thread, not a pool, for one reason: a single
//! FIFO preserves reply order with no sequencing machinery. Jobs are accepted
//! in request order and completions are written in completion order, which for
//! a single worker is the same order -- so two captures from one connection
//! (spaced by the rate limit) always answer in the order they were asked.
//!
//! Reply delivery answers the question the ticket raises -- the requesting
//! connection has long left `serve` by the time the encode finishes -- with
//! the mechanism this socket already has for exactly that shape: the
//! `PendingIdle` hand-off. Dispatch clones the connection's socket (which
//! shares its file status flags, so the clone is non-blocking like the
//! original) and parks the reply-to-be in `State::pending_shots`. The
//! worker's answer comes back over a calloop channel, whose callback runs on
//! the event-loop thread and writes it through that clone with the same
//! [`Outbound`] queue a connection uses -- no second reply channel, and the
//! same framing guarantee (one reply never overtakes another).
//!
//! The consequences, each pinned by a test:
//!
//! - **Ordering.** A connection with a capture in flight answers nothing
//!   else until the capture's reply has gone out: any other request arriving
//!   meanwhile is refused with a retry, the same "refused rather than
//!   delayed" shape the rate limit already has. Without that a pipelined
//!   request's reply would overtake the screenshot's. `flexwm msg` sends one
//!   request per connection and never sees this.
//! - **Disconnect mid-encode.** The parked entry holds no slot and borrows
//!   nothing from the connection; if the peer is gone, the completion write
//!   fails and the entry is dropped. No panic, no wedged worker, nothing
//!   written to a dead connection.
//! - **`wait-idle` is unaffected.** The capture's render runs synchronously
//!   at request time and touches only `needs_render`, never `last_commit` --
//!   which is the clock `wait-idle` watches -- and the encode in flight is
//!   invisible to it: no waiter is parked, nothing is wedged, and a capture
//!   mid-wait neither extends nor shortens the quiet window.
//! - **Bounded memory.** Each in-flight job holds a full frame of pixels
//!   (~6.4 MiB at 1600x1000), so the worker queue is bounded by
//!   [`MAX_IN_FLIGHT_SHOTS`]; past that a capture is refused with a retry
//!   rather than queued without bound.
//! - **Panic isolation.** This workspace builds release with `panic = "abort"`,
//!   under which `catch_unwind` cannot catch anything -- so the isolation is
//!   structural instead: `encode_png` has no panic path to take (checked
//!   arithmetic on client-independent sizes, no indexing past
//!   `chunks_exact`, every fallible call mapped to an error string). The
//!   `catch_unwind` around the job still stands for debug builds, where it
//!   turns a worker panic into an error reply instead of a dead thread. And
//!   if the worker ever does go away entirely, `try_send` fails disconnected
//!   -- which drops the encoder so the next request spawns a fresh one, and
//!   refuses *this* one with a retry rather than hanging it.
//! - **Shutdown with encodes in flight.** The worker is never joined: it
//!   exits on its own when its job channel disconnects, which is what
//!   dropping `State` does. A slow encode can delay nothing.

use std::os::unix::net::UnixStream;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use flexwm_ipc::{Response, Screenshot, encode};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, ExportMem};
use smithay::reexports::calloop::channel::{self, Event as ChannelEvent};
use smithay::utils::Rectangle;

use super::State;
use super::headless::Backend;
use super::ipc::Outbound;

/// How many captures may be accepted-but-undelivered at once, across every
/// connection.
///
/// Each one holds a full frame of raw pixels on its way through the worker
/// (~6.4 MiB at 1600x1000), so this is a memory bound first: four deep is
/// ~26 MiB plus PNG buffers, where the 64-connection cap alone would allow
/// hundreds of megabytes queued behind one thread. It is also a latency
/// bound: the worker clears roughly one capture per ~10ms at that size, so
/// the fourth waiter waits tens of milliseconds, not seconds -- past that a
/// refusal and a retry is cheaper than a queue. Refused, not queued, matching
/// every other bound on this socket: a client that wants a capture retries
/// in a few milliseconds.
pub(super) const MAX_IN_FLIGHT_SHOTS: usize = 4;

/// How long a delivered screenshot reply may go without a byte leaving before
/// it is given up on.
///
/// The same window as `connection.rs`'s `WRITE_STALL_TIMEOUT`, for the same
/// reason: a client draining a multi-megabyte screenshot slowly is making
/// progress and is never given up on, however long it takes -- only a peer
/// that has taken nothing at all in this long is treated as gone. A separate
/// constant rather than a shared one because the test-shortening `Limits`
/// does not reach here (`State` holds no `Limits`); the suites drive
/// [`State::settle_shots_at`] with an explicit clock instead.
const SHOT_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Raw pixels as the framebuffer holds them: little-endian BGRA (the same
/// layout `wl_shm`'s own `Argb8888` uses), owned so the renderer's mapping
/// can be released before the encode starts.
pub struct RawCapture {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

/// One capture on its way through the worker.
struct ShotJob {
    conn: u64,
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

/// A finished encode, back on the event-loop thread.
///
/// The fully framed reply line -- JSON, base64 and trailing newline --
/// encoded on the worker, so the event-loop thread never touches PNG bytes
/// at all: not for the encode, and not for framing it either. `Err` only
/// when framing itself failed (serde cannot fail on these shapes in
/// practice); the message is then framed on the loop instead, where it is a
/// few dozen bytes rather than megabytes.
struct ShotDone {
    conn: u64,
    result: Result<String, String>,
}

/// The encode worker's half of `State`: both ends it needs. The receiving
/// ends live elsewhere -- the job receiver on the worker thread, the
/// completion receiver in the event loop -- so dropping this (with `State`)
/// disconnects both, which is what lets the worker exit on its own: nothing
/// ever joins it, so shutdown never waits on an encode.
pub struct Encoder {
    jobs: SyncSender<ShotJob>,
}

/// A capture accepted but not yet fully written out: the connection it came
/// from (by the id `accept` assigned it) and a clone of that connection's
/// socket to write the answer through.
///
/// Holds no connection slot: the connection it came from is still live and
/// holding its own. Holds nothing borrowed: a client disconnecting
/// mid-encode just makes the completion write fail, and the entry is
/// dropped.
///
/// Two states, told apart by `outbound`: empty while the encode is still
/// running (the completion channel delivers), non-empty while a reply the
/// socket would not take in one write drains (the frame tick services it).
/// The entry exists in both, so the ordering gate (`shot_inflight`) covers
/// the whole window from dispatch to the last byte going out.
pub struct PendingShot {
    conn: u64,
    stream: UnixStream,
    outbound: Outbound,
    /// Bytes that had gone out the last time progress was checked, and when.
    /// See `connection.rs`'s stall deadline for why the give-up is measured
    /// in lack of *progress* rather than total time.
    progress: u64,
    last_progress: Instant,
}

/// Whether `start_screenshot` took a capture, refused one, or failed one.
pub enum ShotStart {
    /// On the worker; the reply arrives later through the completion
    /// channel. The event-loop cost (render and read-back) is spent.
    Dispatched,
    /// The capture itself failed (no backend, bind/copy/map error). The
    /// render still ran first, so the expensive part was spent either way --
    /// the caller stamps its rate limit the same as for `Dispatched`.
    Failed(String),
    /// Refused before any render work: another capture from this connection
    /// still in flight, or the worker queue full. Nothing was spent, so the
    /// caller must not stamp its rate limit -- a refusal is not a capture.
    Refused(String),
}

impl State {
    /// Whether `conn` has a capture anywhere between dispatch and the last
    /// byte of its reply going out. While this holds, `Connection::serve`
    /// refuses that connection's other requests rather than letting their
    /// replies overtake the screenshot's.
    pub fn shot_inflight(&self, conn: u64) -> bool {
        self.pending_shots.iter().any(|shot| shot.conn == conn)
    }

    /// Renders anything outstanding, then captures the screen's raw pixels.
    ///
    /// Synchronous, on the event-loop thread: both steps touch the renderer
    /// and its framebuffer, which are not `Send`. What this does *not* do is
    /// the swizzle or the PNG encode -- those are [`encode_png`], pure over
    /// the returned bytes, on the worker.
    pub fn capture_pixels(&mut self) -> Result<RawCapture, String> {
        self.render();
        let Some(mut backend) = self.backend.take() else {
            return Err("no backend to capture".into());
        };
        let captured = read_back(&mut backend);
        self.backend = Some(backend);
        captured
    }

    /// Takes a screenshot request from connection `conn`: captures
    /// synchronously, hands the pixels to the encode worker, and parks the
    /// reply-to-be until the completion channel delivers it.
    ///
    /// `stream` is a clone of the connection's socket, sharing its file
    /// status flags (non-blocking, like the original -- the same sharing
    /// `PendingIdle` relies on), through which the answer is written when it
    /// is ready.
    pub fn start_screenshot(&mut self, conn: u64, stream: UnixStream) -> ShotStart {
        if self.shot_inflight(conn) {
            return ShotStart::Refused(
                "a screenshot from this connection is still being encoded; \
                 retry in a few milliseconds"
                    .to_string(),
            );
        }
        if self.pending_shots.len() >= MAX_IN_FLIGHT_SHOTS {
            return ShotStart::Refused(format!(
                "the screenshot encoder is busy ({} captures already in flight); \
                 retry in a few milliseconds",
                self.pending_shots.len()
            ));
        }
        let capture = match self.capture_pixels() {
            Ok(capture) => capture,
            Err(message) => return ShotStart::Failed(message),
        };
        if self.ensure_encoder().is_err() {
            // Thread spawn or event-loop insert failed: refuse rather than
            // encode inline, which would stall every other client for the
            // full ~12ms this change exists to remove. Loud, because a
            // machine that cannot spawn one thread is in serious trouble.
            tracing::warn!("could not start the screenshot encoder; refusing the capture");
            return ShotStart::Refused(
                "the screenshot encoder is unavailable; retry in a few milliseconds".to_string(),
            );
        };
        let job = ShotJob {
            conn,
            width: capture.width,
            height: capture.height,
            bgra: capture.bgra,
        };
        // Cannot be full: every accepted job does exactly one `try_send` and
        // leaves `pending_shots` only on delivery, so the channel occupancy
        // is bounded by `pending_shots.len()`, which was just checked
        // against the channel's own bound. A `Full` here would mean that
        // accounting broke; refusing is safe either way. `Disconnected`
        // means the worker went away, so the encoder is dropped and the next
        // request spawns a fresh one -- this one is still just a refusal,
        // never a hang.
        match self
            .screenshot_encoder
            .as_ref()
            .expect("the encoder was just ensured")
            .jobs
            .try_send(job)
        {
            Ok(()) => {
                let now = Instant::now();
                self.pending_shots.push(PendingShot {
                    conn,
                    stream,
                    outbound: Outbound::default(),
                    progress: 0,
                    last_progress: now,
                });
                ShotStart::Dispatched
            }
            Err(TrySendError::Full(_)) => ShotStart::Refused(
                "the screenshot encoder is busy; retry in a few milliseconds".to_string(),
            ),
            Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("the screenshot encoder went away; refusing the capture");
                self.screenshot_encoder = None;
                ShotStart::Refused(
                    "the screenshot encoder is unavailable; retry in a few milliseconds"
                        .to_string(),
                )
            }
        }
    }

    /// Spawns the encode worker and wires its completions into the event
    /// loop, once. Lazy -- a session that never screenshots pays for no
    /// thread and no event source -- and idempotent afterwards.
    fn ensure_encoder(&mut self) -> Result<(), String> {
        if self.screenshot_encoder.is_some() {
            return Ok(());
        }
        let (job_tx, job_rx) = sync_channel::<ShotJob>(MAX_IN_FLIGHT_SHOTS);
        let (done_tx, done_rx) = channel::channel::<ShotDone>();
        thread::Builder::new()
            .name("flexwm-shot".to_string())
            .spawn(move || run_encoder(job_rx, done_tx))
            .map_err(|error| error.to_string())?;
        self.loop_handle
            .insert_source(done_rx, |event, _, state: &mut State| {
                match event {
                    ChannelEvent::Msg(done) => state.finish_shot(done),
                    // The worker exited, which only happens when the job
                    // channel disconnected -- i.e. this `State` (and its
                    // encoder) is already gone, so there is nothing to clean
                    // up here. A worker that died any other way shows up as
                    // `Disconnected` on the next `try_send`, which respawns
                    // it. Either way this event needs no action.
                    ChannelEvent::Closed => {}
                }
            })
            .map_err(|error| error.to_string())?;
        self.screenshot_encoder = Some(Encoder { jobs: job_tx });
        Ok(())
    }

    /// Writes a finished encode out through its connection's cloned socket.
    ///
    /// Runs on the event-loop thread, as the completion channel's callback.
    /// A reply the socket takes in full removes its entry then and there;
    /// whatever is left waits in the entry's [`Outbound`] for
    /// [`State::settle_shots`]. A write error means the peer is gone, and the
    /// entry is dropped -- the work is discarded, cleanly.
    fn finish_shot(&mut self, done: ShotDone) {
        let Some(index) = self
            .pending_shots
            .iter()
            .position(|shot| shot.conn == done.conn)
        else {
            // Unreachable by construction: an entry leaves `pending_shots`
            // only here and in `settle_shots`'s give-up, and each job
            // produces exactly one completion. Warn rather than panic all
            // the same -- a compositor must not abort over bookkeeping.
            tracing::warn!(
                conn = done.conn,
                "a screenshot finished with no pending capture; dropping it"
            );
            return;
        };
        // Framed on the worker (see `ShotDone`); only the fallback -- a few
        // dozen bytes -- is ever encoded here.
        let line = match done.result {
            Ok(line) => line,
            Err(message) => match encode(&Response::error(&message)) {
                Ok(line) => line,
                // `Response::Error` is a `String`; serde cannot fail on it.
                // Nothing to answer with if it somehow did.
                Err(_) => {
                    self.pending_shots.remove(index);
                    return;
                }
            },
        };
        let now = Instant::now();
        let PendingShot {
            stream,
            outbound,
            progress,
            last_progress,
            ..
        } = &mut self.pending_shots[index];
        *last_progress = now;
        // Disjoint fields: the socket is borrowed, the queue is mutated --
        // the same split `Connection::reply` and `PendingIdle::advance`
        // use. `&UnixStream` is itself a `Write`, so no extra fd is needed
        // to write through the clone.
        match outbound.send(&mut &*stream, line) {
            Ok(()) => {
                *progress = outbound.total_sent();
                if outbound.is_empty() {
                    self.pending_shots.remove(index);
                } else {
                    // Part-written: the frame timer retries it. It may have
                    // dropped itself on a quiet screen, since nothing about
                    // this capture marked it dirty.
                    self.ensure_ticking();
                }
            }
            Err(error) => {
                tracing::debug!(%error, "dropped a screenshot reply its client never read");
                self.pending_shots.remove(index);
            }
        }
    }

    /// Pushes out screenshot replies the socket would not take in one write.
    pub fn settle_shots(&mut self) {
        self.settle_shots_at(Instant::now());
    }

    /// [`State::settle_shots`] against an explicit clock, so the suites can
    /// wait out the give-up window without waiting it out.
    pub fn settle_shots_at(&mut self, now: Instant) {
        if self.pending_shots.is_empty() {
            return;
        }
        self.pending_shots.retain_mut(|shot| {
            // Still encoding: the completion channel delivers, not the tick.
            if shot.outbound.is_empty() {
                return true;
            }
            let PendingShot {
                stream,
                outbound,
                progress,
                last_progress,
                ..
            } = shot;
            match outbound.flush(&mut &*stream) {
                Err(error) => {
                    tracing::debug!(%error, "dropped a screenshot reply its client never read");
                    false
                }
                Ok(()) => {
                    if outbound.is_empty() {
                        false
                    } else {
                        let sent = outbound.total_sent();
                        if sent != *progress {
                            *progress = sent;
                            *last_progress = now;
                            true
                        } else if now.duration_since(*last_progress) >= SHOT_WRITE_TIMEOUT {
                            tracing::warn!(
                                pending = outbound.pending(),
                                "gave up writing a screenshot reply: the client stopped reading"
                            );
                            false
                        } else {
                            true
                        }
                    }
                }
            }
        });
    }

    /// Whether any parked capture still has reply bytes to push out -- which
    /// is what keeps the frame timer alive until they are gone (see
    /// `frame_tick`). A capture still encoding needs nothing from the tick;
    /// its completion channel delivers.
    pub fn shots_draining(&self) -> bool {
        self.pending_shots
            .iter()
            .any(|shot| !shot.outbound.is_empty())
    }

    /// Drops the encoder without touching anything parked, so the next
    /// request respawns it. Only a test uses this: the worker exits when its
    /// job channel disconnects, which is what proves a respawned worker
    /// serves again rather than hanging every screenshot after it.
    #[cfg(test)]
    pub fn drop_encoder(&mut self) {
        self.screenshot_encoder = None;
    }

    /// Parks a capture that will never complete, filed under `conn`. Only a
    /// test uses this: it holds the ordering gate (and the worker-queue
    /// bound) open deterministically, without depending on how fast the real
    /// worker encodes -- which is the only way to test "refused while in
    /// flight" without a race.
    #[cfg(test)]
    pub fn park_test_shot(&mut self, conn: u64) {
        let (a, _b) = UnixStream::pair().expect("a socket pair");
        let now = Instant::now();
        self.pending_shots.push(PendingShot {
            conn,
            stream: a,
            outbound: Outbound::default(),
            progress: 0,
            last_progress: now,
        });
    }

    /// How many captures are accepted but not yet fully written out. Only a
    /// test asks: production answers that question per connection, through
    /// [`State::shot_inflight`].
    #[cfg(test)]
    pub fn pending_shot_count(&self) -> usize {
        self.pending_shots.len()
    }
}

fn read_back(backend: &mut Backend) -> Result<RawCapture, String> {
    let (width, height) = backend.size;
    if width <= 0 || height <= 0 {
        return Err(format!("no pixels to capture at {width}x{height}"));
    }
    let region = Rectangle::from_size((width, height).into());
    let Backend {
        renderer, image, ..
    } = backend;

    let framebuffer = renderer.bind(image).map_err(|e| e.to_string())?;
    let mapping = renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
        .map_err(|e| e.to_string())?;
    let pixels = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;

    // Owned before returning: `map_texture`'s contract is a read-only view
    // into the renderer's own mapping, not a buffer that can be sent to
    // another thread. This copy is the one part of the read-back that cannot
    // move off the event-loop thread with the encode.
    Ok(RawCapture {
        width,
        height,
        bgra: pixels.to_vec(),
    })
}

/// Swizzles raw framebuffer pixels to RGBA and encodes them as a PNG.
///
/// Pure: no `State`, no renderer, no Wayland -- this is what runs on the
/// worker thread. Written without a panic path on purpose (see the module
/// doc): release builds abort on panic, so `catch_unwind` cannot isolate one
/// there. Sizes are checked before they feed any allocation, and the pixel
/// loop cannot index out of range.
pub fn encode_png(width: i32, height: i32, bgra: &[u8]) -> Result<Screenshot, String> {
    let width_u32 = u32::try_from(width)
        .map_err(|_| format!("cannot encode a screenshot {width} pixels wide"))?;
    let height_u32 = u32::try_from(height)
        .map_err(|_| format!("cannot encode a screenshot {height} pixels tall"))?;
    let len = (width_u32 as usize)
        .checked_mul(height_u32 as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("cannot encode a screenshot at {width}x{height}"))?;
    if bgra.len() != len {
        return Err(format!(
            "cannot encode a screenshot at {width}x{height}: got {} pixels",
            bgra.len() / 4
        ));
    }

    // Argb8888 is little-endian BGRA in memory; PNG wants RGBA. A copy
    // rather than an in-place swap, as before: the input is the worker's
    // only copy, but aliasing it through `chunks_exact` while writing it
    // back would still be wrong if the lengths ever disagreed -- and the
    // check above is what makes `chunks_exact` yield exactly `len` bytes.
    let mut rgba = Vec::with_capacity(len);
    for pixel in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }

    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width_u32, height_u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // The default (Balanced) optimizes for file size; a screenshot here is a
    // transient thing an agent might pull every render tick, not an asset to
    // store, so latency matters more than shaving off a few KB.
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;

    Ok(Screenshot {
        width: width_u32,
        height: height_u32,
        png,
    })
}

/// The worker thread's whole life: encode jobs in arrival order until there
/// are no more to arrive.
///
/// `jobs` disconnects when `State` -- and with it the encoder holding the
/// matching sender -- goes away, which ends the loop: the thread is never
/// joined, so shutdown never waits on an encode. `done` failing means the
/// completion receiver went away first (same cause), which also ends it. A
/// job that fails, or panics in a debug build, is an error reply for that
/// capture, not a dead worker.
fn run_encoder(jobs: Receiver<ShotJob>, done: channel::Sender<ShotDone>) {
    for job in jobs {
        let result = match catch_unwind(AssertUnwindSafe(|| frame_job(&job))) {
            Ok(result) => result,
            // Release-only note: with `panic = "abort"` this arm is dead --
            // the process is already gone -- and the isolation is
            // `encode_png` having no panic path instead (see its doc). In a
            // debug build this turns a worker panic into one failed capture.
            Err(_) => Err("the screenshot encoder failed".to_string()),
        };
        if done
            .send(ShotDone {
                conn: job.conn,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

/// Encodes one capture and frames its reply line, entirely on the worker.
/// See `ShotDone` for why the framing lives here rather than on the loop.
fn frame_job(job: &ShotJob) -> Result<String, String> {
    let response = match encode_png(job.width, job.height, &job.bgra) {
        Ok(screenshot) => Response::Screenshot(screenshot),
        Err(message) => Response::error(message),
    };
    encode(&response).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest capture that still exercises the real PNG path.
    fn raw_pixels() -> (i32, i32, Vec<u8>) {
        // 2x2 opaque red in little-endian BGRA.
        (2, 2, [0x00, 0x00, 0xFF, 0xFF].repeat(4))
    }

    #[test]
    fn encode_is_deterministic_for_the_same_pixels() {
        let (width, height, bgra) = raw_pixels();
        let first = encode_png(width, height, &bgra).expect("encodes");
        let second = encode_png(width, height, &bgra).expect("encodes again");
        assert_eq!(first, second, "the worker must not depend on hidden state");
    }

    #[test]
    fn encode_reports_the_dimensions_it_was_given() {
        let (width, height, bgra) = raw_pixels();
        let shot = encode_png(width, height, &bgra).expect("encodes");
        assert_eq!((shot.width, shot.height), (2, 2));
        assert!(!shot.png.is_empty());
    }

    #[test]
    fn encode_produces_a_png_a_decoder_accepts() {
        use std::io::Cursor;
        let (width, height, bgra) = raw_pixels();
        let shot = encode_png(width, height, &bgra).expect("encodes");
        let decoder = png::Decoder::new(Cursor::new(&shot.png));
        let mut reader = decoder.read_info().expect("a readable PNG");
        let mut pixels = vec![0u8; reader.output_buffer_size().expect("sized")];
        let info = reader.next_frame(&mut pixels).expect("a frame");
        assert_eq!((info.width, info.height), (2, 2));
        // Opaque red through the BGRA-to-RGBA swizzle, RGBA byte order.
        assert!(
            pixels
                .chunks_exact(4)
                .all(|pixel| pixel == [0xFF, 0x00, 0x00, 0xFF]),
            "the swizzle put the channels in the wrong places"
        );
    }

    #[test]
    fn encode_refuses_pixels_that_do_not_match_the_dimensions() {
        let short = vec![0u8; 3 * 4];
        match encode_png(2, 2, &short) {
            Err(message) => assert!(message.contains("2x2"), "wrong error: {message}"),
            Ok(_) => panic!("encoded 12 bytes as a 2x2 frame"),
        }
        // And trailed garbage with it: more bytes than the frame holds is
        // the same refusal, not a silent truncation.
        let long = vec![0u8; 5 * 4];
        assert!(encode_png(2, 2, &long).is_err());
    }

    #[test]
    fn encode_refuses_unrepresentable_dimensions() {
        let empty: Vec<u8> = Vec::new();
        assert!(encode_png(0, 0, &empty).is_err());
        assert!(encode_png(-1, 10, &empty).is_err());
        assert!(encode_png(10, -1, &empty).is_err());
    }

    #[test]
    fn the_swizzle_is_not_an_in_place_reinterpretation() {
        // A single blue pixel: BGRA [FF, 00, 00, FF] must come out RGBA
        // [00, 00, FF, FF]. If the channels were merely relabelled, red and
        // blue would swap -- which is exactly what an agent matching pixels
        // to window colors would misread.
        let shot = encode_png(1, 1, &[0xFF, 0x00, 0x00, 0xFF]).expect("encodes");
        use std::io::Cursor;
        let decoder = png::Decoder::new(Cursor::new(&shot.png));
        let mut reader = decoder.read_info().expect("a readable PNG");
        let mut pixels = vec![0u8; reader.output_buffer_size().expect("sized")];
        reader.next_frame(&mut pixels).expect("a frame");
        assert_eq!(pixels, vec![0x00, 0x00, 0xFF, 0xFF]);
    }
}
