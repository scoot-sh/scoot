---
title: CLI reference
description: "Every scootbar command and flag, scootbar msg, the agent interface, checks and exit statuses."
---

Every command and flag, how each behaves at the edges, the `msg` control channel, and the agent interface.

Every `scootbar` command and flag, and how it behaves at the edges. What
scootbar is and why is in [Overview](./index.md). Every option lives in
the [config file](./cli.md#the-config-file) too (`$XDG_CONFIG_HOME/scoot/bar.toml`);
the flags stay and override its values, one by one, at start-up and on
every reload.

**Early days:** the bar shows a clock in the center, and workspaces wherever
they are placed (`--left workspaces`). Modules answer the pointer
([click, scroll and hover](./modules.md##pointer-input)).



## Commands

```sh
scootbar daemon                              # a 28-pixel bar along the top of every output, the clock in the middle
scootbar daemon --left workspaces --center clock  # each output's workspace numbers on the left, the clock in the middle
scootbar daemon --clock-format '%H:%M'       # a 24-hour clock (the default is 12-hour: 3:07 pm)
scootbar daemon --font ~/.local/share/fonts/Inter.ttf --font-size 13
scootbar daemon --right clock                # the clock at the right end
scootbar daemon --edge bottom --height 32    # along the bottom, 32 logical pixels tall
scootbar daemon --layer overlay --exclusive false  # over everything, reserving nothing
scootbar daemon --margin 8                   # floating 8 pixels in from its edge and both sides
scootbar daemon --margin 8,12                # 8 above and below, 12 either side
scootbar daemon --background '#101014' --foreground '#e0e0e0'
scootbar daemon --outputs DP-1,eDP-1        # a bar only on those two outputs
scootbar daemon --config ~/alt-bar.toml    # another file than the default
scootbar msg query                         # every placed module's state as JSON
scootbar msg layout                        # where each module is on screen, for a click
scootbar msg invoke volume raise 5         # run a module's action, as its click would
scootbar msg invoke volume popup           # open (or close) the volume slider popup
scootbar msg subscribe                     # stream changes, one JSON line each
scootbar msg reload                        # re-read the file and live-apply it
scootbar msg toggle                        # hide the bar (and release its space), or show it
scootbar --help                              # and `scootbar daemon --help`, `scootbar msg --help`
scootbar --help --json                       # the same content as JSON (see below)
scootbar --version
```

The binaries document themselves for agents: `scootbar help daemon` and
`scootbar help msg` print each command's page, and `scootbar --help --json`
emits commands, daemon flags (with types and defaults), msg commands, the
modules in the build, exit codes and environment — see
[Generated CLI pages](../reference/cli.md) for the contract.

`scootbar daemon` runs in the foreground until the compositor goes away
(start it with `&`, or from your compositor's autostart). It connects to
the compositor named by `$WAYLAND_DISPLAY`, which must support
`wlr-layer-shell` (scoot, sway, niri, Hyprland and most wlroots compositors
do).



## `daemon` flags

Each flag at most once, as `--flag VALUE` or `--flag=VALUE`.

| Flag | Takes | Default | What it does |
| --- | --- | --- | --- |
| `--outputs` | `all`, or connector names, comma-separated | `all` | Which outputs get a bar (`DP-1,eDP-1`). See [Outputs](./modules.md##outputs). |
| `--edge` | `top` or `bottom` | `top` | The output edge the bar runs along. Vertical bars (left and right) are a deliberate omission: the layout is horizontal, and the modules, the hit-testing and the text all assume it. |
| `--layer` | `bottom`, `top` or `overlay` | `top` | The layer-shell layer. See [Layers and the zone](./modules.md##layers-and-the-zone). |
| `--exclusive` | `true` or `false` | `true` | Whether the bar reserves its height, so windows are arranged beside it, or floats over them, reserving nothing. |
| `--height` | 1 to 1024 | 28 | The bar's height in logical pixels. On a scaled output it is drawn at the output's real pixels: 28 at scale 1.5 is 42 device pixels. |
| `--margin` | one to four of 0 to 1024, comma-separated | `0` | Space between the bar and the output's edges, in logical pixels, in CSS order: `ALL`, `VERTICAL,HORIZONTAL`, `TOP,HORIZONTAL,BOTTOM` or `TOP,RIGHT,BOTTOM,LEFT`. See [Margins](./modules.md##margins). |
| `--background` | `'#rrggbb'` | `'#1e1e2e'` | The bar's color, six hex digits in either case. Quote it: the shell reads `#` as a comment. Its opacity is the file's `[bar] opacity`: see [Shape and opacity](./modules.md##shape-and-opacity). |
| `--foreground` | `'#rrggbb'` | `'#cdd6f4'` | The text's color (the theme's `fg` token: see [Colors](./modules.md##colors)). |
| `--font` | a path | the first [well-known font](./modules.md##fonts) found | The font file, TrueType or OpenType (`.ttf`, `.otf`; a collection's first face). Any bytes: a path need not be UTF-8. |
| `--font-size` | 1 to 256 | 14 | The text's size, the em, in logical pixels; drawn at the output's real pixels like the bar. |
| `--left`, `--center`, `--right` | module ids, comma-separated, or empty | the clock in the center | The modules along each part of the bar, in order. Giving any of the three sets the whole layout: a part not given is empty, so `--right clock` moves the clock rather than adding a second one. `--center ''` places nothing: a plain bar that needs no font. See [Layout](./modules.md##layout). |
| `--padding` | 0 to 1024 | 8 | Logical pixels either side of each module's content. |
| `--spacing` | 0 to 1024 | 0 | Logical pixels between neighbouring modules. |
| `--clock-format` | a format, at most 256 bytes | `'%-I:%M %P'` | What the clock shows; see [The clock](./modules.md##the-clock). |
| `--config` | a path | `$XDG_CONFIG_HOME/scoot/bar.toml` (`~/.config/scoot/bar.toml` without it) | The config file to read instead of the default; see [below](./modules.md##the-config-file). |
| `--check` | nothing (a switch) | off | Validate and exit instead of running: see [`--check`](./modules.md##--check-validate-without-running). |

A malformed or out-of-range value, an unknown flag or a flag given twice is
a usage error (exit status 2), and nothing starts; so is an unknown module
id, a module placed twice, or a clock format with an unknown specifier or a
control character.



## The config file

`scootbar daemon` reads `$XDG_CONFIG_HOME/scoot/bar.toml`
(`~/.config/scoot/bar.toml` when `XDG_CONFIG_HOME` is unset or empty), its
own file separate from scoot's `config.toml`, so it works on other
compositors and a bar change never breaks scoot. `--config PATH` reads
another file instead. A missing file is the defaults; an explicit
`--config` naming nothing is a refusal.

Every section is optional; absent is the default. Precedence is defaults,
then the file, then the flags: a flag given replaces the file's value for
its own option, any of `--left`/`--center`/`--right` replaces just that
section of the file's layout, and a reload keeps the flags over the file,
as at start-up.

```toml
outputs = "all"       # or ["DP-1", "eDP-1"]: see Outputs below
left = ["workspaces"]
center = ["clock"]

[bar]
edge = "top"          # top or bottom
layer = "top"         # bottom, top or overlay
exclusive = true      # false: float over the windows, reserving nothing
height = 28           # 1 to 1024
margin = "8,4"        # one number, or the CSS shorthand "VERTICAL,HORIZONTAL", ...
radius = 8            # 0 to 512, and at most half the height; 0 is square
popup-radius = 8      # 0 to 512; unset is the bar's radius: see Popups
opacity = 0.9         # 0 (transparent) to 1 (opaque), the background's alpha
font = "/path/to/Font.ttf"
fallback-fonts = ["/path/to/Symbols.ttf", "/path/to/Cjk.otf"]   # at most 2: see Fonts
font-size = 14        # 1 to 256
padding = 8           # 0 to 1024
spacing = 0           # 0 to 1024
separator = 0         # a line in the gap between modules, 0 to spacing: see Spacing
tooltip-delay = 500   # ms the pointer rests on a module before its tooltip shows, 0 to 10000; 0 is off: see Tooltips

[colors]
background = "#1e1e2e"
foreground = "#cdd6f4"
accent = "#f9e2af"
dim = "#6c7086"
urgent = "#f38ba8"

[clock]
format = "%-I:%M %P"
on-click = { exec = ["foot", "-e", "calcurse"] }   # a command; or on-right-click, on-middle-click, on-scroll-up, on-scroll-down
icon = "\U000f0e65"     # one glyph before the time, from a symbol font: see Fonts
# icon-path = "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"   # or SVG path data, or:
# icon-viewbox = "0 0 24 24"                          # the path's plane (that is the default)
# icon-image = "/abs/path/icon.png"                   # or a PNG (--features icon-image): see Icons
margin = 0            # extra room on each side of the module, 0 to 1024: see Spacing

[workspaces]
margin = 0            # as the clock's
pill-shape = "rect"   # the active number's pill: rect, pill or circle
pill-radius = 0       # a rect's corner radius, 0 to 1024
pill-inset = 0        # the pill's gap from the bar's top and bottom, 0 to 1024
on-scroll-up = "previous"   # the interaction keys, on every module: see Pointer input
on-scroll-down = "next"

[window-title]
show-app-id = false   # the app id after the title: "title - app"
max-width = 480       # the most logical pixels wide the title's span may be
placeholder = ""      # shown when no window is focused (empty takes no space)
allow-close = false   # a middle click (or the close action) closes the window
margin = 0            # as the clock's

[volume]
step = 5              # percent points per scroll notch and per raise, 1 to 50
max-volume = 100      # the cap a raise stops at: 100 is full scale, to 150 is over-amplification
on-right-click = { exec = ["pavucontrol"] }   # a mixer, or any command
on-click = "popup"    # a slider popup under the module, instead of mute: see Popups
margin = 0            # as the clock's

[microphone]          # the default source, same keys as [volume]
step = 5
margin = 0

[battery]
warn-below = 20       # the class turns warn at or below this percent, 0 to 100
urgent-below = 10     # urgent at or below this percent; the on-low crossing
batteries = "combine" # the mean, or "first" for the first battery
on-low = { exec = ["notify-send", "Battery low"] }   # run once per downward crossing of urgent-below
margin = 0            # as the clock's

[brightness]
device = "apple-panel-bl"   # the backlight to follow; absent is the first usable one
step = 5              # percent points per scroll notch and per raise, 1 to 50
margin = 0            # as the clock's

[media]
player = "spotify"    # the player to prefer when several run; absent is the one that played last
max-width = 320       # the most logical pixels wide the module's span may be, 1 to 4096
margin = 0            # as the clock's

[bluetooth]
menu-command = ["fuzzel", "--dmenu"]   # the device picker, fed the device list on stdin
margin = 0            # as the clock's

[button.launcher]     # modules the file defines, placed by name in the lists above
icon = "\U000f0e65"
on-click = { exec = ["scootlaunch"] }
[exec.weather]
command = ["sh", "-c", "while :; do curl -s 'wttr.in?format=1'; sleep 600; done"]
icon = "\U000f0e65"     # one glyph before the text, from a symbol font: see Icons
[push.status]
placeholder = "..."
icon = "\U000f0e65"     # as the exec's

[output."eDP-1"]      # what differs on one output: see Outputs below
height = 36
right = ["clock"]
```

`margin` takes an integer (every side) or the `--margin` shorthand
string. `radius`, `popup-radius`, `opacity`, `separator`, the modules' `margin` and the
pill's keys are file-only: they have no flags (see
[Shape and opacity](./modules.md##shape-and-opacity), [Spacing](./modules.md##spacing) and
[the pill](./modules.md##the-active-workspaces-pill)). The module lists take the ids in [Modules](./modules.md##modules); giving any
of the three sets the whole layout, as the flags do. An unknown key
anywhere is a loud error naming it, as is a bad value, which names its
dotted key (`bar.height`, `colors.background`, `left`, `clock.format`;
`output."eDP-1".height` in an output table).
A bad file refuses to start the daemon (exit status 1); a bad `reload`
is refused and the running bar stands undisturbed.



### `--check`: validate without running

```sh
scootbar daemon --check --config ./bar.toml     # prints `ok` and exits 0, or says why and exits 1
scootbar daemon --check                         # the default file, as a start would read it
```

Does what a start does before it connects to anything, and stops there: reads
the config file, applies the other flags over it, starts the placed modules
and loads the font. It prints `ok` on stdout and exits 0, or prints the
error a start would print (a bad key, a bad value, a font that cannot load,
a missing `--config` file) on stderr and exits 1; a flag the file clashes
with is a usage error, status 2, as at a start. It never connects to the
compositor and never claims the control socket, so it needs no
`WAYLAND_DISPLAY` and no runtime directory, and it runs beside a live bar
without touching it. Its use is a build step or a pre-flight over a file about
to be installed: the Nix module's check runs it over every rendered file
(`checks.<system>.scootbar-modules`, [docs/nix.md](./index.md)).
Modules that report themselves unavailable at start (the clock without a
time zone database, say) print their note as a start does and are left out;
that is not a failure. What the flag costs (aarch64, release, stripped): +2.4 KB of text (code and
help; 1,200,636 to 1,203,036 bytes), no change in data or bss, the file
the same size to the byte (segments are page-aligned), and an idle daemon's
`VmRSS` unchanged within noise (3,360 to 3,376 kB, one 3,552 outlier, five runs each). It moves startup
code into a function the start also uses; nothing on a per-frame path.



## `scootbar msg`

Asks the running daemon over its control socket,
`$XDG_RUNTIME_DIR/scootbar-DISPLAY.sock`, a lock-guarded, owner-only
socket with line-framed JSON, one daemon per display: a second daemon
refuses, saying one already runs.

```sh
scootbar msg query [ID]             # every placed module's state as JSON (or one module's)
scootbar msg layout                 # each module's rectangle in global logical pixels
scootbar msg invoke ID ACTION [N] [--output NAME]  # run an action as a click would
scootbar msg subscribe [module] [output]           # stay connected, print events
scootbar msg reload                 # re-read the file and live-apply it
scootbar msg hide                   # destroy the bar's surfaces and buffers, release its space
scootbar msg show                   # make them again
scootbar msg toggle                 # hide if shown, show if hidden
scootbar msg version                # the daemon's version and protocol, as JSON
scootbar msg kill                   # stop the daemon, once its reply is sent
scootbar msg set ID JSON            # write a push module's text, class, tooltip and icon (below)
```

`query` prints one JSON object: one entry per placed module per output
that shows it (an output with no bar, or one whose [table](./modules.md##outputs) leaves
the module out, has none),
with its `id`, `section` (`left`, `center` or `right`), `output` (the
compositor's `wl_output.name`, `null` where it never sent one), the `text`
it shows and its `class` (`normal`, `warn`, `urgent`, `muted`), plus `icon`
where the module shows a glyph icon (absent otherwise, and for a path or image
icon, which is not text), `tooltip` (absent while empty) and a
`value` where it has one (the workspaces module: `{"active": 2, "workspaces": [1, 2, 3]}`
for that output, `active` `null` when none is; the window-title module:
`{"title": "editor", "app_id": "foot", "fullscreen": false}` for the
focused window, absent when none is focused; the volume module:
`{"volume": 49, "muted": false, "sink": "alsa_output..."}` for the default
sink, absent while no server answers (the microphone variant reports
`"source"` instead of `"sink"`); the battery module:
`{"percent": 72, "state": "discharging", "batteries": 1}`, absent where
there is no battery; the network module:
`{"state": "wifi", "ssid": "Wimbly", "signal": -54, "bars": 4,
"interface": "wlan0", "vpn": false}`, `"ethernet"` and `"vpn"` with the
interface, or `{"state": "disconnected"}`; the brightness module:
`{"percent": 49, "device": "apple-panel-bl"}`, absent where there is no
backlight; the bluetooth module: `{"state": "connected", "adapters": 1,
"powered": true, "connected": 1, "device": "Headset", "battery": 72}`
(`state` is `off`, `on` or `connected`; `battery` only when BlueZ reports
one), absent where there is no adapter).
This is the agent hook: the
bar read as data instead of OCR. `query ID` lists only that module (an id that
is not placed is an error naming the ones that are). The reply is bounded
(512 KiB; an agent's bar is a few hundred bytes a module, text and tooltip
each capped at 256 bytes), and is written from the very state the screen is
drawn from, so it cannot disagree with a screenshot.
`reload` re-reads the file and live-applies it — geometry, style, layout,
modules, the font — after fully validating it first; a bad file is
refused and the running bar stands. `hide`, `show` and `toggle` are
[below](./modules.md##hiding-the-bar). `query`, `layout`, `version`, `reload`, `hide`, `show` and
`toggle` print the reply; `kill`, `set` and `invoke` print nothing on success,
and `subscribe` prints what the daemon sends until it closes (below). `set`
writes to a [`push` module](./modules.md##button-push-and-exec-modules): an id that is not
placed, a module that takes no value (every one but `push`) and a value it
refuses are each a loud error naming why, never a silent ok. Without a
daemon, every command fails saying so (exit status 1).



## The agent interface

What an agent (or a script) uses to read the bar and press it, with no
screenshot to read and no pixels to hunt. All of it is on the bar's own
socket, separate from scoot's IPC.

**`layout`** prints, per output, its `output` name, `origin` (in the
compositor's global logical pixels), `scale`, the `bar` rectangle (`null`
while the bar is hidden or not yet configured) and the `modules` that show
something, left to right, each with `id`, `section` and an `x`, `y`,
`width`, `height` rectangle in the same global logical pixels. It is the
layout **as last drawn**: the spans (in device pixels) the last committed
frame used, converted with the output's **current** scale (the reply does not
record the scale a frame was drawn at, so a scale change that has not been
redrawn yet is the one moment a rectangle and the pixels can disagree) and
rounded outward, so a pointer anywhere on a drawn pixel of a module is inside
its rectangle. Aim scoot's pointer injection at the middle of a rectangle
(`scoot msg pointer click X Y`, which moves there and presses and releases the
left button; `right` and `middle` name the others) and the module is pressed; a test clicks the first and last logical pixel of
every rectangle and one pixel outside it, on two outputs at scales 1 and
1.5. A hidden bar has no rectangles at all. A module whose text is empty
takes no space and is not listed.

**`invoke ID ACTION [N] [--output NAME]`** runs an action exactly as a click
or scroll would: the same code a pointer press ends in, so what an agent does
and what a user does cannot diverge. `ACTION` is one of the module's own
actions (`scootbar msg --help`, or the refusal, lists them: the workspaces
module's `activate N`, `activate-position N`, `previous`, `next`), or a
trigger (`click`, `right-click`, `middle-click`, `scroll-up`, `scroll-down`),
which runs the binding configured for it. A scroll's `N` is its steps (1 to
32, default 1); a module's own action takes the number it asks for. `--output`
names the output whose module is meant (default: the first that shows it).
A module that is not placed, an action it does not have (or a trigger it has
no binding for), a number where none is taken or a missing or out-of-range
one, and an output that does not show the module are each a named error and
run nothing. Success prints nothing.

**`subscribe [module] [output]`** keeps the connection open and prints one
JSON line per event, after one `{"type":"subscribed","events":[...]}` line.
No kind named is both. **There is no snapshot**: a subscription starts from
now and only changes follow, so to read the state and then follow it,
**subscribe first, then `query`** (the other order can miss a change between
the two; this one at worst repeats one the `query` already shows). A `module` event is a `query` entry with `"type":"module"`,
sent when that module's view changed, once per output that shows it; an
`output` event is `{"type":"output","change":"added"|"removed","name":...}`.
Events are **coalesced to the frame rate**: a module that changes a hundred
times in a frame is told once, with its latest view, and a batch goes out at
most every 16 ms (the loop sleeps only until a held batch is due). A reload
tells every module again. A subscribed connection serves no further requests
(it gets one error line, however many it sends), at most 4 may be subscribed
at once (a fifth is refused saying so), and **a subscriber that stops
reading is disconnected, never buffered**: each batch is one nonblocking
write, and one the socket cannot take whole ends the connection. With no
subscriber the daemon does one branch a loop turn.

**How a subscription ends, and what a script may conclude.** The command
prints whole lines only, and there are endings it can tell apart and one it
cannot:

- A `{"type":"dropped"}` line, the last one (printed, then the command exits
  **1** with a line on stderr saying so): the daemon dropped this
  subscriber and said so. It is sent only when the daemon can write it
  without waiting, which is the rare drop: closing a subscriber when out of
  file descriptors with nothing but subscribers to close. A flood of ordinary
  connections never does it: the oldest *non-subscriber* is closed first, and
  subscribers are at most 4 of the 16 connections the daemon holds.
- A line cut in the middle (exit **1**, stderr says the connection ended in
  the middle of a line): the daemon dropped the subscriber part-way through
  writing a batch. The partial line is discarded, never printed as if whole.
- An error reply to the `subscribe` itself (exit **1**): refused, with why.
- **Anything else is exit 0, and it does not mean the daemon went away.** The
  stream just ends. That is what the daemon exiting looks like, and also what
  a subscriber dropped for being too slow, or stopped (`SIGSTOP`, a hung
  pipe: its socket is full, so there is no room for a `dropped` line) looks
  like, as does a drop whose partial write happened to stop on a line
  boundary. A script cannot tell these apart, so after **any** end, exit 0
  included: subscribe again, then `query`.



## Edge cases

- **Side margins wider than the output** leave the bar no width. It is
  then drawn one pixel wide rather than not at all, on scoot and on sway
  (which sends the negative width it works out as a huge unsigned one;
  scootbar treats any side past `i32::MAX` as "yours to choose"). The zone
  is still reserved.
- **A bar taller than the output** is the compositor's to clamp, and they
  differ: scoot gives the bar the output's height and reserves all of it;
  sway 1.12 configures the full `--height` (1024 on a 720-tall output),
  so the bar runs off the edge, and reserves the whole output too. Either
  way no window has room: this is a value to avoid, not a layout.
- **No flag value can make a buffer overflow**: the bounds keep every size
  far from it. A buffer too large for `wl_shm` (a compositor asking for an
  absurd surface) is refused with a message; the draw is tried a few more
  times, then not again until the size or scale changes.
- **The compositor going away** (it exits, crashes, or sends a protocol
  error) ends the daemon with exit status 1 and one line on stderr saying
  why. There is no reconnect; your session's autostart starts it again with
  the compositor.
- **SIGTERM and SIGINT** end it at once, running no destructor: it keeps no
  state, and the compositor removes its surfaces with the connection. Its
  `exec` commands end with it all the same (the kernel's parent-death
  signal, see `exec` above); only what such a command started in turn is
  left to finish. **SIGHUP keeps its default action** (it ends the daemon)
  rather than reloading: catching one would need `unsafe` signal
  registration, which the crate forbids (`#![forbid(unsafe_code)]`), so a
  reload is `scootbar msg reload`.
- **Vertical bars** (`left` and `right` edges) are not offered: a
  deliberate omission, not a gap. The layout, the modules and the click
  hit-test are horizontal; a vertical bar would be a second layout, not an
  option.
- **One daemon per display** holds the control socket: a second daemon
  for the same display refuses at start-up, saying one already runs. A
  socket file left by a crash is recognised by its free lock and
  replaced; the lock file itself is never removed.
- **Linux only:** it does not build on any other system (the build stops
  with "scootbg-mem, scootbg and scootbar run on Linux only").



## Exit status

| Status | When |
| --- | --- |
| 0 | `--help` or `--version`, or `daemon --check` found nothing wrong |
| 1 | no usable font (with a module placed), cannot connect, the compositor lacks `wl_compositor` v4, `wl_shm` or `zwlr_layer_shell_v1`, the compositor went away, `poll(2)` failed, the config file is malformed (`daemon --check` too), or a `msg` command failed (no daemon, or the daemon refused), or a `msg subscribe` ended on a `dropped` line, in the middle of a line, or on stdout closing |
| 2 | a usage error |
