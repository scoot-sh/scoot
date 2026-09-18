---
title: "The `zwp_linux_dmabuf_v1` advertisement answers every import `failed`, which kills any GPU-rendering client outright -- noctalia v5 cannot start a flexwm session at all"
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# A dmabuf advertisement with no import path kills GL clients

Found on the user's Asahi (M2) laptop, 2026-09-18, as a **black screen with a
cursor and nothing else** after a `flake.lock` bump moved flexwm from
`dfea40b` (revCount 158) to `01e0d8d` (revCount 381). That range contains
`599be4e` ("linux-dmabuf: advertise a minimal, honest zwp_linux_dmabuf_v1"),
which is the regression. The session is
`flexwm --tty -- noctalia`, so when noctalia dies there is no shell, no
window, and no way to spawn one: the compositor is up and healthy
(`flexwm msg outputs` answers, eDP-1 at 1280x800 scale 2, `usable` still the
full rect) with zero clients alive.

## The chain, measured

Reproduced under `--nested` with `RUST_LOG=debug` (host: the same machine's
`--tty` session), noctalia v5.1.0:

```
DEBUG flexwm::compositor::layer_shell: layer surface created namespace=noctalia-bar-default layer=Top
DEBUG flexwm::compositor::dmabuf: client attempted a dmabuf import; answering failed
      (this compositor has no dmabuf path) format=DrmFourcc(AR24) modifier=Linear
WARN  flexwm::compositor::state: wayland client killed by a protocol error
      id=InnerClientId { id: 0, serial: 1 }
      error=ProtocolError { code: 7, object_id: 50,
      object_interface: "zwp_linux_buffer_params_v1",
      message: "create_immed failed and produced an invalid wl_buffer" }
[noctalia] [main] Wayland display closed during failed to flush Wayland display
      before poll; shutting down (display_error=0 (none), operation_errno=32 (Broken pipe))
```

1. flexwm advertises `zwp_linux_dmabuf_v1` with a `LINEAR` `Argb8888`/`Xrgb8888`
   tranche and a real `main_device`.
2. Mesa's EGL sees the global, so `dri2_initialize_wayland_drm` succeeds and it
   takes the dmabuf path **instead of** the `wl_shm` swrast path it would
   otherwise fall back to. It allocates `AR24` / `LINEAR` -- exactly what the
   tranche advertises -- and calls `zwp_linux_buffer_params_v1.create_immed`.
3. [`dmabuf.rs`](../../../crates/flexwm/src/compositor/dmabuf.rs)'s
   `dmabuf_imported` answers `notifier.failed()`.
4. For `create_immed` that is **not** a soft answer. Smithay's `ImportNotifier::failed`
   at the pinned rev branches on `Import::Falliable` (i.e. `create`) vs the
   immediate one, and for the latter posts
   `zwp_linux_buffer_params_v1::Error::InvalidWlBuffer` -- a fatal protocol
   error, killing the client (`src/wayland/dmabuf/mod.rs:955-965`).

So the advertisement steers Mesa off the one path flexwm can serve and onto
one that is fatal. The client is dead ~100ms in, right after mapping its bar.

## Why the original reasoning missed it

Nothing in `599be4e` was careless -- the premise it rested on simply expired:

- `dmabuf.rs`'s module doc states the honesty position plainly ("its tranche
  structure implies a dmabuf *import* capability flexwm does not have; any
  import attempt is answered `failed`, the protocol's own 'cannot import for
  implementation-dependent reasons'"). True of `create`; for `create_immed`
  the protocol has no such answer, because the client already holds the
  `wl_buffer`.
- The edge-case list says "`create_params` with garbage is `failed` or a
  protocol error, never a panic" -- accurate about *the compositor*, which is
  what it was reasoning about, and silent about the client.
- The measurement that justified the advertisement
  (`resolved/screencopy-shell-thumbnails-fallback-done.md`) recorded **zero
  `create_params`/`create_immed` wire lines across all three probed sessions**.
  That was true of the shells probed then. noctalia v5 ships its own EGL/GLES
  renderer (`[render] EGL vendor="Mesa Project" ... OpenGL ES 3.2 Mesa 26.2.2`),
  so it does what none of the probed clients did.
- `wl_buffers.rs` already had the shape of the answer written down --
  "`failed()` on an immed import posts `InvalidWlBuffer`, killing the client"
  -- recorded there as accounting for the buffer cap, never joined back up to
  "and therefore no GL client can run here."

## The fix: actually import them

**This does not need roadmap item 6 (a GPU rendering pipeline), and must not
wait on it.** Item 6 is about *flexwm* rendering with the GPU; this is about a
*client* rendering with the GPU and handing over the result. Smithay's
`PixmanRenderer` already implements `ImportDma` **and** `ImportDmaWl` at the
pinned rev (`src/backend/renderer/pixman/mod.rs:1182-1214`): its
`import_dmabuf` `mmap`s plane 0 of a single-plane `LINEAR` dmabuf, wraps the
mapping in a `pixman::Image` with the buffer's own stride, and caches it in
`dmabuf_cache` -- a pure CPU path, no GPU on the compositor side, exactly the
GPU-free requirement. The client renders on the GPU; flexwm composites the
result with pixman. Both halves work, which is the stated goal: this has to
work whether the client is GL or CPU/pixman.

