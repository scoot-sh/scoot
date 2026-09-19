//! `zwp_linux_dmabuf_v1`: advertisement *and* import.
//!
//! scoot composites on the CPU with pixman by default, but a client is free
//! to render on the GPU and hand the result over as a dma-buf -- and Smithay's
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
//! refuses is the same client kill with extra steps.
//!
//! ## The table is the active renderer's, not a list
//!
//! Which renderer will do the importing is a per-session fact --
//! `PixmanRenderer` by default, `GlesRenderer` under `--renderer gles`, and a
//! third `GlesRenderer` on its own EGL display under `--tty --renderer gles`
//! (see [`render`](super::render)) -- and their importable sets are not the
//! same. pixman's is a fixed list of fourccs at `LINEAR`; a GLES one is
//! whatever its EGL display reports, which is a property of the driver and
//! the device.
//!
//! So the tranche is **derived**: [`DMABUF_CANDIDATES`] is what this
//! compositor is willing to serve, and [`tranche`] keeps only those the
//! session's own renderer will really take. A candidate it cannot import is
//! never offered, so the promise cannot be broken by a renderer this
//! compositor does not have. Pinned over the wire, against the session's own
//! backend
//! (`dmabuf/tests.rs::every_advertised_format_is_one_the_renderer_imports`),
//! not by comment.
//!
//! **"Will really take" is a wider question than "lists at `LINEAR`", and
//! getting that wrong is a way to break a working session rather than a
//! client.** Smithay inserts `{fourcc, Modifier::Invalid}` into a GLES
//! renderer's import set unconditionally and explicit modifiers only when the
//! driver answered a modifier query -- so a display with no modifiers
//! extension, or a driver that refuses the query for its own enumerated
//! format, lists `{XR24, Invalid}` and not `{XR24, Linear}` while importing a
//! linear dma-buf perfectly well. Requiring the explicit entry would have
//! advertised *nothing* there. [`imports_linear`] is the rule, and its doc
//! carries the chain through the pinned source.
//!
//! **The hazard that closes**, which is the reason the stage exists: an EGL
//! display with *no* dmabuf-import capability has an empty importable set,
//! and the old hard-coded pixman pair would have been advertised against it
//! anyway -- every GL client on that machine allocating a buffer and being
//! killed through `create_immed` for believing the feedback. That case now
//! ends in no global at all ([`advertise`] returns without creating one),
//! which steers the same client onto `wl_shm`: a slower session, not a dead
//! one. The case is real and this project has no machine that produces it, so
//! it is closed by construction and by unit test
//! (`a_renderer_that_imports_nothing_is_advertised_as_nothing`) rather than
//! by a live reproduction.
//!
//! That "no global" outcome is the right answer only when the renderer really
//! cannot import, which is why the evidence rule above matters so much: a
//! false negative in it lands a *working* GPU session in the same place, and
//! there is nothing else left to catch it. `--tty`'s scanout tier used to
//! carry a second, independent check of the same question (refusing to come
//! up at all, so the session fell back to pixman and kept a usable dmabuf
//! path); stage 4 removed it as a duplicate, which is correct only while this
//! rule has no false negatives. [`advertise`]'s warning is the remaining
//! safety net, and says what to do.
//!
//! **What this deliberately does not fix**, because it never was this: the
//! seven `dmabuf/tests.rs` import tests that fail under
//! `SCOOT_TEST_RENDERER=gles` fail identically after it, and that is the
//! expected result rather than a gap. They build their buffers through
//! `/dev/udmabuf`, and Mesa's `kms_swrast` refuses a udmabuf-backed import
//! (`eglCreateImageKHR: createImageFromDmaBufs failed`, `EGL_BAD_ALLOC`) for
//! *either* format, with or without modifier attributes. That is buffer
//! **provenance**, not format: probing the device scoot's own selection picks
//! on the dev VM found both candidates present with `LINEAR` among that
//! display's 76 import formats, so the derived table there names exactly the
//! two the hard-coded one did. No real client reaches that path on that
//! machine either -- `gbm_bo_create` on its render node is refused outright,
//! so nothing there can produce a GBM dmabuf at all.
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
//! which describe the same formats.
//!
//! The default feedback names a **render node** as `main_device`
//! ([`main_device`]: the active renderer's own device where it can name one,
//! else `/dev/dri/renderD128`, else `card0`, else `0`, whichever rung
//! answered logged once at startup) and the derived tranche --
//! [`DMABUF_CANDIDATES`] (`Xrgb8888` then
//! `Argb8888`) minus anything the renderer cannot import -- with the `LINEAR`
//! layout, which is the only layout a CPU mapping can make sense of and the
//! only one every tier here agrees on. A render node rather than a primary
//! one because the device in the feedback is what a client *allocates
//! against*, and a client that only needs to render has no business on a
//! primary node -- scoot itself never scans out of these buffers, it reads
//! them.
//!
//! ## When it is advertised, and why that is not `State::new`
//!
//! [`advertise`] runs from `headless::init_named`, immediately after the
//! render target is built -- not from
//! [`Screencopy::new`](super::screencopy::Screencopy) with every other global,
//! which is where it used to run. It has to: the tranche is derived from a
//! renderer, and `State::new` has none. The backend cannot simply be built
//! earlier either, because its size comes from `tty::init`, which already
//! needs a `&mut State` to exist.
//!
//! **No client can observe the difference**, which is the load-bearing part
//! given that this global gates quickshell's capture readiness (above) and a
//! shell that binds early must not miss it. The wayland listening socket is a
//! calloop source, so no connection is accepted, no registry served and no
//! global announced until `event_loop.run` -- which `compositor::run` reaches
//! several steps after `init_named`, and after the session's own `--` command
//! is even spawned. Every harness a dmabuf test runs against has the same
//! shape -- `State::new`, `headless::init`, *then* a client -- so a global
//! created anywhere in that window is a global that was there from the
//! client's first `wl_registry`.
//!
//! One suite deliberately does *not*, and it is worth knowing which way it
//! cuts: `output_management/tests.rs`'s
//! `binding_before_the_output_exists_announces_no_head` connects a client,
//! binds a manager, and only then calls `headless::init` -- so that client
//! receives this global as a late `wl_registry.global` event. It passes,
//! which is a small piece of evidence that a late announcement is served
//! rather than lost. It is not the argument above, though: that argument is
//! that no *session* has such a window, not that a late global would be
//! broken if one did.
//!
//! The one visible consequence is the honest one: a `State` with no renderer
//! at all (the bare test harness; any future front-end that has none)
//! advertises no dmabuf global, rather than advertising one that could only
//! ever answer `failed`.
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
//!   So scoot drains the cache itself, from the one event that actually
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
//! [`screencopy`](super::screencopy)'s capture globals: scoot has no
//! security-context support, so an allow-list would be theatre. What this
//! global now does hand out is a *mapping of the client's own buffer*, which
//! is the client's memory, not anyone else's -- an import reads one fd the
//! client passed, and nothing else. See `docs/protocols.md`.
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
//! - **Multi-plane and non-`LINEAR` imports stay refused.** The tranche never
//!   offers either: [`DMABUF_CANDIDATES`] is `LINEAR`-only and the renderer
//!   can only narrow *which fourccs* survive, never widen what modifier is
//!   named -- an entry whose evidence was `Modifier::Invalid` is still
//!   advertised as `LINEAR` (see [`imports_linear`], and note that the
//!   widening is on the evidence side alone). Smithay validates the *format*
//!   against the
//!   table but not the modifier or the plane count, so only a client that
//!   ignores the feedback it was sent can reach that refusal. Under pixman
//!   the refusal is `UnsupportedNumberOfPlanes`/`UnsupportedModifier`; under
//!   GLES it is whatever EGL says. On the async `create` path the client gets
//!   the protocol's `failed` event and lives; on `create_immed` it dies, which
//!   is what the protocol prescribes for a buffer the client already believes
//!   it holds.
//! - **An fd that is not really a dma-buf is refused, not trusted.**
//!   `PixmanRenderer::import_dmabuf` syncs plane 0 before it builds the
//!   image, and `DMA_BUF_IOCTL_SYNC` on (say) a plain memfd fails with
//!   `ENOTTY` -- so the import errors out rather than silently mapping
//!   something that is not a dma-buf at all. A GLES tier refuses the same fd
//!   at `eglCreateImageKHR`.
//! - **A renderer-less `State` cannot be asked in the first place.** It
//!   advertises no global (above), so nothing reaches
//!   [`DmabufHandler::dmabuf_imported`]'s no-backend branch through the
//!   protocol. That branch stays regardless: it is the difference between a
//!   refusal and a panic on an `unwrap`, for any future shape in which a
//!   session has a renderer at startup and loses one.
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
//!   *device*, not a connector or a mode, and the tranche is a property of the
//!   renderer, which no hotplug changes -- so `set_default_feedback` (which
//!   would re-send to every bound feedback object at the pinned rev) is never
//!   called. The assumption that rests on, now that the table is derived: a
//!   session's renderer is fixed for its life. `State::resize_output` rebuilds
//!   the backend, but always as the renderer the session started with, and
//!   `GlesBackend::new`'s device enumeration is deterministic within a boot --
//!   so a rebuild lands on the same EGL display and the same importable set.
//!   A future renderer with real per-connector tranche preferences, or a
//!   resize that could migrate to another device, is when this paragraph stops
//!   being true and `set_default_feedback` becomes the fix.
//! - **No-DRM-node logging is once per boot, not per frame.** Which rung of
//!   [`main_device`] answered -- including the `0` fallback -- is logged where
//!   it is chosen, in [`advertise`], which runs once from
//!   `headless::init_named`.
//!   Per-attempt import refusals are `debug!` for the same reason in the
//!   other direction: every dmabuf-capable client tries at least once at
//!   startup, so anything louder would spam the log per client launch. The
//!   one `info!` on this path is the *first* successful import in a session,
//!   logged on the transition only -- that is the line a "why is this client
//!   blank" report needs, and it cannot repeat.
//! - **A feedback build failure skips the global rather than half-advertising.**
//!   `DmabufFeedbackBuilder::build` fails only if the format-table memfd
//!   cannot be created; [`advertise`] then logs and leaves the state with no
//!   global, and the compositor runs exactly as before this module existed --
//!   the same outcome, and the same code path, as a renderer that can import
//!   none of the candidates.
//!   There is deliberately no `DmabufGlobal` handle stored anywhere: the
//!   display owns the advertisement and the state owns the feedback, so a
//!   bare `DmabufState` is everything a static advertisement needs to keep.
//!   That is also why the "global was destroyed" branch of Smithay's params
//!   handler is unreachable here -- nothing ever destroys it.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufSyncFlags};
use smithay::backend::allocator::{Buffer, Format, Fourcc, Modifier};
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{
    SurfaceData, TraversalAction, is_sync_subsurface, with_surface_tree_downward,
};
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf,
};

