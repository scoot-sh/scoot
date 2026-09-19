//! `ext-image-copy-capture-v1` + `ext-image-capture-source-v1`: the screen
//! capture a shell, a screenshot tool or a screen-share needs.
//!
//! This is the protocol side of something scoot already does for itself.
//! `scoot msg screenshot` (see `screenshot.rs`) answers an agent over the
//! privileged, owner-only IPC socket and is unchanged by this module; what had
//! nothing to consume was the *standard* path -- `grim`, a Quickshell
//! launcher's window thumbnails, a workspace overview's live preview -- which
//! will never speak a compositor-specific protocol. Both shell probes filed it
//! as the same gap (DMS gap 6, Noctalia gap 6).
//!
//! ## Why this protocol
//!
//! `CLAUDE.md`'s standing rule is the `ext-` protocol where one exists, and
//! here one does *and* the pinned Smithay rev implements it
//! (`wayland::image_copy_capture`, based on cosmic-comp's screencopy code,
//! plus `wayland::image_capture_source`) -- unlike the three protocol items
//! before this one, which all had to be hand-rolled against generated
//! bindings. So this module is handler code, not wire format.
//!
//! The older `wlr-screencopy-unstable-v1` is deliberately **not** published
//! alongside it, which is the opposite call to the one
//! `foreign_toplevel_management.rs` made -- and for the same reason, which is
//! measurement rather than preference. The clients that motivated this entry
//! already speak the `ext-` protocol: `grim` 1.5.0 carries
//! `ext_image_copy_capture_v1` and *only* that, and stock `quickshell` 0.3.1
//! (the build both DMS and Noctalia run on) carries the `ext-` manager and
//! both `ext-` source managers. Nothing measured needs the wlr protocol, so
//! nothing here implements it.
//!
//! ## Output capture only, deliberately
//!
//! Two source managers exist in the protocol family: one that makes a capture
//! source out of a `wl_output`, and one that makes one out of an
//! `ext_foreign_toplevel_handle_v1`. scoot publishes **only the output one**.
//!
//! A toplevel source means capturing one window's content in isolation, which
//! for this compositor means rendering that window's own surface tree into a
//! second render target rather than reading a region back out of the shared
//! output framebuffer -- plus its own constraint-refresh path (a window
//! resizes far more often than the output does), its own mid-session
//! teardown (the window closes), and its own answer for a locked session. That
//! is the whole of this module again, so it was
//! [its own backlog item](../../../../docs/backlog/resolved/screencopy-toplevel-capture-done.md)
//! — probed and closed unreachable without building it — rather than a
//! half-implementation here.
//!
//! The toplevel global is not advertised at all rather than advertised and
//! refused: a client that can create a source and is then told `stopped` has
//! to discover the refusal at runtime, where a client that never sees the
//! global takes its own fallback path immediately.
//!
//! ## When a capture actually happens
//!
//! A `capture` request does **not** copy pixels there and then. The frame is
//! parked on its session and serviced from [`State::service_captures`], which
//! runs on the frame tick right after [`State::render`]. Three reasons, in
//! descending order of how much they matter:
//!
//! - **It bounds what a client can ask for.** An immediate copy would let a
//!   client loop `create_frame`/`capture` as fast as its socket allows, and
//!   each one costs a full framebuffer read-back plus a full copy into its
//!   buffer (~8 MB each at 1920x1080). Parked, a session gets at most one
//!   capture per frame tick, and the IPC screenshot path is throttled the same
//!   way for the same reason (`ipc.rs` spaces a connection's screenshots by
//!   [`FRAME_INTERVAL`](super::headless::FRAME_INTERVAL)).
//! - **The protocol asks for exactly this.** "Unless this is the first
//!   successful captured frame performed in this session, the compositor may
//!   wait an indefinite amount of time for the source content to change before
//!   performing the copy." So a session's *first* frame is serviced on the
//!   next tick whatever the screen is doing, and every later one waits for
//!   [`State::frame_serial`] to move -- a static desktop costs a `u64`
//!   comparison per tick and nothing else.
//! - **It keeps the capture out of Wayland request dispatch.** Servicing a
//!   capture means calling [`State::render`], which sends frame callbacks and
//!   flushes every client; doing that from inside the dispatch of one client's
//!   request is re-entrancy this compositor has no reason to take on.
//!
//! ## What this does *not* bound
//!
//! One thing a client can still ask for without limit: **how many sessions it
//! holds at once.** The per-tick work is one framebuffer read-back (shared by
//! every session, which is why [`deliver`] does it once) plus one copy into
//! each due client buffer, so N sessions with N parked frames cost N
//! full-screen copies on the ticks the screen changes.
//!
//! Deliberately not capped here, and the reasoning is worth recording rather
//! than re-derived:
//!
//! - That is the same shape of per-frame, per-object work a client can already
//!   demand by mapping N surfaces, which this compositor accepts unbounded. A
//!   cap on one and not the other would be inconsistent, not safer.
//! - The only non-punitive form of a cap is per *client*, and the handler API
//!   this is written against does not say which client a session belongs to
//!   (`capture_constraints` is handed a source, not a `Client`).
//!   `ImageCopyCaptureHandler::new_session` is handed a `Session` with no
//!   protocol id or client accessor either, and Smithay's
//!   `new_with_filter` counts manager *binds*, not sessions -- so per-client
//!   session accounting has no hook at all today, and building a fifth
//!   independent one here instead of folding sessions into the shared
//!   mechanism the sibling entries will land would contradict the backlog's
//!   own direction. A global cap would let one greedy client deny a
//!   well-behaved one -- exactly the
//!   concern already filed against the IPC connection cap.
//!
//! What *is* capped is the cheaper half of the same attack: **how many live
//! frame objects one client holds**, at [`MAX_FRAMES_PER_CLIENT`], refused
//! pre-delegation with the protocol's own `duplicate_frame` error (see
//! `dispatch.rs`'s module doc for why a protocol error and not a silent
//! ignore). A `create_frame` loop with no `capture` -- which scoot's
//! [`Capture::pending`] throttle never sees, since it only runs on `capture`
//! -- can therefore cost at most sixteen small objects per abusive client,
//! and each refusal kills only the client that overflowed.
//!
//! Filed as
//! [its own item](../../../../docs/backlog/resolved/screencopy-session-cap-done.md),
//! the same way the `wl_shm` per-pool cap names the total it does not bound,
//! rather than treated as covered by the throttling above.
//!
//! ## The session-lock guarantee, and where it comes from
//!
//! A capture taken while `ext-session-lock-v1` holds the session must see the
//! lock screen and never the windows behind it. That falls out of reuse rather
//! than a check here: [`State::render`] decides *once per frame* whether it is
//! drawing the lock screen or the desktop (`headless.rs`'s `locked`), so the
//! framebuffer this module reads back only ever holds one of the two. It is
//! the same inheritance `scoot msg screenshot` already has.
//!
//! The one thing reuse does not cover is the window *between* a lock being
//! accepted and the first blanked frame actually reaching the framebuffer: the
//! session is locked, the framebuffer still holds the desktop, and
//! `session_lock.rs` calls that state "locked, still pending". Captures are not
//! serviced at all while a lock is pending
//! ([`SessionLock::awaiting_blank`](super::session_lock)) -- the frame stays
//! parked, which is what the protocol's "may wait an indefinite amount of time"
//! allows, rather than being answered with desktop pixels.
//!
//! ## Cursors
//!
//! `create_session`'s `paint_cursors` option is accepted and has no effect,
//! and this is a **known deviation** from the protocol ("the cursor must not be
//! composited onto the frame if this flag is not set"), recorded here rather
//! than left to be discovered:
//!
//! - under `--headless` and `--nested` nothing draws a cursor at all (see
//!   `cursor.rs`), so a capture never contains one whatever the flag says;
//! - under `--tty` the cursor is a render element in the one framebuffer this
//!   module reads back, so a capture always contains it.
//!
//! Honouring the flag would mean a second render of the whole output with the
//! cursor element dropped, i.e. doubling the cost of the thing this module
//! spends most of its time on, for a flag whose only effect is on the one
//! backend that has a pointer to draw. `scoot msg screenshot` has the same
//! property today for the same reason.
//!
//! `create_pointer_cursor_session` is refused outright (Smithay's default
//! `cursor_capture_constraints` returns `None`): a cursor session captures the
//! cursor *image* into its own buffer, which is a second, independent render
//! target with its own lifecycle, and no client measured here asks for one.
//!
//! ## Buffer formats
//!
//! `wl_shm` only for *capture*: `BufferConstraints::dma` is always `None`, so
//! a capture session never offers to write into a client's dma-buf.
//!
//! That is a statement about this module, not about the compositor. scoot
//! does import the dma-bufs a client hands it, into whichever renderer the
//! session is running (see [`dmabuf`](super::dmabuf)): a GL client's window
//! composites here like any other. Capture is the other direction, and it
//! stays shm because here scoot is the one *writing* -- filling a client's
//! dma-buf means matching its stride, modifier and sync rules on a path where
//! `wl_shm` already works everywhere and costs a CPU renderer nothing extra.
//!
//! **Still `None` after the advertisement became renderer-derived**, and
//! deliberately, because the two are less related than they look. Knowing
//! what the renderer can *import* says nothing about this: a capture would
//! `Bind` the client's buffer as a **render target** and blit into it, which
//! is a different capability (`dmabuf_render_formats`), a code path that does
//! not exist here at all, and -- under pixman -- the one that trips the
//! bound-target cache eviction `dmabuf.rs`'s `schedule_cache_drain` warns
//! about, which would have to be fixed in the same change.
//! `DmabufConstraints` also requires a `DrmNode`, which the GPU-less
//! containers scoot targets do not have, so the honest answer there would be
//! `None` regardless. If some client ever measurably needs a dma-buf capture,
//! that is its own item -- with its own write path and its own before/after
//! numbers -- rather than a line to flip here.
//!
//! `Xrgb8888` is offered first and `Argb8888` second. Both are the same four
//! bytes in the same order in memory -- the compositor's own framebuffer is
//! `Argb8888`, which is little-endian BGRA (the same fact `screenshot.rs`
//! records for the PNG path) -- and the only difference is what the fourth byte
//! means. `Xrgb8888` is offered first on purpose: `[appearance]
//! background_color` may have an alpha below 255, and a client that took that
//! alpha at face value would render a translucent "screenshot" of an opaque
//! screen. A capture into an `Xrgb8888` buffer has that byte forced to `0xff`
//! while the background is actually translucent; over the default opaque
//! background the framebuffer already carries `0xff` everywhere, so the
//! forcing pass is skipped outright (see [`xrgb_needs_forcing`]). A client
//! that asks for `Argb8888` gets the framebuffer's own alpha, which is
//! what that format means.

