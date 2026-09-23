# Changelog

User-visible changes only — things you would have to do something about, or
would notice while using scoot. The full engineering history, milestone by
milestone, is in [`ROADMAP.md`](ROADMAP.md) and
[`docs/roadmap/`](docs/roadmap/).

scoot has not cut a numbered release yet; entries are dated.

## Unreleased

### 2026-09-22 — fullscreen works

- **A video player's or game's fullscreen button now works**, and so does
  `foot --fullscreen`, `mpv --fs` and a taskbar's fullscreen entry. Before,
  scoot ignored every fullscreen request. A fullscreen window covers its
  whole screen edge to edge — no gaps, no focus ring, your bar hidden —
  while notifications and the lock screen still show over it. Focus another
  window and the layout scrolls to it as usual; come back and it is
  fullscreen again; leave fullscreen and everything is exactly where it was.
- **New default bind: `Super+f` toggles fullscreen** on the focused window.
  If your config already binds `super+f` (the docs used to suggest
  `"super+f" = "set-column-width 2"`), your bind still wins — nothing to do
  unless you want the new default.
- **New actions for scripts and agents:** `toggle-fullscreen`, and
  `set-fullscreen ID on|off` to set one window's state by id without
  toggling. `scootctl windows` reports each window's `fullscreen`. See
  [docs/protocols.md](docs/protocols.md#fullscreen) for the details.

### 2026-09-21 — logs are plain text when they are not going to a terminal

- **`scoot`'s log output no longer contains colour escape sequences when
  stdout is redirected or piped.** Colour is still there when you are
  watching a terminal. Before this, `scoot --tty … > session.log` or
  `… | tee session.log` wrote sequences like
  `scanout\x1b[0m\x1b[2m=\x1b[0m"gpu"` into the file, so `grep` for a field
  name found nothing and any saved log was awkward to read. Nothing to do
  about it — existing logs are unaffected, only newly written ones change.

### 2026-09-21 — GPU scanout has run on a real GPU

No behaviour change; this is a claim in the docs becoming a measurement. If
you run `--tty --renderer gles` from a `gpu-scanout` build, the README and
[docs/tty.md](docs/tty.md) previously told you the tier had never run on a
real GPU and that every number behind it came from a software rasteriser.
On an Apple M2 under Asahi Linux it now measures **4–5x less compositor CPU
than the default tier under damage**, the same pixels, about 0.2 W less
power, and 7–16 MB more memory — with both tiers using no measurable CPU at
idle. pixman is still the default and still the right choice on a machine
without a real GPU. Method, spreads and what it does not cover:
[Asahi.md](Asahi.md)'s Test 4.

### 2026-09-19 — `--nested` follows the host window's size, and an idle session stops talking

Two things from running scoot nested inside
[webtop](https://docs.linuxserver.io/images/docker-webtop/), the deployment
the README names (issues #144 and #145).

- **A `--nested` session now resizes with its host window.** Resize the
  browser window (or drag the window scoot is running in, under any host)
  and the desktop inside fills it, at any size, for the whole life of the
  session — not just the size it came up at. Before this, only the host's
  *first* configure was acted on, and everything after was acked and
  ignored: the desktop kept its starting size and the host letterboxed the
  difference.
  Windows inside the session re-lay-out with it, and `scoot msg outputs`
  and `scoot msg screenshot` both report the new size.
  If scoot cannot follow the host to some new size (a bigger buffer pool it
  could not allocate), it says so in the log, stays at the size it was, and
  **keeps running**. You get letterboxing, not a dead session with your
  windows in it. A failure on the very first configure is still fatal, with
  the error it already printed — nothing is on screen at that point, so
  there is no session to save.
- **An idle session no longer logs two lines a second.** A client
  disconnecting cleanly is now `DEBUG`, not `INFO`. Under Selkies (every
  webtop deployment) the clipboard monitor runs `wl-paste` every 500 ms and
  each run is a whole fresh Wayland connection, so an idle session buried
  its own log in `wayland client disconnected`. A client killed by a
  *protocol error* still logs at `WARN`, unchanged — that is the line worth
  keeping, and it is the one that does not repeat. Run with
  `RUST_LOG=scoot=debug` to get the disconnect lines back.

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

The session-identity half of the same rename — these live in *your* files
rather than scoot's, which is why they bite. If your own setup named the old
desktop, update it:

| Was | Is | Who it breaks |
| --- | --- | --- |
| `XDG_CURRENT_DESKTOP=flexwm` (set by your own wrapper or session script) | `=scoot` (the compositor now exports this to every child itself, unconditionally) | anything matching on it — notably portal backend selection |
| `DesktopNames=flexwm` in a hand-rolled session `.desktop` | `DesktopNames=scoot` | greeter entries |
| `flexwm-portals.conf` | `scoot-portals.conf` | portal backend selection stops resolving |

The repo moved too: `github:yackey-labs/flexwm` → `github:scoot-sh/scoot`.
GitHub redirects the old URL, so an existing clone keeps working while
silently pointing at the old name — update `origin` by hand (`git remote
set-url origin https://github.com/scoot-sh/scoot.git`) and repoint any
pinned flake input (`scoot.url = "github:scoot-sh/scoot"`).

The names clients see followed too: the `wl_seat` name, `wl_output`'s `make`
(and the `xdg_output` description built from it) and the `--nested` window's
own title/app-id are all `scoot` now, so anything matching on those strings
needs updating.