use super::State;
use super::render::Backend;
use super::wl_buffers::WlBuffers;

/// The dma-buf formats this compositor is willing to advertise, in the order a
/// client sees them in the feedback table -- *before* the active renderer has
/// narrowed them (see [`tranche`], which is what actually reaches a client).
///
/// Every entry the renderer keeps is a format it can really import, which is
/// what `dmabuf/tests.rs::every_advertised_format_is_one_the_renderer_imports`
/// pins over the wire against the session's own backend rather than against a
/// comment. That direction matters and the other does not: a renderer's
/// importable set is much longer than this (pixman's is ten-plus fourccs,
/// GLES's dozens), and advertising *fewer* formats than can be imported costs
/// a client nothing, while advertising one that cannot is a `create_immed`
/// kill.
///
/// Deliberately the same two [`screencopy`](super::screencopy) serves captures
/// in, in the same order (`Xrgb8888` first, for the same translucent-background
/// reason that module's doc gives): a client that allocates from this table and
/// a client that captures into an shm buffer then agree on a pixel layout, and
/// the compositor's own framebuffer is that layout too. Kept as `Fourcc` rather
/// than derived from `screencopy`'s `wl_shm` list so there is no format mapping
/// to get wrong; `dmabuf/tests.rs` pins those two lists to each other as well.
const DMABUF_CANDIDATES: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

