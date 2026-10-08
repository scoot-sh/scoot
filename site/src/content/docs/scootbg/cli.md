---
title: CLI reference
description: "Every scootbg command, when it returns, apply-config, query, and exit statuses."
---

Every command, what it waits for, and how it fails — plus `apply-config` (how scoot drives it) and `query` (what each output shows).

Every `scootbg` command, what it waits for, and how it fails. What scootbg
is and why it exists is in [Overview](./index.md); in scoot, a
`[wallpaper]` section runs it for you
([The `[wallpaper]` section](./index.md#the-wallpaper-section)).



## Commands

```sh
scootbg daemon                        # connect to $WAYLAND_DISPLAY and serve the control socket
scootbg daemon --profile sway         # restore and save the `sway` profile's state instead
scootbg daemon --no-restore           # start with nothing shown (the state is kept)
scootbg set '#1e1e2e'                 # every output, including ones plugged in later
scootbg set '#101014' --output DP-2   # one output, by connector name (as `query` lists them)
scootbg set ~/Pictures/hills.jpg      # an image, covering every output (--mode fill)
scootbg set https://example.com/hills.jpg  # a link: downloaded once, cached, then shown
scootbg set https://example.com/hills.jpg --sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
                                      # ...pinned: anything else fails instead of showing
scootbg set ./#draft.png              # a file whose name starts with '#'
scootbg set city.png --output DP-2 --mode fit --fill '#101014'
scootbg set tile.png --mode tile --filter nearest
scootbg set ~/wallpapers --every 30m --shuffle
                                       # a slideshow: its files in turn, one every 30 minutes
scootbg set '#101014' --transition fade --duration-ms 800
scootbg set city.png --transition wipe --angle 90
scootbg set grid.png --transition grow --position 0,0 --output DP-1
scootbg clear                         # back to the compositor's own background
scootbg clear --output DP-2           # ... on one output
scootbg query                         # each output, its surface and what it shows, one JSON line
scootbg version                       # the running daemon's version and protocol
scootbg kill                          # stop it; returns once a new daemon can start
scootbg apply-config --profile scoot '{"color":"#1e1e2e"}'
                                      # what scoot runs with its [wallpaper] section
scootbg --help                        # and `scootbg COMMAND --help`
scootbg --help --json                 # the same content as JSON (see below)
```

The binary documents itself for agents: `scootbg help COMMAND` prints each
command's page, and `scootbg --help --json` emits commands, `set`'s modes
and filters, exit codes and environment — see
[Generated CLI pages](../reference/cli.md) for the contract.



## When a command returns, and its exit status

**`set` and `clear` return once it is on screen:** every targeted output
shows the change and the compositor has processed it (a `wl_display.sync`
round trip after the commits), so a screenshot taken straight after shows
it. With `--transition`, that means once the animation has finished: the
reply waits for the last frame, not the first. A newer `set`
mid-transition starts from the frame showing then (no queue), and the
earlier reply arrives once what replaced it is on screen. An output unplugged meanwhile is left out of that wait; one whose
surface is not configured yet is waited for, but only until a round trip
after scootbg made that surface: one the compositor has not configured
by then no longer holds up the reply (the daemon says so on stderr) and
is drawn when it is; one scootbg has given up on
(`gave-up` in `query`, said on stderr) is left out and shows nothing, and
`set` still exits 0. They print nothing on
success. **A transition flag without `--transition` is refused** (exit
2), as is a `--transition` outside `none`/`fade`/`wipe`/`grow`, a
`--duration-ms` past `60000`, a non-number `--angle`, a `--position`
outside `X,Y` fractions — and a `clear` with any of them. How a change
animates is [Transitions](./transitions.md). **An image that cannot be shown** (no such file, not a regular
file, not a PNG/JPEG/WebP, too large, truncated or corrupt — or, for a
link, no `curl`, no network, an HTTP error, an error page, a file past
32 MiB, or a `sha256` mismatch: see
[a wallpaper from a link](./from-url.md#a-wallpaper-from-a-link)) is an error
saying why, and every output keeps what it showed. **The newest request
wins**: a `set` or `clear` sent while an earlier image is still decoding is
never undone when that image finishes; the earlier `set` changes nothing
and returns 0 once the newer one is on screen, as a replaced color's `set`
does. Exit status: 0 done; 1 no daemon running,
unknown output, image that cannot be shown, or drawing failed
(`scootbg query`'s `draw_error` says why, as the daemon's stderr does);
2 usage error, a malformed color, an unknown
`--mode`/`--filter`, or a refused transition included. A `set` that has to scale the image
(`fill`, `fit` or `stretch` when the image's size differs from the output's,
with any `--filter`) first probes the scaler's whole budget — the output,
each scaled axis's weight tables, and the row scratch when both axes scale
— and fails here when the
daemon has no room for it (an address-space limit, strict overcommit)
instead of ending the daemon; the outputs keep what they showed and the
next `set` retries. See
[Troubleshooting](./troubleshooting.md#symptoms).



## Slideshows

**`scootbg set DIR --every DURATION [--shuffle]`** cycles a directory's
regular files, one every `DURATION`, on a single timer, without polling
the directory: it is listed once, when set, so a file added later starts
showing after the next `set` of the directory. `DURATION` is a number and
`s`, `m`, `h` or `d`, such as `30m`: at least `1m`, whole minutes, at most
`7d`. The files go sorted by name, or shuffled once with `--shuffle`;
`--mode`, `--fill`, `--filter` and `--transition` apply to every file, as
for one image. At most 10,000 files are listed: a larger directory is
refused outright (exit 1, naming the cap) rather than stalling the
daemon's loop to list it. The first file shows before `set` returns, as an image
does; a file that is not an image fails to draw when its turn comes (as a
`set` of it would, `draw_failed` in `query` saying why) until the next
rotation. One slideshow runs at a time. A new `set`, a `clear`, or a
changed `apply-config` stops it (an unchanged section leaves it running,
like a `set` made since); restarting the daemon shows the last image
without resuming it. If the directory itself goes away mid-rotation, the
slideshow stops at the next step — said on the daemon's stderr, and
`query` no longer reports a `rotation` — instead of failing once a
minute until stopped; a re-created directory starts with a fresh `set`.
With no slideshow the timer does not exist: no file
descriptor, no wakeups. With one, the daemon wakes at most once a minute.

> **Symptom:** *`set` says the directory needs `--every`.*
> A bare directory is only ever a slideshow: `scootbg set DIR` is a
> usage error (exit 2) naming `--every`. Diagnose with the pace it
> suggests, e.g. `scootbg set ~/wallpapers --every 30m`.
>
> **Symptom:** *`set` says the directory holds no files.*
> An empty directory reaches the daemon, which refuses it (exit 1),
> changing nothing. Diagnose with `ls -la DIR` — hidden files count,
> subdirectories do not.
>
> **Symptom:** *one step shows nothing / `draw_failed` until the next step.*
> That file is not an image scootbg reads (told apart by content, not
> name), so its turn fails like a `set` of it would and the next file
> follows on schedule. Diagnose with `scootbg query` (`draw_error`
> says why, as the daemon's stderr does) and `scootbg set FILE` on the
> file itself.
>
> **Symptom:** *the slideshow stopped after the directory moved.*
> That is the design above, not a crash: the daemon says so on stderr
> and `query` drops `rotation`. Diagnose with `scootbg query` (no
> `rotation` object) and re-`set` the new path.

## `apply-config`

**`scootbg apply-config [--profile NAME] JSON`** is how a config, rather
than a person, sets the wallpaper: scoot runs it at start-up and on
every reload with its `[wallpaper]` section as JSON (`{}` once the section
is gone). The JSON is one object: `image` (an absolute path, or an
`http(s)` URL, downloaded once and cached) or `color`
(`#rrggbb`), with `mode`/`fill`/`filter` for an image as `set` takes them
and `sha256` (64 hex digits) for a URL `image` as `--sha256` takes it;
`output` holding a table of the same keys per connector name (each table
stands alone: an output's image does not take the top level's `mode`);
and `command` (scoot's, ignored). Anything else (an unknown key, a key
given twice, `null`, a relative path, a non-`http(s)` URL, a bad color,
hash, mode or filter, over
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
[README.md](https://github.com/scoot-sh/scoot/tree/main/docs/scootbg/README.md#apply-config-scoots-wallpaper-section).



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
 "draw_failed":false,"draw_error":null,"shows":{"color":"#1e1e2e"},
 "transition":null}],
 "saving":true,"profile":"default"}
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
with `"url":"https://..."` beside `image` for a download (the link the
cached file came from),
or `null` for nothing. `draw_failed` is `true` when the last attempt to
draw what the output should show failed (an image that exists but cannot
be decoded, a buffer too large, a scaling draw the daemon had no room
for, a link that would not download), which tells that `null` apart from a
`clear`; the next request for the output, or a new size, retries.
`draw_error` says why while `draw_failed` is `true` (the error the
daemon's stderr gives, such as `"no such file"`, `"shared memory:
Cannot allocate memory (os error 12)"`, or `"out of memory: cannot
allocate ... bytes for scaling"`), and is `null` otherwise; it is
for a person or an agent to read, and its wording may change. `transition`
is the animation running on the output now (`"fade"`, `"wipe"` or
`"grow"`, else `null`; `none` lands at once and is never reported). While a
slideshow runs (`scootbg set DIR --every`, below), a `rotation` object
follows `profile`: `{"directory":"/home/me/wallpapers","every_secs":1800,
"shuffle":false,"files":12}` — what it cycles, every how many seconds, in
what order, and how many files that is. Which file shows now is each
output's `shows`, as for a `set`. Absent while no slideshow runs, so a
static wallpaper's reply is what it was. `saving`
(after the list) is `false` while `set` and `clear` are not saved for
the next start (see [Restore](./restore.md#restore)), and `profile` is the profile
whose state is restored and saved. New keys
may be added; none changes meaning within protocol 1. If the daemon cannot
accept clients (out of file descriptors, say), it keeps the wallpaper up,
says so once on stderr, and retries every second rather than exit. The
control protocol itself (one JSON object per line, for scripts that skip
the CLI) is in [README.md](https://github.com/scoot-sh/scoot/tree/main/docs/scootbg/README.md#the-control-protocol).
scoot's `[wallpaper]` section, which runs `apply-config`, is in
([The `[wallpaper]` section](./index.md#the-wallpaper-section)).
