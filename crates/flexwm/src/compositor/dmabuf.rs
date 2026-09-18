//! `zwp_linux_dmabuf_v1`: advertisement *and* import.
//!
//! flexwm composites on the CPU with pixman, but a client is free to render
//! on the GPU and hand the result over as a dma-buf -- and Smithay's
//! [`PixmanRenderer`](smithay::backend::renderer::pixman::PixmanRenderer)
//! imports one without any GPU on the compositor side: it `mmap`s plane 0 of
//! a single-plane `LINEAR` dmabuf and wraps the mapping in a `pixman::Image`
//! with the buffer's own stride. So both halves of the session work, which
//! is the point: a GL client (dmabuf) and a CPU client (`wl_shm`) render
//! side by side through one pixman compositor, with no GPU requirement
//! anywhere.
//!
//! This module owns the global, the feedback it advertises, and the import
//! itself. The advertisement half also gates screen capture: quickshell
//! 0.3.1's `WlBufferManager::isReady` requires real `zwp_linux_dmabuf_v1`
//! feedback before *any* `ScreencopyView` -- including the
//! `ext-image-copy-capture-v1` output path [`screencopy`](super::screencopy)
//! serves over `wl_shm` -- will create its capture context (see
//! `docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md`).
//!
//! ## Why answering `failed` was not an option
//!
//! The first version of this module advertised the global and answered every
//! import [`failed`](smithay::wayland::dmabuf::ImportNotifier::failed), on the
//! reasoning that `failed` is the protocol's own "cannot import for
//! implementation-dependent reasons". That is true of the *asynchronous*
//! `zwp_linux_buffer_params_v1.create` request and false of `create_immed`:
//! at the pinned rev `ImportNotifier::failed` branches on the import kind and,
//! for the immediate one, posts
//! `zwp_linux_buffer_params_v1::Error::InvalidWlBuffer` -- a fatal protocol
//! error (`src/wayland/dmabuf/mod.rs:955-965`). Advertising the global steers
//! Mesa's EGL onto the dmabuf winsys instead of the `wl_shm` swrast one, so
//! the advertisement was *causing* every GL client to allocate a dmabuf and
//! then be killed for it. noctalia v5 died ~100ms into a session that way,
//! taking the whole shell with it
//! (`docs/backlog/resolved/dmabuf-advertised-but-never-imported-done.md`).
//!
//! The consequence that survives the fix: **the tranche is a promise with
//! teeth.** A format or modifier in the feedback table that the renderer then
//! refuses is the same client kill with extra steps, so [`DMABUF_FORMATS`] is
//! pinned to what `PixmanRenderer` can really import -- by test
//! (`dmabuf/tests.rs::every_advertised_format_is_one_pixman_can_import`),
//! not by comment.
//!
//! ## What is advertised, exactly
//!
//! Smithay's `DmabufState` + `DmabufHandler` at the pinned rev, one global via
//! `create_global_with_default_feedback` -- which is what fixes the global's
//! version at **6**: feedback (`get_default_feedback`, v4) needs a v4+
//! global, and Smithay advertises 6 whenever default feedback is present, 3
//! when it is not. There is no version knob to turn here, and none is
//! needed: the probe measured quickshell binding at v5 and mesa's egl queues
//! at v4 against this same v6 global, and both were served the same feedback.
//! A client binding v3 or lower never sees feedback at all -- Smithay answers
//! it with `format`/`modifier` events derived from the main tranche instead,
//! which describe the same two formats.
//!
//! The default feedback names this machine's **render node** as `main_device`
//! ([`main_device`]: `/dev/dri/renderD128`, else `card0`, else `0`, each
//! logged once at startup) and [`DMABUF_FORMATS`] (`Xrgb8888` then
//! `Argb8888`) with the `LINEAR` layout, which is the only layout a CPU
//! mapping can make sense of. The render node comes first because the device
//! in the feedback is what a client *allocates against*, and a client that
//! only needs to render has no business on a primary node -- flexwm itself
//! never scans out of these buffers, it reads them.
//!
//! ## What an import actually does
//!
//! [`DmabufHandler::dmabuf_imported`] calls
//! [`ImportDma::import_dmabuf`](smithay::backend::renderer::ImportDma::import_dmabuf)
//! on the backend's renderer and answers `successful()` or `failed()` with
//! the result. On success the mapping lands in the renderer's own
//! `dmabuf_cache`, and the existing render path picks it up with no further
//! change: `render_elements_from_surface_tree` /
//! `WaylandSurfaceRenderElement` are generic over `ImportAll`, and
//! `handlers.rs` already calls `on_commit_buffer_handler`, which is what
//! re-imports (from the cache) on each commit.
//!
//! Two lifetime facts about that cache, both established against the pinned
//! source rather than assumed, because both are load-bearing:
//!
//! - **The cache does not drain on its own, and rendering is not enough.**
//!   `PixmanRenderer::import_dmabuf` pushes every mapping into `dmabuf_cache`
//!   and holds the dmabuf only weakly; `PixmanRenderer::cleanup` drops the
//!   entries whose `WeakDmabuf` has expired, and it has exactly two call
//!   sites at the pinned rev -- `Renderer::render`
//!   (`src/backend/renderer/pixman/mod.rs:866`) and `cleanup_texture_cache`
//!   (`:892`).
//!
//!   Reaching the first one is not guaranteed by a buffer going away, and an
//!   earlier revision of this module wrongly claimed it was. `render_output`
//!   returns `skipped` before it ever calls `Renderer::render` when there is
//!   no damage (`src/backend/renderer/damage/mod.rs:365-367`), and
//!   `headless::frame_tick` drops the frame timer entirely once nothing needs
//!   a render -- while destroying a `wl_buffer` produces no damage and asks
//!   for no frame. So a client that imports a dmabuf, destroys the
//!   `wl_buffer`, closes its fd and *never commits anything* leaves an
//!   `mmap` and its pinned pages behind, for the process lifetime, and can
//!   repeat that unboundedly. `MAX_BUFFERS_PER_CLIENT` does not bound it: the
//!   live count is back to zero on every iteration, which is exactly the
//!   bypass shape `wl_buffers.rs` exists to catch.
//!
//!   So flexwm drains the cache itself, from the one event that actually
//!   means a mapping may have expired: [`schedule_cache_drain`] is called
//!   from `dispatch.rs`'s `wl_buffer` destruction hook and queues one loop
//!   idle, which calls `cleanup_texture_cache`. An idle rather than the hook
//!   itself, and the ordering is load-bearing rather than incidental: during
//!   `ObjectData::destroyed` the object data *still owns* the `Dmabuf`
//!   (wayland-backend calls `object_data.clone().destroyed(..)` and only
//!   drops its `pending_destructors` vec afterwards --
//!   `rs/server_impl/handle.rs:45-60`), so a `cleanup` run there would find
//!   the `WeakDmabuf` still live and free nothing. `Backend::dispatch_all_clients`
//!   runs that drop before it returns (`rs/server_impl/common_poll.rs:102-103`),
//!   and calloop runs idles after `dispatch_events` returns
//!   (`loop_logic.rs:632-634`, and the same in `run`), so the drain lands in
//!   the same dispatch as the destroy, one step later. One idle per batch, not
//!   per buffer.
//! - **Nothing re-synchronises a cached mapping.** `import_dmabuf` issues
//!   `DMA_BUF_IOCTL_SYNC(START|READ)` immediately followed by `(END|READ)`
//!   **once, at import time** (`pixman/mod.rs:772-773`), and `existing_dmabuf`
//!   returns the cached image on every later import without syncing at all
//!   (`pixman/mod.rs:1189`). A GL client re-rendering into one dmabuf across
//!   frames -- the normal case -- would therefore be composited from whatever
//!   the CPU's caches happened to hold, with no wait on the client's
//!   still-running GPU render. That is why [`sync_committed_dmabufs`] exists:
//!   it issues the same bracket at the cadence the client actually rewrites
//!   the buffer, on commit. See its doc for what the bracket buys.
//!
//! ## Trust model
//!
//! No client filter, the same deliberate consistency as
//! [`screencopy`](super::screencopy)'s capture globals: flexwm has no
//! security-context support, so an allow-list would be theatre. What this
//! global now does hand out is a *mapping of the client's own buffer*, which
//! is the client's memory, not anyone else's -- an import reads one fd the
//! client passed, and nothing else. See `README.md`.
//!
//! ## Edge cases, stated rather than re-derived
//!
//! - **`create_params` with garbage is `failed` or a protocol error, never a
//!   panic.** A valid format/modifier/plane set reaches
//!   [`DmabufHandler::dmabuf_imported`](smithay::wayland::dmabuf::DmabufHandler::dmabuf_imported)
//!   and is answered by what the renderer says. An unknown format never gets
//!   that far -- Smithay posts `InvalidFormat`/`InvalidDimensions`/`OutOfBounds`
//!   on the params object, which disconnects that client and no one else.
//!   Both are covered in `dmabuf/tests.rs`.
//! - **Multi-plane and non-`LINEAR` imports stay refused**, because pixman
//!   refuses them (`UnsupportedNumberOfPlanes`, `UnsupportedModifier`). The
//!   tranche never offers either, and Smithay validates the *format* against
//!   the table but not the modifier or the plane count, so only a client that
//!   ignores the feedback it was sent can reach that refusal. On the async
//!   `create` path it gets the protocol's `failed` event and lives; on
//!   `create_immed` it dies, which is what the protocol prescribes for a
//!   buffer the client already believes it holds.
//! - **An fd that is not really a dma-buf is refused, not trusted.**
//!   `PixmanRenderer::import_dmabuf` syncs plane 0 before it builds the
//!   image, and `DMA_BUF_IOCTL_SYNC` on (say) a plain memfd fails with
//!   `ENOTTY` -- so the import errors out rather than silently mapping
//!   something that is not a dma-buf at all.
//! - **A renderer-less `State` refuses every import.** The bare test harness
//!   has `backend: None`; so would any future front-end with no renderer.
//!   `failed()` is the honest answer there, and it is the *only* case left in
//!   which a well-formed import is refused.
//! - **What bounds the mappings a client can make this compositor hold.**
//!   `MAX_BUFFERS_PER_CLIENT` (512, `wl_buffers.rs`), now that the async
//!   `create` path claims too -- and each mapping's *size* is bounded by the
//!   dma-buf the client actually got the kernel to allocate, since Smithay
//!   seeks the plane fd and refuses an offset/stride/height that runs past its
//!   real end. So a client cannot claim address space it did not first pay for
//!   in real pages, which is why there is deliberately no second, byte-sized
//!   cap here the way `wl_shm` pools have one: an shm pool's size is a number
//!   the client sends, a dma-buf's is a fact about the fd.
//! - **Bind/unbind storms cost nothing here.** Feedback is built once, at
//!   startup; Smithay re-sends the stored copy to each new `get_default_feedback`
//!   without calling back into this module. There is no per-bind work to
//!   storm.
//! - **Hotplug and mode changes need no re-send.** The feedback names the DRM
//!   *device*, not a connector or a mode, and [`DMABUF_FORMATS`] is a property
//!   of the renderer, which no hotplug changes -- so `set_default_feedback` is
//!   never called. If a future renderer grows real per-connector tranche
//!   preferences, that is when this paragraph stops being true.
//! - **No-DRM-node logging is once per boot, not per frame.** The
//!   `main_device = 0` fallback is logged where it is chosen, in
//!   [`advertise`], which runs once in [`Screencopy::new`](super::screencopy::Screencopy).
//!   Per-attempt import refusals are `debug!` for the same reason in the
//!   other direction: every dmabuf-capable client tries at least once at
//!   startup, so anything louder would spam the log per client launch. The
//!   one `info!` on this path is the *first* successful import in a session,
//!   logged on the transition only -- that is the line a "why is this client
//!   blank" report needs, and it cannot repeat.
//! - **A feedback build failure skips the global rather than half-advertising.**
//!   `DmabufFeedbackBuilder::build` fails only if the format-table memfd
//!   cannot be created; [`advertise`] then logs and returns a state with no
//!   global, and the compositor runs exactly as before this module existed.
//!   There is deliberately no `DmabufGlobal` handle stored anywhere: the
//!   display owns the advertisement and the state owns the feedback, so a
//!   bare `DmabufState` is everything a static advertisement needs to keep.
//!   That is also why the "global was destroyed" branch of Smithay's params
//!   handler is unreachable here -- nothing ever destroys it.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufSyncFlags};
use smithay::backend::allocator::{Buffer, Format, Fourcc, Modifier};
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::backend::renderer::{ImportDma, Renderer};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{
    SurfaceData, TraversalAction, is_sync_subsurface, with_surface_tree_downward,
};
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf,
};