/// `/dev/dri/renderD128`, the first rung of [`main_device`]'s path ladder.
const RENDER_NODE: &str = "/dev/dri/renderD128";
/// `/dev/dri/card0`, the second rung of [`main_device`]'s path ladder.
const CARD0: &str = "/dev/dri/card0";

/// Creates the `zwp_linux_dmabuf_v1` global for whatever `backend`'s renderer
/// can really import, or creates nothing at all when it can import none of
/// [`DMABUF_CANDIDATES`] (see the module doc: no global beats one that
/// promises an import this session cannot perform).
///
/// Runs once, from `headless::init_named`, immediately after the render target
/// is built and before the event loop starts -- never per bind, per frame or
/// per hotplug event. See the module doc for why that is still early enough
/// for a client that gates on this global.
pub(super) fn advertise(dh: &DisplayHandle, state: &mut DmabufState, backend: &Backend) {
    let mut formats = tranche(|format| backend.imports_dmabuf_format(format)).peekable();
    if formats.peek().is_none() {
        // Loud, and specific about both halves, because this is a
        // session-shaping degradation an operator has no other way to find
        // out about: every GL client silently drops to Mesa's `wl_shm`
        // swrast path -- software GL, on a configuration someone chose *for*
        // GPU rendering -- and a shell that gates its screen capture on
        // dmabuf feedback (quickshell does; see this module's doc) never
        // creates a capture context at all, so its previews stay blank
        // forever with no error anywhere. Naming the remedy matters as much:
        // pixman imports a linear dma-buf by mapping it and refuses
        // essentially nothing, so `--renderer pixman` is a working session
        // rather than a downgrade to be argued about.
        tracing::warn!(
            candidates = ?DMABUF_CANDIDATES,
            "this session's renderer can import none of the dma-buf formats this \
             compositor serves, so zwp_linux_dmabuf_v1 is not advertised at all: \
             GL clients will fall back to software rendering over wl_shm, and a \
             shell that waits for dmabuf feedback before capturing the screen \
             will never capture anything. Run with --renderer pixman for a \
             session that imports them"
        );
        return;
    }
    let device = main_device(backend.render_node());
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
}

