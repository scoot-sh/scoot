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
scootbar msg query                  # every placed module's state as JSON
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
icon, which is not text). This is the agent hook: the bar read as data instead of OCR.
`reload` re-reads the file and live-applies it — geometry, style, layout,
modules, the font — after fully validating it first; a bad file is
refused and the running bar stands. `hide`, `show` and `toggle` are
[below](#hiding-the-bar). `query`, `version`, `reload`, `hide`, `show` and
`toggle` print the reply; `kill` and `set` print nothing on success. `set`
writes to a [`push` module](#button-push-and-exec-modules): an id that is not
placed, a module that takes no value (every one but `push`) and a value it
refuses are each a loud error naming why, never a silent ok. Without a
daemon, every command fails saying so (exit status 1).

## Modules

| Id | Shows | Wakes |
| --- | --- | --- |
| `clock` | The local time ([below](#the-clock)) | its timer: once a minute, or once a second with seconds shown (each redraw adds the compositor's release, below) |
| `workspaces` | Each output's workspace numbers, the active one marked ([below](#workspaces)) | on the compositor's workspace changes: one redraw per batch, however many events it held |

A build can leave a module out (`cargo build --no-default-features`, then
`--features clock`); naming one that is not built is a usage error that
lists those that are. Every module takes the five
[interaction keys](#pointer-input). Besides these two, the config can define
modules of its own by name, a [`button`, a `push` and an `exec`
module](#button-push-and-exec-modules), each a Cargo feature (`button`,
`push`, `exec`) on by default. The `icon-image` feature is not a module: it adds the PNG
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
  disc: raise `padding` or the `pill-inset` until it is round.
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
  workspace change.
- **Not built (their own follow-ups)**: dot-style indicators (a row of
  small dots in place of the numbers), and colors for the inactive
  workspaces or per-state pill colors.

## Pointer input

Clicks, scrolls and hover, on every module. The bar never takes the keyboard
(its layer surface asks for no keyboard interactivity, so it cannot disturb
focus), and **touch is ignored**: the bar binds the seat's pointer only, so
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
  nothing else has one. A binding replaces the default.
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

**Hover.** A module with a binding is drawn in the `accent` color while
the pointer is over it, and only that module's span is redrawn, on that
output's bar alone; moving between modules redraws the one left and the one
entered. A module with no binding is not tinted (the workspaces module
draws its own pill, and is not tinted either).

**Cost.** The bar asks the seat for a pointer only while a placed module
has a binding or a default of its own (today: the workspaces module).
A clock-only bar with no bindings never takes the pointer, and costs what
it did before there was any input. A reload that adds or removes bindings
takes or drops it. A motion event stores two numbers; no pointer event,
hover repaint or module action allocates (tests count the allocations).
Launching an `exec` command does, as any process spawn must.

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
  over. A command that cannot start (no such program) takes the same path.
  Each restart is said on stderr, throttled to one warning a second per
  module, so a restart that follows another warning within the second is
  silent.
- **Children**: reaped the moment they exit (a pidfd wakes the bar, no
  timer, no zombie). Its stdin is `/dev/null`, its stderr is the bar's own
  (its complaints reach the journal), it leads its own process group, and it
  inherits none of the bar's file descriptors. **The whole process group is
  killed when the module goes**: on a reload (the command is started afresh
  with the new config, whether or not its table changed), and when the
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
carried for [tooltips](backlog/tooltips.md), which are not drawn yet.

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
give one, at most one of them per module; the clock takes them now, and the
button, volume, network and battery modules will take the same three:

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
- **It takes no keyboard focus.** Pointer clicks on the workspaces
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
[maximize](../backlog/core/maximize.md), which does not exist yet. There is
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
  geometry changed is made again. Modules are restarted as at any reload.
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
| 1 | no usable font (with a module placed), cannot connect, the compositor lacks `wl_compositor` v4, `wl_shm` or `zwlr_layer_shell_v1`, the compositor went away, `poll(2)` failed, the config file is malformed (`daemon --check` too), or a `msg` command failed (no daemon, or the daemon refused) |
| 2 | a usage error |