use super::State;
use super::wl_buffers::WlBuffers;

/// The dmabuf formats this compositor advertises, in the order a client sees
/// them in the feedback table.
///
/// Every entry is a format `PixmanRenderer::import_dmabuf` can really map --
/// which is what `dmabuf/tests.rs::every_advertised_format_is_one_pixman_can_import`
/// pins, against the renderer's own `dmabuf_formats()` rather than against a
/// comment. That direction matters and the other does not: pixman's list is
/// much longer (ten-plus fourccs), and advertising *fewer* formats than can be
/// imported costs a client nothing, while advertising one that cannot is a
/// `create_immed` kill.
///
/// Deliberately the same two [`screencopy`](super::screencopy) serves captures
/// in, in the same order (`Xrgb8888` first, for the same translucent-background
/// reason that module's doc gives): a client that allocates from this table and
/// a client that captures into an shm buffer then agree on a pixel layout, and
/// the compositor's own framebuffer is that layout too. Kept as `Fourcc` rather
/// than derived from `screencopy`'s `wl_shm` list so there is no format mapping
/// to get wrong; `dmabuf/tests.rs` pins those two lists to each other as well.
const DMABUF_FORMATS: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

/// Creates the `zwp_linux_dmabuf_v1` global, or returns a bare state when the
/// feedback cannot be built (see the module doc: no global beats a
/// half-advertised one).
///
/// Runs once, from [`Screencopy::new`](super::screencopy::Screencopy) -- never
/// per bind, per frame or per hotplug event.
pub(super) fn advertise(dh: &DisplayHandle) -> DmabufState {
    let mut state = DmabufState::new();
    let device = main_device();
    let formats = DMABUF_FORMATS.iter().map(|code| Format {
        code: *code,
        modifier: Modifier::Linear,
    });
    match DmabufFeedbackBuilder::new(device, formats).build() {
        Ok(feedback) => {
            state.create_global_with_default_feedback::<State>(dh, &feedback);
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "dmabuf feedback table could not be built; running without zwp_linux_dmabuf_v1"
            );
        }
    }
    state
}

