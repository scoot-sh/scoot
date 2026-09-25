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
//! So the tranche is **derived**, and how depends on what kind of answer the
//! renderer gives ([`ImportSet`], from `Backend::dmabuf_import_set`):
//!
//! - **pixman maps the buffer itself**, so the only layout it can promise is
//!   a single-plane `LINEAR` one. Its tranche is [`DMABUF_CANDIDATES`] --
//!   what this compositor's CPU pixel paths speak -- narrowed to those pixman
//!   really takes ([`cpu_mapped_tranche`]). Byte-identical to what it has
//!   always been.
//! - **a GLES renderer hands the buffer to its driver**, and the driver has
//!   already said what it imports, fourcc *and* modifier: tiled and
//!   compressed layouts on a real GPU, multi-plane YUV (`NV12`, `P010`, ...)
//!   as external-only textures. Its tranche is that answer
//!   ([`driver_tranche`]), with Smithay's unconditional `Modifier::Invalid`
//!   entries resolved rather than passed through -- see that function for
//!   the rule and why implicit layouts are never offered next to explicit
//!   ones. This is what lets a GPU client render into its native layout
//!   instead of a linear one, and a video player hand over a decoder's
//!   frame without converting it.
//!
//! Either way a format the renderer cannot import is never offered, so the
//! promise cannot be broken by a renderer this compositor does not have.
//! Pinned over the wire, against the session's own backend
//! (`dmabuf/tests.rs::every_advertised_format_is_one_the_renderer_imports`),
//! and end to end -- allocated, handed over through `create_immed`, and
//! checked on screen for the right colour -- for one representative of each
//! layout class the table carries (`dmabuf/tests/layouts.rs`).
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
//! *any* format, with or without modifier attributes. That is buffer
//! **provenance**, not format: the same driver imports and draws every
//! layout `dmabuf/tests/layouts.rs` builds -- `NV12`, `P010` and three-plane
//! `YU12` included -- from a dumb buffer the device allocated itself. No
//! real client reaches the refused path on that machine either:
//! `gbm_bo_create` on its render node is refused outright, so a Mesa client
//! there renders on the CPU and hands over `wl_shm` instead.
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
//! answered logged once at startup) and one tranche, [`advertised_formats`]:
//!
//! - **pixman**: [`DMABUF_CANDIDATES`] (`Xrgb8888` then `Argb8888`) minus
//!   anything it cannot import, at `LINEAR` -- the only layout a CPU mapping
//!   can make sense of.
//! - **GLES** (both tiers): the same two first wherever the driver lists
//!   them at `LINEAR`, then every other fourcc the driver imports, each at
//!   every explicit modifier the driver named, in the driver's order. On the
//!   dev VM's llvmpipe that is 57 fourccs at `LINEAR` (Mesa lists nothing
//!   else there); on a real GPU it is the driver's tiled and compressed
//!   modifiers too, commonly a few hundred pairs.
//!
//! A render node rather than a primary one because the device in the
//! feedback is what a client *allocates against*, and a client that only
//! needs to render has no business on a primary node. The tranche carries no
//! `scanout` flag: which client buffers could be scanned out directly is a
//! per-surface question, answered on the GPU scanout tier by a second,
//! scanout tranche in front of this one for the fullscreen window covering
//! an output ([`scanout`]), not by the default feedback every client reads.
//! That per-surface feedback is built from this very table and builder
//! ([`DefaultFeedback`]), so it offers nothing this one does not.
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
//! - **What a client that ignores its feedback gets is the renderer's own
//!   answer.** Smithay validates the *fourcc* against the table but not the
//!   modifier or the plane count, so a client can send a layout the table
//!   never offered. Under pixman a multi-plane or non-`LINEAR` buffer is
//!   refused (`UnsupportedNumberOfPlanes`/`UnsupportedModifier`); under GLES
//!   it is whatever EGL says -- including an *implicit*-modifier buffer
//!   (`Modifier::Invalid`), which EGL accepts and which, for a YUV format,
//!   Smithay binds as `GL_TEXTURE_2D` and draws as the wrong colour (measured
//!   on llvmpipe: the `layouts.rs` red fill drew zero red pixels at
//!   `Invalid`, all of them at `LINEAR`). That is why the table never offers
//!   it; a client that allocates implicitly anyway gets a wrong picture, not
//!   a kill, on that driver. On the async `create` path a refusal is the protocol's `failed`
//!   event and the client lives; on `create_immed` it dies, which is what the
//!   protocol prescribes for a buffer the client already believes it holds.
//! - **A pre-feedback client can still reach the implicit path, by design of
//!   the protocol.** A v1/v2 bind is told fourccs only (`format` events),
//!   with no layout; Smithay sends one for every fourcc whose modifiers
//!   include `LINEAR` or `Invalid` (`wayland/dmabuf/dispatch.rs`, `bind`),
//!   which under GLES now includes the YUV ones. Such a client allocates
//!   implicitly and sends `Invalid`, and a YUV buffer imported that way draws
//!   the wrong colours as above -- a wrong picture rather than a kill on the
//!   one driver measured (llvmpipe; a driver refusing implicit YUV would
//!   refuse the import instead), and only for a client too old to have been
//!   told better. Modern clients bind v4+.
//! - **Alpha-carrying YUV (`AYUV`, `Y410`, ...) composites as opaque under
//!   GLES.** Smithay's `has_alpha` knows no YUV fourcc, so the renderer and
//!   the damage/occlusion code both treat such a buffer as opaque -- the two
//!   agree, so the result is a consistent opaque window, never a hole or
//!   garbage. A translucent YUV window is rare enough not to be worth a
//!   hand-kept list here.
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
//!   The per-client fd ledger (`client_fds.rs`): every plane's fd is counted
//!   from its `add` until it closes, so a buffer a surface still has
//!   committed after its `wl_buffer` is destroyed stays counted, and a
//!   mapping cannot outlive its plane's `Dmabuf`, which holds the fds.
//!   `MAX_BUFFERS_PER_CLIENT` (512, `wl_buffers.rs`) bounds the live buffer
//!   objects on top of that. Each mapping's *size* is bounded by the
//!   dma-buf the client actually got the kernel to allocate, since Smithay
//!   seeks the plane fd and refuses an offset/stride/height that runs past its
//!   real end. So a client cannot claim address space it did not first pay for
//!   in real pages, which is why there is deliberately no second, byte-sized
//!   cap here the way `wl_shm` pools have one: an shm pool's size is a number
//!   the client sends, a dma-buf's is a fact about the fd.
//! - **What bounds the plane fds a client can make this compositor hold
//!   before any buffer exists.** Each `zwp_linux_buffer_params_v1.add` hands
//!   over an fd that the params object keeps until it is consumed or
//!   destroyed. [`pending_planes`] caps those at 32 per client (8 under fd
//!   pressure), disconnecting with `wl_display.error(no_memory)` past it.
//!   Before that bound, 220 params objects x 4 adds held 927 fds uncounted.
//! - **Bind/unbind storms: bounded by Smithay and wayland-backend, not by
//!   `bind_budget.rs`.** Feedback is built once, at startup; Smithay re-sends
//!   the stored copy to each `get_default_feedback` without calling back into
//!   this module -- one fd and one `tranche_formats` array of 2 bytes per
//!   entry, whatever the table's length. What *does* scale with the table is
//!   a pre-feedback bind: Smithay answers every v3 bind with one `modifier`
//!   event per entry (v1/v2: one `format` per fourcc) straight from `bind`
//!   (`wayland/dmabuf/dispatch.rs`). This global is **not** one of the four
//!   `bind_budget.rs` counts, and deliberately stays out of it: that budget
//!   bounds binds whose *retained state* grows with the session (a handle per
//!   window), while a dmabuf bind retains one small object whatever the table
//!   and costs only the events it sends. Measured
//!   (`dmabuf/tests.rs::bind_storm_cost`, release, dev VM, 2000 binds, v3
//!   minus v4): the llvmpipe table's 57 events cost 6.4 µs per bind, a
//!   synthetic 408-entry table's 25.8-26.3 µs (~64 ns an event), on top of
//!   the ~5 µs any bind costs. So a v3 storm turns each bind request (at
//!   least 40 bytes: `wl_registry.bind` carries the interface name) into 20
//!   bytes of `modifier` event per entry -- ~1.1 KB at 57 entries, ~8 KB at
//!   408 -- and ~2-5x the compositor CPU of a bare bind, at the client's own
//!   request rate. It is bounded by the client reading its replies:
//!   wayland-backend buffers at most 4 KB per client in user space, and once
//!   a flush finds the client's socket full it disconnects the client
//!   (`rs/server_impl/client.rs`, `write_message` failing). The socket
//!   holds roughly the kernel's send buffer (`wmem_default`, 212992 bytes on
//!   the dev VM, less per-message overhead), so a storm that never reads
//!   ends itself after roughly 25 binds at 408 entries, or roughly 190 at
//!   57. Current Mesa and quickshell bind v4/v5 and never take this path.
//! - **Hotplug and mode changes need no re-send.** The feedback names the DRM
//!   *device*, not a connector or a mode, and the tranche is a property of the
//!   renderer, which no hotplug changes -- so `set_default_feedback` (which
//!   would re-send to every bound feedback object at the pinned rev) is never
//!   called. The assumption that rests on, now that the table is derived and
//!   carries a GLES driver's own tiled modifiers: a session's renderer *and
//!   its device* are fixed for its life. `State::resize_output` keeps a
//!   GLES backend's renderer and reallocates only its target, so it cannot
//!   move device at all; where it or `headless::add_output` does build a new
//!   backend, it is the renderer the session started with, and a GLES
//!   rebuild is pinned to the EGL device the first build landed on
//!   (`render::gles::GlesDevice`, handed down through `State::gles_device`):
//!   if that device cannot build, the resize is refused or the output is not
//!   added, rather than the backend migrating to a device whose driver may
//!   refuse layouts this table promised -- which through `create_immed` is a
//!   client kill. So every backend is on the same EGL display and has the
//!   same importable set, by construction rather than by enumeration order.
//!   A future renderer with real per-connector tranche preferences, or a
//!   deliberate device migration, is when this paragraph stops being true and
//!   `set_default_feedback` becomes the fix. (The per-surface scanout
//!   feedback is the one thing that *does* follow the plane: it is rebuilt on
//!   a CRTC switch and re-sent to the window holding it -- see [`scanout`].)
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
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Buffer, Format, Fourcc, Modifier};
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{
    SurfaceData, TraversalAction, is_sync_subsurface, with_surface_tree_downward,
};
use smithay::wayland::dmabuf::{
    DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState,
    ImportNotifier, get_dmabuf,
};

