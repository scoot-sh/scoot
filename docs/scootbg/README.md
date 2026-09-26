# scootbg

The lightest wallpaper daemon for Wayland, written in Rust, in place of
`awww` (formerly `swww`), `swaybg`, `hyprpaper`, `wpaperd` and friends.
It shows a colour or an image on each output, and one command changes it.

It is built for scoot and set up by scoot's own config, but it is not tied
to it: scootbg speaks only standard protocols, so it also runs on any
compositor with `wlr-layer-shell-v1` (sway, niri, Hyprland, river, labwc).

> **Status: planning.** There is no code yet. This file is the design; the
> work is in [`backlog/`](backlog/README.md). The code will live in
> `crates/scootbg/` once its first feature lands; these docs stay here.

## What it is for

- **The lowest resource use of any wallpaper daemon.** Not a goal but a
  release gate: v1 does not ship while any competitor beats scootbg,
  beyond a noise margin, on any measure both can do (CPU, idle wakeups,
  memory, peak memory while decoding, binary size, startup). The
  competitors are `swaybg`, `awww`, `hyprpaper`, `wpaperd` and `wbg`, on
  the same machine, with the table published here. Every
  dependency has to justify its bytes. See
  [`backlog/lightest.md`](backlog/lightest.md).
- **Colours and images.** A solid colour or a PNG, JPEG or WebP image per
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
scootbg set '#1e1e2e'                          # a colour: anything starting with '#'
scootbg set ./#draft.png                       # a file whose name starts with '#'
scootbg set ~/Pictures/city.png --output DP-1 --mode fit --fill '#101014'
scootbg query                                  # what each output shows, as JSON
scootbg clear --output DP-1                    # back to the compositor's own background
scootbg daemon                                 # outside scoot: start it yourself
```

The commands above are the planned interface, not a working one yet.
One binary: `daemon` runs the Wayland client, every other subcommand talks
to it over its socket.

- **One `background`-layer surface per output**, anchored to all four edges,
  exclusive zone `-1`, no keyboard interactivity and an empty input region,
  so clicks reach the desktop underneath, as on any other compositor.
- **Solid colours** use `wp_single_pixel_buffer_manager_v1` scaled by
  `wp_viewporter`: one pixel of memory for a whole output, with a 1×1
  `wl_shm` buffer where the compositor lacks the protocol.
- **Images** are decoded once, scaled once per output size and scale, and
  written into an opaque `XRGB8888` buffer with its opaque region set, so a
  compositor can skip drawing anything beneath it.
- **Fit modes:** `fill` (cover and crop, the default), `fit` (letterbox
  with a colour), `stretch`, `center`, `tile`.
- **Outputs come and go.** A monitor plugged in gets the wallpaper meant
  for it (by connector name, or the "every output" choice), and one unplugged
  frees its buffer.
- **Restore.** The last choice per output is kept in
  `$XDG_STATE_HOME/scootbg/`, so `scootbg daemon` at login brings it back.

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
  colour, what shows with no wallpaper client at all. scootbg draws over it
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
  `*-linux-gnu` links glibc, `libm` and `libgcc_s`.
- **`#![forbid(unsafe_code)]` in `scootbg`.** The only `unsafe` is
  isolated in one small crate, `scootbg-mem`:
  - the global allocator that returns large blocks to the kernel;
  - the `wl_shm` buffer mapping.

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
