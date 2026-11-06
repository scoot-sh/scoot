---
title: scootbg overview
description: "The lightest wallpaper daemon for Wayland — color or image on each output, one command to change it."
---

The lightest wallpaper daemon for Wayland, in place of `swaybg`,
`hyprpaper`, `wpaperd` and friends. It shows a color or an image on
each output, and one command changes it. Built for scoot and set up by
scoot's own config, but not tied to it: scootbg speaks only standard
protocols, so it also runs on any compositor with
`wlr-layer-shell-v1`.

```sh
scootbg set '#1e1e2e'                  # every output, including ones plugged in later
scootbg set ~/Pictures/hills.jpg      # an image, covering every output
scootbg set ~/Pictures/hills.jpg --output DP-2 --mode fit
scootbg clear                          # back to the compositor's own background
scootbg query                          # each output and what it shows, one JSON line
```

## The `[wallpaper]` section

In scoot, a `[wallpaper]` section runs all of this for you: scoot
starts scootbg itself and re-applies the section on every reload, with
no `[autostart]` entry and no session script. Leave
`scootbg daemon` out of `[autostart]` and your session script when you
use this section (`apply-config` starts the daemon; a second start is
harmless but flashes the wrong profile's wallpaper for a moment).

```toml
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"                    # fill | fit | stretch | center | tile

[wallpaper.output."DP-2"]        # optional, one table per output
color = "#101014"

[wallpaper.workspace."2"]        # optional, while workspace 2 is active
image = "~/Pictures/focus.jpg"

[wallpaper.output."DP-1".workspace."2"]  # optional, workspace 2 on one output
color = "#101014"
```

A link works where a path does: `image = "https://example.com/hills.jpg"`
is downloaded once and cached by scootbg. `sha256` pins the download's bytes.

| Field | Type | Reload | Meaning |
|---|---|---|---|
| `image` | string (path or URL) | live (re-applied) | A PNG, JPEG, GIF or WebP image. A path: `~` expands against `HOME`, a relative path resolves against the config file's directory. A URL (`http://`/`https://`): downloaded once and cached — see [Wallpaper from a link](./from-url.md). |
| `color` | string (`"#rrggbb"`) | live (re-applied) | A solid color. `image` or `color`, never both; neither is nothing (the compositor's own `background_color`). |
| `mode` | string | live | With an `image` only: `fill` (cover and crop), `fit` (letterbox with the fill color), `stretch`, `center`, `tile`. |
| `fill` | string (`"#rrggbb"`) | live | With an `image` only: the color around a `fit` or `center` image. |
| `filter` | string | live | With an `image` only: `lanczos3`, `catmull-rom`, `bilinear` or `nearest`. |
| `sha256` | string (64 hex digits) | live (re-downloads) | With a URL `image` only: the download's expected SHA-256. Anything else fails instead of showing. Refused beside a path. |
| `transition` | string | live (re-applied) | How the next change arrives: `none` (at once), `fade`, `wipe` or `grow`. See [Transitions](./transitions.md). |
| `duration-ms` | string (digits) | live (re-applied) | With a `transition` only: milliseconds, `0`–`60000` (default `500`). |
| `easing` | string | live (re-applied) | With a `transition` only: `linear`, `ease-in`, `ease-out` (default), `ease-in-out` or `smooth`. |
| `angle` | string (number) | live (re-applied) | A wipe's direction in degrees: `0` from the left, `90` from the top, `180` from the right, `270` from the bottom. |
| `position` | string (`X,Y`) | live (re-applied) | Where a grow starts, as fractions (`0.5,0.5` is the center). |
| `output."NAME"` | table | live | The same keys for one output, by connector name (as `scootbg query` lists them). Each output table stands alone: an output's `image` does not take the top level's `mode`. An empty table is nothing on that output. |
| `workspace."NAME"` | table | live | The same keys while workspace `NAME` is active, on every output (the name is what the compositor announces: `"1"`, `"2"`, ... on scoot). Each workspace table stands alone: it takes no other table's `mode` or `transition`. An empty table takes that mapping back off. Absent tables leave live mappings alone. |
| `output."NAME".workspace."WS"` | table | live | The same keys while workspace `WS` is active on output `NAME` alone. Within one section, where both it and `workspace."WS"` name one output, this table wins for that output (it applies after the global one). Against `set --workspace` the newest change wins. |
| `command` | string | live | The `scootbg` to run. Default `"scootbg"` on `PATH`; the Nix modules set it to the installed package's store path. |

Limits: at most 256 outputs, 64 workspace mappings in a section, and
63 KiB of JSON per section; transition keys (`duration-ms`, `easing`,
`angle`, `position`) without a `transition` are refused.

**Whichever you changed last wins.** Edit `[wallpaper]` (and start or
reload scoot): the config's wallpaper shows, workspace mappings included.
Run `scootbg set` (or `scootbg set --workspace 2`) after that: your pick
shows, and keeps showing across restarts and unrelated reloads, until you
next change `[wallpaper]` itself. A workspace table you remove from the
section leaves its live mapping alone (write it empty to take it off, as
`clear --workspace` does); a mapping a `set --workspace` changed since
keeps showing until the section itself changes.

## One wallpaper per workspace

While one workspace is active, its outputs can show one wallpaper, and
another elsewhere: `scootbg set ... --workspace 2` maps one, `scootbg
clear --workspace 2` takes it off, and switching workspaces switches
the wallpaper (through the mapping's transition). It follows the
standard `ext-workspace-v1` protocol, preloaded so the switch is
instant — see [A wallpaper per workspace](./workspaces.md).

**When it fails, the session carries on** with its `background_color`:
scootbg not installed (a warning naming the command; the next reload
tries again), a failed run (exit status in the log), or a section with
a problem (unknown key, wrong type — the rest of the file still
applies, unlike other tables). **Until scootbg's first frame**, scoot
shows its `background_color`.

## A picture for your lock screen

A locker cannot capture scoot's screen: the compositor publishes no
`wlr-screencopy`. So scootbg, which knows what every output shows, keeps a
blurred copy for it. It follows the live wallpaper (`set`, `clear`, a
slideshow step, a per-workspace mapping, a look applied with `scoot look
apply`), and costs nothing between changes.

It lives in `$XDG_RUNTIME_DIR/scootbg-NAME.lockscreen/` (`NAME` as for the
socket), a `0700` directory:

```text
index
output-3.png
output-4.png
```

`index` has one line per output with a picture: the output's name, a tab,
the picture's full path.

```text
DP-1	/run/user/1000/scootbg-wayland-1.lockscreen/output-3.png
HDMI-A-1	/run/user/1000/scootbg-wayland-1.lockscreen/output-4.png
```

- **One picture per output.** A `set --output DP-1` rewrites only DP-1's
  file. An unplugged output loses its line and file; a new one gets them
  once the wallpaper is drawn there. `clear` removes the picture, so the
  locker's own background shows. A color wallpaper gets a flat picture of
  that color.
- **Small and blurred.** The picture is the wallpaper as the output shows
  it (fit, crop and mode applied), averaged down to at most 320 pixels on
  its long side and blurred. A locker that fills the output with it shows
  a blur with the wallpaper's framing. It is a PNG of tens of kilobytes.
- **Written whole.** Each file and the index are replaced atomically, so a
  locker that starts mid-change reads the old picture or the new.
- **Names.** File names are numbers, never output names. An output whose
  name contains `:` or a control character (tab and newline included) has
  no line, because a locker could not take the name whole.
- **Lifetime.** Starting the daemon empties the directory, and a clean exit
  (`kill`, or losing the compositor) removes it. A signal ends the daemon
  on the spot and leaves the last pictures in place. If the directory
  cannot be made, scootbg says so once and carries on without it.

The desktop profile's locker (`scoot-locker`) reads the index at lock time
and passes one `-i OUTPUT:FILE` to swaylock per line; an output with no
picture shows the blur built from the look, and below that the lock's
color ([Idle and lock](/desktop/idle-lock/)). To do the same by hand (this
assumes a display name of letters, digits and `.`, `_`, `-`; any other byte
is an `_` in the directory's name):

```sh
args=()
display="${WAYLAND_DISPLAY:-wayland-0}"
index="$XDG_RUNTIME_DIR/scootbg-${display##*/}.lockscreen/index"
if [ -r "$index" ]; then
    while IFS=$'\t' read -r name file; do
        args+=(-i "$name:$file")
    done <"$index"
fi
exec swaylock -f "${args[@]}"
```

Cost: the blur is made on the thread that decodes the image, once per
size, so the daemon's loop never waits for it, and a `set` on a 4K output
takes 5 to 10 ms longer (about 205 ms where it took about 200 ms). Nothing
runs between changes: no wakeups, no per-frame work. The picture is kept
with the pixels it was made from (so a workspace switch re-points the lock
screen with no work), about 50 KB per image and size for a photograph.

## What it costs

On a compositor with `wp_single_pixel_buffer_manager_v1` and
`wp_viewporter` (scoot, sway, and most others) a color costs no shared
memory at all: one single-pixel buffer scaled to the output. Without
single-pixel buffers it is a 1×1 `wl_shm` buffer under the viewport, and
without a viewporter a full-size buffer. A static color asks for no frame
callbacks and never wakes the daemon.

Once on screen a wallpaper costs no CPU (no wakeups in a minute, measured)
and one buffer per output at most: about 4.0 MB RSS with a color, 36.8 MB
with an image on a 4K output, and the same 36.8 MB with two 4K outputs
showing it. The measured budget is in
[README.md](https://github.com/scoot-sh/scoot/tree/main/docs/scootbg/README.md#the-resource-budget),
and so is [the comparison](https://github.com/scoot-sh/scoot/tree/main/docs/scootbg/README.md#against-the-other-daemons)
with swaybg, awww, wbg and wpaperd on one machine. It is not yet the
lightest on every row: awww holds 1.1–1.5 MiB less idle RSS (1.0–1.4 MiB
PSS) above the output buffers. `scripts/scootbg-bench/bench.py` re-runs the comparison.



## The daemon and its socket

One daemon per display: its socket is
`$XDG_RUNTIME_DIR/scootbg-NAME.sock`, `NAME` being the last component of
`$WAYLAND_DISPLAY` (`wayland-0` when it is unset or empty). A second `scootbg daemon` refuses while one runs, and
a socket left by a dead one is replaced. The socket
is `0600` from the moment its name exists, whatever your umask (a crash at
start-up can leave an empty `.scootbg-*` directory beside it,
safe to delete). The daemon exits 0 on `kill` and 1 when
the compositor goes away, removing its socket (and its [lock screen pictures](#a-picture-for-your-lock-screen)) either way. A signal
(SIGTERM, Ctrl-C) ends it on the spot and leaves the socket file; the other
commands then say no daemon is running (exit 1), and the next
`scootbg daemon` replaces the file. Every command exits 2 on a usage
error (an unknown command or argument, or a bad `--profile` name).