use super::State;
use super::render::{Backend, ImportSet};
use super::wl_buffers::WlBuffers;

/// The dma-buf formats a **CPU-mapping** renderer (pixman) is advertised, in
/// the order a client sees them -- *before* the renderer has narrowed them
/// (see [`cpu_mapped_tranche`]) -- and the fourccs every renderer's table
/// leads with (see [`driver_tranche`]).
///
/// Every entry the renderer keeps is a format it can really import, which is
/// what `dmabuf/tests.rs::every_advertised_format_is_one_the_renderer_imports`
/// pins over the wire against the session's own backend rather than against a
/// comment. For pixman that direction matters and the other does not: its
/// importable set is ten-plus fourccs, and advertising *fewer* formats than
/// can be imported costs a client nothing, while advertising one that cannot
/// is a `create_immed` kill.
///
/// Deliberately the same two [`screencopy`](super::screencopy) serves captures
/// in, in the same order (`Xrgb8888` first, for the same translucent-background
/// reason that module's doc gives): a client that allocates from this table and
/// a client that captures into an shm buffer then agree on a pixel layout, and
/// the compositor's own framebuffer is that layout too. Kept as `Fourcc` rather
/// than derived from `screencopy`'s `wl_shm` list so there is no format mapping
/// to get wrong; `dmabuf/tests.rs` pins those two lists to each other as well.
///
/// Under GLES these two are only the *head* of the table: the rest is the
/// driver's own answer, which is where tiled layouts and multi-plane YUV come
/// from. That agreement with the capture path is a pixman concern -- a GLES
/// renderer samples whatever layout it imported into the one framebuffer
/// format capture reads, so no client buffer's layout ever reaches a capture.
const DMABUF_CANDIDATES: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