use std::collections::HashMap;
use std::time::Duration;

use smithay::output::{Output, WeakOutput};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{Client, DisplayHandle};
use smithay::utils::{Buffer as BufferCoords, Clock, IsAlive, Monotonic, Rectangle, Transform};
use smithay::wayland::dmabuf::DmabufState;
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
    OutputCaptureSourceState,
};
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CaptureFailureReason, Frame, FrameRef, ImageCopyCaptureHandler,
    ImageCopyCaptureState, Session, SessionRef,
};
use smithay::wayland::shm::with_buffer_contents_mut;

use super::State;
use super::render::{Backend, CaptureStage};

#[cfg(test)]
mod tests;

/// The shm formats every session is offered, in the order a client sees them.
///
/// See this module's doc for why `Xrgb8888` comes first.
///
/// `pub(super)` rather than private: [`dmabuf`](super::dmabuf)'s feedback
/// table names these same formats, and its test pins the two lists to each
/// other -- which it can only do if it can read this one.
pub(super) const FORMATS: [wl_shm::Format; 2] =
    [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888];

/// How many capture frames one Wayland client may hold at once, across all of
/// its sessions, before a further `create_frame` is refused with the
/// protocol's own `duplicate_frame` error.
///
/// Why this number, and why per client rather than per session (which is what
/// the protocol's rule literally says -- "at most one frame object ... for a
/// given session"):
///
/// - The pinned Smithay rev exposes no way to learn *which* session a
///   `create_frame` names before delegating it: `Session`/`SessionRef` carry
///   no protocol id or client, and `SessionData`'s fields are private, so a
///   `dispatch.rs` intercept cannot map the request to this module's
///   [`Capture`] list. What the intercept *does* see is the [`Client`], so
///   the bound is keyed by that. See `dispatch.rs`'s module doc for the full
///   derivation, including why a silent ignore is not an option.
/// - 16 is far above anything legitimate: the protocol allows one live frame
///   per session, and a client would need sixteen sessions each mid-flight at
///   once to trip this -- `grim` holds one per run, a shell's persistent
///   overview preview one per session, and the suites here never exceed two.
///   The failure mode of tripping it is the client being disconnected, so the
///   margin errs generous: a tighter number would bound a few more small
///   objects per abuser, a looser one risks nothing an abuser could not already
///   do with sixteen mapped surfaces.
/// - Per client rather than global, deliberately: a global cap would let one
///   greedy client deny a well-behaved one, the concern already filed against
///   the IPC connection cap. Refusing this way kills only the client that
///   overflowed, never an innocent one.
pub(super) const MAX_FRAMES_PER_CLIENT: u32 = 16;

