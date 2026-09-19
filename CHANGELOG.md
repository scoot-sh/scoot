# Changelog

User-visible changes only — things you would have to do something about, or
would notice while using scoot. The full engineering history, milestone by
milestone, is in [`ROADMAP.md`](ROADMAP.md) and
[`docs/roadmap/`](docs/roadmap/).

scoot has not cut a numbered release yet; entries are dated.

## Unreleased

### 2026-09-19 — `--headless --outputs N`, and a screenshot that refuses the wrong screen

`scoot --headless --outputs N` (1–8, default 1) creates N virtual outputs
side by side, each with its own `wl_output` (`headless`, `headless-2`, ...),
its own place in the coordinate space and its own scrolling strip. It exists
so per-output behaviour is testable without a second monitor; `--nested` and
`--tty` warn and ignore it, having one host window and one CRTC respectively.

**Only the first output is composited**, so two things follow that you would
notice:

- `scoot msg screenshot --output ID` now **refuses** an output scoot does not
  composite, instead of answering it with the first output's pixels. The `ID`
  was previously ignored outright, which meant `--output 2` returned a picture
  of output 1 labelled as output 2 — a mislabel an agent cannot detect.
  Omitting `--output` still always means the composited output.
- A bar on a second output reserves no space anywhere, the pointer is
  hit-tested against the first output's layer surfaces, one
  `wlr-output-management` head and one `ext-workspace` group are published,
  and a session lock covers the first output. See
  [docs/configuration.md](docs/configuration.md#more-than-one-output).

One fix you would only have hit with more than one output: a layer surface
created on an output other than the first is now configured against that
output and unmapped from it. Before, its initial configure was never sent
(the client would wait for one forever) and destroying it left it arranged.

### 2026-09-19 — dma-buf formats follow the renderer

The `zwp_linux_dmabuf_v1` feedback a client reads now names only what the
renderer this session is actually running can import, instead of a fixed pair
the CPU renderer could map. **If you use dma-buf clients you no longer have
to stay on `pixman`** — the line the entry below says that in is now wrong,
and this is what fixed it.

Three things you might notice:

- On a renderer that can import *neither* of the formats scoot serves, there
  is now **no `zwp_linux_dmabuf_v1` global at all** rather than one that would
  disconnect any client believing it. That is the safe answer, but it is not
  a small one, so scoot says so loudly at startup: every GL client falls back
  to Mesa's `wl_shm` swrast path — **software rendering**, on a machine you
  presumably picked `gles` for — and a shell that waits for dmabuf feedback
  before it will capture the screen (quickshell does) never captures
  anything, with no error of its own. If you see
  `can import none of the dma-buf formats this compositor serves` in the log,
  run `--renderer pixman`: it imports a linear dma-buf by mapping it and
  refuses essentially nothing.
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
