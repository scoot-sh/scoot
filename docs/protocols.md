# Wayland protocols

Written for someone porting a bar, a launcher, a locker or a shell to scoot.
Every global listed here is available to every client — scoot has no
security-context support to distinguish a privileged client from any other,
so an allow-list would be theatre. **A same-uid process is inside the trust
boundary**: anything that can reach scoot's Wayland socket can read your
screen, read your clipboard and take over an abandoned session lock. It could
read your files anyway.

## What is implemented

| Protocol | Version | State |
| --- | --- | --- |
| `xdg-shell` | 7 | Windows and popups. An `xdg_toplevel` is a column entry, told it is [tiled](#tiled-windows) on all four edges, or a [floating](#floating-windows) window, told neither; [`set_fullscreen`](#fullscreen) is honoured. |
| `xdg-dialog-v1` | 1 | `xdg_wm_dialog_v1`: a toplevel marked as a dialog (modal or not) [floats](#floating-windows) when it maps. |
| `xdg-decoration-v1` | 1 | `zxdg_decoration_manager_v1` — server-side decorations, so a client stops drawing its own titlebar; see [`prefer_no_csd`](configuration.md#appearance). scoot draws a focus ring, never a titlebar. |
| `wlr-layer-shell-v1` | 5 | [Bars, docks, wallpapers, launchers](#layer-shell-bars-wallpapers-launchers). |
| `ext-workspace-v1` | 1 | [Workspaces](#workspaces-ext-workspace-v1). |
| `ext-foreign-toplevel-list-v1` | 1 | [Window lists](#window-lists-two-protocols), enumeration only. |
| `wlr-foreign-toplevel-management-v1` | 3 | [Window lists](#window-lists-two-protocols), with `activate`/`close`/`set_fullscreen`. |
| `wlr-output-management-v1` | 4 | [Display information](#display-information-wlr-output-management-v1) — read-only. |
| `ext-image-copy-capture-v1` | 1 | [Screen capture](#screen-capture-ext-image-copy-capture-v1), output only. |
| `ext-image-capture-source-v1` | 1 | Output sources only; no toplevel source manager. |
| `zwp_linux_dmabuf_v1` | 6 | [Real dmabuf import](#gpu-rendering-clients-zwp_linux_dmabuf_v1), formats derived from the active renderer: `LINEAR` single-plane under pixman, the driver's own formats and modifiers (tiled, multi-plane YUV) under GLES; a [scanout tranche](#per-surface-feedback-the-scanout-tranche) for a fullscreen window on the GPU scanout tier. |
| `linux-drm-syncobj-v1` | 1 | [Explicit sync](#explicit-sync-linux-drm-syncobj-v1) for GPU clients, **only on the `--tty` GPU scanout tier** and only where the DRM device supports syncobj timelines with eventfd. Not offered anywhere else. |
| `ext-session-lock-v1` | 1 | [Screen locking](#screen-locking-ext-session-lock-v1). |
| `ext-idle-notify-v1` | 2 | [Idle detection](#idle-detection). |
| `idle-inhibit-v1` | 1 | [Idle inhibitors](#idle-detection). |
| `wlr-data-control-v1` | 2 | [Clipboard managers](#clipboard-and-primary-selection). |
| `ext-data-control-v1` | 1 | Clipboard managers, successor protocol. |
| `primary-selection-v1` | 1 | Middle-click paste, focus-gated. |
| `wlr-gamma-control-v1` | 1 | [Night light](#night-light-wlr-gamma-control-v1). |
| `wp-cursor-shape-v1` | 2 | [Cursor shapes](#cursor-shapes-wp-cursor-shape-v1) from the installed theme. |
| `xdg-activation-v1` | 1 | [Focus handoff](#focus-handoff-xdg-activation-v1). |
| `xdg-toplevel-icon-v1` | 1 | [Window icons](#window-icons-xdg-toplevel-icon-v1). |
| `text-input-v3` | 1 | [Input methods](#input-methods-text-input-v3-input-method-v2). |
| `input-method-v2` | 1 | Input methods. |
| `wp-fractional-scale-v1` | 1 | [Output scaling](#output-scaling). |
| `wp-viewporter` | 1 | Output scaling, and `wp_single_pixel_buffer` scaling. |
| `wp-single-pixel-buffer-v1` | 1 | [Single-pixel buffers](#single-pixel-buffers). |
| `relative-pointer-v1` | 1 | [Relative pointer](#relative-pointer-and-pointer-constraints). |
| `pointer-constraints-v1` | 1 | Pointer lock and confinement. |
| `tablet-v2` | 1 | [Drawing tablets](#drawing-tablets-tablet-v2) — tools only, no pads. |
| `wp-presentation-time` | 2 | [Presentation feedback](#presentation-time-feedback-wp_presentation); `zero_copy` for a buffer scanned out directly. |
| `wp-alpha-modifier-v1` | 1 | [Whole-surface opacity](#rendering-hints). |
| `wp-content-type-v1` | 1 | Accepted, [no effect](#rendering-hints). |
| `xwayland_shell_v1` | 1 | [XWayland, opt-in skeleton](#xwayland-opt-in-skeleton): X-window-to-surface association; no X window enters the layout yet. |
| `zwp_xwayland_keyboard_grab_manager_v1` | 1 | [XWayland, opt-in skeleton](#xwayland-opt-in-skeleton): the grab manager exists; no grab is ever granted yet. |

**Not implemented:**

- **XWayland window mapping.** The server half above is an opt-in skeleton
  (`--xwayland`, needs an `xwayland` build): X11 clients connect and get a
  `DISPLAY`, but their windows map nowhere yet. See below.

The rest of this list *is* deliberate:

- **`wlr-screencopy-v1`.** The clients that motivated capture already speak
  the `ext-` protocol: `grim` 1.5.0 carries `ext_image_copy_capture_v1` and
  nothing else, and stock quickshell 0.3.1 — the build both DMS and Noctalia
  run on — carries the `ext-` manager and both `ext-` source managers.
- **`ext_foreign_toplevel_image_capture_source_manager_v1`.** A single window
  cannot be captured on its own; the global is not advertised, so a client
  takes its fallback path immediately instead of discovering a refusal at
  runtime.
- **`hyprland-toplevel-export-v1`.** The other interface a per-window
  thumbnail is commonly requested through, and also not advertised. Stock
  quickshell routes per-window thumbnails here, so it falls back cleanly
  rather than failing.
- **Maximized and minimized window states.** scoot has no concept of either,
  so the state bits are never sent and the matching requests do nothing — a
  taskbar's minimise button is inert rather than lying. (Fullscreen is real:
  see [Fullscreen](#fullscreen).) The `xdg_toplevel` `wm_capabilities` event
  still lists `maximize` and `minimize` — Smithay's default set, unchanged
  here — so a version 5+ client may show those buttons; pressing them does
  nothing.

## Per-client limits on what scoot keeps

A client can make scoot keep file descriptors on its behalf: every
`wl_shm.create_pool` hands over one, every `zwp_linux_buffer_params_v1.add`
hands over one per dma-buf plane, and every
`wp_linux_drm_syncobj_manager_v1.import_timeline` one more. scoot counts
those **per client, for as long as each fd is really open** — not for as
long as the object it arrived on exists. That difference matters: a
buffer a surface still shows keeps its pool's (or its planes') fds after
the client has destroyed both the `wl_buffer` and the pool, and a sync
point keeps its timeline's fd after the timeline object is destroyed.
Where the GLES renderer keeps a copy of each imported dma-buf plane's fd
(Mesa's software renderer does; hardware drivers are expected not to, but
that is not yet measured), the copy counts too, against the client whose
buffer it is, from the moment the plane is added.

- **512 fds per client**, every kind together. The request that would take
  the client past 512 is refused; a plane's renderer copy is charged when
  the plane is added, so importing it later can never take a client past
  the limit. Before refusing, scoot checks which of the client's fds have
  really closed, so a client that allocates and releases buffers does not
  creep toward the limit. The refusal kills only that
  client: `wl_shm.create_pool` gets `invalid_stride` on `wl_shm`, an `add`
  gets `wl_display.error` `no_memory`, `import_timeline` gets
  `invalid_timeline`. The message says which limit was hit and how many
  fds scoot still held.
- **128 fds per client while the compositor's fd table is nearly full**
  (fewer than 128 of its fds free). Past that, the client's next pool,
  plane or timeline is refused the same way. A client under 128 is never
  refused for someone else's use. scoot looks at a client at most once per
  16 new fds, so one it last looked at while the table was calm can go up
  to 16 past where it was then before it is refused.
- **1024 fds a client sent but no request used yet** (128 on a machine
  whose hard fd limit is 1024; see below). Every fd travels attached to a
  request, and one attached to a request that takes no fd is never used. A
  client that leaves more than the limit unused is disconnected with
  `wl_display.error` `invalid_method` ("too many file descriptors queued
  (more than N)"), and the fds are closed. The count includes fds sent
  ahead of the requests that will use them, which any client does when one
  flush carries more than 28 fds (a client on `wayland-client`'s pure-Rust
  backend for every such flush; a libwayland client once its socket filled
  while the compositor was busy). 1024 is libwayland-server's default
  limit, the one compositors built on it (mutter, KWin, sway, weston) apply
  unless they raise it. This
  limit lives in scoot's fork of wayland-backend ([forks.md](forks.md)).
- **Object limits on top**, unchanged: 512 live `wl_buffer`s, 128 live
  `wl_shm_pool`s, 32 planes added to params objects not yet made into a
  buffer (see [GPU-rendering clients](#gpu-rendering-clients-zwp_linux_dmabuf_v1)),
  128 imported timelines and 64 commits waiting on acquire points (see
  [Explicit sync](#explicit-sync-linux-drm-syncobj-v1)).

Real clients are far below all of these: a `foot` window keeps 2 fds, a
GPU client one per buffer it has allocated (a few per window), a Vulkan
window 16 timelines. At every limit at once, one client can make scoot
hold about 1600 fds, unused ones included, against the point (65408 of
65536) where scoot starts turning newcomers away. Many clients together
still can reach it (see
[`pressure-many-light-connections`](backlog/core/pressure-many-light-connections.md)).

**scoot raises its own fd limit at startup** (the soft `RLIMIT_NOFILE` is
set to the hard limit capped at 65536, which also lowers a larger one;
logged at startup), and **every program it starts gets the original limit
back**: apps spawned by a keybinding, IPC
`spawn`, `[autostart]` or the session command see the limit scoot was
started with, so a program that uses `select()` is not handed fds it
cannot watch. (The XWayland server raises its own limit to the hard limit
whatever it inherits; that is upstream Xwayland's choice.) Where the hard
limit is 1024 (some containers), nothing is raised, the log says so, and
the smaller figures apply: 128 unused fds per client, about 710 fds per
client at every limit against a line of 896, and a libwayland client that
queued more than about 128 fd-carrying requests behind a full socket can
be disconnected. Raise the hard limit (e.g. `--ulimit nofile=1024:65536`)
to get the full limits.

## Tiled windows

Every window in the scrolling layout is sent all four `xdg_toplevel` tiled
states (`tiled_left`, `tiled_right`, `tiled_top`, `tiled_bottom`) in the same
configure as its size, from its first configure on. That includes the
configure that answers a re-map: a window that unmaps (a null buffer)
loses all its toplevel state, as xdg-shell says it must. The configure
answering its next map is rebuilt from the layout: its column's size (if
it is visible; a hidden window is sized when it next shows), the tiled
states, `activated` if it has focus, and `ServerSide` decorations
under `prefer_no_csd`. A fullscreen window is sent `fullscreen` instead,
never both, and gets the four back when it leaves. A [floating
window](#floating-windows) is sent neither — it sizes itself, and is told
so. A client bound
to `xdg_wm_base` below version 2 is sent none of the tiled states (they do
not exist at its version).

Being told it is tiled is what makes a client fill its slot exactly. A
client that believes it floats may size itself: `foot`'s default
`resize-by-cells` rounds a floating window down to whole character cells,
which left a sliver of background along its right and bottom edges. GTK
also trims the shadow it draws around a tiled window's edges.

Some clients still draw less than their slot, tiled or not. On the dev VM a
GTK 4 dialog (`zenity --info`) kept its own 300x223 size in a 966x1083 slot,
and `mpv` kept its video's size. scoot rounds the corners and draws the focus
ring around what such a window actually draws, and reports that area as its
`rect` over IPC ([ipc.md](ipc.md#what-the-replies-carry)). It does not use the whole slot.

One case is not solved yet. libadwaita dialogs (`zenity --info` above)
round their own corners, with a larger radius than scoot's. With
`corner_radius` set, a crescent of background can show at each corner,
between the dialog's own curve and scoot's tighter ring. Such dialogs now
[float](#floating-windows) (they carry `xdg_dialog_v1`), so the ring hugs a
window drawn at its own size rather than a short client in a tall column,
but the corner radius still does not match. Tracked in
[`core/client-rounded-corners-vs-ring.md`](backlog/core/client-rounded-corners-vs-ring.md).

## Floating windows

Dialogs, file pickers, settings windows and anything a user picks with a
rule float above the scrolling strip instead of taking a column. What
floating means for the layout — placement, stacking, focus, what the toggle
puts back — is in [configuration.md](configuration.md#floating); this is
the protocol side.

**What floats, decided once, at the window's first commit.** By then a
client has sent its app id, title, parent and size limits. In order:

1. an `xdg_dialog_v1` object (`xdg_wm_dialog_v1.get_xdg_dialog`, modal or
   not) — GTK 4 attaches one to its dialogs: `zenity --info`, `--question`
   and `--file-selection` (GTK 4.22) all float this way;
2. a parent (`xdg_toplevel.set_parent`) — GTK 3's dialogs (the About dialog
   of `gtk3-widget-factory` 3.24.52) float this way;
3. a fixed size — equal, non-zero `set_min_size` and `set_max_size` on both
   axes;

each off together under `[floating] auto = false`; then the
[`[[window_rule]]`s](configuration.md#window_rule), which have the last
word (`float = false` keeps a dialog in the strip). A hint, parent or title
that changes after the first commit re-decides nothing, and a window that
unmaps and maps again keeps whatever it was.

**What the window is told.** A toplevel is placed as a column the moment it
is created (before its first commit), so its first configure is the tiled
one. The first commit's answer adds a second configure: size `0x0` (the
client chooses) — or a rule's `size` — and no `tiled_*` state. Both arrive
in the same flush, and a client acks the newest before drawing, so its
first frame is at the size it chose: on the dev VM `zenity --info` drew its
300x223 dialog, and `foot --app-id` matched by a rule its 693x500 default,
rounded to whole cells as a floating `foot` does. From then on it is sent no
size unless it draws itself larger than the output's usable area, which asks
it to fit. Floating a tiled window sends `0x0` with the tiled states
cleared, and the client picks its floating size again (`foot` went back to
its 693x500 default on the dev VM). Un-floating sends its column's size with the tiled states back.

**Drawn above the strip, below the `top` layer.** A floating window is drawn
over every tiled window on its output, with its focus ring drawn over them
too (directly under the window itself, so of two overlapping floating
windows the upper one's ring shows over the lower one), and below `top`
and `overlay` layer surfaces. The pointer and clicks follow the same order.
A lock screen covers it like everything else. It is drawn and clickable only
on its own output. When a floating window appears, moves or is raised under
a pointer that is not moving, the pointer is re-entered on whatever is now
under it, so the next click goes to the dialog, not the window beneath —
what is drawn under the pointer is what a click reaches, the same rule a
fullscreen window appearing follows. A click meant for the window beneath,
made in the instant a dialog appears there, lands on the dialog; for a
window the user is looking at that is the right target.

**Popups** of a floating window are fitted into its output's usable area
like any window's (see [Popup menus](#popup-menus-xdg_popup)). A popup is
drawn with its own window, so a menu opened from a *tiled* window draws
below any floating window it runs under; open floating windows are above the
strip, its menus included, by design.

**Output changes.** A floating window keeps its place relative to its
output. When the output changes size (a scale change, a `--nested` window
resized), or the window lands on another output because its own went away,
it is re-centred on its parent (or the output) there rather than left at a
position measured on the old one -- including a window the user had moved
there, and a drag under way ends.

**Stacking.** Floating windows are drawn in stacking order (the most
recently focused on top), except that a window's own floating dialogs are
always drawn above it: clicking an app raises and focuses it, and its modal
dialog stays visible over it rather than going under it.

**Moving and resizing: `xdg_toplevel.move` and `.resize`.** A client-side
titlebar drag (GTK's headerbar) or border drag works on a floating window.
The request is honoured only while the button press it rides on is still
held: the pointer's grab must be the implicit grab that press installed,
under the request's serial, and that press must have gone to the requesting
client's own surface -- so a stale serial, a guessed one, another client's
press, or the serial of a grab that holds no button (a popup's, which is
installed under whatever key or enter serial the client offered) is refused
(logged at debug), and nothing happens. A request from a tiled window is
ignored: tiled windows are placed by the strip, and the client's own drag
simply carries on with nothing moving. So is one from a fullscreen window,
and any while the session is locked. `resize` with edge `none` resizes
nothing. While the compositor holds the drag, the client gets a pointer
`leave`, no button events, and an `enter` when it ends.

The same drag starts from `[floating] modifier` (Super) held with the left
button (move) or the right (resize from the nearest edge or corner) anywhere
on a floating window; that press and its release are never delivered to the
client. During a resize the window is sent configures carrying the
`resizing` state and the size the drag asks for (clamped to its own
`min_size`/`max_size`, re-read when the drag starts, and to the room to the
usable area's edge, which wins over a minimum that does not fit), paced
to the client: the drag sends a new size only once the client has acked the
last, so a 1000Hz mouse does not queue sizes a 60Hz client has to skip.
Best effort, not a guarantee of one outstanding configure: any other
relayout meanwhile (the client's own resized frame, say) also sends the
newest size. The edge not being dragged stays put whatever size the client settles
on (a terminal rounding to whole cells, say). The drag's last configure
drops `resizing` and keeps the size. X11 windows (the XWayland skeleton)
stay refused: their `move_request`/`resize_request` start nothing.

## Fullscreen

A client's fullscreen button works: `xdg_toplevel.set_fullscreen` puts the
window into scoot's fullscreen state, and `unset_fullscreen` takes it out.
The same state is reachable three more ways — a taskbar's
`wlr-foreign-toplevel` `set_fullscreen`, the `Super+f` bind, and IPC
`toggle-fullscreen` / `set-fullscreen ID on|off` ([ipc.md](ipc.md#actions)).

**What the window is told.** Every request is answered with a configure, as
the protocol requires, even one that changed nothing. Entering sends the
`fullscreen` state bit with the output's whole size; leaving sends the bit
cleared with its column's current tiled size — the size it had before, unless
the layout changed around it meanwhile (a config reload, a resized output). A request made before the
window's first commit (`foot --fullscreen` does this) is what its first
configure carries, so its first frame is already fullscreen. Unmapping (a
null buffer) discards the state, as xdg-shell says it must: a window that
maps again comes back tiled.

**What it covers.** While its column is the focused one of its output's
active workspace, a fullscreen window covers that output edge to edge:
the layout gaps, the focus ring and a bar's exclusive zone included, with
no rounded corners even when `corner_radius` is set. On that output, while
it covers:

- the `background` and `bottom` layers are below it, as always;
- the **`top` layer is hidden** — not drawn, not hit by the pointer (a click
  where the bar was reaches the window), and given no keyboard, so a
  launcher on the `top` layer that maps meanwhile waits until the output is
  uncovered (launchers on `overlay`, such as fuzzel's default, are
  unaffected). This includes notification daemons that draw on `top`: mako
  does by default, so its notifications are hidden under a fullscreen window
  unless it is configured with `layer=overlay`;
- **focus goes to the fullscreen window when it starts covering.** A
  launcher or other `exclusive` surface on the `top` layer that held the
  keyboard loses it to the window at that moment — the focused window going
  fullscreen hides the launcher, and a hidden surface cannot hold the
  keyboard. It gets it back as soon as the output is uncovered, if it is
  still mapped. `overlay` surfaces keep the keyboard;
- the **`overlay` layer stays above it** — notifications and OSDs that draw
  there still show, and still take clicks and an `exclusive` keyboard;
- a **lock screen** covers everything, fullscreen windows included.

Other outputs are untouched: fullscreen is per output.

**On the GPU scanout tier** (`--tty --renderer gles`, `gpu-scanout` build),
a covering fullscreen window whose buffer is a dma-buf the display can take
is scanned out directly -- shown from the client's own buffer, with no
compositing -- provided it is opaque (an opaque-format buffer, or an opaque
region covering it), or the background is black and no wallpaper other than
a black single-pixel-buffer one lies under it. Anything drawn over it (an `overlay` notification, a popup
menu, a cursor the hardware cursor plane cannot carry), a translucent
window (`wp_alpha_modifier_v1`), a lock screen, or a client capturing the
screen makes those frames composite instead; nothing changes on screen
either way. Mechanics and where it has been seen: [tty.md](tty.md).

**What it keeps.** The window keeps its column: focus another column and the
view scrolls there the usual way. Focused away, the fullscreen window keeps
its fullscreen size and sits in the strip exactly where a column that wide
would, one ordinary gap from its neighbours on either side — so it can
still show, partly, beside the focused window, but never over it, and never
on another output: the part of it past its own output's edge is neither drawn
nor clickable (see [More than one
output](configuration.md#more-than-one-output)). Nothing
covers the output then, so `top`-layer surfaces are drawn again (a bar on
`top` is drawn over the fullscreen window where they meet; one on `bottom`,
waybar's default, stays under it). Focus back and it covers the screen
again; switching workspaces works the same.
Leaving restores the layout exactly. Other windows stacked in its column are
hidden while it holds; focusing one of them ends the fullscreen, as does
moving the window to another workspace or output, or consume/expel. A window
that closes while fullscreen just leaves the layout. Only a column's focused
window can go fullscreen — a request from a window stacked under another in
its column is answered with a configure that leaves it tiled.

**The output hint.** `set_fullscreen(output)` is honoured when the
requesting window is the focused one and the session is unlocked: the window
moves to that output (focus follows, as with `move-window-to-output`) and
goes fullscreen there. Otherwise the hint is ignored and the window goes
fullscreen on the output it is on — a client cannot move a window the user is
not looking at to another screen.

**While locked.** A window's own request is honoured (it is not drawn, and
the session is as the client left it at unlock); requests on the user's
behalf — a taskbar, the bind, IPC — are refused like every other action.

**Floating windows.** A floating window can go fullscreen; leaving puts it
back where it floated, told `0x0` with no tiled state. The rules are the
same for a fullscreen window in either layer:

- **While it has focus it covers the output**, and the other floating
  windows on that workspace are hidden — except its own dialogs (windows
  whose parent chain reaches it), which stay up and are drawn and clicked
  above it. Clicking a fullscreen app does not hide the dialog it opened: a
  modal dialog blocks input to its app, and one hidden under it would make
  the app look hung. With a dialog above it the frame stays eligible for
  direct scanout: Smithay composites only when the dialog cannot be given a
  plane of its own (on hardware with a free overlay plane the fullscreen
  window can still go direct, the dialog on the overlay).
- **While a floating window above it has focus** (typically that dialog),
  it stays in place, full size, under the floating layer; nothing counts as
  covering the output then (so the `top` layer is drawn again, and direct
  scanout pauses). A floating fullscreen window also hides the strip then,
  and the floating windows drawn under it -- never its own dialogs, which
  are drawn above it even when it was clicked above them in the stack.
- **While focus is anywhere else** it is not in front: a column keeps its
  strip slot as usual, a floating fullscreen window is hidden.

## XWayland (opt-in skeleton)

`--xwayland` (or `[xwayland] enabled` in the config file — either one turns
it on) starts an XWayland server inside the session, and `DISPLAY` is
exported to everything the session spawns. Off by default, and needs a
build with the `xwayland` Cargo feature: without one the knob warns and
the session runs Wayland-only, and a missing `Xwayland` binary at startup
is the same shape (a loud log line, then a Wayland-only session — the
session never fails to start over X).

Phase-1 skeleton means exactly this: the server starts, X11 clients
connect, and their windows map nowhere. No X window enters the layout,
`scoot msg windows`, or either foreign-toplevel list; map and configure
requests are refused (logged at `debug`), and no keyboard grab is ever
granted. "Xwayland starts" is not "X apps work" — window mapping, the
focus/activation gate, and clipboard/DnD/IME are later phases (see
`docs/backlog/protocols/xwayland-support.md`, which stays open).

**Trust model: running one X client extends full trust to it.** The
same-uid boundary above bites harder here than anywhere else in this file:
X11 clients can keylog and snoop on each other *by design* — no exploit,
no bug, the protocol works that way. Starting the server is harmless on
its own; connecting your first X client is the trust decision. Wayland
clients stay isolated from each other as before.

## Layer shell (bars, wallpapers, launchers)

`wlr-layer-shell-unstable-v1` version 5 — what `waybar`, `swaybg`, `mako`,
`wofi`, `fuzzel` and `yambar` use. Start one the same way you start anything
else inside the session:

```sh
swaybg -c '#123456' &     # a wallpaper, on the background layer
waybar &                  # a bar, on the top layer
fuzzel                    # a launcher, on the overlay layer
```

- **All four layers.** `background` and `bottom` draw behind windows (and
  behind the focus ring, so a wallpaper never hides it); `top` and `overlay`
  draw in front.
- **Anchors, margins and sizing**, including the protocol's rules for a
  surface anchored to opposite edges or given a zero dimension.
- **Exclusive zones.** A bar that reserves its own height shrinks the area
  windows are tiled within, and gives that space back the moment it exits or
  its client dies. Several surfaces reserving on the same edge stack. `-1`
  ("don't push me around") reserves nothing, which is what a full-screen
  wallpaper wants. `scoot msg outputs` reports the result as `usable`.
- **Pointer input.** A click, scroll or motion over a layer surface goes to
  that surface, not to whatever window is behind it, and clicking a bar does
  not move window focus.

### More than one output

Each output keeps its own layer surfaces, its own exclusive zones and its
own keyboard derivation:

- A bar reserves the edge of the output it is mapped on, and no other's. A
  bar on the second screen never shrinks the first screen's tiling area.
- A surface that names an output (`get_layer_surface` with a `wl_output`)
  appears on that output. A surface that names none appears on the first
  output — the compositor's choice. New windows, by contrast, open on the
  output under the pointer (falling back to the first when the pointer is
  over no output).
- Pointer input and keyboard focus follow the output under the pointer: a
  click on a second-screen bar focuses that bar, and an `exclusive`
  launcher mapped where the pointer is takes the keyboard there. If
  `exclusive` surfaces are mapped on two screens at once, the pointer's
  screen wins; any screen's `exclusive` surface still outranks every window,
  so a launcher stays usable with the pointer on the other screen.

### Keyboard focus

Following the protocol's `keyboard_interactivity` — except under a
fullscreen window, where the `top` layer is hidden and holds no keyboard
([below](#fullscreen)):

- `none` (the default, and what a bar, wallpaper or notification daemon asks
  for) never takes the keyboard.
- `exclusive` on `top` or `overlay` takes the keyboard as soon as the surface
  maps and holds it until it unmaps — what a launcher needs. If several ask
  at once, the front-most wins, and the keyboard falls back to the next one
  down when it goes away.
- `on_demand` is click-to-focus, exactly like a window: click the surface to
  give it the keyboard, click a window, a bar or bare desktop to take it
  back. `exclusive` on `bottom` or `background` is treated the same way — the
  spec allows normal focus semantics there, and nothing should be typing into
  a wallpaper unasked. **A keybinding does not release an `on_demand`
  surface**: a hotkey pressed while you are typing in a panel's search field
  keeps typing in that panel, which is where the keyboard visibly is. A
  *request* to move focus does release it — an IPC focus action, a
  `wlr-foreign-toplevel` `activate`, an `ext-workspace-v1` `activate` or an
  `xdg-activation-v1` token all hand the keyboard on to the window.
- A surface that has committed but never attached a buffer, or that unmapped
  itself, can't hold the keyboard however it asks.
- A click-focused surface can hand the keyboard back itself, by committing
  `keyboard_interactivity: none` or by unmapping — and the click that focused
  it is spent when it does. Asking for `on_demand` again, or mapping again,
  gets it drawn again but not focused again: it waits for a fresh click. So a
  bar that collapses and re-opens a search field never gets the keyboard back
  without the user actually clicking it. (An `exclusive` surface on
  `top`/`overlay` is the exception: it takes the keyboard whenever it is
  mapped, clicked or not.)

**Keybindings always win.** They are matched before anything is forwarded to
the focused client, so `Super+Shift+E` (quit) and, on `--tty`,
`Ctrl+Alt+F1`..`F12` (VT switch) still work while a full-screen layer surface
is holding every keystroke. That is the escape hatch if one wedges.

**A layer-shell "lock screen" is not a security boundary** — use a real
`ext-session-lock-v1` locker. An `exclusive` layer surface will receive what
you type instead of leaking it to the window behind, but the escape hatch
that makes exclusive focus safe is also a way around such a lock: the quit
binding and the VT switches keep working while it is up, and everything
behind it is still drawn and still capturable. Treat one as a screen
*blanker* you can type a password into.

### Popup menus (`xdg_popup`)

An application menu, a combo box, a bar's own dropdown: all `xdg_popup`
surfaces, and all of them map, draw, take clicks and take the keyboard.

- **A grab routes input into the menu.** While an explicit `xdg_popup.grab`
  is held, keyboard focus is on the popup, so Escape-to-close, arrow-key
  navigation and typeahead reach it; clicking inside keeps it up; clicking
  outside dismisses it. Dismissal is also the only thing that ever sends
  `popup_done`, so without a grab nothing closes a menu.
- **Submenus nest.** A grab on a popup whose parent is the current grab takes
  over, and closing it unwinds to the parent menu rather than closing the
  whole chain.
- **A bar's own dropdowns work too** — a popup parented to a *layer* surface
  (`zwlr_layer_surface_v1.get_popup`), not just to a window. As the
  protocol says, the popup must be created with a null parent and handed
  to the bar before it is first configured; anything else is refused (see
  below).
- **A menu is kept on its screen.** A popup that lets the compositor adjust
  it (`xdg_positioner.set_constraint_adjustment`: flip, slide, resize —
  GTK3's context menus ask for all six) is flipped, slid or resized, in the
  protocol's order, to fit inside the screen its window is on, instead of
  being cut at the edge. The edge between two screens counts like an outer
  one: a menu is moved back onto its own window's screen, never drawn over
  the neighbour. The area it is fitted into:
  - a window's menu: the screen minus any bar's exclusive zone — the same
    area windows tile into — because a window's menus draw *under* a
    `top`-layer bar, and a menu slid under the bar would be hidden by it;
  - the menu of the window covering its screen fullscreen: the whole
    screen, since the bar is not drawn then;
  - a bar's own dropdown: the whole screen, bar zone included.

  A popup that asked for no adjustment on an axis is left exactly where it
  asked to be on that axis, and cut if that is off the screen — the
  protocol's rule. The fit is applied when the menu opens and on
  `xdg_popup.reposition`. It is not redone afterwards: a `reactive` popup
  whose window scrolls while it is open keeps its position (tracked in
  [`docs/backlog/core/popup-reactive-reconstrain.md`](backlog/core/popup-reactive-reconstrain.md)).
- **Popups nest at most 64 deep, and cannot loop.** A menu, its submenu,
  that submenu's submenu and so on may go 64 levels deep — real menus stop
  at a handful — and a 65th is refused. That holds for how scoot itself
  stores and draws them, not just for each popup's parent chain: no
  application popup ever sits more than 64 levels down, however the client
  got it there (an input method's candidate window over the deepest one
  can add a 65th level; it is never anyone's parent). So is a popup that
  is its own ancestor (its own `xdg_surface` named as its parent). Before, a chain a
  few thousand deep crashed the compositor, and a loop froze it, taking
  every other client down too. A refused popup's client is disconnected
  with a protocol error, and so is a client that breaks one of the
  protocol's rules that keep an open chain from growing afterwards, which
  scoot enforces where the pinned Smithay does not:

  | What the client did | Protocol error | Posted on |
  |---|---|---|
  | Nested a popup more than 64 deep, or made one its own ancestor | `xdg_wm_base.invalid_popup_parent` | the new `xdg_popup` |
  | Made a popup of an `xdg_surface` with no live `xdg_toplevel` or `xdg_popup` (a bare one, or one whose popup was destroyed) | `xdg_wm_base.invalid_popup_parent` | the new `xdg_popup` |
  | Called `get_popup` for a `wl_surface` whose popup is still alive (on the same `xdg_surface` or a second one) | `xdg_surface.already_constructed` | the new popup's `xdg_surface` |
  | Destroyed a popup that still has child popups open | `xdg_wm_base.not_the_topmost_popup` | the destroyed popup's `xdg_surface` |
  | Handed a bar (`zwlr_layer_surface_v1.get_popup`) a popup that was created with a parent, or was already configured | `xdg_wm_base.invalid_popup_parent` | the `xdg_popup` |

  The `xdg_wm_base` errors are posted on another object because the pinned
  Smithay keeps the client's `xdg_wm_base` private: the code is
  `xdg_wm_base`'s, and the message starts with the error's name — read
  that, since on `xdg_surface` the same number means something else. Each
  refusal also logs a `warn` naming the client and the reason. Closing
  menus innermost first, as GTK 3 does (and as wlroots and mutter also
  require), is unaffected, and so is a client disconnecting with menus
  open. An input method's candidate window over a menu's text field does
  not count as the menu's child: the menu can close under it.
- **The grab's serial has to name a real interaction.** A grab is refused —
  dismissed, with a warning in the compositor log naming the client and
  serial, since the protocol posts no error — unless its serial is a recent
  key, button or focus `enter` actually delivered to the client asking, so a
  client that was never focused cannot take the keyboard on its own say-so. A
  grab that continues the client's own open menu (a nested submenu, or a menu
  replacing the one just closed) is not refused for reusing its opening
  serial past that window.
- **Keyboard focus does not move onto a popup that did not grab.** A tooltip
  is an ordinary `xdg_popup` too, and handing one the keyboard would take it
  away from the window you are typing into.

**A grab loses to the compositor's own focus rules**, in this order:

1. **The session lock wins.** Locking dismisses an open menu, and a grab
   requested while locked is refused.
2. **An `exclusive` layer surface on `top`/`overlay` wins** — a launcher
   opened over a menu is typeable, and the menu is dismissed rather than left
   on screen holding input it can no longer use.
3. **An input method holding the keyboard wins.** While an IME (fcitx5, or
   any `zwp_input_method_v2` client holding its keyboard grab, which a real
   IME does for its whole active span) has the seat, a grab asked for is
   refused, and a grab already held is dismissed when the IME takes the
   keyboard. Letting a menu pre-empt an IME mid-compose would interrupt
   composition in an unrelated text field; leaving the menu mapped with no
   keyboard would leave one Escape cannot close.
4. **The menu wins over everything else**: over the focused window, over a
   layer surface that got the keyboard from a click, and over the `exclusive`
   surface the menu itself hangs off — so a bar's own dropdown is not
   dismissed by the bar that opened it. A *different* `exclusive` surface
   still wins per rule 2.

Losing is always spelled `popup_done`, never "leave it up but take its input
away". Keybindings still win over a grab, so a client cannot wedge the
session by holding a menu open.

For what a grab means to an agent driving the socket, see
[ipc.md](ipc.md#rules-an-agent-needs).

### Subsurfaces (`wl_subsurface`)

A video under its player's window, a toolkit's offloaded texture, a
client-drawn titlebar: `wl_subcompositor.get_subsurface` works as the core
protocol says, with one limit.

- **Subsurfaces nest at most 64 levels below a surface tree's root.** The
  root is the surface at the top of the tree, the one with no parent: a
  window, a popup, a layer surface, a cursor, a plain surface with no role
  yet -- or a subsurface cut loose from its parent, by
  `wl_subsurface.destroy` or by its parent `wl_surface` being destroyed,
  which keeps its role and its own subsurfaces but is a root until it is
  attached again. Its own subsurface is level 1, a subsurface of that is level 2,
  and so on; real clients use a level or two. That holds for every tree,
  however it was built. A `get_subsurface` is refused if it would put *any*
  surface deeper than level 64, and that includes the subsurfaces already
  hanging below the surface being attached, so a tree built bottom-up,
  re-attached after `wl_subsurface.destroy`, or re-attached after its
  parent `wl_surface` was destroyed is held to the same limit as one built
  a level at a time. Before, a tree a few thousand levels deep crashed the
  compositor, taking every other client down too.

  | What the client did | Protocol error | Posted on |
  |---|---|---|
  | Called `get_subsurface` where the new subsurface, or a subsurface already below it, could end up more than 64 levels below its tree's root (see below for "could") | `wl_subcompositor.bad_parent` | the `wl_subcompositor` |

  The message starts with `bad_parent:` and gives an upper bound on the
  level the deepest surface could have reached (the `warn` the refusal
  logs, naming the client, gives the same bound as `deepest_bound`). The `wl_subsurface` is not created, and the surfaces are not
  linked.

  What hangs below a surface is judged by a recorded height that is never
  lowered, which keeps the check cheap however wide a client's trees are.
  It can be higher than anything really there, in two ways: a surface
  keeps the height of subsurfaces it has since lost, and it passes that
  height on to every surface it is later attached below. So a surface that
  once had, say, 60 levels of subsurfaces below it and lost them, then
  attached below a plain surface, makes that plain surface count as 61
  levels high too -- and attaching *that* one 3 levels deep is refused,
  though the real tree would be 5 deep. The error only ever goes that way:
  a tree that really is too deep is always refused. No real client comes
  near it: the deepest measured is 2 levels.

  A parent that is the surface itself, or one of its own subsurfaces, is
  still refused as before, with `wl_subcompositor.bad_surface`.

## Workspaces (`ext-workspace-v1`)

Version 1, the compositor-agnostic successor to the one-off wlr workspace
protocols, so a panel's workspace module, a workspace switcher or an
indicator can list workspaces, follow which one is active and switch between
them. The global is `ext_workspace_manager_v1`.

- **One workspace group per output**, each carrying its own output. A client
  that binds `wl_output` after the manager still gets an `output_enter` for
  it on that output's group, so registry order doesn't matter. Each group's
  workspaces are positions in that output's own list with that output's own
  active index: switching on one output never disturbs another's.
- **One `ext_workspace_handle_v1` per workspace**, named `"1"`, `"2"`, … in
  layout order, with matching one-dimensional `coordinates` — sort by those,
  not by name (`"10"` sorts before `"2"` as a string). The active one carries
  the `active` state bit; nothing else is ever set.
- **`activate` is the only capability advertised**, on workspaces. The group
  advertises none.
- **Changes arrive in batches closed by `done`**, one per change, and none at
  all when the workspaces didn't change. Draw on `done`, not on each event: a
  switch is one `state` event turning the old workspace off and another
  turning the new one on, and the list is momentarily inconsistent between
  them.

To switch a workspace, send `activate` on its handle **and then `commit` on
the manager** — the protocol batches requests, so an `activate` with no
`commit` after it does nothing.

Worth knowing before you write against it:

- **Workspaces are positions, not identities, and scoot sends no `id`
  event.** The list grows as you use the trailing empty workspace and shrinks
  when a workspace empties out, which renumbers everything after it: handle 2
  means "the third workspace", not "that workspace". Redraw from what the
  last `done` said rather than remembering a handle as a particular user's
  workspace.
- **`deactivate`, `remove`, `assign` and `create_workspace` are ignored**,
  and no capability is advertised for them: an output always has exactly one
  active workspace, workspaces are created and dropped by the layout itself
  rather than by the user, and a workspace belongs to the output whose group
  announced it.
- **`activate` is a request, not a guarantee** (as the protocol says): one
  naming a workspace that vanished between the client reading the list and
  the `commit` arriving is dropped, one for the workspace that is already
  active does nothing, and one for an output that is not the focused output
  is dropped unless it is already active there too — switching another
  output's workspaces from a bar is a later phase's work, and silently
  switching the wrong output would be worse than refusing.
- **An agent switches by number too.** `scoot msg action
  focus-workspace-index N` drives the same core action `activate` does, on
  the focused output.
- **With one output the wire is exactly what it always was** — one group,
  one `done` per batch. A second output adds a second group block before
  that `done`, nothing else.
- **A group never changes outputs, so `output_leave` never fires on one.**
  Each group is announced with its own output and keeps it: moving a window
  across outputs changes workspace membership, not group assignment (the
  protocol's `output_leave` is for an output *removed* from a group, which
  only runtime output add/remove could produce — out of scope, outputs are
  fixed for the session). The `ext-foreign-toplevel-list-v1` next door has
  no output events at all, so a move changes nothing on it either.
- **One client may hold at most 8 binds** across this manager, both
  window-list globals and the display manager combined; a ninth is answered
  `done` then `finished` rather than announced. A bar binds this global once.

## Window lists (two protocols)

scoot publishes its window list through **two** protocols at once:

- **`ext-foreign-toplevel-list-v1`** (version 1, global
  `ext_foreign_toplevel_list_v1`) — the compositor-agnostic successor, and
  what a standards-following taskbar, dock or alt-tab switcher should prefer.
  Enumeration only; the protocol has no requests.
- **`wlr-foreign-toplevel-management-unstable-v1`** (version 3, global
  `zwlr_foreign_toplevel_manager_v1`) — the older protocol, which is what
  every Quickshell-based shell (DMS, Noctalia) actually binds today. Stock
  quickshell 0.3.1 is offered the `ext-` list and never binds it, because its
  `ToplevelManager` is a wlr client; without the wlr protocol both shells'
  window sections render empty. It enumerates *and* controls.

Both are the protocol-side twin of `scoot msg windows`: the same windows, the
same lifetime, live rather than polled — and, because they are driven from
the same three window-lifecycle events inside the compositor, a client bound
to both sees one window list described twice rather than two lists that can
drift.

### `ext-foreign-toplevel-list-v1`

- **One `ext_foreign_toplevel_handle_v1` per window**, carrying `identifier`,
  `title` and `app_id`. Binding the global announces every window that
  already exists, so registry order doesn't matter.
- **Changes arrive in batches closed by `done`.** A window is announced the
  moment its `xdg_toplevel` exists, which is *before* the toolkit has sent
  its app id and title: the first batch is usually two empty strings,
  followed a moment later by a batch per field as they arrive.
- **`closed` when the window goes**, after which nothing else is sent on that
  handle.
- **`stop` is answered with `finished`**, and means "no more *new* windows":
  handles the client already has keep reporting title and app id changes
  until it destroys them, which is what the protocol's own teardown sequence
  requires.
- **Binding counts against the same 8-bind per-client budget** as the
  workspace manager; a ninth bind is answered `finished` rather than
  announced.

**The identifier is `<generation>-<window id>`** — e.g. `a3e689a2-1`: eight
hex digits of per-session randomness (the "opaque generation value" the
protocol recommends, so identifiers from two scoot sessions never collide)
and, after the last dash, the window id `scoot msg windows` reports. That
suffix is the bridge between the two: this protocol has no requests at all,
so a client that finds a window here and wants to *act* on it sends `scoot
msg action focus-window-id N` with the number after the dash.

Worth knowing before you write against it:

- **There is no control half, by design.** No `activate`, `close`,
  `minimize`, `fullscreen`, geometry or per-output state — those are meant
  for extension protocols that don't exist yet. Use the wlr protocol below,
  or scoot's IPC.
- **A handle covers a window's whole life, not the time it is mapped.** The
  protocol talks about "mapped" toplevels, but scoot has no map/unmap
  boundary at all — a window is in the layout, the focus order and `scoot msg
  windows` from the moment its `xdg_toplevel` exists.
- **An identifier is never reused.** Close a window and open another and it
  gets a new one, even from the same client.
- **The list stays live while the session is locked.**

### `wlr-foreign-toplevel-management-unstable-v1`

Per window, on a `zwlr_foreign_toplevel_handle_v1`:

- **`title` and `app_id`**, and **`output_enter`** naming the screen it is
  on. Binding the global announces every window that already exists, oldest
  first — and a client that binds `wl_output` *after* the manager is sent the
  `output_enter` it missed as soon as it does.
- **`state`, carrying `activated` and `fullscreen`** — scoot's real window
  focus, the same one `scoot msg windows` reports as `focused`, and whether
  the window is fullscreen, the same flag `scoot msg windows` reports.
  `fullscreen` only exists from version 2 of the protocol, so a client that
  bound version 1 is never sent it (a change to it alone sends such a client
  nothing).
- **Changes arrive in batches closed by `done`.** A window is announced
  before its toolkit has sent a title or taken focus, so its first batch is
  usually two empty strings and an empty state array.
- **`closed` when the window goes.** The handle then becomes inert: still a
  valid object until the client destroys it, and every request on it is
  ignored.
- **`stop` is answered with `finished`**, same teardown sequence as above.
- **Binding counts against the same 8-bind per-client budget.**

What a client can ask for:

- **`activate`** focuses that window, with the same effect on the keyboard as
  a click on the window itself: if your panel is a layer surface that took
  the keyboard when the user clicked it, `activate` hands the keyboard on to
  the window. It does that whether or not the window was already focused.
  `scoot msg action focus-window-id N` behaves the same way — see
  [ipc.md](ipc.md#rules-an-agent-needs).
- **`close`** sends the window's `xdg_toplevel.close`. Whether the window
  actually goes is up to its own client; `closed` follows if and when it
  does.
- **`set_fullscreen` / `unset_fullscreen`** put that window into
  fullscreen and back, by the same rules as the window's own request and
  `Super+f` (see [Fullscreen](#fullscreen)), without moving focus. The
  optional output is honoured only for the focused window, like the
  client's own hint. Refused while the session is locked, like `activate`
  and `close`.
- **`set_maximized`, `unset_maximized`, `set_minimized`, `unset_minimized`
  and `set_rectangle` are accepted and do nothing.** `set_rectangle` is a minimise-animation hint scoot reads
  nothing from; unlike wlroots, an invalid rectangle is ignored rather than
  answered with a protocol error, because disconnecting a shell over a number
  nothing looks at would be worse.

Worth knowing before you write against it:

- **`activate` needs a seat, and a headless shell may not have one.**
  quickshell sources the `wl_seat` argument from Qt's last input device, so a
  `ShellRoot` with no window and no input event sends nothing at all when you
  call `activate()` — no request reaches the compositor. With a real
  `PanelWindow` and a real click it works. Test it with a window.
- **`output_leave` is sent when a move carries the window across
  outputs** — one `output_leave` for the old screen plus one `output_enter`
  for the new one, closed by `done`, on exactly the window that moved (which
  is the only thing that can change a window's screen: new windows open on
  the pointer's output, and switching workspaces does not move one). Closing a
  window sends `closed` with no `leave` first — the handle's death is the
  `closed` event, and nothing may be sent on it after. A client holding no
  `wl_output` for either screen hears nothing about the move, not even a
  bare `done`.
- **`parent` is never sent** (the version 3 event). scoot's layout has no
  parent/child relation; every `xdg_toplevel` is an independent column entry,
  dialogs included.
- **The list stays live while the session is locked, but the two requests are
  refused** — a window you cannot see must not be focused or closed from
  behind the lock screen.

## Display information (`wlr-output-management-v1`)

`zwlr_output_manager_v1` version 4, which is what `wlr-randr`, `kanshi` and a
shell's Settings → Display page read the screen's modes, position, scale and
transform from. `wl_output` says what the screen *is*; this is the management
protocol on top of it. This is the wlr protocol rather than an `ext-` one
only because no `ext-` successor exists yet.

**It is read-only. `apply` and `test` always answer `failed`.** scoot's
outputs are fixed for the life of the process — created at startup, never
moved, disabled or rescaled by a client — so there is nothing a
configuration could change; a configuration that reported `succeeded` and
changed nothing would give you a Display page whose buttons appear to work.
`wlr-randr --output <name> --pos 100,100` prints `failed to apply
configuration` and exits non-zero.

- **One `zwlr_output_head_v1` per output**, each carrying its own `name` (the
  same one `wl_output` reports — a DRM connector name like `HDMI-A-1` under
  `--tty`, `headless`, `headless-2`, … otherwise), `description`, `make`,
  `model`, `enabled`, `position`, `transform`, `scale` and `adaptive_sync`
  (always `disabled`; scoot has no VRR support).
- **A `zwlr_output_mode_v1` per mode the output knows**, with its size,
  refresh rate and whether it is preferred. Binding announces everything
  immediately.
- **Changes arrive in batches closed by `done`**, carrying a serial that
  advances on every real change.
- **`stop` is answered with `finished`**, after which the head and mode
  objects the client already has stay valid until it destroys them.
- **Binding counts against the same 8-bind per-client budget.**

Worth knowing before you write against it:

- **The refresh rate is always 60 Hz**, including under `--tty` on a faster
  panel. `wl_output` reports the same thing; this mirrors it rather than
  adding a second, differently-wrong number.
- **No `physical_size` and no `serial_number`.** scoot knows neither (its
  physical size is `0x0` on `wl_output` too, and its serial is a
  placeholder), and the protocol allows omitting both. A client that keys a
  saved per-monitor profile off a serial would otherwise match every scoot
  session on every machine.
- **The mode list only grows, never shrinks.** A new mode is added rather
  than replacing the old one, matching what `wl_output` does. Under `--tty`
  that happens once per mode the display actually changes to, so a VM window
  moved between a 2x and a 1x screen a few times leaves a mode for each
  distinct size. Under `--nested` it happens once per size the host
  configures scoot's window to, which for a window dragged to resize is once
  per distinct size that drag passed through. Both are bounded by *distinct
  sizes*, not by how many events arrived: a configure or a hotplug that lands
  on a size already in the list adds nothing, and a `--nested` configure at
  the size scoot is already at does not even resize the output.
- **A `--tty` VT switch changes nothing by itself.** The output does not go
  away when you switch to another VT, it just stops being drawn, so the head
  stays enabled with the same mode and no `done` is sent. The one thing a
  switch *back* can produce is a mode change, and the switch is not what
  caused it: a display plugged in or resized while scoot was on another VT
  could not be acted on then, so the switch back re-probes and applies
  whatever moved.
- **One head per output, still read-only.** `--headless --outputs N`
  announces N heads (see above); `apply`/`test` stay refused — more outputs
  don't change that.

## Screen capture (`ext-image-copy-capture-v1`)

Version 1 together with `ext-image-capture-source-v1` (version 1) — what
`grim`, a shell's workspace-overview live preview, a screen recorder or a
conferencing screen-share uses. Three globals:

| Global | What it is for |
| ------ | -------------- |
| `ext_image_copy_capture_manager_v1` | Creating a capture session and its frames |
| `ext_output_image_capture_source_manager_v1` | Turning a `wl_output` into a capture source |
| `zwp_linux_dmabuf_v1` | Real dmabuf import (GPU-rendering clients), plus format feedback |

`grim` works with no flags:

```sh
grim /tmp/screen.png          # the whole output
grim -t ppm - | ...           # or to stdout
```

This is separate from `scoot msg screenshot`, which goes over the privileged,
owner-only IPC socket and is what an agent uses. This is the
standard-protocol path, for tools that will never speak scoot's own IPC.

What to know before pointing a client at it:

- **Each output is captured from its own framebuffer.** A source made from
  the second output (`grim -o headless-2`) is accepted and reads that
  output's own pixels — never the first output's. A source naming no output
  of this compositor is answered `stopped`.
- **Output capture only.** A source can be made from a `wl_output`; there is
  no toplevel capture source manager, so a *single window* cannot be captured
  on its own. The global is not advertised at all, so a client takes its
  fallback path immediately instead of discovering a refusal at runtime. The
  remaining thumbnail fallback (region crop out of the output) needs no
  compositor work — a screen-source `ScreencopyView` in a clipped container
  at the window's rect — but DMS's `TileItem.qml` hard-requires a `Toplevel`
  source, so the shells must change.
- **`wl_shm` buffers only, `Xrgb8888` or `Argb8888`.** A capture session
  never offers to write into a dma-buf — capture is the direction where scoot
  does the writing, and `wl_shm` already works everywhere at no extra cost to
  a CPU renderer. This is unchanged by the renderer-derived import formats
  below: importing a client's buffer and *rendering into* one are different
  capabilities, and the second has no code path here.

  `Xrgb8888` is offered first: if `[appearance]
  background_color` has an alpha below 1.0 then the framebuffer really is
  translucent, and an `Xrgb8888` capture forces the fourth byte opaque so you
  get a screenshot rather than a translucent image. With the default opaque
  background that pass is skipped — same bytes either way. An `Argb8888`
  capture hands you the framebuffer's own alpha.
- **The buffer size is the framebuffer's, and it is re-advertised on a
  resize.** If `--tty` follows a hotplug to a new mode, every live session
  gets a fresh `buffer_size` + `done`. A capture whose buffer is now *too
  small* is answered `failed(buffer_constraints)`, asking the client to
  re-allocate — but one whose buffer is still large enough (the output
  shrank) succeeds into the oversized buffer, with the client's own margin
  pixels left untouched outside the captured region.
- **A session's first capture is served on the next frame; later ones wait
  for the screen to change.** The protocol allows exactly this, and it is
  what keeps a live-preview client from costing a full-screen copy every
  frame on a desktop that is not moving. A capture parked this way is served
  the moment anything redraws.
- **Captures show rounded corners.** `screenshot` and `screencopy` read the
  composited frame, so a non-zero `[appearance] corner_radius` appears in
  captures: window corners show whatever is behind the window. An agent
  diffing screenshots against expected pixels must account for the session's
  configured radius.
- **At most one *outstanding capture* per session** — a second `capture`
  request before the first has been answered is failed rather than queued.
  And at most sixteen live frame *objects* per client, across all of its
  sessions: a further `create_frame` is refused with the protocol's own
  `duplicate_frame` error, which disconnects the client that overflowed. The
  protocol's own rule is stricter still (one frame object per *session*),
  which the pinned Smithay rev never enforces and exposes no hook to enforce
  per session.
- **While the session is locked, a capture sees the lock screen** — never the
  windows behind it, and never a half-drawn transition.
- **On the GPU scanout tier a capture never sees a stale screen, including
  under a fullscreen window scanned out directly.** A capture of such a
  frame forces one composite frame first. A session that keeps capturing —
  a frame waiting, or one asked for within the last second — keeps the
  output composited instead, so a recorder or screen-share costs what it
  always did; the output goes back to direct scanout about a second after
  the last capture.
- **`paint_cursors` is honoured, on every backend and renderer.** A session
  that asked for it (`grim -c`) gets the pointer composited into every
  capture, exactly as the screen draws it — image, hotspot, scale, over
  whatever is under it; a session that did not (plain `grim`) never does,
  whichever the frame it was read from held. That includes `--headless` and
  `--nested`, which draw no cursor on screen, and the `--tty` GPU scanout
  tier, whose cursor rides a hardware plane. It is done by re-rendering
  just the cursor's region for the capture where the frame does not
  already match the request; mechanics and measured cost per tier in
  [tty.md](tty.md#captures-and-the-pointer) (nothing measurable on
  pixman; a 0.4-0.9 ms region render per capture on the dev VM's
  software-rendered GPU tier). Where the cursor rode an overlay plane that
  may be an underlay — which leaves a transparent hole in the frame a
  capture reads — that place is re-rendered whether or not the pointer was
  asked for, so no capture shows the hole. A session that asked for the
  pointer is also served a new frame when only the pointer moves — nothing
  else has to redraw — while one that did not keeps waiting for the scene
  itself to change, including under `--tty`, where moving the pointer
  redraws the frame. The pointer is only in the capture of the output it is
  on.
- **Cursor capture sessions are refused.** `create_pointer_cursor_session`
  itself gets no event — the cursor-session object has no `stopped` of its
  own — but the `ext_image_copy_capture_session_v1` a client gets back from
  its `get_capture_session` is answered `stopped` immediately.

### GPU-rendering clients (`zwp_linux_dmabuf_v1`)

`zwp_linux_dmabuf_v1` is advertised (version 6), and dmabufs really are
imported — a GPU-rendering client works here under either renderer. Under
the default pixman renderer there is no GPU on the compositor side at all:
the client renders with the GPU and hands over a dma-buf, and scoot `mmap`s
it and composites it on the CPU, next to `wl_shm` clients in the same
session. No `LIBGL_ALWAYS_SOFTWARE=1` needed. Under `--renderer gles` the
buffer goes to the GPU driver instead, as a texture.

What is advertised is **what the renderer this session is actually running
can import**, because a format in the table that could not then be imported
would kill the client that believed it
(`zwp_linux_buffer_params_v1.create_immed` has no soft refusal). The table
is derived from that renderer rather than fixed, and pinned over the wire by
test:

- **pixman:** `Xrgb8888` then `Argb8888`, `LINEAR` only, single-plane only —
  the only layout a CPU mapping can read — minus either one pixman cannot
  import.
- **GLES (`--renderer gles`, on every backend, including the `--tty` GPU
  scanout tier):** every format and modifier the GPU driver says it
  imports. On real hardware that means GPU clients get their **native
  tiled/compressed layouts** instead of being forced into slow linear
  buffers, and video players can hand over **multi-plane YUV** (`NV12`,
  `P010`, three-plane `YUV420`, packed `YUYV`, …) straight from a decoder,
  composited through the driver's own YUV sampling. `Xrgb8888` and
  `Argb8888` are listed first, each at whatever layouts the driver names
  for it, so wherever the driver offers both at `LINEAR` the pixman table is
  the head of the GLES one — but not on a driver that lists them only as
  tiled, which then gets no `LINEAR` entry for them at all.

  Two things are deliberately left out of the GLES table. An **implicit
  modifier** (`DRM_FORMAT_MOD_INVALID`, "the driver's default layout") is
  never offered for a format the driver named explicit layouts for: a YUV
  buffer imported that way is sampled as if it were RGB and shows the wrong
  colours (measured, not assumed). And a format the driver lists without
  naming *any* layout — a display without
  `EGL_EXT_image_dma_buf_import_modifiers`, or a driver that refuses the
  modifier query for it — is offered only if it is `Xrgb8888`/`Argb8888`,
  at `LINEAR`, which such a driver is known to import; anything else there
  would be a guess.

A client too old for feedback (protocol version 1 or 2) is told only
fourccs, with no layout, and allocates implicitly; for a YUV format under
GLES that is the wrong-colours path below — a wrong picture rather than a
disconnect on the drivers measured (llvmpipe only; a driver that refuses
implicit YUV imports would refuse it instead).

A client that ignores the feedback and offers a layout the table never
named gets whatever the renderer says: `failed` on the asynchronous
`create`, which it survives, and a protocol error on `create_immed`, which
the protocol prescribes. (Under GLES a client that allocates an implicit
YUV buffer anyway is not killed — it just draws the wrong colours.)

On the dev VM's software GL (Mesa llvmpipe) the GLES table is 57 formats,
all at `LINEAR` — Mesa lists no other layout there. What it looks like on
real GPU hardware is recorded per machine, not promised here. The line to
look for is logged once at startup:
`dmabuf feedback: advertising the renderer's importable formats pairs=… fourccs=…`
(and the whole table at `RUST_LOG=scoot=debug`).

Two edges of that derivation are worth knowing before you debug one of
them. If the active renderer can import *nothing* this compositor can
vouch for — an EGL display with no dma-buf import capability at all —
**no dmabuf global is advertised**. That steers GL clients onto `wl_shm`
rather than killing them, but it is not free: they then render in
software, and a shell that waits for dmabuf feedback before capturing
(below) waits forever. scoot logs `reported no dma-buf format it can be
trusted to import` when it happens, and `--renderer pixman` is the working
session on such a machine. A compositor with no renderer at all advertises
nothing here for the same reason.

`main_device` names a **render node**: the active renderer's own, where it
can name one, else `/dev/dri/renderD128`, else `card0`, else `0`. The
renderer's own device leads because that is the device an import can
actually succeed against — on a two-GPU machine a client that allocated
against the other node would hand over a buffer this renderer cannot
import. Which rung answered is logged once at startup
(`dmabuf feedback main device device=… source=…`).

The global also gates screen capture for some shells: quickshell's buffer
manager instantiates no capture context at all — not even the `wl_shm` one
— until it has seen real dmabuf feedback, so without this advertisement
every quickshell `ScreencopyView` stays blank despite the capture protocol
working.

**This follows `--renderer`**, and no longer asks you to avoid one: the
table is the active renderer's, so `gles` advertises what the GPU driver
can import and pixman advertises what pixman can. See
[tty.md](tty.md#which-renderer-draws-the-frames).

**Limits.** Every plane `add`ed is one fd, and counts against the client's
[512 fds](#per-client-limits-on-what-scoot-keeps) until it really closes:
while it sits in a params object, while it is part of a buffer, and after
the client destroys that `wl_buffer` if a surface still shows it. A
four-plane buffer is four. Under a GLES renderer that keeps its own copy
of each imported plane (Mesa's software renderer, measured on the dev VM:
a three-plane `YU12` buffer costs scoot six fds) the copies count too,
from the `add`; scoot measures this once per session, on the first import
it can measure cleanly, and logs it
(`dmabuf: learned how many fds the renderer keeps of each imported
plane`). A dma-buf `wl_buffer` also counts against the same 512 live
buffers per client as every other buffer, and both `create` and
`create_immed` claim. Planes that have been `add`ed to a
`zwp_linux_buffer_params_v1` object which has not yet been turned into a
buffer are also bounded on their own: without that, a client could add
planes and never create anything; review measured 220 params objects x 4
planes holding 927 fds. A client may have **32** such planes at once,
across all its params objects. (Under fd pressure there is no separate
grace for them any more; the client's whole fd count is what the
[pressure limit](#per-client-limits-on-what-scoot-keeps) reads.) A plane
stops counting as pending when its params object is consumed by
`create`/`create_immed` (whatever the import's outcome) or destroyed, and
when the client disconnects. An `add` past the bound
disconnects the client with `wl_display.error` `no_memory`; the params
interface has no error for "too many". A client that adds one buffer's
planes (at most four) and creates it straight away, which is how the
protocol is meant to be used, stays far below; that is reasoned from the
protocol, not checked against each GPU client, since none makes a dma-buf
on the dev VM.

#### Per-surface feedback: the scanout tranche

The default feedback — the one every client gets, and what
`get_surface_feedback` answers for almost every surface — is one tranche
with no `scanout` flag. There is one exception, and it exists only on the
GPU scanout tier (`--tty --renderer gles`, `gpu-scanout` build): the
**fullscreen window covering an output** is sent per-surface feedback whose
first tranche is flagged `scanout`, names the display device as
`tranche_target_device`, and lists the layouts the output's primary plane
can scan out directly. The default tranche follows it unchanged. A client
that acts on it reallocates into one of those layouts, and its buffers can
then be shown straight from its own memory instead of being composited.

- **Nothing new is promised.** The scanout tranche is a subset of the
  default table — the entries the plane would accept — so every pair in it
  already imports; the format table and main tranche are the default's, entry
  for entry. A client that allocates from it and then gets composited anyway
  (a notification drawn over it, a capture stream) is imported like any
  other.
- **Who gets it.** Only the root surface of the window covering the output,
  and only while that output can go direct at all: unlocked, not being
  streamed by a capture client, nothing translucent, and the window's buffer
  the one the display would be offered: the window opaque over the whole
  output (an opaque-format buffer or an opaque region), or a black
  background with no wallpaper under it (a black single-pixel-buffer
  wallpaper counts as black background). Otherwise -- a transparent window
  over the default background, or over a wallpaper -- the display is never
  offered the window's buffer, and steering the client would cost it a
  reallocation for nothing (the rules in [tty.md](tty.md)). One known cost:
  the scanout feedback reaches a window only after it has redrawn at the
  fullscreen size (before that it does not span the output), so a client
  that acts on it reallocates its buffers twice on entering fullscreen --
  once for the size, once for the layout. Subsurfaces are not steered. A surface that first asks
  for feedback after its window went fullscreen gets the scanout feedback on
  that first answer.
- **When it changes.** Only on a change, never per frame. When the covering
  window changes (it leaves fullscreen, unmaps, another window or workspace
  takes the output) the old one gets the default feedback back at once. When
  the same window still covers the output but a lock, a capture stream or a
  translucent moment stops it going direct, it keeps the scanout feedback,
  and gets the default back on the first frame drawn two seconds or more
  later if it still cannot — so a shell refreshing a thumbnail, which counts
  as streaming for a second, never makes the game reallocate. (The check
  rides on drawn frames: a screen that draws nothing, like a still lock
  screen, keeps the scanout feedback until it next draws, which is harmless —
  every pair in it imports.) A notification or popup drawn over the window changes
  nothing: it is transient, and the window goes direct again the moment it is
  gone.
- **What the plane accepts.** Explicit tiled or compressed modifiers only
  where the plane names them (`IN_FORMATS`), `LINEAR` where the plane names
  it, or — on a plane that names no modifiers at all, like the dev VM's
  virtio-gpu — `LINEAR` for single-plane formats the plane lists. An
  alpha format counts as its opaque twin (the display ignores alpha on the
  bottom plane). Never an implicit modifier. A modifier the display's buffer
  manager has been seen to lose on import is dropped from the tranche and the
  window re-sent, because such a buffer can only ever composite.
- **Logged once per plane set:** `dmabuf feedback: scanout tranche for
  fullscreen windows pairs=… lost=… device=…`, or `the primary plane takes
  none of the advertised formats; no scanout tranche` when there is nothing to
  offer (then nothing is ever sent). On the dev VM it is `XR24` and `AR24` at
  `LINEAR`. What a real GPU's tranche holds, and whether a GL client there
  reallocates into it and goes direct, is `Asahi.md` Test 6 — not claimed
  here.

No other backend or tier sends per-surface feedback that differs from the
default.

## Explicit sync (`linux-drm-syncobj-v1`)

`wp_linux_drm_syncobj_manager_v1` lets a GPU client hand over a dma-buf
together with two points on DRM timeline syncobjs: an *acquire* point its
GPU signals when the buffer is finished, and a *release* point scoot signals
when it is done reading the buffer. NVIDIA's driver effectively requires
it, and Mesa's Vulkan WSI uses it where the compositor offers it and the
driver supports it; without it a GPU
client depends on implicit fencing, which not every driver provides.

**Where it is offered.** Only on the GPU scanout tier (`--tty --renderer
gles` in a `gpu-scanout` build), and only when a DRM device passes
Smithay's syncobj-eventfd probe (timeline syncobjs plus
`DRM_IOCTL_SYNCOBJ_EVENTFD`). The display device scoot drives is tried
first; if its driver has no syncobj support -- possible on a machine whose
display controller is not its GPU, like Apple Silicon -- the render nodes
(`/dev/dri/renderD*`, in name order) are tried instead (a syncobj works on
any DRM device that supports them, whichever GPU made it). The startup log says which:
`drm: explicit sync (wp_linux_drm_syncobj_manager_v1) offered device=…`,
or `drm: no device here has syncobj timeline eventfd support; explicit
sync … is not offered`. The dev VM's virtio-gpu passes on the display
device.

**Where it is not, and why.** Not under pixman (`--headless`, `--nested`,
the default dumb-buffer `--tty`), and not under `--renderer gles` on
`--headless`/`--nested`. Honouring the points needs a way to wait on an
acquire point without blocking the compositor and a render fence to wait
out before signalling a release point, and only the scanout tier has both
wired. A client that binds the global and then has its points ignored is
worse off than one that never saw it (it would scan out or composite
unfinished buffers), so everywhere else the global does not exist and the
client falls back to implicit sync, as it would on any compositor without
the protocol.

What scoot does with the points:

- **Acquire.** A commit whose acquire point has not signalled is held until
  it does. The surface keeps showing its previous buffer in the meantime,
  and later commits to the same surface queue behind it in order. Other
  surfaces, including the same client's, and every other client carry on:
  a client whose GPU never signals stalls only its own surface. A point
  already signalled when the commit arrives costs one query and nothing
  else. A surface destroyed while it waits, or a client that disconnects
  mid-wait, takes its waits with it at once rather than when (or if) the
  point signals. Waits keep running while the session is VT-switched away.
- **Release.** A buffer's release point is signalled when scoot is done
  reading it. For a composited frame, that means once the frame's GPU work
  has finished and it has flipped, not merely when the client commits its
  next buffer (a frame that will never be shown -- the session switched
  away, a refused commit -- lets its buffers go at once). For
  a fullscreen buffer scanned out directly, it means once the display has
  stopped scanning it out. A buffer replaced before it was ever shown, or
  whose surface is destroyed, is released straight away. Buffers committed
  without sync points keep the release timing they always had.
- **Malformed requests** (an acquire point without a release point, a
  release point not after the acquire point on the same timeline, points on
  a `wl_shm` buffer, a timeline fd that is not a syncobj) get the protocol's
  own errors, from Smithay.
- **Bounds.** A client may have scoot hold **128** of its imported
  timelines and have **64** commits waiting on acquire points at once (each
  is an eventfd). Timelines also count toward the client's
  [512 fds](#per-client-limits-on-what-scoot-keeps), with its pools and
  planes. While the compositor's fd table is nearly full an import is
  refused once the client's fds of every kind are past 128 (see the same
  section), and a wait once it has more than **16** waiting, so it can
  reach 17 waits. A timeline
  counts for as long as scoot holds its syncobj fd, which is not the same as
  for as long as the timeline object lives: a sync point set on a surface
  keeps its timeline's fd open after the object is destroyed (the protocol
  says destroying a timeline does not unset its points), until the point
  itself goes -- replaced, the surface destroyed, or its buffer released.
  Those count too. Before this, the bound counted objects, and review
  measured 440 surfaces with points on destroyed timelines holding 927 fds
  with nothing counted. An import that finds the client at 128 first checks
  which of them are really still open, so timelines a client destroyed and
  that nothing references cost it nothing however many it churns. The
  import is refused, with `invalid_timeline`, only if more than 112 are
  still open (fewer than 16 could be reclaimed); below that, the check buys
  the next 16 imports without another one. Under fd pressure the same kind
  of check decides, on the client's whole fd count. Past the wait bound the client is disconnected
  with `wl_display.error` `no_memory`. A real client stays far below both:
  Mesa's Vulkan WSI imports two timelines per swapchain image, and a
  swapchain cannot run more than its image count ahead.

**A leak fixed by forking Smithay.** Upstream Smithay, at the revision scoot
pinned and on its `master`, never destroys the kernel handle an
`import_timeline` creates. So each import would leave one syncobj handle on
scoot's DRM file until scoot exits, about 80 bytes of kernel memory each,
together with any wait scoot abandoned on that syncobj when a surface was
destroyed mid-wait (about 200 bytes each). A hostile client could drive
that at wire speed (review measured about 24 MB/s), and no per-client bound
stops it, because every iteration ends with nothing live. scoot therefore
builds against a scoot-sh fork of Smithay that adds the one missing `Drop`.
Measured with it, both loops stay flat. Details are in
[`backlog/resolved/syncobj-handle-leak-done.md`](backlog/resolved/syncobj-handle-leak-done.md).
Returning to upstream once it has the fix is
[`backlog/core/smithay-fork-repin.md`](backlog/core/smithay-fork-repin.md).

**What has been verified.** On the dev VM's GPU tier (virtio-gpu), with a
test client that renders into card0 dumb buffers and signals its own syncobj
timelines standing in for a GPU: a held commit shown only after its signal,
also across a VT switch; a client that never signals stalling only itself,
while another client's frame pacing and IPC latency stay unchanged; the fds
of a client killed mid-wait all returned; both bounds disconnecting only
the offender; a fullscreen explicit client going direct on every frame, its
replaced buffers released when the replacement's flip completed; no change
in compositor CPU. No real explicit-sync client has been run against it:
on the dev VM, Mesa's `vkcube` (lavapipe) and `es2gears_wayland` (llvmpipe)
both draw through `wl_shm` and never bind the global. The real-GPU check is
[`Asahi.md`](../Asahi.md)'s Test 7. `--nested` was not run live; the global
cannot appear there, since the only code that offers it runs in `--tty`'s
startup.

## Screen locking (`ext-session-lock-v1`)

Version 1, so a real locker (`swaylock` 1.7+, `gtklock`, `hyprlock`,
`waylock`) can lock the session with the compositor enforcing it, rather than
a layer surface asking politely. The global is
`ext_session_lock_manager_v1`.

**What is guaranteed while the session is locked:**

- **Nothing but the lock client's own surfaces is drawn.** Not "drawn behind
  an opaque backdrop" — windows, layer surfaces (on every layer including
  `overlay`) and the focus ring are not gathered into the frame at all. The
  screen is the lock surface, an opaque backdrop where it doesn't cover, and
  the pointer cursor. A `scoot msg screenshot` reads that same framebuffer
  (with the pointer drawn in or left out as the request asks).
- **Only the lock surface receives input.** Keyboard focus moves to it (or to
  nobody, if the client hasn't created one yet) the instant the lock request
  arrives, and pointer focus is moved with it, so a click can't land in the
  window that happened to be under the pointer. Pointer focus is re-derived
  again on the commit that maps the lock surface, so the first click lands on
  it without the mouse having to move first. Any grab in flight is dropped as
  well — not only when the lock is taken, but at every transition that
  changes which lock surfaces count (a lock surface destroyed, a lock given
  up, a locker that died), because a grab outlives focus changes by design.
  That covers an open popup menu, whose grab holds the *keyboard* and would
  otherwise receive the password.
- **"The lock surface" means the *current* lock's.** A lock object can be
  destroyed while the client that made it stays connected and keeps the
  `wl_surface` underneath alive, so "is this surface alive" is not the same
  question as "does this surface still belong to the lock that owns the
  session". A surface from a lock that was given up, or replaced, stops being
  drawn and stops receiving input immediately.
- **A destroyed lock surface falls back to the backdrop straight away** — the
  protocol's own rule, and "straight away" means without waiting for anything
  else to change on screen, including when the client destroys only the
  `ext_session_lock_surface_v1` and keeps the `wl_surface` under it alive.
- **Keybindings that run an action don't fire.** `Super+Q`, a `spawn` bind,
  every layout motion: suppressed, and forwarded to the lock client as
  ordinary keystrokes instead. The one exception is the `--tty`
  `Ctrl+Alt+F1`..`F12` VT switches. That is a session-level escape hatch, not
  a way in — the VT it switches to has its own login, and this session stays
  locked behind it.
- **`scoot msg action ...` is refused**, with an error saying why. So is an
  `ext-workspace-v1` client's `activate`.
- **Ordinary clients stop drawing.** They get no frame callbacks while
  locked, which is what the protocol asks for and also what keeps them from
  burning CPU behind a lock screen.

**If the lock client dies, the session stays locked.** That is the protocol's
rule and the point of it. The recovery story:

- the screen turns **solid red**, so you can tell "my locker crashed" from
  "my locker is showing a black screen" — including when the locker had a
  surface up and drawing at the moment it went;
- **run a lock client again and it takes over** — it puts its own surface up
  and can unlock once you authenticate. It is told `locked` immediately when
  the lock it replaces had already blanked the screen; if that lock went in
  the window *before* its first blanked frame, the replacement waits for one
  exactly like a fresh lock does, because until then your actual desktop is
  still what's on the display.

The same applies to a lock client that gives up without dying: destroying an
`ext_session_lock_v1` before `locked` arrives is legal (only
`unlock_and_destroy` is forbidden that early), and a locker that times out
waiting may well do it. The session stays locked and reads as abandoned, and
the surfaces that client had up stop being drawn and stop receiving input at
that moment, even though its connection and its `wl_surface`s are still
alive.

**What is *not* guaranteed — read this before trusting it:**

- **A same-uid process is inside the boundary, and always was.** Anything
  that can reach scoot's Wayland socket can take over a lock whose client has
  died and then unlock the session; anything that can reach its IPC socket
  can screenshot the lock screen and inject keystrokes into it (that is how
  an agent drives a lock screen). Both sockets are owner-only. This lock
  keeps *someone at the keyboard* out, not a process already running as you.
- **`scoot msg windows` still lists your windows while locked**, titles
  included, and `scoot msg outputs` still answers. Nothing is drawn from
  them, but the IPC surface is not blanked.
- **So do both foreign-toplevel protocols**: handles stay, titles keep
  updating, and a window opened behind the lock screen is still announced.
  Sending `closed` for windows that did not close would be a lie a taskbar
  could not recover from, since the `ext-` protocol forbids reusing their
  identifiers afterwards. The wlr protocol's two *requests* are refused.
- **The `locked` event is sent once a blanked frame is confirmed on screen,
  not merely rendered.** Under `--headless`/`--nested` the render is the
  confirmation. Under `--tty` the render hands the frame to the presenter and
  `locked` waits for the vblank confirming the page flip that carries it, so
  the previous, possibly unlocked, frame can no longer outstay the event by a
  vblank. That costs up to one vblank of lock latency under contention. If no
  vblank can arrive at all — you switched VT away, a modeset discarded the
  flip — the lock is confirmed anyway after one second (logged as a warning)
  rather than hanging the locker forever.
- **Up to one frame of the unlocked screen can still be on the display**
  between the lock request and the first blanked frame. That is inherent —
  the protocol's `locked` ordering exists precisely because of it. Input is
  already captured during that frame, so nothing typed in that window reaches
  the unlocked session.
- **scoot blanks immediately rather than waiting for the lock client to
  draw.** Some compositors wait up to a second for lock surfaces so the
  transition doesn't flash black; waiting means rendering the unlocked
  session for that whole second.
- **One lock surface per output.** A lock surface is configured to the size
  of the `wl_output` it names, and each output's frame shows that output's
  own surface over the opaque backdrop -- never another output's. The
  keyboard goes to the surface on the output under the pointer (falling back
  to the first surface when the pointer is over no output, or its output has
  none), and pointer input is hit-tested per output the same way, so exactly
  one surface holds each at a time. `locked` is sent only once *every*
  output has shown its blanked frame: an output with no surface counts on
  its backdrop frame, but (with more than one output) one whose surface is
  admitted and not yet drawn holds the confirmation open, because its screen
  is still showing a placeholder rather than the locker's blank. If that
  surface never draws, `locked` never fires — that wedge is the secure
  default (confirming would show an unblanked screen as locked); kill the
  locker and the abandoned path takes every screen red. A surface admitted after the
  lock already confirmed is sized and shown with no second confirmation. A
  second `get_lock_surface` for an already-covered output is refused with
  the protocol's `duplicate_output` error even when it names the output
  through a different `wl_output` bind (destroying the first surface frees
  the output for a rebuild).

## Idle detection

`ext_idle_notifier_v1` (version 2), so a `swayidle`-style daemon can learn
the seat has been quiet N milliseconds and dim the screen, lock it or suspend
the machine. Every input source resets the timers: real devices under
`--tty`, host-forwarded input under `--nested`, and IPC-injected input
(`scoot msg type`/`key`/`pointer` count as a user at the machine, which is
what keeps an agent's own activity from looking like idleness). What does
*not* reset them is the compositor re-running its own hit test on a lock
transition — that is not input.

`zwp_idle_inhibit_manager_v1` (version 1) is the reverse: a video player or
presentation app creates an inhibitor on one of its surfaces and `idled`
holds off until the inhibitor is gone — destroyed explicitly, or released
implicitly when the surface dies or the client disconnects. The
input-specific watch (`get_input_idle_notification`, for daemons with their
own inhibit policy) ignores inhibitors by design.

Two things to know: there is no built-in auto-locker — the timeouts and
commands are the daemon's config, not scoot's, the swayidle way — and an
inhibitor counts while its surface is *alive*, whether or not it is visible.

## Clipboard and primary selection

All three selection globals, on every backend:

- **`zwlr_data_control_manager_v1`** (version 2): clipboard managers
  (`cliphist`, `clipman`). Set the clipboard without needing focus, read
  anything any client copies.
- **`ext_data_control_manager_v1`** (version 1): the successor protocol. Both
  generations are exposed side by side, as current compositors do, so a
  manager speaks whichever one it was written for.
- **`zwp_primary_selection_device_manager_v1`** (version 1): middle-click
  paste. Unlike the clipboard, this one is focus-gated — the compositor only
  accepts a `set_selection` from the client holding the keyboard, and only
  offers the selection to devices whose client holds it. A background client
  setting the primary selection is silently denied, not queued.

## Night light (`wlr-gamma-control-v1`)

`zwlr_gamma_control_manager_v1` (version 1), so `gammastep` and `wlsunset`
work, on every backend. One control per output: a second `get_gamma_control`
on the same output transfers control, the old control gets `failed` and stops
affecting anything, while a control on any other output is untouched — and
destroying a live control (or disconnecting with one held) restores the
default linear ramp.

- **Under `--tty`**, the ramp is pushed to the CRTC gamma LUT, so the screen
  really warms. The advertised `gamma_size` is the CRTC's own (256 on the
  hardware measured so far), re-read whenever a hotplug moves the session to
  a different CRTC; a live control hears `failed` over that move either way,
  so it re-reads `gamma_size` and re-pushes (a modeset moves planes, not LUT
  contents). Anything the DRM device refuses retires the control with
  `failed` and the session keeps running.
- **Under `--headless`/`--nested`** there is no hardware LUT, so the ramp is
  accepted but changes nothing on screen — and a `scoot msg screenshot` reads
  the framebuffer, which is pre-LUT, so captures show the unmodified frame
  either way. `gamma_size` is 256 there.

A `set_gamma` fd must hold exactly three ramps of `gamma_size` little-endian
`u16` entries (red, green, blue); anything else — short, long, empty,
unreadable — is an `invalid_gamma` protocol error. Gamma control keeps
working while the session is locked: it changes no pixel's content, only the
output's color temperature.

## Cursor shapes (`wp-cursor-shape-v1`)

`wp_cursor_shape_manager_v1` (version 2), so a client can *name* the cursor
it wants — `text`, `ew-resize`, `not-allowed` — instead of loading an xcursor
theme and uploading a surface of its own. Modern GTK4/Qt6 toolkits and `foot`
prefer this when it exists; without it `foot` logs "compositor does not
implement server-side cursors".

A named shape is answered from **the cursor theme already installed on the
machine**: scoot resolves `[appearance] cursor_theme`, else `$XCURSOR_THEME`,
else `default`, and draws that theme's own artwork — the same pixels the
client would have loaded for itself. Parsing is the MIT-licensed `xcursor`
crate; nothing is vendored, and scoot ships no theme of its own.

**When no theme is installed** — a linuxserver webtop or any minimal
container — named shapes fall back to ten shapes scoot draws procedurally
itself. The mapping collapses the names a user cannot tell apart at 16
pixels:

| Drawn as | Named shapes it answers |
| --- | --- |
| arrow | `default`, plus every name with no row of its own — `help`, `wait`, `progress`, `pointer`, `zoom-in`, … |
| I-beam | `text` |
| sideways I-beam | `vertical-text` |
| crosshair | `crosshair`, `cell` |
| vertical double arrow | `n-resize`, `s-resize`, `ns-resize`, `row-resize` |
| horizontal double arrow | `e-resize`, `w-resize`, `ew-resize`, `col-resize` |
| diagonal double arrow | `ne-resize`, `sw-resize`, `nesw-resize` / `nw-resize`, `se-resize`, `nwse-resize` |
| four-way arrow | `move`, `all-scroll`, `all-resize`, `grab`, `grabbing` |
| circle with a slash | `not-allowed`, `no-drop` |

The drawn shapes use the `[appearance]` `cursor_size`/`cursor_color` settings
and are built once at startup. Theme images are loaded when a client first
asks for that shape and cached from then on, including a negative cache so a
theme missing `zoom-in` is not re-searched on every hover.

A client that uploads its own cursor *surface* still gets its own pixels
drawn. scoot also exports `XCURSOR_THEME`/`XCURSOR_SIZE` to everything it
spawns, so a client that loads a theme itself (GTK3, and anything predating
this protocol) picks the same one the compositor draws.

Cursors are only drawn under `--tty`; `--headless` has no display and
`--nested` shows the host compositor's own cursor.

## Focus handoff (`xdg-activation-v1`)

`xdg_activation_v1` (version 1), so a launcher can hand focus to the app it
started and a notification daemon can focus the app its popup came from.

Honoring every such request unconditionally would be a focus-stealing
primitive, so scoot checks the token three ways:

- **The token must name a real, recent input event that went to the client
  asking.** `set_serial(serial, seat)` is how a client says which click or
  keypress caused it to ask, and a token that names none of them — no serial
  at all, a seat scoot doesn't own, or a stale or made-up number — is refused
  when it is created. What counts is a serial scoot issued for a key or
  button event (press *or* release; pointer motion never counts, and neither
  does a *focus* event, which every newly mapped window gets for free) within
  the last few input events **and the last 10 seconds**, **and that was
  delivered to that same client**. Both bounds are needed: an idle session —
  an agent driving scoot over IPC makes one, since injected actions are not
  input events — never rotates the event history, so without the clock a
  click from this morning would still be spendable tonight. Wayland serials
  come from one process-wide counter shared with non-input events, so a
  number alone is cheap to observe and guess; pairing it with who actually
  received the event is what makes this a check on interaction.
- **A token is valid for 30 seconds.** Long enough for a cold-starting app to
  redeem the token its launcher gave it; short enough that a token is still a
  receipt for something the user just did.
- **At most 64 unredeemed tokens exist at once**, across all clients, with
  expired ones swept first. `get_activation_token` is unauthenticated and
  unlimited, and nothing upstream prunes what it hands out.

The serial is checked when the token is *created*, never when it is redeemed:
a launcher hands its token to a process that may take seconds to start, by
which point plenty of newer input has happened, and re-checking then would
break the one case this protocol exists for.

An app scoot itself started — a keybinding or `msg action spawn` — gets its
token a different way: the compositor mints one and hands it to the child in
`$XDG_ACTIVATION_TOKEN`, so the child can activate its own window when it
maps one. That covers the slow cold start, where focus has moved elsewhere
before the window appears. The token lives under the same two bounds; a spawn
past a full table simply gets no token, and the window is still focused on
map.

What a refused token costs depends on what was being activated. For a **fresh
spawn** it is invisible: the app maps a window, and mapping focuses it. For
**something already running** — a single-instance app (Firefox, Chromium,
anything on `GApplication`) re-invoked from a launcher, or the notification
daemon case — nothing maps, so nothing else focuses it and the activation
simply does not happen. Worth knowing because of the one case scoot refuses
that the protocol would allow: a launcher that mints its token from a *focus*
serial rather than a key or button one (fuzzel does this when an entry is
picked with the mouse) gets no activation.

A redeemed token is removed whether or not it was honored, so one user action
cannot be replayed into focus later. Activation goes through the same action
path a keybinding does, so it is refused while the session is locked and it
scrolls the activated column into view rather than only marking it focused. A
refused activation does nothing visible: scoot has no per-window urgency
state to raise instead.

## Window icons (`xdg-toplevel-icon-v1`)

`xdg_toplevel_icon_manager_v1` (version 1), so a client can say which icon
belongs to its window. scoot draws no icons itself — it has no titlebars,
taskbar or window switcher — so this exists for the two consumers outside it:
a bar or dock showing a window list, and an agent driving the session, which
reads the name off `scoot msg windows`' `icon` field.

Two limits: no *preferred icon sizes* are advertised, since nothing in scoot
draws an icon and so it has no size to prefer; and a client that supplies raw
pixel buffers instead of a name reads as having no icon, since handing those
over IPC would mean re-encoding shm buffers to PNG per query.

The buffer half, for the toolkit author: pixel buffers must be square and
`wl_shm`-backed (anything else is refused with `invalid_buffer`), and they
are ordinary live `wl_buffer`s under the 512-per-client bound, so a client
already at its budget is refused further creations. There is no `release` for
icon buffers — the protocol leaves the event unused — and a buffer destroyed
while its icon still lives disconnects that client (`no_buffer`); destroying
the icon first makes destroying its buffers safe. Pixels never leave the
compositor: neither foreign-toplevel protocol has an icon event, and IPC
carries the name only.

## Input methods (`text-input-v3`, `input-method-v2`)

`zwp_text_input_manager_v3` (version 1) and `zwp_input_method_manager_v2`
(version 1) — the two halves of IME support, neither of which is useful
alone. An application binds the first to say "there is a text field here"; an
input method (fcitx5, ibus, an on-screen keyboard) binds the second to
compose into it. Without the first, `foot` logs "text input interface not
implemented by compositor; IME will be disabled".

Which text field is focused follows keyboard focus automatically, so it works
for a layer-shell surface with a search field as well as for an ordinary
window. What scoot owns is the input method's **popup** — the candidate
window beside the text cursor — which is tracked against whichever surface
has the field and drawn with that surface's own popups, so it follows the
window, gets frame callbacks, and disappears when the field is disabled. That
includes a lock screen's password field: while the session is locked the
candidate window is drawn over the lock screen at the caret, and only that
popup is — background windows' popups stay hidden and callback-starved until
unlock.

An input method is more privileged than a clipboard manager — it can grab the
keyboard and inject text into the focused client — and there is still no
client filter, for the trust-model reason at the top of this page. That grab
outranks an `xdg_popup.grab` in both orders, so no context menu opens in a
text field while an IME is active there.

## Output scaling

A HiDPI panel needs the compositor to tell clients to render at a scale
greater than 1, or everything comes out physically tiny. `[output] scale`
sets that scale; see [configuration.md](configuration.md#output). It is
advertised three ways, matching what clients actually support:

- **`wl_output.scale`** — the integer `ceil(scale)`. Every client that binds
  an output gets it automatically (re-sent on bind and whenever the output's
  state changes). A scale of `1.5` is advertised as `2` here, which is what a
  client that only understands integer scaling should draw at.
- **`wp_fractional_scale_v1`** — the exact fractional value. A client that
  creates a `wp_fractional_scale_v1` for one of its surfaces is sent
  `preferred_scale` (`1.5`, not `2`), and can render a larger buffer and let
  the compositor scale it down. The `wp_viewporter` global is advertised
  alongside it, because that is the protocol a client uses to submit such a
  buffer — without it, a fractional client has no way to render.
- **`wl_surface.preferred_buffer_scale`** (needs client `wl_compositor` v6) —
  the integer preference that accompanies the fractional value, sent with the
  default `preferred_buffer_transform` (`normal`). It is a separate event on
  a separate object, so a client that opts into fractional scaling receives
  **both**: the exact `1.5` *and* the integer `2`. This is protocol
  completeness (it is what wlroots sends). A client below `wl_compositor` v6
  is not sent the event and keeps the implicit default of 1.

Real limits rather than polish:

- **Session-wide.** `scootctl reload` re-applies the scale live
  (re-advertised on `wl_output`, re-sent to every live surface, geometry
  recomputed), and there is no per-output setting.
  Changing it per output means waiting on per-output configuration.
- **`--nested` is scale-1 only.** The host compositor owns the scale of the
  window scoot is drawn inside, so a non-1.0 `scale` there would double-count
  it; scoot logs a warning and uses `1.0`.
- **Screenshots are physical pixels; layout coordinates are logical** — see
  [ipc.md](ipc.md#rules-an-agent-needs).

## Single-pixel buffers

`wp_single_pixel_buffer_manager_v1` (version 1), so a client can mint a solid
-color 1x1 buffer straight from four `u32` channels instead of allocating an
shm pool for a single pixel. The buffer is what the channels say (the full
`uint` range is valid per channel, read as a percentage), reports 1x1, and
renders as a solid fill; a client that wants it bigger scales it through
`wp_viewporter` rather than by uploading a larger buffer.

- **No shm, no pool budget — but inside the buffer bound.** These buffers
  allocate nothing, so the per-client `wl_shm` pool count and fd count never
  move for them. They still count against the 512-live-`wl_buffer` bound (uniform
  accounting — the hook can't observe buffer kind, and excluding them would
  let cheap destroys drain retaining units).
- **Destroying the manager leaves its buffers working.** The spec says the
  child objects are unaffected, and they are.
- **Destroying an attached buffer is legal and safe.** Wayland lets a client
  destroy a `wl_buffer` its surface still names; nothing panics and nobody is
  disconnected for it.

## Relative pointer and pointer constraints

`zwp_relative_pointer_manager_v1` (version 1) with
`zwp_pointer_constraints_v1` (version 1), the pair games and 3D apps expect:
the client locks or confines the pointer to its surface and reads raw
relative motion deltas off its relative-pointer object.

- **Relative events are gated on pointer focus, not on the lock.** A client
  whose surface has pointer focus receives `relative_motion` whether or not
  it locked; a client without focus receives nothing. That is the protocol's
  own rule.
- **Unaccelerated means pre-libinput-acceleration on `--tty`.** A `--tty`
  mouse reports both an accelerated and a raw device delta, and the relative
  event carries each as its own (`dx`/`dy` vs `dx_unaccel`/`dy_unaccel`).
  Every absolute source (IPC injection, `--nested` host motion, tablets)
  applies no acceleration of its own, so both pairs carry the same position
  change there.
- **Relative deltas are unclipped.** Motion stopped by the output edge, a
  lock, or a confinement still reports the full vector; only the absolute
  position stops.
- **A lock holds the absolute position; a confinement clamps it.** While a
  lock taken on the focused surface is active, the cursor does not move (the
  relative stream keeps flowing). A confinement keeps the pointer on its
  surface, clamped per axis to its region. A lock taken while unfocused stays
  inactive. Destroying a persistent lock or confinement is silent (no
  `unlocked`/`unconfined` event) and frees the pointer immediately.
- **A session lock deactivates a held lock or confinement.** Locking sends
  `unlocked`/`unconfined` to the holding client and moves pointer focus to
  the lock surface, so no pointer input reaches the game while locked. The
  persistent entry stays registered: unlocking returns focus to the game
  surface and re-arms it there with no new request, and the relative stream
  resumes with absolute still held.

An agent driving the pointer during a game or 3D session sees `ok` replies
with a frozen cursor — see [ipc.md](ipc.md#rules-an-agent-needs).

## Drawing tablets (`tablet-v2`)

`zwp_tablet_manager_v2` version 1 — the most Smithay carries at the pinned
revision; the protocol's own version 2 only adds a tablet bustype event and
pad dials, neither of which scoot mints. libinput tool events reach
tablet-aware clients (Krita, Xournal++) on `--tty` hardware, and every other
client gets a pen that moves the cursor and clicks.

- **A pen moves the cursor and clicks; there is no second focus system.**
  Tool proximity and motion run the same path mouse motion does. A tip tap is
  a left click through the same click path: it focuses the window, mints
  activation like a click, and dismisses a menu tapped outside of.
- **Pressure, tilt, rotation, slider and wheel ride the tool's axis events**,
  announced with proximity and updated by motion. Only changed axes are sent:
  a hovering pen restates no pressure.
- **Stylus barrel buttons are tool-only.** The tool sees the exact button
  number; nothing is synthesized onto the pointer, because no mapping from a
  stylus button onto a mouse button exists to honour.
- **Pads, strips, rings and dials are not supported.** Smithay carries no pad
  objects at the pinned revision, so `pad_added` never fires.
- **A tool cursor is the cursor.** A shape or surface a client names for its
  tool lands in the same cursor the pointer uses.
- **A tap on the lock screen reaches the locker**, through both the tool and
  the pointer halves, and moves nothing behind it.

## Presentation-time feedback (`wp_presentation`)

Version 2: a client requests feedback on its surface and learns, per content
update, either exactly when that update reached the screen (`presented`, with
a `CLOCK_MONOTONIC` timestamp, the output's refresh, a frame sequence and
flags) or that the update was superseded before it ever got there
(`discarded`).

- **The timestamp is the frame handoff, and each backend hands off somewhere
  else.** With no presenter (`--headless` under IPC-only control) the
  framebuffer is the final image, so the timestamp is when the frame finished
  rendering. Under `--nested` it is when the frame was committed to the host
  compositor. Under `--tty` it is when the page flip was issued to DRM, up to
  one vblank before the photons — and the `vsync` flag is set there, because
  the flip is vblank-synchronized; the other backends report no flags.
- **`zero_copy` means the buffer went to the display directly.** Only on the
  GPU scanout tier (`--tty --renderer gles`, `gpu-scanout` build), and only
  for the surface whose buffer was scanned out on the primary plane that
  frame — the covering fullscreen window, on a frame that
  [went direct](#per-surface-feedback-the-scanout-tranche). Every composited
  frame, and every other surface, is reported without it. A client cursor
  carried by a hardware cursor or overlay plane is not reported `zero_copy`:
  the cursor plane is a copy, and an overlay one is simply not counted
  (under-reported, never misreported).
- **`refresh` is the mode scoot advertises, not the panel's.** Always 60 Hz,
  including under `--tty` on a faster panel. A client pacing frames should
  trust the timestamps, not `refresh` plus arithmetic.
- **`seq` is zero except on `--tty`.** Headless has no vertical retrace to
  count and nested output is self-refreshing with no queryable count, so the
  protocol requires zero there; `--tty` reports the issued flip's number (a
  per-flip counter, not the kernel's refresh count).
- **Only displayed surfaces are stamped.** Mapped windows, layer surfaces,
  popups and the client cursor get feedback for a frame that showed them;
  while locked, only the lock surfaces do. Anything else keeps its feedback
  queued until it is shown or superseded.
- **A rendered-but-dropped frame stamps nothing.** A flip skipped for a busy
  CRTC, or a host commit dropped for lack of a free buffer, leaves pending
  feedback for the next presented frame rather than stamping a time nothing
  was shown at.

## Rendering hints

`wp_alpha_modifier_v1` (version 1) and `wp_content_type_manager_v1` (version
1). One of them works, and the other is stored and honestly ignored.

- **Alpha does what it says.** A client names a `u32` multiplier on its
  surface (`0` transparent, `u32::MAX` opaque) and the compositor blends it —
  windows, layer surfaces, lock surfaces and client cursor surfaces alike,
  all through the same render path. Destroying the modifier object is
  `set_multiplier(u32::MAX)` on the next commit, and destroying the manager
  leaves existing modifier objects working.
- **Content type is accepted and has no effect.** A client can label a
  surface `photo`, `video`, `game` or `none`; the compositor stores the label
  and changes no pixel for it — a CPU renderer with no adaptive-sync story
  has no consumer for the hint.
- **Neither touches any bound.** No pool, buffer or fd is created anywhere on
  either path.
