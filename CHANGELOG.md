# Changelog

User-visible changes only — things you would have to do something about, or
would notice while using scoot. The full engineering history, milestone by
milestone, is in [`ROADMAP.md`](ROADMAP.md) and
[`docs/roadmap/`](docs/roadmap/).

scoot has not cut a numbered release yet; entries are dated.

## Unreleased

### 2026-09-25 — X11 applications (opt-in XWayland)

- **X11 apps run** with `--xwayland` (or `[xwayland] enabled`) in a build
  with the `xwayland` feature and `Xwayland` on `PATH`. Their windows tile
  like any other, dialogs and transients float centred on their parent,
  fullscreen works (the app's own button, `Super+f`, a taskbar), menus and
  tooltips appear where the app puts them, and every window gets the focus
  ring and rounded corners. They show up in `scootctl windows` and in
  taskbars under their `WM_CLASS` class (`XTerm`, `Gimp`), which is also
  what a `[[window_rule]]` `match_app_id` matches.
- **X apps do not steal focus.** A new X window takes focus only when no
  window has it, when it belongs to the X app you are using (any window of
  the same process), or when scoot itself started the app (a keybinding, `scootctl
  action spawn`, autostart) moments ago; otherwise it opens without focus
  and a click, a keybinding or a taskbar focuses it. An X app asking to be
  activated (`_NET_ACTIVE_WINDOW`, `xdotool windowactivate`) gets the same
  answer.
- **Trust:** running an X app extends full trust to it -- X11 apps can read
  each other's keystrokes and windows by design, and an X app's menus and
  pop-ups can cover anything on screen, Wayland apps included. Not bridged
  yet: copy and paste or drag-and-drop between X and Wayland apps, and X
  input methods.

### 2026-09-25 — move and resize floating windows

- **Drag floating windows.** Hold Super and drag with the left button to
  move a floating window, or with the right button to resize it from the
  nearest edge or corner. A dialog's own titlebar and borders work too
  (GTK headerbars, client-side decorations). Windows stay where you put
  them, inside the screen's usable area, and a resize respects the window's
  own minimum and maximum size wherever the screen has room for them (it
  never grows a window past the usable area, even to reach its minimum). Dropping one over another output moves it
  there. Under `--nested`, where the host often keeps Super, set
  `[floating] modifier = "alt"` (or `ctrl`, `shift`). Tiled windows are not
  dragged: a titlebar drag or Super+drag on one does nothing special.
- **For agents:** `scootctl action move-floating ID X Y` and
  `resize-floating ID WIDTH HEIGHT` (clamped; `windows` reports where the
  window went).
- **A window's dialogs stay on top of it.** Clicking a floating app (or a
  fullscreen game) no longer hides the dialog it opened underneath it.

### 2026-09-25 — dialogs and chosen apps float above the scrolling strip

- **Dialogs float.** A confirmation dialog, file picker, About box or
  settings window now opens centred above the strip (on the window it
  belongs to, when that is on screen) instead of taking a column and
  scrolling everything sideways. scoot floats a window automatically when
  its toolkit marks it a dialog (`xdg-dialog-v1`, which GTK 4 uses), when it
  names a parent window (GTK 3 dialogs), or when it cannot be resized. The
  strip underneath is left exactly as it was. Turn the automatic part off
  with `[floating] auto = false`.