/// Bytes per pixel in both formats [`FORMATS`] offers, and in the `Argb8888`
/// framebuffer they are read back from.
const BYTES_PER_PIXEL: i32 = 4;

/// Whether an `Xrgb8888` capture's undefined fourth byte has to be forced
/// opaque on this tick.
///
/// True exactly while `[appearance] background_color`'s alpha is below 1.0.
/// Read off [`State::appearance`](super::State) once per frame tick in
/// [`State::service_captures`], not cached anywhere: there is
/// no config reload today (nothing writes `appearance` after `State::new`),
/// so any read is current -- but a startup-cached bool would go stale the
/// moment a reload exists, and a per-tick read cannot.
///
/// Exact `< 1.0`, no epsilon, and that is deliberate rather than lazy.
/// `Color::parse` produces `byte / 255.0`, so the only opaque value a config
/// can spell is exactly `1.0`; the nearest translucent one is `254 / 255`
/// (~0.996), nowhere near float dust. An epsilon band would misclassify a
/// directly-constructed `0.9999999` as opaque while the framebuffer pixman
/// cleared with it stayed translucent -- the exact failure this exists to
/// prevent. `1.0` means the clear left `0xff` in every background pixel (see
/// below), anything else means it did not.
///
/// Why the background alpha alone decides this, rather than anything about
/// the windows: every window composites onto the framebuffer with
/// Porter-Duff source-over, and over an opaque destination that operation is
/// alpha-preserving-opaque -- the result is `0xff` whatever the source's own
/// alpha was. (`Xrgb8888` windows take the `Src` path instead, where the
/// destination is irrelevant — but pixman normalizes a no-alpha source to
/// opaque on store, measured live, so that path lands on `0xff` too. This
/// second half is renderer-specific to the pixman pipeline: a future GPU
/// sampler must re-derive it rather than inherit this paragraph.) Only the
/// clear color writes raw alpha into the framebuffer, so
/// an opaque background means every pixel the read-back sees already carries
/// `0xff` and the forcing pass below would OR `0xff` onto `0xff`: a no-op
/// worth skipping.
fn xrgb_needs_forcing(background_alpha: f32) -> bool {
    background_alpha < 1.0
}

/// How many pixels the `Xrgb8888` opacity pass covers per step.
///
/// Four, i.e. sixteen bytes at a time. Measured rather than picked -- see
/// [`OPAQUE`].
const PIXELS_PER_STEP: usize = 4;