/// `/dev/dri/renderD128`, the first rung of [`main_device`]'s path ladder.
const RENDER_NODE: &str = "/dev/dri/renderD128";
/// `/dev/dri/card0`, the second rung of [`main_device`]'s path ladder.
const CARD0: &str = "/dev/dri/card0";

/// Creates the `zwp_linux_dmabuf_v1` global for whatever `backend`'s renderer
/// can really import ([`advertised_formats`]), or creates nothing at all when
/// that is nothing (see the module doc: no global beats one that promises an
/// import this session cannot perform).
///
/// Runs once, from `headless::init_named`, immediately after the render target
/// is built and before the event loop starts -- never per bind, per frame or
/// per hotplug event. See the module doc for why that is still early enough
/// for a client that gates on this global.
///
/// Returns what was advertised -- the feedback every client is sent by
/// default, and the builder and table it came from -- so the GPU scanout
/// tier can build a per-surface feedback whose main tranche is *this* one and
/// revert a surface to exactly this one ([`scanout`]). `None` when nothing
/// was advertised.
pub(super) fn advertise(
    dh: &DisplayHandle,
    state: &mut DmabufState,
    backend: &Backend,
) -> Option<DefaultFeedback> {
    let formats = advertised_formats(backend);
    if formats.is_empty() {
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
            "this session's renderer reported no dma-buf format it can be trusted \
             to import, so zwp_linux_dmabuf_v1 is not advertised at all: GL \
             clients will fall back to software rendering over wl_shm, and a \
             shell that waits for dmabuf feedback before capturing the screen \
             will never capture anything. Run with --renderer pixman for a \
             session that imports linear dma-bufs"
        );
        return None;
    }
    // Once per session. The count is the line a "why does my GL client
    // render into linear buffers" report needs; the table itself is `debug`,
    // because on a real GPU it is hundreds of pairs.
    tracing::info!(
        pairs = formats.len(),
        fourccs = fourcc_count(&formats),
        "dmabuf feedback: advertising the renderer's importable formats"
    );
    tracing::debug!(table = ?formats, "dmabuf feedback table");
    let device = main_device(backend.render_node());
    let builder = DmabufFeedbackBuilder::new(device, formats.iter().copied());
    match builder.clone().build() {
        Ok(feedback) => {
            state.create_global_with_default_feedback::<State>(dh, &feedback);
            Some(DefaultFeedback {
                builder,
                feedback,
                formats,
            })
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "dmabuf feedback table could not be built; running without zwp_linux_dmabuf_v1"
            );
            None
        }
    }
}

