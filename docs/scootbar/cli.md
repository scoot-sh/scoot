# scootbar command reference

Every `scootbar` command and flag, and how it behaves at the edges. What
scootbar is and why is in [README.md](README.md). There is no config file
yet ([config-cli-and-reload](backlog/config-cli-and-reload.md) brings one);
until then every option is a flag, and the flags stay as overrides once
the file exists.

**Early days:** the bar is one solid color. The clock and the module API
are [next](backlog/module-api-and-clock.md).

## Commands

```sh
scootbar daemon                              # a 28-pixel bar along the top of every output
scootbar daemon --edge bottom --height 32    # along the bottom, 32 logical pixels tall
scootbar daemon --margin 8                   # floating 8 pixels in from its edge and both sides
scootbar daemon --margin 8,12                # 8 above and below, 12 either side
scootbar daemon --background '#101014'       # its color
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

A malformed or out-of-range value, an unknown flag or a flag given twice is
a usage error (exit status 2), and nothing starts.

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
- **It draws only when something changes** (a new size or scale), and
  otherwise makes no system calls: idle, it wakes zero times a minute.
  There are no timers and no frame callbacks.
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
| 1 | cannot connect, the compositor lacks `wl_compositor` v4, `wl_shm` or `zwlr_layer_shell_v1`, the compositor went away, or `poll(2)` failed |
| 2 | a usage error |
