# flexwm

A scrolling-tiling Wayland compositor, in the shape of [niri](https://github.com/YaLTeR/niri):
lightweight, fast, GPU-optional, and built to be driven by a script or an agent
as easily as by a keyboard.

Two things distinguish it from a typical compositor:

- **It runs with no GPU.** Rendering goes through [pixman](http://pixman.org/)
  on the CPU, so it works headless and works in a GPU-less container (the
  target is running inside [webtop](https://github.com/linuxserver/docker-webtop)).
- **It's IPC-first.** Every action a keybind would trigger — focus, move,
  resize, spawn, close — and every input a user could give — key presses,
  pointer movement, clicks — is also a request on a Unix socket, alongside
  screenshots and window/output introspection. The compositor itself is
  driven the same way in its own end-to-end test (see `scripts/smoke-test.sh`).
  The intent is that an agent doing simple computer-use tasks in a VM is a
  first-class client, not an afterthought bolted on later. Because that socket
  can inject any keystroke, it's treated as a privileged channel: it lives in
  `$XDG_RUNTIME_DIR` (override with `$FLEXWM_SOCKET`), is created `0600`, and
  serves only connections from the same user as the compositor.

`flexwm-core`, the layout/state engine, is kept platform-independent on
purpose: it knows nothing about Wayland. The plan is for the same engine to
eventually back a macOS Accessibility-API adapter, doing OmniWM-style window
layout on a Mac, not just on Linux.

## Status

Early, but all three Wayland backends are real and working: `--headless`
(pixman rendering, xdg-shell, seat/input, full IPC control surface),
`--nested` (runs as a window inside an existing compositor, e.g. for webtop),
and `--tty` (a real DRM/KMS + libseat + libinput backend on actual hardware,
including VT switching; it picks its DRM device by trying every one on the
seat rather than trusting the first guess, and takes `--gpu PATH` when even
that picks wrong — see Which DRM device `--tty` drives below; and it renders
a pointer cursor, a client's own image when it supplies one, a built-in shape
otherwise, whose size and color the config file can override). Also done:
vim-style keybindings, a TOML config file (`--config`, `[layout]`/
`[appearance]`/`[binds]`, see Configuration below), window decorations (a
niri-style focus ring, background color, server-side
`zxdg_decoration_manager_v1`),
`wlr-layer-shell-unstable-v1`, so bars, docks, wallpapers, launchers and
notification daemons work — including keyboard focus for the ones that ask
for it (see Layer-shell clients below), `ext-workspace-v1`, so those
 bars can also list, follow and switch workspaces (see Workspaces for bars
 below), `ext-foreign-toplevel-list-v1`, so a taskbar, dock or alt-tab
 switcher can list the windows themselves (see Window lists for bars below),
 `ext-session-lock-v1`, so a real screen locker can lock the session
 with the compositor itself enforcing it (see Screen locking below),
 `ext-idle-notify-v1` and `idle-inhibit-unstable-v1`, so a `swayidle`-style
 daemon can idle the seat and lock it automatically while a video player
 holds it awake (see Idle detection below), the
 clipboard/primary-selection globals and `zwlr_gamma_control_manager_v1`, so
clipboard managers, middle-click paste and night-light tools work (see
Clipboard managers and Night light below),
`wp-cursor-shape-v1`, so a client can name the cursor it wants and get it
drawn from the machine's own installed xcursor theme -- or, where none is
installed, from ten shapes the compositor draws itself (see Cursor shapes
below), `xdg-activation-v1`, so a launcher can hand focus to
the app it started (see Focus handoff between clients below),
`xdg-toplevel-icon-v1`, so a bar or an agent can read a window's icon name
off `flexwm msg windows` (see Window icons below), `text-input-v3` and
`input-method-v2`, so an IME or on-screen keyboard can compose into the
focused text field, popup and all (see Input methods below), and
output scaling (`[output] scale` over `wl_output.scale`,
`wp_fractional_scale_v1`, `wl_surface.preferred_buffer_scale` and
`wp_viewporter`, so a HiDPI panel gets
correctly-sized clients and text instead of everything rendered physically
tiny — see Output scaling below), plus a
hardened control socket (owner-only
permissions, a same-user peer check, a 1 MiB cap on a single request,
screenshots rate-limited to one per connection per frame, at most 64
connections at once, a connection whose peer has stopped reading its
reply dropped after ten seconds, and a `wait-idle` capped at a minute
however long it asked for) whose connections
are non-blocking end to end, so no client — however slow, chunked or
unresponsive — can stall the compositor for anyone else. Sizes a client or a
config supplies are bounded too: each individual `wl_shm` pool is capped at
512 MiB (four full-screen 8K frames' worth — a request past it gets a
protocol error rather than a multi-gigabyte mapping for that pool; the total
across many pools from one client isn't bounded yet, see
`docs/backlog/security/shm-total-per-client-unbounded.md`), a client's
declared minimum window size can't exceed the largest
output's usable area on each axis, and `gap` and `cursor_size` each have an
upper bound as well as a lower one. A missing `$XDG_RUNTIME_DIR` is a one-line
startup error, not a crash. Verified
end-to-end on every backend — a real
client maps, tiles, receives synthetic input, and a screenshot proves it. Not
yet started: a GPU rendering path and the macOS adapter.

## Layout

```
crates/
  flexwm-core/   Platform-independent window state and scrolling-tile layout.
                 No Wayland, no I/O — pure data and functions, fuzz-tested.
  flexwm-ipc/    The wire protocol (requests/responses) and a client over it.
                 Builds on any platform.
  flexwm/        The CLI and, on Linux only, the Smithay-based compositor.
                 `flexwm msg ...` builds everywhere, so it can drive a
                 compositor running in a VM from a Mac.
vm/              A NixOS VM + flake for developing the Linux-only compositor
                 from macOS. See vm/README.md.
scripts/         smoke-test.sh: an end-to-end test driven entirely over IPC.
```

## Building

Two paths, both from the flake at the repo root. To just get the binary
(clone, then):

```sh
nix build                            # ./result/bin/flexwm
nix run . -- --headless -- foot      # build and run it in one step
nix run . -- msg windows             # the client, same binary
```

On Linux that builds the compositor. On macOS the compositor is compiled out
of the crate and the same package gives you `flexwm msg`, the client — useful
for driving a compositor running in a VM, but `flexwm --headless` there exits
with `the compositor only runs on Linux`. `nix build` deliberately does not
run the test suite (`flake.nix` says why); `cargo test` below is where that
runs.

To work on the code:

```sh
nix develop        # every dependency, on Linux or macOS
cargo build         # flexwm-core, flexwm-ipc, and the CLI build anywhere;
                     # the compositor itself only compiles on Linux
cargo test --workspace
cargo nextest run --workspace   # optional: one process per test, and faster
```

`cargo nextest` runs each test in its own process, which keeps the
compositor's real-Wayland-client test suites from sharing process-global
state. It is an addition, not a replacement — it does not run doctests, so
`cargo test` stays the baseline and needs no extra tool.

To actually run the compositor you need a real (or virtual) Linux machine with
a seat — see `vm/README.md` for a Mac-native NixOS VM that provides one.

## Running

```sh
flexwm --headless --width 1280 --height 800 -- foot   # start, spawn a terminal
flexwm --nested --width 1280 --height 800 -- foot     # inside your existing compositor
flexwm --tty -- foot                                  # on a real DRM/KMS seat
flexwm --tty --gpu /dev/dri/card1 -- foot             # ...naming the DRM device yourself
flexwm --tty --mode 1920x1080 -- foot                 # ...naming the display mode (see below)
flexwm msg windows                                     # in another shell
flexwm msg action focus-column left
flexwm msg screenshot --out /tmp/shot.png
flexwm msg type "hello"
flexwm msg wait-idle --quiet-ms 200
```

Add `--config PATH` to any of the three to load a TOML config; see
Configuration below for the full schema and default keybindings. Run
`flexwm --help` for the full request/action list.

`flexwm msg type TEXT` types text the way a person would, on whatever
keyboard layout the session is running: for each character it finds the key
that carries it and holds down whatever modifiers that key's level needs —
Shift for `A` or `!`, AltGr for a German layout's `@` — so a client receives
the same key *and* modifier events it would see from a real keyboard, not
just a bare keysym. `\n` and `\t` are sent as `Return` and `Tab`. Three
things worth knowing:

- A character the active layout can't produce is an error naming it (``no
  key for `é` in this layout``). Dead keys and compose sequences aren't
  driven, so a character that needs one counts as "no key" too — which is
  layout-dependent and worth checking before assuming ASCII is safe: `^`
  and `` ` `` are dead on `de`, `es`, `pt`, `se`, `no` and `dk`, and on the
  last four `~` is dead too. A character that sits on a level the layout only
  reaches through a *locking or latching* modifier gets its own, different
  message (``[character] needs a modifier this layout only locks or
  latches``) — flexwm will not press Caps Lock to type a capital, since
  that would leave it on for everything afterwards. In every case the
  characters *before* it in the string have already been typed: the request
  stops at the first character it can't type rather than rolling back.
- Keybindings still apply to what it types, exactly as they would to a real
  keypress. That only matters for a bind with no modifiers, or one on
  Shift plus a key; if a character does hit a bind, the compositor logs a
  warning naming it rather than swallowing it silently.
- `flexwm msg key COMBO` is the other one, and it is *not* the same: it
  presses exactly the combination named and holds exactly the modifiers
  named, nothing more. So name the key as it is with nothing held, plus the
  modifiers: `shift+1`, not `exclam`; `shift+a`, not `A`. A name that this
  layout only carries above its unmodified level is refused, because the
  key that carries it types a *different* character when pressed bare —
  `flexwm msg key exclam` would press the `1` key and deliver `1`. Some
  characters can't be named as a combination at all (`@` on a German layout
  needs AltGr, which `key` has no name for); `flexwm msg type` is the one
  that works the modifiers out from the layout, and the one to reach for
  when the goal is text rather than a chord.

### What the control socket refuses

Five bounds an agent driving flexwm over IPC can actually hit. The first
three are refusals with a reason — an ordinary `error` response, which
`flexwm msg` prints and exits non-zero on — rather than a silent drop or a
delay. The last two can't be: one drops a peer that is by definition not
reading its socket, and the other shortens a wait rather than refusing it.

- **One request line may be at most 1 MiB.** Past that the connection is
  told so and closed; there is no resynchronizing mid-line.
- **One screenshot per connection per 16ms frame.** A capture costs a full
  render and PNG encode on the thread that serves every other client, so a
  second one inside the same frame is refused rather than queued. Retry
  after a frame.
- **At most 64 connections at once**, across every client. A 65th is
  refused with a message naming the limit and closed immediately, not
  queued behind the others. This is what keeps the per-connection bounds
  above meaningful — otherwise reconnecting resets them — so an agent that
  wants many requests should pipeline them on *one* connection rather than
  open a connection per request. `flexwm msg` opens one per invocation and
  closes it as soon as it has its answer, so ordinary scripted use never
  approaches this.
- **A connection whose peer stops reading is dropped**, ten to twenty
  seconds after the last byte it took (the check runs on a deadline of its
  own, so the exact moment falls in that range rather than on the ten
  exactly). Nothing is sent when this happens — there is nobody reading to
  send it to; the connection simply closes. Replies that don't fit in the
  socket are queued and pushed out as the client reads; a client that takes
  no bytes at all for that long — the classic case being one that sends a
  request, does `shutdown(SHUT_WR)` and then never reads the answer — is
  treated as gone, and its connection and fds are released. Reading *slowly*
  is fine and is never given up on: the clock is measured from the last byte
  that actually went out, not from when the reply was queued, so draining a
  multi-megabyte screenshot over a minute costs nothing.
- **`wait-idle` waits at most 60 seconds**, whatever `--timeout-ms` asks
  for. A longer request isn't refused, it's shortened: the answer comes back
  as usual, at the minute mark at the latest. A waiting `wait-idle` keeps
  its connection (and one of the 64 slots above) for as long as it waits,
  and — uniquely on this socket — cannot notice its client dying while it
  waits, so an unbounded wait from a client that then exits would hold that
  slot for the rest of the session. The default is 5 seconds and the request
  is meant for hundreds of milliseconds, so this is well out of the way of
  any real use.

`--tty` needs a seat (`seatd` or logind) with a DRM device on it. On a modern
kernel, on every non-root `--tty` run, Smithay logs `Unable to become drm
master, assuming unprivileged mode` at startup — expected, not a failure: the
session manager opens the device and (normally) already holds DRM master on
flexwm's behalf, and flexwm simply isn't permitted to call `SET_MASTER`
itself on a file another process opened. `vm/README.md`'s troubleshooting
section has the kernel-level reason and two commands that check whether
master really is held, rather than assuming the log line alone settles it.

### Which DRM device `--tty` drives

Normally: whichever one works. flexwm asks Smithay for the seat's primary
GPU, and if that device turns out not to be able to drive a display, it
tries every other DRM device on the seat in turn before giving up. On an
ordinary PC the first pick is right and nothing else is ever opened; the
log line worth grepping for either way is `drm: driving this device`, with
the device path on it (`RUST_LOG=info`, the default level, is enough — a
rejected device gets a `drm: device unusable` warning naming it and saying
what it said).

The fallback exists because the usual heuristic — "the GPU whose PCI parent
has `boot_vga=1`, else the first one with a render node" — assumes the 3D
GPU and the display controller are the same DRM device. On Apple Silicon
under Asahi Linux they are not: `asahi`/AGX has the render node, `apple,dcp`
owns the CRTCs and connectors, and there is no PCI GPU or VGA BIOS for the
first rule to match. Picking the render-only device there fails with
`Operation not supported (os error 95)` loading its KMS resources. (That
specific machine is where the bug was reported from; the fallback is built
and tested, but no one has yet confirmed it end to end on Apple Silicon —
`--gpu` is the first thing to try there until someone does. It is not a
guarantee: naming a device skips the *search*, not the checks, so the device
named still has to open through the session and pass the same KMS probe every
automatic candidate does.)

If the automatic search still picks wrong, name the device:

```sh
flexwm --tty --gpu /dev/dri/card1 -- foot
```

`--gpu PATH` replaces the search entirely — exactly that device, no
fallback — so a wrong path is a clean startup error naming the device and
what failed, not a silent fall back to something else. It only means
anything under `--tty`; on `--headless` or `--nested` it is ignored with a
warning.

The output's size is the connector's preferred mode. When that is the wrong
size — under Apple's Virtualization framework (vfkit, UTM) the "preferred"
mode is just the host window's size in backing pixels, so it doubles or
halves with whichever screen the window opened on — name the mode:

```sh
flexwm --tty --mode 1920x1080 -- foot
```

`--mode WxH` picks the connector mode of exactly that size, and falls back to
the preferred one with a warning if the connector lists no such mode (`cat
/sys/class/drm/card*-*/modes` shows what it lists). Like `--gpu`, it is
ignored with a warning outside `--tty`.

Under `--tty` the output is named after its connector — `HDMI-A-1`, `eDP-1`,
`Virtual-1`, the same spelling as `/sys/class/drm/card*-*` — so bars and
shells label the screen as they would under any other compositor;
`flexwm msg outputs` shows the same name. `--headless` and `--nested` have no
connector and keep the name `headless`.

To see what the seat has, and which driver is behind each device:

```sh
ls /dev/dri/card*
for c in /sys/class/drm/card*/device/driver; do echo "$c -> $(readlink -f "$c")"; done
```

When nothing works, the startup error lists every device that was tried and
why each one was rejected, rather than naming only the first. If the *session*
refused all of them — which is what happens when another compositor already
holds the seat, since a seat takes one client at a time — the error says so
instead of suggesting `--gpu`: no choice of device gets around a busy seat.
And if enumerating the seat's other devices fails outright, the primary pick
is still tried on its own (with a `could not list the seat's other devices`
warning), rather than losing a working device to a failure in the fallback
machinery.

## Layer-shell clients (bars, wallpapers, launchers)

flexwm implements `wlr-layer-shell-unstable-v1` (version 5), the protocol
every panel, dock, wallpaper setter and notification daemon in the
wlroots-adjacent ecosystem uses — `waybar`, `swaybg`, `mako`, `wofi`,
`fuzzel`, `yambar` and friends. Start one the same way you start anything
else inside the session:

```sh
flexwm --tty -- foot              # ...then, in a shell inside the session:
swaybg -c '#123456' &             # a wallpaper, on the background layer
waybar &                          # a bar, on the top layer
fuzzel                            # a launcher, on the overlay layer
```

What works:

- **All four layers.** `background` and `bottom` draw behind windows (and
  behind the focus ring, so a wallpaper never hides it); `top` and `overlay`
  draw in front of them.
- **Anchors, margins and sizing**, including the protocol's rules for a
  surface anchored to opposite edges or given a zero dimension.
- **Exclusive zones.** A bar that reserves its own height shrinks the area
  windows are tiled within, so nothing is ever laid out underneath it — and
  gives that space back the moment it exits or its client dies. Several
  surfaces reserving on the same edge stack. `-1` ("don't push me around")
  reserves nothing, which is what a full-screen wallpaper wants.
- **Pointer input.** A click, scroll or motion over a layer surface goes to
  that surface, not to whatever window is behind it, and clicking a bar does
  not move window focus.
- **Keyboard focus**, following the protocol's `keyboard_interactivity`:
  - `none` (the default, and what a bar, wallpaper or notification daemon
    asks for) never takes the keyboard. Nothing changes for those clients.
  - `exclusive` on the `top` or `overlay` layer takes the keyboard as soon as
    the surface maps and holds it until it unmaps — what a launcher
    (`fuzzel`, `wofi`, `rofi`) and a layer-shell lock screen need. If several
    ask at once, the front-most one wins, and the keyboard falls back to the
    next one down when it goes away.
  - `on_demand` is click-to-focus, exactly like a window: click the surface
    to give it the keyboard, click a window, a bar or bare desktop to take it
    back. `exclusive` on the `bottom` or `background` layer is treated the
    same way — the spec allows normal focus semantics there, and nothing
    should be typing into a wallpaper unasked. Note that no *input* other
    than a click releases an `on_demand` surface: a keybinding or a `flexwm
    msg action focus-*` moves window focus but leaves the keyboard where it
    is. The spec leaves this implementation-defined, and click-only is the
    deliberate choice — the alternative silently steals a launcher's keyboard
    whenever a script re-focuses a window.
  - A surface that has committed but never attached a buffer, or that
    unmapped itself, can't hold the keyboard however it asks: there is
    nothing of it on screen to type into.
  - A click-focused surface can also hand the keyboard back itself, by
    committing `keyboard_interactivity: none` or by unmapping — and the click
    that focused it is spent when it does. Asking for `on_demand` again, or
    mapping again, gets it drawn again but not focused again: it waits for a
    fresh click, the same as the first time. So a bar that collapses and
    re-opens a search field, or a notification daemon that closes and
    re-opens an inline reply, never gets the keyboard back without the user
    actually clicking it. (An `exclusive` surface on `top`/`overlay` is the
    exception, per the rule above: it takes the keyboard whenever it is
    mapped, clicked or not.)

  **Keybindings always win.** They are matched before anything is forwarded
  to the focused client, so `Super+Shift+E` (quit) and, on `--tty`,
  `Ctrl+Alt+F1`..`F12` (VT switch) still work while a full-screen layer
  surface is holding every keystroke. That is the escape hatch if one wedges.

### Popup menus (`xdg_popup`)

An application menu, a combo box, a bar's own dropdown: all of them are
`xdg_popup` surfaces, and all of them map, draw, take clicks and — since
popup grabs landed — take the keyboard.

- **A grab routes input into the menu.** When a client opens a menu it asks
  for an explicit grab (`xdg_popup.grab`). While one is held, keyboard focus
  is on the popup, so Escape-to-close, arrow-key navigation and typeahead
  reach it; clicking inside it keeps it up; clicking anywhere outside
  dismisses it. Dismissal is also the only thing that ever sends
  `popup_done`, so without a grab nothing closes a menu.
- **Submenus nest.** A grab on a popup whose parent is the current grab
  takes over, and closing it unwinds to the parent menu rather than closing
  the whole chain.
- **A bar's own dropdowns work too** — a popup parented to a *layer* surface
  (`zwlr_layer_surface_v1.get_popup`), not just to a window.
- **A grab loses to the compositor's own focus rules, rather than fighting
  them**, in this order:
  1. **The session lock wins.** Locking dismisses an open menu, and a grab
     requested while locked is refused. Nothing but a lock surface receives a
     keystroke while the session is locked — a menu is not a way around that.
  2. **An `exclusive` layer surface on `top`/`overlay` wins** — a launcher
     opened over a menu is typeable, and the menu is dismissed rather than
     left on screen holding input it can no longer use.
  3. **The menu wins over everything else**: over the focused window, and
     over a layer surface that got the keyboard from a click — so an
     `on_demand` bar's own dropdown is not dismissed by the bar that opened
     it. An `exclusive` bar's own dropdown is the one case this doesn't hold
     for yet: rule 2 above dismisses it too, even though it is the very
     surface that opened it. Filed as
     `docs/backlog/protocols/popup-grab-exclusive-self-dismiss.md`.

  Losing is always spelled `popup_done` (the protocol lets a compositor
  dismiss a popup at any time), never "leave it up but take its input away".
- **Keybindings still win over a grab**, exactly as they do over an
  `exclusive` layer surface and for the same reason: they are matched before
  anything is forwarded. A client cannot wedge the session by holding a menu
  open.
- **Keyboard focus does not move onto a popup that did not grab.** A tooltip
  is an ordinary `xdg_popup` too, and handing one the keyboard would take it
  away from the window you are typing into. Menus and combo boxes that want
  keys take a grab; that is what the request is for.
- **The grab's serial is not checked against a real interaction, yet.**
  Unlike `xdg-activation-v1`'s token (see Focus handoff below), any same-uid
  client can map a popup and grab the keyboard with any serial it names —
  there is no proof-of-interaction gate here today. The other bounds still
  apply (locking or an exclusive layer surface dismisses the grab, any click
  outside dismisses it, keybindings still fire), so this is narrower than it
  sounds, but a client that was never focused can still take the keyboard on
  its own say-so. Filed as
  `docs/backlog/protocols/popup-grab-serial-validation.md`.

What doesn't, yet:

- **A layer-shell "lock screen" is still not a security boundary** — use a
  real `ext-session-lock-v1` locker instead (see Screen locking below).
  A layer surface with `exclusive` keyboard interactivity will now actually
  receive what you type instead of leaking it to the window behind, but the
  escape hatch that makes exclusive focus safe is also a way around such a
  lock: the quit binding and the `--tty` VT switches keep working while it is
  up, and everything behind it is still drawn and still capturable. Treat one
  as a screen *blanker* you can type a password into, not as something that
  keeps anyone out.
- **`flexwm msg outputs`** reports each output's *full* rectangle (in logical
  pixels, along with the output's `scale`) plus its *usable* rectangle — the
  full output minus whatever a bar reserved at its edges, which is where
  windows actually go. An agent asking "how big is the screen" wants `rect`;
  asking "where can a window be" wants `usable`. (An all-zero `usable` means
  the server predates the field — fall back to `rect`, which is what every
  client did before it existed.) Screenshots are captured at
  physical resolution, so multiply a logical rectangle by `scale` to convert
  it to screenshot pixels. Every success reply (`ok`) also carries the
  session-lock state it was built under (`locked`), so an agent typing a
  password over IPC learns the unlock landed from the very next reply —
  `true` is always truthful; `false` means unlocked or a server predating
  the field.

Two things for agents to know about layer surfaces:

**Window focus and keyboard focus are separate now.** `flexwm msg windows`'
`focused` flag, the focus ring and `flexwm msg action focus-*` all still mean
the *window*, and a layer surface holding the keyboard never appears there —
what it reports is where focus returns to once that surface goes away. So if
a launcher is up, `flexwm msg type "..."` and `flexwm msg key` go to the
launcher (which is usually what you want), while `flexwm msg windows` still
names the window behind it. There is no IPC request that reports a layer
surface yet.

The other: a bar redraws on its own schedule, and
`flexwm msg wait-idle` waits for *nothing on screen* to have redrawn.
Measured on real hardware with a `waybar` clock ticking once a second,
`--quiet-ms 200` still settles normally (204 ms) while `--quiet-ms 1500`
never does and times out. Keep `--quiet-ms` below whatever your bar's own
redraw interval is — the same caveat an animated cursor already carried.
(`--timeout-ms` is capped at 60 seconds, for the reason under What the
control socket refuses above.)

## Workspaces for bars (`ext-workspace-v1`)

flexwm implements `ext-workspace-v1` (version 1), the compositor-agnostic
successor to the one-off wlr workspace protocols, so a panel's workspace
module, a workspace switcher or an indicator can list workspaces, follow which
one is active, and switch between them. Together with layer shell above,
that's everything a bar needs. The global is `ext_workspace_manager_v1`,
available to every client, no privilege or allow-list.

What a client sees:

- **One workspace group**, carrying flexwm's single output. A client that
  binds `wl_output` after the manager still gets an `output_enter` for it, so
  registry order doesn't matter.
- **One `ext_workspace_handle_v1` per workspace**, named `"1"`, `"2"`, … in
  layout order, with matching one-dimensional `coordinates` — the workspace
  named `"1"` reports `[1]`, `"2"` reports `[2]`, and so on (sort by those,
  not by name — `"10"` sorts before `"2"` as a string). The active one carries
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
`commit` after it does nothing, deliberately.

Worth knowing before you write against it:

- **Workspaces are positions, not identities, and flexwm sends no `id`
  event.** The list grows as you use the trailing empty workspace and shrinks
  when a workspace empties out, which renumbers everything after it: handle 2
  means "the third workspace", not "that workspace". Redraw from what the last
  `done` said rather than remembering a handle as a particular user's
  workspace.
- **`deactivate`, `remove`, `assign` and `create_workspace` are ignored**, and
  no capability is advertised for them, because none of them exists in
  flexwm's layout model: an output always has exactly one active workspace,
  workspaces are created and dropped by the layout itself rather than by the
  user, and there is only one group.
- **`activate` is a request, not a guarantee** (as the protocol says): one
  naming a workspace that vanished between the client reading the list and the
  `commit` arriving is dropped, and one for the workspace that is already
  active does nothing.
- **Switching to a workspace *by number* works from both sides now.**
  `flexwm msg action focus-workspace-index N` (0-based, out of range does
  nothing) drives the same core action `ext-workspace-v1`'s `activate`
  already used, so an agent jumps straight to a workspace instead of
  stepping one at a time.
- **Multiple outputs will change the shape of this** — a group per output is
  what the protocol is built for — but flexwm has exactly one output today, so
  there is exactly one group.

## Window lists for bars (`ext-foreign-toplevel-list-v1`)

flexwm implements `ext-foreign-toplevel-list-v1` (version 1), the
compositor-agnostic protocol a taskbar, dock or alt-tab switcher reads the
*window* list from — the other half of what a shell needs alongside
`ext-workspace-v1` above. The global is `ext_foreign_toplevel_list_v1`,
available to every client, no privilege or allow-list.

It is the protocol-side twin of `flexwm msg windows`: the same windows, the
same lifetime, live rather than polled.

What a client sees:

- **One `ext_foreign_toplevel_handle_v1` per window**, carrying `identifier`,
  `title` and `app_id`. Binding the global announces every window that
  already exists, so registry order doesn't matter.
- **Changes arrive in batches closed by `done`** — draw on `done`, not on
  each event. In particular, a window is announced the moment its
  `xdg_toplevel` exists, which is *before* the toolkit has sent its app id
  and title: the first batch is usually two empty strings, followed a
  moment later by a batch per field as they arrive.
- **`closed` when the window goes**, after which nothing else is ever sent on
  that handle.
- **`stop` is answered with `finished`**, and means "no more *new* windows":
  handles the client already has keep reporting title and app id changes
  until it destroys them, which is what the protocol's own teardown sequence
  (stop, wait for `finished`, then destroy the handles) requires.

**The identifier is `<generation>-<window id>`** — e.g. `a3e689a2-1`: eight
hex digits of per-session randomness (the "opaque generation value" the
protocol recommends, so identifiers from two flexwm sessions never collide)
and, after the last dash, the window id `flexwm msg windows` reports. That
suffix is deliberate and is the bridge between the two: this protocol has no
requests at all, so a client that finds a window here and wants to *act* on
it sends `flexwm msg action focus-window-id N` with the number after the
dash.

Worth knowing before you write against it:

- **There is no control half, by design.** The protocol is deliberately
  minimal — no `activate`, `close`, `minimize`, `fullscreen`, geometry or
  per-output state. Those are meant for extension protocols that don't exist
  yet. flexwm's IPC covers the case (`flexwm msg action focus-window-id N`,
  `flexwm msg action close` for the focused window).
- **This is the `ext-` protocol, not `wlr-foreign-toplevel-management-v1`,
  and that matters for Quickshell-based shells today.** A tool that speaks
  only the older wlr protocol sees no global and shows no windows —
  measured: `quickshell` 0.3.1 (what DMS and Noctalia run on) is offered
  this global and never binds it, so their launchers' window sections stay
  empty. See `docs/backlog/protocols/wlr-foreign-toplevel-management.md`;
  the wlr protocol is a control protocol as much as an enumeration one, so
  it is an item of its own rather than a switch to flip.
- **A handle covers a window's whole life, not the time it is mapped.** The
  protocol talks about "mapped" toplevels, but flexwm has no map/unmap
  boundary at all — a window is in the layout, the focus order and
  `flexwm msg windows` from the moment its `xdg_toplevel` exists — so a
  handle covers exactly that, and the two lists can never disagree.
- **An identifier is never reused.** Close a window and open another and it
  gets a new one, even from the same client.
- **The list stays live while the session is locked**, exactly as `flexwm msg
  windows` does — see Screen locking below for the trust model that sits
  behind that.

## Screen locking (`ext-session-lock-v1`)

flexwm implements `ext-session-lock-v1` (version 1), so a real locker
(`swaylock` 1.7+, `gtklock`, `hyprlock`, `waylock`) can lock the session with
the compositor enforcing it, rather than a layer surface asking politely. The
global is `ext_session_lock_manager_v1`, available to every client (flexwm has
no security-context support to distinguish a privileged client from any
other, so an allow-list would be theatre — see the trust note below).

**What is guaranteed while the session is locked**, each of these verified on
real `--tty` hardware by screenshot and by what clients were actually sent:

- **Nothing but the lock client's own surfaces is drawn.** Not "drawn behind
  an opaque backdrop" — windows, layer surfaces (bars, wallpapers, launchers,
  on every layer including `overlay`) and the focus ring are not gathered into
  the frame at all. The screen is the lock surface, an opaque backdrop where
  it doesn't cover, and the pointer cursor. A `flexwm msg screenshot` reads
  that same framebuffer, so a capture taken while locked shows the lock screen
  and nothing behind it.
