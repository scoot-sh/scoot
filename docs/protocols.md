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
| `xdg-shell` | 7 | Windows and popups. Every `xdg_toplevel` is a column entry. |
| `xdg-decoration-v1` | 1 | `zxdg_decoration_manager_v1` — server-side decorations, so a client stops drawing its own titlebar; see [`prefer_no_csd`](configuration.md#appearance). scoot draws a focus ring, never a titlebar. |
| `wlr-layer-shell-v1` | 5 | [Bars, docks, wallpapers, launchers](#layer-shell-bars-wallpapers-launchers). |
| `ext-workspace-v1` | 1 | [Workspaces](#workspaces-ext-workspace-v1). |
| `ext-foreign-toplevel-list-v1` | 1 | [Window lists](#window-lists-two-protocols), enumeration only. |
| `wlr-foreign-toplevel-management-v1` | 3 | [Window lists](#window-lists-two-protocols), with `activate`/`close`. |
| `wlr-output-management-v1` | 4 | [Display information](#display-information-wlr-output-management-v1) — read-only. |
| `ext-image-copy-capture-v1` | 1 | [Screen capture](#screen-capture-ext-image-copy-capture-v1), output only. |
| `ext-image-capture-source-v1` | 1 | Output sources only; no toplevel source manager. |
| `zwp_linux_dmabuf_v1` | 6 | [Real dmabuf import](#screen-capture-ext-image-copy-capture-v1), `LINEAR` single-plane. |
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
| `wp-presentation-time` | 2 | [Presentation feedback](#presentation-time-feedback-wp_presentation). |
| `wp-alpha-modifier-v1` | 1 | [Whole-surface opacity](#rendering-hints). |
| `wp-content-type-v1` | 1 | Accepted, [no effect](#rendering-hints). |

**Not implemented:**

- **XWayland.** X11 clients do not run. There is no XWayland integration in
  the tree at all — not ruled out, just not written.

The rest of this list *is* deliberate:

- **`wlr-screencopy-v1`.** The clients that motivated capture already speak
  the `ext-` protocol: `grim` 1.5.0 carries `ext_image_copy_capture_v1` and
  nothing else, and stock quickshell 0.3.1 — the build both DMS and Noctalia
  run on — carries the `ext-` manager and both `ext-` source managers.
- **`ext_foreign_toplevel_image_capture_source_manager_v1`.** A single window
  cannot be captured on its own; the global is not advertised, so a client
  takes its fallback path immediately instead of discovering a refusal at
  runtime.
- **Maximized, minimized and fullscreen window states.** scoot has no concept
  of any of them, so the state bits are never sent and the matching requests
  do nothing — a taskbar's minimise button is inert rather than lying.

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

### Keyboard focus

Following the protocol's `keyboard_interactivity`:

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
  (`zwlr_layer_surface_v1.get_popup`), not just to a window.
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

## Workspaces (`ext-workspace-v1`)

Version 1, the compositor-agnostic successor to the one-off wlr workspace
protocols, so a panel's workspace module, a workspace switcher or an
indicator can list workspaces, follow which one is active and switch between
them. The global is `ext_workspace_manager_v1`.

- **One workspace group**, carrying scoot's single output. A client that
  binds `wl_output` after the manager still gets an `output_enter` for it, so
  registry order doesn't matter.
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
  rather than by the user, and there is only one group.
- **`activate` is a request, not a guarantee** (as the protocol says): one
  naming a workspace that vanished between the client reading the list and
  the `commit` arriving is dropped, and one for the workspace that is already
  active does nothing.
- **An agent switches by number too.** `scoot msg action
  focus-workspace-index N` drives the same core action `activate` does.
- **Multiple outputs will change the shape of this** — a group per output is
  what the protocol is built for — but scoot has exactly one output today.
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
- **`state`, carrying `activated` and nothing else** — scoot's real window
  focus, the same one `scoot msg windows` reports as `focused`.
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
- **`set_maximized`, `unset_maximized`, `set_minimized`, `unset_minimized`,
  `set_fullscreen`, `unset_fullscreen` and `set_rectangle` are accepted and
  do nothing.** `set_rectangle` is a minimise-animation hint scoot reads
  nothing from; unlike wlroots, an invalid rectangle is ignored rather than
  answered with a protocol error, because disconnecting a shell over a number
  nothing looks at would be worse.

Worth knowing before you write against it:

- **`activate` needs a seat, and a headless shell may not have one.**
  quickshell sources the `wl_seat` argument from Qt's last input device, so a
  `ShellRoot` with no window and no input event sends nothing at all when you
  call `activate()` — no request reaches the compositor. With a real
  `PanelWindow` and a real click it works. Test it with a window.
- **`output_leave` is never sent.** scoot has one output, a window is on it
  for its whole life, and switching workspaces does not move it — the same
  answer wlroots-based compositors give.
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

**It is read-only. `apply` and `test` always answer `failed`.** scoot has
exactly one output, whose mode, position, scale and transform are fixed for
the life of the process, so there is nothing a configuration could change; a
configuration that reported `succeeded` and changed nothing would give you a
Display page whose buttons appear to work. `wlr-randr --output <name> --pos
100,100` prints `failed to apply configuration` and exits non-zero.

- **One `zwlr_output_head_v1`**, carrying its `name` (the same one
  `wl_output` reports — a DRM connector name like `HDMI-A-1` under `--tty`,
  `headless` otherwise), `description`, `make`, `model`, `enabled`,
  `position`, `transform`, `scale` and `adaptive_sync` (always `disabled`;
  scoot has no VRR support).
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
  than replacing the old one, matching what `wl_output` does. Under
  `--nested` that happens at most once (only the host's *initial* configure,
  if it proposes a size other than `--width`/`--height`). Under `--tty` it
  happens once per mode the display actually changes to, so a VM window moved
  between a 2x and a 1x screen a few times leaves a mode for each distinct
  size. Bounded by the number of distinct sizes the connector has offered,
  not by how many hotplug events arrive.
- **A `--tty` VT switch changes nothing by itself.** The output does not go
  away when you switch to another VT, it just stops being drawn, so the head
  stays enabled with the same mode and no `done` is sent. The one thing a
  switch *back* can produce is a mode change, and the switch is not what
  caused it: a display plugged in or resized while scoot was on another VT
  could not be acted on then, so the switch back re-probes and applies
  whatever moved.
- **Multiple outputs will change the shape of this**, but scoot has exactly
  one output today.

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
  a CPU renderer. `Xrgb8888` is offered first: if `[appearance]
  background_color` has an alpha below 1.0 then the framebuffer really is
  translucent, and an `Xrgb8888` capture forces the fourth byte opaque so you
  get a screenshot rather than a translucent image. With the default opaque
  background that pass is skipped — same bytes either way. An `Argb8888`
  capture hands you the framebuffer's own alpha.
- **`zwp_linux_dmabuf_v1` is advertised (version 6), and dmabufs really are
  imported — a GPU-rendering client works here, with no GPU on the compositor
  side.** The client renders with the GPU and hands over a dma-buf; scoot
  `mmap`s it and composites it with pixman, on the CPU, next to `wl_shm`
  clients in the same session. No `LIBGL_ALWAYS_SOFTWARE=1` needed.

  What is advertised: `Xrgb8888` then `Argb8888`, `LINEAR` only, single-plane
  only — exactly what the pixman renderer can map. A format in the table that
  could not then be imported would kill the client that believed it
  (`zwp_linux_buffer_params_v1.create_immed` has no soft refusal), so the
  table is pinned to the renderer's own importable set by test. A client that
  ignores the feedback and offers a multi-plane or non-`LINEAR` buffer is
  refused: `failed` on the asynchronous `create`, which it survives, and a
  protocol error on `create_immed`, which the protocol prescribes.
  `main_device` names this machine's **render node**
  (`/dev/dri/renderD128`, else `card0`, else `0`).

  The global also gates screen capture for some shells: quickshell's buffer
  manager instantiates no capture context at all — not even the `wl_shm` one
  — until it has seen real dmabuf feedback, so without this advertisement
  every quickshell `ScreencopyView` stays blank despite the capture protocol
  working.

  **If you use dma-buf clients, stay on the pixman renderer**: the format
  table is the CPU renderer's whichever renderer is active, so a client
  handing over a GPU buffer `--renderer gles` cannot import has it refused.
  See [tty.md](tty.md#which-renderer-draws-the-frames).
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
- **The `paint_cursors` option is accepted and has no effect**, which is a
  known deviation. Under `--headless` and `--nested` nothing draws a cursor
  at all, so a capture never contains one. Under `--tty` the cursor is part
  of the one framebuffer a capture is read out of, so a capture always
  contains it, flag or no flag.
- **Cursor capture sessions are refused.** `create_pointer_cursor_session`
  itself gets no event — the cursor-session object has no `stopped` of its
  own — but the `ext_image_copy_capture_session_v1` a client gets back from
  its `get_capture_session` is answered `stopped` immediately.

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
  the pointer cursor. A `scoot msg screenshot` reads that same framebuffer.
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
- **One output.** A lock surface is configured per `wl_output` and scoot has
  exactly one, so the one live surface a lock may hold is configured to that
  output's size, and the first blanked frame on it is what sends `locked`; a
  second `get_lock_surface` for the same output is refused with the
  protocol's `duplicate_output` error even when it names the output through a
  different `wl_output` bind (destroying the first surface frees the output
  for a rebuild).

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
work, on every backend. One control per output, and scoot has exactly one: a
second `get_gamma_control` transfers control, the old control gets `failed`
and stops affecting anything, and destroying the live control (or
disconnecting with one held) restores the default linear ramp.

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

- **Startup only, and one output.** The scale is read once when scoot starts
  and never changes; there is no config reload and no per-output setting.
  Changing it means restarting scoot.
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
  allocate nothing, so the per-client `wl_shm` pool count never moves for
  them. They still count against the 512-live-`wl_buffer` bound (uniform
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