/// [`PIXELS_PER_STEP`] fully opaque alpha channels, as the little-endian word
/// that many pixels of either [`FORMATS`] entry read as.
///
/// Both formats are little-endian BGRA in memory, so the alpha byte -- for
/// `Xrgb8888` the undefined `X` byte -- is the *high* byte of each 32-bit
/// pixel. OR-ing this into four pixels at once is what makes an `Xrgb8888`
/// capture opaque; see this module's doc for why it has to be.
///
/// **Four at a time, rather than one, and deliberately still a second pass
/// over the row rather than folded into the copy.** "Don't walk the row twice"
/// is the obvious shape for this, and measuring says it is the wrong one.
/// Isolated over a 1920x1080 frame, at the two optimization levels this crate
/// is built at (`ms/frame`, dev VM, median of runs):
///
/// | | `opt-level=0` | `opt-level=3` |
/// | --- | --- | --- |
/// | row `memcpy` alone (what the `Argb8888` path does) | 0.84 | 0.20 |
/// | ...plus one alpha byte per pixel | 30.4 | 1.56 |
/// | one pass, per pixel, copying and forcing together | 186.4 | 1.29 |
/// | ...plus alpha two pixels at a time (`u64`) | 83.8 | 0.89 |
/// | **...plus alpha four pixels at a time (`u128`)** | **33.7** | **0.92** |
///
/// A single per-pixel pass is **six times slower** at `opt-level=0` -- which is
/// what every test, every smoke test and every dev-VM session here runs -- and
/// buys ~17% of this pass in release. Two *wide* passes beat one narrow one:
/// the copy stays a `memcpy` intrinsic, and the opacity pass does a quarter as
/// many iterations as a per-pixel one.
///
/// This isolated table is a lower bound on the dev-build cost, not a
/// prediction of it: measured end to end (below), the dev-build gap is
/// 2-2.5x wider than this table's own +3.3ms would suggest, most likely
/// because the standalone micro-benchmark doesn't reproduce the real
/// function's `opt-level=0` codegen, where `u128::from_le_bytes`/
/// `to_le_bytes`/`read_unaligned` are genuine, non-inlined calls in context.
/// The *ranking* of the five rows is what this table is good for; the
/// absolute numbers should be read from the end-to-end measurement.
///
/// End to end, over a whole `grim` capture rather than this pass alone
/// (`ms/capture`, 1920x1080, three 20s runs each, median):
///
/// | | before (per byte) | after (`u128`) |
/// | --- | --- | --- |
/// | release build (`opt-level=3`, fat LTO) | 10.66 | **10.26** |
/// | dev build (`opt-level=0`) | 54.8 | 63.3 |
///
/// The dev-build cost is unambiguous: independently reproduced (+15% here,
/// +18% on a second run), no overlap between the two distributions either
/// time. The release-build win is real but smaller relative to its own
/// run-to-run spread than "~4%" suggests on its own -- a block-ordered
/// re-run (all of one binary's samples, then all of the other's) briefly
/// inverted it, and only alternating between binaries (the method this
/// table uses) recovered a consistent ~3-4% direction, with individual
/// samples from *before* still occasionally beating individual samples from
/// *after*. So: the trade actually being accepted is a certain 15-18% dev-
/// build cost for a probable, smaller release-build gain, not two equally
/// solid numbers -- and it is accepted on that basis, not a stronger one:
/// `Cargo.toml`'s own release-profile comment is explicit that
/// "lightweight" is judged in release. Neither figure moves a compositor
/// nobody is capturing from: measured unchanged at 147 vs 148 jiffies over
/// 20s.
///
/// The row this table does not have is the one that would dwarf all of them:
/// **not forcing the byte at all** is ~13% off a release capture and ~77% off
/// a debug one, because `Xrgb8888`'s fourth byte is undefined by the format
/// and a conforming client never reads it. That was a *behaviour* question
/// rather than a performance one, so it was
/// [filed as its own item](../../../../docs/backlog/resolved/screencopy-xrgb-alpha-forcing-done.md)
/// rather than decided here -- and resolved as the conditional this module
/// now implements: force only while the background alpha is below 1.0 (see
/// [`xrgb_needs_forcing`]), which costs nothing in the default opaque
/// configuration and keeps the guarantee where it was actually needed.
const OPAQUE: u128 = 0xFF00_0000_FF00_0000_FF00_0000_FF00_0000;

/// Everything this compositor keeps for `ext-image-copy-capture-v1`.
pub struct Screencopy {
    /// The `ext_output_image_capture_source_manager_v1` global, which every
    /// `create_source` request is routed through
    /// ([`OutputCaptureSourceHandler::output_capture_source_state`]).
    ///
    /// There is deliberately no `ToplevelCaptureSourceState` beside it, and no
    /// `ImageCaptureSourceState` either: the latter is a unit struct at the
    /// pinned rev with no global of its own and no accessor on its handler
    /// trait, so storing one would document nothing. A Smithay revision that
    /// gives it state will fail to compile here rather than silently lose it.
    output_sources: OutputCaptureSourceState,
    /// The `ext_image_copy_capture_manager_v1` global plus Smithay's own
    /// session/frame bookkeeping. Read on every session and every frame.
    capture: ImageCopyCaptureState,
    /// The `zwp_linux_dmabuf_v1` delegate type Smithay routes the dmabuf
    /// global through.
    ///
    /// The advertisement itself lives in [`dmabuf`](super::dmabuf): this
    /// field is only where the state the ticket names keeps the delegate so
    /// [`DmabufHandler::dmabuf_state`](smithay::wayland::dmabuf::DmabufHandler::dmabuf_state)
    /// has one field to return. Created empty here and given its global by
    /// `dmabuf::advertise` from `headless::init_named`, once the renderer
    /// whose importable formats the advertisement is derived from exists --
    /// never touched per frame, per bind or per hotplug event afterwards. A
    /// state with no global is a complete, working value: that is also what a
    /// session whose renderer can import nothing keeps.
    pub(super) dmabuf: DmabufState,
    /// One entry per live capture session, in creation order.
    ///
    /// The owned [`Session`] lives here and nowhere else: dropping one sends
    /// `stopped` and fails its outstanding frames, so an entry leaving this
    /// list *is* the session ending. Entries are removed in
    /// [`ImageCopyCaptureHandler::session_destroyed`], which is the only place
    /// a session is dropped.
    sessions: Vec<Capture>,
    /// How many live `ext_image_copy_capture_frame_v1` objects each Wayland
    /// client holds, by client id.
    ///
    /// Counted up in [`State::refuse_excess_capture_frame`] (called from
    /// `dispatch.rs` for every `create_frame` before Smithay sees it) and
    /// back down in [`State::forget_capture_frame`] (called from the same
    /// file's destruction hook for every dead frame object). An entry exists
    /// only while the client holds at least one frame, so this is bounded by
    /// live protocol objects the same way wayland-backend already bounds
    /// them -- and a frame that outlives its session (legal per the
    /// protocol) keeps counting until *it* dies, which is what keeps the
    /// bookkeeping exact across session teardown.
    frames_per_client: HashMap<ClientId, u32>,
    /// The monotonic clock a frame's `presentation_time` is read from.
    ///
    /// Not [`State::start_time`](super::State), which measures time since this
    /// process started: the protocol asks for "system monotonic time", which
    /// is what a client needs to line a capture up against anything else on
    /// the machine.
    clock: Clock<Monotonic>,
}

/// One capture session, and the single frame a client may have outstanding on
/// it.
struct Capture {
    session: Session,
    /// The frame whose `capture` request has arrived and whose pixels have not
    /// been written yet.
    ///
    /// At most one, which is what the protocol says ("at most one frame object
    /// can exist for a given session at any time") -- but *not* what the
    /// pinned Smithay rev enforces: its session handler pushes every
    /// `create_frame` onto an unbounded list and never raises
    /// `duplicate_frame`. So this slot is also the bound: a second outstanding
    /// capture on one session is failed rather than queued, which is the
    /// behaviour a conforming client cannot tell from the protocol error it
    /// should have got.
    pending: Option<Frame>,
    /// [`State::frame_serial`](super::State) as of this session's last
    /// delivered capture, or `None` while it has delivered none.
    ///
    /// `None` is what makes a session's first frame exempt from waiting for
    /// the screen to change; see this module's doc.
    delivered: Option<u64>,
}