- **Only the lock surface receives input.** Keyboard focus moves to it (or to
  nobody, if the client hasn't created one yet) the instant the lock request
  arrives, and pointer focus is moved with it, so a click can't land in the
  window that happened to be under the pointer. Any grab in flight is
  dropped as well — not only when the lock is taken, but at every transition
  that changes which lock surfaces count (a lock surface destroyed, a lock
  given up, a locker that died), because a grab outlives focus changes by
  design and would otherwise keep steering input to whoever holds it. That
  covers an open popup menu, whose grab holds the *keyboard* and would
  otherwise swallow the focus change entirely and receive the password: the
  menu is dismissed, and a new grab asked for while locked is refused.
  A held-button grab and an open popup grab are each exercised by a test; the
  drag-and-drop case is still code-traced rather than exercised, since there
  is no drag-and-drop fixture here.
- **"The lock surface" means the *current* lock's.** A lock object can be
  destroyed while the client that made it stays connected and keeps the
  `wl_surface` underneath alive, so "is this surface alive" is not the same
  question as "does this surface still belong to the lock that owns the
  session". flexwm asks the second one everywhere: a surface from a lock that
  was given up, or replaced, stops being drawn and stops receiving keyboard
  and pointer input immediately, without waiting for anything to replace it.
- **A destroyed lock surface falls back to the backdrop straight away.** The
  protocol's own rule ("the compositor must fall back to rendering a solid
  color"), and "straight away" means without waiting for anything else to
  change on screen — including when the client destroys only the
  `ext_session_lock_surface_v1` and keeps the `wl_surface` under it alive,
  which is what a locker does when an output is removed under it.
- **Keybindings that run an action don't fire.** `Super+Q`, a `spawn` bind,
  every layout motion: suppressed, and forwarded to the lock client as
  ordinary keystrokes instead. The one exception is deliberate: the `--tty`
  `Ctrl+Alt+F1`..`F12` VT switches still work. That is a session-level escape
  hatch, not a way in — the VT it switches to has its own login, and this
  session stays locked behind it (verified: switch away, switch back, the lock
  screen is pixel-identical).
- **`flexwm msg action ...` is refused**, with an error saying why, for the
  same reason. So is an `ext-workspace-v1` client's `activate`.
- **Ordinary clients stop drawing.** They get no frame callbacks while
  locked, which is what the protocol asks for and also what keeps them from
  burning CPU behind a lock screen: measured on real hardware, a terminal
  running `while true; do date; done` costs the compositor 229 jiffies/10s
  unlocked and **2 jiffies/10s** with the same client still running behind a
  lock.

**If the lock client dies, the session stays locked.** That is the protocol's
rule and the point of it: a dead locker is not evidence that you want your
screen unlocked. flexwm's recovery story, so a crashed locker isn't a dead
session:

- the screen turns **solid red**, so you can tell "my locker crashed" from "my
  locker is showing a black screen" — including when the locker had a surface
  up and drawing at the moment it went, which is the case where showing its
  last pixels instead would leave you with no way to tell;
- **run a lock client again and it takes over** — it puts its own surface up
  and can unlock once you authenticate. It is told `locked` immediately when
  the lock it replaces had already blanked the screen; if that lock went in
  the window *before* its first blanked frame, the replacement waits for one
  exactly like a fresh lock does, because until then your actual desktop is
  still what's on the display. Taking the lock over at all rather than
  refusing is what sway does too (`lock.c`'s `handle_session_lock` replaces an
  abandoned lock and refuses a live one, exactly as flexwm does); the
  *conditional* confirmation is niri's (`Niri::lock` confirms immediately only
  from an already-locked state, never from a lock still waiting for its first
  frame). sway confirms unconditionally, fresh lock or takeover alike, so that
  half is deliberately not sway's behavior.

