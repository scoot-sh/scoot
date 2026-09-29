# scootbar command reference

Every `scootbar` command and flag, and how it behaves at the edges. What
scootbar is and why is in [README.md](README.md). There is no config file
yet ([config-cli-and-reload](backlog/config-cli-and-reload.md) brings one);
until then every option is a flag, and the flags stay as overrides once
the file exists.

**Early days:** the bar shows a clock. Workspaces are
[next](backlog/workspaces-module.md).

## Commands

```sh
scootbar daemon                              # a 28-pixel bar along the top of every output, the clock in the middle
scootbar daemon --clock-format '%H:%M'       # a 24-hour clock (the default is 12-hour: 3:07 pm)
scootbar daemon --font ~/.local/share/fonts/Inter.ttf --font-size 13
scootbar daemon --right clock                # the clock at the right end
scootbar daemon --edge bottom --height 32    # along the bottom, 32 logical pixels tall
scootbar daemon --margin 8                   # floating 8 pixels in from its edge and both sides
scootbar daemon --margin 8,12                # 8 above and below, 12 either side
scootbar daemon --background '#101014' --foreground '#e0e0e0'
scootbar --help                              # and `scootbar daemon --help`
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
| `--edge` | `top` or `bottom` | `top` | The output edge the bar runs along. Vertical bars are not in the first version. |
| `--height` | 1 to 1024 | 28 | The bar's height in logical pixels. On a scaled output it is drawn at the output's real pixels: 28 at scale 1.5 is 42 device pixels. |
| `--margin` | one to four of 0 to 1024, comma-separated | `0` | Space between the bar and the output's edges, in logical pixels, in CSS order: `ALL`, `VERTICAL,HORIZONTAL`, `TOP,HORIZONTAL,BOTTOM` or `TOP,RIGHT,BOTTOM,LEFT`. See [Margins](#margins). |
| `--background` | `'#rrggbb'` | `'#1e1e2e'` | The bar's color, six hex digits in either case. Quote it: the shell reads `#` as a comment. No alpha yet. |
| `--foreground` | `'#rrggbb'` | `'#cdd6f4'` | The text's color (the theme's `fg` token: see [Colors](#colors)). |
| `--font` | a path | the first [well-known font](#fonts) found | The font file, TrueType or OpenType (`.ttf`, `.otf`; a collection's first face). Any bytes: a path need not be UTF-8. |
| `--font-size` | 1 to 256 | 14 | The text's size, the em, in logical pixels; drawn at the output's real pixels like the bar. |
| `--left`, `--center`, `--right` | module ids, comma-separated, or empty | the clock in the center | The modules along each part of the bar, in order. Giving any of the three sets the whole layout: a part not given is empty, so `--right clock` moves the clock rather than adding a second one. `--center ''` places nothing: a plain bar that needs no font. See [Layout](#layout). |
| `--padding` | 0 to 1024 | 8 | Logical pixels either side of each module's content. |
| `--spacing` | 0 to 1024 | 0 | Logical pixels between neighbouring modules. |
| `--clock-format` | a format, at most 256 bytes | `'%-I:%M %P'` | What the clock shows; see [The clock](#the-clock). |

A malformed or out-of-range value, an unknown flag or a flag given twice is
a usage error (exit status 2), and nothing starts; so is an unknown module
id, a module placed twice, or a clock format with an unknown specifier or a
control character.

## Modules

| Id | Shows | Wakes |
| --- | --- | --- |
| `clock` | The local time ([below](#the-clock)) | once a minute, or once a second with seconds shown |

A build can leave a module out (`cargo build --no-default-features`, then
`--features clock`); naming one that is not built is a usage error that
lists those that are.

## Layout

- **Left** modules are packed from the left edge, **right** ones from the
  right edge, in the order listed; **center** ones are packed together and
  centered on the bar.
- Each module is as wide as its content plus `--padding` on both sides;
  `--spacing` separates neighbours. A module with nothing to show takes no
  space at all, padding and spacing included.
- **When they do not fit**, the left part keeps its place, the right part
  gives way to it, and the center part is pushed off center to fit between
  them, then cut. Nothing overlaps and nothing is drawn past the bar's end;
  text is clipped to its module's space.
- Text is vertically centered on the bar. It is not shaped: one glyph per
  character, no ligatures, kerning or right-to-left runs. A character the
  font lacks draws the font's missing-glyph box. Control characters are
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
- **It wakes once a minute** (on the minute), or **once a second** when the
  format shows seconds (`%S`, `%T`), and at no other time: no polling.
- **The time zone** is read as glibc reads it: `$TZ` if set (a zone name
  such as `Europe/London`, looked up under `$TZDIR` or
  `/usr/share/zoneinfo`; an absolute path, with or without a leading `:`;
  or a POSIX rule such as `EST5EDT,M3.2.0,M11.1.0`), else `/etc/localtime`.
  An empty `$TZ` is UTC. A zone that cannot be read is shown as UTC, with
  one line on stderr saying why; the bar still starts.
- **A new zone shows at the next tick**: `/etc/localtime` (or the file `$TZ`
  names) is checked once each wake, so `timedatectl set-timezone` takes
  effect within the minute. A zone file that disappears keeps the zone last
  read until one is back.
- **Clock steps, suspend and summer time** show at once or on the boundary:
  the timer is on the wall clock and cancelled by any step (NTP, `date
  -s`, a resume from suspend), which wakes the bar at once to redraw; a
  summer-time change shows at the first tick after it.

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
has `share/fonts/truetype/DejaVuSans.ttf`).

**Replacing the font file while the bar runs is safe.** A font on a
read-only mount (NixOS's `/nix/store`) is mapped, which costs only the
pages drawn from, shared with every other program using the font, and
nothing can truncate it there. A font anywhere else (your
`~/.local/share/fonts`, `/usr/share/fonts`) is read into the bar's memory
once, costing its size (about 740 KB for DejaVu Sans): copying a new file
over it with `cp`, which truncates it in place, would kill a bar that had
mapped it, and cannot touch one that read it. The file is read once, at
start: a new font needs a restart.

## Colors

Modules name a state, never a color, and the state picks one of the
theme's color tokens: `normal` is drawn in `fg`, `warn` in `accent`,
`urgent` in `urgent` and `muted` in `dim`. The clock is always `normal`.
Only `bg` (`--background`) and `fg` (`--foreground`) are flags so far; the
config file gives the others keys. Their defaults are Catppuccin Mocha's:

| Token | Default |
| --- | --- |
| `bg` | `#1e1e2e` |
| `fg` | `#cdd6f4` |
| `accent` | `#f9e2af` |
| `dim` | `#6c7086` |
| `urgent` | `#f38ba8` |

## What it does on the compositor

- **One bar per output**, a `top`-layer surface with the namespace
  `scootbar` (for compositor rules that match on it), anchored to its edge
  and both sides. An output plugged in later gets a bar; an output
  unplugged takes its bar with it, and the others are untouched. With no
  outputs at all the daemon waits, idle, for the first.
- **It reserves its space** (an exclusive zone), so windows are arranged
  beside it, never under it. The zone is set before the bar's first frame
  is drawn: on scoot, windows move out of the way once, when the bar
  connects, and do not jump again when it draws.
- **It takes no keyboard focus.** It takes no pointer input yet either
  (clicks on it do nothing).
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
one gap below the bar, as they sit one gap from each other.

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
  absurd surface) is refused with a message, not drawn, and not retried
  until the size or scale changes.
- **The compositor going away** (it exits, crashes, or sends a protocol
  error) ends the daemon with exit status 1 and one line on stderr saying
  why. There is no reconnect; your session's autostart starts it again with
  the compositor.
- **SIGTERM and SIGINT** end it at once. That is harmless: it keeps no
  state, and the compositor removes its surfaces with the connection.
- **One per display is not enforced** yet: two daemons on one display give
  two bars, each reserving its own space. A control socket, and with it a
  refusal of a second daemon, comes with
  [config-cli-and-reload](backlog/config-cli-and-reload.md).
- **Not Linux:** the binary builds everywhere but runs on Linux only; on
  any other system it says so and exits with status 1.

## Exit status

| Status | When |
| --- | --- |
| 0 | `--help` or `--version` |
| 1 | no usable font (with a module placed), cannot connect, the compositor lacks `wl_compositor` v4, `wl_shm` or `zwlr_layer_shell_v1`, the compositor went away, or `poll(2)` failed |
| 2 | a usage error |
