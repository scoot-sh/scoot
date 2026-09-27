# scootbg

The lightest wallpaper daemon for Wayland, written in Rust, in place of
`awww` (formerly `swww`), `swaybg`, `hyprpaper`, `wpaperd` and friends.
It shows a color or an image on each output, and one command changes it.

It is built for scoot and set up by scoot's own config, but it is not tied
to it: scootbg speaks only standard protocols, so it also runs on any
compositor with `wlr-layer-shell-v1` (sway, niri, Hyprland, river, labwc).

> **Status: early.** The daemon exists (`crates/scootbg/`, with its
> `unsafe` in `crates/scootbg-mem/`): `scootbg daemon` connects, tracks
> every output as it comes and goes, and gives each one a `background`
> layer surface. **Solid colors and images work**: `scootbg set '#rrggbb'`
> or `scootbg set PATH` (PNG, JPEG, WebP) with a fit mode, on every output
> or one (`--output NAME`), and `scootbg clear`, drawn at each output's
> real device pixels, fractional scales included; `query` reports what
> each output shows, and `version` and `kill` work. **The wallpaper is
> restored at the next start**, per profile ([below](#restore)). Not yet:
> scoot's `[wallpaper]` section, the next item in
> [`backlog/`](backlog/README.md). These docs stay here.

## What it is for

- **The lowest resource use of any wallpaper daemon.** Not a goal but a
  release gate: v1 does not ship while any competitor beats scootbg,
  beyond a noise margin, on any measure both can do (CPU, idle wakeups,
  memory, peak memory while decoding, binary size, startup). The
  competitors are `swaybg`, `awww`, `hyprpaper`, `wpaperd` and `wbg`, on
  the same machine, with the table published here. Every
  dependency has to justify its bytes. See
  [`backlog/lightest.md`](backlog/lightest.md).
- **Colors and images.** A solid color or a PNG, JPEG or WebP image per
  output, changed live with one command and restored at login. That is
  v1.
- **Seamless in scoot.** A `[wallpaper]` section in scoot's `config.toml`
  is all it takes: scoot starts scootbg and re-applies the section on
  `scootctl reload`. No autostart entry, no session script.
- **Free when idle.** A static wallpaper costs nothing after it is on
  screen: no timers, no frame callbacks, no wakeups, one buffer per output
  at most (outputs of one size showing one image share one), no file
  descriptor held per buffer, and the decoded source image dropped once it
  is scaled. Measured: [the resource budget](#the-resource-budget).
- **GPU-free.** Decoding, scaling and (later) transitions all run on the CPU
  into `wl_shm` buffers, the same bar the compositor holds itself to for
  webtop and no-GPU boxes. A GPU path, if ever, is an optional tier.
- **Sharp at any scale.** Buffers are drawn at the output's real device
  pixels, fractional scales included, never scaled up by the compositor.
  Working, and measured ([below](#measured-so-far)).
- **Scriptable.** A small CLI over a control socket with JSON replies, like
  `scootctl`, so an agent or a script can set, query and clear wallpapers.

Transitions (fade, wipe, grow) and animated images (GIF, APNG, animated
WebP) are the next milestone, not v1. They are the features people move to
awww for, and they are also a steady CPU cost under a software renderer,
so they arrive once the static path is measured and they can be held to a
per-frame budget.

## How it will work

In scoot, the whole setup is to be one config section. **This is planned,
not working:** no scoot release accepts `[wallpaper]` yet, and scoot's
config rejects unknown sections by ignoring the *whole* file, keybindings
and layout included. Do not add it until the
[integration item](backlog/scoot-integration.md) lands.

```toml
# ~/.config/scoot/config.toml  (planned; see above)
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"                    # fill | fit | stretch | center | tile

[wallpaper.output."DP-2"]        # optional, per output
color = "#101014"
```

And one command changes it live, in scoot or anywhere else:

```sh
scootbg set ~/Pictures/city.png                # every output
scootbg set '#1e1e2e'                          # a color: anything starting with '#'
scootbg set ./#draft.png                       # a file whose name starts with '#'
scootbg set ~/Pictures/city.png --output DP-1 --mode fit --fill '#101014'
scootbg query                                  # what each output shows, as JSON
scootbg clear --output DP-1                    # back to the compositor's own background
scootbg daemon                                 # outside scoot: start it yourself
scootbg daemon --profile sway                  # ... restoring and saving the `sway` profile
```

Every command above works today (`set` also takes `--filter
lanczos3|catmull-rom|bilinear|nearest`, and `daemon` `--no-restore`); what
does not yet is the `[wallpaper]` section. The root
[README](../../README.md#scootbg-early) has the details (exit codes, what
`set` waits for). One binary: `daemon` runs the Wayland client, every
other subcommand talks to it over its socket.

- **One `background`-layer surface per output**, anchored to all four edges,
  exclusive zone `-1`, no keyboard interactivity and an empty input region,
  so clicks reach the desktop underneath, as on any other compositor.
- **Solid colors** use `wp_single_pixel_buffer_manager_v1` scaled by
  `wp_viewporter`: no shared memory at all for a whole output, with a 1×1
  `wl_shm` buffer under the viewport where the compositor lacks
  single-pixel buffers, and a full-size `wl_shm` buffer where it has no
  viewporter either. Working, and measured
  ([below](#measured-so-far)).
- **Images** are decoded on a worker thread, never on the Wayland loop,
  once for every output that needs them, scaled once per buffer size, and
  written into an opaque `XRGB8888` buffer at the output's device pixels,
  with its opaque region set, so a compositor can skip drawing anything
  beneath it. With `wp_fractional_scale_v1` and `wp_viewporter` that is
  the surface's logical size times the scale the compositor asks for,
  rounded as that protocol says, under a viewport (1601×1001 for scoot's
  1067×667 at 1.5 on a 1600×1000 output, which scoot draws one to one);
  without them, or for a moment while the compositor has not told the
  surface its new scale, the surface size times the integer scale (the
  larger of `wl_surface.preferred_buffer_scale` and `wl_output.scale`),
  which the compositor scales down, never up. The decoded image is dropped once drawn (an
  output plugged in later shares the pixels of an output of its size
  showing the image, and otherwise reads the file again). **Outputs of one
  size showing one image share one buffer's memory**: one memfd, mapped
  once, with a `wl_buffer` per output over it (one `wl_buffer` on several
  surfaces would leave its releases undefined, `wl_surface.attach` says),
  so a second 4K output costs no second 32.4 MB. EXIF orientation is
  applied as the pixels are packed, with no extra buffer. A file that
  cannot be shown (missing, not PNG/JPEG/WebP, over 16384×16384 pixels,
  truncated or corrupt) is an error reply that changes nothing. Working,
  and measured ([below](#measured-so-far)).
- **Fit modes:** `fill` (cover and crop, the default), `fit` (letterbox
  with a color), `stretch`, `center`, `tile`.
- **Outputs come and go.** A monitor plugged in gets the wallpaper meant
  for it (by connector name, or the "every output" choice), and one unplugged
  frees its buffer (or its share of one: the others keep showing it).
- **Restore.** The last choice per output is kept in
  `$XDG_STATE_HOME/scootbg/PROFILE`, so `scootbg daemon` at login brings
  it back. Working ([below](#restore)).

## Restore

Every `set` and `clear` the daemon records is saved, and `scootbg daemon`
shows it again when it starts.

- **One state file per profile:** `$XDG_STATE_HOME/scootbg/PROFILE`, or
  `~/.local/state/scootbg/PROFILE` when `XDG_STATE_HOME` is unset, empty
  or relative (the XDG base directory spec). With neither it nor `HOME`
  an absolute path there is nowhere to keep state: nothing is restored
  or saved, and the daemon says so on stderr. `scootbg daemon --profile
  NAME` picks the profile, `default` when not given. A name is 1 to 64
  of `A-Z a-z 0-9 . _ -`, not starting with `.` and without `..`, so it
  is one plain file name; anything else is a usage error (exit 2).
  Profiles, not displays: scoot binds the first free `wayland-N`, so the
  socket name shifts with start order and is no session identity.
  Sessions with different profiles never restore each other's wallpaper;
  two sharing one share it, the last change winning.
- **What is saved** is the daemon's choices as `set` and `clear` made them:
  the one for every output, and each one made with `--output NAME`, by
  connector name, plugged in now or not. A color or a `clear` is saved as
  soon as the daemon has it, an image once it has decoded (one that
  cannot be shown changes nothing, so saves nothing). A request superseded
  by a newer one before it landed saves nothing: the file follows the
  same newest-wins rule as the screen.
- **What survives:** a restore itself writes nothing. Choices for outputs
  not plugged in now, and images that could not be restored, stay saved
  until a `set` or `clear` replaces them; a `set` for other outputs keeps
  them. A `set` or `clear` without `--output` replaces every per-output
  choice, as it does on screen.
- **Written off the loop, atomically:** a temporary file beside it
  (`.PROFILE.PID.tmp`), `fsync`, `rename` over the old file, `fsync` of
  the directory; the file is 0600, a directory it makes 0700. The write
  runs on a thread started for it, which ends once nothing more waits
  (an idle daemon has one thread); the loop only builds the text and
  hands it over (about 26 µs), so a slow disk never stalls it. While a
  write is under way a newer text replaces any that waits, so a burst of
  changes is at most two writes, ending with the last. `scootbg kill`
  waits for a write under way (up to 2 s) before the daemon exits; a
  signal leaves the old file or the new one, whole, and at worst a
  temporary file.
- **Restoring:** the choice for every output first, then each per-output
  one, before the daemon serves any request (so any request is newer).
  Each output is drawn when its `configure` comes; an image is decoded
  once for them. **A saved image that is gone** (checked with one `stat`,
  not a decode) is skipped with a warning on stderr, and that output shows
  the compositor's own background; the daemon starts all the same. One
  that exists but cannot be decoded fails when it is drawn, and says so
  then. `--no-restore` shows nothing at start; the file is still read, so
  a `set` then updates it, keeping the rest.

**The file** is a hand-written line format, versioned
([`dependencies-done.md` §5](backlog/resolved/dependencies-done.md#5-serialization-control-socket-and-state-file)
chose it over TOML, 180 KB lighter):

```text
scootbg-state 1
profile default
fingerprint 9c1e…
all color #1e1e2e
output DP-1 image /home/me/My%20Pictures/hills.jpg fill #000000 lanczos3
output HDMI-A-1 clear
```

- The first line is `scootbg-state` and the version. Each other line is a
  key and its fields, single spaces between. `all` and `output NAME` take
  `clear`, `color #rrggbb`, or `image PATH MODE FILL FILTER` (a path is
  absolute). `profile` names the profile the file belongs to (the file
  name is the authority; a mismatch is a warning). `fingerprint` is
  reserved for scoot's `[wallpaper]` section
  ([scoot-integration.md](backlog/scoot-integration.md)): only
  `apply-config` will write it, and every other save keeps it as read.
- **Fields are escaped:** `%`, space and every ASCII control byte (newline
  and tab included) are written `%XX`, so any path or connector name
  round-trips exactly, `#` and spaces included. Other bytes, UTF-8
  included, are written as they are.
- **Read defensively; never fatal.** A malformed line (a field missing or
  extra, a bad escape, a relative path, an unknown mode or filter), one
  whose fields are not UTF-8 once unescaped, or an unknown key is skipped
  with a warning naming its line, and the rest is used. The same key
  twice: the later line counts, with a warning. More than 256 `output`
  lines: the rest are skipped. A file over 256 KiB, or whose first line is
  not `scootbg-state` and a version, restores nothing and is replaced at
  the next save. **A later version** (a newer scootbg's file) restores
  nothing and is never written over, so a downgrade cannot destroy what
  the newer build saved. A file that cannot be read (permissions, not a
  regular file) restores nothing, and nothing is saved over it that
  session.
- **Compatibility:** within version 1 a key may be added only if a reader
  that skips it (with its warning) loses nothing it needs; anything else
  bumps the version. `profile` and `fingerprint` are read and kept from
  version 1 on, so the scoot integration writes them with no bump.

## The control protocol

Every command but `daemon` is one request on the daemon's socket,
`$XDG_RUNTIME_DIR/scootbg-NAME.sock` (`NAME` the last component of
`$WAYLAND_DISPLAY`), so a script or an agent can skip the CLI: one JSON
object per line each way, each request naming the protocol it speaks.

```text
{"protocol":1,"type":"set","color":"#1e1e2e"}                 -> {"type":"ok"}
{"protocol":1,"type":"set","color":"#1e1e2e","output":"DP-1"} -> {"type":"ok"}
{"protocol":1,"type":"set","image":"/abs/a.jpg"}              -> {"type":"ok"}
{"protocol":1,"type":"set","image":"/abs/a.jpg","mode":"fit","fill":"#101014","filter":"lanczos3","output":"DP-1"}
                                                              -> {"type":"ok"}
{"protocol":1,"type":"clear"}                                 -> {"type":"ok"}
{"protocol":1,"type":"clear","output":"DP-1"}                 -> {"type":"ok"}
{"protocol":1,"type":"query"}                                 -> {"type":"outputs","outputs":[...]}
{"protocol":1,"type":"version"}                               -> {"type":"version","protocol":1,"version":"..."}
{"protocol":1,"type":"kill"}                                  -> {"type":"ok"}
anything wrong                                                -> {"type":"error","message":"..."}
```

- `set` and `clear` answer once every targeted output shows the change and
  a `wl_display.sync` sent after the commits has come back, so the
  compositor has it; the daemon never blocks on that, other clients are
  served meanwhile. Requests sent behind one on the same connection are
  answered after it, in order. An output unplugged before its commit is
  left out of the wait; an output whose surface is not configured yet is
  waited for, until a `wl_display.sync` sent after scootbg made that
  surface comes back: one the compositor has not configured by then no
  longer holds up replies (said on stderr, once per surface), and is drawn
  when its `configure` comes. No compositor checked is that slow (scoot
  and sway configure before the round trip returns). An output scootbg gave up on (`gave-up` in `query`, said on
  stderr) is left out and shows nothing, and the reply is still `ok`. A client that hangs up before its reply loses only the
  reply: the change still happens.
- `set` takes a `color` or an `image`, never both. An `image` is an
  absolute path (the daemon's working directory is not the client's;
  `scootbg set` resolves a relative one before sending it) to a PNG, JPEG
  or WebP; `mode` (`fill`, `fit`, `stretch`, `center`, `tile`), `fill`
  (`#rrggbb`) and `filter` (`lanczos3`, `catmull-rom`, `bilinear`,
  `nearest`) may be left out, for `fill`, `#000000` and `lanczos3`, and
  are refused with a `color`.
- **An image changes nothing until it has been decoded.** One that cannot
  be shown gets an error reply saying why (no such file, not a regular
  file, not an image scootbg reads, image too large, truncated or
  corrupt), and every output keeps what it showed. Decoding runs on a
  worker thread; other requests are served meanwhile.
- **The newest request wins**, whatever order the work finishes in: a
  `set` or `clear` sent after an image's `set` is never undone when that
  image finishes decoding. An image request that newer ones have replaced
  on every output it asked for, before it was decoded or while, is
  answered `ok` once what replaced it is on screen, as a replaced color
  is, and changes nothing (it may never be decoded at all). At most 32 image
  requests wait at once; one more is refused ("too many images are
  waiting"), nothing changed.
- A request refused outright changes nothing: an unknown `output` name
  (the name must belong to an output present now), a `color` that is not
  `#rrggbb`, a relative `image` path, an unknown `mode` or `filter`. A
  draw that fails on an output (a buffer too large for `wl_shm`, out of
  memory) is different: the choice is recorded and every other targeted
  output shows it; the reply is an error once they have, and the daemon's
  stderr says which output failed and why.
- `query`'s `surface.scale` is the scale an image, or a color on the
  full-size fallback, is drawn at on that output (`1.5` from
  `wp_fractional_scale_v1`, else an integer; a color on a single-pixel or
  1×1 buffer is drawn at 1 whatever it says) and
  `surface.pixels` the size of an image's buffer in device pixels, both
  `null` until the surface is configured; the top-level `scale` stays
  `wl_output`'s integer.
- `query`'s `shows` is `{"color":"#rrggbb"}` (lowercase),
  `{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`,
  or `null`. Every key of an entry is always present; keys may be added
  within protocol 1, none removed or changed.
- A choice for every output is kept for outputs plugged in later; a choice
  for one output is kept by its name, across unplugging it, and across a
  daemon restart ([Restore](#restore)).
- **An image `set` sent before its outputs are configured** (as the
  daemon starts) waits to be decoded until none of the outputs it targets
  is about to be configured, bounded by a round trip the daemon has
  already sent, so the file is decoded once, for them (it used to be
  decoded once to check it and again for the `configure`).

## The resource budget

What scootbg costs, measured against `scoot --headless` (a debug build,
scale 1) on a 4-CPU Claude Code web container with no GPU, release
builds; the method and every raw number are in
[memory-and-idle-done.md](backlog/resolved/memory-and-idle-done.md#measurements).
The image is a 6000×4000 JPEG, `fill`.

| What | Result |
|---|---|
| **Idle**, 60 s windows, a color or the image on 3840×2160 | 0 context switches and 0 CPU ticks in every window (`/proc`); `perf stat` counts no task-clock, context switch, page fault or wakeup: the daemon never runs. One thread. |
| **Memory at rest with the image**, 1× 1920×1080 | RSS 12.3–12.4 MB, PSS 7.1–7.3 MB (the 8.1 MB buffer is shared with the compositor, so PSS counts half); anonymous 456 kB, `[heap]` 40 kB; 8 fds |
| 1× 3840×2160 | RSS 36.7–36.8 MB, PSS 19.4–19.5 MB; anonymous 564–572 kB, `[heap]` 40 kB; 8 fds |
| **2× 3840×2160** | **RSS 36.7–36.8 MB, PSS 19.4–19.6 MB**, the same as one output: both show one 32.4 MB buffer's pixels (69.2 MB and 35.7–35.8 MB before ticket 8) |
| Memory at rest with a color (a single-pixel buffer: no shared memory at any size), on 3840×2160 | RSS 3.6–3.7 MB, PSS 2.0–2.1 MB; anonymous 196–200 kB |
| **Peak while changing** the image on 2× 4K | 120.4–120.7 MB (152.9–153.0 MB before ticket 8); 87.9–88.1 MB for a first image |
| **Startup**, `scootbg daemon` to its first answer | 1.8–2.5 ms |
| to a color on screen (a `set` sent at once, answered after the commit and a round trip) | 3.0–4.1 ms |
| to the image on one 4K output, likewise | 450–491 ms, one decode (705–718 ms before ticket 9, two decodes; a `set` once the output is configured takes 437–468 ms) |
| **to a restored image** on one 4K output (the state file names the JPEG) | 443–477 ms to the first `query` that reports it shown: one decode, [restore-state-done.md](backlog/resolved/restore-state-done.md#measurements) |
| Saving a choice | about 26 µs on the loop (build the text, hand it over); the atomic write on a thread of its own, median 334 µs, worst 54.5 ms of 200 on ext4 |
| File descriptors | 8 whatever is shown: a buffer's memfd is closed once the compositor has it |

## Measured so far

Release build, against `scoot --headless --outputs 2` (1600×1000 each,
a debug build of scoot; the image rows on one 3840×2160 output), on a
4-CPU Claude Code web container; the records, with the method and every
raw number, are in
[solid-color-done.md](backlog/resolved/solid-color-done.md#measurements),
[images-decode-and-fit-done.md](backlog/resolved/images-decode-and-fit-done.md#measurements),
[hidpi-fractional-scale-done.md](backlog/resolved/hidpi-fractional-scale-done.md#measurements)
and [memory-and-idle-done.md](backlog/resolved/memory-and-idle-done.md#measurements).
Not yet against competitors: that is [lightest.md](backlog/lightest.md).

| What | Result |
|---|---|
| Stripped binary | 1,500,008 B with images (783,072 B with colors only); links only `libc.so.6`, `libm.so.6` and `libgcc_s.so.1` |
| `set` of a 6000×4000 JPEG onto a 3840×2160 output, request to reply, ×3 | 397.0–433.6 ms, 390–420 ms of CPU (PNG 408.1–417.6 ms; WebP 1,218.0–1,289.4 ms); peak RSS 120.5–120.6 MB with the previous wallpaper still mapped (88.0 MB for a first set; PNG 120.4–120.6 MB; WebP 142.0–142.1 MB) |
| After it, idle 30 s | 1 thread, heap 372–568 kB, one 32.4 MB buffer; 0 context switches, 0 CPU |
| A few hundred bytes claiming 16384×16384 (PNG, JPEG, WebP) | refused in under 1 ms; peak RSS within 72 kB of before |
| Idle with a color set, 30 s ×3 | 0 context switches, 0 CPU ticks; RSS 2,720 kB, PSS 1,524 kB, 1 thread |
| PSS with a color, 2 outputs: single-pixel / 1×1 shm / full-size shm | 1,556–1,560 / 1,556–1,560 / 7,808 kB (14,060 kB once a change left a spare buffer per output, until ticket 8: no spare is kept now) |
| `set`, request to reply, 10,000 changes ×3 | median 400–433 µs, p99 1.6 ms; no memory growth |
| The same JPEG at scale 1.5 on 3840×2160: buffer, and request to reply ×3 | 33,177,600 B (was 58,982,400 B at `wl_output`'s 2); 420.0–434.4 ms (was 494.9–507.3 ms). On 1600×1000: 6,410,404 B (was 11,387,024 B) |

## Relation to scoot

- **scoot drives scootbg, never the reverse.** At startup and on each
  reload, while `[wallpaper]` exists, scoot spawns one command,
  `scootbg apply-config`, with the section's values. It starts the daemon
  if none is running, otherwise hands the values over, and changes the
  wallpaper only if the section itself changed. scoot never waits on it
  from its event loop. scoot depends only on scootbg's CLI,
  not its crate, and scootbg knows nothing about scoot's config, so each
  stays usable without the other. See
  [`backlog/scoot-integration.md`](backlog/scoot-integration.md).
- **Config versus `scootbg set`: whichever you changed last wins.** Edit
  `[wallpaper]` and the config's wallpaper shows. Run `scootbg set` after
  that and your choice shows, across restarts and unrelated reloads, until
  you next change `[wallpaper]` itself.
- **Packaging.** `scootbg` is its own package, like `scootctl`. scoot runs
  it from `PATH` (or `[wallpaper] command`), and the home-manager module
  installs it and points scoot at it when `[wallpaper]` is set.
- scoot's `[appearance] background_color` stays: it is the frame clear
  color, what shows with no wallpaper client at all. scootbg draws over it
  on the background layer.
- The compositor's `scootctl screenshot` and `ext-image-copy-capture-v1`
  already include layer surfaces, so screenshots show the wallpaper with no
  extra work.
- scoot's headless backend is the test harness: start
  `scoot --headless --outputs 2`, run scootbg in it, and check real pixels
  with `scootctl screenshot --output N`, the way `scripts/smoke-test.sh`
  already checks the compositor.
- A wallpaper per workspace is planned through `ext-workspace-v1`, a
  standard protocol, so it is not scoot-only either.

## Licensing

MIT, like the rest of scoot. awww/swww are GPL-3.0 and may inspire the
design (their feature set is the bar to meet), but no code, shaders or
assets are copied from them, nor from any other GPL wallpaper daemon.
Every dependency's licence is checked before it is added. All of the
chosen set is permissive and MIT-compatible: MIT, Apache-2.0, Zlib,
Unlicense, and BSD-3-Clause OR Apache-2.0 for the scaler,
`pic-scale-safe`, whose notice a binary package carries alongside the
others. The set and every licence are recorded in
[`backlog/resolved/dependencies-done.md`](backlog/resolved/dependencies-done.md#8-licences).

## Standards

scootbg is held to the same bar as the compositor (see the root
`CLAUDE.md`): no allocation on per-frame paths, crash or hang paths treated
as data loss, edge cases (zero outputs, hotplug mid-decode, a 20000×20000
image, a truncated file) considered explicitly, benchmarks before and after
anything on a hot path, and the full per-feature cycle with independent
review before merge. On top of that, as the user set it, speed and
safety are not traded against each other:

- **Pure Rust, no C.** No C calls and no C dependencies: no `libc` crate,
  no `-sys` crate that links a library, no build script that compiles C.
  The honest exception is the Rust standard library itself, which on
  `*-linux-gnu` links glibc, `libm` and `libgcc_s`. `wayland-backend`'s
  and `wayland-sys`'s C-bringing features stay off: `client_system`
  links libwayland, and `log`, although it only adds the pure-Rust `log`
  crate as a dependency, makes `wayland-backend/build.rs` compile two C
  shims (`src/sys/*/log_shim.c`) with `cc`, even for the pure-Rust
  backend. (Checked against the pinned fork, `70f81e00`.) Without
  `log`, the backend reports errors with `eprintln!`, so the daemon
  guards against that instead: see `crates/scootbg/src/daemon/crash.rs`.
- **`#![forbid(unsafe_code)]` in `scootbg`.** The only `unsafe` is
  isolated in one small crate, `scootbg-mem`:
  - the global allocator that returns large blocks to the kernel;
  - the `wl_shm` buffer mapping.

  Signals are deliberately not caught: SIGTERM and friends kill the daemon
  with their default action, and the stale socket file it leaves is
  harmless (a lock, not the file, says a daemon is alive). Catching them
  would have taken a third `unsafe` module on unstable rustix APIs; see
  [the record](backlog/resolved/crate-and-daemon-done.md#departures-from-the-plan-and-why).

  It is named for what it owns rather than `-sys`, which by Cargo
  convention means bindings to a native library, the one thing it
  exists to avoid. Every `unsafe` block there has a written safety
  argument.
- **Malformed input never panics where it can be avoided.** Sizes and
  file contents are validated before they reach a decoder or the scaler,
  and a bad file is an error reply. A panic aborts the daemon.
- **Dependencies are weighed on safety as well as weight:** how much
  `unsafe` they carry and what for, whether they are fuzzed upstream, and
  their RustSec record. Where a dependency is not fuzzed upstream,
  scootbg fuzzes the path through it.

The audit behind these rules, and the one measured cost they carry (CPU
per change from the safe scaler), is in
[`backlog/resolved/dependencies-done.md`](backlog/resolved/dependencies-done.md).
