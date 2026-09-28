# scootbg command reference

Every `scootbg` command, what it waits for, and how it fails. What scootbg
is and why it exists is in [README.md](README.md); in scoot, a
`[wallpaper]` section runs it for you
([configuration.md](../configuration.md#wallpaper)).

## Commands

```sh
scootbg daemon                        # connect to $WAYLAND_DISPLAY and serve the control socket
scootbg daemon --profile sway         # restore and save the `sway` profile's state instead
scootbg daemon --no-restore           # start with nothing shown (the state is kept)
scootbg set '#1e1e2e'                 # every output, including ones plugged in later
scootbg set '#101014' --output DP-2   # one output, by connector name (as `query` lists them)
scootbg set ~/Pictures/hills.jpg      # an image, covering every output (--mode fill)
scootbg set ./#draft.png              # a file whose name starts with '#'
scootbg set city.png --output DP-2 --mode fit --fill '#101014'
scootbg set tile.png --mode tile --filter nearest
scootbg clear                         # back to the compositor's own background
scootbg clear --output DP-2           # ... on one output
scootbg query                         # each output, its surface and what it shows, one JSON line
scootbg version                       # the running daemon's version and protocol
scootbg kill                          # stop it; returns once a new daemon can start
scootbg apply-config --profile scoot '{"color":"#1e1e2e"}'
                                      # what scoot runs with its [wallpaper] section
scootbg --help                        # and `scootbg COMMAND --help`
```

## Colors and images

**Colors** are `#rrggbb`, six hex digits in either case; quote them, since
the shell reads `#` as a comment. No `#rgb` shorthand, no alpha (a
wallpaper is opaque). **Anything not starting with `#` is an image path**,
so a file whose name starts with `#` is given as `./#name.png`. The path is
made absolute before it is sent (the daemon's working directory is not
yours) and must be valid UTF-8. PNG, JPEG and WebP are read, told apart by
content, not by name (an animated PNG or WebP shows its first frame);
transparency is shown over the fill color, and EXIF orientation is
applied (a JPEG's, a WebP's, or a PNG's `eXIf` chunk). `--mode` fits it to each output:

| `--mode` | |
|---|---|
| `fill` (default) | cover the output, cropping what overflows, centred |
| `fit` | all of it, as large as fits, centred, the rest in `--fill` |
| `stretch` | the output's size, whatever the aspect |
| `center` | unscaled, centred: cropped if larger, the rest in `--fill` |
| `tile` | unscaled, repeated from the top-left corner |

`--fill '#rrggbb'` is the color around a fitted or centred image (default
`#000000`), `--filter lanczos3|catmull-rom|bilinear|nearest` the scaling
filter (default `lanczos3`; `nearest` keeps pixel art hard). `--mode`,
`--fill` and `--filter` with a color are a usage error. The image is
decoded and scaled on a worker thread and the decoded pixels are dropped
once drawn. Outputs of one size showing one image share one buffer's
memory (32.4 MB at 4K, however many outputs show it). An output plugged
in later shares the pixels of an output of its size already showing the
image, with no decode; otherwise it, like a new scale that needs a new
size, reads the file again. Images over 16384×16384 pixels are refused.

## Scale

**Images are drawn at each output's real device pixels, fractional scales
included.** On a compositor with `wp_fractional_scale_v1` and
`wp_viewporter` (scoot, sway, and most others) the buffer is the surface's
logical size times the scale the compositor asks for, rounded as that
protocol says: 1601×1001 for scoot's 1067×667 surface at 1.5 on a
1600×1000 output, which scoot draws one buffer pixel to one device pixel
(the last column and row fall off the edge). An image the size of the
output with `--mode center` comes out exact to the pixel. Without those
protocols it is drawn at the integer scale (the larger of `wl_surface`'s
preferred buffer scale and `wl_output`'s) and the compositor scales it
down: sharp, but larger than the output (2134×1334 there). The same
happens for a moment when the compositor has not yet told a surface its
new scale (sway does not while nothing is shown on it): the image is on
screen when `set` returns, drawn larger and scaled down, never stretched,
and redrawn exact once the compositor sends the scale. A compositor scale that is
not a multiple of 1/120 (1.33, say) cannot be drawn exactly by any client,
since the protocol cannot say it.

## One output or every output

`set` with no `--output` replaces every choice, per-output ones included;
with `--output NAME` it sets that output only, and the choice is kept by
name, so the monitor shows it again when it is unplugged and plugged back
in. A name no output has right now is an error (exit 1) and changes
nothing. `clear` does the same with nothing to show.

## When a command returns, and its exit status

**`set` and `clear` return once it is on screen:** every targeted output
shows the change and the compositor has processed it (a `wl_display.sync`
round trip after the commits), so a screenshot taken straight after shows
it. An output unplugged meanwhile is left out of that wait; one whose
surface is not configured yet is waited for, but only until a round trip
after scootbg made that surface: one the compositor has not configured
by then no longer holds up the reply (the daemon says so on stderr) and
is drawn when it is; one scootbg has given up on
(`gave-up` in `query`, said on stderr) is left out and shows nothing, and
`set` still exits 0. They print nothing on
success. **An image that cannot be shown** (no such file, not a regular
file, not a PNG/JPEG/WebP, too large, truncated or corrupt) is an error
saying why, and every output keeps what it showed. **The newest request
wins**: a `set` or `clear` sent while an earlier image is still decoding is
never undone when that image finishes; the earlier `set` changes nothing
and returns 0 once the newer one is on screen, as a replaced color's `set`
does. Exit status: 0 done; 1 no daemon running,
unknown output, image that cannot be shown, or drawing failed (the
daemon's stderr says why); 2 usage error, a malformed color or an unknown
`--mode`/`--filter` included.

## What it costs

On a compositor with `wp_single_pixel_buffer_manager_v1` and
`wp_viewporter` (scoot, sway, and most others) a color costs no shared
memory at all: one single-pixel buffer scaled to the output. Without
single-pixel buffers it is a 1×1 `wl_shm` buffer under the viewport, and
without a viewporter a full-size buffer. A static color asks for no frame
callbacks and wakes the daemon for nothing.

Once on screen a wallpaper costs no CPU (no wakeups in a minute, measured)
and one buffer per output at most: about 4.0 MB RSS with a color, 36.8 MB
with an image on a 4K output, and the same 36.8 MB with two 4K outputs
showing it. The measured budget is in
[README.md](README.md#the-resource-budget),
and so is [the comparison](README.md#against-the-other-daemons)
with swaybg, awww, wbg and wpaperd on one machine. It is not yet the
lightest on every row: awww holds 1.0–1.6 MiB less idle memory above the
output buffers. `scripts/scootbg-bench/bench.py` re-runs the comparison.

## The daemon and its socket

One daemon per display: its socket is
`$XDG_RUNTIME_DIR/scootbg-NAME.sock`, `NAME` being the last component of
`$WAYLAND_DISPLAY`. A second `scootbg daemon` refuses while one runs, and
a socket left by a dead one is replaced. It exits 0 on `kill` and 1 when
the compositor goes away, removing its socket either way. A signal
(SIGTERM, Ctrl-C) ends it on the spot and leaves the socket file; the other
commands then say no daemon is running (exit 1), and the next
`scootbg daemon` replaces the file. Every command exits 2 on a usage
error (an unknown command or argument, or a bad `--profile` name).

## `apply-config`

**`scootbg apply-config [--profile NAME] JSON`** is how a config, rather
than a person, sets the wallpaper: scoot runs it at start-up and on
every reload with its `[wallpaper]` section as JSON (`{}` once the section
is gone). The JSON is one object: `image` (an absolute path) or `color`
(`#rrggbb`), with `mode`/`fill`/`filter` for an image as `set` takes them;
`output` holding a table of the same keys per connector name (each table
stands alone: an output's image does not take the top level's `mode`);
and `command` (scoot's, ignored). Anything else (an unknown key, a key
given twice, `null`, a relative path, a bad color, mode or filter, over
256 outputs or 63 KiB) is refused with exit 2, so a typo is never a
setting silently not applied.

- **Whichever you changed last wins.** The section is applied only if it
  changed since the last `apply-config` for that profile (a SHA-256
  fingerprint of it, `command` left out, kept in the profile's state
  file). So a `scootbg set` made afterwards keeps showing across restarts
  and reloads until the section itself changes. The one order it cannot
  see: a section removed while scoot is not running and re-added unchanged
  before the next start counts as unchanged.
- **It starts the daemon when none runs**, detached (a session of its own,
  its stderr `apply-config`'s: send that to a file or a log, not a pipe
  read to its end), starting from the section, so the saved state never
  flashes first. Two started at once make one daemon, settled by its lock.
  With no daemon and an empty section it starts none, and records the
  clear in the profile's state instead.
- **A running daemon adopts the profile** it is sent, whatever
  `--profile` it started with, so an `[autostart]` `scootbg daemon` ends up
  on scoot's profile too. `scootbg query` reports the profile in use
  (`"profile"`, after `"saving"`).
- **Another build is reported:** a daemon of another version is a
  warning (the section is still sent), one of another protocol or too old
  for `apply-config` an error.
- It returns once every output shows it, as `set` does, and prints nothing
  on success. Exit status: 0 applied or unchanged, and shown; 1 no daemon
  could be started or reached (5 s), no reply (30 s), the daemon closed
  the connection before answering (`scootbg kill`, a crash), another
  protocol, a daemon too old for `apply-config`, an image in the section
  that is not a file (the rest is applied; reported by every
  `apply-config` until the file is back, and then shown), drawing failed,
  or the state file could not be written; 2 a usage error.

The schema, the fingerprint's exact encoding, the precedence table and
the protocol are in
[README.md](README.md#apply-config-scoots-wallpaper-section).

## Restore

**The wallpaper is restored at the next start.** Every `set` and `clear`
is saved, per output (the choice for every output, and each one made with
`--output NAME`), in a state file, `$XDG_STATE_HOME/scootbg/PROFILE`
(`~/.local/state/scootbg/PROFILE` when `XDG_STATE_HOME` is unset or not
absolute), and `scootbg daemon` shows it again when it starts:

- **A profile, not a display, names the state.** `--profile NAME` picks
  it (default `default`): 1 to 64 of `A-Z a-z 0-9 . _ -`, not starting
  with `.` and without `..`. scoot binds the first free `wayland-N`, so
  the display name is no session identity; give each session its own
  profile (a sway autostart can pass `--profile sway`) and they never
  restore each other's wallpaper. Two sessions sharing a profile share
  its state, the last change winning.
- **When it is saved:** a color or a `clear` as soon as the daemon has it,
  an image once it has decoded (one that cannot be shown changes nothing,
  so saves nothing). The file is written off the daemon's loop,
  atomically (a fresh temporary file that never follows a symbolic link,
  `fsync`, `rename`), private (0600, in a 0700 directory; a state
  directory that already exists and is someone else's, or writable by
  its group or others, is a warning at start-up); `scootbg kill` waits
  for a write under way, and a signal leaves the old file or the new one, never half
  of each.
- **What survives:** a choice for an output that is not plugged in now
  stays saved and comes back with it; `set` without `--output` replaces
  every per-output choice, as it does on screen. A restore saves nothing.
  At most 256 per-output choices are kept (and 256 KiB in all): past
  that, the least recently set go first, with a warning.
- **A saved image that is gone** (moved, deleted, on a disk not mounted
  yet) is skipped with a warning on the daemon's stderr, and that output
  shows the compositor's own background. The daemon starts all the same,
  and the entry stays saved until a `set` or `clear` replaces it, so it
  is restored once the file is back.
- **`--no-restore`** starts with nothing shown. The state is still read,
  and a `set` or `clear` then updates it as usual, keeping the rest.
- **A state file it cannot use never stops the daemon.** A malformed line
  (or one whose path is not UTF-8) is skipped with a warning and the rest
  is used; a file that is not one (no `scootbg-state 1` first line, over
  256 KiB) restores nothing and is replaced at the next save; a newer
  scootbg's format (a later version) restores nothing and is never
  written over; an unreadable one restores nothing, and nothing is saved
  over it. In those two cases **saving is off until the daemon
  restarts**: `set` still works on screen, stderr says at start-up which
  file to remove or fix to save again, and `scootbg query` reports
  `"saving":false`. The format is a documented, versioned line format
  ([README.md](README.md#restore)).
- **A restored image is decoded once**, when its outputs are configured:
  about 450 ms to a 6000×4000 JPEG on screen on a 4K output, the same as a
  `set` on a running daemon. So is an image `set` sent the moment the
  daemon starts: it waits for the outputs' `configure` (bounded by a
  round trip already sent) rather than decode once to check the file and
  again to draw it.

## `query`

Outputs are tracked as they come and go: each gets one surface on the
`background` layer (namespace `wallpaper`), covering the whole output,
reserving no space and taking no input. With no outputs at all the daemon
simply waits. `scootbg query` answers, on one line (wrapped here):

```text
{"type":"outputs","outputs":[{"name":"DP-1","description":"...",
 "mode":{"width":3840,"height":2160},"scale":2,"transform":"normal",
 "logical":{"width":2560,"height":1440},
 "surface":{"state":"configured","size":{"width":2560,"height":1440},
            "scale":1.5,"pixels":{"width":3840,"height":2160}},
 "draw_failed":false,"shows":{"color":"#1e1e2e"}}],"saving":true,
 "profile":"default"}
```

Every key is always present, `null` when not known yet. `mode` is in
device pixels. `scale` is `wl_output`'s integer scale (a fractional one
rounded up), and `transform` is `wl_output`'s, counter-clockwise.
`logical` is the output's size in logical pixels: exact once the surface is
configured, before that worked out from the mode and the best scale known
(within a pixel). `surface.scale` is the scale an image, or a color on the
full-size fallback, is drawn at on that surface (`1.5` from
`wp_fractional_scale_v1`, else an integer; a color on a single-pixel or
1×1 buffer is always drawn at 1) and `surface.pixels` the size in device
pixels an image is drawn at, both `null` until the surface is
configured. `surface.state` is `waiting` (the
output has not reported itself yet), `pending`, `configured` (with `size`),
`closed` (the compositor closed it; scootbg makes it again, once) or
`gave-up` (closed a second time over the output's life: scootbg has
stopped trying on that output, says so on stderr, and the other outputs
are unaffected; it lasts until that output is unplugged and plugged back
in, which makes it a new output). `shows` is what the output has on
screen: `{"color":"#rrggbb"}` (lowercase),
`{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`,
or `null` for nothing. `draw_failed` is `true` when the last attempt to
draw what the output should show failed (an image that exists but cannot
be decoded, a buffer too large; stderr says why), which tells that `null`
apart from a `clear`; the next request for the output, or a new size,
retries. `saving` (after the list) is `false` while `set` and `clear` are
not saved for the next start (see [Restore](#restore)), and `profile` is the profile
whose state is restored and saved. New keys
may be added; none changes meaning within protocol 1. If the daemon cannot
accept clients (out of file descriptors, say), it keeps the wallpaper up,
says so once on stderr, and retries every second rather than exit. The
control protocol itself (one JSON object per line, for scripts that skip
the CLI) is in [README.md](README.md#the-control-protocol).
scoot's `[wallpaper]` section, which runs `apply-config`, is in
[configuration.md](../configuration.md#wallpaper).