impl Capture {
    /// Whether this session's parked frame should be copied on this tick.
    ///
    /// The first frame of a session always is. A later one only once the
    /// screen has actually been redrawn since the previous delivery -- which
    /// the protocol explicitly permits waiting for, and which is what keeps a
    /// live-preview client from costing a full framebuffer copy per tick on a
    /// desktop that is not moving.
    fn due(&self, serial: u64) -> bool {
        self.pending.is_some() && self.delivered != Some(serial)
    }
}

impl Screencopy {
    /// Creates the `ext_image_copy_capture_manager_v1` and
    /// `ext_output_image_capture_source_manager_v1` globals, and the empty
    /// dmabuf state `dmabuf::advertise` later hangs `zwp_linux_dmabuf_v1` on.
    ///
    /// No client filter, for the same reason the session-lock, data-control
    /// and input-method globals have none: scoot has no security-context
    /// support, so an allow-list would be theatre (see `docs/protocols.md`'s
    /// trust note). Worth naming here because screen capture is the most
    /// obviously sensitive of those -- a client that can reach this socket can
    /// read the screen -- so this is a deliberate consistency with the trust
    /// model the whole compositor already states, not an oversight about what
    /// the protocol can do. The dmabuf global extends that note rather than
    /// widening it: it hands out no pixels by itself, only format feedback,
    /// and an import is a mapping of the client's *own* buffer (see
    /// [`dmabuf`](super::dmabuf)).
    pub(super) fn new(dh: &DisplayHandle) -> Self {
        Self {
            output_sources: OutputCaptureSourceState::new::<State>(dh),
            capture: ImageCopyCaptureState::new::<State>(dh),
            dmabuf: DmabufState::new(),
            sessions: Vec::new(),
            frames_per_client: HashMap::new(),
            clock: Clock::new(),
        }
    }

    /// How many sessions this compositor is tracking, and how many Smithay
    /// still is -- which have to be the same number, and have to fall back to
    /// zero.
    ///
    /// Both halves, because they are two lists with two different owners and
    /// only one of them is swept by code in this repository (see
    /// [`ImageCopyCaptureHandler::session_destroyed`]). A test that asked only
    /// the first would pass against a version that leaked every session a
    /// client ever created.
    #[cfg(test)]
    pub(super) fn session_count(&self) -> (usize, usize) {
        (self.sessions.len(), self.capture.sessions().len())
    }

    /// How many capture frames all clients hold between them. Test-only: the
    /// flood and cycling tests assert the bookkeeping drains, which a
    /// compositor-side count states directly -- while Smithay's own
    /// `active_frames` lists stay private, so no test could read them.
    #[cfg(test)]
    pub(super) fn frames_in_flight(&self) -> usize {
        self.frames_per_client.values().sum::<u32>() as usize
    }
}

impl State {
    /// Records one more live capture frame for `client`, refusing past
    /// [`MAX_FRAMES_PER_CLIENT`].
    ///
    /// Returns whether the caller must refuse the `create_frame` -- and the
    /// refusal itself (the `duplicate_frame` protocol error, posted on the
    /// session) stays with the caller in `dispatch.rs`, which holds the
    /// session object this module never sees. Counting and refusing split
    /// that way for the same reason the frozen-icon guard keeps both halves
    /// together does not apply here: the counted key (the client) and the
    /// error target (the session) are different objects.
    ///
    /// Called *before* delegation, so a refused frame is never created and a
    /// counted one always is: Smithay's `create_frame` handler cannot fail
    /// (it initialises the object and pushes it unconditionally), which is
    /// what keeps this count exact rather than leaking on a path that counts
    /// but never creates.
    pub(super) fn refuse_excess_capture_frame(&mut self, client: &Client) -> bool {
        let count = self
            .screencopy
            .frames_per_client
            .entry(client.id())
            .or_insert(0);
        if *count >= MAX_FRAMES_PER_CLIENT {
            return true;
        }
        *count += 1;
        false
    }

    /// Forgets one live capture frame for the client a dead frame object
    /// belonged to.
    ///
    /// Called from `dispatch.rs`'s destruction hook for every destroyed
    /// `ext_image_copy_capture_frame_v1`, which is also what bounds the map:
    /// an entry leaves it exactly when its last frame's protocol object does,
    /// including on client disconnect (whose cleanup destroys every object)
    /// and for frames that outlive their session. `saturating_sub`, because a
    /// destroy the count never saw would be a bookkeeping bug, not a client
    /// one -- and a compositor must not panic on its own accounting.
    pub(super) fn forget_capture_frame(&mut self, client: &ClientId) {
        if let Some(count) = self.screencopy.frames_per_client.get_mut(client) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.screencopy.frames_per_client.remove(client);
            }
        }
    }
}

impl ImageCaptureSourceHandler for State {}