/// The feedback tranche: [`DMABUF_CANDIDATES`] in their advertised order,
/// minus any the active renderer cannot import.
///
/// A predicate rather than a renderer, for two reasons. It is what makes the
/// *decision* testable without the renderer that would have to be there to
/// make it -- including the case that matters most and that no machine here
/// can produce on demand, a renderer that imports nothing. And it states the
/// rule in one place: the filter is the whole of what "renderer-derived"
/// means here.
///
/// Filtering a fixed candidate list rather than advertising the renderer's own
/// set wholesale, which is the other thing "derive it from the renderer" could
/// have meant and is not what this does. The direction is asymmetric:
/// advertising *fewer* formats than can be imported costs a client nothing
/// (it falls back to `wl_shm`), while advertising one that cannot be imported
/// is a `create_immed` kill. GLES on a real driver imports dozens of fourccs,
/// many of them multi-plane or YUV, none of which this compositor's capture
/// path, framebuffer layout or `screencopy` shm list agrees with -- so the
/// candidates stay the two that every other pixel path here already speaks
/// (see [`DMABUF_CANDIDATES`]) and the renderer only ever narrows them.
///
/// What the entries say on the wire is always `LINEAR`; what counts as
/// *evidence* that the renderer will take one is [`imports_linear`], which is
/// wider and has to be.
fn tranche(can_import: impl Fn(Format) -> bool) -> impl Iterator<Item = Format> {
    DMABUF_CANDIDATES
        .into_iter()
        .filter(move |code| imports_linear(*code, &can_import))
        .map(|code| Format {
            code,
            modifier: Modifier::Linear,
        })
}