/// The `main_device` for default feedback: this machine's render node.
///
/// `/dev/dri/renderD128` first, `card0` where there is no render node, `0`
/// where there is no DRM node at all -- plausibly *the* production shape on
/// GPU-less containers -- each logged once, here, at startup. `0` is the
/// `dev_t`/kernel convention for "no device", so the fallback ladder degrades
/// to that rather than to a guess.
///
/// The render node leads because the device named here is the one clients
/// *allocate against* now that imports really happen: a client that only needs
/// to render into a buffer flexwm will read on the CPU has no reason to open a
/// primary node, which needs privileges a render node does not. flexwm itself
/// never scans out of an imported buffer -- `--tty` scans out of its own dumb
/// buffers -- so the primary node was never the right hint, only the more
/// visible one. (On this project's own reference machine `/dev/dri/card0` does
/// not even exist: the Asahi M2 enumerates `card1`/`card2` plus `renderD128`.)
fn main_device() -> libc::dev_t {
    /// Bound to `PathBuf` (rather than `&str`) so the ladder below reads as
    /// data, not as three near-identical `metadata` calls.
    const RENDER: &str = "/dev/dri/renderD128";
    const CARD0: &str = "/dev/dri/card0";
    let (device, source) = main_device_from(&PathBuf::from(RENDER), &PathBuf::from(CARD0));
    tracing::info!(device, source, "dmabuf feedback main device");
    device
}

