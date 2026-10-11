---
title: "Swap pixman <-> GPU at runtime over IPC without restarting clients (no client kills)"
status: "research"
area: "core"
priority: "research"
blocked: "renderer-auto-policy, scanout-seat-reconnect"
---

# Swap pixman <-> GPU at runtime over IPC without restarting clients (no client kills)

Filed 2026-10-10. Serves **daily-drive**: a user who docks a GPU (or
whose auto choice proves wrong) should flip tiers without losing every
client's unsaved state — a compositor restart takes all clients with it.
Research because the design gate below may prove the `--tty` direction
unbuildable, in which case this resolves as a documented refusal, not a
half-built swap.

## The gap

The renderer is fixed for the process's life (`State::renderer` doc,
`state.rs:477-485`). Changing tiers today means restarting scoot, which
disconnects every client. The dma-buf feedback that would make a swap
safe is built once per session and never re-sent (`dmabuf.rs`), and
`create_immed` can only refuse with a fatal error — so a GPU→CPU swap
with live tiled or compressed client buffers would kill clients. That is
the shape this ticket must rule out, not discover mid-PR.

## What to do

IPC first (`scoot-ipc` protocol 10 → 11, under the existing bump rule in
`crates/scoot-ipc/src/lib.rs:70-100`): `Request::Renderer` →
`Response::Renderer { requested, source, active, reason }` (read-only;
could ship first as a tiny PR after `renderer-auto-policy`), then
`Request::SetRenderer { request }` → the same response or an `Error`
naming the refusal. CLI: `scoot msg renderer` and `scoot msg renderer
set auto|pixman|gles`. Docs in `site/src/content/docs/msg/requests.md`.
Order: headless/nested first (rebuild `Backend` per output via
`Backend::new`), `--tty` second (reuses `scanout-seat-reconnect`'s
rebuild machinery — hence the block).

**Rule 1, no client kill.** A swap proceeds only if every format and
modifier pair currently *advertised* by `zwp_linux_dmabuf_v1` is
importable by the target renderer (`Backend::imports_dmabuf_format` /
`dmabuf_import_set`), *and* every live imported client dma-buf is too —
otherwise refuse, naming the reason. Consequence: pixman→gles is usually
allowed; gles→pixman is refused whenever the GLES advertisement carried
tiled/compressed modifiers or any client holds one. Re-sending v4
feedback and waiting for clients to reallocate is a research question,
not a default.

**Rule 2, teardown first** (the #548 discipline,
`tty/reconnect.rs:21-46`): refuse while a flip is in flight, or retry
from idle once `flip_tracker` reports none; drop old presenters and
pipelines before building new ones; on any failure rebuild the old tier,
never leave a dark session.

**Rule 3, refuse while the session is locked or a lock is pending**
(the locked-frame confirmation in `session_lock.rs` must not straddle
two renderers).

**Everything that assumes the renderer is fixed needs a decision:**
`State::renderer`'s doc, `gles::FIRST_BUILD_LOGGED`, the `GlesDevice`
pin, `dmabuf` advertise-once, the `drm_syncobj` global (enabled only on
the scanout tier at startup, `tty/mod.rs:414-435` — a swap to dumb must
keep honoring acquire points of already-bound clients; a swap to scanout
cannot add the global retroactively), cursor/overlay plane state,
`Captures`/`ensure_scanout_capture_current`, screencopy sessions (size
unchanged, buffers re-validated), presentation feedback, the
per-renderer texture cache (every surface re-imports on the first
frame), and reload's restart-only refusal (keep it: `reload` stays
restart-only, the IPC `set` is the live path). Measure the swap's wall
time and the frame gap (screenshot timestamps). No allocation on
per-frame paths; the swap itself is a one-off like a resize.

## Not in this ticket

The `auto` request (`renderer-auto-policy`); the seat-reconnect rebuild
it stands on (`scanout-seat-reconnect`); relaxing reload's restart-only
`renderer.backend` refusal (stays; the IPC `set` is the live path, not a
second live path); feedback re-send as a default (refuse instead until
the research lands).
