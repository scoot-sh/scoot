# scootbar command reference

Every `scootbar` command and flag, and how it behaves at the edges. What
scootbar is and why is in [README.md](README.md). Every option lives in
the [config file](#the-config-file) too (`$XDG_CONFIG_HOME/scoot/bar.toml`);
the flags stay and override its values, one by one, at start-up and on
every reload.

**Early days:** the bar shows a clock in the center, and workspaces wherever
they are placed (`--left workspaces`). Modules answer the pointer
([click, scroll and hover](#pointer-input)).

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
scootbar --version
```

`scootbar daemon` runs in the foreground until the compositor goes away
(start it with `&`, or from your compositor's autostart). It connects to
the compositor named by `$WAYLAND_DISPLAY`, which must support
`wlr-layer-shell` (scoot, sway, niri, Hyprland and most wlroots compositors
do).

## `daemon` flags

Each flag at most once, as `--flag VALUE` or `--flag=VALUE`.

| Flag | Takes | Default | What it does |
| --- | --- | --- | --- |
| `--outputs` | `all`, or connector names, comma-separated | `all` | Which outputs get a bar (`DP-1,eDP-1`). See [Outputs](#outputs). |
| `--edge` | `top` or `bottom` | `top` | The output edge the bar runs along. Vertical bars (left and right) are a deliberate omission: the layout is horizontal, and the modules, the hit-testing and the text all assume it. |
| `--layer` | `bottom`, `top` or `overlay` | `top` | The layer-shell layer. See [Layers and the zone](#layers-and-the-zone). |
| `--exclusive` | `true` or `false` | `true` | Whether the bar reserves its height, so windows are arranged beside it, or floats over them, reserving nothing. |
| `--height` | 1 to 1024 | 28 | The bar's height in logical pixels. On a scaled output it is drawn at the output's real pixels: 28 at scale 1.5 is 42 device pixels. |
| `--margin` | one to four of 0 to 1024, comma-separated | `0` | Space between the bar and the output's edges, in logical pixels, in CSS order: `ALL`, `VERTICAL,HORIZONTAL`, `TOP,HORIZONTAL,BOTTOM` or `TOP,RIGHT,BOTTOM,LEFT`. See [Margins](#margins). |
| `--background` | `'#rrggbb'` | `'#1e1e2e'` | The bar's color, six hex digits in either case. Quote it: the shell reads `#` as a comment. Its opacity is the file's `[bar] opacity`: see [Shape and opacity](#shape-and-opacity). |
| `--foreground` | `'#rrggbb'` | `'#cdd6f4'` | The text's color (the theme's `fg` token: see [Colors](#colors)). |
| `--font` | a path | the first [well-known font](#fonts) found | The font file, TrueType or OpenType (`.ttf`, `.otf`; a collection's first face). Any bytes: a path need not be UTF-8. |
| `--font-size` | 1 to 256 | 14 | The text's size, the em, in logical pixels; drawn at the output's real pixels like the bar. |
| `--left`, `--center`, `--right` | module ids, comma-separated, or empty | the clock in the center | The modules along each part of the bar, in order. Giving any of the three sets the whole layout: a part not given is empty, so `--right clock` moves the clock rather than adding a second one. `--center ''` places nothing: a plain bar that needs no font. See [Layout](#layout). |
| `--padding` | 0 to 1024 | 8 | Logical pixels either side of each module's content. |
| `--spacing` | 0 to 1024 | 0 | Logical pixels between neighbouring modules. |
| `--clock-format` | a format, at most 256 bytes | `'%-I:%M %P'` | What the clock shows; see [The clock](#the-clock). |
| `--config` | a path | `$XDG_CONFIG_HOME/scoot/bar.toml` (`~/.config/scoot/bar.toml` without it) | The config file to read instead of the default; see [below](#the-config-file). |
| `--check` | nothing (a switch) | off | Validate and exit instead of running: see [`--check`](#--check-validate-without-running). |

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
[push.status]
placeholder = "..."

[output."eDP-1"]      # what differs on one output: see Outputs below
height = 36
right = ["clock"]
```

`margin` takes an integer (every side) or the `--margin` shorthand
string. `radius`, `opacity`, `separator`, the modules' `margin` and the
pill's keys are file-only: they have no flags (see
[Shape and opacity](#shape-and-opacity), [Spacing](#spacing) and
[the pill](#the-active-workspaces-pill)). The module lists take the ids in [Modules](#modules); giving any
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
(`checks.<system>.scootbar-modules`, [docs/nix.md](../nix.md#the-modules-programsscootbar)).
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
scootbar msg set ID JSON            # write a push module's text, class and tooltip (below)
```

`query` prints one JSON object: one entry per placed module per output
that shows it (an output with no bar, or one whose [table](#outputs) leaves
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
[below](#hiding-the-bar). `query`, `layout`, `version`, `reload`, `hide`, `show` and
`toggle` print the reply; `kill`, `set` and `invoke` print nothing on success,
and `subscribe` prints what the daemon sends until it closes (below). `set`
writes to a [`push` module](#button-push-and-exec-modules): an id that is not
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

## Modules

| Id | Shows | Wakes |
| --- | --- | --- |
| `clock` | The local time ([below](#the-clock)) | its timer: once a minute, or once a second with seconds shown (each redraw adds the compositor's release, below) |
| `workspaces` | Each output's workspace numbers, the active one marked ([below](#workspaces)) | on the compositor's workspace changes: one redraw per batch, however many events it held |
| `window-title` | The focused window's title on the bar's own output ([below](#window-title)) | on the compositor's toplevel changes: focus at once, retitles at most ten times a second |
| `volume` | The default sink's level and mute ([below](#volume)) | on the sound server's sink events, and the server's own: one redraw per batch |
| `microphone` | The default source's level and mute ([below](#volume)) | as `volume`, for sources |
| `battery` | The batteries' charge and state ([below](#battery)) | on the kernel's power-supply events, and once a minute while discharging |
| `network` | The shown interface's state: name, SSID, VPN or offline ([below](#network)) | on the kernel's link, address, route and WiFi events: one redraw per batch, however many events it held |
| `brightness` | The panel backlight's level ([below](#brightness)) | on the kernel's backlight events: one redraw per batch, however many events it held |
| `tray` | The applications' tray icons, StatusNotifierItem ([below](#tray)) | on the session bus's traffic: item registrations, icon changes, owners vanishing |
| `media` | What the players on the session bus are playing, and their controls, over MPRIS ([below](#media)) | on the bus's MPRIS traffic only: a player appearing or vanishing, its track or state changing |
| `bluetooth` | The adapter's power and the connected devices over BlueZ, on the system bus ([below](#bluetooth)) | on the bus's BlueZ traffic only: BlueZ appearing or leaving, an adapter or device coming or going, power, connection, name or charge changing |

A build can leave a module out (`cargo build --no-default-features`, then
`--features clock`); naming one that is not built is a usage error that
lists those that are. Every module takes the five
[interaction keys](#pointer-input). Besides these two, the config can define
modules of its own by name, a [`button`, a `push` and an `exec`
module](#button-push-and-exec-modules), each a Cargo feature (`button`,
`push`, `exec`) on by default. The `popup` feature is not a module either:
it is the [popup](#popups) code the volume and microphone modules use, on
by default, and a build without it has none of it and does not bind
`xdg_wm_base`. The `icon-image` feature adds the PNG
decoder for [image icons](#icons), and is off by default.

## Layout

- **Left** modules are packed from the left edge, **right** ones from the
  right edge, in the order listed; **center** ones are packed together and
  centered on the bar.
- Each module is as wide as its content plus `--padding` on both sides;
  `--spacing` separates neighbours, and a module's own `margin` adds room
  on each side of it ([Spacing](#spacing)). A module with nothing to show
  takes no space at all, padding, margin and spacing included.
- **A rounded bar keeps its ends clear of the corners**: the first module
  on the left and the last on the right start `radius` less half a padding
  in from the bar's end, so their ink (a padding further in) and the
  workspaces pill (half a padding out) are never in a corner square.
- **When they do not fit**, the left part keeps its place, the right part
  gives way to it, and the center part is pushed off center to fit between
  them, then cut. Nothing overlaps and nothing is drawn past the bar's end;
  text is clipped to its module's space.
- Text is vertically centered on the bar. It is not shaped: one glyph per
  character, no ligatures, kerning or right-to-left runs (see
  [Fonts](#fonts) for what is out of scope). A character no font in the chain has
  draws the primary font's missing-glyph box. Control characters are
  not drawn.

## The clock

The local time, in the format `--clock-format` gives, a small `strftime`
subset. The default, `%-I:%M %P`, is a 12-hour clock with no leading zero:
`3:07 pm`. `%H:%M` is the 24-hour one: `15:07`.

| Specifier | Shows | Specifier | Shows |
| --- | --- | --- | --- |
| `%H` | hour, `00`-`23` | `%I` | hour, `01`-`12` |
| `%k` | hour, ` 0`-`23` (space-padded) | `%l` | hour, ` 1`-`12` (space-padded) |
| `%M` | minute, `00`-`59` | `%S` | second, `00`-`59` |
| `%p` | `AM` or `PM` | `%P` | `am` or `pm` |
| `%a` | `Mon` | `%A` | `Monday` |
| `%b`, `%h` | `Sep` | `%B` | `September` |
| `%d` | day, `01`-`31` | `%e` | day, ` 1`-`31` (space-padded) |
| `%m` | month, `01`-`12` | `%j` | day of the year, `001`-`366` |
| `%y` | year, two digits | `%Y` | year |
| `%u` | weekday, `1`-`7` from Monday | `%w` | weekday, `0`-`6` from Sunday |
| `%Z` | zone abbreviation (`EDT`, `+0530`) | `%z` | offset from UTC, `+hhmm` |
| `%R` | `%H:%M` | `%T` | `%H:%M:%S` |
| `%F` | `%Y-%m-%d` | `%D` | `%m/%d/%y` |
| `%%` | a `%` | | |

- **Padding flags**, as GNU `date`: between the `%` and the letter, `-`
  drops a number's padding (`%-d` is `5`), `_` pads it with spaces and `0`
  with zeros (`%_H` is ` 7`).
- **English names, no locale lookup**: day and month names and `AM`/`PM`
  are the C locale's, whatever `LANG` says.
- **Everything else in the format is shown as it is**, any Unicode
  included (as long as the font has it).
- **Its timer fires once a minute** (on the minute), or **once a second**
  when the format shows seconds (`%S`, `%T`): no polling. Each tick that
  changes the text is one frame, and the compositor answers each frame
  with one `wl_buffer.release`, so an idle minute clock wakes the bar twice
  a minute ([What it does on the compositor](#what-it-does-on-the-compositor)).
- **The time zone** is read as glibc reads it: `$TZ` if set (a zone name
  such as `Europe/London`, looked up under `$TZDIR` or
  `/usr/share/zoneinfo`; an absolute path, with or without a leading `:`;
  or a POSIX rule such as `EST5EDT,M3.2.0,M11.1.0`), else `/etc/localtime`.
  An empty `$TZ` is UTC. A POSIX value naming summer time but no dates
  (`TZ=EST5EDT`) takes glibc's default US rules, as `date` does. A zone
  that cannot be read is shown as UTC, with one line on stderr saying
  why; the bar still starts.
- **A new zone shows at the next tick**: `/etc/localtime` (or the file `$TZ`
  names) is checked once each wake, so `timedatectl set-timezone` takes
  effect within the minute. A zone file that disappears keeps the zone last
  read until one is back.
- **Clock steps, suspend and summer time** show at once or on the boundary:
  the timer is on the wall clock and cancelled by any step (NTP, `date
  -s`, a resume from suspend), which wakes the bar at once to redraw; a
  summer-time change shows at the first tick after it.

## Workspaces

Each output's workspace numbers, spoken over `ext-workspace-v1`, with the
active one marked by a pill in the accent color (its number drawn in the
bar's background). One number per workspace the compositor reports for the
output the bar is on, sorted by position: what scoot reports is what is
shown, no fixed slots. A workspace adopted from an unplugged monitor shows
its number (`"2 DP-1"` shows `2`). A name with no leading number (a foreign
compositor's free-form name) shows its 1-based position.

**Click a number to show that workspace**: the bar sends `activate` for it
and `commit`s the batch. Clicking the active one sends nothing. A click on
a non-focused output's bar is dropped by scoot today (it takes no output);
`output-targeted workspace switch` (in the backlog) will carry it across.
On a second monitor's bar this is the one limit of the multi-output
setup: each bar shows its own output's workspaces, but until scoot can
switch a specific output's, only the focused output's bar switches
(`focus-output` first, then click). Nothing in scootbar changes when scoot
gains it: the click is the same `activate`.

- **The pill is square and the bar's full height by default**; its shape
  is [configurable](#the-active-workspaces-pill): rounded, a pill, or a
  circle.
- **The protocol is bound only while the module is placed.** A bar with no
  workspaces module never binds `ext_workspace_manager_v1` or `wl_seat`, so
  the compositor sends it nothing about workspaces and it wakes for none of
  it. A `scootbar msg reload` that adds the module binds both then; one
  that removes it stops the manager, destroys its handles and releases the
  seat and pointer (a seat below version 5 cannot be released and stays
  bound; the descriptor count returns to what it was). A
  compositor that starts advertising the protocol later is bound at that
  point if the module is placed and nothing was bound before. A
  compositor that finishes the manager and later re-advertises it is not
  rebound until a reload removes and re-adds the module.
- **Without `ext-workspace-v1`** the module shows nothing (one stderr note
  at start-up says the protocol is missing) and takes no space.
  **Without `wl_seat`** clicks do nothing (said too: see [Pointer input](#pointer-input)). The bar still starts;
  the clock is unaffected.
- **The list grows and shrinks with the trailing empty workspace**,
  renumbering what follows; only the `active` state bit is shown so far
  (occupied and urgent need scoot-side work, in the module's entry).

### The active workspace's pill

`[workspaces]` shapes the pill behind the active number (all file-only,
logical pixels; the defaults are the square, full-height pill the module
always drew):

| Key | Values | Meaning |
| --- | --- | --- |
| `pill-shape` | `"rect"` (default), `"pill"`, `"circle"` | `rect`: the item's extent, corners per `pill-radius`. `pill`: the same extent with ends as round as fit (half circles). `circle`: at least as wide as tall, centered on the number. |
| `pill-radius` | 0 to 1024 | a `rect`'s corner radius, cut back to half its shorter side. Refused with `pill` or `circle`, which are already as round as they fit. |
| `pill-inset` | 0 to 1024 | the gap from the bar's top and bottom edges, so the pill is shorter than the bar. Cut back so the pill is never shorter than the text's line: the number is drawn in the bar's color over the fill, and a shorter pill would clip it away. |
| `item-gap` | 1 to 8 (default 1) | the spaces between two numbers: one space is about a third of the font size, so 1 is the old tight row and 4 a roomy one. The pill and the click target of each number follow; a click in the gap hits nothing. The module's text is cut at 256 bytes, so a bigger gap shows fewer workspaces: at 8 spaces about 26 numbered up to 99 (28 single-digit ones), the rest cut off the end. |
| `display` | `"numbers"` (default), `"dots"` | `dots`: a dot per workspace in place of the numbers — the active one filled like the pill, the rest dim (or the state colors above). Clicks land by the dots' places, exactly as by the numbers'. A `query` still reports the numbers. |
| `disc` | off by default | with a `circle` pill: grow the module's own span to the disc's diameter, so a single digit is a disc at any `padding`. Refused with any other shape, and showing dots. |
| `active-color` | a color, unset by default | the active pill's fill instead of `accent`. The pointer's `hover` still wins while it is over the module. |
| `inactive-color` | a color, unset by default | inactive numbers' ink instead of `fg` (inactive dots are `dim` unless set). |

```toml
# A rounded pill, lifted 4 off the bar's edges
[workspaces]
pill-shape = "pill"
pill-inset = 4
```

```toml
# More room between the numbers (each number's pill and click target follow)
[workspaces]
item-gap = 4
```

```toml
# A circle around a one-digit number
[bar]
padding = 10           # the circle can be as wide as the module: see below
[workspaces]
pill-shape = "circle"
pill-inset = 3
```

- **How a circle is sized**: its diameter is the pill's height (the bar's
  height less twice the inset). A **two-digit number widens the circle into
  a pill** as wide as the text needs, never a clipped disc. Growth is
  limited to the room around the number: it stops at a neighbouring
  number's ink, and cannot pass the module's own span (the numbers plus
  `padding` either side). So on a crowded bar, or one whose `padding` is
  small next to its height, a single digit gets an oval rather than a
  disc: raise `padding` or the `pill-inset` until it is round, or set
  `disc` to grow the span to the disc instead.
- **Clicks follow the pill**: a press on the drawn pill of the active
  number does nothing (it is already shown), never a neighbour's switch,
  even where a grown circle overlaps the neighbour's own click area; a press
  on the padding around any other number still activates that workspace.
- The corners are the same analytic coverage as the bar's, with no
  supersampling and no allocation, painted only when the module repaints
  (a workspace change). Measured (release build, one pill 80 device pixels
  wide on a 1600x28 bar): the default square full-height pill costs 151 ns
  (the plain fill it replaced, 153 ns); a rounded pill inset by a quarter of
  the height costs 6.3 us, and 20 us on 3200x56 (scale 2), once per
  workspace change. Dots cost one small maximally-rounded fill each, about
  6 us at a 50-pixel test em (linear in the count: 25/49/100 us for 4/8/16
  on the dev VM); at real sizes the discs cover an order of magnitude fewer
  pixels. A grown disc costs its bigger fill, about 8.5 us at the same test
  scale against 5 us for the plain pill. Neither adds a file descriptor, a
  timer or a wakeup: idle costs nothing new.
- **Not built**: urgent and occupied workspace
  colors. The protocol has an `urgent` bit (`ext-workspace-v1`), but the bar
  ignores it (`daemon/workspaces.rs` reads only `Active`) and scoot never
  sends it (nothing marks a window demanding attention yet: no
  `xdg_activation` support) — so there is nothing to drive the colors with.
  The work is read-the-bit plus send-the-bit.

## Window title

The focused window's title on the output the bar is on, spoken over
`wlr-foreign-toplevel-management-v1`. Each output shows the title of the
window activated on it; with none focused the module shows its
`placeholder` (empty by default, taking no space at all).

- **Title and app id.** The title alone, or `title - app` with
  `window-title.show-app-id`; a window with an empty title shows its app
  id, so something is shown whenever a window is focused. The full title
  is the tooltip (and the `query` value's `title`), uncut by the span.
- **Width.** The span never grows past `window-title.max-width` (default
  480 logical pixels, 1 to 4096), so the title yields the bar to the other
  modules; a longer title is cut with an ellipsis by measured pixel
  width, never counted in characters.
- **Click to focus.** A left click activates the window (as
  `scootbar msg invoke window-title activate` does). A middle click closes
  it only with `window-title.allow-close = true` (off by default: closing
  a window by an accidental click loses work); naming `close` in a binding
  with closing off is refused when the file is read. The module's own
  actions are `activate` and `close`, both taking no number.
- **Cost.** Event-driven, no polling: the protocol is bound only while
  the module is placed (a bar with no window-title module is never told a
  title), and without it the module shows nothing and takes no space. A
  retitle flood draws about ten times a second — focus, output, close and
  fullscreen changes always draw at once; only title and app-id text waits
  — and only this module's span is redrawn. Titles are untrusted text:
  kept at 256 bytes, control characters stripped before they reach the
  view (and the glyph cache).
- **Only the wlr protocol.** `ext-foreign-toplevel-list-v1` has no
  `activated` state, no output events and no requests, and the two lists
  share no client-visible key, so a bar cannot correlate their handles;
  the module binds only the wlr manager.

## Volume

The default sink's level and mute, spoken over the PulseAudio native
protocol (PipeWire's `pipewire-pulse` answers it, as does PulseAudio
itself), with no libpulse and no child process: one client on the bar's
own `poll` loop. The `microphone` module is the same code for the default
source (its level and mute, the `source` key in `query`), configured under
`[microphone]`. Each is a Cargo feature (`volume`, `microphone`), both on
by default.

- **What it shows** is `49%` with the level's icon (muted, low, medium,
  high; a static `icon`, `icon-path` or `icon-image` in the table replaces
  all four), in the `muted` class while muted. The tooltip names the
  device: `Built-in Audio: 49%`, with `(muted)` after it. While no server
  answers it shows nothing and takes no space.
- **A click toggles mute, a scroll raises or lowers**, with no binding at
  all; a binding you set runs instead, as on every module. The module's
  own actions are `raise`, `lower` and `toggle-mute`, none taking a number
  (`scootbar msg invoke volume raise` raises one step), `set N`, which sets
  the level to `N` percent (held to 0 and `max-volume`, whatever number is
  given), and `popup`, which opens the [slider popup](#popups)
  (`on-click = "popup"`). A scroll's steps
  arrive through the scroll itself, one step each. There is no default for
  a right click: point `on-right-click` at a mixer (`pavucontrol`, or
  `wpctl`).
- **`step`** (default 5, 1 to 50) is the percent points per scroll notch
  and per raise. **`max-volume`** (default 100, 100 to 150) is the cap a
  raise stops at: 100 is full scale, past it is over-amplification.
- **Cost.** One socket while the server is up, one inotify watch on its
  directory while it is not: no timer, no polling, no wakeups with no
  audio activity. A server that restarts is found when its socket comes
  back, or at once when it kept the path and only dropped the connection
  (one probe, after the watch is armed). A server that refuses the
  handshake (a cookie it does not accept) is not probed again until
  something new happens in the socket's directory, so a standing refusal
  costs no connect loop. Sets are absolute levels from what is shown
  (never accumulated steps, so a touchpad flood cannot drift), one in
  flight at most, and every answered set is re-read, so what the server
  clamped to is what is shown. The server is `$PULSE_SERVER` when that names a unix socket,
  else `$XDG_RUNTIME_DIR/pulse/native`; device names from it are untrusted
  text, kept at 128 bytes.

## Network

Link state, WiFi name and signal, spoken over rtnetlink and nl80211 with
no daemon and no child process: two netlink sockets on the bar's own
`poll` loop. A Cargo feature (`network`), on by default.

- **What it shows** is one interface's state: the default route's (the
  first usable, v4 before v6), or `network.interface` by name. Ethernet shows the name
  (`eth0`); WiFi shows the SSID (`Wimbly`); a tunnel shows `VPN` (in the normal
  class: being on a VPN is not a warning, only `offline` warns); anything
  without an address shows `offline`, in the `warn` class. A second VPN
  up beside the shown interface appends `· VPN`. The tooltip adds the
  signal (`Wimbly · −54 dBm on wlan0`). Before the first event it shows
  nothing and takes no space. An icon can stand before the text, or
  alone: `icon`, `icon-path` with `icon-viewbox`, or `icon-image` (at
  most one, as the [clock's](#icons)) for every state, or one glyph each
  in `icon-ethernet`, `icon-wifi`, `icon-vpn` and `icon-offline` — a
  state with its own glyph shows it, any other state the static icon.
  `icon-wifi` takes four glyphs as well as one
  (`icon-wifi = ["\U000f091f", "\U000f0922", "\U000f0925", "\U000f0928"]`, weakest to
  strongest, Nerd Font's wifi-strength-1..4): the level picks one (four at −55 dBm and better, one at −78
  and worse, as `query`'s `bars` counts them), while a single glyph shows
  at every level. The four must be exactly four, each one glyph, or the
  config is refused naming the key. A static `icon-path`/`icon-image`
  icon stays static: per-level vector or PNG icons are out of scope.
  `show-text = false` draws only the icon, with the text moved into the
  tooltip (the WiFi and tunnel tooltips already name what the text does;
  the dBm is the icon's level, finer).
- **A click opens the picker**, with no binding at all: `network.menu-command`
  is spawned with the cached scan's SSIDs on stdin (one per line), a
  dmenu-style launcher fed from the scan list. Connecting is the
  command's own business, for example
  `menu-command = ["sh", "-c", "fuzzel --dmenu | xargs -d '\\n' -r -n1 nmcli device wifi connect"]`
  (no shell sees the SSID: it travels by pipe into `xargs`, which passes
  it as one argument). With `show-ssid = false` the picker refuses to
  open instead: the scan list would expose the SSIDs the bar hides. The module's own action is `menu`, taking no
  number (`scootbar msg invoke network menu`); it is refused naming why
  with no command configured or no networks seen.
- **A native list**, opt-in as the volume popup is: `on-click = "popup"`
  opens the scan as a [popup](#popups) list instead of the dmenu picker
  (the click binding wins over the picker's default). Each row names a
  network, starting with its strength glyph where `icon-wifi` names four
  glyphs and the scan carries a signal (a row with no signal carries
  none), and the
  associated one selected; a wheel over the list scrolls it a row a notch
  where the popup is taller than what the compositor configures for it,
  and a row too wide for it is cut with an ellipsis. Selecting a
  row closes the popup and runs `connect N` (`scootbar msg invoke network
  connect 1` connects to the list's row 1): `network.connect-command` is
  spawned with the SSID as its last argument, never through a shell
  (SSIDs are attacker-controlled radio data, so no byte in one starts a
  command; a password prompt is the command's own business, as with the
  picker). The argument is the SSID as the row shows it, the same text
  the picker is fed: invalid UTF-8 and control characters are replaced
  with U+FFFD, so a network whose name has such bytes reaches the command
  under that shown name and the connect fails, never landing on another
  network. The list holds at most 16 rows (the popup's widget limit): a
  scan with more shows the first 16, and the picker still lists them all. `N` names what the list showed: a scan that moved underneath
  is refused rather than connected to the wrong network. `connect` is
  refused naming why with no command configured, a row past the list or
  gone from the scan, or while one runs. With `show-ssid = false` the list
  stays closed, as the picker does.
- **`interface`** (1 to 15 bytes, a kernel interface name) pins what is
  shown; absent is the default route's, tracked by index so a rename
  keeps it. **`show-ssid`** (default true) hides the SSID when false —
  the bar shows `WiFi`, and `query` omits the SSID — because the bar
  is visible in screenshots and to an agent's `query`.
- **`query`** reports `{"state": "wifi", "ssid": "Wimbly", "signal": -54,
  "bars": 4, "interface": "wlan0", "vpn": false}`, `"ethernet"` and
  `"vpn"` with the interface, or `{"state": "disconnected"}`.
- **Cost.** Event-driven, no polling: link, address, route and (where the
  kernel lets the socket join them) `scan`/`mlme` multicast groups; a
  flapping link drains into one redraw per turn. Signal strength has no
  event on drivers without CQM thresholds, so a timer re-reads it every
  10 seconds — armed only while a WiFi network is shown, nowhere else —
  and connect, disconnect and roam stay events. Idle wakeups are only
  real network events. A missed burst, a dead socket or a resume
  re-dumps everything; `Unavailable` (and nothing shown) where the
  machine has no network interface — interfaces that appear later (a
  plugged-in dongle) are picked up by `scootbar msg reload`. Interface names and SSIDs are
  untrusted text: sanitized once, at parse time.

## Brightness

The panel backlight's level in percent, read from `/sys/class/backlight`
and woken by the kernel's uevents on a netlink socket filtered to the
`backlight` subsystem. A Cargo feature (`brightness`), on by default.

- **What it shows** is `49%` (`251` of `509` on the reference machine, an
  M2 Air). The tooltip names the device: `apple-panel-bl: 49%`. Where
  there is no backlight at all (a desktop, a VM) it shows nothing and
  takes no space, owning no fd.
- **A scroll raises or lowers**, with no binding at all; a binding you set
  runs instead, as on every module. The module's own actions are `raise`
  and `lower`, neither taking a number (a scroll's steps arrive through
  the scroll itself, one step each), and `set`, taking the absolute
  percent (`scootbar msg invoke brightness set 50`). There is no default
  for a click: there is nothing to toggle.
- **`device`** names the backlight where the machine has several
  (`intel_backlight` beside `acpi_video0`); absent is the first usable one
  in sorted name order. **`step`** (default 5, 1 to 50) is the percent
  points per scroll notch and per raise.
- **Writes need permission**: the bar writes the raw value to the device's
  `brightness` file directly (no daemon, no child; logind's `SetBrightness`
  waits for the shared D-Bus client), and a udev rule for the backlight
  class or the `video` group grants it. Without it the action is refused
  naming that. Every write is an absolute level from what is shown (never
  accumulated steps, so a touchpad flood cannot drift), re-read before the
  call returns, and never below raw 1: 0 blanks the panel on the drivers
  measured, and the bar never darkens its own screen past what a scroll
  can bring back.
- **Cost.** The uevent socket, and nothing else: no timer, no polling, no
  wakeups with no backlight activity. A uevent storm drains into one
  re-read per turn; the uevent a write itself emits finds the re-read
  already done. Sysfs values are untrusted text: read once each into
  fixed buffers, digits only, a zero range or a missing file skipping its
  device.

## Battery

The batteries' charge in percent, read from `/sys/class/power_supply`,
woken by the kernel's uevents on a netlink socket (group 1). The
subsystem filter is in userspace: the bar wakes on every kernel uevent
and drops those that are not `power_supply`. A Cargo feature (`battery`), on by default.

- **What it shows** is `72%`, in the `warn` class at or below
  `warn-below` (default 20) and `urgent` at or below `urgent-below`
  (default 10, 0 to 100 each), by level alone; `warn-below` must be at
  least `urgent-below`, else warn is unreachable and the file is refused. The tooltip names the
  state: `Discharging 72%` (`Charging`, `Full`, `Not charging` or
  `Unknown` for a status string no kernel documents, which never refuses
  the battery). Where there is no battery at all (a desktop, a VM) it
  shows nothing and takes no space, owning no fd.
- **Several batteries** are combined by default (`batteries =
  "combine"`: the mean capacity, discharging winning the state), or the
  first in sorted name order with `batteries = "first"`. A battery
  removed at runtime hides the module until one is back; the uevent socket
  stays as the appearance watch, so a reinsert shows again with no
  polling. The percentage always comes from `capacity`, never from
  `charge_now`/`charge_full` (on the reference machine the two disagree
  by four points at full). No time-remaining is shown: it needs rate
  smoothing to be honest, and a wrong estimate is worse than none.
- **`on-low = { exec = [...] }`** runs once per downward crossing of
  `urgent-below` (and re-arms when the level rises back above, so the next
  crossing fires again; starting below it is not a crossing). It runs
  through the bar's own spawner, like a click binding: bounded and
  reaped, never through a shell. The module defines no actions of its
  own, so its five interaction keys take commands only.
- **Cost.** The uevent socket always (one wake per kernel event, however
  many datagrams arrive, whatever their subsystem: a storm is drained and
  re-read once per turn), and a timerfd re-reading once a minute only
  while discharging. Measured on the Asahi M2: plug and unplug each emit a
  burst of `power_supply` uevents, and capacity steps while discharging
  emit none (five steps over 62 minutes, zero uevents), so the discharge
  timer is what sees them. Other drivers may differ. Charging, full and
  absent batteries own no timer. Sysfs files are read once each into fixed buffers; a
  capacity past 100 is clamped, an unparsable one skips its battery,
  and a removed battery is one line on stderr, not one per wake.

## Tray

The applications' tray icons: the StatusNotifierItem watcher and host over
the session bus, one icon per item, drawn at the output's real device
pixels. A Cargo feature (`tray`), on by default; the smallest build
(`--no-default-features`) has none of it. It speaks D-Bus through the
bar's own client (`src/dbus`: no `zbus`, no libdbus, no thread; its
socket is one more source in the `poll` loop), which is the
[spike's](spikes/dbus-client.md) hand-rolled choice.

- **The bar is the watcher.** It owns `org.kde.StatusNotifierWatcher` (and
  the `org.freedesktop` twin) when they are free, answers apps'
  registrations itself, and re-takes the name if its owner leaves. Where
  another process already owns it (a desktop's own tray), the bar hosts
  against that watcher instead: its items still appear, a click still
  works, and the bar takes over when that owner goes. Items that
  registered before the bar started are found by listing the bus's names
  at connect (an item behind a plain unique name registers explicitly, and
  is only seen if it registers after the bar is up: the KDE watcher's own
  limit). With no session bus the module shows nothing and waits on the
  bus socket's directory (one inotify watch, no polling), and a bus that
  dies mid-run drops every icon at once and dials once more. A bus that
  takes the bar in and drops it within five seconds, three times running,
  is not dialled again at once (a refusing bus must not spin the bar): it
  is tried once more after 30 seconds, and again each time that try dies
  the same way, or as soon as its socket is made anew. Only the bus's own
  `NameOwnerChanged` is believed (a peer's, addressed to the bar, removes
  nothing), and when hosting only the watcher hosted against speaks for it.
  The address is `DBUS_SESSION_BUS_ADDRESS`'s `unix:path=` (`%xx` escapes
  decoded), else `$XDG_RUNTIME_DIR/bus` when it is not set; an address
  with no path (`unix:abstract=`, `tcp:`, `autolaunch:`) is refused with a
  line on stderr and no tray, not replaced by another bus that happens to
  be at the default place. A bus that refuses the bar the watcher name (a
  policy that denies `own`) is said once on stderr, and the bar hosts.
- **What is drawn.** An item's `IconPixmap` (raw `ARGB32` over the bus),
  picked at the output's device size from the entries sent and scaled
  only when none matches, from the shared icon cache: a steady bar
  re-reads nothing. An item whose `Status` is `Passive` (the spec's "hide
  me"), or that sends only an icon *name* (themed icons need an icon-theme
  lookup and an image decoder, which this build does not have), is tracked
  and reachable by index but takes no room. Attention and overlay icons,
  tooltip icons and `IconThemePath` are read for shape and not drawn. The
  tooltip over the module lists the shown items' titles.
- **Clicks, with no binding at all**: a left click is `activate`, a middle
  click `secondary` (the spec's `SecondaryActivate`), a scroll `wheel-up`
  or `wheel-down`, each on the item under the pointer. A right click does
  nothing by default; the item's menu (`ContextMenu`, the DBusMenu
  protocol) is not built: [popups](#popups) exist to draw one in, but the
  DBusMenu client that reads a layout (`GetLayout`, `Event`,
  `AboutToShow`, the update signals) and maps it onto popup content does
  not. The module's actions all take the item's index (`scootbar msg invoke tray activate
  0`), in the order `query` lists them: `activate`, `secondary`,
  `wheel-up`, `wheel-down`, and `menu`, which is refused saying menus are
  not built.
  (The wheel actions are not called `scroll-up` and `scroll-down`: those
  are the names of the interaction keys, and `msg invoke` reads them as
  those.) `Activate` and `SecondaryActivate` are sent with position `(0,
  0)` (a bar has no screen coordinates to give), and a wheel action sends
  `Scroll(n, "vertical")` with `n` the notch count clamped to 64, positive
  for `wheel-up` and negative for `wheel-down`: the sign is KDE's (a Qt
  wheel's), and hosts disagree (Waybar sends GTK's, up negative); no item
  was checked against it. Bound to a key (`on-scroll-up = "wheel-up 0"`)
  the notch count of the scroll itself is what is sent.
- **`query`** reports `{ "watcher": "owner" | "host", "items": [{ "id",
  "title", "status", "shown" }] }` while any item is tracked, and nothing
  while none is.
- **Margin and keys.** `[tray]` takes `margin` and the five
  [interaction keys](#pointer-input) (`on-click`, `on-right-click`,
  `on-middle-click`, `on-scroll-up`, `on-scroll-down`); the module has no
  options of its own. A binding replaces the default for its trigger, as
  on every module.
- **Bounds, for a hostile or broken item.** Everything an item says is
  untrusted bytes from a same-user peer: a message past 1 MiB (the spec
  allows 128 MiB, and SNI cannot ask an item for a size, so a 512 by 512
  pixmap is just over) is skipped whole as it arrives: the answer is
  dropped, the item keeps its last state and is read again at its next
  signal, and the connection lives; a header that is no message, or past
  128 MiB, ends the connection. Nesting past 32 and pixmaps past 256
  pixels a side are refused the same way (the entry, or the answer, is
  dropped). A flood of signals (the match rule has no sender, so any peer
   may send what the bar listens for) is read up to 1 MiB + 64 KiB staged
   (the read watermark: one capped message and a read's worth) and the
   rest left in the socket, 256 events a wake with the bar's other sources
   between, and costs the connection nothing; while an over-cap message
   is discarded, one turn reads at most 256 KiB more (the poll is woken
   for the rest), so a sender that outruns the reader holds one turn,
   not the bar. Titles are cut to 128 bytes
  with controls stripped; at most 32 items, 8 from one service or one registrant, 8 pixmap
  entries each; one `GetAll` in flight per item however many signals it
  sends; a call nobody answers is forgotten after 30 seconds when its slot
  is wanted (no bus times a call out by default, measured on a stock
  `dbus-daemon` and on `dbus-broker`; only a client library does). A bus that stops reading drops the connection, and
  not the bar. The bus
  set-up (auth and `Hello`) is blocking, bounded to 2 seconds in total.
  The parser is fuzzed (`crates/scootbar/fuzz`, target `dbus`).
- **Cost.** One fd (the bus socket, with `OUT` only while a write or a
  staged message waits), or one inotify fd while there is no bus, and a
  one-shot timer only while an item waits out its 50 ms floor between
  reads or after the bus kept dropping the bar. Measured (dev VM, one
  60 s idle window per row): **zero wakeups** with no bus, with a bus and
  no items, and with one and with eight items on it; RSS 4156 kB with the
   tray alone and no bus or no items, 4352 kB with one item, 4388 kB with
   eight (differences under about 130 kB are within one run's resolution;
   level with the same tree's `main` in every row). The binary: **no byte
   on disk and +2,496 B of `.text` (+0.2%)** against `main` (2,036,448 B
   on disk both sides). An item that
  re-announces its icon continuously is read at most every 50 ms (0.7% of
  a core measured, against 9.7% with no floor). The table and its method
  are in the
  [resource ratchet](backlog/lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02);
  the hardening and link rows are in
  [the same file](backlog/lightest.md#m6-tray-hardening-and-one-bus-lifecycle-measured-2026-10-03).

## Media

What the players on the session bus are playing, and play/pause, next and
previous, spoken over MPRIS (`org.mpris.MediaPlayer2.*`: mpv, VLC,
Spotify, Firefox, Chromium and most others) through the bar's own D-Bus
client (see [Tray](#tray)): no `zbus`, no libdbus, no thread, no polling. A
Cargo feature (`media`), on by default; the smallest build
(`--no-default-features`) has none of it.

- **What it shows** is `artist - title` (or whichever of the two the player
  sent, or the player's name when it sent neither) with a play or pause
  icon for the state, cut to `max-width` with an ellipsis measured in
  pixels, in the `muted` class while paused. The tooltip over the module (after the bar's
  `tooltip-delay`, as every module's; see [Tooltips](#tooltips)) is the uncut
  line with the player's name: `mpv (playing): Ada - Song`. Several artists are joined with a
  comma. **A stopped player shows nothing**, and with no player the module
  takes no space; start playback from the player.
- **Which player.** Several may run. Of the ones that are playing or
  paused, `player` (a short name: `spotify` for
  `org.mpris.MediaPlayer2.spotify`, and for its second copies, which a
  player names `.instance` and a suffix: `.instance1234`, `.instance-abc`) is
  shown if it is among them; else the one that most recently started
  playing; else, when none plays, the one that played last; a tie (players
  that never played) is broken by name, so the choice is the same every
  run. The controls go to the player shown. A connection that owns two MPRIS
  names is one player (the first name seen; if it releases that one, the
  other takes over); at most 8 are held (see Bounds for what a ninth does).
- **Controls, with no binding at all**: a click is `play-pause`, a right
  click or a scroll down `next`, a middle click or a scroll up `previous`;
  nothing happens when no player is shown. The module's actions are
  `play-pause`, `next` and `previous`, none taking a number
  (`scootbar msg invoke media next`). Each is one call to the player's
  connection that wants no reply: the track changes on the bar when the
  player says so, as a signal. An action is refused saying why when there
  is no bus, no player playing or paused, or the player says it cannot
  (`CanControl`, `CanGoNext`, `CanGoPrevious` false). **A skip within 250 ms
  of the last is refused** (a scroll arrives at up to sixty actions a
  second, and a scroll of thirty steps is one skip, not thirty; a scroll
  that hits the limit is one line on stderr a second at most, as any failing
  action): an agent that wants two skips waits for the first to show in
  `query`.
- **`query`** reports `{ "player", "bus_name", "status", "title", "artist",
  "players": [{ "bus_name", "status" }] }` for the player shown (`status` is
  `playing` or `paused`; `players` lists every one held, stopped included),
  and nothing while none is.
- **Options.** `[media]` takes `player` (a name as it is on the bus:
  letters, digits, `_`, `-` and `.`), `max-width` (default 320 logical
  pixels, 1 to 4096), `margin` and the five [interaction keys](#pointer-input).
  The playback position and the volume are never shown (the position has no
  change signal, so showing it would need a timer), and the art URL a player
  names is never fetched.
- **A run of changes is drawn ten times a second at most.** The first
  change after a quiet spell (a new track) is drawn in the turn it arrives;
  changes that follow within 100 ms (a title, a state flipping between
  playing and paused, another player becoming the one shown) wait for one
  timer, and the bar then shows the latest, however many came; the last
  change is never lost. The module appearing (a first player) or emptying
  (the last one gone or stopped, the bus lost) is never held. (The window
  title's rule, for the same reason: a player that rewrites its title or
  flaps its state hundreds of times a second is a bug or a title that
  carries progress.)
- **Idle cost: nothing.** The bus does the filtering: the bar asks for
  `NameOwnerChanged` of the `org.mpris.MediaPlayer2` namespace and for
  `PropertiesChanged` of the Player interface on the one MPRIS object, so an
  app unrelated to media coming or going, or a player's `Seeked` or
  position, never wakes the bar (tested against a real `dbus-daemon`). A
  track change is one signal carrying the value (no round trip); a player
  that only invalidates a property is read once, no oftener than every
  50 ms. With no bus the module waits on the bus socket's directory (one
  inotify watch); a bus that goes away drops every player at once and dials
  once more, and one that keeps dropping the bar is left alone for 30
  seconds, said once on stderr and again at each 30 s retry that dies the
  same way (the [tray's](#tray) rules, in `src/dbus/link.rs`).
- **Bounds, for a hostile or broken player.** Anything on the session bus
  can claim a player name and say anything. What the code guarantees: it
  cannot crash or hang the bar or another player, or grow the bar without
   bound; only the bus's own `NameOwnerChanged` is believed, and a
   `PropertiesChanged` only from the connection that owns a held name. (Method
   replies are matched by serial *and* sender: measured 2026-10-03,
   `dbus-daemon` 1.16.2 delivers an unsolicited reply from a peer that was
   not the callee, while `dbus-broker` 37 does not, so a reply whose sender
   is not the callee is refused whenever the callee is known — the bus's
   own calls, or a unique name's. What remains is a call made of a
   well-known name, whose holder the client cannot know: a forged answer
   to one of those is accepted, like any peer's own claim to the name.) A hostile peer costs the bar the work of its own
  signals and no more (a flood of positions about 0.6% of a core at 500 a
  second, measured; title and state changes are drawn ten times a second at
  most); it holds at most one of 8 slots, one a connection. What it *can*
  still do is cost visibility, boundedly: connections that keep 8 live
  players (playing or paused) fill the table, and 16 more names announcing
  after them fill the waiting list, so a player that arrives behind all of
  those is not shown until a slot frees, and the oldest waiting name is
  forgotten past 16; that is a loss of what the module shows, never of the
  bar. The rule: at most 8 players are held, one a connection. When all 8
  are held a newcomer takes the place of the oldest *stopped* player whose
  read has been answered or has errored (it shows nothing; it goes to the
  waiting list flagged as evicted), else, with no such player, it waits in
  the list of 16 names (a 17th forgets the oldest; a name is dropped from
  it when it loses its owner and re-keyed when it changes hands). A freed
  slot gives each waiting name one attempt to be held (bounded: nothing
  re-lists the bus), and a held player that stops swaps in one waiting name
  that was not itself evicted; an evicted name comes back only through a
  freed slot, so an evicted player that starts playing is not seen until
  some held player is removed (this matters at 9 or more MPRIS names). The second
  name of a connection waits in the same list, so a connection that releases
  one keeps its player under the other. A connection owning hundreds of
  names is asked about in a window of 8 at a time, and each is held or waits
  by the same rule. Titles and artists are cleaned (controls stripped) and
  cut to 120 bytes where they are stored. A message past 1 MiB is skipped
  whole and the player keeps its last state; an answer that does not parse
  is dropped whole; a read that errors (a timeout, or `UnknownObject` from a
  player that has the name before it exports the object) leaves the player
  held, shown as nothing, and read again at its next signal; one that never
  answers is forgotten after 30 seconds when its slot is wanted; a property
  of the wrong type is skipped alone; dictionaries past 128 entries are
  refused. (`ListNames` is read to 4096 names before the MPRIS filter: a bus
  holding more can hide a player from the start-up listing, though not from
  its `NameOwnerChanged`.) The parser (`src/dbus/mpris.rs`, `std` only) is
  fuzzed with the D-Bus client's (`crates/scootbar/fuzz`, target `dbus`), and
  checked against what sd-bus marshals.
- **Cost.** One fd (the bus socket, with `OUT` only while a write or a
  staged message waits), or one inotify fd while there is no bus, and a
  one-shot timer only while a player waits out its 50 ms floor between reads,
  while a change of title waits out its 100 ms between draws, or (30 s) after
  the bus kept dropping the bar.
  Measured (dev VM, one 60 s idle window per row): **zero wakeups** with no
  bus, with a bus and no player, with one player paused, with one playing,
  with eight playing, and with a real mpv (its MPRIS script) playing a file;
  one thread throughout; RSS 4156 kB with a bus and no player, 4388 kB with
  one playing player, 4392 kB with eight (differences under about 130 kB
  are within one run's resolution; the clock alone is 4236 kB). A player
  that signals constantly pays for itself and no more: a stub signalling its
  position 500 times a second for 20 s (10,143 signals) cost the bar 0.6% of
  a core, one sending a new title 480 times a second 0.85%, and one flipping
  between playing and paused 500 times a second 0.75%, RSS flat in all three
  (the tests pin that a position draws and reads nothing, and that title and
  state changes are drawn ten times a second at most). A player that
  re-sends unchanged metadata (mpv playing its synthetic `lavfi` source
  does, once a second) wakes the bar once a second and draws nothing. The
  binary: **+65,536 B on disk (1,905,376 to 1,970,912, +3.4%) and +52,912 B
  of loaded sections (+2.9%, of which `.text` +45,376 B)** against `main`;
  the feature built but off is +3,096 B loaded and no more on disk. The
  maintainer waived this row on 2026-10-03 (this row only) and `media`
  stays in `default`. The table and its method are in the
  [resource ratchet](backlog/lightest.md#m6-media-module-level-cost-measured-2026-10-03).

## Bluetooth

The adapter's power and the connected devices, spoken over BlueZ
(`org.bluez`) on the **system bus** through the bar's own D-Bus client
(see [Tray](#tray)): no `zbus`, no libdbus, no thread, no polling. The
system bus is `DBUS_SYSTEM_BUS_ADDRESS` when it names a filesystem path,
else `/run/dbus/system_bus_socket` (`src/dbus/conn.rs`); the client
authenticates the same `EXTERNAL` way on both. A Cargo feature
(`bluetooth`), on by default; the smallest build (`--no-default-features`)
has none of it.

- **What it shows** is the first connected device's name in path order
  (with its charge when BlueZ reports one: `Headset 72%`), `on` while an
  adapter is powered with nothing connected, and `off`, in the `muted`
  class, while every adapter is off. Adapter power dominates a device that
  still claims to be connected (its disconnect is on its way). With no
  adapter, or no BlueZ at all, the module shows nothing and takes no
  space. The tooltip lists every connected device with its charge.
- **Which device.** Several may be connected; the one shown is the first
  in path order, so the choice is the same every run. The toggle goes to
  the first adapter in path order.
- **A click toggles the first adapter's power, with no binding at all**:
  one `Set` of `Adapter1.Powered` that wants no reply (the state changes
  on the bar when BlueZ says so, as a signal); nothing happens with no
  adapter. The module's actions are `toggle` and `menu`, neither taking a
  number (`scootbar msg invoke bluetooth toggle`).   `menu` opens the
  picker, a dmenu-style command fed the device list (see below); it is
  refused naming why with no command configured, no system bus, no
  adapter, or no devices seen yet.
- **Picking a device** is `bluetooth.menu-command`, spawned with the
  device list on stdin (one per line, a connected device marked
  `(connected)`), a dmenu-style launcher fed from the held set, e.g.
  `menu-command = ["sh", "-c", "fuzzel --dmenu | ..."]`. Connecting is the
  command's own business; the bar never reads the choice back. The picker
  is the interim path, as the network module's is: a native list waits for
  the popups to grow one (see the network module's entry).
- **`query`** reports `{"state": "connected", "adapters": 1, "powered":
  true, "connected": 1, "device": "Headset", "battery": 72}` (`state` is
  `off`, `on` or `connected`; `device` is the device shown; `battery`
  only when BlueZ reports one), and nothing while there is no adapter.
- **Options.** `[bluetooth]` takes `menu-command` (no empty argument),
  `margin` and the five [interaction keys](#pointer-input). The charge is
  shown only when BlueZ reports it (`Battery1`); it is never polled.
- **A run of changes is drawn ten times a second at most**, as the media
  module's: the first change after a quiet spell is drawn in the turn it
  arrives, the rest wait for one 100 ms timer; the module appearing (a
  first adapter) or emptying (the last one gone, BlueZ leaving, the bus
  lost) is never held.
- **Idle cost: nothing.** The bus does the filtering: the bar asks for
  `NameOwnerChanged` of exactly `org.bluez` and for the object-manager
  and `PropertiesChanged` signals under `/org/bluez`, so anything else on
  the system bus never wakes the bar (tested against a real
  `dbus-daemon`). A power or connection change is one signal carrying the
  value (no round trip); an object whose signal only invalidates a shown
  property is read once (`GetAll`), no oftener than every 50 ms. With no
  system bus the module waits on the socket's directory (one inotify
  watch); a bus that goes away drops everything at once and dials once
  more, and one that keeps dropping the bar is left alone for 30 seconds
  (the [tray's](#tray) rules, in `src/dbus/link.rs`).
- **Bounds, for a hostile or missing BlueZ.** Anything on the system bus
  can own `org.bluez` when BlueZ itself is absent and say anything. What
  the code guarantees: it cannot crash or hang the bar or grow it without
  bound; only the bus's own `NameOwnerChanged` is believed, every other
  signal only from the tracked owner of `org.bluez` (checked per signal),
  and only for a held path (a signal for an unknown path re-reads the set
  once on a 50 ms timer instead of trusting it). (Method replies are
  matched by serial *and* sender: measured 2026-10-03, `dbus-daemon`
  1.16.2 delivers an unsolicited reply from a peer that was not the
  callee, while `dbus-broker` 37 does not, so a reply whose sender is not
  the callee is refused whenever the callee is known — the bus's own
  calls, or a unique name's. What remains is a call made of the
  well-known `org.bluez`, whose holder the client cannot know: a forged
  answer to one of those is accepted, like any peer's own claim to the
  name.) A hostile peer costs the bar the
  work of its own signals and no more (connect/disconnect storms are drawn
  ten times a second at most); it holds at most one of 8 adapter slots or
  64 device slots, and a newcomer to a full room is ignored, said once.
  What it *can* still do is cost visibility, boundedly: full tables hide a
  later object until a slot frees. Names (`Name`, else `Alias`, else the
  path's last element) are cleaned (controls stripped) and cut to 120 bytes
  where they are stored. A `GetManagedObjects` answer past 1 MiB is skipped
  whole and the module keeps showing its last state: one retry on the
  coalesce timer, then it reads again at the next signal (never asking at
  once for the same oversized answer in a loop); an answer that does not
  parse is dropped whole; a read that
  errors leaves the last state and is read again at the next signal; one
  that never answers is forgotten after 30 seconds when its slot is
  wanted; a property of the wrong type is skipped alone; dictionaries past
  128 entries, interfaces past 32 of one object, and overlong name lists
  are refused. The parser (`src/dbus/bluez.rs`, `std` only) is fuzzed with
  the D-Bus client's (`crates/scootbar/fuzz`, target `dbus`), and the
  module is exercised against a scripted bus and a fake BlueZ on a real
  `dbus-daemon`.
- **Cost.** One fd (the bus socket, with `OUT` only while a write or a
  staged message waits), or one inotify fd while there is no bus, and a
  one-shot timer only while an object waits out its 50 ms floor between
  reads, while a set re-read waits out its own, or while a change of text
  waits out its 100 ms between draws, or (30 s) after the bus kept
  dropping the bar; a pidfd only while the picker runs.
  Measured (dev VM, one 60 s idle window per row): **zero wakeups** with no
  bus, with a bus and no BlueZ, and with an idle BlueZ (adapter on, one
  device connected, no traffic: the bar shows `Headset 72%` throughout);
  one thread throughout; RSS 4168 kB with a bus and no BlueZ, 4388 kB with
  the idle BlueZ (placing the module costs the font every module needs:
  3880 kB with no module placed, 4312 kB with the clock). A burst of
  10,000 connection signals in under a second (an independent raw-socket
  peer, `bench/m6-bluetooth-vm/scripts/bluez.pl`) costs about 2,000
  wakeups and 0.03 CPU-seconds, RSS flat, and then silence: the draws are
  held at ten a second and identical signals draw nothing (the unit test
  pins at most two draws for 400 alternating flips). The binary:
  **+65,536 B on disk (1,970,912 to 2,036,448, +3.3%) and +66,224 B of
  loaded sections (+3.6%, of which `.text` +58,400 B)** against `main`;
  the feature built but off is +3,032 B loaded and no more on disk. The
  size row is the same shape as every module before it, and the rule's own
  exception covers only a row the module adds, so it is a regression for
  the maintainer to waive or not. The table and its method are in the
  [resource ratchet](backlog/lightest.md#m6-bluetooth-module-level-cost-measured-2026-10-03).


## Pointer input

Clicks, scrolls and hover, on every module. The bar never takes the keyboard
outside a [popup](#popups)'s lifetime (its layer surface asks for no
keyboard interactivity, so it cannot disturb focus; a popup that grabbed
takes it, for Escape, while it is open), and **touch is ignored**: the bar binds the seat's pointer only, so
a touch screen's taps reach nothing here (until a touch design exists,
they are not translated into clicks).

**Interaction keys.** Each module's table takes five keys, each holding one
action:

| Key | Runs on |
| --- | --- |
| `on-click` | a left click |
| `on-right-click` | a right click |
| `on-middle-click` | a middle click |
| `on-scroll-up` | the wheel or a two-finger swipe up |
| `on-scroll-down` | down |

A value is one of:

```toml
on-scroll-down = "next"                          # an action the module defines
on-middle-click = "activate 3"                   # ... with a whole number
on-click = { exec = ["foot", "-e", "btop"] }     # a command, run directly
on-click = { scoot = "quit" }                    # a request to scoot's control socket
```

- **A module's own actions** are named, checked when the file is read (a
  typo is a refusal naming the key and listing what the module has), and
  optionally take one whole number. The `clock` has none; `workspaces` has
  `activate N` (switch to the workspace showing number `N` on that bar's
  output, the first if two show it), `activate-position N` (the `N`th as
  drawn, 1 first: what a click on a number means, exact even where two
  items show one number, such as an adopted `2 DP-1` beside a native 2),
  `previous` and `next` (move the active one, stopping at the ends; a
  scroll of several notches moves that many places).
- **`exec`** is an array, the command then its arguments, at most 32 of
  them of at most 4096 bytes each, none holding a NUL. **It is never run
  through a shell**; write `["sh", "-c", "..."]` to use one, and the
  quoting is yours. A string instead of an array is a refusal that says
  so. The command's stdin, stdout and stderr are `/dev/null`, it leads its
  own process group, and **it inherits none of the file descriptors the
  bar opens** (every one is close-on-exec). What was open in the bar when it
  *started* is not the bar's: a launcher (a shell's `exec 4<file`, a
  service manager, a CI runner) that leaves a descriptor open without
  close-on-exec hands it to the bar, and the bar hands it on to every
  command it launches, as to any child. Its environment is the bar's. It is reaped the moment it
  exits (no zombie, no timer), and at most 8 launched commands run at once:
  a ninth is refused with a line on stderr, not queued, so a hung command
  and a flood of clicks cannot fill the process table. The bar does not
  supervise what it launched: it lives as long as it likes (see
  [the unit's `KillMode`](../nix.md#the-status-bar-scootbar) for what a
  restart of the bar does to it).
- **`{ scoot = "quit" }`** asks scoot to end the session over its control
  socket (`SCOOT_SOCKET`, else `$XDG_RUNTIME_DIR/scoot.sock`) with no
  process spawn: one line, with a 250 ms bound each way on a fresh
  connection, so a wedged scoot cannot hang the bar. Under another
  compositor, or with no session, it says it cannot reach scoot on stderr
  and does nothing. `quit` is the only value.
- **A key you do not set keeps the module's default**: the workspaces
  module's left click on a number switches to it (as it always did), and
  other modules have defaults of their own, which their sections list (the
  [media module's](#media), for one). A binding replaces the default.
- **A failing action** (a program that is not there, a full table) is one
  line on stderr, at most one a second, with a count of the ones held back.
  The bar carries on.

**What a click is.** A button *press* arms the module under the pointer; the
*release* runs the action only if the pointer is still over that same
module. A release anywhere else (the pointer slid off, left the bar, or
the module went away meanwhile: a reload, a module that now shows nothing)
does nothing and leaves nothing armed. A second button pressed while one
is held cancels both. Buttons other than left, right and middle (back,
forward, touch) are ignored. A click lands on the layout that is on screen
(what the last draw committed), so a click during a redraw goes to what you
saw.

**What a scroll is.** Vertical scroll only; horizontal scroll is ignored.
A wheel notch is one step (`axis_value120`, or `axis_discrete` on an older
compositor), and a smooth scroll (a touchpad) adds up to steps at 15 pixels
a step, the remainder carried between events and dropped when the direction
reverses, the scroll ends or the pointer leaves. Down is `on-scroll-down`.
**However fast the device sends events, a scroll binding runs at most once
a frame (16 ms)**, carrying the steps that piled up (at most 32: a flood
past that is dropped, not queued): a module action moves that many
places, and an `exec` command runs once however many steps it covers. While
steps wait the loop sleeps only until the frame is due; with nothing
waiting it sleeps as before.

**Hover.** A module with a binding is drawn in the `hover` color while
the pointer is over it, and only that module's span is redrawn, on that
output's bar alone; moving between modules redraws the one left and the one
entered. A module with no binding is not tinted. The workspaces module
tints its active pill the same way (its pill is drawn by the module
itself, so the tint lands there rather than over its text). Unset,
`hover` follows a custom `accent`; setting it pins the tint.

**Cost.** The bar asks the seat for a pointer only while a placed module
has a binding or a default of its own (today: the workspaces module's
click, the window title's click and the media module's).
A clock-only bar with no bindings never takes the pointer, and costs what
it did before there was any input. A reload that adds or removes bindings
takes or drops it. A motion event stores two numbers; no pointer event,
hover repaint or module action allocates (tests count the allocations).
Launching an `exec` command does, as any process spawn must.

## Popups

A module's **popup** is a small panel drawn in an `xdg_popup` parented to the
bar's layer surface (`zwlr_layer_surface_v1.get_popup`), opened under the
module and gone when closed: nothing of it exists while it is not open, so
a bar that never opens one costs what it did before. The `popup` Cargo
feature (on by default) is the code; a build without it has none of it.

**Opt-in.** Nothing opens a popup until the config binds one. The first
consumer is the volume module (and `microphone`, which shares its code):

```toml
[volume]
on-click = "popup"      # the slider, instead of the default mute
```

Any of the click triggers takes it (`on-right-click = "popup"` keeps the
click for mute); a scroll cannot (it carries no input serial the popup grab
needs), and says so on stderr. `scootbar msg invoke volume popup` opens it
too, **with no grab** (an agent has no input event to grab with): it stays
until invoked again, or any of the endings below, and takes no keyboard.
The network module's picker is unchanged by default (`menu-command` and
the dmenu-style launcher are still how a click connects); `on-click =
"popup"` on the network module opens its native list instead (see
[Network](#network)).

- **What the volume popup shows**: the device's name and level (`Built-in
  Audio  49%`), a slider from 0 to `max-volume`, and a Mute (Unmute) button.
  Pressing the slider sets that level and dragging it follows the pointer, one
  `set N` action per new value (the module coalesces a fast drag into the
  latest, one request in flight); the button runs `toggle-mute` on release.
  It follows the module, so a level changed elsewhere moves the slider, and it
  is drawn in the bar's own colors (the `bg`, `fg`, `accent` and `dim`
  tokens, one pixel frame) at the output's real scale.
- **What the network list shows**: one row per named network in scan
  order (unnamed ones are not rows), each starting with its strength
  glyph where `icon-wifi` names four glyphs and the scan carries a
  signal, and the associated one selected. A wheel over the list
  scrolls it a row a notch where it is taller than what the compositor
  configures for it; a row too wide is cut with an ellipsis, as
  a window title's is. Selecting a row closes the popup and runs
  `connect N` on the network module. Keyboard navigation (arrows, Enter)
  is a [separate entry](backlog/popup-list-keyboard.md).
- **Opened on the press, not the release.** The one exception to "clicks
  fire on release" ([pointer input](#pointer-input)): a compositor may refuse
  a popup grab whose serial is not a button still held (the protocol allows
  it, and some compositors check), and a refused grab never sees a click
  outside or Escape. The release after it does nothing. scoot and sway were
  measured to accept either, so on those the choice is invisible.
- **Where it goes.** Anchored to the module's span on the bar, centered under
  it (above, on a bottom bar), and the compositor slides it along the bar and
  flips it across it where the output's edge would cut it.
- **It closes** on: a click anywhere outside it (the compositor's `popup_done`),
  **Escape**, a press on the bar (so a second click on the module toggles it,
  and a click on another module closes it without acting), its module
  having nothing to show (the sound server went away) or leaving the bar, its
  output being unplugged, the bar being hidden (`msg hide`) or made again,
  a scale change, a `reload`, and, for a popup that grabbed, the session
  locking (scoot dismisses popup grabs on lock; one opened with `invoke` has
  no grab and is not dismissed by it, and is not drawn over the lock screen). Every one leaves the bar running and
  says nothing on stderr.
- **The keyboard.** The bar's layer surface asks for no keyboard and still
  does not. A `wl_keyboard` is taken from the seat **only while a popup that
  grabbed is open**, to hear Escape (the grab is what gives the popup the
  keyboard), and released with it. Keybindings of the compositor still win.
- **Limits.** One popup at a time (a [tooltip](#tooltips) is not one: it closes
  when a popup opens, and none shows while one is open). A drag ends when the pointer leaves the
  popup (the compositor's popup grab moves the pointer's focus off it, so
  nothing more of the drag arrives); past the slider's ends but still over the
  popup it clamps to them. No keyboard navigation but Escape. A compositor
  without `xdg_wm_base` refuses `popup` (said on stderr, or to `invoke`);
  the global is bound while some binding in the config names `popup`, and
  by an `invoke` for a bar with none; that one stays bound until the next
  `reload` (the binds are re-decided on a reload and on registry events, not
  when the popup closes). A bar that never opts in and never invokes binds
  nothing new, and a click is the mute it always was.
- **Cost.** Opening and closing are the only costs: one surface, a
  positioner, two `wl_shm` buffers made on demand and dropped on close, and a
  keyboard. An open popup that nothing changes makes no wakeups and no system
  calls; redraws are one per turn of the loop, however many pointer motions
  arrived, and the popup's own code (content refill, layout, pointer state,
  paint) allocates nothing once it is open. A slider drag still goes through
  the volume module's `set` action, which builds one request per distinct value
  (coalesced to one in flight, as a scroll does). Measured in [the
  resource ratchet](backlog/resolved/popups-done.md#evidence).
- **Writing one** is a module's [`Module::popup`](../../crates/scootbar/src/modules/mod.rs)
  (the content: text, a slider, buttons; a list is a column of buttons) and
  the actions its widgets name, which are ordinary module actions, so
  everything a popup does is something a binding or `invoke` could do.

## Tooltips

A module's `tooltip` (the line a module's view carries beside its text) is
shown in a small panel under the module after the pointer has rested on it for
`bar.tooltip-delay` milliseconds, and gone when the pointer leaves. It is the
[popup](#popups) machinery without a grab, so the `popup` Cargo feature (on by
default) is also the code of tooltips, and a build without it has no
tooltips and refuses the `tooltip-delay` key.

```toml
[bar]
tooltip-delay = 300     # ms, 0 to 10000; 0 turns tooltips off (default 500)
```

There is no flag for it: the config file only.

- **Which modules have one**, with nothing new read to make it: each module's
  tooltip is the one its view already carried (the window title's full title,
  uncut by the span; the network's interface, SSID and signal; the battery's
  `Charging 80%`; the volume and microphone's device and level; the
  brightness's device; the tray's item titles; the media module's player and line; a `push` or `exec` module's
  `tooltip` key, [the update payload](#the-update-payload)). A module with no
  tooltip, or whose tooltip is empty right now, shows none and arms nothing:
  the clock, workspaces and `button` modules have none. A new module's tooltip
  is `tooltip_mut` in its `view` and `Module::tooltips` saying so.
- **When.** The delay runs from the pointer entering the module (motion
  within it does not restart it), and a pointer that crosses a module and
  moves on before the delay shows nothing. Moving to another module hides the
  tooltip and starts that module's delay over.
- **It goes** on: the pointer leaving the module or the bar, **any press**
  (the press then acts as it always does: a tooltip is not a popup, and a
  click is never spent closing one), **any scroll**, a [popup](#popups)
  opening, the module's tooltip going empty, its module leaving the bar, its
  output being unplugged, the bar hidden or made again, a
  `reload`, and the session locking (the compositor takes it, and it is never
  drawn over the lock screen). After a press, a scroll or the compositor's
  taking it away it does **not** come back until the pointer has left that
  module. A scale change draws it again at the new scale.
- **One popup at a time, and a click popup wins.** While a popup is open no
  tooltip shows; a popup opening takes a tooltip down. A pointer that rested
  on a module while a popup was open, once it closes, gets that module's
  tooltip after the delay (unless the popup closed on the very module whose
  tooltip had been due or shown, which stays dismissed until the pointer
  leaves).
- **It never takes the keyboard, a grab or a click.** No `wl_keyboard`, no
  `xdg_popup.grab`, and an empty input region, so the pointer never enters it
  and nothing under it is hidden from a click.
- **Where it goes and how big.** Anchored to the module's span like a popup,
  centered under it (above, on a bottom bar), slid along the bar and flipped
  across it by the compositor where the output's edge would cut it. Text is
  wrapped at spaces at 30 ems (never wider than the bar), at most six lines,
  the last ending in an ellipsis (`…`) where it was cut; a word longer than
  the line is broken where it fills it. A newline in a tooltip
  breaks a line (the `push` and `exec` payloads turn control characters,
  newlines included, into spaces, so theirs wrap only). The frame and colors
  are the popup's.
- **While it is shown** a changed tooltip text (a clock-like tooltip) redraws it
  in place when the new text fits the size it opened at, damage limited to the
  tooltip's own surface; a text that needs more room, or its module moving
  along the bar (a neighbor's text grew), makes it again at once with no
  delay.
- **Cost.** With nothing hovered there is nothing: no timer file descriptor (the
  delay is the loop's `poll` timeout, set only while the pointer rests on a
  module with a tooltip that has not shown) and no wakeup. The bar takes a
  `wl_pointer` for tooltips only when tooltips are on and a placed module can
  have one (a bar of a clock alone takes none for them), and binds
  `xdg_wm_base` when the first tooltip shows (kept until the next `reload` or
  registry event, as an `invoke`'s is), so a bar nobody hovers binds nothing
  new. Showing one is a surface, a positioner, an empty region and two
  `wl_shm` buffers made then and dropped when it goes; a shown tooltip makes
  no wakeups and no system calls. A failure to show one (no `xdg_wm_base`, no
  buffer) is silent: a hover is not a request. Numbers: [the resource
  ratchet](backlog/resolved/tooltips-done.md#evidence).

## Button, push and exec modules

Three modules that extend the bar without writing Rust. Each is defined by a
table named for its kind and **a name of your choosing**, and the lists then
place it by that name like any built-in module:

```toml
left   = ["workspaces", "launcher"]
right  = ["weather", "status", "clock"]

[button.launcher]
icon = "\U000f0e65"                         # or icon-path, icon-image: as the clock's
text = "Apps"                               # shown after the icon
on-click = { exec = ["scootlaunch"] }

[exec.weather]
command = ["sh", "-c", "while :; do curl -s 'wttr.in?format=1'; sleep 600; done"]
format = "text"                             # or "json"
placeholder = "..."                         # until the first line

[push.status]
placeholder = ""
```

A name is 1 to 32 letters, digits, `-` or `_` starting with a letter or digit,
is not a built-in module's id (`clock`, `workspaces`) and is unique across the
three kinds; at most 32 modules are defined. Every table also takes `margin`
(as the clock's) and the five [interaction keys](#pointer-input), so any of
them can run a command or send `{ scoot = "quit" }` on a click or a scroll.
A table no list names is never started and costs nothing. These modules are
named in the config file's lists only: `--left`, `--center` and `--right`
take the built-in ids. A bad table is refused naming its dotted key
(`exec.weather.command`), and a bad `reload` changes nothing.

### `button`

An icon and/or text that never changes: a launcher, a power menu, a toggle.
`text` and the three icon keys (`icon`, `icon-path` with `icon-viewbox`,
`icon-image`, at most one) are the clock's, with the same refusals. A button
with neither shows nothing and takes no space. It has no fd and no wakeup.
`on-click = { exec = ["scootlaunch"] }` is the launcher and
`on-click = { scoot = "quit" }` log out, in the config
that is the whole of them.

### `push`

A place anything can write to with `scootbar msg set ID VALUE`, costing
nothing until it is: no fd of its own beyond the control socket, no timer, no
thread. `VALUE` is JSON, the [payload](#the-update-payload) below:

```sh
scootbar msg set status '{"text": "build ok", "class": "normal"}'
scootbar msg set status '"3 new mails"'      # a JSON string is the text alone
scootbar msg set status null                 # clears it (the module takes no space)
```

`set` only changes what the module shows; it cannot run anything. A value
that is refused (too long, not JSON, a class that does not exist, a
`version` this bar does not speak) leaves what was shown as it was and is
answered by name. Several `set`s that arrive in one turn of the loop are
drawn once; one that changes nothing is not drawn.

### `exec`

Runs a command and shows what it prints, one line per update. **It streams,
it does not poll**: the command runs once and the bar waits on its output, so
a script that can wait for an event prints when it has one and the bar does
nothing in between. There is **no `interval` key**, on purpose: a script
that has to poll writes `while :; do ...; sleep 60; done` (print first, so
the module shows its line at once, not after the first sleep), which puts the
cost (a process every minute, whatever it spawns) in a script you can see,
not in a bar option. A command that prints once and exits (`["date"]`) is a
poll too: the restart rule below runs it again, backing off to once a
minute, so write the loop yourself when you want a different rhythm. `command` is an array, the program and its arguments, **never
run through a shell** (write `["sh", "-c", "..."]` to use one), at most 32
arguments of at most 4096 bytes. `format` says how a line is read: `text`
(the line is the text, the default) or `json` (one object per line, below).

- **What is shown** is the last line of what the bar read at once (an
  update is a state, so earlier ones in the same read are already stale).
  A line that is not valid (JSON mode: not JSON, a bad key, a newer
  `version`) is ignored with one line on stderr, at most one a second, and
  what was shown stays. A blank line shows nothing.
- **Bounds**: a line longer than 4096 bytes is **dropped whole**, never
  truncated, with one warning a second at most: the bar holds one line at
  most however much the command prints. While output keeps coming the bar
  reads at most 4 KiB once every 16 ms and does not even poll the pipe in
  between, so a command printing as fast as it can fills the pipe and
  blocks (the kernel's back-pressure) and costs the bar about 60 small
  reads a second and no memory; one that prints once a minute costs
  nothing between lines.
- **Restarts**: when the command exits it is started again after 1 s, then 2,
  4, ... up to 60 s; a run that lasted 30 s or more starts the sequence
  over. A command that cannot start (no such program) takes the same path,
  and the one warning says why: `exit status: 127, command not found`, or
  `126, not executable`. Status `125` is the bar's for any other reason it
  could not run the command (and then a line before says why), but programs
  exit 125 themselves (`docker run`, GNU `timeout`, `env` and `nice`, `git
  bisect run`), so the warning says only that it is the command's own or the
  bar's and that any line above says why.
  Each restart is said on stderr, throttled to one warning a second per
  module, so a restart that follows another warning within the second is
  silent.
- **Children**: reaped the moment they exit (a pidfd wakes the bar, no
  timer, no zombie). Its stdin is `/dev/null`, its stderr is the bar's own
  (its complaints reach the journal), it leads its own process group, and it
  inherits none of the file descriptors the bar opens (one a launcher left
  open when it started the bar reaches it, as it does any child).
  **The whole process group is
  killed when the module goes**: on a reload that changed its table or
  removed it (one whose table is unchanged keeps its child, pipe, timer
  and shown output, wherever the lists place it now), and when the
  command exits, so a worker it backgrounded does not pile up across
  restarts. **The command ends with the bar, however the bar ends**: on a
  clean exit with the group kill, and on `SIGTERM`, `SIGINT`, `SIGHUP`,
  `SIGKILL`, a crash or the out-of-memory killer by the kernel's
  parent-death signal (`SIGKILL`), which the bar arms by starting the
  command through itself (`scootbar` re-executes as a tiny guard and
  becomes the command, so there is no extra process). What that does **not**
  reach is what the command started in turn: a shell loop dies, and the
  `sleep 60` it was in the middle of runs out its sleep (its next write to
  the closed pipe ends a writer such as `date` or `curl` with `SIGPIPE`), and
  a worker the command backgrounded and left is not touched. A command that
  is a set-user-id program is not covered either (the kernel clears the
  signal on such an `exec`). Commands a pointer binding starts are not
  guarded: a launched application outlives a bar restart on purpose.
  **The guard needs `/proc`** (it is `/proc/self/exe`): where `/proc` is not
  mounted no `exec` module can start, and the warning is ``cannot start
  `/proc/self/exe` to run `sh`: No such file or directory (the bar runs each
  command through itself, which needs /proc mounted)``.
- **Count**: at most 8 `exec` modules are placed (on any output); each
  holds a child, a `timerfd` and at most two polled fds.
- **Start-up**: a command is started right after the bar's first frame, not
  before it, so a slow `fork` never delays the bar.

### The update payload

What a `push` takes and an `exec` in `json` mode prints per line, scootbar's
own and **deliberately not Waybar's** (no `alt`, no `percentage`, no class
lists, nothing to translate), version 1:

```json
{"version": 1, "text": "72%", "class": "warn", "tooltip": "battery low"}
```

Every key is optional. `text` and `tooltip` are strings, `class` is one of
`normal`, `warn`, `urgent` or `muted` (colored by the theme's tokens: the
`urgent` and `dim` colors and so on), `version` is the shape this was
written for; a `version` above 1 is refused by name rather than half
understood, and keys it does not know are ignored, so later versions can add
some. Text and tooltip are cut at 256 bytes on a character boundary, and
every control character (a tab, a carriage return, an escape) becomes a
space, so nothing but printable text reaches the bar. A line or value
past 4096 bytes, or JSON nested more than 8 deep, is refused. The tooltip is
shown as a [tooltip](#tooltips) where the module has one.

## Fonts

A font file, not a font name: there is no fontconfig. Without `--font`,
the first of these that exists and loads is used:

```text
/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf         Debian, Ubuntu
/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf       Fedora
/usr/share/fonts/TTF/DejaVuSans.ttf                     Arch
/usr/share/fonts/truetype/DejaVuSans.ttf                openSUSE
/usr/share/fonts/dejavu/DejaVuSans.ttf                  Alpine
/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf   NixOS, with fonts.fontDir.enable
/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf     Debian, Ubuntu (Noto)
/usr/share/fonts/noto/NotoSans-Regular.ttf              Arch (Noto)
```

With none of them, and no `--font`, the daemon **refuses to start** (exit
status 1), saying so and how to give one; it does the same for a `--font`
it cannot use (missing, not a regular file, empty, over 64 MiB, or not a
TrueType or OpenType font), naming the file and why. A bar with no modules
placed draws no text and needs no font. On NixOS those directories are
usually empty: give `--font` a store path (`nix build nixpkgs#dejavu_fonts`
has `share/fonts/truetype/DejaVuSans.ttf`), or run the flake's
`scootbar-demo`, which gives it DejaVu Sans by default
([docs/nix.md](../nix.md#the-status-bar-scootbar)).

**Replacing the font file while the bar runs cannot crash it**, unless a
*root* process rewrites a mapped store file in place through a read-write view
(root ignores the write bit; `nix-daemon` never does, it adds, unlinks and
renames whole paths). The font is mapped
(costing only the pages drawn from, shared with every other program using
the font) only when it is owned by root, has no write bit for anyone, and
lies on a read-only mount: NixOS's `/nix/store`. Every other font (your
`~/.local/share/fonts`, `/usr/share/fonts`, and a writable file seen
through a read-only view such as systemd's `ProtectHome=read-only` or
flatpak's `/run/host/fonts`) is read into the bar's memory once, costing
its size (about 740 KB for DejaVu Sans): copying a new file over it with
`cp`, which truncates it in place, would kill a bar that had mapped it,
and cannot touch one that read it. The file is read at start and on every
reload: changing `bar.font` (or any option) and running
`scootbar msg reload` swaps it live.

### Fallback fonts and icons

`bar.fallback-fonts` (file only, no flag) names at most **two** more font
files, tried in order for a character the primary lacks. A character is drawn
from the first font in the chain that has a glyph for it; a fallback is asked
only about characters the fonts before it lack. A character in none of them
draws the **primary's** missing-glyph box: never a blank, never a panic. Each
fallback must load like the primary: one that cannot is a refusal naming it
(a start-up error, or a refused reload with the running bar untouched), and
more than two is a config error naming `bar.fallback-fonts`. Line height and
vertical centering come from the primary alone. There is no fontconfig, so
give the paths (Stylix supplies them on NixOS).

That is also how **icons** work: an icon is a glyph from a symbol font (Nerd
Font, Material Symbols, Font Awesome) given as text in the config, with the
symbol font as a fallback (or the primary). The clock takes one, drawn before
the time with a space between:

```toml
[bar]
fallback-fonts = ["/path/to/SymbolsNerdFont-Regular.ttf"]
[clock]
icon = "\U000f0e65"   # or the character itself; exactly one, else a config error
```

The icon is one code point, not one grapheme: an emoji plus a variation
selector or a ZWJ sequence is refused, but a lone format or combining
character passes and draws as a `.notdef` box or a blank, so give a real
symbol.

Glyphs are cached per font, size and scale, at most 512 glyphs and 4 MiB;
past either the cache is dropped and refilled from what is drawn next, so
arbitrary text (a window title) costs bounded memory. It is keyed by size, so
outputs at different scales each keep their glyphs until the bound, rather than
one dropping the other's on every frame.

**Out of scope, and will look wrong**: shaping (ligatures, complex scripts such
as Arabic or Devanagari, combining marks), right-to-left and bidirectional
layout (text runs left to right in logical order), and color emoji (outlines in
one color only; an emoji is drawn only if a font in the chain has an outline for
it). A title in such a script draws per codepoint.

**Real fonts, checked**: DejaVu Sans with `SymbolsNerdFont-Regular.ttf` and
`NotoSansCJK-VF.otf.ttc` (nixpkgs' `nerd-fonts.symbols-only` and
`noto-fonts-cjk-sans`) draw Latin, a symbol icon and Japanese, Korean and
Chinese together. A `.ttc` collection loads (its first face), and so does a
variable CFF2 font (its default instance). A CJK font is 30 MB or more, so
put it where it is mapped (a read-only `/nix/store`) or expect its size in the
bar's memory, as for any font not on a read-only mount; see
[icons.md](icons.md#fonts-what-the-real-ones-do).

### Icons

An icon is drawn before a module's text, `em` device pixels on a side (the size
of the text, at the output's real scale, so it is sharp at 1.5x and never a
smaller bitmap stretched), with a space after it when text follows. Three keys
give one, at most one of them per module; the clock, `button` and volume
modules take them, the network module takes them plus one glyph per state
([above](#network)), and the battery module will take the same three:

| Key | Takes | Drawn |
| --- | --- | --- |
| `icon` | exactly one character: a glyph from a symbol font in the [font chain](#fallback-fonts-and-icons) | as text, in the state's color |
| `icon-path` | SVG path data, a `d` string: `M m L l H h V v C c S s Q q T t A a Z z`, up to 16 KiB and 1024 commands | filled by the bar itself, anti-aliased, **tinted from the theme token** of the module's state (`normal` `fg`, `warn` `accent`, `urgent` `urgent`, `muted` `dim`), so it follows Stylix |
| `icon-viewbox` | `"min-x min-y width height"`, only with `icon-path`; default `"0 0 24 24"` | which part of the path's plane is the icon, fitted into the square and centered |
| `icon-image` | an absolute path to a PNG file (a build with `--features icon-image`) | scaled to the size, in its own colors (not tinted) |

```toml
[clock]
# Material's "home", straight from an SVG's <path d="...">:
icon-path = "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"
# Font Awesome-style paths are on a 512 plane:
# icon-path = "M..."
# icon-viewbox = "0 0 512 512"
```

![A path icon at scale 1.5](icons/path-1.5x.png)

A path that is not valid is a config error naming `clock.icon-path` and the
byte it stopped at (a refused reload leaves the running bar as it was): the
whole path grammar is accepted, and nothing else, with no guessing. A viewbox
without a path, or two of the three icon keys, is an error too. A PNG is
decoded when the config is read, so a missing, unreadable, non-regular (a FIFO,
a device), huge (past 1024 x 1024 or 8 MiB), truncated or corrupt file is a
config error naming `clock.icon-image` and the file, and the file may be moved
afterward: the bar holds the picture, and drops it at the next reload. Give a
PNG **at least as large as the icon**; it is scaled by a premultiplied
bilinear/area filter and centered keeping its aspect ratio.

![A PNG icon at scale 1.5](icons/image-1.5x.png)

Without the `icon-image` feature `icon-image` is an unknown key, and the config
error says so. **SVG files are not read** (a renderer is a large dependency and
an untrusted-markup parser): put the `d` string of a one-color icon in
`icon-path`, or convert a full-color SVG to PNG ahead of time. How it works, the
costs and the decisions are in [icons.md](icons.md).

## Colors

Modules name a state, never a color, and the state picks one of the
theme's color tokens: `normal` is drawn in `fg`, `warn` in `accent`,
`urgent` in `urgent` and `muted` in `dim`. The clock is always `normal`.
Every token has a `[colors]` key; `bg` and `fg` are `--background` and
`--foreground` flags too. Their defaults are Catppuccin Mocha's:

| Token | Default |
| --- | --- |
| `bg` | `#1e1e2e` |
| `fg` | `#cdd6f4` |
| `accent` | `#f9e2af` |
| `hover` | the `accent` value (unset, it follows a custom `accent`: the tint was the accent before it had a token of its own) |
| `dim` | `#6c7086` |
| `urgent` | `#f38ba8` |

## What it does on the compositor

- **One bar per selected output** ([Outputs](#outputs)), a layer surface (`top` by default) with the namespace
  `scootbar` (for compositor rules that match on it), anchored to its edge
  and both sides. An output plugged in later gets a bar; an output
  unplugged takes its bar with it, and the others are untouched. With no
  outputs at all the daemon waits, idle, for the first.
- **It reserves its space** (an exclusive zone), so windows are arranged
  beside it, never under it. The zone is set before the bar's first frame
  is drawn: on scoot, windows move out of the way once, when the bar
  connects, and do not jump again when it draws.
- **It takes no keyboard focus** (but for the Escape key of an open
  [popup](#popups), for as long as it is open). Pointer clicks on the workspaces
  module's numbers switch to them ([above](#workspaces)); anywhere else
  clicks do nothing.
- **It draws at each output's real device pixels**, fractional scales
  included (`wp_fractional_scale_v1` with `wp_viewporter`). A compositor
  without those two gets the bar drawn at its integer scale (the fraction
  rounded up) and scaled down: sharp, not device-exact. The daemon says so
  on stderr at start-up.
- **It draws only when something changes** (a new size or scale, or a
  module's content), and otherwise makes no system calls. Idle with the
  clock it wakes **twice a minute**: the clock's tick, and about a
  millisecond later the compositor's `wl_buffer.release` for the buffer
  that tick's frame replaced (every `wl_shm` client gets one per frame;
  measured on scoot and sway). With no module placed it wakes zero times.
  There are no frame callbacks. A change redraws, and tells the compositor
  about, only the module that changed: a tick repaints and damages the
  clock's own span, not the bar.
- If the compositor closes a bar (some do when an output goes away), it is
  made again once; closed a second time, that output is given up on until
  it is unplugged and plugged back in, and stderr says so.

## Layers and the zone

`--layer` (or `[bar] layer`) picks the layer-shell layer the bar sits in.
On scoot, `bottom` sits behind windows (they cover the bar where they
overlap it) and `top` and `overlay` in front, and **a fullscreen window hides
the `top` layer** (and the bar's zone is covered with it): the bar
disappears while something is fullscreen, which is the right default for a
bar. `overlay` stays over fullscreen windows; ask for it on purpose.
Fullscreen and maximize are different on purpose: a window that should fill
the screen *with* the bar visible wants scoot's
[maximize](../backlog/resolved/maximize-done.md) (`Super+m`,
`toggle-maximize`). There is
no `background` layer: that is the wallpaper's.

`--exclusive false` (or `exclusive = false`) sends an exclusive zone of -1:
the bar reserves nothing and windows go under it, and it also ignores any
other bar's zone. It is the choice for a floating overlay-style bar. With
the default `true` the zone is the bar's height, and the compositor adds the
margin on the anchored edge (see [Margins](#margins)). Whichever layer:
the bar never takes the keyboard, so clicking it never moves window focus.

Each of the three layers on each of the two edges, with and without the
zone, is checked on headless scoot (`crates/scootbar/tests/visibility.rs`),
along with the layer and the zone in the protocol requests themselves.
Changing either on a running bar is a `reload`, which makes the surfaces
again.

To toggle the bar from a key, bind it in scoot's own config (the bar takes
no keyboard, so it has no hotkey of its own):

```toml
[binds]
"super+b" = "spawn scootbar msg toggle"
```

## Outputs

By default every output gets a bar, all alike. The top-level `outputs` key
(or `--outputs`) picks which, and `[output."NAME"]` tables change one
output's bar. `NAME` is the compositor's own name for the output
(`wl_output.name`: the connector, `DP-1`, `eDP-1`, `HDMI-A-1`; `scootctl
outputs` on scoot, `swaymsg -t get_outputs` on sway).

```toml
outputs = ["eDP-1", "DP-1"]     # before any [table]: TOML puts later keys in it
left = ["workspaces"]
right = ["clock"]

[output."DP-1"]                 # the external monitor: taller, at the bottom, no workspaces
height = 36
edge = "bottom"
left = []
right = ["clock"]
```

- **`outputs`** is `"all"` (the default) or a list of names: only those
  outputs get a bar. An output plugged in later that is listed gets one; one
  that leaves loses its bar (and nothing else). An output the compositor
  never named (a `wl_output` older than version 4) matches only `all`.
  Names are compared byte for byte. The flag is `--outputs all` or
  `--outputs DP-1,eDP-1`; given, it replaces the file's list.
- **`[output."NAME"]`** takes `edge`, `layer`, `exclusive`, `height`, `margin`
  (the same values as `[bar]`) and `left`/`center`/`right`. A key not given
  keeps the shared value. Giving any of the three lists sets that output's
  whole layout, as at the top level: a list not given is empty, and
  `left = []` alone is a bar with nothing on it. Colors, the font, `padding`,
  `spacing`, `radius`, `opacity` and each module's own options are shared by
  every bar. An output table wins over a flag for its output (`--height 40`
  is the height of every output that does not set its own).
- **Refused, naming the key:** `outputs = []` (hide the bar with `msg hide`
  instead), a name listed twice, an empty or over-long (128 bytes) name or one
  with a control character, more than 32 names or tables, an
  `[output."X"]` table for an output `outputs` leaves out (it could never
  apply; with `outputs = "all"` a table for an output that is not plugged in
  is fine), an unknown key, a bad value, and a module placed twice **within
  one output** (the same module on two outputs is the point). `--outputs`
  is checked against the file's tables the same way, so the pair is a
  usage error, not a silent dead table.
- **One set of modules, started once.** The daemon starts each module in
  the shared layout or any output's override a single time, even if a
  `[output]` table then leaves it off every bar, and every output's bar
  reads it, so a second
  monitor adds a surface and its two buffers, not a second clock timer or a
  second set of file descriptors (checked in `tests/outputs.rs`: the daemon
  holds 8 fds with one output and 8 with two). A module's change redraws
  only the outputs that show it. The workspaces module is shared too, and
  each bar shows its own output's workspaces (see [Workspaces](#workspaces)
  for the switching limit). There is no "primary" output: to put a module
  on one output only, name that output in its table.
- **Scale.** Each bar is drawn at its own output's real device pixels
  (fractional scales included), so text and pill scale with their output. A
  per-output `font-size` is not built; the em is the same logical size
  everywhere.
- **`radius`** is shared and at most half the shared height: on an output
  whose own `height` is smaller the corners are cut back to what that bar
  holds (`radius` is 0 to half the height at the file level).
- **A reload** re-places every output: one the list now leaves out loses
  its bar (and its zone), one it now includes gets one, and a bar whose
  geometry changed is made again. Modules are restarted as at any reload,
  except an `exec` whose table is unchanged, which keeps its child (above).
- **`hide`/`show`** keep to the list: a shown bar is made only on a selected
  output. The two reasons a bar can be absent (hidden, not selected) are
  independent, so a `show` after a reload that changed the list makes the
  new list's bars.

## Hiding the bar

`scootbar msg hide` **destroys the bar's layer surface and its buffers** on
every output: a hidden bar holds no `wl_shm` buffer (checked in
`tests/visibility.rs` by counting the daemon's memory mappings, which go to
zero and come back), and its exclusive zone is released, so windows
reclaim the space. `show` makes them again, committed with no buffer before
the first draw as at start-up, so windows move once (back out of the bar's
way) and do not jump again when it draws; the round trip leaves a window
where it started (also checked, sampling the window's rectangle every few
milliseconds across the show). `toggle` flips whichever it is. The reply says
what is now the case: `{"type":"bar","visible":false}`.

- All three are idempotent, and applied **once per loop turn** with the net
  result: a burst of requests (forty `toggle`s at once, say) is one change
  of the final state, never a hide-show-hide flicker, since the surfaces are
  made or destroyed only after every request that arrived together has been
  served.
- Hidden is runtime state. A `reload` keeps it (a geometry change while
  hidden just waits for the `show`), and a restarted daemon starts shown.
- An output plugged in while hidden gets no bar until `show` (and only if the
  [list](#outputs) selects it); one that was
  unplugged and plugged in again is the same. `show` with no output is fine:
  each output's bar is made when it arrives.
- A hide during a redraw needs no care: a draw is one synchronous step of
  the loop, and the hide is another. Events for a destroyed surface (a
  `configure`, a buffer release) are dropped, as after any removal.
- The modules keep running while hidden (the clock's timer still fires
  every minute, or every second with a seconds format; a workspace change
  still wakes the daemon), and nothing is
  drawn: a hidden bar costs the process, not the surfaces. A bar with no
  module placed wakes zero times, hidden or not.
- A `top` bar under a fullscreen window needs nothing from scootbar: the
  compositor stops drawing the layer and the bar has nothing to do.
- **`hide` after the compositor closed a bar** (an output going away) forgets
  that: `show` gives it a fresh one, with one retry as at start-up.

### Not built: auto-hide

An auto-hide bar that appears on pointer contact needs a thin
always-present sensing surface at the edge, which is exactly what `hide`
removes (a surface, a buffer, an input region and pointer events that wake
the daemon). It was **decided against for now, not measured**: the cost
that matters (a sensing strip's pages and its pointer wakeups) would only
be worth paying if the strip could beat a key bind that toggles the bar
(above), which costs nothing when unused. Pointer input on the bar exists
now ([Pointer input](#pointer-input)), so the strip's cost could be
measured; it has not been, and the decision stands until it is.

## Margins

The margin is sent to the compositor as the layer surface's own margin, so
the bar surface is exactly the bar: no transparent border, and nothing on
the margin catches clicks. The compositor keeps windows clear of the bar
**and** the margin on its edge: with `--height 20 --margin 8,4`, a top bar
sits 8 pixels below the top edge, 4 in from each side, and windows start
28 pixels down. That is what the layer-shell protocol specifies ("the
exclusive zone includes the margin"), and what scoot and sway both do.

The margin on the edge opposite the bar's (the bottom margin of a top bar)
does nothing; the protocol says so.

To line a floating bar up with scoot's tiling, match `--margin` to scoot's
`[layout] gap` ([configuration.md](../configuration.md)): windows then sit
one gap below the bar, as they sit one gap from each other. The default is
still a flush, square, opaque bar (flush to the edge with no margin);
making it float is a choice, and these are the two settings to change
together, with the values that match scoot's defaults (`gap` 12):

```toml
# ~/.config/scoot/bar.toml: a floating bar that lines up with the windows
[bar]
margin = 12            # = scoot's [layout] gap
radius = 8             # = scoot's [appearance] corner_radius, if you round windows
opacity = 1.0
```

```toml
# ~/.config/scoot/config.toml: what they must match
[layout]
gap = 12               # the bar's margin

[appearance]
corner_radius = 8      # the bar's radius (0, square, is scoot's default)
```

Change `gap` and `margin` together and the bar keeps lining up; change
only one and the bar's edge drifts from the windows'. The bar does not read
scoot's config (it works on other compositors), so nothing keeps them in
step but you.

## Spacing

Three lengths, all logical pixels and all bounded 0 to 1024 (a value past
that, negative or not a whole number is a loud refusal naming its key):

| Key | Flag | What it spaces |
| --- | --- | --- |
| `[bar] padding` | `--padding` | inside each module: room either side of its content |
| `[bar] spacing` | `--spacing` | between neighbouring modules in a section |
| `[clock] margin`, `[workspaces] margin` | none | outside one module: that much more room on each side of it, on top of `spacing`. Its own span never covers it, so a repaint of the module leaves it alone. A module with nothing to show takes none. |

`[bar] separator` draws a line in the gap between neighbouring modules of a
section: that many logical pixels wide, in the theme's `dim` color, from a
quarter to three quarters of the bar's height, centered in the gap. It sits
in the gap, so it needs one: **`separator` may not exceed `spacing`** (a
larger value is refused, naming both), and the drawn line is cut back to the
actual gap. The check is against the file's `spacing`: a `--spacing` flag
given later replaces it and can leave a separator wider than the gap, which
is then cut to it (drawn narrower, never over a module). There is none between two sections, beside a module with
nothing to show, or at the bar's ends. `0`, the default, draws none.

```toml
left = ["workspaces", "clock"]

[bar]
spacing = 12
separator = 1          # a hairline centered in each 12-pixel gap

[workspaces]
margin = 4             # 4 more either side: the gaps beside it are 20
```

Nothing here costs a frame: the lines are painted with the whole bar, never
for a module's own repaint, and the layout is worked out only when a width
or the size changes.

## Shape and opacity

`[bar] radius` rounds the bar's four corners, in logical pixels: 0 (the
default) is square, and the most is half the height (a pill), which the file
enforces (`bar.radius` names the limit when it is refused). `[bar] opacity`
is the background's alpha, 1 (the default) opaque down to 0 transparent;
text stays fully opaque over it. There is no blur, gradient or shadow.

```toml
[bar]
height = 28
margin = "8,8"     # match scoot's [layout] gap: see Margins
radius = 10
opacity = 0.9
```

- **The corners are analytic**: each edge pixel's alpha is how far its
  center sits inside the circle, no supersampling. The coverage of one
  corner is computed once per radius and scale, and the other three mirror
  it, so a repaint costs a table lookup for the few pixels in a corner.
- **A rounded or translucent bar is an `ARGB8888` buffer** (premultiplied);
  a square, opaque bar stays `XRGB8888`, exactly as before either option.
  Only the surface is the bar, never a bigger transparent one: the margin
  is still the protocol's, and the corner pixels are the only transparent
  ones.
- **Opaque region**: an opaque bar tells the compositor which pixels are
  opaque, so it can skip blending under them (all of a square bar; all but
  the four corner squares of a rounded one). A translucent bar declares
  none: the compositor blends all of it, which is the cost of the option.
- **The radius is cut back to fit**: `--height` below twice the file's
  `radius` (the flag replaces the file's height), or a compositor giving
  the bar less height than asked, draws the corners as large as the bar
  holds.
- **Text is kept out of the corners, not clipped to them**: the layout
  clears the bar's ends by the radius (see [Layout](#layout)), so no ink
  is near a corner whatever `padding` is. This costs the radius less half a
  padding of room at each end, and needs no per-pixel clipping in the
  paint; a bar whose radius is 0 loses nothing.
- **Corners are not clickable**: the surface's input region is the rounded
  shape (one rectangle per corner row, at most `2 x radius + 1`, set when
  the size or radius changes), so a click in a cut corner reaches what is
  behind the bar (the wallpaper, a window under a bar that does not reserve
  space) instead of an invisible bar. scoot honors `wl_surface.set_input_region`
  on layer surfaces, checked end to end on a headless scoot: a click in the
  window's corner under a rounded bar focuses that window, and the same click
  under a square bar does not. The region is in logical pixels, so it is the
  same at every scale (at a fractional scale it can differ from the drawn
  edge by a device pixel). A compositor that ignores input regions leaves
  the corners clickable, which is the protocol's fallback.
- Measured costs (release build, 1600x28 bar, radius 14): filling the whole
  bar takes 4.9 us square and 6.1 us rounded and translucent; 3200x56 (scale
  2), 16.2 us and 29.2 us. A repaint of one module's span, the common case,
  costs the same as before except in the rows of a corner.

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