/// The ladder [`main_device`] logs, split out so a test can drive it with
/// paths it controls. Returns the device and which rung answered, for the
/// log line above.
fn main_device_from(render: &Path, card0: &Path) -> (libc::dev_t, &'static str) {
    if let Some(device) = node_rdev(render) {
        return (device, "/dev/dri/renderD128");
    }
    if let Some(device) = node_rdev(card0) {
        return (device, "/dev/dri/card0");
    }
    (0, "no DRM node")
}

/// The `rdev` of `path`, or `None` when it names nothing that can be
/// `stat`ed. A path that exists but is not a device node (a regular file, a
/// directory) reports an `rdev` of 0, which is indistinguishable from -- and
/// therefore correctly handled as -- "no device".
fn node_rdev(path: &Path) -> Option<libc::dev_t> {
    let rdev = std::fs::metadata(path).ok()?.rdev();
    // `rdev()` is 0 for non-device files; only a real device node answers the
    // question this ladder asks. But `Some(0)` must still mean "answered":
    // returning `None` for a present-but-not-device path would fall through
    // to the next rung and log the wrong source.
    Some(rdev)
}

/// Waits for the GPU to finish the frame a client just committed, so what the
/// compositor later reads out of the mapping is a whole frame rather than half
/// of one.
///
/// The pinned rev does this exactly once per dmabuf and never again:
/// `PixmanRenderer::import_dmabuf` issues `DMA_BUF_IOCTL_SYNC(START|READ)` and
/// `(END|READ)` at import (`pixman/mod.rs:772-773`), and `existing_dmabuf`
/// re-serves the cached mapping on every later commit without syncing at all
/// (`pixman/mod.rs:1189`) -- so a client re-rendering into one dmabuf across
/// frames, the normal GL case, is composited from whatever state the buffer
/// happens to be in. This issues the same pair at the cadence the client
/// actually rewrites the buffer.
///
/// **What the pair does and does not buy, precisely** -- the two halves are
/// not equal here, and the honest reading matters more than the tidy one:
///
/// - `START|READ` enters `dma_buf_begin_cpu_access`, which *waits on the
///   buffer's reservation fences*. That is the load-bearing half, and it lands
///   correctly: when it returns, the client's GPU job for that buffer is done,
///   so the later read cannot see a half-drawn frame.
/// - The cache-invalidation half does **not** survive to the read. `START`
///   also invalidates the CPU's view, but `END|READ` closes the access window
///   immediately afterwards and pixman reads the mapping later, at render. So
///   this is *not* a correctly bracketed CPU access and must not be described
///   as one. It is issued as an adjacent pair because that is what upstream
///   does at import, because the kernel expects begin/end to balance, and
///   because the half that actually keeps a torn frame off screen is complete
///   when `START` returns. A genuinely bracketed read would mean holding the
///   window open from commit across render, for which the renderer's API
///   offers no seam at the pinned rev. On the coherent mappings a LINEAR
///   dmabuf gives on this project's hardware the invalidate is a no-op anyway;
///   on an architecture where it is not, this is the paragraph that says what
///   would have to change.
///
/// Commit time, not render time, because commit is precisely "the client has
/// finished writing this buffer": there is one sync per buffer the client
/// actually rewrote, rather than one per frame per surface (a pointer motion
/// can redraw an unchanged surface many times over).
///
/// **This blocks the event loop for as long as the client's GPU job takes,
/// while holding that surface's user-data mutex**, and that is a known cost
/// rather than an oversight. The wait is the whole point (above). The mutex
/// comes from where the wait happens: `with_surface_tree_downward` runs its
/// processor closure inside `TreeSurfaceData::map`, which holds
/// `lock_user_data(surface)` -- and every ancestor's, since it recurses while
/// holding -- across the call (`wayland/compositor/tree.rs:500-520`). Benign
/// in this single-threaded design, where nothing else can contend for those
/// locks while the loop is inside the ioctl, but it is a real widening of
/// what a hung client GPU job stalls, so it is written down rather than left
/// to be discovered.
///
/// The exposure that buys is a client whose GPU job hangs stalling the loop
/// until the driver resets it (drivers time out and force-signal; the kernel
/// wait itself has no deadline). Taken knowingly, because the alternative --
/// what the pinned rev does, which is not to wait at all -- is a torn frame
/// for every GL client. A non-blocking version means polling the plane fd for
/// readability and deferring the frame, machinery this item does not need and
/// explicit sync would supersede.
///
/// Walks the subtree for the same reason
/// [`on_commit_buffer_handler`](smithay::backend::renderer::utils::on_commit_buffer_handler)
/// does, and skips a synchronized subsurface for the same reason it does: that
/// surface's buffer is not applied until its parent commits, so syncing it
/// here would sync the *previous* buffer and miss the new one. The parent's
/// commit reaches it.
///
/// Only called when [`State::imports_dmabufs`](super::State) says some import
/// has actually succeeded in this session, so an shm-only session never pays
/// for this walk at all (see that field's doc). That gate is session-wide, not
/// per client, so once any client imports a dmabuf every commit of every
/// client walks its own tree -- measured, because a per-event path in this
/// project gets a number rather than an argument
/// (`dmabuf/tests.rs::commit_sync_cost`, release build, this machine,
/// 20k rounds on a one-surface tree):
///
/// ```text
/// gate off (no dmabuf in the session): not called -- one bool test in `commit`
/// gate armed, surface has no dmabuf:   99ns per commit
/// gate armed, surface has a dmabuf:    1.39us per commit
/// ```
///
/// The middle row is what an innocent `wl_shm` client pays for a GL
/// neighbour: 99ns against a commit that already walks this same tree once in
/// `on_commit_buffer_handler`. Twenty surfaces at 60Hz is ~120us per second,
/// or 0.01% of a core. The last row is the client the work is for, and is
/// almost entirely the two ioctls (no fences to wait on in that measurement;
/// a real GPU job makes it as long as that job takes, by design -- above).
pub(super) fn sync_committed_dmabufs(surface: &WlSurface) {
    if is_sync_subsurface(surface) {
        return;
    }
    with_surface_tree_downward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_, states, _| sync_surface_dmabuf(states),
        |_, _, _| true,
    );
}