`render_elements_from_surface_tree` / `WaylandSurfaceRenderElement` are already
generic over `ImportAll`, and `handlers.rs:73` already calls
`on_commit_buffer_handler::<Self>`, so once the import succeeds the existing
render path consumes a dmabuf-backed surface with no further change. `State`
reaches the renderer through `state.backend: Option<Backend>`
(`headless::Backend::renderer`), shared by all three backends.

Sketch, not a patch:

```rust
fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
    let Some(backend) = self.backend.as_mut() else { return notifier.failed() };
    match ImportDma::import_dmabuf(&mut backend.renderer, &dmabuf, None) {
        Ok(_) => { let _ = notifier.successful::<State>(); }
        Err(error) => { tracing::debug!(%error, "dmabuf import refused"); notifier.failed(); }
    }
}
```

## What this change has to get right

Each of these is a real consequence elsewhere, not a style note:

1. **`wl_buffers.rs`'s buffer cap must start claiming on the `create` path.**
   Its doc says so in as many words: "*If a future renderer ever calls
   `successful()` on a `create` notifier, that path starts creating buffers and
   must claim here too.*" That future is this item. `ImportNotifier::successful`
   on a `Falliable` (i.e. `create`) notifier creates a real `wl_buffer`; today
   only `create_immed` claims a unit, so the async path would mint uncounted,
   fd-retaining buffers and bypass `MAX_BUFFERS_PER_CLIENT` entirely. Claim in
   `dispatch.rs` alongside the other two factories, release through the
   existing `wl_buffer` destruction hook.
2. **Advertise exactly what can be imported, no more.** A `create_immed` the
   compositor refuses is still a client kill, so the tranche is now a promise
   with teeth: a format or modifier in the table that `import_dmabuf` then
   rejects is the same crash with extra steps. `PixmanRenderer::dmabuf_formats()`
   is `SUPPORTED_FORMATS x LINEAR`; `DMABUF_FORMATS`'s hardcoded pair is a
   subset, which is safe, but the two lists should be pinned to each other by
   test rather than by comment. Multi-plane and non-`LINEAR` imports must stay
   refused -- the tranche never offers them, so only a misbehaving client
   reaches that path.
3. **Mapping lifetime and `cleanup()`.** `import_dmabuf` pushes every imported
   image into `PixmanRenderer::dmabuf_cache`, and only `PixmanRenderer::cleanup`
   drops the entries whose `WeakDmabuf` has expired. Check whether any flexwm
   render path calls `cleanup()`; if none does, every dmabuf a client ever
   commits keeps an `mmap` and an fd alive for the process lifetime -- which
   `wl_buffers.rs`'s per-client cap does not bound, because the cache outlives
   the `wl_buffer`. This is the item's real leak risk and deserves a test that
   watches the cache drain.
4. **Per-frame sync, not just per-import.** `import_dmabuf` issues
   `DmabufSyncFlags::START | READ` then `END | READ` **once, at import time**,
   and `existing_dmabuf` returns the cached image on every later commit without
   re-syncing. A client re-rendering into one dmabuf across frames (the normal
   GL case) can therefore present torn or stale pixels. Establish what the
   pinned rev actually guarantees here before assuming the cache is safe to
   reuse across commits.
5. **`main_device` should name the render node.** Today it prefers
   `/dev/dri/card0`, falling back to `renderD128`. With a real import path the
   feedback's device is what clients allocate against, and a primary node is
   the wrong hint for a client that only needs to render.
6. **The module doc's "Honesty, in one sentence" section becomes false** and
   must be rewritten rather than left to age -- the advertisement stops
   implying a capability flexwm lacks and starts describing one it has.

## Verification

The bar is the reproducer above, on real hardware: a `--tty` session started as
`flexwm --tty -- noctalia` must come up with its bar and wallpaper, with no
`LIBGL_ALWAYS_SOFTWARE=1` anywhere, and `flexwm msg outputs` must show the
bar's exclusive zone applied (`usable.y == 34`, `height == 766` at this
machine's scale). The nested repro is the cheap inner loop; the `--tty` session
is the one that was broken and the one that has to be shown fixed.

Regression-test both directions, because both are the point: a GL client
(dmabuf commit renders) and a CPU client (`foot`, shm) in the same session.

## Workaround in place meanwhile

The user's `nixos-config` pins the session's shell to llvmpipe
(`env LIBGL_ALWAYS_SOFTWARE=1 noctalia` on the `Exec` line), which keeps Mesa
on the `wl_shm` swrast winsys so no dmabuf is ever allocated. Verified working
live. It is a per-client bandaid on one machine's config -- every other
GL client on flexwm is still killed -- and should be removed once this lands.
