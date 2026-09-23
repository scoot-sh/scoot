# Changelog

User-visible changes only — things you would have to do something about, or
would notice while using scoot. The full engineering history, milestone by
milestone, is in [`ROADMAP.md`](ROADMAP.md) and
[`docs/roadmap/`](docs/roadmap/).

scoot has not cut a numbered release yet; entries are dated.

## Unreleased

### 2026-09-23 — screenshots show the pointer the same way on every backend

- **Screenshots now show the pointer on every backend and renderer**, and
  leave it out only when asked. Before, whether it showed depended on how
  the session ran: `--headless` and `--nested` screenshots never had it,
  `--tty` ones always did, and on the GPU tier (`--tty --renderer gles`)
  it went missing wherever the display carried it on a hardware cursor
  plane. Now `scootctl screenshot` draws it in by default everywhere —
  **so a `--headless` or `--nested` screenshot now has a pointer in it**
  (at the centre of the screen until something moves it) — and
  `scootctl screenshot --no-cursor` (IPC: `"cursor": false`) leaves it out.
- **Screen-capture clients get the pointer exactly when they ask for it.**
  `grim -c` (the `paint_cursors` option) now draws it in on every backend,
  and plain `grim` never shows it — including under `--tty`'s default
  renderer, where it used to be in every capture. A recorder or screen-share
  that asked for the pointer also sees it move when nothing else on screen
  changes.

### 2026-09-23 — fullscreen apps are told what the display can show directly (GPU tier)

- **On the opt-in GPU tier (`--tty --renderer gles`, `gpu-scanout` build),
  a fullscreen app is now told which of its buffer layouts the display can
  show straight from the app's memory**, so a GPU app that listens can pick
  one and skip compositing — instead of picking whatever renders fastest and
  being composited without knowing why. It is only a suggestion: every
  layout it names is one the app was already offered, so nothing can break
  for an app that ignores it, or that ends up composited anyway (a
  notification over it, a screen recording). An app is only told when its
  buffer could actually be shown that way: its window is opaque (an
  opaque-format buffer, or marked opaque), or the background is black and
  there is no wallpaper under it other than a plain black one. An app with a transparent window over
  scoot's default background, or over a wallpaper, is not told — the
  display would never be offered its buffer. Nothing to configure. Seen on
  the dev VM with a test client; not yet with a real GPU app or on real GPU
  hardware ([Asahi.md](Asahi.md), Test 6).
- **Presentation timing now says when a frame was shown with zero copy.**
  Apps that ask for `wp_presentation` feedback (video players, games) are
  told `zero_copy` for frames scanned out straight from their buffer.

### 2026-09-23 — GPU apps get their GPU's own buffer formats (GLES renderer)

- **Under `--renderer gles`, GPU-rendering apps are now offered every
  buffer format and layout the GPU driver can take**, where they used to be
  told "plain linear RGB only". On a real GPU that lets GL and Vulkan apps
  render into the layouts the GPU prefers (tiled, compressed), which is
  typically faster than linear, and lets a video player hand over the YUV frames (`NV12`, `P010`, …) a
  hardware decoder produces instead of converting them first. The default
  pixman renderer is unchanged. Checked on the dev VM's software GPU (which
  offers 57 formats, all linear); what real GPU hardware offers has not
  been captured yet ([Asahi.md](Asahi.md), Test 6). One consequence to know
  on the GPU tier: an app that picks a tiled layout the display cannot show
  directly makes its fullscreen window composite rather than go straight to
  the screen, until scoot learns to steer fullscreen apps toward a layout
  the display takes.