/// [`sync_committed_dmabufs`] for one surface: a no-op unless that surface's
/// current buffer is a dmabuf.
///
/// Plane 0 only, because plane 0 is all pixman ever maps (`import_dmabuf`
/// refuses anything multi-plane outright), so it is the only plane whose
/// contents this compositor can read stale.
///
/// A sync failure is logged at `debug` and otherwise ignored: the import that
/// let this buffer exist already performed the same ioctl successfully, so a
/// failure here means the fd changed under us or the exporter withdrew
/// `begin_cpu_access` -- neither a reason to tear down a client that is
/// otherwise behaving, and neither something a retry would fix.
fn sync_surface_dmabuf(states: &SurfaceData) {
    let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() else {
        return;
    };
    // `on_commit_buffer_handler` took and released this very lock immediately
    // before this runs (`handlers.rs`), on this thread, so it is neither held
    // nor poisoned by the time we get here -- a poisoned one would have
    // panicked there first.
    let state = data.lock().expect("renderer surface state");
    let Some(buffer) = state.buffer() else {
        return;
    };
    let Ok(dmabuf) = get_dmabuf(buffer) else {
        return;
    };
    for (phase, flags) in [
        ("start", DmabufSyncFlags::START | DmabufSyncFlags::READ),
        ("end", DmabufSyncFlags::END | DmabufSyncFlags::READ),
    ] {
        if let Err(error) = dmabuf.sync_plane(0, flags) {
            tracing::debug!(%error, phase, "dmabuf plane sync failed on commit");
            return;
        }
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.screencopy.dmabuf
    }

    /// Imports the dmabuf into this session's pixman renderer, answering the
    /// client with what the renderer said.
    ///
    /// A success mints the client's `wl_buffer` and leaves an `mmap` of plane
    /// 0 in the renderer's cache, which the ordinary surface render path then
    /// composites like any other texture. A refusal is the protocol's own
    /// `failed` -- soft on the asynchronous `create`, fatal on `create_immed`,
    /// which is the protocol's choice, not this compositor's (see the module
    /// doc).
    ///
    /// Logged at `debug`: every dmabuf-capable client imports at least once at
    /// startup and a busy one imports per buffer, so anything louder would log
    /// per client launch (see the module doc).
    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let Some(backend) = self.backend.as_mut() else {
            tracing::debug!(
                format = ?dmabuf.format().code,
                "dmabuf import refused: this session has no renderer"
            );
            refuse_import(&mut self.wl_buffers, notifier);
            return;
        };
        // The texture is dropped right here on purpose: the mapping it names
        // lives in the renderer's `dmabuf_cache` (keyed by the dmabuf itself),
        // and the render path re-imports from that cache on every commit. What
        // this call is *for* is establishing that the mapping can be made at
        // all, before the client is told its buffer exists.
        match ImportDma::import_dmabuf(&mut backend.renderer, &dmabuf, None) {
            Ok(_texture) => {
                if !self.imports_dmabufs {
                    // Once per session, on the transition only: the line a
                    // "why is this client blank / why did it die" report needs
                    // is "a GPU client's buffer really was mapped here", and
                    // repeating it per buffer would log per frame. Everything
                    // after the first import is silent unless it is refused.
                    tracing::info!(
                        format = ?dmabuf.format().code,
                        modifier = ?dmabuf.format().modifier,
                        "imported a client dmabuf into the pixman renderer"
                    );
                    self.imports_dmabufs = true;
                }
                if let Err(error) = notifier.successful::<State>() {
                    // The client died between allocating and being told --
                    // nothing to release, since the buffer object was never
                    // created and its claim lands on an entry with no live
                    // client (see `wl_buffers.rs`).
                    tracing::debug!(%error, "dmabuf imported, but the client was already gone");
                }
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    format = ?dmabuf.format().code,
                    modifier = ?dmabuf.format().modifier,
                    planes = dmabuf.num_planes(),
                    "dmabuf import refused by the renderer"
                );
                refuse_import(&mut self.wl_buffers, notifier);
            }
        }
    }
}