/// What [`advertise`] put on the global: the default feedback every client
/// is sent, plus the builder and the table it was built from.
///
/// Kept because a per-surface feedback has to *extend* this one, not
/// approximate it. The GPU scanout tier's scanout tranche ([`scanout`]) is
/// added in front of this builder's main tranche, so the per-surface
/// feedback's format table and main tranche are this feedback's, entry for
/// entry; and reverting a surface sends it this very object, which is what
/// Smithay compares against to decide whether anything needs re-sending.
/// Built once, at startup, never per frame.
#[cfg_attr(
    not(feature = "gpu-scanout"),
    expect(
        dead_code,
        reason = "read only by the GPU scanout tier's per-surface feedback"
    )
)]
pub(super) struct DefaultFeedback {
    /// The builder the default feedback was built from: `main_device` and
    /// the one main tranche, [`advertised_formats`]'s table.
    builder: DmabufFeedbackBuilder,
    /// The default feedback itself, as the global holds it.
    feedback: DmabufFeedback,
    /// The advertised table, in wire order.
    formats: Vec<Format>,
}

/// The feedback tranche for `backend`'s renderer, in wire order: the one
/// table [`advertise`] puts in the format-table memfd, and what the tests
/// compare the wire against.
///
/// Dispatches on *what kind of answer* the renderer gives
/// ([`ImportSet`]), not on which renderer it is -- see
/// [`Backend::dmabuf_import_set`](super::render::Backend) for why that is
/// its own question.
fn advertised_formats(backend: &Backend) -> Vec<Format> {
    match backend.dmabuf_import_set() {
        ImportSet::CpuMapped => {
            cpu_mapped_tranche(|format| backend.imports_dmabuf_format(format)).collect()
        }
        ImportSet::Driver(importable) => driver_tranche(&importable),
    }
}