The same applies to a lock client that gives up without dying: destroying an
`ext_session_lock_v1` before `locked` arrives is legal (only
`unlock_and_destroy` is forbidden that early), and a locker that times out
waiting may well do it. The session stays locked and reads as abandoned — the
red screen, recovered by running a locker again — and the surfaces that client
had up stop being drawn and stop receiving input at that moment, even though
its connection and its `wl_surface`s are still perfectly alive.

**What is *not* guaranteed — read this before trusting it:**

- **A same-uid process is inside the boundary, and always was.** Anything that
  can reach flexwm's wayland socket can take over a lock whose client has died
  and then unlock the session; anything that can reach its IPC socket can
  screenshot the lock screen and inject keystrokes into it (that is how an
  agent drives a lock screen, and it is refused for `action` requests only).
  Both sockets are owner-only. This lock keeps *someone at the keyboard* out,
  not a process already running as you — which could read your files anyway.
- **`flexwm msg windows` still lists your windows while locked**, titles
  included, and `flexwm msg outputs` still answers. Nothing is drawn from
  them, but the IPC surface is not blanked.
- **So does `ext-foreign-toplevel-list-v1`** (see Window lists for bars
  above): handles stay, titles keep updating, and a window opened behind the
  lock screen is still announced. Same boundary as the line above — a client
  that can reach the wayland socket is a same-uid process — and sending
  `closed` for windows that did not close would be a lie a taskbar could not
  recover from, since the protocol forbids reusing their identifiers
  afterwards.