- **On a machine with more than one GPU, a `gles` session stays on the GPU
  it started on.** If that GPU cannot rebuild the renderer at a new size,
  scoot keeps its output at the old size (under `--nested` the host window
  has still been resized; scoot's picture inside it has not), instead of
  quietly moving to another GPU that might not accept the apps' buffers.

### 2026-09-23 — fullscreen video without compositing (GPU tier)

- **On the opt-in GPU tier (`--tty --renderer gles`, `gpu-scanout` build),
  a fullscreen window can be shown straight from the app's own buffer**
  when the display accepts that buffer, instead of being composited every
  frame — far less work for the compositor. Nothing to configure;
  anything drawn over the window (a notification on the `overlay` layer, a
  menu), the lock screen, or a program recording the screen makes scoot
  composite as before. Screenshots and screen captures still show the
  current screen; a screenshot of such a frame takes a few milliseconds
  longer. Seen on the dev VM with a test client's dumb buffers; not yet
  with a GPU-rendered app, a real video player, or real GPU hardware.

### 2026-09-23 — nested subsurfaces can no longer crash scoot

- **A program can no longer crash scoot — and every other app with it —
  by nesting subsurfaces thousands deep.** Subsurfaces (the separate
  pieces some apps draw a video, a titlebar or an overlay in) now nest at
  most 64 levels below their window, menu, bar or other top surface; real
  apps use one or two. A program that tries to put a subsurface deeper —
  by nesting them directly, or by attaching a stack of subsurfaces it
  built separately — is disconnected instead. mpv, foot, weston's
  subsurface demo and GTK 4's demo video player were checked and are
  unaffected. If a program you use
  is disconnected this way, the compositor log names it (see
  `docs/protocols.md`) — please report it.

### 2026-09-23 — nested menus can no longer crash scoot

- **A program can no longer crash scoot — and every other app with it —
  by opening menus inside menus thousands deep.** Menus now nest at most
  64 levels (real ones stop at a handful); a program that goes deeper is
  disconnected instead. The same goes for a program that re-parents menus
  it already has open to get around that limit, in ways Wayland's rules
  forbid anyway: asking for a second menu on a surface whose menu is still
  open, closing a menu while its submenu is still open, opening a menu off
  a surface that is not an open window or menu, or handing a bar a menu
  that was already open somewhere else. Ordinary apps close
  submenus first and are unaffected (GTK was checked). If a program you
  use is disconnected when you open or close a menu, the compositor log
  names it and the rule it broke (see `docs/protocols.md`) — please report
  it.

### 2026-09-23 — menus stay on screen

- **A right-click menu or dropdown opened near the edge of a screen now
  opens fully on screen** — flipped to the other side of the pointer or
  slid back inside — instead of being cut off at the edge. That includes
  the edge between two screens, where a menu was cut off since the change
  below: it now moves back onto its own window's screen. A window's menu
  also stays clear of a bar at the top or bottom of the screen rather than
  opening underneath it. Apps choose whether their menus may be moved;
  GTK's menus allow it, and one that does not is still cut at the edge.
- **A menu that is its own parent no longer freezes scoot.** A popup that
  named itself, or a loop of popups, as its parent used to hang the
  compositor and every app in it; that program is now disconnected instead.

### 2026-09-23 — windows stay on their own screen

- **With more than one output, a window no longer draws onto — or takes
  clicks from — the screen next to it.** A column scrolled part-way past the
  edge of its screen, or a fullscreen window whose column you focused away
  from, used to show on the neighbouring screen on top of that screen's own
  windows, and a click there went to it. Now it is cut at the edge of its
  own screen, and the neighbour shows (and clicks) its own windows. A menu
  opened right at the shared edge is cut there too, the same as at the outer
  edge of a screen.
- **The focus ring and rounded corners now draw on the second and later
  outputs.** They were built in the first output's coordinates, so on any
  other output the ring landed off-screen and the corner rounding missed.
- For scripts and agents: in `scootctl windows`, only the part of a
  window's `rect` inside its own output's `rect` is drawn and clickable.

### 2026-09-22 — fullscreen works

- **A video player's or game's fullscreen button now works** — any app
  that asks through the standard Wayland request (`foot --fullscreen` is
  one) — and so does a taskbar's fullscreen entry. Before, scoot ignored
  every fullscreen request. A fullscreen window covers its
  whole screen edge to edge — no gaps, no focus ring, your bar hidden — and
  the lock screen still shows over it. Notifications show over it only if
  your notification daemon draws on the `overlay` layer: mako, for one,
  defaults to `top`, which is hidden under a fullscreen window like a bar —
  set `layer=overlay` in its config to see them. Focus a window in another
  column and the layout scrolls to it as usual; come back and it is
  fullscreen again. Focusing a window stacked in the *same* column ends
  fullscreen instead. Leave fullscreen and everything is exactly where it
  was.
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