/// How many distinct fourccs `formats` names, for [`advertise`]'s one log
/// line. Quadratic in the fourcc count, which is a few dozen, once per
/// session.
fn fourcc_count(formats: &[Format]) -> usize {
    formats
        .iter()
        .enumerate()
        .filter(|(index, format)| {
            !formats[..*index]
                .iter()
                .any(|seen| seen.code == format.code)
        })
        .count()
}

/// The pixman tranche: [`DMABUF_CANDIDATES`] in their advertised order, minus
/// any the renderer cannot import, every one at `LINEAR`.
///
/// A fixed candidate list narrowed by the renderer, rather than the
/// renderer's own set, because pixman's set is a list of fourccs it can
/// *map*, and the question a client needs answered is narrower than that:
/// what can this compositor read out of a CPU mapping that every other pixel
/// path here -- `screencopy`'s shm list, the framebuffer layout -- already
/// speaks. Advertising *fewer* formats than can be imported costs a client
/// nothing (it falls back to `wl_shm`); advertising one that cannot is a
/// `create_immed` kill. Pinned byte-for-byte on the wire by
/// `dmabuf/tests.rs::default_feedback_names_a_device_and_the_renderers_own_formats`.
///
/// A predicate rather than a renderer, because that is what makes the
/// *decision* testable without the renderer that would have to be there to
/// make it -- including a renderer that imports nothing, which no machine
/// here can produce on demand.
///
/// What the entries say on the wire is always `LINEAR`; what counts as
/// *evidence* that the renderer will take one is [`imports_linear`], which is
/// wider and has to be.
fn cpu_mapped_tranche(can_import: impl Fn(Format) -> bool) -> impl Iterator<Item = Format> {
    DMABUF_CANDIDATES
        .into_iter()
        .filter(move |code| imports_linear(*code, &can_import))
        .map(|code| Format {
            code,
            modifier: Modifier::Linear,
        })
}

