# Configuration reference

- [Command-line flags](#command-line-flags)
- [The config file](#the-config-file)
- [Reloading the config](#reloading-the-config)
- [`[layout]`](#layout) · [`[appearance]`](#appearance) · [`[output]`](#output) · [`[renderer]`](#renderer) · [`[tty]`](#tty) · [`[autostart]`](#autostart) · [`[binds]`](#binds)
- [Default keybindings](#default-keybindings)
- [Example `config.toml`](#example-configtoml)

## Command-line flags

```
scoot --headless [--width 1-65535] [--height 1-65535] [--outputs 1-8] [--renderer pixman|gles] [--socket PATH] [--config PATH] [-- COMMAND...]
scoot --nested   [--width 1-65535] [--height 1-65535] [--renderer pixman|gles] [--socket PATH] [--config PATH] [-- COMMAND...]
scoot --tty      [--gpu PATH] [--mode WxH] [--renderer pixman|gles] [--socket PATH] [--config PATH] [-- COMMAND...]
scoot msg REQUEST          # the scootctl client, kept as an alias (see below)
scoot --print-default-config [--write]   # emit a starting config file to stdout, or place it directly with --write
scoot --version                # identify this build without starting anything
scoot --help
```

`scoot msg REQUEST` is byte-for-byte the `scootctl REQUEST` client kept on
the compositor binary as a permanent alias -- agents reach for `scootctl`
(see [ipc.md](ipc.md)); everything under `REQUEST` is documented there, not
duplicated here.

| Flag | Meaning |
| --- | --- |
| `--headless` | No display at all. Renders into a framebuffer that `scootctl screenshot` reads. |
| `--nested` | Runs as a window inside an existing compositor. The session follows that window's size: resize it and the desktop inside resizes with it, for the life of the session. If the host cannot be followed to a new size (it could not be allocated), scoot logs it, stays at the size it was, and keeps running — the host letterboxes the difference. |
| `--tty` | A real DRM/KMS + libseat + libinput session — see [tty.md](tty.md). |
| `--width N`, `--height N` | The `--headless`/`--nested` output size, 1–65535 per axis (default 1600x1000) — whatever DRM itself can report for a mode (`drm_mode_modeinfo` stores each axis in a `u16`), with room to spare past any real display. Anything else is a startup error naming the flag and the expected range (``invalid --width: `70000` (expected 1-65535)``), not a silently different size. Under `--nested` it is only the size scoot *asks* for: the host's first configure decides what the window comes up at, and every later one moves it. Under `--headless` it is the size for the whole session. |
| `--outputs N` | How many outputs `--headless` creates, 1–8 (default 1). Each is `--width` by `--height` and sits immediately right of the last, so two 1280-wide outputs at scale 1 cover x 0–1279 and 1280–2559. Out of range is a startup error naming the range, like `--width`. `--headless` only: `--nested` presents one window in its host and `--tty` drives one CRTC, so both warn and ignore it. See [More than one output](#more-than-one-output) for what a second output does and does not do yet. |
| `--renderer pixman\|gles` | Which renderer composites each frame. Config-file form: `[renderer] backend`. See [tty.md](tty.md#which-renderer-draws-the-frames). |
| `--gpu PATH` | Which DRM device `--tty` drives. Config-file form: `[tty] gpu`. Ignored with a warning outside `--tty`. See [tty.md](tty.md#which-drm-device---tty-drives). |
| `--mode WxH` | Which connector mode `--tty` picks. Ignored with a warning outside `--tty`. |
| `--socket PATH` | Where the IPC control socket lives, overriding `$SCOOT_SOCKET` and the default `$XDG_RUNTIME_DIR/scoot.sock`. See [ipc.md](ipc.md#the-socket). |
| `--config PATH` | Load this TOML file instead of searching the default paths. |
| `-- COMMAND...` | Spawn this command once the session is up, after `[autostart]` entries (see [Starting a session](#starting-a-session)). |
| `--version` | Print `scoot <version> (ipc protocol <N>)` — the binary's own version plus the IPC protocol number — and exit. Needs no compositor; `scootctl --version` prints the same line, so a client can check compatibility against a remote compositor before connecting. |
| `--help` | Usage, every request and every action. |

Environment scoot reads: `$XDG_RUNTIME_DIR` (required — a missing one is a
one-line startup error), `$SCOOT_SOCKET`, `$XDG_CONFIG_HOME`,
`$XCURSOR_THEME`, and the session locale (`$LC_ALL`, `$LC_CTYPE`, `$LANG`)
for [`scootctl type`](ipc.md#type-vs-key). Environment scoot exports to what
it spawns: `$WAYLAND_DISPLAY`, `$SCOOT_SOCKET`, `$XCURSOR_THEME`,
`$XCURSOR_SIZE`, `$XDG_CURRENT_DESKTOP=scoot` (always — a child talks to
this compositor, so it must name this compositor even under `--nested`
inside another one), `$XDG_SESSION_TYPE=wayland` and
`$XDG_SESSION_DESKTOP=scoot` (only where unset or empty — on a logind seat
both are logind's to set, and scoot keeps its owner's values), and —
unless the token table is full — a fresh
`$XDG_ACTIVATION_TOKEN`. A token the compositor was itself started with is
removed rather than passed on: it is a receipt for someone else's user
action.

### Portals and the D-Bus activation environment

`$XDG_CURRENT_DESKTOP=scoot` above is what lets
[xdg-desktop-portal](https://github.com/flatpak/xdg-desktop-portal) pick a
backend for this session — without it a browser has no screen sharing on
Wayland and degraded file choosers. But setting it on scoot's children is
only half: the portal itself is D-Bus activated, so it inherits the **D-Bus
activation environment**, not the environment of whatever client asked.
That half belongs to the session script (config is state, the script is
behavior — scoot itself never touches the bus):

```sh
# systemd session (the niri/xdpw shape):
dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
# s6 / seat without a user manager (the webtop target):
dbus-update-activation-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
```

Only those two travel to the bus: the `$XDG_SESSION_*` values scoot fills
for its children stay out (nothing on the bus reads them). `resources/scoot-portals.conf` names which backend serves what once the
lookup can find us: everything falls through to `gtk`, while
`ScreenCast`/`Screenshot` go to `wlr` — which binds against scoot through
`ext-image-copy-capture-v1` (needs xdg-desktop-portal-wlr 0.8.0+; older
releases need the `wlr-screencopy` global scoot deliberately omits) and
through `grim` for screenshots (so `grim` must be installed). `ScreenCast`
covers whole outputs; per-window sharing is refused — scoot does not
advertise the toplevel capture-source manager (see
`backlog/resolved/screencopy-toplevel-capture-done.md`). Install the
file as `scoot-portals.conf` in the first of these your setup provides —
`~/.config/xdg-desktop-portal/`, `/etc/xdg-desktop-portal/`,
`/usr/share/xdg-desktop-portal/` — and xdg-desktop-portal 1.17+ does the
rest. (Under home-manager this copy-over is the module's job:
`programs.scoot.portals.enable`, on by default — see
[`docs/nix.md`](nix.md). Without the module, it stays manual.)

### More than one output

`--headless --outputs N` gives a session N virtual outputs with no second
monitor in the building. It exists so per-output behaviour is testable; it is
the foundation for multi-output, not the whole of it.

What a second output **is**, today:

- a real `wl_output` global with its own name (`headless`, `headless-2`, ...),
  its own mode and its own position in the global coordinate space, so a
  client can address it;
- its own output in `scootctl outputs`, with its own id, rectangle and name;
- its own workspaces and its own scrolling strip in the layout, so a window is
  on exactly one output and nothing scrolls across a boundary;
- a layer surface naming it is configured against it and unmapped from it;
  a surface naming no output lands on the first output;
- its own exclusive zones: a bar on one output shrinks that output's tiling
  area and no other's, and gives it back when it exits or its client dies;
- its own layer-shell input and keyboard derivation: the pointer is
  hit-tested against the output under it, a click on that output's bar
  focuses the bar, and an `exclusive` launcher mapped where the pointer is
  takes the keyboard there (the pointer's output wins ties);
- a window list that names the output each window is on
  (`wlr-foreign-toplevel-management` `output_enter` per window's own output,
  paired with `output_leave` when a move carries it across);
- its own composited strip: every output has a render target of its own and
  the render loop draws each one, so `scootctl screenshot --output 2` answers
  with the second output's own pixels, a screen capture of it (`grim -o
  headless-2`) reads its own framebuffer, a gamma control names it
  specifically, and its layer surfaces get frame callbacks at its own
  cadence. Each output is `--width` by `--height`, like the first;
- its own session-lock surface: a locker puts one surface up per output,
  each configured to its own output's size, each drawn onto its own screen,
  with the keyboard on the pointer's output's surface -- and `locked` waits
  for every output's blanked frame, so no screen confirms while another
  still shows the desktop;
- its own workspaces in `ext-workspace-v1`: one group per output, each with
  that output's list and active index, so a bar reads its own screen and a
  switch on one output never disturbs another's;
- its own head in `wlr-output-management`: one head per output with its own
  name, mode and position (`apply`/`test` stay refused — see
  [protocols.md](protocols.md#display-information-wlr-output-management-v1));
- a pointer that crosses it: relative motion clamps to the union of every
  output's geometry, so the mouse reaches the second screen instead of
  trapping on the first — the seam pixel belongs to the output on its right,
  and absolute motion is never clamped.

What it is **not**, yet — the work tracked in
`docs/backlog/core/multi-output.md` (milestone 19, phase E and beyond):

- new windows open on the output under the pointer (falling back to the
  first output when the pointer is over no output); moving one across
  outputs, and focusing another output from the keyboard, is bound by
  default for outputs 1 and 2 (see [Moving across
  outputs](#moving-across-outputs)) — ids 3+ stay manual;
- no per-output mode/scale/position configuration surface:
  `wlr-output-management` `apply`/`test` stay refused;
- `--tty` driving two connectors at once (phase E, hardware-gated).

## Starting a session

scoot starts the programs inside the session two ways, which compose rather
than compete. A bar, a wallpaper, a notification daemon, a launcher — each
starts "the same way you start anything else inside the session" (see
[protocols.md](protocols.md#layer-shell-bars-wallpapers-launchers)); this
section says where that shell runs.

**The session script** is the `-- COMMAND...` flag: everything after `--` is
one program and its arguments, run once the session is up with
`WAYLAND_DISPLAY`, `SCOOT_SOCKET` and the session environment already set.
It is the 20% route — the one with ordering, conditionals, and `wait`:

```sh
#!/bin/sh
# ~/bin/session.sh
swaybg -c '#123456' &     # a wallpaper, on the background layer
waybar &                  # a bar, on the top layer
mako &                    # a notification daemon
exec foot                 # the terminal the session starts with
```

```sh
scoot --tty -- ~/bin/session.sh
```

scoot does not wait on the script and does not exit when it exits — fire
and forget — and it restarts nothing that dies. Supervision is explicitly
out of scope: restarting a crashed bar is a service manager's job (on the
webtop target, the container's — that target has no systemd). Exited
children are reaped, so nothing lingers as a zombie.

**On webtop**, scoot runs `--nested` inside the host compositor, launched
from `/defaults/startwm.sh`:

```sh
#!/bin/sh
# /defaults/startwm.sh (linuxserver webtop): the container's desktop init
# execs this with the session environment already set. Start scoot nested
# in the host compositor, with the same session script:
exec scoot --nested -- /path/to/session.sh
```

**`[autostart]`** is the 80% route — the programs with no ordering or
conditionals, as action strings in the config file (see
[`[autostart]`](#autostart)). Entries run first, in file order, then the
`--` command: the config declares the session baseline, the script carries
the behavior. Spawning the same bar in both places yields two bars — pick
one route per program.

(Under home-manager, `programs.scoot.settings` renders this TOML and a
session script carries the behavior half — see
[`docs/nix.md`](nix.md), which owns that mapping.)

## The config file

`--config PATH` loads a TOML file explicitly. Without it, scoot looks for
`$XDG_CONFIG_HOME/scoot/config.toml`, falling back to
`~/.config/scoot/config.toml` if `$XDG_CONFIG_HOME` is unset or empty, and
runs on built-in defaults if neither exists. Seven optional tables:
`[layout]`, `[appearance]`, `[output]`, `[renderer]`, `[tty]`,
`[autostart]`, `[binds]`.
Every field in every table is itself optional and defaults independently, so
a config that only sets `gap` leaves everything else — including the rest of
`[layout]` — at its built-in default.

No config file yet? `scoot --print-default-config >
~/.config/scoot/config.toml` writes a starting one to stdout (never to a
path, so it cannot clobber anything), generated from the same defaults this
page documents — every key present and commented out with its default as the
value, so the file as-is is exactly the defaults. `scoot
--print-default-config --write` places that same emission at the default
location directly (`$XDG_CONFIG_HOME/scoot/config.toml`, else
`~/.config/scoot/config.toml`) and prints `wrote <path>`: it creates the
parent directory when missing, writes the file private to you (`0o600`),
and refuses loudly rather than overwriting anything already there —
including a symlink, which is refused as itself without being followed.
There is no custom destination and no overwrite: stdout composes for every
other path (`> wherever`). (On a machine with no
`scoot` binary — macOS, where only the `scootctl` client builds — copy the
[example below](#example-configtoml) instead.)

**Most settings are read once at startup; some can be reloaded live.**
`scootctl reload` (see [Reloading the config](#reloading-the-config))
re-reads this same file and re-applies the layout (gap, column widths and
the default column width), the appearance (including
the cursor size, color and theme) and the
keybindings. Everything else is startup-only and a reload refuses it with a
message rather than silently ignoring it.

### Failure semantics

An explicit `--config PATH` that doesn't exist or can't be read is a hard
startup error — you pointed at it on purpose. Every other problem falls back
to defaults and logs instead of blocking startup, with two exceptions (both
cases where guessing would be worse than refusing — see below and
[`[renderer]`](#renderer)):

- No file at the *default* path: silent, not even a log line (a fresh
  install, not a mistake).
- The default path exists but can't be read (permissions): logged as an
  error, full defaults. (A broken symlink at that path resolves to "no such
  file," which is the silent case above.)
- Malformed TOML, an unknown/misspelled field name, or a field given the
  wrong type (a string where a number is expected, a negative number for a
  field that's unsigned) **anywhere in the file**: logged as an error, and
  the *entire* file is discarded for full built-in defaults — a single bad
  field in `[layout]` also throws away an otherwise-valid `[binds]` table
  elsewhere in the same file.
- One bad `[appearance]` color string, one bad `[binds]` entry, or one bad
  `[autostart]` entry: logged as a warning, and only that field/bind/entry
  falls back — every other field and bind in the file still applies.
- A set-but-unusable `[tty] gpu` (a wrong path, or an empty one): a hard
  startup error naming the key, not a silent fallback to the automatic pick.
  This is one of the two deliberate startup exceptions; the other is
  `[renderer] backend = "gles"` with no working EGL (see
  [`[renderer]`](#renderer)).

The reason for the general rule: on `--tty`, the real deployment target,
scoot *is* the session — there's no other window manager to fall back to and
often no easy remote access. A compositor that refuses to boot over a config
typo is a hard lockout with no recovery, so it always starts with something
usable and says what's wrong in the log instead. `[tty] gpu` is the exception
because falling back there would mean silently driving a device the config
explicitly ruled out.

## Reloading the config

Two triggers re-read the same file startup used — the explicit `--config
PATH` when one was given, else the resolved default path — and re-apply
what can be re-applied live:

- `scootctl reload` (and `scoot msg reload`, the same client), which answers
  with the applied-vs-refused report below;
- `kill -HUP <compositor pid>` (`systemctl reload`-shaped tooling works
  without a socket client), which drives the same path with no reply
  channel: the applied/refused summary goes to the compositor log instead.

No file watching: a live-edited config would fire mid-keystroke, while both
triggers above say exactly when. A HUP to the `scootctl` client itself means
nothing — only the compositor installs the handler.

**Applied:** `[layout] gap`, `column_widths` and `default_column_width`
(the arrangement is recomputed and the screen
redrawn; a shorter width list clamps live columns onto the nearest
surviving entry, and new windows take the reloaded default), the `[appearance]` focus-ring width and colors, the background
color, `corner_radius`, `prefer_no_csd`, and the cursor `cursor_size`,
`cursor_color` and `cursor_theme` (the fallback bitmaps are rebuilt, the
theme reloaded, and the screen redrawn without re-arranging -- cursor
pixels are not placement), and the whole `[binds]` table (rebuilt from the
defaults plus the file, so a reload both adds and overrides binds).

**Refused, explicitly:**
`[output] scale`
(clients were told it at bind time), `[tty] gpu` (the session already
drives its device), `[renderer] backend` (the live renderer holds client
textures), and `[autostart] commands` (entries run once, at session start —
a reload never re-runs them).

The reply says which was which:

```json
{ "type": "reloaded", "applied": ["layout.gap", "binds"],
  "refused": ["output.scale (startup-only: clients were told the scale at bind time)"] }
```

Both lists name only fields that *differed* — a field the file and the
session agree on appears in neither, so two empty lists together mean "the
reload changed nothing it was asked to". A reload that cannot load or
validate the file at all (unreadable, malformed TOML, an unknown field)
answers an `error` instead, keeps the running config untouched, and logs —
never defaults, never a half-applied session, never an exit. `scootctl`
exits non-zero on that error like any other.

Two guarantees the applied set pins:

- A key held across a `[binds]` rebuild neither wedges nor drops: release
  routing follows what the press decided, not what the table says now.
- Under `--tty`, a reload cannot strip the `Ctrl+Alt+F1..F12` VT-switch
  recovery bindings — they are layered back on last, overriding any
  colliding file bind with a warning, exactly as at startup. On
  `--headless`/`--nested` a reload gains no VT bindings either; there is no
  VT to switch to there.

A reload applies while the session is locked: nothing in the applied set
can disclose locked content (appearance changes touch nothing the locked
frame draws; gap, column widths and binds are input-side -- widths only
re-derive column frames from config proportions -- and binds cannot fire actions
while locked anyway).

## `[layout]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `gap` | integer (pixels) | `12` | Gap between columns, between windows stacked in a column, and at output edges. Clamped into `0..=10000`: negatives become `0`, and anything above `10000` becomes `10000` — already wider than the long edge of an 8K display, and it keeps the layout's own integer arithmetic well away from overflow. A gap that large leaves no usable area, so windows end up 1x1; it's a guard against a typo or a probe, not a usable setting. Re-applied live by `scootctl reload`. |
| `column_widths` | array of floats | `[0.333…, 0.5, 0.666…]` (i.e. `1/3`, `1/2`, `2/3`) | Column widths as fractions of the output width, in the order `cycle-column-width` steps through and `set-column-width N` indexes into (0-based). Non-finite or non-positive entries are dropped; an empty list falls back to the built-in three. Re-applied live by `scootctl reload` (a shorter list clamps live columns onto the nearest surviving entry; see [Reloading the config](#reloading-the-config)). |
| `default_column_width` | integer (unsigned) | `1` | Index into `column_widths` used for newly created columns (`1` selects `0.5`, i.e. half the output). Too large is clamped to the last valid index; negative isn't a valid value for this field at all, so it's a whole-file parse error, not a clamp. Re-applied live by `scootctl reload` (new windows take the reloaded default; live columns hold still). |

## `[appearance]`

scoot draws no titlebars by design — a focused window gets a colored ring
drawn *around* it (in the layout's own gap), and there's a solid background
behind everything. That's why there's no titlebar-color/font option below:
this table controls the ring, the background, and the built-in pointer
cursor.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `focus_ring_width` | integer (pixels) | `3` | Ring thickness. Clamped at load time to at most half of `gap`, so it can never visually reach a neighboring window. Re-applied live by `scootctl reload` (re-clamped against the reloaded gap). |
| `focus_ring_active_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#6ba6fa` (accent blue) | Ring color around the focused window. Re-applied live. |
| `focus_ring_inactive_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#595961` (muted gray) | Ring color around every other window. Re-applied live. |
| `background_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#141419` (near-black, pixel-sampled under pixman) | Cleared behind all window content — there's no separate background render element, this is the frame clear color. Re-applied live. |
| `corner_radius` | integer (pixels) | `0` | Window corner radius in logical pixels; `0` is square. Rounds the window content (including its subsurfaces) and the focus ring together — a square ring around a rounded window would be worse than none — so whatever is behind a window shows through its corners. A negative value warns and becomes `0`, at startup and on `scootctl reload` (both validate the file through the same path). The effective radius is clamped per window to half its smaller dimension, so an absurd value rounds small windows into stadiums rather than breaking. Popups stay square (whether menus round too is a separate decision). Re-applied live. Costs a little per frame when non-zero (a few dozen extra composite ops per window, plus the opacity loss where corners reveal what is below — measured on the dev VM at ~+9% per frame on a three-window session under pixman, ~+30% under software GLES; the default `0` costs nothing). |
| `cursor_size` | integer (pixels) | `16` | Both dimensions of the built-in pointer cursor. Clamped into `4..=256`: under `4` the shape is left with at most one interior pixel (none at all below 3), and a pointer that small is indistinguishable from a dead pixel; over `256` it covers a quarter of a 1080p display's height and the bitmap it allocates stops being small. A value outside `i32` altogether (or a float) is a whole-file parse error, not a clamp. Re-applied live by `scootctl reload` (the fallback bitmaps are rebuilt; drawn only under `--tty`). |
| `cursor_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#ffffff` (white) | Fill color of the built-in pointer cursor. Its 1px outline is always black, at this color's own alpha, and isn't separately configurable — the outline exists to keep the shape's edges visible against similarly-colored content. That doesn't help against a *dark* `cursor_color`: with a near-black fill, the outline blends into it and the pointer can be hard to spot against dark window content. An alpha of `00` makes the built-in cursor invisible; that's your call, not a clamped value. Re-applied live by `scootctl reload` (drawn only under `--tty`). |
| `cursor_theme` | string | unset | Which installed xcursor theme named cursor shapes are drawn from (see [protocols.md](protocols.md#cursor-shapes-wp-cursor-shape-v1)). Unset means follow `$XCURSOR_THEME`, then `default` — i.e. whatever the rest of the desktop uses; an empty string means the same as unset. This only *names* a theme, it never makes scoot ship one, and a name that matches nothing installed is not an error: named shapes then come from scoot's own drawn set. Re-applied live by `scootctl reload` (the theme is reloaded and future children inherit the new `XCURSOR_THEME`/`XCURSOR_SIZE`; drawn only under `--tty`). Reloading back to unset keeps the startup-resolved name rather than re-reading the outer environment (startup overwrites `$XCURSOR_THEME` once) — self-consistent, and only a restart picks up an externally changed variable. |
| `prefer_no_csd` | boolean | `true` | Whether to answer a client's `zxdg_toplevel_decoration_v1` request with `ServerSide`, so a well-behaved client stops drawing its own titlebar (which would otherwise double up with the ring). Re-applied live (answers future requests; repaints nothing). |

`cursor_size` and `cursor_color` apply to scoot's own drawn shapes — the
fallback used when the machine has no cursor theme installed, drawn only
under `--tty`. `cursor_size` also picks which size is taken out of a real
theme's file, and `cursor_color` has no effect there: a theme's artwork
brings its own colors. None of the three affects a client that supplies its
own cursor *image*.

The first three hex values above are the actual rendered colors
(pixel-sampled from a real screenshot under pixman, the default renderer;
pasting one back reproduces the default within 1 LSB — the pixman and GLES
paths disagree by that much on backgrounds, so no hex string is pixel-exact
everywhere). Internally those three
built-in defaults are stored as raw RGBA floats (`0.42, 0.65, 0.98`, `0.35,
0.35, 0.38`, and `0.08, 0.08, 0.1`, each `1.0` alpha), and none of those
floats is exactly representable as an 8-bit `"#rrggbb"` string. Leave a color
field unset to get the real built-in default; only set it to a hex string if
you want to *change* it. (`cursor_color`'s `#ffffff` is the one exception:
pure white *is* exactly representable.)

## `[output]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `scale` | float | `1.0` | Output scale advertised to clients and rendered at. `1.0` renders identically to no setting at all; anything else advertises `ceil(scale)` on `wl_output` and `wl_surface.preferred_buffer_scale`, and the exact value through `wp_fractional_scale_v1`/`wp_viewporter` (see [protocols.md](protocols.md#output-scaling)). Clamped into `0.5..=4.0` with a warning, and a non-finite value falls back to `1.0`; startup-only — a reload refuses changes (see [Reloading the config](#reloading-the-config)). `--nested` ignores a non-1.0 value with a warning, since the host owns the scale of the window scoot draws inside. |

## `[renderer]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `backend` | string (`"pixman"` or `"gles"`) | `"pixman"` | Which renderer composites each frame — the config-file form of `--renderer` (see [tty.md](tty.md#which-renderer-draws-the-frames)). `"pixman"` is the CPU renderer and needs no graphics device at all. `"gles"` draws with GLES on an EGL device; under `--tty` it needs a `--features gpu-scanout` build, where it scans out from the GPU, and warns and keeps pixman without one. `--renderer` wins when both name one, including `--renderer pixman` against a file asking for `gles`. A name that is neither is a warning and the default, like any other malformed value; but a name this build *knows* and then cannot build (`"gles"` with no working EGL) is a startup error — the second deliberate one — because silently drawing with the other renderer would be a session quietly different from the one you asked for. Startup-only — a reload refuses changes. |

## `[tty]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `gpu` | string (device path) | unset | Which DRM device `--tty` drives, when the automatic choice is wrong — the config-file form of `--gpu PATH` (see [tty.md](tty.md#which-drm-device---tty-drives)). Unset means the automatic search picks: Smithay's primary GPU first, then every other DRM device on the seat until one works. Set means exactly that device, no fallback: a wrong path is a clean startup error naming the key and what failed, so this key is fail-closed where every other config field degrades gracefully. `--gpu` wins when both name one; an empty value (`gpu = ""`) is a startup error naming the key, on every backend. Only means anything under `--tty`; on `--headless` or `--nested` a set non-empty value is ignored with a warning. Startup-only — a reload refuses changes. |

## `[binds]`

A table of `"key combination" = "action string"`. A combo is
`modifier+modifier+...+key` (e.g. `"super+shift+h"`), or a bare key with no
modifier at all (e.g. `"Return" = "close"` — legal, and it intercepts every
press of that key with none of Super/Shift/Ctrl/Alt held; `Shift+Return`, for
instance, still reaches the focused client normally). Whitespace around `+`
is ignored.

Modifier names are case-insensitive: `ctrl`/`control`, `shift`, `alt`, and
`super`/`logo`/`meta`/`cmd` (all four spellings mean the same modifier —
scoot's own tables and these docs call it "Super"). The key is an xkb keysym
name (`h`, `Return`, `F5`, ...), resolved by trying the name exactly as
written first and then case-insensitively — so `"return"` and `"RETURN"` both
find `Return`. A single ASCII letter is folded to lowercase before that
lookup, so `"H"` and `"h"` name the same key (only single letters fold:
`"OE"` and `"oe"` are distinct keysyms and stay that way).

Binds are matched against a key's *unshifted* symbol, with Shift tracked as
an ordinary modifier, so **a capital letter names the unshifted key, not
Shift plus that key**: `"A"` means plain `a`, exactly like `"a"` — write
`"shift+a"` for the Shift chord. Folding a bare capital logs a warning naming
the bind; with Shift named there is nothing ambiguous, so `"shift+A"` folds
quietly. Note `scootctl key A` is a different story on purpose: it keeps
refusing, because pressing `A` with nothing held would type a different
character.

Action strings use the grammar in [ipc.md](ipc.md#actions) — one parser
handles `scootctl action ...` (and its `scoot msg` alias) and a config file's
`[binds]` values:

```
focus-column|move-column|consume-or-expel   left|right
focus-window|move-window                    up|down
focus-workspace|move-window-to-workspace    up|down
focus-window-id ID | focus-workspace-index N | move-window-to-workspace-index N | focus-output ID | move-window-to-output ID | cycle-column-width | set-column-width N | close | spawn COMMAND... | quit
```

e.g. `"focus-column left"`, `"close"`, or `"spawn foot -e htop"` (split on
whitespace, not run through a shell, so a path or argument containing a space
can't be expressed this way).

### Moving across outputs

With more than one output, two actions reach across screens — and the first
two outputs have default binds, promoted from the manual example (bare Super
focuses, Shift carries the focused window there, the same split the
workspace digits keep):

```toml
[binds]
"super+comma" = "focus-output 1"
"super+period" = "focus-output 2"
"super+shift+comma" = "move-window-to-output 1"
"super+shift+period" = "move-window-to-output 2"
```

Those four lines are the defaults: uncommenting them in a file printed by
`scoot --print-default-config` changes nothing. Outputs 3 and up stay
manual — add your own binds naming those ids, in the same form.

The ids are what `scootctl outputs` reports (stable for the session, unlike
workspace positions). `move-window-to-output` carries the focused window to
that output's active workspace and follows it there; `focus-output` focuses
that output's active workspace (or nothing, when it holds no windows). An
unknown id does nothing. Both are refused while the session is locked, like
every other action.

Two failure behaviors specific to `[binds]`, both worth knowing since they
fail silently rather than as a startup error:

- **A bind that doesn't parse** (unknown modifier, unknown key name, an
  invalid action string, or trailing text after the action) is skipped with a
  warning naming just that bind; every other bind in the file still loads.
- **Two different combo strings that resolve to the same actual key
  combination** — different modifier aliases, different modifier order, or
  different case (`"Super+H"` vs `"super+h"`) — are **both** skipped, with one
  warning naming all of them. `[binds]` is read into a `HashMap`, whose
  iteration order has no relationship to the order the keys were written in
  the file, so "the last one wins" isn't something this code can honor
  truthfully; rather than pick an arbitrary, run-to-run-unstable winner, the
  whole colliding group is dropped and whatever was bound to that combo
  before the file loaded is left in place.

A user bind on a combo that already has a default simply replaces it; there
is no "unbind" action. `scootctl reload` rebuilds the whole table from the
defaults plus the file, so a reload both adds and overrides binds — and a
bind removed from the file falls back to its default (or to unbound, if it
never had one).

## `[autostart]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `commands` | array of strings | `[]` | Action strings to run once each, in file order, at session startup — before the `--` command (see [Starting a session](#starting-a-session)). |

Each entry is an action string in the grammar in [ipc.md](ipc.md#actions) —
one parser handles `scootctl action ...` (and its `scoot msg` alias), a
config file's `[binds]` values, and these entries:

```toml
[autostart]
commands = [
    "spawn waybar",
    "spawn mako",
]
```

Deliberately the *action* grammar, not a bare argv list: every string here
is something `scoot msg action` would accept and a `[binds]` value could
contain, which is what keeps the config agent-legible. There is no
spawn-only restriction — a non-`spawn` action at startup (say,
`"focus-workspace-index 2"`) is the user's choice, documented as such; it
runs through the same `act` path a keybind or IPC request would take.
(`"quit"` is in this class too: it ends the session cleanly before the `--`
command runs, so an autostart list containing it is a session that starts
and immediately exits.)

Three behaviors worth knowing:

- **Fail-open per entry.** A malformed entry (an unknown action, a missing
  argument, trailing text after the action) is skipped with a warning naming
  just that entry; every other entry still runs, the rest of the file still
  applies, and the session always starts. On `--tty` scoot *is* the session,
  so a typo must never cost it — the same rule `[binds]` follows (see
  [Failure semantics](#failure-semantics)).
- **Ordering.** Entries run first, in file order, then the `--` command.
  Spawning the same bar in both places yields two bars — the same class as
  two `spawn` binds, the user's composition to fix.
- **No supervision.** An entry that exits instantly is reaped, not restarted;
  telling a deliberate quit from a crash loop is a service manager's job,
  not the compositor's.
- **Never re-run.** Entries run once, at session start. `scootctl reload`
  refuses `autostart.commands` changes with a message rather than running
  anything a second time (see [Reloading the config](#reloading-the-config)).

## Default keybindings

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
| `Super+1`..`Super+9` | Focus workspace 1–9 directly (index 0–8) |
| `Super+Shift+1`..`Super+Shift+9` | Move window to workspace 1–9 directly (index 0–8) |
| `Super+comma` / `Super+period` | Focus output 1 / 2 |
| `Super+Shift+comma` / `Super+Shift+period` | Move window to output 1 / 2 directly |
| `Super+r` | Cycle column width |
| `Super+q` | Close focused window |
| `Super+Return` | Spawn `foot` |
| `Super+Shift+e` | Quit |

All 40 of them — vim motions (`h`/`j`/`k`/`l`) for direction, Super as
scoot's own modifier throughout. Quit is deliberately `Super+Shift+e`, not
`Super+Shift+q`: that combo is one slipped Shift away from `Super+q` (close
focused window), and a slip of the finger shouldn't be able to end the whole
session.

Targeting a workspace that doesn't exist yet — `Super+9` with only three
workspaces open, or either index action with a stale number — does nothing:
it neither creates a workspace nor falls back to the last one. (The
workspace set is dynamic: empty workspaces are dropped, so an index only
means anything against the list it was read from.)

`set-column-width N` lands the focused column directly on entry `N` of
`[layout] column_widths` (0-based) instead of stepping past every other
entry like `cycle-column-width` — the closest thing to "fullscreen" scoot
has. An index past the end of the list does nothing, like a stale workspace
index. It has no default bind, by decision: the width list is yours to size,
so no key family maps onto it the way digits map onto workspaces (the same
call phase F made for the output actions above). Add your own, e.g. with a
`1.0` entry in the list:

```toml
[layout]
column_widths = [0.3333333333333333, 0.5, 1.0]

[binds]
"super+f" = "set-column-width 2"
```

`--tty` additionally binds `Ctrl+Alt+F1` through `Ctrl+Alt+F12` to VT
switching — not present under `--headless`/`--nested`, since VT switching is
a Linux-session concept with no meaning there. See
[tty.md](tty.md#vt-switching) for why they always win over a config-file
bind — at startup and on every `scootctl reload`, which layers them back on
last rather than letting a reloaded file strip the recovery path.

Moving a window across outputs, or focusing another output, is bound by
default for outputs 1 and 2 (`Super+comma`/`Super+period` and Shift for the
carry — see [Moving across outputs](#moving-across-outputs)); outputs 3 and
up are config binds you add yourself.

## Example `config.toml`

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
# panel, or text and widgets render far too small.
scale = 1.0

# [renderer]
# Unset means "pixman", the CPU renderer -- the right answer on a GPU-less
# box and the default everywhere. "gles" is opt-in; under --tty it scans out
# from the GPU in a --features gpu-scanout build,
# and buys correctness parity rather than speed today. --renderer wins over
# this when both name one. See docs/tty.md before switching.
# backend = "gles"

# [tty]
# Uncomment only on hardware where the automatic DRM device search picks
# wrong. Apple Silicon under Asahi Linux -- where the 3D GPU and the display
# controller are separate devices -- was the motivating case, but the search
# was confirmed correct there on 2026-09-18 and needs no key. Unset means the
# automatic search picks; --gpu PATH on the command line wins over this when
# both name one. Name the *display controller*, never the render node, and
# prefer a stable /dev/dri/by-path/... alias over a cardN minor number.
# gpu = "/dev/dri/by-path/platform-soc:display-subsystem-card"

[binds]
"super+n" = "focus-column right"
"super+shift+n" = "move-column right"
"super+t" = "spawn foot"
"super+shift+t" = "spawn foot -e htop"
"ctrl+alt+space" = "spawn wofi --show drun"

[autostart]
commands = [
    "spawn waybar",
    "spawn mako",
]
```