/// Queues the one loop idle that drops expired entries from the renderer's
/// dmabuf cache, if this session has any and one is not queued already.
///
/// Called from `dispatch.rs`'s `wl_buffer` destruction hook -- for buffers of
/// every kind, because that hook cannot observe which kind died (see
/// `wl_buffers.rs`), and an extra scan of a short `Vec` is cheaper than the
/// bookkeeping to find out. Gated on
/// [`State::imports_dmabufs`](super::State), so a session that has never
/// imported one never queues anything at all.
///
/// Why an idle and not the hook itself, and why this is the whole fix rather
/// than a belt-and-braces addition to rendering: see the module doc's cache
/// section. Short version -- the `Dmabuf` is still owned by the object data
/// whose `destroyed` is running, so a drain there frees nothing; and a
/// destroyed buffer causes no damage, so nothing guarantees a later frame.
///
/// One idle per batch: `pending` is what keeps a client destroying 512
/// buffers in one dispatch from queueing 512 scans. The same batching
/// argument (and pattern) as `bind_budget.rs`'s deferred refusals.
///
/// ## Maintenance hazard: `cleanup_texture_cache` is not dmabuf-only
///
/// `Renderer::cleanup_texture_cache` reaches `PixmanRenderer::cleanup`, which
/// retains over **both** of that renderer's caches -- and the second retain
/// drops every entry whose `dmabuf` is `None` (`pixman/mod.rs:807-815`), i.e.
/// it evicts `self.buffers` wholesale rather than dropping expired entries
/// from it. That is free today only because flexwm never populates
/// `self.buffers`: it is filled solely by `Bind<Dmabuf>`, and every `bind`
/// call here hands over a `pixman::Image` or a dumb buffer instead
/// (`headless.rs`, `test_support.rs`), so the extra retain scans an empty
/// `Vec`.
///
/// The moment flexwm binds a dmabuf render target -- a future GPU tier, or a
/// dmabuf screencopy path -- that stops being free: *every* `wl_buffer`
/// destruction in the session would then evict the bound-target cache and
/// force a re-`mmap` on the next frame. Whoever adds that has to narrow this
/// drain (or upstream's retain) at the same time.
pub(super) fn schedule_cache_drain(state: &mut State) {
    if !state.imports_dmabufs || state.dmabuf_drain_queued {
        return;
    }
    state.dmabuf_drain_queued = true;
    state.loop_handle.insert_idle(drain_cache);
}

