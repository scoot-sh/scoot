# Changelog

User-visible changes only — things you would have to do something about, or
would notice while using scoot. The full engineering history, milestone by
milestone, is in [`ROADMAP.md`](ROADMAP.md) and
[`docs/roadmap/`](docs/roadmap/).

scoot has not cut a numbered release yet; entries are dated.

## Unreleased

### 2026-09-19 — dma-buf formats follow the renderer

The `zwp_linux_dmabuf_v1` feedback a client reads now names only what the
renderer this session is actually running can import, instead of a fixed pair
the CPU renderer could map. **If you use dma-buf clients you no longer have
to stay on `pixman`** — the line the entry below says that in is now wrong,
and this is what fixed it.

Three things you might notice:

- On a renderer that can import *neither* of the formats scoot serves, there
  is now **no `zwp_linux_dmabuf_v1` global at all** rather than one that would
  disconnect any client believing it. GL clients fall back to `wl_shm`; a
  shell that waits for dmabuf feedback before it will capture the screen
  (quickshell does) will keep waiting on such a machine.
- `main_device` in that feedback is the active renderer's own DRM render node
  where it has one, rather than `/dev/dri/renderD128` by path. On a machine
  with more than one GPU that is the difference between allocating on the
  device the import will happen on and allocating on the other one.
- `--tty --renderer gles` no longer refuses to start the GPU scanout tier
  over dma-buf formats. It used to fall back to the CPU renderer on a device
  whose GLES renderer could not import what was advertised; there is now
  nothing to contradict, so it comes up.

Screen *capture* is unchanged and still `wl_shm`-only: writing into a
client's dma-buf is a different capability from importing one, and has its
own item.

### 2026-09-19 — a second renderer, opt-in

`--renderer pixman|gles` (config: `[renderer] backend`) chooses what
composites each frame. **pixman, the CPU renderer, stays the default and is
not changing** — running with no GPU at all is a hard requirement. `gles` is
`--headless`/`--nested` only, and buys correctness parity rather than speed
today: it composites into an offscreen buffer and reads it back exactly as
pixman does, so on a software rasteriser it is slower. GPU scanout under
`--tty` is a separate, later piece of work. See
[`docs/tty.md`](docs/tty.md#which-renderer-draws-the-frames).

If you use dma-buf clients, stay on `pixman`: the advertised buffer formats
are still the CPU renderer's whichever renderer is active.

### 2026-09-18 — renamed from `flexwm` to `scoot`

A clean break, with **no fallback to the old names**:

| Was | Is |
| --- | --- |
| binary `flexwm` | binary `scoot` |
| crates `flexwm`/`flexwm-core`/`flexwm-ipc` | `scoot`/`scoot-core`/`scoot-ipc` |
| `flexwm.sock` | `scoot.sock` |
| `$FLEXWM_SOCKET` | `$SCOOT_SOCKET` |
| `~/.config/flexwm/config.toml` | `~/.config/scoot/config.toml` |

A config file left at `~/.config/flexwm/config.toml` is **not** loaded — move
it, or pass `--config PATH`.

The names clients see followed too: the `wl_seat` name, `wl_output`'s `make`
(and the `xdg_output` description built from it) and the `--nested` window's
own title/app-id are all `scoot` now, so anything matching on those strings
needs updating.