/// The GLES tranche: every `{fourcc, modifier}` the renderer's driver said it
/// imports, with the one kind of entry that is not the driver's own answer
/// resolved rather than passed through.
///
/// Per fourcc, [`DMABUF_CANDIDATES`] first (so the old two-entry table stays
/// the head of the new one wherever the driver lists them at `LINEAR`), then
/// every other fourcc in the driver's own order:
///
/// - **Explicit modifiers -- `LINEAR` or any tiled/compressed one -- are
///   advertised as the driver listed them**, external-only ones included.
///   These are the driver's answer to `eglQueryDmaBufModifiersEXT`, which is
///   by definition the list it imports. External-only is not a reason to
///   leave one out: `GlesRenderer::import_dmabuf` binds an entry the render
///   set lacks to `GL_TEXTURE_EXTERNAL_OES` (`gles/mod.rs:1269`), and every
///   `GlesRenderer` can sample one, since its texture program always
///   compiles the `EXTERNAL` variant (`gles/shaders/mod.rs:217`) -- which is
///   exactly how a multi-plane YUV buffer (`NV12`, `P010`) is composited.
/// - **`Modifier::Invalid` -- "implicit layout" -- is never advertised for a
///   fourcc that has explicit modifiers.** Smithay inserts `{fourcc, Invalid}`
///   into *both* the texture and the render set unconditionally
///   (`egl/display.rs:994-1001`), so an implicit buffer of a format whose every
///   explicit layout is external-only would be bound as `GL_TEXTURE_2D`
///   against the driver's own answer -- and nothing checks the GL error after
///   `EGLImageTargetTexture2DOES`, so that would be a black or garbage window
///   rather than a refusal no test of the import could see (measured on
///   llvmpipe -- see the module doc's note on clients that ignore their
///   feedback).
///   A client that supports modifiers picks an explicit one anyway. wlroots'
///   `init_dmabuf_formats` draws the same line -- `INVALID` only for a format
///   with no explicit modifiers -- as read in the archived `swaywm/wlroots`
///   mirror, not re-checked against current upstream.
/// - **A fourcc with *no* explicit modifiers** is one the driver could not be
///   asked about (no `EGL_EXT_image_dma_buf_import_modifiers`, or a driver
///   refusing the query for its own format). Only the two candidates survive
///   that, at `LINEAR`, on the evidence [`imports_linear`] documents -- the
///   exact advertisement this compositor made on such a driver before the
///   table was widened. Anything else there is a guess, and is left out.
///
/// So the widening from `Invalid` to `LINEAR` now applies *only* where the
/// driver gave no explicit answer. Where it did, and `LINEAR` is not in it,
/// `LINEAR` is not advertised -- which the candidate-only rule used to do,
/// on `Invalid` evidence, for a driver that had just said otherwise (see
/// [`imports_linear`]'s doc for why that import could be refused).
///
/// Startup-only, and the quadratic walk is over a set the driver built once:
/// a few dozen fourccs by a few modifiers each, measured in
/// `dmabuf/tests/tranche.rs::driver_tranche_cost`.
fn driver_tranche(importable: &FormatSet) -> Vec<Format> {
    let mut fourccs: Vec<Fourcc> = DMABUF_CANDIDATES.to_vec();
    for format in importable.iter() {
        if !fourccs.contains(&format.code) {
            fourccs.push(format.code);
        }
    }
    let mut tranche = Vec::with_capacity(importable.indexset().len());
    for code in fourccs {
        let before = tranche.len();
        tranche.extend(
            importable
                .iter()
                .filter(|format| format.code == code && format.modifier != Modifier::Invalid)
                .copied(),
        );
        let no_explicit_answer = tranche.len() == before;
        if no_explicit_answer
            && DMABUF_CANDIDATES.contains(&code)
            && imports_linear(code, &|format| importable.contains(&format))
        {
            tranche.push(Format {
                code,
                modifier: Modifier::Linear,
            });
        }
    }
    tranche
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
///   `egl/display.rs:754-759`).
///
/// So requiring an explicit `LINEAR` entry would have thrown away a
/// capability that genuinely worked, ending in no global at all and every GL
/// client on software rendering (see [`advertise`]'s warning). Accepting
/// `Invalid` restores precisely what the hard-coded table did before this
/// stage and promises nothing more: the advertised modifier is still
/// `LINEAR`, and a client that allocates one still gets the import that used
/// to succeed.
///
/// **Where the rule is sound, stated precisely -- an earlier version of this
/// doc overstated it.** It said no modifier attribute reaches the `EGLImage`.
/// That holds only on a display *without* the modifiers extension:
/// `create_image_from_dmabuf` attaches the modifier whenever it is not
/// `Invalid` and the extension is present (`egl/display.rs:817-824`), and
/// `LINEAR` is not `Invalid`. So on a display that has the extension, the
/// `LINEAR` buffer a client allocates from this table is imported as an
/// explicit `LINEAR` one -- which the driver vouched for only if it listed
/// it. Two cases follow, and [`driver_tranche`] treats them differently:
///
/// - the driver gave **no** explicit modifier for `code` (the query was
///   refused or answered zero): nothing contradicts `LINEAR`, and the
///   widening keeps the table this compositor has always offered there;
/// - the driver **did** name its layouts and `LINEAR` is not among them: the
///   `Invalid` entry is only Smithay's unconditional insertion, the driver
///   has just said otherwise, and offering `LINEAR` would invite a refusal
///   through `create_immed`. The GLES table no longer does; it offers the
///   layouts the driver named instead.
///
/// pixman is unaffected by either: it lists no `Invalid` entries, and it maps
/// a linear buffer itself rather than asking a driver.
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

    /// A surface asking for feedback for the first time while it is the
    /// fullscreen window the GPU scanout tier is steering gets the scanout
    /// feedback at once, rather than the default until its window next
    /// changes eligibility (see [`scanout`]'s module doc). Everything else --
    /// and every surface on every other tier -- gets the default, which is
    /// what `None` asks Smithay for. Smithay calls this once per surface, on
    /// its first `get_surface_feedback`; later requests share the surface's
    /// stored feedback.
    #[cfg(feature = "gpu-scanout")]
    fn new_surface_feedback(
        &mut self,
        surface: &WlSurface,
        _global: &DmabufGlobal,
    ) -> Option<DmabufFeedback> {
        self.scanout_feedback.for_new_surface(surface)
    }

    /// Imports the dmabuf into this session's active renderer, answering the
    /// client with what that renderer said. Which renderer that is depends on
    /// `--renderer`; do not assume pixman here.
    ///
    /// A success mints the client's `wl_buffer` and leaves the import in the
    /// renderer's cache -- an `mmap` of plane 0 under pixman, an `EGLImage`
    /// texture (external for a YUV layout) under GLES -- which the ordinary
    /// surface render path then composites like any other texture. A refusal is the protocol's own
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
        if self.backends.is_empty() {
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
        //
        // Into every backend, not just one: each output has its own renderer
        // with its own cache, and a window may be shown on any output -- an
        // import proven on the primary alone would fail to map when the
        // window moves to the second screen. All backends share the session's
        // renderer kind and, under GLES, its one EGL device (pinned -- see
        // `render::gles::GlesDevice`), so they agree; the first refusal
        // decides. A refusal
        // past the first leaves the earlier backends' mappings cached, which
        // `drain_cache` drops with the buffer -- the same shape a retried
        // import would rebuild.
        // Once per session, the fds naming the buffer before the import, so
        // what the renderer opens for it can be told from what was already
        // there (see `dmabuf/renderer_copies.rs`).
        let probe = renderer_copies::Probe::before(self, &dmabuf);
        let mut refused: Option<String> = None;
        for backend in self.backends.values_mut() {
            if let Err(error) = backend.import_dmabuf(&dmabuf) {
                refused = Some(error);
                break;
            }
        }
        match refused {
            None => {
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
                // The planes' fds, and the renderer copies this import has
                // just made, were charged to the client at their `add`s.
                // This learns the real number of copies once per session and
                // lowers the planes' weights to it; it never raises one.
                renderer_copies::charge(self, &dmabuf, probe);
                if let Err(error) = notifier.successful::<State>() {
                    // The client died between allocating and being told --
                    // nothing to release, since the buffer object was never
                    // created and its claim lands on an entry with no live
                    // client (see `wl_buffers.rs`).
                    tracing::debug!(%error, "dmabuf imported, but the client was already gone");
                }
            }
            Some(error) => {
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
/// bookkeeping to find out. Also from its `wl_surface` destruction hook: a
/// surface can hold the last reference to a buffer whose `wl_buffer` is
/// already gone, and on a GLES renderer that keeps a copy of each imported
/// plane's fd, that copy is only closed by a drain (see
/// `dmabuf/renderer_copies.rs`). Gated on
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
    if state.backends.is_empty() {
        return;
    };
    // Every backend: each output's renderer holds its own cache, so draining
    // one would leak the mappings a multi-output session built in the rest.
    for backend in state.backends.values_mut() {
        if let Err(error) = backend.cleanup_texture_cache() {
            tracing::debug!(%error, "dropping expired dmabuf mappings failed");
        }
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

pub(super) mod pending_planes;
pub(super) mod renderer_copies;
#[cfg(feature = "gpu-scanout")]
pub(super) mod scanout;
#[cfg(test)]
pub(in crate::compositor) mod tests;