- **Window rules.** `[[window_rule]]` tables float (`float = true`) or keep
  tiled (`float = false`) windows by app id or title, with glob matching and
  an optional starting `size`; `scootctl reload` applies them to windows
  opened after it. See [configuration.md](docs/configuration.md#window_rule).
- **New keys and actions.** `Super+Shift+Space` (`toggle-floating`) floats
  the focused window or puts it back as a column; `Super+Space`
  (`toggle-floating-focus`) moves focus between floating windows and the
  strip; `scootctl action set-floating ID on|off` sets one window by id.
  `scootctl windows` reports `floating` for each window. If you had bound
  either key yourself, your binding still wins.
- Floating windows are placed by scoot (centred, kept on screen); moving and
  resizing them with the pointer, including a dialog's own titlebar drag, is
  not there yet.

### 2026-09-25 — scoot raises its file-descriptor limit, and an app can no longer make it hold hundreds of descriptors by attaching them to ordinary requests

- **A misbehaving app can no longer make scoot hold hundreds of file
  descriptors by attaching them to ordinary requests.** An app could send
  file descriptors along with requests that never use one, and scoot kept
  every one of them for as long as the app stayed connected. A single idle
  app could take scoot from 18 to 999, and scoot then turned away every
  new app and `scootctl`. scoot now disconnects an app that leaves more
  than 1024 unused: the default limit of libwayland, the library GNOME,
  KDE, sway and weston are built on, so apps those serve are served here,
  including apps that queued many requests while scoot was busy.
- **scoot raises its own file-descriptor limit at startup** (to the
  system's hard limit, capped at 65536, which also lowers a larger one) and
  logs it; **programs scoot starts
  get the normal limit back**, so older programs that use `select()` keep
  working. With the higher limit, one app can no longer come near the point
  where scoot turns new apps and `scootctl` away. On a machine or container
  whose hard limit is 1024, nothing is raised, the startup log says so, the
  unused-descriptor limit is 128, and an app that queued more than about
  128 requests carrying descriptors while scoot was busy can be
  disconnected there; raise the hard limit to avoid that. Nothing to
  configure. Details: [protocols.md](docs/protocols.md#per-client-limits-on-what-scoot-keeps).

### 2026-09-24 — rounded corners and the focus ring now match the window

- **Rounded corners and the focus ring now match the window**, including
  terminals that size themselves to whole character cells. With
  `corner_radius` set, `foot`'s default settings left a sliver of
  background along the right and bottom edges. The ring and the rounded
  corners then curved around empty space instead of the terminal (gh #205).
  scoot now tells every window in the layout that it is tiled, so `foot`
  fills its space exactly (GTK apps also trim their shadows). A window
  that still draws less than its space, such as a fixed-size dialog or a
  video player at its video's size, has its own corners rounded and the
  ring drawn around it. One exception remains: libadwaita dialogs (GNOME
  apps' message boxes) round their own corners more than scoot does. A
  small crescent of background still shows between their corner and the
  ring. It is much closer than before, when the ring circled empty space,
  and it is tracked for a follow-up.
- **The focus ring's outer corners are round all the way round.** They
  used to have square "shoulders" beside the window's top and bottom
  edges.
- **For scripts and agents:** `scootctl windows` / IPC `rect` now reports
  the area a window has actually drawn. That is its layout slot, or less
  if the window draws less. Clicks there land on the window, unless a bar
  or other layer surface covers that spot or the window's own input region
  excludes it. Clicks in the undrawn part of a slot never reached the
  window. Details:
  [ipc.md](docs/ipc.md#what-the-replies-carry).

### 2026-09-24 — one app can no longer make scoot turn everything else away by keeping old buffers on screen

- **An app could still make scoot hold almost all of its file
  descriptors**, a different way from the one fixed earlier today: by
  showing a buffer on a surface and then throwing the buffer away. scoot
  had to keep the buffer's memory (and the file behind it) for as long as
  the surface showed it, but stopped counting it the moment the app threw
  the buffer away. Repeated on enough surfaces, new apps and `scootctl`
  were turned away, and on the GPU tier scoot could run out of file
  descriptors entirely. A GPU buffer made of several pieces (video frames
  are usually two or three) also used to count as one. scoot now counts
  every file an app hands it, for exactly as long as scoot really keeps
  it, including the extra copy some graphics drivers keep; an app that
  goes past a generous limit (512) is disconnected, and everything else
  keeps working. No real app comes near it (a terminal keeps 2). Nothing
  to configure. Details: [protocols.md](docs/protocols.md#per-client-limits-on-what-scoot-keeps).

### 2026-09-24 — screenshots on the GPU renderer no longer grow memory

- **Screenshots on the GPU renderer no longer grow scoot's memory.** With
  `--renderer gles` (including the `--tty` GPU tier), each capture taken
  while nothing on screen was changing kept a whole screen's worth of
  memory: about 6 MB per capture at 1600x1000, 8 MB at 1080p and 33 MB at
  4K. It was only given back once something redrew, so an agent polling
  screenshots of a still screen could run scoot out of memory. Both
  `scootctl screenshot` and screen-capture tools (`grim`, and anything else
  using `ext-image-copy-capture-v1`) could do it. Memory now stays
  flat however many captures are taken, and a screenshot is no slower
  (about 1 ms faster on the dev VM). The default pixman renderer never did
  this. Nothing to configure.

### 2026-09-24 — a misbehaving app can no longer make scoot turn other apps away this way

- **One app could make scoot hold almost all of its file descriptors**, by
  handing over GPU buffer pieces it never finished building, or explicit-sync
  timelines it had already thrown away. scoot then turned *new* apps (and
  `scootctl`) away to protect itself, while the app causing it carried on.
  Both are now counted per app: an app that goes past a generous limit on
  either is disconnected, and everything else keeps working. (Other ways of
  holding descriptors are not all counted yet; see
  `docs/backlog/resolved/wayland-backend-fd-queue-done.md` and
  `docs/backlog/resolved/buffer-fds-past-their-object-done.md`, both since
  fixed.) No real app comes near
  the limits (a GPU app normally has one buffer's pieces in flight at a time; a
  Vulkan window uses 16 timelines, and the limit is 128). Nothing to
  configure. Details and the exact numbers: [protocols.md](docs/protocols.md).

### 2026-09-24 — `--nested --renderer gles` hands its frames to the host as GPU buffers

- **In a `gpu-scanout` build (`nix build .#scoot-gpu`), `scoot --nested
  --renderer gles` no longer copies every frame back to the CPU** when the
  compositor it runs inside draws on the same GPU: each frame goes to the
  host as a GPU buffer (a dma-buf) instead. Nothing to configure. When it
  cannot -- the default build, pixman, a host on another GPU or without
  dma-buf support -- it presents the way it always has, and the startup log
  says which it chose and why (`nested: presenting to the host by ...`).
  If the host later refuses one of those buffers, scoot switches back for
  the rest of the session and logs one warning. Screenshots and screen
  capture are unaffected. Seen working on the dev VM, whose GPU is
  software, so no speedup could show there; real GPU hardware is
  [Asahi.md](Asahi.md)'s Test 8.

### 2026-09-23 — explicit sync for GPU apps on the GPU tier

- **GPU apps that use explicit sync now get it on the GPU tier** (`--tty
  --renderer gles`, `gpu-scanout` build), where the GPU device supports it.
  That covers NVIDIA's driver, which relies on it, and Mesa's Vulkan
  drivers, which use it where offered. scoot waits for an app's GPU to
  finish a frame before showing it, without holding up anything else: an
  app whose GPU never finishes freezes only its own window. It also tells
  the app when a buffer is free to reuse only once scoot is really done
  with it. Nothing to configure. It is not offered anywhere else (pixman,
  `--headless`, `--nested`), where it could not be honoured. The startup
  log says whether it was: `explicit sync (wp_linux_drm_syncobj_manager_v1)
  offered`. Seen on the dev VM with a test client; not yet with an NVIDIA
  or Vulkan app or on real GPU hardware ([Asahi.md](Asahi.md), Test 7).
- **A client is disconnected if it has more than 64 frames waiting on its
  GPU at once, or more than 128 sync timelines open.** No real app comes
  near either limit.

### 2026-09-23 — resizing a `--nested` window no longer rebuilds the GPU renderer

- **Under `--renderer gles`, a resize now keeps the renderer** and swaps in a
  new render target at the new size instead of starting a whole new GPU
  context. Dragging the edge of a `--nested --renderer gles` window used to
  pay that rebuild for every size it passed through; now the resize itself
  costs about what it does under pixman, and what is left is drawing the
  frame at the new size. Nothing to change on your side; pixman, the
  default, is unaffected.

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
  changes — and one that did not is no longer sent a new, identical frame
  every time the pointer moves under `--tty`.

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
