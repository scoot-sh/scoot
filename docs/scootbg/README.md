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
> layer surface. **Solid colors work**: `scootbg set '#rrggbb'`, on every
> output or one (`--output NAME`), and `scootbg clear`; `query` reports
> what each output shows, and `version` and `kill` work. Images are the
> next items in [`backlog/`](backlog/README.md). These docs stay here.

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
  screen: no timers, no frame callbacks, no wakeups, one buffer per output,
  and the decoded source image dropped once it is scaled.
- **GPU-free.** Decoding, scaling and (later) transitions all run on the CPU
  into `wl_shm` buffers, the same bar the compositor holds itself to for
  webtop and no-GPU boxes. A GPU path, if ever, is an optional tier.
- **Sharp at any scale.** Buffers are drawn at the output's real device
  pixels, fractional scales included, never scaled up by the compositor.
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
```

The commands above are the planned interface. What works so far:
`daemon`, `set` with a color (`#rrggbb`) and `--output`, `clear` with
`--output`, `query`, `version` and `kill`; `set` with a path says images
come in a later version, and `--mode`/`--fill` do not exist yet. The root
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
- **Images** are decoded once, scaled once per output size and scale, and
  written into an opaque `XRGB8888` buffer with its opaque region set, so a
  compositor can skip drawing anything beneath it.
- **Fit modes:** `fill` (cover and crop, the default), `fit` (letterbox
  with a color), `stretch`, `center`, `tile`.
- **Outputs come and go.** A monitor plugged in gets the wallpaper meant
  for it (by connector name, or the "every output" choice), and one unplugged
  frees its buffer.
- **Restore.** The last choice per output is kept in
  `$XDG_STATE_HOME/scootbg/`, so `scootbg daemon` at login brings it back.

## The control protocol

Every command but `daemon` is one request on the daemon's socket,
`$XDG_RUNTIME_DIR/scootbg-NAME.sock` (`NAME` the last component of
`$WAYLAND_DISPLAY`), so a script or an agent can skip the CLI: one JSON
object per line each way, each request naming the protocol it speaks.

```text
{"protocol":1,"type":"set","color":"#1e1e2e"}                 -> {"type":"ok"}
{"protocol":1,"type":"set","color":"#1e1e2e","output":"DP-1"} -> {"type":"ok"}
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
  waited for; an output scootbg gave up on (`gave-up` in `query`, said on
  stderr) is left out and shows nothing, and the reply is still `ok`. A client that hangs up before its reply loses only the
  reply: the change still happens.
- A request refused outright changes nothing: an unknown `output` name
  (the name must belong to an output present now), a `color` that is not
  `#rrggbb`, a `set` with no `color`. A draw that fails on an output (a
  buffer too large for `wl_shm`, out of memory) is different: the choice
  is recorded and every other targeted output shows it; the reply is an
  error once they have, and the daemon's stderr says which output failed
  and why.
- `query`'s `shows` is `{"color":"#rrggbb"}`, lowercase, or `null`. Every
  key of an entry is always present; keys may be added within protocol 1,
  none removed or changed.
- A choice for every output is kept for outputs plugged in later; a choice
  for one output is kept by its name, across unplugging it. Choices are
  not saved across a daemon restart yet
  ([restore-state.md](backlog/restore-state.md)).

## Measured so far

Release build, against `scoot --headless --outputs 2` (1600×1000 each,
a debug build of scoot), on a 4-CPU Claude Code web container; the record,
with the method and every raw number, is in
[solid-color-done.md](backlog/resolved/solid-color-done.md#measurements).
Not yet against competitors: that is [lightest.md](backlog/lightest.md).

| What | Result |
|---|---|
| Stripped binary | 783,072 B; links only `libc.so.6` and `libgcc_s.so.1` |
| Idle with a color set, 30 s ×3 | 0 context switches, 0 CPU ticks; RSS 2,720 kB, PSS 1,524 kB, 1 thread |
| PSS with a color, 2 outputs: single-pixel / 1×1 shm / full-size shm | 1,556–1,560 / 1,556–1,560 / 7,808 kB (14,060 kB once a change leaves a spare buffer per output) |
| `set`, request to reply, 10,000 changes ×3 | median 400–433 µs, p99 1.6 ms; no memory growth |

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