impl OutputCaptureSourceHandler for State {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.screencopy.output_sources
    }

    /// Records which output a source names, which is the only thing that makes
    /// the opaque source object mean anything later.
    ///
    /// A [`WeakOutput`], not an [`Output`]: a source outlives nothing here
    /// today (scoot has one output for the process's life), but holding a
    /// strong `Output` in a client-owned object's user data would make a
    /// client's lifetime decide the compositor's.
    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for State {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.screencopy.capture
    }

    /// What a client must allocate to capture `source`, or `None` to refuse it.
    ///
    /// Refused when the source is not one of *this* compositor's outputs --
    /// which covers a source built from a `wl_output` that has since gone, and
    /// (were the toplevel source manager ever advertised) any source that does
    /// not name an output at all. Refusing is `stopped` on the session the
    /// client just made, which is the protocol's own way of saying "not this
    /// one".
    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let weak = source.user_data().get::<WeakOutput>()?;
        // Compared against the compositor's own output rather than merely
        // upgraded: `upgrade` answers "some output still exists", which is not
        // the question -- the capture below reads *this* compositor's one
        // framebuffer, so a source naming anything else must not be told a
        // size it would then be handed the wrong pixels for.
        if self.output.as_ref() != Some(&weak.upgrade()?) {
            return None;
        }
        Some(constraints(self.backend.as_ref()?))
    }

    fn new_session(&mut self, session: Session) {
        self.screencopy.sessions.push(Capture {
            session,
            pending: None,
            delivered: None,
        });
    }

    /// Parks a capture request until the next frame tick. See this module's
    /// doc for why it is not served here.
    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        let Some(capture) = self
            .screencopy
            .sessions
            .iter_mut()
            .find(|capture| capture.session == *session)
        else {
            // Unreachable: Smithay only routes a frame whose session is in its
            // own list, and every session in that list was pushed here by
            // `new_session`. A frame with nowhere to be parked must still be
            // answered, or the client waits forever for an event that cannot
            // come.
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };
        if capture.pending.is_some() {
            // The `duplicate_frame` protocol error's job, done with a failure
            // the client has to handle anyway -- see `Capture::pending`.
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        capture.pending = Some(frame);
        // Not `request_render()`: nothing about a capture request changed what
        // is on screen, and marking the screen dirty would redraw the whole
        // output for every capture *and* bump `frame_serial`, which would make
        // every session's "has anything changed" test answer yes forever. This
        // only guarantees a tick happens, which is all the parked frame needs.
        self.ensure_ticking();
    }

    /// A frame object went away before it was serviced -- the client destroyed
    /// it, or disconnected.
    ///
    /// Dropping the parked [`Frame`] is what answers it: `Frame`'s own `Drop`
    /// fails it, which is a no-op on an object that is already gone.
    fn frame_aborted(&mut self, frame: FrameRef) {
        for capture in &mut self.screencopy.sessions {
            if capture
                .pending
                .as_ref()
                .is_some_and(|parked| *parked == frame)
            {
                capture.pending = None;
                return;
            }
        }
    }

    /// A session object went away -- destroyed by the client, or with it.
    ///
    /// Two things have to happen and neither is optional:
    ///
    /// - the [`Capture`] is dropped, and with it the [`Frame`] parked on it,
    ///   whose own `Drop` fails it. It is specifically **not** [`Session`]'s
    ///   `Drop` that does this, even though that one also fails frames: by the
    ///   time this runs the session object is dead, and `Session::drop` opens
    ///   by returning early on exactly that. Without the parked frame being
    ///   dropped here, a client that destroyed a session with a capture
    ///   outstanding would never hear `ready` or `failed` for it;
    /// - Smithay's *own* session list is swept. Nothing upstream removes a
    ///   destroyed session from `ImageCopyCaptureState::sessions`; only
    ///   `cleanup()` does, and nothing upstream calls that either. Without
    ///   this, every session a client ever created stays in a `Vec` that the
    ///   `capture` request then walks linearly -- an unbounded, client-driven
    ///   leak.
    fn session_destroyed(&mut self, session: SessionRef) {
        self.screencopy
            .sessions
            .retain(|capture| capture.session != session);
        self.screencopy.capture.cleanup();
    }
}

impl State {
    /// Copies the framebuffer into whichever parked capture frames are due.
    ///
    /// Called from the frame tick, immediately after [`State::render`], so the
    /// pixels a capture sees are the ones that frame just drew. Costs a `Vec`
    /// length check when no client is capturing, which is the normal case.
    pub(super) fn service_captures(&mut self) {
        let serial = self.frame_serial;
        if !self
            .screencopy
            .sessions
            .iter()
            .any(|capture| capture.due(serial))
        {
            return;
        }
        // A lock that has been accepted but not yet blanked the screen leaves
        // the desktop in the framebuffer while `is_locked()` already answers
        // true. Nothing is failed and nothing is answered: the frames stay
        // parked and are serviced on the tick after `confirm_lock`. See this
        // module's doc. Under `--tty` that tick is scheduled by the confirm
        // itself (`State::confirm_lock` re-arms the frame ticker whenever it
        // takes a wait): the tick that rendered the blank drops the timer,
        // so without the re-arm nothing would serve the parked frames
        // afterwards.
        if self.session_lock.awaiting_blank() {
            return;
        }
        let Some(mut backend) = self.backend.take() else {
            // No render target at all, which nothing can produce after
            // `headless::init_named` -- but a frame that is never answered is
            // a client that waits forever, so say so rather than return.
            fail_due(
                &mut self.screencopy.sessions,
                serial,
                CaptureFailureReason::Unknown,
            );
            return;
        };
        let presented = Duration::from(self.screencopy.clock.now());
        // Read here, on the tick the captures are served on, rather than
        // cached: see `xrgb_needs_forcing`. No TOCTOU between this and the
        // writes below -- this whole tick is synchronous on the one event-loop
        // thread, nothing dispatches in the middle of it, and nothing writes
        // `appearance` at all today.
        let force_xrgb_alpha = xrgb_needs_forcing(self.appearance.background_color.a);
        deliver(
            &mut backend,
            &mut self.screencopy.sessions,
            serial,
            presented,
            force_xrgb_alpha,
        );
        self.backend = Some(backend);
        // `render()` flushes at its end, and `post_dispatch` flushes after
        // every wakeup -- but this runs *between* the two, and the `ready`
        // event a client is blocked on is queued here. Under the real event
        // loop `post_dispatch` would cover it a moment later; a caller
        // dispatching the loop directly (every test harness) has no such
        // guarantee, and a flush with nothing queued costs no syscall (see
        // `mod.rs`'s `post_dispatch`).
        let _ = self.display_handle.flush_clients();
    }