- **The `locked` event is sent once a blanked frame has been *rendered*, not
  once a vblank has confirmed it on screen.** Under `--headless`/`--nested`
  that is exact — there is no scanout at all, and the framebuffer a screenshot
  reads *is* that frame. Under `--tty` it is weaker than this section used to
  claim: the frame is rendered, and then copied into a scanout buffer with a
  page flip requested *only if the presenter can take it right then*. It takes
  neither step while the session is inactive (you switched VT away) or while a
  previous page flip hasn't been confirmed by a vblank yet — and that second
  case is ordinary, frequent throttling, not a rare edge. So under real
  contention the previous, possibly unlocked, frame can remain on the scanout
  buffer for up to one more vblank after `locked` has gone out. A client that
  suspends the machine the instant it sees `locked` is racing that. Closing it
  properly means confirming from the vblank handler instead; it's on the
  backlog rather than done.
- **Up to one frame of the unlocked screen can still be on the display**
  between the lock request and the first blanked frame. That is inherent (the
  protocol's `locked` ordering exists precisely because of it), not something
  flexwm defers.
- **The first click on a fresh lock screen, before the mouse has moved,
  reaches nobody.** Pointer focus is re-derived when the lock surface is
  created, which is before it has drawn anything, so the hit test finds
  nothing and the click goes nowhere; one pointer motion fixes it for the rest
  of the session. It fails safe — "nobody" is nobody, never the window that
  was under the pointer before the lock — and a locker is a keyboard-first
  thing, so it's a papercut rather than a hole, but it is on the backlog
  rather than fixed.
- **flexwm blanks immediately rather than waiting for the lock client to
  draw.** Some compositors wait up to a second for lock surfaces so the
  transition doesn't flash black; flexwm doesn't, deliberately — waiting means
  rendering the unlocked session for that whole second.
- **One output.** A lock surface is configured per `wl_output` and flexwm has
  exactly one; multi-output support has to revisit this.

## Idle detection (`ext-idle-notify-v1`, `idle-inhibit-unstable-v1`)

flexwm implements `ext_idle_notifier_v1` (version 2), so a `swayidle`-style
daemon can learn the seat has been quiet N milliseconds and dim the screen,
lock it (see Screen locking above) or suspend the machine — verified live
with real swayidle: the timeout command fires after a quiet window, input
runs the resume command, and the next quiet window fires again. Every input
source resets the timers: real devices under `--tty`, host-forwarded input
under `--nested`, and IPC-injected input (`flexwm msg type`/`key`/`pointer`
count as a user at the machine, which is also what keeps an agent's own
activity from looking like idleness). What does *not* reset them is the
compositor re-running its own hit test on a lock transition — that is not
input and doesn't claim to be.

`zwp_idle_inhibit_manager_v1` (version 1) is the reverse: a video player or
presentation app creates an inhibitor on one of its surfaces and `idled`
holds off until the inhibitor is gone — destroyed explicitly, or released
implicitly when the surface dies or the client disconnects. The
input-specific watch (`get_input_idle_notification`, for daemons with their
own inhibit policy) ignores inhibitors by design.

Two things to know: there is no built-in auto-locker — the timeouts and
commands are the daemon's config, not flexwm's, the swayidle way — and an
inhibitor counts while its surface is *alive*, whether or not it is visible.
A client inhibiting from a surface it never maps holds off `idled`; that
client is local either way (same trust model as the session-lock global
above).

## Clipboard managers and primary selection

flexwm exposes all three selection globals, on every backend, available to
every client (no security-context support here, so an allow-list would be
theatre — same trust model as the session-lock global: a same-uid process is
inside the boundary, see Screen locking above):

- **`zwlr_data_control_manager_v1`** (version 2): clipboard managers
  (`cliphist`, `clipman`). Set the clipboard without needing focus, read
  anything any client copies.
- **`ext_data_control_manager_v1`** (version 1): the successor protocol.
  Both generations are exposed side by side, as current compositors do, so a
  manager speaks whichever one it was written for.
- **`zwp_primary_selection_device_manager_v1`** (version 1): middle-click
  paste. Unlike the clipboard, this one is focus-gated — the compositor only
  accepts a `set_selection` from the client holding the keyboard, and only
  offers the selection to devices whose client holds it. A background client
  setting the primary selection is silently denied, not queued.

## Night light (`zwlr_gamma_control_manager_v1`)

flexwm implements `zwlr_gamma_control_manager_v1` (version 1), so
`gammastep` and `wlsunset` work, on every backend, available to every client
(same trust note as above — there is no privileged seat to reserve this
for). One control per output, and flexwm has exactly one output: a second
`get_gamma_control` transfers control, the old control gets `failed` and
stops affecting anything, and destroying the live control (or disconnecting
with one held) restores the default linear ramp.

What actually happens to the ramp depends on the backend:

- **Under `--tty`**, the ramp is pushed to the CRTC gamma LUT, so the screen
  really warms. The advertised `gamma_size` is the CRTC's own (256 on the
  hardware measured so far); anything the DRM device refuses retires the
  control with `failed` and the session keeps running.