/// Drops every dmabuf mapping whose buffer has gone, and clears the flag that
/// lets the next destruction queue another drain.
///
/// `Renderer::cleanup_texture_cache` is `PixmanRenderer::cleanup` with a
/// `Result` around it (`pixman/mod.rs:891-894`); the pixman implementation
/// cannot fail, so the error arm is for a renderer this compositor does not
/// have yet. It logs rather than propagating: there is nothing a caller on
/// the idle queue could do about it, and a session that cannot drain its
/// cache is still a session worth keeping up.
fn drain_cache(state: &mut State) {
    state.dmabuf_drain_queued = false;
    let Some(backend) = state.backend.as_mut() else {
        return;
    };
    if let Err(error) = Renderer::cleanup_texture_cache(&mut backend.renderer) {
        tracing::debug!(%error, "dropping expired dmabuf mappings failed");
    }
}

/// Answers an import `failed`, handing back the live-buffer unit
/// `dispatch.rs` claimed for the creation request that got here.
///
/// Both dmabuf factories claim before delegating (see `wl_buffers.rs`), and
/// on this path neither produces the `wl_buffer` whose destruction would
/// otherwise release the unit:
///
/// - `create` (asynchronous): the client is told `failed` and *lives*, so
///   without this release a client that keeps offering buffers the renderer
///   cannot map would ratchet its own count to the cap and then be refused
///   outright. This is the one release that has to happen.
/// - `create_immed`: the client is killed by `failed()` itself, and the
///   `wl_buffer` Smithay initialised before calling in here *will* reach the
///   destruction hook during disconnect cleanup -- so the unit is released
///   twice. `WlBuffers::forget_buffer` saturates, the entry belongs to a
///   client that is already dead, and no live client's budget is touched.
///
/// `ImportNotifier` does not say which kind it is (`Import` is private at the
/// pinned rev), which is why this is written to be correct for both rather
/// than branching.
fn refuse_import(buffers: &mut WlBuffers, notifier: ImportNotifier) {
    if let Some(client) = notifier.client() {
        buffers.forget_buffer(&client.id());
    }
    notifier.failed();
}

#[cfg(test)]
mod tests;