    /// Re-advertises the buffer size to every live session.
    ///
    /// Called from [`State::resize_output`](super::State), the only thing that
    /// changes the framebuffer's size after startup: a client holding a
    /// session sized for the old mode has to be told to re-allocate, or its
    /// next capture is failed with `buffer_constraints` and it never learns
    /// why. Sends the whole constraint batch plus `done`, which is what the
    /// protocol requires of an update ("regardless of whether it sends the
    /// initial constraints or an update").
    pub(super) fn refresh_capture_constraints(&mut self) {
        let Some(backend) = self.backend.as_ref() else {
            return;
        };
        let constraints = constraints(backend);
        for capture in &self.screencopy.sessions {
            capture.session.update_constraints(constraints.clone());
        }
    }
}

/// What a client must allocate to capture the whole output.
///
/// The size is the *framebuffer*'s, not the output's logical size: this is the
/// buffer the capture is read back out of, so it is the one number the client's
/// buffer has to match. The two agree by construction (`headless.rs`'s
/// `init_named` and `resize_output` set the mode and build the render target
/// from the same pair), and taking it from the backend means they cannot drift
/// apart here even if that ever stops being true.
fn constraints(backend: &Backend) -> BufferConstraints {
    let (width, height) = backend.size();
    BufferConstraints {
        size: (width, height).into(),
        shm: FORMATS.to_vec(),
        // Capture writes into the client's buffer, which is a `Bind` and a
        // blit this module has no path for -- a different renderer capability
        // from the import side, and its own item. See this module's doc.
        dma: None,
    }
}

/// Reads the framebuffer back once and writes it into every due frame.
///
/// One read-back for all of them: `copy_framebuffer` allocates and fills a
/// fresh pixman image every call (confirmed in the pinned rev's
/// `PixmanRenderer`), so doing it per session would cost a full extra copy of
/// the screen for each client watching.
fn deliver(
    backend: &mut Backend,
    sessions: &mut [Capture],
    serial: u64,
    presented: Duration,
    force_xrgb_alpha: bool,
) {
    let (width, height) = backend.size();

    // The same read-back `screenshot.rs` reads a PNG out of, and deliberately
    // the same one: it is what makes the session-lock guarantee above
    // inherited rather than re-derived. Each of its steps is a real failure
    // mode (a renderer that cannot bind its own target, an allocation that
    // failed), and each one has to end in *some* answer per parked frame or
    // its client blocks forever -- which is why the error arm below answers
    // every due frame rather than returning quietly.
    //
    // The write happens inside the callback rather than over a returned
    // buffer: the pixels are a borrowed view into the renderer's own mapping,
    // and copying them out first would cost a full extra copy of the screen
    // before any client had been written to.
    let outcome = backend.capture(|pixels| {
        write_due_captures(
            pixels,
            sessions,
            serial,
            presented,
            force_xrgb_alpha,
            (width, height),
        )
    });
    if let Err(failure) = outcome {
        match failure.stage {
            CaptureStage::Bind => tracing::warn!(
                error = %failure,
                "could not bind the framebuffer for a screen capture"
            ),
            CaptureStage::Copy => tracing::warn!(
                error = %failure,
                "could not copy the framebuffer for a screen capture"
            ),
            CaptureStage::Map => tracing::warn!(
                error = %failure,
                "could not map the framebuffer for a screen capture"
            ),
        }
        fail_due(sessions, serial, CaptureFailureReason::Unknown);
    }
}

/// Writes one read-back frame into every due capture session.
///
/// Split out of [`deliver`] only so the read-back's callback stays readable;
/// the `(width, height)` pair is the backend's own size, which is also what
/// every client's buffer was sized against.
fn write_due_captures(
    pixels: &[u8],
    sessions: &mut [Capture],
    serial: u64,
    presented: Duration,
    force_xrgb_alpha: bool,
    (width, height): (i32, i32),
) {
    // pixman lays a 32-bit image out at `stride * height` bytes with the
    // stride rounded up to a multiple of four -- i.e. exactly `width * 4` for
    // these formats -- but the row length is derived rather than assumed, so a
    // renderer that ever padded its rows would copy correct pixels instead of
    // sheared ones.
    let stride = pixels
        .len()
        .checked_div(height.max(1) as usize)
        .unwrap_or(0);
    let row = (width as usize).saturating_mul(BYTES_PER_PIXEL as usize);
    if height <= 0 || width <= 0 || stride < row {
        tracing::warn!(
            width,
            height,
            len = pixels.len(),
            "the framebuffer read back too small to capture"
        );
        fail_due(sessions, serial, CaptureFailureReason::Unknown);
        return;
    }

    let damage: Vec<Rectangle<i32, BufferCoords>> =
        vec![Rectangle::from_size((width, height).into())];
    for capture in sessions {
        if !capture.due(serial) {
            continue;
        }
        // Checked here rather than left to the write below: a session that has
        // been stopped (its source went away, or the compositor stopped it)
        // owes its frame a `stopped` failure, not a copy.
        let alive = capture.session.alive();
        let Some(frame) = capture.pending.take() else {
            continue;
        };
        if !alive {
            frame.fail(CaptureFailureReason::Stopped);
            continue;
        }
        // `FrameRef::buffer` panics when no buffer is attached, and cannot
        // here: Smithay's `capture` handler fails the frame and returns
        // *before* handing it over if none is, and `attach_buffer` is ignored
        // once `capture` has been seen -- so a frame that reached this list
        // has a buffer, and nothing can take it away again.
        match write_capture(
            &frame.buffer(),
            pixels,
            width,
            height,
            stride,
            force_xrgb_alpha,
        ) {
            Ok(()) => {
                capture.delivered = Some(serial);
                // Full damage every time, not the damage tracker's. A session
                // may sit through several redraws before its next frame
                // arrives, so the real answer would have to be accumulated
                // per session; "all of it" is always a valid superset ("at
                // least the union"), and it is what the protocol requires of a
                // session's first frame anyway.
                frame.success(Transform::Normal, damage.clone(), presented);
            }
            Err(reason) => frame.fail(reason),
        }
    }
}