- **Under `--headless`/`--nested`** there is no hardware LUT, so the ramp is
  accepted and stored but changes nothing on screen — and a `flexwm msg
  screenshot` reads the framebuffer, which is pre-LUT, so captures show the
  unmodified frame either way. `gamma_size` is 256 there.

A `set_gamma` fd must hold exactly three ramps of `gamma_size`
little-endian `u16` entries (red, green, blue); anything else — short, long,
empty, unreadable — is an `invalid_gamma` protocol error. Gamma control keeps
working while the session is locked: it changes no pixel's content, only the
output's color temperature, so a daemon warming the screen over a lock screen
is the ordinary case.

No config option, keybinding, CLI flag or IPC surface comes with any of
this: clipboard, primary selection and gamma are pure Wayland protocols, used
by existing clients as-is.

## Cursor shapes (`wp-cursor-shape-v1`)

flexwm implements `wp_cursor_shape_manager_v1` (version 2), so a client can
*name* the cursor it wants — `text`, `ew-resize`, `not-allowed` — instead of
loading an xcursor theme and uploading a surface of its own. Modern GTK4/Qt6
toolkits and `foot` prefer this when it exists; without it `foot` logs
"compositor does not implement server-side cursors".

A named shape is answered from **the cursor theme already installed on the
machine**: flexwm resolves `[appearance] cursor_theme`, else `$XCURSOR_THEME`,
else `default`, and draws that theme's own artwork — the same pixels the
client would have loaded for itself. That is what makes the protocol a win
rather than a downgrade: without it, advertising cursor-shape would take a
correctly themed I-beam away from a client that had been uploading one and
replace it with line art.

