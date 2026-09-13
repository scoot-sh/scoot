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
including VT switching and a rendered pointer cursor — a client's own cursor
image when it supplies one, a built-in shape otherwise, whose size and color
the config file can override). Also done: vim-style
keybindings, a TOML config file (`--config`, `[layout]`/`[appearance]`/
`[binds]`, see Configuration below), window decorations (a niri-style focus
ring, background color, server-side `zxdg_decoration_manager_v1`), and a
`wlr-layer-shell-unstable-v1`, so bars, docks, wallpapers, launchers and
notification daemons work — including keyboard focus for the ones that ask
for it (see Layer-shell clients below) — and a
hardened control socket (owner-only
permissions, a same-user peer check, a 1 MiB cap on a single request, and
screenshots rate-limited to one per connection per frame) whose connections
are non-blocking end to end, so no client — however slow, chunked or
unresponsive — can stall the compositor for anyone else. Sizes a client or a
config supplies are bounded too: each individual `wl_shm` pool is capped at
512 MiB (four full-screen 8K frames' worth — a request past it gets a
protocol error rather than a multi-gigabyte mapping for that pool; the total
across many pools from one client isn't bounded yet, see `ROADMAP.md`'s
Backlog), a client's declared minimum window size can't exceed the largest
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

```sh
nix develop        # every dependency, on Linux or macOS
cargo build         # flexwm-core, flexwm-ipc, and the CLI build anywhere;
                     # the compositor itself only compiles on Linux
cargo test --workspace
```

To actually run the compositor you need a real (or virtual) Linux machine with
a seat — see `vm/README.md` for a Mac-native NixOS VM that provides one.

## Running

```sh
flexwm --headless --width 1280 --height 800 -- foot   # start, spawn a terminal
flexwm --nested --width 1280 --height 800 -- foot     # inside your existing compositor
flexwm --tty -- foot                                  # on a real DRM/KMS seat
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
  and `` ` `` are dead on `de` and `es`, and `~` is dead as well on `pt`,
  `se`, `no` and `dk`. A character that sits on a level the layout only
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

`--tty` needs a seat (`seatd` or logind) with a DRM device on it. On a modern
kernel, on every non-root `--tty` run, Smithay logs `Unable to become drm
master, assuming unprivileged mode` at startup — expected, not a failure: the
session manager opens the device and (normally) already holds DRM master on
flexwm's behalf, and flexwm simply isn't permitted to call `SET_MASTER`
itself on a file another process opened. `vm/README.md`'s troubleshooting
section has the kernel-level reason and two commands that check whether
master really is held, rather than assuming the log line alone settles it.

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

What doesn't, yet:

- **Screen locking is not a security boundary here.** The keyboard model
  above is what a layer-shell locker (`gtklock`, `swaylock-effects`) needs to
  function, and one will now actually receive what you type instead of
  leaking it to the window behind — but flexwm implements no
  `ext-session-lock-v1`, and the escape hatch that makes exclusive focus safe
  is also a way around a lock: the quit binding and the `--tty` VT switches
  keep working while the locker is up. Treat it as a screen *blanker* you can
  type a password into, not as something that keeps anyone out.
- **Popups from a layer surface** (a bar's own dropdown menu or tooltip) are
  not tracked yet, for a reason that predates this: flexwm doesn't send the
  initial configure for *any* `xdg_popup` yet, so no popup maps, from a window
  or a layer surface. Also in the backlog.
- **`flexwm msg outputs`** reports each output's *full* rectangle. The
  reserved area a bar takes isn't exposed over IPC yet; an agent asking "how
  big is the screen" gets the screen.

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

## Configuration

`--config PATH` loads a TOML file explicitly. Without it, flexwm looks for
`$XDG_CONFIG_HOME/flexwm/config.toml`, falling back to
`~/.config/flexwm/config.toml` if `$XDG_CONFIG_HOME` is unset or empty, and runs on
built-in defaults if neither exists. Three optional tables: `[layout]`,
`[appearance]`, `[binds]`. Every field in every table is itself optional and
defaults independently, so a config that only sets `gap` leaves everything
else — including the rest of `[layout]` — at its built-in default.

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
| `prefer_no_csd` | boolean | `true` | Whether to answer a client's `zxdg_toplevel_decoration_v1` request with `ServerSide`, so a well-behaved client stops drawing its own titlebar (which would otherwise double up with the ring). |

The two `cursor_*` fields apply to flexwm's own procedurally-drawn fallback
shape — a filled triangle whose point is the hotspot, drawn only under
`--tty` (`--headless` has no display and `--nested` already shows the host's
cursor). They do **not** affect a client that supplies its own cursor image
(a text I-beam, a resize arrow, a spinner): those pixels come from the client
over the wire, and flexwm draws them at the size and hotspot the client
chose. There is deliberately no `cursor_theme` option: honoring a named
xcursor shape means loading a real theme asset, and flexwm has no MIT-clean
one to load (see `ROADMAP.md`'s Backlog). Both values are read once at
startup, like every other setting here — there's no config reload.

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
`ROADMAP.md` backlog entry for the accept-it-anyway fix.

Action strings use exactly the grammar `flexwm --help`'s ACTIONS section
documents — one parser handles both `flexwm msg action ...` and a config
file's `[binds]` values:

```
focus-column|move-column|consume-or-expel   left|right
focus-window|move-window                    up|down
focus-workspace|move-window-to-workspace    up|down
focus-window-id ID | cycle-column-width | close | spawn COMMAND... | quit
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
prefer_no_csd = true

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