/// Answers every parked frame that was due with `reason`, for the paths where
/// no pixels could be produced at all.
fn fail_due(sessions: &mut [Capture], serial: u64, reason: CaptureFailureReason) {
    for capture in sessions {
        if !capture.due(serial) {
            continue;
        }
        if let Some(frame) = capture.pending.take() {
            frame.fail(reason);
        }
    }
}

/// Writes a `width` x `height` BGRA image into a client's shm buffer.
///
/// Everything about `buffer` is the client's own number, so everything is
/// re-checked here rather than trusted:
///
/// - **The size, against the framebuffer as it is now.** Smithay validated the
///   buffer against the constraints the session carried when the *frame* was
///   created; a [`State::resize_output`](super::State) between then and now
///   leaves that snapshot stale, and a buffer sized for the old output would
///   otherwise be written past its end.
/// - **The format.** Only the two [`FORMATS`] advertises are written. A third
///   one cannot reach here (Smithay refuses a buffer whose format is not in
///   the constraints), but "cannot reach here" is a property of code in
///   another crate.
/// - **The reach of the last row**, in `i64`, so no product of two client
///   numbers can wrap an `i32` or a `usize` on the way to a pointer.
///
/// Returns the failure reason to send the client, never a panic and never a
/// partial write.
fn write_capture(
    buffer: &WlBuffer,
    pixels: &[u8],
    width: i32,
    height: i32,
    stride: usize,
    force_xrgb_alpha: bool,
) -> Result<(), CaptureFailureReason> {
    let row = width as i64 * BYTES_PER_PIXEL as i64;
    with_buffer_contents_mut(buffer, |ptr, len, data| {
        // The caller only ever passes the framebuffer's own size, which is
        // positive by construction -- but this is the one function that turns
        // these numbers into a pointer offset, so it proves the sign itself
        // rather than inheriting it.
        if width <= 0 || height <= 0 {
            return Err(CaptureFailureReason::Unknown);
        }
        if data.width < width || data.height < height || data.offset < 0 {
            return Err(CaptureFailureReason::BufferConstraints);
        }
        let opaque = match data.format {
            // Forced only while the background is actually translucent (see
            // `xrgb_needs_forcing`): over the default opaque background the
            // framebuffer already carries `0xff` everywhere, so the pass
            // would be a no-op -- and an `Argb8888` capture never forces,
            // getting the framebuffer's own alpha, which is what that format
            // means.
            wl_shm::Format::Xrgb8888 => force_xrgb_alpha,
            wl_shm::Format::Argb8888 => false,
            _ => return Err(CaptureFailureReason::BufferConstraints),
        };
        let dst_stride = data.stride as i64;
        if dst_stride < row {
            return Err(CaptureFailureReason::BufferConstraints);
        }
        // The last byte this writes, as an absolute offset into the pool.
        let reach = data.offset as i64 + (height as i64 - 1) * dst_stride + row;
        if reach > len as i64 {
            return Err(CaptureFailureReason::BufferConstraints);
        }
        for y in 0..height as i64 {
            let src = &pixels[y as usize * stride..][..row as usize];
            // SAFETY: `ptr` is valid for `len` bytes for the duration of this
            // closure (`with_buffer_contents_mut`'s contract), and the bounds
            // check above proves `offset + y * dst_stride + row <= len` for
            // every `y` in range. The destination is shared memory the client
            // may mutate concurrently, which is why this writes through the
            // raw pointer rather than materializing a `&mut [u8]` -- a
            // reference into it would be undefined behavior the moment the
            // client touched the same bytes. The `opaque` branch below reads
            // through the same raw pointer for the same reason: it reads back
            // bytes this same loop iteration just wrote, so a concurrent
            // client write can only race with the compositor's own data,
            // never observe uninitialized memory -- the worst case is the
            // client corrupting its own buffer, not a leak of anything else.
            unsafe {
                let dst = ptr.add((data.offset as i64 + y * dst_stride) as usize);
                std::ptr::copy_nonoverlapping(src.as_ptr(), dst, row as usize);
                if opaque {
                    // `Xrgb8888`'s fourth byte is undefined, and the
                    // framebuffer's own alpha is not what a client reading an
                    // opaque format expects to find there. See this module's
                    // doc.
                    //
                    // [`PIXELS_PER_STEP`] pixels per iteration rather than one,
                    // which is the whole reason this stayed a second pass over
                    // the row instead of becoming part of the copy -- see
                    // `OPAQUE`'s doc for the measurements that decided it.
                    let width = width as usize;
                    let steps = width / PIXELS_PER_STEP;
                    const STEP_BYTES: usize = PIXELS_PER_STEP * BYTES_PER_PIXEL as usize;
                    for step in 0..steps {
                        let at = dst.add(step * STEP_BYTES).cast::<[u8; STEP_BYTES]>();
                        // `from_le_bytes`/`to_le_bytes` rather than a native
                        // read: they say "the alpha byte is the high byte of a
                        // little-endian pixel" in as many words, so nothing
                        // here silently changes meaning on a big-endian target.
                        // `*_unaligned`, because `data.offset` and `data.stride`
                        // are the client's own numbers and need not leave this
                        // on any particular boundary.
                        let wide = u128::from_le_bytes(at.read_unaligned()) | OPAQUE;
                        at.write_unaligned(wide.to_le_bytes());
                    }
                    // The 0..3 pixels a width that is not a multiple of
                    // [`PIXELS_PER_STEP`] leaves over -- the same byte, written
                    // one pixel at a time. Reachable for any odd output width,
                    // which `--tty` takes from the connector and does not
                    // choose.
                    for x in steps * PIXELS_PER_STEP..width {
                        dst.add(x * BYTES_PER_PIXEL as usize + 3).write(0xff);
                    }
                }
            }
        }
        Ok(())
    })
    // The buffer is not one `wl_shm` manages, its pool could not be mapped, or
    // the client destroyed it between `capture` and this tick. None of those
    // is something the client can fix by re-allocating, so it is `unknown`
    // rather than `buffer_constraints`.
    .map_err(|_| CaptureFailureReason::Unknown)?
}