flexwm still ships no theme (niri's assets are GPL and Adwaita's aren't
MIT-clean — see License below), and it does not need to: reading the user's
own installed theme carries no such obligation, and is what sway, niri and
Hyprland do. Parsing is the MIT-licensed `xcursor` crate; nothing is
vendored.

**When no theme is installed** — a linuxserver webtop or any minimal
container, which is a first-class flexwm target — named shapes fall back to
ten shapes flexwm **draws procedurally itself**, in `cursor/shapes.rs`. The
mapping collapses the names a user cannot tell apart at 16 pixels:

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

The drawn shapes use the same `[appearance]` `cursor_size`/`cursor_color` the
built-in arrow already did, are built once at startup (never per frame and
never per request), and the arrow stays byte-identical to what it has always
been. Theme images are loaded when a client first asks for that shape — an
event, not a frame — and cached from then on, including a negative cache so a
theme missing `zoom-in` is not re-searched on every hover.

A client that uploads its own cursor *surface* still gets its own pixels
drawn, exactly as before. flexwm also exports `XCURSOR_THEME`/`XCURSOR_SIZE`
to everything it spawns, so a client that loads a theme itself (GTK3, and
anything predating this protocol) picks the same one the compositor draws —
which is the "consistent cursor across clients" half the protocol cannot
reach on its own.

Cursors are only drawn under `--tty`; `--headless` has no display and
`--nested` shows the host compositor's own cursor.

## Focus handoff between clients (`xdg-activation-v1`)

flexwm implements `xdg_activation_v1` (version 1), so a launcher can hand
focus to the app it started and a notification daemon can focus the app its
popup came from. Without it the only way to move focus is flexwm's own
keybindings and IPC — a client has no standard way to ask.

Honoring every such request unconditionally would be a focus-stealing
primitive, so flexwm checks the token three ways (see
`compositor/activation.rs`):

- **The token must name a real, recent input event that went to the client
  asking.** `set_serial(serial, seat)` is how a client says which click or
  keypress caused it to ask, and a token that names none of them — no serial
  at all, a seat flexwm doesn't own, or a stale or made-up number — is
  refused when it is created. What counts is a serial flexwm issued for a key
  or button event (press *or* release; pointer motion never counts, and
  neither does a *focus* event, which every newly mapped window gets for
  free) within the last few input events **and the last 10 seconds**, **and
  that was delivered to that same client**. Both bounds are needed: an idle
  session — an agent driving flexwm over IPC makes one, since injected
  actions are not input events — never rotates the event history, so without
  the clock a click from this morning would still be spendable tonight. The
  recipient half matters too: Wayland serials come from one
  process-wide counter shared with non-input events, so a number alone is
  cheap to observe and guess — pairing it with who actually received the
  event is what makes this a check on interaction. A launcher minting its
  token inside its own input handler passes; a background client that was
  never typed into or clicked has nothing to offer, which is exactly the
  focus-steal case the two bounds below do not stop.
- **A token is valid for 30 seconds.** Long enough for a cold-starting app to
  finish launching and redeem the token its launcher gave it; short enough
  that a token is still a receipt for something the user just did rather than
  a permit a client can sit on.
- **At most 64 unredeemed tokens exist at once**, across all clients, with
  expired ones swept first. `get_activation_token` is unauthenticated and
  unlimited, and nothing upstream prunes what it hands out; this is the same
  resource bound as the `wl_shm` pool cap.

The serial is checked when the token is *created*, never when it is redeemed:
a launcher hands its token to a process that may take seconds to start, by
which point plenty of newer input has happened, and re-checking then would
break the one case this protocol exists for. A client the user really did
interact with can still activate itself off that interaction — the user just
clicked it, which is the protocol working as intended.

What a refused token costs depends on what was being activated. For a **fresh
spawn** it is invisible: the app maps a window, and mapping focuses it, which
is where a launched app's focus came from before this protocol existed. For
**something already running** — a single-instance app (Firefox, Chromium,
anything on `GApplication`) re-invoked from a launcher, or the notification
daemon case above — nothing maps, so nothing else focuses it and the
activation simply does not happen. That is worth knowing because of the one
case flexwm refuses that the protocol would allow: a launcher that mints its
token from a *focus* serial rather than a key or button one (fuzzel does this
when an entry is picked with the mouse) gets no activation, and for an
already-running target that is the whole outcome.

A redeemed token is removed whether or not it was honored, so one user action
cannot be replayed into focus later. Activation goes through the same action
path a keybinding does, which means it is refused while the session is locked
and it scrolls the activated column into view rather than only marking it
focused. A refused activation does nothing visible: flexwm has no per-window
urgency state to raise instead.

## Window icons (`xdg-toplevel-icon-v1`)

flexwm implements `xdg_toplevel_icon_manager_v1` (version 1), so a client can
say which icon belongs to its window. flexwm draws no icons itself — it has
no titlebars, taskbar or window switcher — so this exists for the two
consumers outside it: a bar or dock showing a window list, and an agent
driving the session.

`flexwm msg windows` therefore grew one field:

```json
{ "id": 1, "app_id": "foot", "title": "zsh", "icon": "foot", ... }
```

`icon` is the freedesktop icon name the client committed, or `null`. It is
read off the surface's own current state when asked, so it is never stale and
never reports an icon the client attached but has not committed. The field is
optional on the wire (an older server simply omits it), so it does not bump
`PROTOCOL_VERSION`. Two deliberate limits: no *preferred icon sizes* are
advertised, since nothing in flexwm draws an icon and so it has no size to
prefer; and a client that supplies raw pixel buffers instead of a name reads
as having no icon, since handing those over IPC would mean re-encoding shm
buffers to PNG per query and no consumer has asked for it.

## Input methods (`text-input-v3`, `input-method-v2`)

flexwm implements `zwp_text_input_manager_v3` (version 1) and
`zwp_input_method_manager_v2` (version 1) — the two halves of IME support,
neither of which is useful alone. An application binds the first to say
"there is a text field here"; an input method (fcitx5, ibus, an on-screen
keyboard) binds the second to compose into it. Without the first, `foot` logs
"text input interface not implemented by compositor; IME will be disabled"
and never sets the path up at all.