/// Whether `can_import` is evidence that this renderer will accept a
/// single-plane **linear** dma-buf of `code` -- which is *not* the same
/// question as whether it lists `code` at `Modifier::Linear`.
///
/// `Modifier::Invalid` counts, and leaving it out was a real bug in the first
/// version of this module (caught in review of PR #147, before it could
/// reach anyone). The chain, all checked against the pinned rev rather than
/// reasoned about:
///
/// - Smithay builds a GLES renderer's import set by inserting
///   `{fourcc, Invalid}` **unconditionally** for every fourcc it enumerates,
///   and inserting *explicit* modifiers only when `QueryDmaBufModifiersEXT`
///   answered a non-zero count (`egl/display.rs:962-1001`).
/// - That count stays zero on real hardware, not only in theory: a display
///   without `EGL_EXT_image_dma_buf_import_modifiers` at all, a driver that
///   answers `EGL_BAD_PARAMETER` for a format it just enumerated (upstream's
///   own comment names NVIDIA proprietary >= 520, `:938-953`), or a driver
///   that simply reports no modifiers.
/// - On such a display the import set therefore contains `{XR24, Invalid}`
///   and **not** `{XR24, Linear}` -- while the import itself succeeds:
///   `GlesRenderer::import_dmabuf` never consults the set to admit a buffer
///   (it reads it only for the `is_external` flag, `gles/mod.rs:1268-1274`),
///   `Dmabuf::has_modifier()` is false for `Linear` so the
///   modifiers-extension guard does not fire (`allocator/dmabuf.rs:229`,
///   `egl/display.rs:754-759`), and no modifier attribute is attached to the
///   `EGLImage` either (`:817`) -- i.e. exactly the implicit-layout import
///   such a driver does support.
///
/// So requiring an explicit `LINEAR` entry would have thrown away a
/// capability that genuinely worked, ending in no global at all and every GL
/// client on software rendering (see [`advertise`]'s warning). Accepting
/// `Invalid` restores precisely what the hard-coded table did before this
/// stage and promises nothing more: the advertised modifier is still
/// `LINEAR`, and a client that allocates one still gets the import that used
/// to succeed.
fn imports_linear(code: Fourcc, can_import: &impl Fn(Format) -> bool) -> bool {
    [Modifier::Linear, Modifier::Invalid]
        .into_iter()
        .any(|modifier| can_import(Format { code, modifier }))
}

