# Configuration reference

- [Command-line flags](#command-line-flags)
- [The config file](#the-config-file)
- [Reloading the config](#reloading-the-config)
- [`[layout]`](#layout) · [`[appearance]`](#appearance) · [`[output]`](#output) · [`[[outputs]]`](#outputs) · [`[renderer]`](#renderer) · [`[tty]`](#tty) · [`[xwayland]`](#xwayland) · [`[autostart]`](#autostart) · [`[floating]`](#floating) · [`[[window_rule]]`](#window_rule) · [`[wallpaper]`](#wallpaper) · [`[binds]`](#binds)
- [Default keybindings](#default-keybindings)
- [Example `config.toml`](#example-configtoml)

## Command-line flags

```
scoot --headless [--width 1-65535] [--height 1-65535] [--outputs 1-8] [--renderer pixman|gles] [--xwayland] [--socket PATH] [--config PATH] [-- COMMAND...]
scoot --nested   [--width 1-65535] [--height 1-65535] [--renderer pixman|gles] [--xwayland] [--socket PATH] [--config PATH] [-- COMMAND...]
scoot --tty      [--gpu PATH] [--mode WxH] [--renderer pixman|gles] [--xwayland] [--socket PATH] [--config PATH] [-- COMMAND...]
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
| `--width N`, `--height N` | The `--headless`/`--nested` output size, 1–65535 per axis (default 1600x1000) — whatever DRM itself can report for a mode (`drm_mode_modeinfo` stores each axis in a `u16`), with room to spare past any real display. Anything else is a startup error naming the flag and the expected range (``invalid --width: `70000` (expected 1-65535)``), not a silently different size. Under `--nested` it is only the size scoot *asks* for: the host's first configure decides what the window comes up at, and every later one moves it. Under `--headless` it is the size for the whole session. An [`[[outputs]]`](#outputs) entry's `mode` overrides it for the output it names. |
| `--outputs N` | How many outputs `--headless` creates, 1–8 (default 1). Each is `--width` by `--height` and sits immediately right of the last, so two 1280-wide outputs at scale 1 cover x 0–1279 and 1280–2559. Out of range is a startup error naming the range, like `--width`. `--headless` only: `--nested` presents one window in its host and `--tty` drives one CRTC, so both warn and ignore it. See [More than one output](#more-than-one-output) for what a second output does and does not do yet. |
| `--renderer pixman\|gles` | Which renderer composites each frame. Config-file form: `[renderer] backend`. See [tty.md](tty.md#which-renderer-draws-the-frames). |
| `--gpu PATH` | Which DRM device `--tty` drives. Config-file form: `[tty] gpu`. Ignored with a warning outside `--tty`. See [tty.md](tty.md#which-drm-device---tty-drives). |
| `--mode WxH` | Which connector mode `--tty` picks. Ignored with a warning outside `--tty`. An [`[[outputs]]`](#outputs) entry's `mode` overrides it for the connector it names. |
| `--xwayland` | Run an XWayland server inside the session, so X11-only applications get a `DISPLAY` to connect to. Config-file form: `[xwayland] enabled` (either one turns it on). Opt-in and off by default — the server costs a whole extra process (~55 MB RSS idle, ~87 MB with a few X clients) plus a `Xwayland` binary on `PATH`, and any X client can keylog/snoop by design (see [protocols.md](protocols.md#xwayland-opt-in)). Needs an `xwayland` build; without one it warns and the session runs Wayland-only. X windows map into the layout like any other (dialogs float, `[[window_rule]]`s match their `WM_CLASS` class), and take focus by themselves only when nothing is focused, when they belong to the focused X app, or when scoot started them. |
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
action. `$DISPLAY` joins them while the session's XWayland server is
believed live (`--xwayland` / `[xwayland] enabled` — see
[`[xwayland]`](#xwayland)); otherwise `DISPLAY` is left untouched, so a
host-provided value under `--nested` survives. While it is live, the child
also gets the same activation token as `$DESKTOP_STARTUP_ID` (an inherited
one is removed, like `$XDG_ACTIVATION_TOKEN`): the variable X toolkits turn
into `_NET_STARTUP_ID`, which is how an X app scoot started takes focus when
it maps. While that child runs, only it, or a process it starts, can
redeem it that way: another X client that copies the startup id is
refused. Once it has exited (an app handing over to an already-running
instance, or forking into the background), any X window naming the id may
redeem it, once, as before (see
[protocols.md](protocols.md#focus-x-windows-ask-scoot-decides)).

### Portals and the D-Bus activation environment

`$XDG_CURRENT_DESKTOP=scoot` above is what lets
[xdg-desktop-portal](https://github.com/flatpak/xdg-desktop-portal) pick a
backend for this session — without it a browser has no screen sharing on
Wayland and degraded file choosers. But setting it on scoot's children is
only half: the portal itself is D-Bus activated, so it inherits the **D-Bus
activation environment**, not the environment of whatever client asked.
That half belongs outside the compositor (config is state, the script is
behavior — scoot itself never touches the bus):

- **A systemd session** (a greeter entry running `scoot-session`, i.e.
  the NixOS module's `programs.scoot.session.enable` default — see
  [nix.md](nix.md#what-a-greeter-login-starts)): the launcher owns this
  import. It brings the login environment into the user manager, waits
  for the compositor to answer IPC, runs the equivalent of the two
  lines below for `WAYLAND_DISPLAY` and `XDG_CURRENT_DESKTOP`, and
  only then starts `scoot-session.target` (which pulls in
  `graphical-session.target`, so the target is reached with the
  display already imported) — no session-script line needed.
- **Anything else** (s6, a seat with no user manager, the webtop
  target): the session script does it:

```sh
# systemd session started by hand (a session script, not the launcher):
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
- its own windows, and only its own: a window is drawn and takes the
  pointer only on the output it is on, so a column scrolled part-way past
  the shared edge (or a fullscreen window its column is focused away from)
  is cut at that edge rather than drawn over — and clicked instead of — the
  neighbouring output's windows. Its focus ring and rounded corners are
  drawn on its own output too;
- its own menus: a popup belongs to its window's output like the window
  does, and a menu that would cross the shared edge is flipped or slid back
  onto its window's output when it lets the compositor adjust it (toolkit
  menus do) — the same as at the output's outer edges. One that asks for no
  adjustment is cut at the edge. See
  [protocols.md](protocols.md#popup-menus-xdg_popup);
- a window list that names the output each window is on
  (`wlr-foreign-toplevel-management` `output_enter` per window's own output,
  paired with `output_leave` when a move carries it across);
- its own composited strip: every output has a render target of its own and
  the render loop draws each one, so `scootctl screenshot --output 2` answers
  with the second output's own pixels, a screen capture of it (`grim -o
  headless-2`) reads its own framebuffer, a gamma control names it
  specifically, and its layer surfaces get frame callbacks at its own
  cadence. Each output is `--width` by `--height`, like the first, unless an
  [`[[outputs]]`](#outputs) entry gives it a `mode` of its own;
- its own scale, when an [`[[outputs]]`](#outputs) entry gives it one: its
  `wl_output.scale`, its logical size (a 1600x1000 output at 2 is 800x500
  logical pixels, and the next output starts at its right edge), and the
  scale every surface on it is told, windows included -- a window carried
  to an output at another scale is re-told that output's scale;
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

Under `--tty` every connected monitor is an output like these. It is named
after its connector and placed left to right at its own mode (in connector
order at startup, a later one on the right), and it is added or removed
when a monitor is plugged in or pulled out
(see [tty.md](tty.md#more-than-one-monitor)).

What it is **not**, yet, is tracked in
`docs/backlog/core/multi-output-remainder.md`:

- new windows open on the output under the pointer (falling back to the
  first output when the pointer is over no output); stepping across
  outputs, and carrying a window along, is bound by default
  (`Super+comma`/`Super+period`, wrapping left and right — see [Moving
  across outputs](#moving-across-outputs)) — fixed screens stay manual
  (`focus-output-index N`, or an id);
- no position setting: outputs line up left to right. Scale and mode are
  set per output in the file ([`[[outputs]]`](#outputs)), not from
  `wlr-randr` or a settings app -- `wlr-output-management` `apply`/`test`
  stay refused;

A menu left open while its window scrolls, or while an output changes, is
re-fitted to the new position when it asked `reactive`, and told with a
fresh configure pair (a non-reactive menu is never re-configured — the
protocol forbids it; see
`docs/backlog/resolved/popup-reactive-reconstrain-done.md`).

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
runs on built-in defaults if neither exists. Ten optional tables:
`[layout]`, `[appearance]`, `[output]`, `[renderer]`, `[tty]`,
`[xwayland]`, `[autostart]`, `[floating]`, `[wallpaper]`, `[binds]` — plus
any number of `[[window_rule]]` entries (an array of tables).
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

**Most settings are read once at startup; most can be reloaded live.**
`scootctl reload` (see [Reloading the config](#reloading-the-config))
re-reads this same file and re-applies the layout (gap, column widths and
the default column width), the output scale (the default and each
`[[outputs]]` entry's), the appearance (including
the cursor size, color and theme), the
keybindings, `[floating]` and the `[[window_rule]]`s (for windows that map
after the reload), new `[autostart]` spawn entries, and `[wallpaper]`
(handed to scootbg again). Only `[tty] gpu`,
`[renderer] backend`, `[xwayland] enabled` and an `[[outputs]]` entry's
`mode` need a restart, and a reload refuses them with a message naming
that rather than silently ignoring them.

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
- One bad `[appearance]` color string, one bad `[binds]` entry, one bad
  `[autostart]` entry, or one unusable `[[window_rule]]` (no matcher, a
  size that is not positive, ...): logged as a warning, and only that
  field/bind/entry/rule falls back — every other field, bind and rule in the
  file still applies.
- Anything wrong inside `[wallpaper]` (an unknown key, a value of the wrong
  type, a path that cannot be resolved; nesting more than 16 levels deep is
  the exception, a whole-file parse error like the case above): logged as
  an error, and only the
  wallpaper is skipped — the rest of the file applies. An unknown key
  anywhere else still discards the whole file, as above.
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
surviving entry, and new windows take the reloaded default), the `[output] scale`
and each [`[[outputs]]`](#outputs) entry's `scale` (every output's scale is
re-decided -- its entry's, else the default -- and every output re-advertises its
mode and scale on `wl_output` (clients bound to an output whose scale did not
move hear the same values again); the fractional value and
its integer companion are re-sent to every live surface, each at its own
output's scale; the logical geometry is recomputed and the arrangement
re-derived -- clients that cached the scale may lag until they re-read the
events. Reported as `output.scale` for the default and
`outputs.<name>.scale` for an entry, including one for a monitor that is
not plugged in, which is stored and applies when it is. Under `--nested`
any change is refused, since the host owns the scale), the `[appearance]` focus-ring widths and colors, the background
color, `corner_radius`, `prefer_no_csd`, and the cursor `cursor_size`,
`cursor_color`, `cursor_theme` (the fallback bitmaps are rebuilt, the
theme reloaded, and the screen redrawn without re-arranging -- cursor
pixels are not placement) and `cursor_hide_after_ms` (the hide deadline is
re-derived at once, not on the next layout change), and the whole `[binds]` table (rebuilt from the
defaults plus the file, so a reload both adds and overrides binds) -- plus
new `[autostart]` spawn entries (each entry the session has not seen yet
runs once, in file order, through the same path startup drains; a reloaded
entry that is not a `spawn` -- `quit` included -- is refused by name and
never acted on; a `spawn` whose program fails to start is refused by name
instead of reported applied, and stays pending -- the next reload retries
it. Seen is by value per occurrence: an edited entry counts as
new, a removed-then-re-added entry runs again, and a second identical
reload is silent), `[floating] modifier` (the next drag uses it; a value
that is not a modifier is refused by name), and
`[floating] auto` and the `[[window_rule]]` list
(swapped whole; they decide for windows that map after the reload, and
windows already mapped keep their place -- a rule that cannot be used is
refused by name, e.g. `window_rule #3 (sets neither float nor size): skipped
as unusable`, on every reload that finds it), `[xwayland] fractional`
(reported as `xwayland.fractional`: the X scale is re-chosen through the
one chooser, so the client scale, the XSETTINGS toolkits read and every
open X window's configure follow when the choice moves it, and nothing is
sent when it does not; a value that names neither `sharp` nor `light` is
refused by name), and `[wallpaper]` (handed to
scootbg on every reload while the section exists, `{}` on the reload that
removes it; reported as `wallpaper` when its values changed and
`wallpaper.command` when `command` changed, either or both, and refused
by name, keeping and re-running the running section, when it has a
problem -- see [`[wallpaper]`](#wallpaper)).

**Refused, explicitly, pending a restart:**
`[tty] gpu` (the session already
drives its device), `[renderer] backend` (the live renderer holds client
textures), `[xwayland] enabled` (the X server starts once at startup, or
never) and an `[[outputs]]` entry's `mode` (`outputs.<name>.mode`: a reload
does not modeset a running monitor; the session keeps the mode it started
with, which is also the mode that monitor comes back at when replugged) --
all four take effect on restart, and each refusal says so.

The reply says which was which:

```json
{ "type": "reloaded", "applied": ["layout.gap", "binds"],
  "refused": ["tty.gpu (takes effect on restart: the session already drives its device)"] }
```

Both lists name only fields that *differed* — a field the file and the
session agree on appears in neither, so two empty lists together mean "the
reload changed nothing it was asked to". Three exceptions are refused on
every reload that finds them, changed or not, because none is ever in
effect, so each always differs from what the file asks for: an unusable
`[[window_rule]]`, a `[wallpaper]` section with a problem (an unknown
key or a value of the wrong type inside it, a path that cannot be
resolved), and an `[[outputs]]` `mode` that differs from the one the
session started with. A reload that cannot load or validate the file at all
(unreadable, malformed TOML, an unknown field anywhere but inside
`[wallpaper]`, a value nested absurdly deep) answers an `error` instead,
keeps the running config untouched, and logs — never defaults, never a
half-applied session, never an exit. `scootctl` exits non-zero on that
error like any other. An unknown field inside `[wallpaper]` is not that
case: it is the refusal above, and the rest of the file applies.

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
while locked anyway; a scale change only re-derives the same geometry the
lock path already publishes and re-sends config-derived scale values to
surfaces still showing the blanked frame). New autostart entries are the
one thing a locked reload skips: a spawned program at lock time could
disclose a window onto, or interfere with, the locked session, so the
reload refuses the field as skipped-while-locked and decides the still-pending
entries on the first unlocked reload instead -- new spawns run, non-spawns
are refused by name; deferred, not denied.

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

(Under home-manager with Stylix, `programs.scoot.stylix.enable` defaults
the ring and background colors, the cursor theme and size from the scheme —
see [`docs/nix.md`](nix.md#stylix). A value written here always wins.)

| Field | Type | Default | Meaning |
|---|---|---|---|
| `focus_ring_width` | integer (pixels) | `3` | Ring thickness. The ring is drawn around what the window actually draws: normally its whole slot in the layout, but a window that draws less than its slot (a fixed-size dialog, a video player keeping its own size) is ringed where its content ends, not around the empty part of the slot. Clamped at load time to at most half of `gap`, so it can never visually reach a neighboring window. Re-applied live by `scootctl reload` (re-clamped against the reloaded gap). |
| `focus_ring_inactive_width` | integer (pixels) | unset (same as `focus_ring_width`) | Ring thickness around every window that is not focused, so the unfocused rings can be thinner (or thicker) than the focused one. Unset means the same as `focus_ring_width`, so a config that never sets it renders exactly as before; `0` draws no ring at all on unfocused windows. Clamped at load time to at most half of `gap`, with a warning, like `focus_ring_width`; a negative value draws no ring, also like it. A ring changes thickness with focus: the window losing focus shrinks its ring to this width and the one gaining it grows to `focus_ring_width`, and with `corner_radius` set each ring's outer arc stays concentric with its window's corner (radius plus *that window's* ring width). Re-applied live by `scootctl reload`, which also redraws. |
| `focus_ring_active_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#6ba6fa` (accent blue) | Ring color around the focused window. Re-applied live. |
| `focus_ring_inactive_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#595961` (muted gray) | Ring color around every other window. Re-applied live. |
| `background_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#141419` (near-black, pixel-sampled under pixman) | Cleared behind all window content — there's no separate background render element, this is the frame clear color. Re-applied live. |
| `corner_radius` | integer (pixels) | `0` | Window corner radius in logical pixels; `0` is square. Rounds the window content (including its subsurfaces) and the focus ring together — a square ring around a rounded window would be worse than none — so whatever is behind a window shows through its corners. Both follow what the window actually draws, so a window smaller than its slot has its own corners rounded, and the ring's outer edge is an arc concentric with the window's corner (radius plus the window's own ring width, `focus_ring_width` or `focus_ring_inactive_width` by focus). A client that rounds its own corners more than this (libadwaita dialogs) gets a backdrop in its ring color under its drawn rect instead, so its corners read as the ring hugging its own curve rather than a crescent of background. See [protocols.md](protocols.md#tiled-windows). A negative value warns and becomes `0`, at startup and on `scootctl reload` (both validate the file through the same path). The effective radius is clamped per window to half its smaller dimension, so an absurd value rounds small windows into stadiums rather than breaking. Popups stay square (whether menus round too is a separate decision). Re-applied live. Costs a little per frame when non-zero (a few dozen extra composite ops per window, plus the opacity loss where corners reveal what is below — measured on the dev VM at ~+9% per frame on a three-window session under pixman, ~+30% under software GLES; the default `0` costs nothing). |
| `cursor_size` | integer (pixels) | `16` | Both dimensions of the built-in pointer cursor. Clamped into `4..=256`: under `4` the shape is left with at most one interior pixel (none at all below 3), and a pointer that small is indistinguishable from a dead pixel; over `256` it covers a quarter of a 1080p display's height and the bitmap it allocates stops being small. A value outside `i32` altogether (or a float) is a whole-file parse error, not a clamp. Re-applied live by `scootctl reload` (the fallback bitmaps are rebuilt; drawn only under `--tty`). |
| `cursor_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#ffffff` (white) | Fill color of the built-in pointer cursor. Its 1px outline is always black, at this color's own alpha, and isn't separately configurable — the outline exists to keep the shape's edges visible against similarly-colored content. That doesn't help against a *dark* `cursor_color`: with a near-black fill, the outline blends into it and the pointer can be hard to spot against dark window content. An alpha of `00` makes the built-in cursor invisible; that's your call, not a clamped value. Re-applied live by `scootctl reload` (drawn only under `--tty`). |
| `cursor_theme` | string | unset | Which installed xcursor theme named cursor shapes are drawn from (see [protocols.md](protocols.md#cursor-shapes-wp-cursor-shape-v1)). Unset means follow `$XCURSOR_THEME`, then `default` — i.e. whatever the rest of the desktop uses; an empty string means the same as unset. This only *names* a theme, it never makes scoot ship one, and a name that matches nothing installed is not an error: named shapes then come from scoot's own drawn set. Re-applied live by `scootctl reload` (the theme is reloaded and future children inherit the new `XCURSOR_THEME`/`XCURSOR_SIZE`; drawn only under `--tty`). Reloading back to unset keeps the startup-resolved name rather than re-reading the outer environment (startup overwrites `$XCURSOR_THEME` once) — self-consistent, and only a restart picks up an externally changed variable. |
| `cursor_hide_after_ms` | integer (milliseconds) | `0` (never hides) | How long the pointer sits still over a fullscreen window covering its output before scoot hides it. `0` disables the feature — a compositor hiding the pointer unasked would be presumptuous, so this is opt-in. While hidden, frames carry no cursor elements, which is what lets a fullscreen window scan out directly on a display with no cursor plane (see [tty.md](tty.md)); the next pointer motion, button press or scroll shows the pointer again, and on a display with no cursor plane frames composite until it has sat still for the delay again. Only arms while the pointer is over the covering window itself (not a popup, a notification, or another output) and never behind the session lock — the lock screen keeps its pointer, and locking shows a hidden one again. Key presses do not reset the wait; tablet motion does, through the same pointer paths a mouse uses. A value past a day warns and clamps to a day. A hidden pointer reads as hidden in screenshots and screen captures, asked for or not — hiding is not the client's own hidden cursor, and the client's image, hotspot and theme choice all survive for the next motion. Re-applied live by `scootctl reload` (the deadline is re-derived at once). |
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
| `scale` | float | `1.0` | Output scale advertised to clients and rendered at. `1.0` renders identically to no setting at all; anything else advertises `ceil(scale)` on `wl_output` and `wl_surface.preferred_buffer_scale`, and the exact value through `wp_fractional_scale_v1`/`wp_viewporter` (see [protocols.md](protocols.md#output-scaling)). Clamped into `0.5..=4.0` with a warning, and a non-finite value falls back to `1.0`; the clamped value is then resolved to the nearest multiple of 1/120 (so `1.33` becomes `160/120`), silently when the value was already in range, the finest scale `wp_fractional_scale_v1` can express — rendering and every advertisement agree on that one value, so a client buffer sized for the announced scale lands one to one. Re-applied live by `scootctl reload` (see [Reloading the config](#reloading-the-config)). The default for every output: an [`[[outputs]]`](#outputs) entry sets one output's own. With `--xwayland`, X apps draw at `ceil(scale)` -- or at `floor(scale)` with [`[xwayland]`](#xwayland) `fractional = "light"` -- when the whole output layout fits X's 32767-pixel coordinate limit at that scale, else at the largest integer scale at which it fits (1 at worst; logged at info when it drops), and toolkits are told it over XSETTINGS, live across a reload or an output being added, removed or resized (see [`[xwayland]`](#xwayland)). At a fractional scale that costs memory, which matters most on pixman/no-GPU machines: each X window's buffers are 4x what they were before X apps followed the scale (a window filling a 1920x1080 output at 1.25 is 1536x864 logical, drawn at 2 into a 3072x1728 buffer of ≈21 MB, held twice -- the X server's pixmap and the shared-memory buffer -- for ≈42 MB, against ≈10.6 MB when X drew at 1); the per-frame composite cost measured flat. `--nested` ignores a non-1.0 value with a warning at startup and refuses it on reload, since the host owns the scale of the window scoot draws inside. |

## `[[outputs]]`

Per-output overrides of `[output] scale` -- and of `--mode` under `--tty`,
or `--width`/`--height` under `--headless` -- one table per output, each
matched by the name `scootctl outputs` lists (`eDP-1`, `DP-1`, `HDMI-A-1`
under `--tty`; `headless`, `headless-2`, ... under `--headless`). An output
without an entry, and every output of a file without any, runs at the
defaults exactly as before. A table with no `name`, or a value of the wrong
type (`scale = "2"`, `mode = 1920`), fails the whole file like any other
malformed key: startup falls back to defaults (binds included) and a reload
reports the error and applies nothing.

```toml
[output]
scale = 1.5            # every output without an entry of its own

[[outputs]]
name = "eDP-1"         # the laptop panel
scale = 2.0

[[outputs]]
name = "DP-1"          # an external monitor
scale = 1.0
mode = "1920x1080"
```

| Field | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | The output this entry is for, matched exactly (case included) against its connector name. The name only, never the position: a monitor unplugged and plugged back in returns under the same connector name but a new output id and possibly a new place in the order. An entry for a monitor that is not connected waits for it and applies when it is plugged in. |
| `scale` | float | `[output] scale` | This output's scale, resolved exactly like `[output] scale` (clamped into `0.5..=4.0` with a warning, then to the nearest 1/120). A value that is not a finite number is ignored with a warning and the output keeps `[output] scale`. Re-applied live by `scootctl reload`. |
| `mode` | `"WxH"` | `--mode`, or `--width`/`--height` | Under `--tty`, which connector mode to drive -- the same choice `--mode` makes, per connector, falling back to the connector's preferred mode with a warning when it offers no mode of that size. Under `--headless`, that output's size. Beats `--mode`/`--width`/`--height` for this output (the flag is the default for every output; the entry names one). Read at startup and whenever the monitor is plugged in, not on reload: a reload refuses a changed `mode` by name. A value that is not `WxH` with both sides 1-65535 is ignored with a warning. |

An entry with an empty `name`, a second entry for a name already seen (the
first wins), and an entry left with nothing to set are skipped with a
warning; an unknown key inside an entry (a misspelt `scale`) is an unknown
field like any other, and the whole file falls back to defaults at startup
(a reload refuses it). `--nested` ignores every entry with a warning, since
the host compositor owns the window's size and scale.

**Which scale a surface is told.** A window is told the scale of the output
it is placed on -- its workspace's output -- and re-told when it moves to an
output at another scale (a move, an unplugged monitor's windows adopted,
a replug restoring them). A floating window straddling two outputs follows
the one it is placed on, not whichever holds more of it. Its popups and
subsurfaces follow it. A bar or other layer surface is told its own
output's scale, a lock surface its own output's, and a client's cursor
surface the scale of the output under the pointer, re-told each time the
client sets it (a pointer crossing between two outputs inside one window
keeps the old scale on its cursor surface until the client next sets it; a
drag icon is told the pointer's output's scale when it is created and is not
re-told while dragged across the seam). A
surface with no role yet is told the scale of the output under the pointer,
where a new window opens. See
[protocols.md](protocols.md#output-scaling).

With `--xwayland`, X apps draw at the largest `wl_output.scale` integer
among the outputs (X has one scale for every screen), and the renderer
scales them down on the others -- see [`[xwayland]`](#xwayland).

## `[renderer]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `backend` | string (`"pixman"` or `"gles"`) | `"pixman"` | Which renderer composites each frame — the config-file form of `--renderer` (see [tty.md](tty.md#which-renderer-draws-the-frames)). `"pixman"` is the CPU renderer and needs no graphics device at all. `"gles"` draws with GLES on an EGL device; under `--tty` it needs a `--features gpu-scanout` build, where it scans out from the GPU, and warns and keeps pixman without one. `--renderer` wins when both name one, including `--renderer pixman` against a file asking for `gles`. A name that is neither is a warning and the default, like any other malformed value; but a name this build *knows* and then cannot build (`"gles"` with no working EGL) is a startup error — the second deliberate one — because silently drawing with the other renderer would be a session quietly different from the one you asked for. Takes effect on restart — a reload refuses changes with a message naming that. |

## `[tty]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `gpu` | string (device path) | unset | Which DRM device `--tty` drives, when the automatic choice is wrong — the config-file form of `--gpu PATH` (see [tty.md](tty.md#which-drm-device---tty-drives)). Unset means the automatic search picks: Smithay's primary GPU first, then every other DRM device on the seat until one works. Set means exactly that device, no fallback: a wrong path is a clean startup error naming the key and what failed, so this key is fail-closed where every other config field degrades gracefully. `--gpu` wins when both name one; an empty value (`gpu = ""`) is a startup error naming the key, on every backend. Only means anything under `--tty`; on `--headless` or `--nested` a set non-empty value is ignored with a warning. Takes effect on restart — a reload refuses changes with a message naming that. |

## `[xwayland]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | boolean | `false` | Whether to run an XWayland server inside the session, so X11-only applications get a `DISPLAY` to connect to — the config-file form of `--xwayland`, and either one turns it on (a flag can only say yes, so the two are OR-ed). Off unless asked: the server is a whole extra process (~55 MB RSS idle, ~87 MB with a few X clients) plus a hard `PATH` dependency on the `Xwayland` binary, and any X client can keylog/snoop by design (see [protocols.md](protocols.md#xwayland-opt-in)). Needs an `xwayland` Cargo-feature build -- from Nix, `packages.scoot-xwayland` (or `scoot-gpu-xwayland`), which also puts `Xwayland` on the compositor's `PATH` (see [nix.md](nix.md#xwayland-from-the-flake)); without the feature the knob parses but warns and the session runs Wayland-only. A missing binary at startup is the same shape: a loud log line naming `` `Xwayland` must be on PATH `` and the `PATH` searched, then a Wayland-only session, never a crash. Takes effect on restart — a reload refuses changes with a message naming that. `DISPLAY` is exported to spawned children (add it to the `dbus-update-activation-environment` line in your session script alongside `WAYLAND_DISPLAY` if anything D-Bus activated needs X), and so is each child's activation token as `DESKTOP_STARTUP_ID`, which is how an X app scoot started takes focus when it maps. X windows are columns (their own position ignored), dialogs and transients float, and `[[window_rule]]` `match_app_id` matches an X window's `WM_CLASS` class — see [protocols.md](protocols.md#xwayland-opt-in) for the whole policy and the trust model. The clipboard and primary selection cross between X and Wayland apps both ways, but X apps read or set them only while an X window has the keyboard (never while locked); drag-and-drop works in every direction (X to Wayland, Wayland to X, X to X, within one X app), and XIM is not provided — see [protocols.md](protocols.md#clipboard-drag-and-drop-and-input-methods). At an [`[output] scale`](#output) above 1 the X server draws at `ceil(scale)` by default (`fractional = "sharp"` below; `"light"` draws at `floor(scale)` instead -- sharp at 2, scaled down from 2 at 1.5) -- the largest such integer among the outputs when [`[[outputs]]`](#outputs) gives them different scales, scaled down on the others -- or, when the whole layout at that scale would be wider or taller than X's 32767-pixel limit, at the largest integer scale at which it fits -- and tells toolkits the scale over XSETTINGS, re-told on a reload or when an output change moves it (GTK measured; Qt 6 and Java documented to read it but unmeasured; Qt 5 gets scaled fonts only unless the app enables high-DPI scaling); an X app that reads no toolkit setting (bare Xlib, Wine) comes out that many times smaller, and one that reads only the `Xft.dpi` resource needs `xrdb -merge` — see [protocols.md](protocols.md#x-windows-in-the-layout). |
| `fractional` | string (`"sharp"` \| `"light"`) | `"sharp"` | What the X server draws at a fractional [`[output]`](#output) `scale` (1.25, 1.5, ...): `"sharp"` draws at `ceil(scale)` (2 at 1.5) for sharp X apps at about four times the buffer memory of drawing at 1, `"light"` draws at `floor(scale)` at a non-integer scale (1 below 2) for blurry upscaled X apps at about a quarter of the memory. Scale 1 and integer scales draw at themselves either way, byte for byte. Re-applied live by `scootctl reload` (reported as `xwayland.fractional`; a value that names neither choice is refused and the session keeps its own). At a fractional scale the memory cost is per X window: each window's buffers are its X pixels times 4 bytes, held twice -- the X server's pixmap and the shared-memory buffer -- so a window filling a 1920x1080 output at 1.25 holds ≈42 MB sharp (1536x864 logical, drawn at 2) against ≈10.6 MB light; the per-frame composite cost measured flat. |

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
focus-window-id ID | focus-workspace-index N [--output ID] | move-window-to-workspace-index N | focus-output ID | move-window-to-output ID | focus-output-index N | move-window-to-output-index N | focus-output-left | focus-output-right | move-window-to-output-left | move-window-to-output-right | cycle-column-width | set-column-width N | toggle-fullscreen | set-fullscreen ID on|off | toggle-maximize | set-maximized ID on|off | close | spawn COMMAND... | quit
toggle-floating | set-floating ID on|off | toggle-floating-focus
move-floating ID X Y | resize-floating ID WIDTH HEIGHT
```

e.g. `"focus-column left"`, `"close"`, or `"spawn foot -e htop"` (split on
whitespace, not run through a shell, so a path or argument containing a space
can't be expressed this way).

### Moving across outputs

With more than one output, three pairs of actions reach across screens —
stepping relatively, and naming a screen by position or by id. The relative
pair has
the default binds (bare Super focuses, Shift carries the focused window
there, the same split the workspace digits keep):

```toml
[binds]
"super+comma" = "focus-output-left"
"super+period" = "focus-output-right"
"super+shift+comma" = "move-window-to-output-left"
"super+shift+period" = "move-window-to-output-right"
```

Those four lines are the defaults: uncommenting them in a file printed by
`scoot --print-default-config` changes nothing. Each steps to the
neighbouring output in geometry order — left to right by x, then top to
bottom by y — wrapping around, so with three or more monitors either key
walks the whole row: left of the leftmost is the rightmost, and the
reverse. With one output there is nowhere to go, so either direction does
nothing; with two, either names the other. The geometry rule is stated so
it stays right once outputs become placeable (today the two orders
coincide, because outputs pack left to right; see
`docs/backlog/core/output-position-and-live-mode.md`, which is where
placing outputs lives).

**Behavior change, stated plainly:** on two monitors `Super+comma` used to
mean "go to screen 0" and `Super+period` "go to screen 1"
(`focus-output-index 0` / `1`); now they mean "step left / right, wrapping", so from the left
monitor `Super+comma` lands on the right one. The old behavior is one bind
away: `"super+comma" = "focus-output-index 0"` in your own `[binds]`
overrides the default.

**Positions and ids still name fixed screens.** `focus-output-index N`
and `move-window-to-output-index N` take a 0-based position in creation
order (the first screen, the second screen); `focus-output ID` and
`move-window-to-output ID` take the output id `scootctl outputs` reports.

**Stepping needs neither an id nor a position — that is the stability
rule.** Output ids are what `scootctl outputs` reports: stable for the
session and never reused, so a monitor that is unplugged and plugged back
in comes back as a *new* output under a *new* id. The default binds step
from the focused output instead of naming one at all, so they keep
reaching every monitor across a replug: after the return it sits in the
ring under its fresh id. An explicit bind naming an id (`focus-output 2`,
`move-window-to-output 2`) keeps meaning that exact id — check `scootctl
outputs` after a replug and bind the new one, or restart the session — and
a positional bind (`focus-output-index N`) keeps meaning creation-order
position N. An unknown id, and an out-of-range position, both do nothing.
`move-window-to-output` carries the focused window to
that output's active workspace and follows it there; `focus-output` focuses
that output's active workspace (or nothing, when it holds no windows). An
unknown id does nothing. All six are refused while the session is locked,
like every other action.

A workspace switch can name its output the same way:
`"focus-workspace-index 1 --output 2"` switches the second output's
workspace list and moves focus there, the way a bar's workspace button on
that monitor does over `ext-workspace-v1`. Without the flag it keeps
meaning the focused output's list. The same replug rule applies: the id
names one specific output for the session, so after a replug it names the
output that went away, and an unknown id does nothing.

A returning monitor also gets its windows back: when an output is removed,
its workspaces (and their windows, in order) are adopted by the remaining
output and stay there, and when an output with a matching identity is added
the still-open ones move back — the same workspaces, active index and column
order. A window moved elsewhere by hand in between stays where it was put;
a closed window drops out. Matching is by connector identity: the connector
name (`DP-1`) plus the EDID make/model/serial where the connector has an
EDID blob to read (name alone for panels without one, KVMs hiding it, and
the connector-less backends) — so a *different* monitor plugged into the
same connector does not inherit the old one's windows.

When the removed output had focus, the adopting screen switches to the
adopted workspace holding the focused window and keeps focus on it: the
work follows the session instead of vanishing behind the screen's own
workspace, which stays one keystroke away (`Super+1`, `Super+Ctrl+k`). When
focus was elsewhere, nothing on the adopting screen changes — a monitor
dropping in standby while you work on the other screen leaves it exactly as
it was, and its return does too. The restore returns the adopting screen to
the workspace it showed before, when it is still showing the adopted one,
and the focused window follows its own window home; otherwise focus stays
where it is. Adopted workspaces announce themselves to bars over
`ext-workspace-v1` with their monitor's name ("2 DP-1", back to "2" on
restore or once you empty them), and `scootctl windows` reports each
window's 0-based workspace, whether it was adopted, and from which
connector. Without a bar there is no on-screen cue that the panel's own
previous workspace went out of view — it stays one keystroke away, but
nothing says so.

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

Three behaviors worth knowing, plus the reload rule:

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
- **A reload runs the spawn delta.** Entries the session has not seen yet
  run once each, in file order, through the same path startup drains --
  new `spawn` entries only. A reloaded entry that is not a `spawn`
  (`quit` included) is refused by name and never acted on, so a reloaded
  `quit` cannot end the session. A `spawn` whose program fails to start is
  refused by name and stays pending: the next reload retries it, so
  `applied` means the entry started, never merely that it was attempted.
  Seen is by value per occurrence: editing
  an entry in place counts as new, removing one and re-adding it runs it
  again, duplicates count per occurrence, and a second identical reload is
  silent. A reload under session lock skips new entries (refused as
  skipped-while-locked) and decides the still-pending ones on the first
  unlocked reload instead (see [Reloading the config](#reloading-the-config)).

### Idle: locking and screen power

scoot provides the protocols (`ext-idle-notify-v1`,
`ext-session-lock-v1`, `wlr-output-power-management-v1` -- see
[protocols.md](protocols.md)); the policy ships with the desktop profile,
which runs it with the maintainer's measured timeouts (dim 2 min to 10%,
lock at 4, screens off at 5, lock before sleep -- see
[nix.md](nix.md#idle-and-lock)). What follows is the manual recipe for
sessions outside the flake.

`spawn` splits on whitespace with no shell and no quoting, so a
swayidle line (whose quoted subcommands carry spaces) lives in a small
script the session starts:

```sh
# ~/.config/scoot/idle.sh
#!/bin/sh
exec swayidle timeout 600 'swaylock' timeout 900 'wlopm --off \*' resume 'wlopm --on \*'
```

(The `\*` is load-bearing: swayidle runs each command through `sh -c`,
which would glob a bare `*` against its working directory -- the backslash
reaches `sh` intact inside the single quotes and leaves `wlopm` a literal
`*`, which is its "every output".)

```toml
[autostart]
commands = [
    "spawn /home/you/.config/scoot/idle.sh",
]
```

Lock after ten minutes, panels off after fifteen, back on at the first
input (`resume` runs on activity, locked or not). The same state is
drivable over IPC -- `scootctl output-power 1 off`, `scootctl
output-power all on`, `scootctl outputs` reporting it -- for agents and
scripts that do not speak Wayland (see [ipc.md](ipc.md#requests) and
[protocols.md](protocols.md#screen-power)). Input by itself never turns a
screen back on; only the daemon (or an explicit on) does.

## `[floating]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `auto` | bool | `true` | Float a window automatically when it first maps if it is a dialog (a modal `xdg-dialog-v1` hint, or a dialog object on a window that also names a parent — an unparented, non-modal dialog object, which GTK 4 attaches to every toplevel, tiles), names a parent (`xdg_toplevel.set_parent`, a transient window), or has a fixed size (equal non-zero minimum and maximum). `false` turns all three off; `[[window_rule]]`s still apply. Re-applied live by `scootctl reload`, for windows that map after it. |
| `modifier` | string | `"super"` | The modifier held to drag floating windows: with the left button anywhere on one it moves it, with the right button it resizes it. `super`, `alt`, `ctrl` or `shift` (or an alias a `[binds]` combo accepts: `logo`, `meta`, `cmd`, `control`); anything else falls back to `super` with a warning at startup, and is refused by name on a reload (the running modifier stays). Worth changing under `--nested`, where the host compositor often keeps Super for itself. Re-applied live by `scootctl reload`. |

What floating is — a layer above each workspace's scrolling strip, for
confirmation dialogs, file pickers, settings windows and anything you pick
with a rule:

- **Where it goes.** Centred on its parent when the parent is visible on
  the same workspace, otherwise on the output — always inside the output's
  usable area (a bar's reserved strip excluded). The window picks its own
  size (a rule's `size` asks for one); one larger than the usable area is
  asked to fit it. It keeps its centre as it resizes itself, and is re-centred
  when carried to another output (or its output changes size).
- **Moving and resizing.** Hold the `modifier` (Super) and drag with the
  left button to move a floating window, or with the right button to resize
  it: the edge or corner nearest the pointer follows it (the window is split
  in thirds each way; the middle goes to the nearest corner), and the
  opposite edge stays put. A window's own titlebar and borders work too
  (`xdg_toplevel.move`/`resize`, what GTK's headerbar and client-side
  borders send). Either way the window stays where you leave it, inside the
  usable area; a resize stops at the usable area rather than pushing the
  far edge, and respects the window's own minimum and maximum size where
  there is room for them (the usable area wins over a minimum). Drop a
  window with its middle over another output and it moves to that output's
  active workspace. A drag ends when the button is released, another button
  is pressed, the window closes or stops floating, its workspace is switched
  away, the screen locks, the session switches VT, or the output changes
  size. The modifier on a tiled window, and a tiled window's own titlebar
  drag, do nothing special: tiled windows are placed by the strip. Agents
  use `move-floating ID X Y` and `resize-floating ID WIDTH HEIGHT` (see
  [ipc.md](ipc.md)).
- **Stacking.** The most recently focused floating window is on top; a click,
  `focus-window-id`, or a taskbar activating it raises it -- but a window's
  own floating dialogs are always drawn above it, so clicking an app never
  buries the dialog it opened.
- **Focus.** `Super+Space` (`toggle-floating-focus`) moves focus between
  the floating windows and the strip. With a floating window focused,
  `focus-column left|right` goes back to the strip's focused column,
  `focus-window up|down` cycles the floating windows, and the strip's own
  actions (`move-column`, `move-window`, `consume-or-expel`,
  `cycle-column-width`, `set-column-width`) do nothing. Moving the window to
  another workspace or output keeps it floating there.
- **The strip underneath is untouched.** A dialog that floats as it opens
  leaves every column, width and scroll position exactly as it found them.
  Floating a column takes it out (focus in the strip goes to the column on
  its left); un-floating (`Super+Shift+Space`) puts it back as a column right
  of the strip's focused column, at the width it had -- back where it was,
  except the leftmost column (it comes back second) and a window floated out
  of a stacked column (it comes back as a column of its own).
- **Fullscreen.** A floating window can go fullscreen and comes back floating.
  A dialog opened by a fullscreen app shows above it, and stays up when you
  click the app (a modal dialog hidden under its app would look like a hung
  app); other floating windows hide while a fullscreen window has focus.
  A fullscreen window, tiled or floating, stays in place under a floating
  window that has focus.
- **Decided once.** Whether a window floats is decided when it first maps; a
  title change later does not re-decide it. The toggle changes it any time.

## `[[window_rule]]`

Each rule is its own `[[window_rule]]` table; repeat the header for more.

| Field | Type | Meaning |
|---|---|---|
| `match_app_id` | string (glob) | Matches the window's app id -- for an X11 window (under [`--xwayland`](#xwayland)), its `WM_CLASS` class: `XTerm` for `xterm`, `Gimp` for GIMP. |
| `match_title` | string (glob) | Matches the window's title. |
| `float` | bool | `true` floats a matching window when it maps; `false` keeps it in the strip even if `[floating] auto` would float it. |
| `size` | `[width, height]`, logical px | The size to ask a floating window for when it maps (each axis `1..=65535`, clamped to the output's usable area). No effect on a window that tiles. |

A rule needs at least one matcher, and must set `float`, `size` or both;
when it names both matchers, both must match. Matchers are **globs over the
whole string**, case-sensitive: `*` matches any run of characters, `?`
exactly one, everything else itself — so `"foot"` matches only `foot`, not
`footclient`, and `"*Preferences*"` matches any title containing
`Preferences`. `match_app_id = "*"` matches every window. There is no
escape: a literal `*` or `?` in an app id or title cannot be matched as
itself -- use `?` in its place (it matches any one character, the `*`
included). Rules apply in
file order; a later matching rule overrides an earlier one for each field it
sets. Rules are checked when a window first maps, after `[floating] auto`'s
heuristics, so a rule always has the last word. `scootctl windows` shows the
app id and title to match against.

```toml
[[window_rule]]
match_app_id = "org.gnome.Calculator"
float = true

[[window_rule]]
match_title = "*Picture-in-Picture*"
float = true
size = [640, 360]

# An X11 app (under --xwayland), matched by its WM_CLASS class:
[[window_rule]]
match_app_id = "XCalc"
float = true

# A dialog you would rather have as a column:
[[window_rule]]
match_app_id = "org.gnome.Nautilus"
match_title = "*Properties*"
float = false
```

## `[wallpaper]`

The wallpaper, drawn by [scootbg](scootbg/README.md), scoot's wallpaper
daemon (its own binary and package). The section is all it takes: scoot
starts scootbg itself and re-applies the section on every reload, with no
`[autostart]` entry and no session script.

```toml
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"                    # fill | fit | stretch | center | tile

[wallpaper.output."DP-2"]        # optional, one table per output
color = "#101014"
```

A link works where a path does: `image = "https://example.com/hills.jpg"`
is downloaded once and cached by scootbg (below), so an example look can
name an image nobody commits. `sha256` pins the download's bytes.

| Field | Type | Meaning |
|---|---|---|
| `image` | string (path or URL) | A PNG, JPEG or WebP image. A path: `~` and `~/...` expand against `HOME`, and a relative path resolves against the directory the config file is in (the link's directory for a symlinked config, not its target's). A URL (`http://` or `https://`): downloaded once and cached by scootbg — see [scootbg's command reference](scootbg/cli.md#colors-and-images). |
| `color` | string (`"#rrggbb"`) | A solid color. `image` or `color`, never both; neither is nothing (the compositor's own `background_color`). |
| `mode` | string | With an `image` only: `fill` (cover and crop, scootbg's default), `fit` (letterbox with `fill`), `stretch`, `center` or `tile`. |
| `fill` | string (`"#rrggbb"`) | With an `image` only: the color around a `fit` or `center` image. |
| `filter` | string | With an `image` only: the scaling filter, `lanczos3`, `catmull-rom`, `bilinear` or `nearest`. |
| `sha256` | string (64 hex digits) | With a URL `image` only: the download's expected SHA-256 (as `sha256sum` prints). Anything else fails instead of showing. Refused beside a path. |
| `output."NAME"` | table | The same six keys for one output, by connector name (as `scootbg query` and `scootctl outputs` list them: `headless-2`, `DP-2`, ...). Each output table stands alone: an output's `image` does not take the top level's `mode`. An empty table is nothing on that output. |
| `command` | string | The `scootbg` to run. Default `"scootbg"`, found on `PATH`; a path with a `/` in it resolves like `image`. The Nix modules set it to the installed package's store path. |

A worked example with a wallpaper, ring colors, a bar and a terminal palette is in
[examples/radial-burst](examples/radial-burst/README.md).

(Under home-manager with Stylix, `programs.scoot.stylix.enable` defaults
`image` and `mode` from `stylix.image` and `stylix.imageScalingMode` — see
[`docs/nix.md`](nix.md#stylix). A value written here always wins, except a
`color` written next to Stylix's `image`: `image` or `color`, never both,
so that combination is refused — for a solid color, set
`programs.scoot.stylix.wallpaper.enable = false` (the themed ring,
background and cursor stay), or set your own `image`, or turn the Stylix
defaults off.)

An empty `[wallpaper]` table is a section too: it says "no wallpaper from
the config" and clears whatever the config set before. For no wallpaper
handling at all, leave the whole table out.

**What scoot does with it.** At startup, and on every reload, while the
section exists, scoot runs `COMMAND apply-config --profile PROFILE JSON`
with the section's values as JSON (paths resolved, only the keys you
wrote, `command` left out); on the reload that removes it, the same with
`{}`; at startup without one, nothing. `PROFILE`, the saved state scootbg
restores and records, is one per backend: `scoot` under `--tty` (your
real session), `scoot-nested` under `--nested` (apart from its host) and
`scoot-headless` under `--headless`, so a `scootbg set` made in a
throwaway headless session (an agent's, a test's) never becomes what your
real login restores, and two sessions never write one state file. `apply-config` starts the scootbg daemon when
none runs and hands it the section otherwise; see
[scootbg's `apply-config`](scootbg/README.md#apply-config-scoots-wallpaper-section)
for everything on its side.

**Whichever you changed last wins.** Edit `[wallpaper]` (and start or
reload scoot): the config's wallpaper shows. Run `scootbg set` after that:
your pick shows, and keeps showing across restarts and unrelated reloads,
until you next change `[wallpaper]` itself. scootbg compares a fingerprint
of the section with the last one applied for the profile, so a reload that
leaves the section alone re-applies nothing (the run still happens: it is
what brings back a daemon that crashed).

| Sequence | Result |
|---|---|
| section A, `scootbg set X`, restart | A unchanged, so X |
| section A, reload with B, `scootbg set X`, restart | B unchanged since its reload, so X |
| section A, reload with B, restart | B |
| section removed by a reload, later re-added as A | A |

"Changed" is as written: `#1E1E2E` and `#1e1e2e`, or an explicit `mode =
"fill"` and none, are different sections, and so is a moved image. A new
`command` alone is not a change (the Nix modules' store path moves on every
upgrade). One order is not seen: the section removed while scoot is not
running and re-added unchanged before the next start counts as unchanged.

**Never in the way.** scoot spawns `apply-config` and never waits for it
(`apply-config` returns once the wallpaper is on screen, which takes scoot
processing scootbg's frames). Its stdin, stdout and stderr are scoot's, so
its messages (and the daemon's) land in scoot's log; it gets no other file
descriptor of scoot's. Runs are one at a time: a reload while one runs
waits for it, and of several waiting only the newest section runs. A run
is waited on for 40 s at most, then scoot logs that and moves on (a state
file on a hung network disk is the only way to get there); the run is
still reaped and its end logged. Nothing here costs anything per frame or
per input event.

**When it fails, the session carries on** with its `background_color`:

- **scootbg is not installed** (or `command` names nothing): a warning in
  scoot's log naming the command and how to install it. The next reload
  tries again.
- **A run fails**: scoot logs its exit status and command. Status 1 is a
  runtime failure (scootbg says which just above in the log: an image that
  is not a file, no daemon reached, ...); status 2 is a section scootbg
  refused (a malformed color, an unknown mode or filter: values scoot
  passes through for scootbg to check). A failed section is tried again on
  the next reload, and whenever a run scoot stopped waiting on ends.
- **The section itself has a problem** (an unknown key, a value that is not
  a string, `~` with no `HOME`): at startup, an error in the log names it
  and no wallpaper is set from the file, but **the rest of the file
  applies** -- unlike other tables, where an unknown key discards the whole
  file (see [Failure semantics](#failure-semantics)); on a reload, the
  field is refused by name, on every reload until it is fixed, and the
  running section is kept and re-run (so a crashed daemon still comes
  back).
- **A value nested absurdly deep** inside `[wallpaper]` (more than 16
  levels of arrays and tables) is not skipped: the whole file is refused
  as malformed TOML is, since nothing a wallpaper value means nests that
  deep.

**Reloading.** `scootctl reload` reports `wallpaper` as applied when the
section's values changed (added, edited or removed) and
`wallpaper.command` when `command` changed (both, when both did);
"applied" means handed to scootbg, and the outcome is in the log. An unchanged section is silent in
the reply but still runs. It applies under session lock too: the
wallpaper is on the background layer, which a locked frame does not show.

**Leave `scootbg daemon` out of `[autostart]`** and your session script
when you use this section: `apply-config` starts the daemon. A second
start is harmless (one daemon wins, and it moves to scoot's profile), but
one that starts first shows the `default` profile's wallpaper for a moment
before scoot's section replaces it.

**Until scootbg's first frame**, scoot shows its `background_color`. How
long that is at a real `--tty` login has not been measured yet.

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
| `Super+comma` / `Super+period` | Focus the output left / right, wrapping around every monitor |
| `Super+Shift+comma` / `Super+Shift+period` | Move the window to the output left / right, wrapping, and follow it |
| `Super+r` | Cycle column width |
| `Super+f` | Toggle fullscreen |
| `Super+m` | Toggle maximize (fill the usable area, bar visible) |
| `Super+Shift+Space` | Float the focused window, or put it back in the strip |
| `Super+Space` | Move focus between the floating windows and the strip |
| `Super` + left-drag | Move a floating window (`[floating] modifier`) |
| `Super` + right-drag | Resize a floating window from the nearest edge or corner |
| `Super+q` | Close focused window |
| `Super+Return` | Spawn `foot` |
| `Super+Shift+e` | Quit |

All 44 key bindings (plus the two drags, which are not `[binds]` entries:
their modifier is `[floating] modifier`) — vim motions (`h`/`j`/`k`/`l`) for direction, Super as
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
entry like `cycle-column-width`. An index past the end of the list does
nothing, like a stale workspace index. It has no default bind, by decision:
the width list is yours to size, so no key family maps onto it the way
digits map onto workspaces. Add your own, e.g.:

```toml
[layout]
column_widths = [0.3333333333333333, 0.5, 1.0]

[binds]
"super+w" = "set-column-width 2"
```

(For "a window as large as the screen that still keeps the gaps, the focus
ring and the bar", that used to be the recipe — a `1.0` entry bound to a
key. `Super+m` (`toggle-maximize`) is now the real version: it fills the
usable area exactly, restores the layout on leave, and tells the client it
is maximized.)

`Super+f` (`toggle-fullscreen`) puts the focused window into fullscreen and
back. A fullscreen window covers its whole output while its column is the
focused one — gaps, the focus ring and a bar's reserved strip included.
Surfaces on the `top` layer are hidden under it — bars, but also
notifications from a daemon that draws there (mako's default; set
`layer=overlay` in its config to keep them on top); the `overlay` layer and
the lock screen stay above it. It keeps its place in the strip: focus
another column and the view scrolls there as usual, focus back and it covers
the screen again, and
leaving fullscreen puts the layout back exactly as it was. Moving the window
to another workspace or output, or consuming/expelling it, ends fullscreen;
so does moving focus to a window stacked in the same column. The same state
is what a client's own fullscreen button asks for — see
[protocols.md](protocols.md#fullscreen) for every way in.

`Super+m` (`toggle-maximize`) puts the focused window into maximized and
back. A maximized window fills its output's usable area — the whole output
minus a bar's reserved strip, minus the layout gap — while its column is
the focused one: full width and full height of the workspace strip, inside
the configured gaps, with its focus ring kept. The bar stays visible, which
is the difference from fullscreen. Like fullscreen it keeps its place in
the strip and leaving restores the layout exactly; moving the window to
another workspace or output, consume/expel, focusing a window stacked in
the same column, or floating/un-floating it ends maximized. Choosing a
column width (`cycle-column-width`, `set-column-width`) does nothing while
the focused window is maximized. Fullscreen wins while both hold: leaving
fullscreen returns to maximized, not to the plain strip. The same state is
what a client's own maximize button asks for — see
[protocols.md](protocols.md#maximized) for every way in.

`--tty` additionally binds `Ctrl+Alt+F1` through `Ctrl+Alt+F12` to VT
switching — not present under `--headless`/`--nested`, since VT switching is
a Linux-session concept with no meaning there. See
[tty.md](tty.md#vt-switching) for why they always win over a config-file
bind — at startup and on every `scootctl reload`, which layers them back on
last rather than letting a reloaded file strip the recovery path.

Moving a window across outputs, or focusing another output, steps left and
right through every monitor by default, wrapping around
(`Super+comma`/`Super+period` and Shift for the carry — see [Moving across
outputs](#moving-across-outputs)); fixed screens are config binds you add
yourself (`focus-output-index N`, or an output id).

## Example `config.toml`

```toml
[layout]
gap = 8
column_widths = [0.25, 0.5, 0.75, 1.0]
default_column_width = 1

[appearance]
focus_ring_width = 4
focus_ring_inactive_width = 2
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

# A HiDPI laptop panel beside an ordinary monitor: each its own scale, by
# the connector name `scootctl outputs` lists.
# [[outputs]]
# name = "eDP-1"
# scale = 2.0
#
# [[outputs]]
# name = "DP-1"
# mode = "1920x1080"

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

[floating]
# Dialogs, transient and fixed-size windows float when they map; false
# tiles everything except what a rule floats.
auto = true
# Alt+drag moves and resizes floating windows (Super is the default).
modifier = "alt"

# pavucontrol's app id is org.pulseaudio.pavucontrol (`scootctl windows`
# shows any window's).
[[window_rule]]
match_app_id = "*pavucontrol"
float = true
size = [700, 500]

# Needs scootbg installed (the Nix modules do it for you).
[wallpaper]
image = "~/Pictures/hills.jpg"
mode = "fill"

[wallpaper.output."DP-2"]
color = "#101014"
```