Which text field is focused follows keyboard focus automatically, so it works
for a layer-shell surface with a search field (a launcher) as well as for an
ordinary window. What flexwm owns is the input method's **popup** — the
candidate window beside the text cursor — which is tracked against whichever
surface has the field and drawn with that surface's own popups, so it follows
the window, gets frame callbacks, and disappears when the field is disabled.

Same trust note as the other privileged globals: there is no client filter on
`zwp_input_method_manager_v2`, because an allow-list would be theatre without
security-context support. An input method is more privileged than a clipboard
manager — it can grab the keyboard and inject text into the focused client —
so this is a deliberate consistency with flexwm's existing trust model rather
than an oversight.

## Output scaling

A HiDPI panel needs the compositor to tell clients to render at a scale
greater than 1, or everything comes out physically tiny (text especially).
`[output] scale` sets that scale:

```toml
[output]
scale = 2.0        # 1.5, 1.25, ... all work; 1.0 is the default
```

It is advertised three ways, matching what clients actually support:

- **`wl_output.scale`** — the integer `ceil(scale)`. Every client that binds
  an output gets it automatically (re-sent on bind and whenever the output's
  state changes). A scale of `1.5` is therefore advertised as `2` here, which
  is what a client that only understands integer scaling should draw at.
- **`wp_fractional_scale_v1`** — the exact fractional value. A client that
  creates a `wp_fractional_scale_v1` for one of its surfaces is sent
  `preferred_scale` (`1.5`, not `2`), and can render a larger buffer and let
  the compositor scale it down. The `wp_viewporter` global is advertised
  alongside it, because that is the protocol a client uses to submit such a
  buffer (it sets the surface's logical destination size and flexwm scales the
  buffer into it) — without it, a fractional client has no way to render.
- **`wl_surface.preferred_buffer_scale`** (needs client `wl_compositor` v6) —
  the integer preference that accompanies the fractional value, sent with the
  default `preferred_buffer_transform` (`normal`). It is a separate event on a
  separate object from the fractional one, so a client that opts into
  fractional scaling receives **both**: the exact `1.5` *and* the integer `2`.
  This is protocol completeness (it is what wlroots sends); it is not claimed
  to be what makes any particular toolkit render. A client below
  `wl_compositor` v6 is not sent the event and keeps the implicit default of 1,
  exactly as before.

Notes, because they are real limits rather than polish:

- **Startup only, and one output.** The scale is read once when flexwm starts
  and never changes; there is no config reload and no per-output setting
  (flexwm has exactly one output). Changing it means restarting flexwm.
- **Clamped to `0.5..=4.0`**, warn-and-continue like every other config field
  (see Configuration below): a `scale` outside that range is brought into it
  and logged, and `nan`/`inf` fall back to `1.0`. A non-finite or zero scale
  would make the logical output size nonsense, so this is a correctness bound,
  not taste.
- **`--nested` is scale-1 only.** The host compositor owns the scale of the
  window flexwm is drawn inside, so a non-1.0 `scale` there would double-count
  it; flexwm logs a warning and uses `1.0`. `--headless` and `--tty` honour
  the setting.
- **Screenshots are physical pixels; layout coordinates are logical.**
  `flexwm msg screenshot` captures the framebuffer at full physical
  resolution, while `flexwm msg windows`/`outputs` report logical rectangles.
  An agent converts with `physical = logical * scale`, rounded down where a
  rectangle's edge lands mid-pixel (the logical size is `ceil(physical /
  scale)`, so a full-output `logical * scale` can overshoot by under one
  pixel). `flexwm msg outputs` reports each output's `scale` for exactly that
  (older servers omit it, which decodes as `1.0`).

## Configuration

`--config PATH` loads a TOML file explicitly. Without it, flexwm looks for
`$XDG_CONFIG_HOME/flexwm/config.toml`, falling back to
`~/.config/flexwm/config.toml` if `$XDG_CONFIG_HOME` is unset or empty, and runs on
built-in defaults if neither exists. Four optional tables: `[layout]`,
`[appearance]`, `[output]`, `[binds]`. Every field in every table is itself
optional and defaults independently, so a config that only sets `gap` leaves
everything else — including the rest of `[layout]` — at its built-in default.

**Failure semantics are deliberate, not an oversight.** An explicit
`--config PATH` that doesn't exist or can't be read is a hard startup
error — you pointed at it on purpose, so silently ignoring it would be worse
than failing loud. Every other problem falls back to defaults and logs
instead of blocking startup:

- No file at the *default* path: silent, not even a log line (a fresh
  install, not a mistake).
- The default path exists but can't be read (permissions): logged as an
  error, full defaults. (A broken symlink at that path resolves to "no such
  file," which is the silent case above, not this one.)
- Malformed TOML, an unknown/misspelled field name, or a field given the
  wrong type (a string where a number is expected, a negative number for a
  field that's unsigned) **anywhere in the file** (`[layout]`,
  `[appearance]`, `[binds]`, or the top level): logged as an error, and the
  *entire* file is discarded for full built-in defaults — a single bad field
  in `[layout]` also throws away an otherwise-valid `[binds]` table
  elsewhere in the same file.
- One bad `[appearance]` color string, or one bad `[binds]` entry: logged as
  a warning, and only that field/bind falls back — every other field and
  bind in the file still applies.

This is deliberate: on `--tty`, the real deployment target, flexwm *is* the
session — there's no other window manager to fall back to and often no easy
remote access. A compositor that refuses to boot over a config typo is a
hard lockout with no recovery, so it always starts with something usable and
says what's wrong in the log instead.

### `[layout]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `gap` | integer (pixels) | `12` | Gap between columns, between windows stacked in a column, and at output edges. Clamped into `0..=10000`: negatives become `0`, and anything above `10000` becomes `10000` — that is already wider than the long edge of an 8K display (so it insets any real output to nothing), and it keeps the layout's own integer arithmetic well away from overflow. A gap that large leaves no usable area, so windows end up 1x1; it's a guard against a typo or a probe, not a usable setting. |
| `column_widths` | array of floats | `[0.333…, 0.5, 0.666…]` (i.e. `1/3`, `1/2`, `2/3`) | Column widths as fractions of the output width, in the order `cycle-column-width` steps through. Non-finite or non-positive entries are dropped; an empty list falls back to the built-in three. |
| `default_column_width` | integer (unsigned) | `1` | Index into `column_widths` used for newly created columns (`1` selects `0.5`, i.e. half the output). Too large is clamped to the last valid index; negative isn't a valid value for this field at all, so it's a whole-file parse error (see Failure semantics above), not a clamp. |

### `[appearance]`