/// The `main_device` for default feedback: the device a client should allocate
/// against.
///
/// `renderer` is the DRM render node of the device the active renderer is on
/// ([`Backend::render_node`](super::render::Backend) -- its EGL device, or on
/// the scanout tier the GBM device its EGL display was made on, which is the
/// same device), and it leads because it is the only rung that can be
/// *checked* rather than guessed: an import is performed by one specific
/// renderer on one specific device, and on a machine with two GPUs a client
/// that allocates against the other node hands over a dma-buf that renderer
/// cannot import -- which `create_immed` turns into a disconnect. pixman has
/// no device at all (it `mmap`s whatever it is handed, whichever node
/// allocated it), and neither a software EGL device nor a display-only DRM
/// device has a render node, so those fall through to the path ladder below.
///
/// The ladder: `/dev/dri/renderD128` first, `card0` where there is no render
/// node, `0` where there is no DRM node at all -- plausibly *the* production
/// shape on GPU-less containers. `0` is the `dev_t`/kernel convention for "no
/// device", so it degrades to that rather than to a guess. The render node
/// leads there for the same reason it does above: a client that only needs to
/// render into a buffer scoot will read on the CPU has no reason to open a
/// primary node, which needs privileges a render node does not. (On this
/// project's own reference machine `/dev/dri/card0` does not even exist: the
/// Asahi M2 enumerates `card1`/`card2` plus `renderD128`.)
///
/// Whichever rung answers is logged once, here, at startup.
fn main_device(renderer: Option<libc::dev_t>) -> libc::dev_t {
    let (device, source) = match renderer {
        Some(device) => (device, "the renderer's own device"),
        None => main_device_from(Path::new(RENDER_NODE), Path::new(CARD0)),
    };
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
/// **This is pixman's workaround, and only pixman's.** Everything above is
/// true because the renderer composites out of a CPU mapping *this* process
/// made and nothing else synchronises. Both GLES tiers hand the buffer to the
/// driver as an `EGLImage` and never map it here, so the buffer's implicit
/// fences are the driver's to honour when it samples -- what every GL
/// compositor relies on, none of which issues a per-commit
/// `DMA_BUF_IOCTL_SYNC` -- and running it anyway would keep the event-loop
/// block described below for a mapping that no longer exists.
/// `handlers.rs` therefore gates this on
/// [`Backend::maps_dmabufs_on_the_cpu`](super::render::Backend) as well as on
/// the flag below -- a *renderer* question, deliberately kept out of
/// [`State::imports_dmabufs`](super::State), whose other reader (the cache
/// drain) is right for every renderer.
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

    /// Imports the dmabuf into this session's active renderer, answering the
    /// client with what that renderer said. Which renderer that is depends on
    /// `--renderer`; do not assume pixman here.
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
        // The texture the import produced is dropped inside
        // `Backend::import_dmabuf` on purpose: the mapping it names lives in
        // the renderer's own cache (keyed by the dmabuf itself), and the
        // render path re-imports from that cache on every commit. What this
        // call is *for* is establishing that the mapping can be made at all,
        // before the client is told its buffer exists.
        match backend.import_dmabuf(&dmabuf) {
            Ok(()) => {
                if !self.imports_dmabufs {
                    // Once per session, on the transition only: the line a
                    // "why is this client blank / why did it die" report needs
                    // is "a GPU client's buffer really was mapped here", and
                    // repeating it per buffer would log per frame. Everything
                    // after the first import is silent unless it is refused.
                    tracing::info!(
                        format = ?dmabuf.format().code,
                        modifier = ?dmabuf.format().modifier,
                        "imported a client dmabuf into the active renderer"
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
/// `Backend::cleanup_texture_cache` reaches `PixmanRenderer::cleanup`, which
/// retains over **both** of that renderer's caches -- and the second retain
/// drops every entry whose `dmabuf` is `None` (`pixman/mod.rs:807-815`), i.e.
/// it evicts `self.buffers` wholesale rather than dropping expired entries
/// from it. That is free today only because scoot never populates *pixman's*
/// `self.buffers`: it is filled solely by `Bind<Dmabuf>`, and the only bind
/// the pixman pipeline makes hands over a `pixman::Image` (`render.rs`'s
/// `draw_frame`/`Backend::capture`), so the extra retain scans an empty
/// `Vec`.
///
/// **A dmabuf render target does exist now, on one tier, and it does not
/// share this hazard** -- stated because the previous version of this note
/// predicted the arrival and not which renderer it would arrive on. The GPU
/// scanout tier's `Backend::capture` binds the swapchain slot's `Dmabuf`
/// (`render/scanout.rs`), but through `GlesRenderer`, whose `cleanup` retains
/// `self.buffers` on `!dmabuf.is_gone()` alone (`gles/mod.rs:820-824`) rather
/// than evicting entries that have no dmabuf. So a `wl_buffer` destruction
/// there drops nothing live: the capture pool holds its own clone of every
/// exported slot, which is exactly what keeps `is_gone()` false.
///
/// The hazard is still real for the two combinations that have not happened:
/// a *pixman* dmabuf render target, or a dmabuf screencopy path that binds a
/// client's buffer under pixman. Either would make *every* `wl_buffer`
/// destruction in the session evict the bound-target cache and force a
/// re-`mmap` on the next frame. Whoever adds one has to narrow this drain (or
/// upstream's retain) at the same time.
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
/// `Backend::cleanup_texture_cache` is `PixmanRenderer::cleanup` with a
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
    if let Err(error) = backend.cleanup_texture_cache() {
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