flexwm draws no titlebars by design — a focused window gets a colored ring
drawn *around* it (in the layout's own gap), and there's a solid background
behind everything, instead of a per-window title bar with text or buttons.
That's why there's no titlebar-color/font option below: this table controls
the ring, the background, and the built-in pointer cursor — nothing else.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `focus_ring_width` | integer (pixels) | `3` | Ring thickness. Clamped at load time to at most half of `gap`, so it can never visually reach a neighboring window. |
| `focus_ring_active_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#6ba6fa` (accent blue) | Ring color around the focused window. |
| `focus_ring_inactive_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#595961` (muted gray) | Ring color around every other window. |
| `background_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#141419` (near-black) | Cleared behind all window content — there's no separate background render element, this is the frame clear color. |
| `cursor_size` | integer (pixels) | `16` | Both dimensions of the built-in pointer cursor (see below). Clamped into `4..=256`: under `4` the shape is left with at most one interior pixel (none at all below 3), and a pointer that small is indistinguishable from a dead pixel; over `256` it covers a quarter of a 1080p display's height and the bitmap it allocates stops being small. A value outside `i32` altogether (or a float) isn't a valid value for this field at all, so it's a whole-file parse error (see Failure semantics above), not a clamp. |
| `cursor_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#ffffff` (white) | Fill color of the built-in pointer cursor. Its 1px outline is always black, at this color's own alpha, and isn't separately configurable — the outline exists to keep the shape's edges visible against similarly-colored content. That doesn't help against a *dark* `cursor_color`: with black (or any near-black) fill, the outline blends into it and the pointer can be hard to spot against dark window content. An alpha of `00` makes the built-in cursor invisible; that's your call, not a clamped value. |
| `cursor_theme` | string | unset | Which installed xcursor theme named cursor shapes are drawn from (see Cursor shapes above). Unset means follow `$XCURSOR_THEME`, then `default` — i.e. whatever the rest of the desktop uses; an empty string means the same as unset. This only *names* a theme, it never makes flexwm ship one, and a name that matches nothing installed is not an error: named shapes then come from flexwm's own drawn set, exactly as on a machine with no themes at all. |
| `prefer_no_csd` | boolean | `true` | Whether to answer a client's `zxdg_toplevel_decoration_v1` request with `ServerSide`, so a well-behaved client stops drawing its own titlebar (which would otherwise double up with the ring). |

`cursor_size` and `cursor_color` apply to flexwm's own drawn shapes — the
fallback used when the machine has no cursor theme installed, drawn only
under `--tty` (`--headless` has no display and `--nested` already shows the
host's cursor). `cursor_size` also picks which size is taken out of a real
theme's file, and `cursor_color` has no effect there: a theme's artwork
brings its own colors. None of the three affects a client that supplies its
own cursor *image* (a spinner, say): those pixels come from the client over
the wire, and flexwm draws them at the size and hotspot the client chose.
All three are read once at startup, like every other setting here — there's
no config reload.

The first three hex values above are the actual rendered colors (pixel-sampled
from a real screenshot, and pasting any of them back into the matching
config field reproduces the default exactly). Internally those three built-in
defaults are stored as raw RGBA floats (`0.42, 0.65, 0.98`,
`0.35, 0.35, 0.38`, and `0.08, 0.08, 0.1`, each `1.0` alpha), and none of
those floats is exactly representable as an 8-bit `"#rrggbb"` string — that's
a storage detail, not a reason to distrust the hex above. Leave a color
field unset to get the real built-in default; only set it to a hex string if
you want to *change* it. (`cursor_color`'s `#ffffff` is the one exception:
pure white *is* exactly representable, so writing it out changes nothing at
all.)

### `[output]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `scale` | float | `1.0` | Output scale advertised to clients and rendered at. `1.0` renders identically to no setting at all; anything else advertises `ceil(scale)` on `wl_output` and `wl_surface.preferred_buffer_scale`, and the exact value through `wp_fractional_scale_v1`/`wp_viewporter` (see Output scaling above). Clamped into `0.5..=4.0` with a warning, and a non-finite value falls back to `1.0`; startup-only. `--nested` ignores a non-1.0 value with a warning. |

### `[binds]`

A table of `"key combination" = "action string"`. A combo is
`modifier+modifier+...+key` (e.g. `"super+shift+h"`), or a bare key with no
modifier at all (e.g. `"Return" = "close"` — legal, and it intercepts every
press of that key with none of Super/Shift/Ctrl/Alt held; `Shift+Return`,
for instance, still reaches the focused client normally). Whitespace around
`+` is ignored. Modifier names are
case-insensitive: `ctrl`/`control`, `shift`, `alt`, and `super`/`logo`/
`meta`/`cmd` (all four spellings mean the same modifier — flexwm's own
tables and this doc call it "Super"). The key is an xkb keysym name (`h`,
`Return`, `F5`, ...), resolved by trying the name exactly as written first
and then case-insensitively — so `"return"` and `"RETURN"` both find
`Return`.

Binds are matched against a key's *unshifted* symbol, with Shift tracked as
an ordinary modifier, so **write the lowercase letter and name Shift
separately**: `"shift+a"`, not `"A"`. A single capital resolves exactly, to
the distinct `A` keysym, which is not what any keypress reports at the level
binds match on — so `"A" = "close"` parses and loads but can never fire
(verified on `--headless`; `"shift+a" = "close"` fires as expected). See the
`docs/backlog/config/binds-capital-letter.md` for the accept-it-anyway fix.

Action strings use exactly the grammar `flexwm --help`'s ACTIONS section
documents — one parser handles both `flexwm msg action ...` and a config
file's `[binds]` values:

```
focus-column|move-column|consume-or-expel   left|right
focus-window|move-window                    up|down
focus-workspace|move-window-to-workspace    up|down
focus-window-id ID | focus-workspace-index N | cycle-column-width | close | spawn COMMAND... | quit
```

e.g. `"focus-column left"`, `"close"`, or `"spawn foot -e htop"` (split on
whitespace, not run through a shell, so a path or argument containing a
space can't be expressed this way).

Two failure behaviors specific to `[binds]`, both worth knowing since they
fail silently rather than as a startup error:

- **A bind that doesn't parse** (unknown modifier, unknown key name, an
  invalid action string, or trailing text after the action) is skipped with
  a warning naming just that bind; every other bind in the file still loads.
- **Two different combo strings that resolve to the same actual key
  combination** — different modifier aliases, different modifier order, or
  different case (`"Super+H"` vs `"super+h"`) — are **both** skipped, with
  one warning naming all of them. `[binds]` is read into a `HashMap`, whose
  iteration order has no relationship to the order the keys were written in
  the file, so "the last one wins" isn't something this code can honor
  truthfully; rather than pick an arbitrary, run-to-run-unstable winner, the
  whole colliding group is dropped and whatever was bound to that combo
  before the file loaded (a default, or nothing) is left in place.

A user bind on a combo that already has a default simply replaces it; there
is no "unbind" action.

### Default keybindings

| Combo | Action |
|---|---|
| `Super+h` | Focus column left |
| `Super+l` | Focus column right |
| `Super+j` | Focus window down |
| `Super+k` | Focus window up |
| `Super+Shift+h` | Move column left |
| `Super+Shift+l` | Move column right |
| `Super+Shift+j` | Move window down |
| `Super+Shift+k` | Move window up |
| `Super+Alt+h` | Consume or expel column left |
| `Super+Alt+l` | Consume or expel column right |
| `Super+Ctrl+j` | Focus workspace down |
| `Super+Ctrl+k` | Focus workspace up |
| `Super+Ctrl+Shift+j` | Move window to workspace down |
| `Super+Ctrl+Shift+k` | Move window to workspace up |
| `Super+r` | Cycle column width |
| `Super+q` | Close focused window |
| `Super+Return` | Spawn `foot` |
| `Super+Shift+e` | Quit |

That's all 18 default bindings — vim motions (`h`/`j`/`k`/`l`) for direction,
Super as flexwm's own modifier throughout. Quit is deliberately
`Super+Shift+e`, not `Super+Shift+q`: that combo is one slipped Shift away
from `Super+q` (close focused window), and a slip of the finger shouldn't be
able to end the whole session.

`--tty` additionally binds `Ctrl+Alt+F1` through `Ctrl+Alt+F12` to switching
to VT 1 through 12 — not present under `--headless`/`--nested`, since VT
switching is a Linux-session concept with no meaning there. These are added
*after* the config file loads and always win over a colliding config-file
bind (logging a warning naming whatever they displaced): on real hardware,
with no other window manager and often no easy remote access, Ctrl+Alt+Fn is
the one recovery path if the display ever gets wedged, so it can't be
allowed to silently lose to a config-file typo or a well-meaning rebind.

### Example `config.toml`

```toml
[layout]
gap = 8
column_widths = [0.25, 0.5, 0.75, 1.0]
default_column_width = 1

[appearance]
focus_ring_width = 4
focus_ring_active_color = "#ffaa00"
focus_ring_inactive_color = "#333333"
background_color = "#101014"
cursor_size = 24
cursor_color = "#ffcc66"
# Unset follows $XCURSOR_THEME, then "default" -- i.e. the rest of the
# desktop. Name one here only to override that.
# cursor_theme = "Adwaita"
prefer_no_csd = true

[output]
# 1.0 is correct for a non-HiDPI display; raise it (e.g. 2.0) on a HiDPI
# panel, or text and widgets render far too small. See the reference above.
scale = 1.0

[binds]
"super+n" = "focus-column right"
"super+shift+n" = "move-column right"
"super+t" = "spawn foot"
"super+shift+t" = "spawn foot -e htop"
"ctrl+alt+space" = "spawn wofi --show drun"
```

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
