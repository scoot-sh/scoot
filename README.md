# scoot

A scrolling-tiling Wayland compositor, in the shape of [niri](https://github.com/YaLTeR/niri):
lightweight, fast, GPU-optional, and built to be driven by a script or an agent
as easily as by a keyboard. [scoot.sh](https://scoot.sh) ·
[github.com/scoot-sh/scoot](https://github.com/scoot-sh/scoot)

Two things distinguish it from a typical compositor:

- **It runs with no GPU.** Rendering goes through [pixman](http://pixman.org/)
  on the CPU, so it works headless and works in a GPU-less container (the
  target is running inside [webtop](https://github.com/linuxserver/docker-webtop)).
  A GLES renderer is available opt-in (`--renderer gles`, `--headless`/
  `--nested` only) but pixman stays the default — see Which renderer draws
  the frames below for what that does and does not buy today.
- **It's IPC-first.** Every action a keybind would trigger — focus, move,
  resize, spawn, close — and every input a user could give — key presses,
  pointer movement, clicks — is also a request on a Unix socket, alongside
  screenshots and window/output introspection. The compositor itself is
  driven the same way in its own end-to-end test (see `scripts/smoke-test.sh`).
  The intent is that an agent doing simple computer-use tasks in a VM is a
  first-class client, not an afterthought bolted on later. Because that socket
  can inject any keystroke, it's treated as a privileged channel: it lives in
  `$XDG_RUNTIME_DIR` (override with `$SCOOT_SOCKET`), is created `0600`, and
  serves only connections from the same user as the compositor.

`scoot-core`, the layout/state engine, is kept platform-independent on
purpose: it knows nothing about Wayland. The plan is for the same engine to
eventually back a macOS Accessibility-API adapter, doing OmniWM-style window
layout on a Mac, not just on Linux.

**Renamed from `flexwm` on 2026-09-18**, a clean break with no fallback to the
old names: the binary is `scoot`, the crates are `scoot`/`scoot-core`/
`scoot-ipc`, the socket is `scoot.sock` (overridden by `$SCOOT_SOCKET`, not
`$FLEXWM_SOCKET`), and the config file is read from `~/.config/scoot/` (or
`$XDG_CONFIG_HOME/scoot/`) only — a file left at `~/.config/flexwm/config.toml`
is not loaded, so move it or pass `--config PATH`. The names clients see
followed too: the `wl_seat` name, `wl_output`'s `make` (and the `xdg_output`
description built from it) and the `--nested` window's own title/app-id are
all `scoot` now, so anything matching on those strings needs updating.

## Status

Early, but all three Wayland backends are real and working: `--headless`
(pixman rendering, xdg-shell, seat/input, full IPC control surface),
`--nested` (runs as a window inside an existing compositor, e.g. for webtop),
and `--tty` (a real DRM/KMS + libseat + libinput backend on actual hardware,
including VT switching; it picks its DRM device by trying every one on the
seat rather than trusting the first guess, and takes `--gpu PATH` when even
that picks wrong — see Which DRM device `--tty` drives below; it follows DRM
hotplug, so plugging a monitor in, pulling one out or resizing a VM's window
re-modesets instead of leaving the screen wrong; and it renders
a pointer cursor, a client's own image when it supplies one, a built-in shape
otherwise, whose size and color the config file can override). Also done:
vim-style keybindings, a TOML config file (`--config`, `[layout]`/
`[appearance]`/`[output]`/`[renderer]`/`[tty]`/`[binds]`, see Configuration
below), window decorations (a
niri-style focus ring, background color, server-side
`zxdg_decoration_manager_v1`),
`wlr-layer-shell-unstable-v1`, so bars, docks, wallpapers, launchers and
notification daemons work — including keyboard focus for the ones that ask
for it (see Layer-shell clients below), `ext-workspace-v1`, so those
 bars can also list, follow and switch workspaces (see Workspaces for bars
 below), `ext-foreign-toplevel-list-v1` **and**
 `wlr-foreign-toplevel-management-unstable-v1`, so a taskbar, dock or
 alt-tab switcher can list the windows themselves — and, through the wlr
 one, focus and close them (see Window lists for bars below),
 `wlr-output-management-unstable-v1`, so `wlr-randr` and a shell's
 Settings → Display page can read the screen's modes, position, scale and
 transform — read-only, every reconfiguration is refused (see Display
 information below),
 `ext-image-copy-capture-v1` with `ext-image-capture-source-v1`, so `grim`,
 a shell's workspace-overview preview or a screen-share can capture the
 screen over the standard protocol — output capture only, `wl_shm` only, and
 a capture taken while the session is locked sees the lock screen, never the
 windows behind it (see Screen capture for clients below),
 `ext-session-lock-v1`, so a real screen locker can lock the session
 with the compositor itself enforcing it (see Screen locking below),
 `ext-idle-notify-v1` and `idle-inhibit-unstable-v1`, so a `swayidle`-style
 daemon can idle the seat and lock it automatically while a video player
 holds it awake (see Idle detection below), the
 clipboard/primary-selection globals and `zwlr_gamma_control_manager_v1`, so
clipboard managers, middle-click paste and night-light tools work (see
Clipboard managers and Night light below),
`wp-cursor-shape-v1`, so a client can name the cursor it wants and get it
drawn from the machine's own installed xcursor theme -- or, where none is
installed, from ten shapes the compositor draws itself (see Cursor shapes
below), `xdg-activation-v1`, so a launcher can hand focus to
the app it started (see Focus handoff between clients below),
`xdg-toplevel-icon-v1`, so a bar or an agent can read a window's icon name
off `scoot msg windows` (see Window icons below), `text-input-v3` and
`input-method-v2`, so an IME or on-screen keyboard can compose into the
focused text field, popup and all (see Input methods below), and
output scaling (`[output] scale` over `wl_output.scale`,
`wp_fractional_scale_v1`, `wl_surface.preferred_buffer_scale` and
`wp_viewporter`, so a HiDPI panel gets
correctly-sized clients and text instead of everything rendered physically
tiny — see Output scaling below),
`wp_single_pixel_buffer_manager_v1`, so a toolkit can paint a solid fill
without allocating a shm pool (see Single-pixel buffers below),
`zwp_relative_pointer_manager_v1` with `zwp_pointer_constraints_v1`, so a
game or 3D app can lock or confine the pointer and read raw unaccelerated
deltas (see Relative pointer below), `zwp_tablet_manager_v2`, so a
drawing tablet's pen moves the cursor, taps click, and pressure reaches
tablet-aware clients (see Drawing tablets below),
`wp_presentation`, so a video player or animation client learns exactly when
each of its frames reached the screen (see Presentation-time feedback
below), `wp_alpha_modifier_v1`, so a client can ask for whole-surface
opacity and get it blended by the compositor instead of compositing it
itself, and `wp_content_type_manager_v1`, so a client can label what kind
of pixels a surface holds (see Rendering hints below), plus a
hardened control socket (owner-only
permissions, a same-user peer check, a 1 MiB cap on a single request,
screenshots rate-limited to one per connection per frame, at most 64
connections at once, a connection whose peer has stopped reading its
reply dropped after ten seconds, and a `wait-idle` capped at a minute
however long it asked for) whose connections
are non-blocking end to end, so no client — however slow, chunked or
unresponsive — can stall the compositor for anyone else. Sizes a client or a
config supplies are bounded too: each individual `wl_shm` pool is capped at
512 MiB (four full-screen 8K frames' worth — a request past it gets a
protocol error rather than a multi-gigabyte mapping for that pool), one
client may hold at most 128 live pool objects at once (past that the excess
`create_pool` gets the same protocol error — this bounds live pool objects
and the address-space envelope, *not* fds or mappings: a buffer outlives
its pool object, retaining both, so those are bounded by the live-buffer
count below), and one client may hold at most 512 live `wl_buffer`s at
once whatever created them (pool, dmabuf, single-pixel — past that the
excess creation gets a protocol error on the creating object; each
surviving shm or dmabuf buffer is what retains a compositor fd and, for
shm, its mapping (single-pixel buffers retain neither but are counted
uniformly — the hook can't observe buffer kind) — see
`docs/backlog/resolved/shm-pool-count-cap-done.md` and
`docs/backlog/resolved/shm-pool-cap-misses-retained-fds-done.md`). An
*imported dmabuf's* mapping is the one thing that count does not bound,
because it lives in the renderer's cache and outlives the `wl_buffer`
that carried it; it is released instead from the same buffer-destruction
hook, immediately and without waiting for a frame (see
`docs/backlog/resolved/dmabuf-advertised-but-never-imported-done.md`).
Per-connection bounds still multiply across connections, so a
compositor-wide ceiling sits above them: while fewer than 128 fds stand
free, newcomers shed (a Wayland connection gets an immediate EOF — there
is no protocol channel for a reason — an IPC one is refused with a
message naming the pressure and closed), and a client already holding
past 128 live buffers or 64 live pools is refused its next creation with
the same protocol error. A client under those graces — every client in
the measured-and-reasoned workload model, at dozens of times the measured
single-window floor (quickshell's exact concurrency is still unmeasured;
see the record) — is never
refused for another client's greed (see
`docs/backlog/resolved/wayland-global-fd-ceiling-done.md`), a client's
declared minimum window size can't exceed the largest
output's usable area on each axis, and `gap` and `cursor_size` each have an
upper bound as well as a lower one. One client may also hold at most 8
manager/list binds across the workspace, window-list and display globals
combined (see Workspaces for bars below) — past that the excess bind is
closed with `finished` (plus the `done` that batching requires, where the
protocol has one) and announced nothing, so a greedy client costs itself
its ninth subscription, never another client's. A missing `$XDG_RUNTIME_DIR` is a one-line
startup error, not a crash. Verified
end-to-end on every backend — a real
client maps, tiles, receives synthetic input, and a screenshot proves it. Not
yet started: a GPU rendering path and the macOS adapter.

## Layout

```
crates/
  scoot-core/   Platform-independent window state and scrolling-tile layout.
                 No Wayland, no I/O — pure data and functions, fuzz-tested.
  scoot-ipc/    The wire protocol (requests/responses) and a client over it.
                 Builds on any platform.
  scoot/        The CLI and, on Linux only, the Smithay-based compositor.
                 `scoot msg ...` builds everywhere, so it can drive a
                 compositor running in a VM from a Mac.
vm/              A NixOS VM + flake for developing the Linux-only compositor
                 from macOS. See vm/README.md.
scripts/         smoke-test.sh: an end-to-end test driven entirely over IPC.
```

## Building

Two paths, both from the flake at the repo root. To just get the binary
(clone, then):

```sh
nix build                            # ./result/bin/scoot
nix run . -- --headless -- foot      # build and run it in one step
nix run . -- msg windows             # the client, same binary
```

On Linux that builds the compositor. On macOS the compositor is compiled out
of the crate and the same package gives you `scoot msg`, the client — useful
for driving a compositor running in a VM, but `scoot --headless` there exits
with `the compositor only runs on Linux`. `nix build` deliberately does not
run the test suite (`flake.nix` says why); `cargo test` below is where that
runs. On macOS the flake covers Apple Silicon (`aarch64-darwin`) only: the
pinned nixpkgs dropped Intel (`x86_64-darwin`), so an Intel Mac builds the
same client from source with `cargo build` below instead.

To work on the code:

```sh
nix develop        # every dependency, on Linux or macOS
cargo build         # scoot-core, scoot-ipc, and the CLI build anywhere;
                     # the compositor itself only compiles on Linux
cargo test --workspace
cargo nextest run --workspace   # optional: one process per test, and faster
```

`cargo nextest` runs each test in its own process, which keeps the
compositor's real-Wayland-client test suites from sharing process-global
state. It is an addition, not a replacement — it does not run doctests, so
`cargo test` stays the baseline and needs no extra tool.

There is one optional Cargo feature, **`gpu-scanout`**, off by default:

```sh
cargo build -p scoot --features gpu-scanout
```

It compiles the `--tty` **GPU scanout** tier: `--renderer gles` under `--tty`
then composites straight into the buffer the screen scans out, with no
read-back (see Which renderer draws the frames below). Without it, `--tty`
has only the CPU/dumb-buffer path it has always had, and `--renderer gles`
there warns and uses pixman.

It is off by default because it is the one thing in the tree that adds a
**link-time** dependency on `libgbm`: the resulting binary carries
`libgbm.so.1` in its `DT_NEEDED` list and will not start on a machine without
it, while the default build has no such entry and runs anywhere. Running with
no GPU stack at all is a hard requirement here, so `cargo build` keeps
producing the binary that does, and packaging for real hardware is where you
turn this on. (`--renderer gles` is *not* in the same position and needs no
feature: libEGL/libGLESv2 are `dlopen`ed, so a GPU-less machine only fails
when that renderer is asked for, at startup, with a message.)

To actually run the compositor you need a real (or virtual) Linux machine with
a seat — see `vm/README.md` for a Mac-native NixOS VM that provides one.

## Running

```sh
scoot --headless --width 1280 --height 800 -- foot   # start, spawn a terminal
scoot --nested --width 1280 --height 800 -- foot     # inside your existing compositor
scoot --headless --renderer gles -- foot             # ...drawing with GLES instead of the CPU
scoot --tty -- foot                                  # on a real DRM/KMS seat
scoot --tty --renderer gles -- foot                  # ...scanning out from the GPU (needs --features gpu-scanout)
scoot --tty --gpu /dev/dri/card0 -- foot             # ...naming the DRM device yourself
scoot --tty --mode 1920x1080 -- foot                 # ...naming the display mode (see below)
scoot msg windows                                     # in another shell
scoot msg action focus-column left
scoot msg screenshot --out /tmp/shot.png
scoot msg type "hello"
scoot msg wait-idle --quiet-ms 200
```

Add `--config PATH` to any of the three to load a TOML config; see
Configuration below for the full schema and default keybindings. Run
`scoot --help` for the full request/action list.

`--width`/`--height` (the `--headless`/`--nested` output size) accept
1–65,535 per axis — whatever DRM itself can report for a mode
(`drm_mode_modeinfo` stores each axis in a `u16`), with room to spare past
any real display. Anything else is a startup error naming the flag and the
expected range (`invalid --width: '70000' (expected 1-65535)`), not a
silently different size.

### Which renderer draws the frames

`--renderer pixman|gles` (config: `[renderer] backend`, see below) picks what
composites each frame. **The default is `pixman`, the CPU renderer, and that
is not changing** — running with no GPU at all is a hard requirement here, not
a fallback tier. `gles` is opt-in, and worth being precise about what it is
and is not today:

- **What it buys you.** Correctness parity with pixman on a second renderer,
  and the groundwork for scanning a GPU buffer out directly under `--tty`.
  Every pixel-readback test in the suite — session lock, layer shell, alpha
  modifier, single-pixel buffers, the cursor, output scaling — passes
  byte-identically under either renderer.
- **On `--headless` and `--nested`, it does not buy you speed.** There the
  frame is composited into an offscreen buffer and then read back to main
  memory exactly as pixman's is, so `gles` adds a GPU round trip without
  removing any CPU copy; on a machine whose "GPU" is a software rasteriser
  (llvmpipe, which is what a VM or a GPU-less container has) it is several
  times *slower* than pixman.
- **On `--tty` it is a different thing entirely: GPU scanout.** In a build
  carrying the `gpu-scanout` Cargo feature (see Building above),
  `--tty --renderer gles` composites straight into the buffer the screen
  scans out — Smithay's `DrmCompositor` over a GBM swapchain — so there is no
  read-back and no memcpy into a dumb buffer at all. Which tier a session came
  up on is in the startup log, on the line that names the device:
  `drm: driving this device path=/dev/dri/card0 connector=eDP-1 ...
  scanout="gpu"` (or `scanout="dumb"`). Scope today: the primary plane only —
  no cursor or overlay planes, and no direct scan-out of a client's own
  buffer.
- **In a build *without* that feature, `--tty --renderer gles` warns and uses
  pixman**, naming the missing feature, because the only GLES pipeline such a
  build has reads the GPU frame back only to memcpy it into a dumb buffer,
  which is strictly worse than compositing on the CPU in the first place.
- **On `--tty`, a `gles` that cannot be built is a warning, not a refusal to
  start.** If the device has no usable GBM node, if a GLES renderer cannot be
  built on it, or if no scan-out format works for both the CRTC's primary
  plane and the renderer, scoot says so and runs the session on pixman with
  dumb buffers. That is the opposite of the `--headless`/`--nested` rule
  below, and deliberately so: on `--tty` scoot *is* your session, and
  refusing to start would leave you with no desktop at all.
- **Hardware first.** The EGL device is chosen by preferring a real device
  over a software one and taking the first that yields a working renderer,
  so a box with a GPU uses the GPU. Note that "software" here means only
  that the device advertises `EGL_MESA_device_software`: a real device *node*
  backed by a software driver answers no, so it is preferred and then served
  in software anyway. That is what happens on the dev VM, and it is why
  numbers measured there are llvmpipe's. The chosen device is logged at
  startup (`the GLES renderer is up device=/dev/dri/renderD128
  software=false`) — trust that line over the flag name.
- **On `--headless`/`--nested`, a wrong `--renderer gles` is a startup error,
  not a silent downgrade.** If no EGL device can drive it, scoot says so and
  names each failure rather than quietly compositing with the other renderer.
  Nothing is at stake there, so failing loudly is free.
- **Known gap: dma-buf clients.** The buffer formats scoot advertises to
  clients are still the CPU renderer's, whichever renderer is active, so a
  client handing over a GPU buffer the active GLES renderer cannot import has
  it refused (and, through `create_immed`, is disconnected for it). On
  `--headless`/`--nested` with `gles`, stay on `pixman` if you use dma-buf
  clients. The `--tty` scanout tier does not have that exposure: it refuses to
  come up at all on a device whose GLES renderer cannot import what scoot
  advertises, and falls back to pixman, so the tier can never be the reason a
  dma-buf client is disconnected.

`scoot msg type TEXT` types text the way a person would, on whatever
keyboard layout the session is running: for each character it finds the key
that carries it and holds down whatever modifiers that key's level needs —
Shift for `A` or `!`, AltGr for a German layout's `@` — so a client receives
the same key *and* modifier events it would see from a real keyboard, not
just a bare keysym. `\n` and `\t` are sent as `Return` and `Tab`. A
character no single keypress produces goes through a second path instead of
failing: the two-key dead-led sequence from the session-locale compose table
(`dead_acute` then `e` for `é` on a German layout), pressed as the two
keypresses a person would type, each with its own level's modifiers. That
covers accented Latin wherever the active layout carries the dead key, and
it closes the dead-key ASCII gaps below entirely — all 95 printable ASCII
characters now type on every one of the fourteen swept Latin layouts (`us`,
`us(intl)`, `gb`, `de`, `de(neo)`, `fr`, `fr(oss)`, `es`, `it`, `pt`, `se`,
`no`, `dk`, `pl`). Three things worth knowing:

- A character the active layout can't produce is an error naming it (``no
  key for `é` in this layout``). What counts as "can't produce" is now the
  compose table as well as the keymap: a character with no two-key dead-led
  sequence there stays refused — plain `us` carries no dead keys and no
  Compose key, so `é` is still "no key" on it, and three-key `Multi_key`
  sequences are not driven even on a layout that has a Compose key. A
  character that lives on an *inactive* layout group is refused too: nothing
  here switches the session's layout to go and find it. A character that
  sits on a level the layout only reaches through a *locking or latching*
  modifier gets its own, different
  message (``[character] needs a modifier this layout only locks or
  latches``) — scoot will not press Caps Lock to type a capital, since
  that would leave it on for everything afterwards. The sequences come from
  the session locale (`LC_ALL`, then `LC_CTYPE`, then `LANG`, as the
  compositor process sees them — the same source toolkits read), so a
  missing locale entry falls back the way client toolkits do. In every case the
  characters *before* it in the string have already been typed: the request
  stops at the first character it can't type rather than rolling back.
- Keybindings still apply to what it types, exactly as they would to a real
  keypress. That only matters for a bind with no modifiers, or one on
  Shift plus a key; if a character does hit a bind, the compositor logs a
  warning naming it rather than swallowing it silently.
- `scoot msg key COMBO` is the other one, and it is *not* the same: it
  presses exactly the combination named and holds exactly the modifiers
  named, nothing more. So name the key as it is with nothing held, plus the
  modifiers: `shift+1`, not `exclam`; `shift+a`, not `A`. A name that this
  layout only carries above its unmodified level is refused, because the
  key that carries it types a *different* character when pressed bare —
  `scoot msg key exclam` would press the `1` key and deliver `1`. Some
  characters can't be named as a combination at all (`@` on a German layout
  needs AltGr, which `key` has no name for); `scoot msg type` is the one
  that works the modifiers out from the layout, and the one to reach for
  when the goal is text rather than a chord. The modifiers themselves are
  resolved from the active layout the same way — whichever key actually
  holds Shift/Control/Alt/Super is what gets held (either hand's key, or
  the one a layout option like `grp:lshift_toggle` left in place) — so a
  combo is refused for its modifier only when no key on the layout can
  hold it.

### What the control socket refuses

Nine bounds an agent driving scoot over IPC can actually hit. The first
seven are refusals with a reason — an ordinary `error` response, which
`scoot msg` prints and exits non-zero on — rather than a silent drop or a
delay. The last two can't be: one drops a peer that is by definition not
reading its socket, and the other shortens a wait rather than refusing it.

- **One request line may be at most 1 MiB.** Past that the connection is
  told so and closed; there is no resynchronizing mid-line.
- **`type` text is limited to 16,384 characters per request.** Each
  character becomes key events typed synchronously on the thread that
  serves every other client, so a megabyte of text would stall the whole
  compositor for seconds; past the cap the request is refused with a
  message naming the limit instead. Split the text across several `type`
  requests. A few hundred characters -- a shell command line -- costs
  under a millisecond and never notices this. Counted in characters, not
  bytes; even the longest encodings land far under the 1 MiB line limit
  above, so this cap always fires first.
- **One screenshot per connection per 16ms frame.** A capture costs a render
  and framebuffer read-back on the thread that serves every other client, so
  a second one inside the same frame is refused rather than queued. Retry
  after a frame. Note the reply order for that pair: the refusal is answered
  immediately, while the capture it follows is still encoding (see below) --
  so the *second* request's reply arrives *first*. A client pipelining
  screenshots matches replies by content, not by position.
- **One capture in flight per connection.** The PNG encode runs on a worker
  thread, so other connections are answered while it runs -- but the
  capture's own reply still has to go out before any later reply on that
  same connection, or a client reading replies in request order sees them
  swap. Any other request arriving on a connection with a capture in flight
  is therefore refused with a retry rather than answered out of order.
  `scoot msg` sends one request per connection and never meets this; nor
  does a capture refuse one on *another* connection. A capture whose earlier
  replies are still going out is refused the same way, for the same framing
  reason -- retry once the queue has drained.
- **Four captures in flight at once, across every client.** Each one holds a
  full frame of raw pixels on its way through the worker, so past four the
  next capture is refused with a retry rather than queued without bound.
  Measured at 1600x1000 in a release build: the event-loop stall per capture
  is down to the render and read-back (~2ms -- the encode's ~10ms share of
  the ticket's ~12ms figure no longer blocks anyone), and a `version` round
  trip during a capture flood answers in ~8ms instead of ~11ms. Under N concurrent
  capturers the render share is still paid per capture, so a bystander waits
  longer than that; what no longer happens is N encodes stacking end to end
  on the loop.
- **At most 64 connections at once**, across every client. A 65th is
  refused with a message naming the limit and closed immediately, not
  queued behind the others. This is what keeps the per-connection bounds
  above meaningful — otherwise reconnecting resets them — so an agent that
  wants many requests should pipeline them on *one* connection rather than
  open a connection per request. `scoot msg` opens one per invocation and
   closes it as soon as it has its answer, so ordinary scripted use never
   approaches this.
- **A newcomer under file-descriptor pressure is refused, too.** While
   fewer than 128 fds stand free process-wide, a new IPC connection is
   refused with a message naming the pressure and closed immediately — no
   slot taken, living connections untouched. Retry in a moment: pressure
   lifts as soon as whoever is holding fds lets go, and ordinary use (an
   idle compositor holds 14 fds, a `foot` window 17) never comes near it.
   The Wayland half of the same ceiling gets an EOF instead — there is no
   protocol channel for a reason there — and creations past a per-client
   grace (128 live buffers, 64 live pools) are refused with a protocol
   error while pressure holds; see the paragraph above.
- **A connection whose peer stops reading is dropped**, ten to twenty
  seconds after the last byte it took (the check runs on a deadline of its
  own, so the exact moment falls in that range rather than on the ten
  exactly). Nothing is sent when this happens — there is nobody reading to
  send it to; the connection simply closes. Replies that don't fit in the
  socket are queued and pushed out as the client reads; a client that takes
  no bytes at all for that long — the classic case being one that sends a
  request, does `shutdown(SHUT_WR)` and then never reads the answer — is
  treated as gone, and its connection and fds are released. Reading *slowly*
  is fine and is never given up on: the clock is measured from the last byte
  that actually went out, not from when the reply was queued, so draining a
  multi-megabyte screenshot over a minute costs nothing.
- **`wait-idle` waits at most 60 seconds**, whatever `--timeout-ms` asks
  for. A longer request isn't refused, it's shortened: the answer comes back
  as usual, at the minute mark at the latest. A waiting `wait-idle` keeps
  its connection (and one of the 64 slots above) for as long as it waits,
  and — uniquely on this socket — cannot notice its client dying while it
  waits, so an unbounded wait from a client that then exits would hold that
  slot for the rest of the session. The default is 5 seconds and the request
  is meant for hundreds of milliseconds, so this is well out of the way of
  any real use. A capture in flight neither extends nor shortens a wait: it
  touches no commit clock, and parks no waiter.

One thing this list does not bound, because it is not a refusal: while a
client holds an active pointer lock (`zwp_pointer_constraints_v1`, see
Relative pointer below), `pointer move` and `click` still answer `ok` but
move nothing -- the lock owns the pointer until its client releases it. An
agent driving the pointer during a game or 3D session sees success replies
with a frozen cursor; clicks still reach whatever surface holds pointer
focus. A session lock ends that freeze: locking deactivates the held
constraint, so injected motion and clicks reach the lock surface exactly
like a real mouse, and unlocking re-arms the game's lock. An agent must
not assume a lock it observed survives a session lock -- after one, the
game holds its pointer again and the stream has resumed.

`--tty` needs a seat (`seatd` or logind) with a DRM device on it. On a modern
kernel, on every non-root `--tty` run, Smithay logs `Unable to become drm
master, assuming unprivileged mode` at startup — expected, not a failure: the
session manager opens the device and (normally) already holds DRM master on
scoot's behalf, and scoot simply isn't permitted to call `SET_MASTER`
itself on a file another process opened. `vm/README.md`'s troubleshooting
section has the kernel-level reason and two commands that check whether
master really is held, rather than assuming the log line alone settles it.

### Which DRM device `--tty` drives

Normally: whichever one works. scoot asks Smithay for the seat's primary
GPU, and if that device turns out not to be able to drive a display, it
tries every other DRM device on the seat in turn before giving up. On an
ordinary PC the first pick is right and nothing else is ever opened; the
log line worth grepping for either way is `drm: driving this device`, with
the device path on it (`RUST_LOG=info`, the default level, is enough — a
rejected device gets a `drm: device unusable` warning naming it and saying
what it said).

The fallback exists because the usual heuristic — "the GPU whose PCI parent
has `boot_vga=1`, else the first one with a render node" — assumes the 3D
GPU and the display controller are the same DRM device. On Apple Silicon
under Asahi Linux they are not: `asahi`/AGX has the render node, `apple,dcp`
owns the CRTCs and connectors, and there is no PCI GPU or VGA BIOS for the
first rule to match. Picking the render-only device there fails with
`Operation not supported (os error 95)` loading its KMS resources.

**Confirmed working on Apple Silicon (2026-09-18).** On the machine this was
reported from (Apple M2, `apple,t8112`), the automatic search rejects the
`asahi` render node and drives the `apple-drm` display controller unattended,
as a daily-driven `--tty` session with no `--gpu` and no `[tty] gpu`:

```
drm: device unusable path=/dev/dri/card1 reason=has no usable KMS pipeline
     -- loading its DRM resources failed (Operation not supported (os error 95))
drm: driving this device path=/dev/dri/card2 connector=eDP-1 width=2560 height=1600
```

So **`--gpu` is not needed there** — don't reach for it first on Apple
Silicon. If you do name a device, it is not a guarantee: naming one skips the
*search*, not the checks, so it still has to open through the session and pass
the same KMS probe every automatic candidate does. [`Asahi.md`](Asahi.md)
records that run and remains the runbook for re-checking it on a different
Apple Silicon model.

If the automatic search still picks wrong, name the device — the one that
owns the connectors, never a render-only node:

```sh
scoot --tty --gpu /dev/dri/card0 -- foot
```

`--gpu PATH` replaces the search entirely — exactly that device, no
fallback — so a wrong path is a clean startup error naming the device and
what failed, not a silent fall back to something else. It only means
anything under `--tty`; on `--headless` or `--nested` it is ignored with a
warning. One thing it can cost: hotplug (below) is followed only for
devices udev lists as GPUs on this seat, and `--gpu` can name one that
isn't in that list. scoot says so at startup — `the chosen device is not
in udev's list for this seat, so display changes on it will not be
noticed` — and the session otherwise runs exactly as it always did, with
the mode it started on.

On hardware where the automatic search picks wrong every time, the config
file saves retyping the flag on every launch:

```toml
[tty]
gpu = "/dev/dri/card0"
```

`[tty] gpu` names the same device the same way — exactly that device, no
fallback, and the same clean startup error (naming the key, not the flag)
when it cannot be driven. `--gpu` wins when both name one: an explicit
flag beats a file, the way `--config` beats the default path. An
explicitly-set-but-empty path in either (`--gpu ""`, `gpu = ""`) is a
startup error naming the surface that set it, not something the session is
asked to open — on every backend, including `--headless`/`--nested`
(a malformed key is refused before the backend branch, so a shared config
file with an empty value fails fast everywhere rather than silently
differing by backend). A set non-empty value, like `--gpu`, is ignored
with a warning outside `--tty`. See the `[tty]` reference under
Configuration.

The output's size is the connector's preferred mode. When that is the wrong
size — under Apple's Virtualization framework (vfkit, UTM) the "preferred"
mode is just the host window's size in backing pixels, so it doubles or
halves with whichever screen the window opened on — name the mode:

```sh
scoot --tty --mode 1920x1080 -- foot
```

`--mode WxH` picks the connector mode of exactly that size, and falls back to
the preferred one with a warning if the connector lists no such mode (`cat
/sys/class/drm/card*-*/modes` shows what it lists). Like `--gpu`, it is
ignored with a warning outside `--tty`.

### `--tty` follows the display: hotplug and host resizes

`--tty` watches udev for DRM changes and re-runs the choice above whenever
the display underneath it moves, so nothing here is a once-at-startup
decision any more:

- **Plug a monitor in or pull one out.** Unplugging the connector scoot is
  driving makes it pick another connected one and mode-set onto it — moving
  to a different CRTC when the display controller only routes that connector
  there (ordinary PC graphics route any connector to any CRTC; ARM SoCs often
  wire encoders to specific CRTCs). If no CRTC on the device can drive the
  new connector it logs `no other crtc on this device can drive the new
  connector` and stays put, retrying on the next hotplug. Plugging one back
  in after everything was unplugged mode-sets back onto it.
- **Resize, rescale or full-screen a VM window.** Apple's Virtualization
  framework reconfigures the guest display when you do, which reaches the
  guest as a hotplug with a new mode list and a new preferred mode; scoot
  follows it. `--mode WxH` is still honoured on every re-probe, not just the
  first: if the size you named is in the new list it wins, and if it is not
  you get the same warning and the new preferred mode.

A change reaches everything that cares: the render target, `wl_output`
(`mode` + `done`), `wlr-output-management`, layer-shell surfaces (bars
re-anchor and re-arrange), the window layout, and `scoot msg outputs`,
which reports the new rectangle on the next query with no extra plumbing.

Two limits worth knowing, both deliberate:

- **Still one output.** Plugging a second monitor into a laptop already
  running on `eDP-1` keeps the session on `eDP-1` rather than jumping to the
  new screen — scoot drives one output, so one of the two has to be dark,
  and the one you are looking at is the one it keeps. Real multi-output is a
  separate, larger piece of work.
- **The output keeps the name it started with.** A session that started on
  `HDMI-A-1` and fell back to `eDP-1` when the cable came out still reports
  `HDMI-A-1`. Renaming a `wl_output` is not something the protocol allows;
  recreating it would make every client re-enter the output and re-map its
  surfaces, which is a bigger lie about what happened than a stale name.

With nothing connected at all, scoot holds the last frame, keeps the
session running and logs `nothing is connected to this device any more` —
plug a display back in and it mode-sets onto it. Hotplug events that arrive
while scoot is on another VT are picked up on the switch back, since a
session without DRM master cannot mode-set when they happen.

Under `--tty` the output is named after its connector — `HDMI-A-1`, `eDP-1`,
`Virtual-1`, the same spelling as `/sys/class/drm/card*-*` — so bars and
shells label the screen as they would under any other compositor;
`scoot msg outputs` shows the same name. `--headless` and `--nested` have no
connector and keep the name `headless`.

To see what the seat has, and which driver is behind each device:

```sh
ls /dev/dri/card*
for c in /sys/class/drm/card*/device/driver; do echo "$c -> $(readlink -f "$c")"; done
```

When nothing works, the startup error lists every device that was tried and
why each one was rejected, rather than naming only the first. If the *session*
refused all of them — which is what happens when another compositor already
holds the seat, since a seat takes one client at a time — the error says so
instead of suggesting `--gpu`: no choice of device gets around a busy seat.
And if enumerating the seat's other devices fails outright, the primary pick
is still tried on its own (with a `could not list the seat's other devices`
warning), rather than losing a working device to a failure in the fallback
machinery.

## Layer-shell clients (bars, wallpapers, launchers)

scoot implements `wlr-layer-shell-unstable-v1` (version 5), the protocol
every panel, dock, wallpaper setter and notification daemon in the
wlroots-adjacent ecosystem uses — `waybar`, `swaybg`, `mako`, `wofi`,
`fuzzel`, `yambar` and friends. Start one the same way you start anything
else inside the session:

```sh
scoot --tty -- foot               # ...then, in a shell inside the session:
swaybg -c '#123456' &             # a wallpaper, on the background layer
waybar &                          # a bar, on the top layer
fuzzel                            # a launcher, on the overlay layer
```

What works:

- **All four layers.** `background` and `bottom` draw behind windows (and
  behind the focus ring, so a wallpaper never hides it); `top` and `overlay`
  draw in front of them.
- **Anchors, margins and sizing**, including the protocol's rules for a
  surface anchored to opposite edges or given a zero dimension.
- **Exclusive zones.** A bar that reserves its own height shrinks the area
  windows are tiled within, so nothing is ever laid out underneath it — and
  gives that space back the moment it exits or its client dies. Several
  surfaces reserving on the same edge stack. `-1` ("don't push me around")
  reserves nothing, which is what a full-screen wallpaper wants.
- **Pointer input.** A click, scroll or motion over a layer surface goes to
  that surface, not to whatever window is behind it, and clicking a bar does
  not move window focus.
- **Keyboard focus**, following the protocol's `keyboard_interactivity`:
  - `none` (the default, and what a bar, wallpaper or notification daemon
    asks for) never takes the keyboard. Nothing changes for those clients.
  - `exclusive` on the `top` or `overlay` layer takes the keyboard as soon as
    the surface maps and holds it until it unmaps — what a launcher
    (`fuzzel`, `wofi`, `rofi`) and a layer-shell lock screen need. If several
    ask at once, the front-most one wins, and the keyboard falls back to the
    next one down when it goes away.
  - `on_demand` is click-to-focus, exactly like a window: click the surface
    to give it the keyboard, click a window, a bar or bare desktop to take it
    back. `exclusive` on the `bottom` or `background` layer is treated the
    same way — the spec allows normal focus semantics there, and nothing
    should be typing into a wallpaper unasked. Note that no *input* other
    than a click releases an `on_demand` surface: a keybinding or a `scoot
    msg action focus-*` moves window focus but leaves the keyboard where it
    is. The spec leaves this implementation-defined, and click-only is the
    deliberate choice — the alternative silently steals a launcher's keyboard
    whenever a script re-focuses a window.
  - A surface that has committed but never attached a buffer, or that
    unmapped itself, can't hold the keyboard however it asks: there is
    nothing of it on screen to type into.
  - A click-focused surface can also hand the keyboard back itself, by
    committing `keyboard_interactivity: none` or by unmapping — and the click
    that focused it is spent when it does. Asking for `on_demand` again, or
    mapping again, gets it drawn again but not focused again: it waits for a
    fresh click, the same as the first time. So a bar that collapses and
    re-opens a search field, or a notification daemon that closes and
    re-opens an inline reply, never gets the keyboard back without the user
    actually clicking it. (An `exclusive` surface on `top`/`overlay` is the
    exception, per the rule above: it takes the keyboard whenever it is
    mapped, clicked or not.)

  **Keybindings always win.** They are matched before anything is forwarded
  to the focused client, so `Super+Shift+E` (quit) and, on `--tty`,
  `Ctrl+Alt+F1`..`F12` (VT switch) still work while a full-screen layer
  surface is holding every keystroke. That is the escape hatch if one wedges.

### Popup menus (`xdg_popup`)

An application menu, a combo box, a bar's own dropdown: all of them are
`xdg_popup` surfaces, and all of them map, draw, take clicks and — since
popup grabs landed — take the keyboard.

- **A grab routes input into the menu.** When a client opens a menu it asks
  for an explicit grab (`xdg_popup.grab`). While one is held, keyboard focus
  is on the popup, so Escape-to-close, arrow-key navigation and typeahead
  reach it; clicking inside it keeps it up; clicking anywhere outside
  dismisses it. Dismissal is also the only thing that ever sends
  `popup_done`, so without a grab nothing closes a menu.
- **Submenus nest.** A grab on a popup whose parent is the current grab
  takes over, and closing it unwinds to the parent menu rather than closing
  the whole chain.
- **A bar's own dropdowns work too** — a popup parented to a *layer* surface
  (`zwlr_layer_surface_v1.get_popup`), not just to a window.
- **A grab loses to the compositor's own focus rules, rather than fighting
  them**, in this order:
  1. **The session lock wins.** Locking dismisses an open menu, and a grab
     requested while locked is refused. Nothing but a lock surface receives a
     keystroke while the session is locked — a menu is not a way around that.
  2. **An `exclusive` layer surface on `top`/`overlay` wins** — a launcher
     opened over a menu is typeable, and the menu is dismissed rather than
     left on screen holding input it can no longer use.
  3. **An input method holding the keyboard wins** — while an IME (fcitx5,
     or any `zwp_input_method_v2` client holding its keyboard grab, which a
     real IME does for its whole active span, not just while composing) has
     the seat, a grab asked for is refused, and a grab already held is
     dismissed when the IME takes the keyboard. Letting a menu pre-empt an
     IME mid-compose would silently interrupt composition in an unrelated
     text field; leaving the menu mapped with no keyboard would leave one
     Escape cannot close. Either order ends the same way: no menu survives
     while the IME holds the seat.
  4. **The menu wins over everything else**: over the focused window, over
     a layer surface that got the keyboard from a click, and over the
     `exclusive` surface the menu itself hangs off — so a bar's own
     dropdown is not dismissed by the bar that opened it, whichever
     keyboard interactivity the bar asked for. A *different* `exclusive`
     surface still wins per rule 2.

  Losing is always spelled `popup_done` (the protocol lets a compositor
  dismiss a popup at any time), never "leave it up but take its input away".
- **Keybindings still win over a grab**, exactly as they do over an
  `exclusive` layer surface and for the same reason: they are matched before
  anything is forwarded. A client cannot wedge the session by holding a menu
  open.
- **Keyboard focus does not move onto a popup that did not grab.** A tooltip
  is an ordinary `xdg_popup` too, and handing one the keyboard would take it
  away from the window you are typing into. Menus and combo boxes that want
  keys take a grab; that is what the request is for.
- **The grab's serial has to name a real interaction.** Like
  `xdg-activation-v1`'s token (see Focus handoff below), a grab is refused --
  dismissed, with a warning in the compositor log naming the client and
  serial, since the protocol posts no error -- unless its serial is a recent
  key, button or focus `enter` actually delivered to the client asking, so a
  client that was never focused cannot take the keyboard on its own say-so.
  A grab that continues the client's own open menu (a nested submenu, or a
  menu replacing the one just closed) is not refused for reusing its opening
   serial past that window. The other bounds still apply on top (locking
   dismisses the grab, as does a *different* exclusive layer surface — never
   the one the menu hangs off — as does an IME taking the keyboard, any
   click outside dismisses it, and keybindings still fire).

What doesn't, yet:

- **A layer-shell "lock screen" is still not a security boundary** — use a
  real `ext-session-lock-v1` locker instead (see Screen locking below).
  A layer surface with `exclusive` keyboard interactivity will now actually
  receive what you type instead of leaking it to the window behind, but the
  escape hatch that makes exclusive focus safe is also a way around such a
  lock: the quit binding and the `--tty` VT switches keep working while it is
  up, and everything behind it is still drawn and still capturable. Treat one
  as a screen *blanker* you can type a password into, not as something that
  keeps anyone out.
- **`scoot msg outputs`** reports each output's *full* rectangle (in logical
  pixels, along with the output's `scale`) plus its *usable* rectangle — the
  full output minus whatever a bar reserved at its edges, which is where
  windows actually go. An agent asking "how big is the screen" wants `rect`;
  asking "where can a window be" wants `usable`. (An all-zero `usable` means
  the server predates the field — fall back to `rect`, which is what every
  client did before it existed.) Screenshots are captured at
  physical resolution, so multiply a logical rectangle by `scale` to convert
  it to screenshot pixels. Every success reply (`ok`) also carries the
  session-lock state it was built under (`locked`), so an agent typing a
  password over IPC learns the unlock landed from the very next reply —
  `true` is always truthful; `false` means unlocked or a server predating
  the field.

Two things for agents to know about layer surfaces:

**Window focus and keyboard focus are separate now.** `scoot msg windows`'
`focused` flag, the focus ring and `scoot msg action focus-*` all still mean
the *window*, and a layer surface holding the keyboard never appears there —
what it reports is where focus returns to once that surface goes away. So if
a launcher is up, `scoot msg type "..."` and `scoot msg key` go to the
launcher (which is usually what you want), while `scoot msg windows` still
names the window behind it. There is no IPC request that reports a layer
surface yet.

A popup menu holding the keyboard is the subtler case of the same split. An
explicit `xdg_popup.grab` routes every keystroke to the menu until it is
dismissed, and compositor focus still moves underneath it — focus
keybindings keep firing with a menu open, by design. So after such a
keybinding, `scoot msg windows` can report window B as `"focused": true`
while every key still reaches window A's menu, possibly off-screen, with no
error. Each window therefore also reports `"popup_grab"` while its own
popup tree holds the keyboard:

```json
{ "id": 1, ..., "focused": false, "popup_grab": true }
```

The agent rule: if any window reports `"popup_grab": true`, keystrokes go
to that window's menu, not to the focused window — wait for the menu to
close (Escape or a click dismisses it) before typing at anything else.
`false` means no grab, or a server predating the field (like `icon`, it is
a defaulted additive field and does not bump `PROTOCOL_VERSION`). Two
limits, both deliberate: a grab rooted at a layer surface — a bar's own
dropdown — belongs to no window and leaves every window `false`; and while
the session is locked there is never a grab to report, because locking
dismisses any open one and refuses new ones.

The other: a bar redraws on its own schedule, and
`scoot msg wait-idle` waits for *nothing on screen* to have redrawn.
Measured on real hardware with a `waybar` clock ticking once a second,
`--quiet-ms 200` still settles normally (204 ms) while `--quiet-ms 1500`
never does and times out. Keep `--quiet-ms` below whatever your bar's own
redraw interval is — the same caveat an animated cursor already carried.
(`--timeout-ms` is capped at 60 seconds, for the reason under What the
control socket refuses above.)

## Workspaces for bars (`ext-workspace-v1`)

scoot implements `ext-workspace-v1` (version 1), the compositor-agnostic
successor to the one-off wlr workspace protocols, so a panel's workspace
module, a workspace switcher or an indicator can list workspaces, follow which
one is active, and switch between them. Together with layer shell above,
that's everything a bar needs. The global is `ext_workspace_manager_v1`,
available to every client, no privilege or allow-list.

What a client sees:

- **One workspace group**, carrying scoot's single output. A client that
  binds `wl_output` after the manager still gets an `output_enter` for it, so
  registry order doesn't matter.
- **One `ext_workspace_handle_v1` per workspace**, named `"1"`, `"2"`, … in
  layout order, with matching one-dimensional `coordinates` — the workspace
  named `"1"` reports `[1]`, `"2"` reports `[2]`, and so on (sort by those,
  not by name — `"10"` sorts before `"2"` as a string). The active one carries
  the `active` state bit; nothing else is ever set.
- **`activate` is the only capability advertised**, on workspaces. The group
  advertises none.
- **Changes arrive in batches closed by `done`**, one per change, and none at
  all when the workspaces didn't change. Draw on `done`, not on each event: a
  switch is one `state` event turning the old workspace off and another
  turning the new one on, and the list is momentarily inconsistent between
  them.

To switch a workspace, send `activate` on its handle **and then `commit` on
the manager** — the protocol batches requests, so an `activate` with no
`commit` after it does nothing, deliberately.

Worth knowing before you write against it:

- **Workspaces are positions, not identities, and scoot sends no `id`
  event.** The list grows as you use the trailing empty workspace and shrinks
  when a workspace empties out, which renumbers everything after it: handle 2
  means "the third workspace", not "that workspace". Redraw from what the last
  `done` said rather than remembering a handle as a particular user's
  workspace.
- **`deactivate`, `remove`, `assign` and `create_workspace` are ignored**, and
  no capability is advertised for them, because none of them exists in
  scoot's layout model: an output always has exactly one active workspace,
  workspaces are created and dropped by the layout itself rather than by the
  user, and there is only one group.
- **`activate` is a request, not a guarantee** (as the protocol says): one
  naming a workspace that vanished between the client reading the list and the
  `commit` arriving is dropped, and one for the workspace that is already
  active does nothing.
- **Switching to a workspace *by number* works from both sides now.**
  `scoot msg action focus-workspace-index N` (0-based, out of range does
  nothing) drives the same core action `ext-workspace-v1`'s `activate`
  already used, so an agent jumps straight to a workspace instead of
  stepping one at a time.
- **Multiple outputs will change the shape of this** — a group per output is
  what the protocol is built for — but scoot has exactly one output today, so
  there is exactly one group.
- **One connection may hold at most 8 binds across the workspace manager,
  both window-list globals below and the display manager**, counted per
  client: a ninth bind is answered `done` and then `finished` rather than
  announced, and a client that over-binds denies nothing to any other client.
  A bar binds this global once — stock quickshell binds one window-list
  manager per connection and nothing else here — so legitimate use sits at a
  quarter of the budget or less; see
  `docs/backlog/resolved/ext-workspace-object-binding-cap-done.md` for the
  sizing and the refusal form of each global.

## Window lists for bars (two protocols)

scoot publishes its window list through **two** protocols at once, both
available to every client with no privilege or allow-list:

- **`ext-foreign-toplevel-list-v1`** (version 1, global
  `ext_foreign_toplevel_list_v1`) — the compositor-agnostic successor, and
  what a standards-following taskbar, dock or alt-tab switcher should
  prefer. Enumeration only; the protocol has no requests.
- **`wlr-foreign-toplevel-management-unstable-v1`** (version 3, global
  `zwlr_foreign_toplevel_manager_v1`) — the older protocol, which is what
  every Quickshell-based shell (DMS, Noctalia) actually binds today. It
  enumerates *and* controls: `activate` and `close` are answered.

Both are the protocol-side twin of `scoot msg windows`: the same windows,
the same lifetime, live rather than polled — and, because they are driven
from the same three window-lifecycle events inside the compositor, a client
bound to both sees one window list described twice rather than two lists
that can drift.

### `ext-foreign-toplevel-list-v1`

What a client sees:

- **One `ext_foreign_toplevel_handle_v1` per window**, carrying `identifier`,
  `title` and `app_id`. Binding the global announces every window that
  already exists, so registry order doesn't matter.
- **Changes arrive in batches closed by `done`** — draw on `done`, not on
  each event. In particular, a window is announced the moment its
  `xdg_toplevel` exists, which is *before* the toolkit has sent its app id
  and title: the first batch is usually two empty strings, followed a
  moment later by a batch per field as they arrive.
- **`closed` when the window goes**, after which nothing else is ever sent on
  that handle.
- **`stop` is answered with `finished`**, and means "no more *new* windows":
  handles the client already has keep reporting title and app id changes
  until it destroys them, which is what the protocol's own teardown sequence
  (stop, wait for `finished`, then destroy the handles) requires.
- **Binding the list counts against the same 8-bind per-client budget as the
  workspace manager above** — a ninth bind of any of the four globals is
  answered `finished` (after which no `toplevel` events arrive; destroy the
  object) rather than announced.

**The identifier is `<generation>-<window id>`** — e.g. `a3e689a2-1`: eight
hex digits of per-session randomness (the "opaque generation value" the
protocol recommends, so identifiers from two scoot sessions never collide)
and, after the last dash, the window id `scoot msg windows` reports. That
suffix is deliberate and is the bridge between the two: this protocol has no
requests at all, so a client that finds a window here and wants to *act* on
it sends `scoot msg action focus-window-id N` with the number after the
dash.

Worth knowing before you write against it:

- **There is no control half, by design.** The protocol is deliberately
  minimal — no `activate`, `close`, `minimize`, `fullscreen`, geometry or
  per-output state. Those are meant for extension protocols that don't exist
  yet. Use the wlr protocol below, or scoot's IPC (`scoot msg action
  focus-window-id N`, `scoot msg action close` for the focused window).
- **A handle covers a window's whole life, not the time it is mapped.** The
  protocol talks about "mapped" toplevels, but scoot has no map/unmap
  boundary at all — a window is in the layout, the focus order and
  `scoot msg windows` from the moment its `xdg_toplevel` exists — so a
  handle covers exactly that, and the two lists can never disagree.
- **An identifier is never reused.** Close a window and open another and it
  gets a new one, even from the same client.
- **The list stays live while the session is locked**, exactly as `scoot msg
  windows` does — see Screen locking below for the trust model that sits
  behind that.

### `wlr-foreign-toplevel-management-unstable-v1`

The older protocol, kept alongside the `ext-` one rather than instead of it.
The reason is measurement, not preference: stock `quickshell` 0.3.1 — the
build DMS and Noctalia both run on — is offered `ext_foreign_toplevel_list_v1`
and never binds it, because its `ToplevelManager` is a wlr client. Without
this, both shells' window sections render empty. See
`docs/backlog/resolved/wlr-foreign-toplevel-management-done.md` for the
measurement, both before and after.

What a client sees, per window, on a `zwlr_foreign_toplevel_handle_v1`:

- **`title` and `app_id`**, and **`output_enter`** naming the screen it is
  on. Binding the global announces every window that already exists,
  oldest first, so registry order doesn't matter — and a client that binds
  `wl_output` *after* the manager is sent the `output_enter` it missed as
  soon as it does.
- **`state`, carrying `activated` and nothing else** — scoot's real window
  focus, the same one `scoot msg windows` reports as `focused`.
- **Changes arrive in batches closed by `done`**, same as the `ext-` list:
  draw on `done`. A window is announced before its toolkit has sent a title
  or taken focus, so its first batch is usually two empty strings and an
  empty state array, with a batch per fact as they arrive.
- **`closed` when the window goes.** The handle then becomes inert: it is
  still a valid object until the client destroys it, and every request on it
  is ignored.
- **`stop` is answered with `finished`**, and means "no more *new* windows":
  handles the client already has keep reporting until it destroys them,
  which is what the protocol's own teardown sequence requires.
- **Binding the manager counts against the same 8-bind per-client budget as
  the workspace manager above** — a ninth bind of any of the four globals is
  answered `finished` rather than announced.

What a client can ask for:

- **`activate`** focuses that window, with the same effect on the keyboard as
  a click on the window itself: if your panel is a layer surface that took
  the keyboard when the user clicked it, `activate` hands the keyboard on to
  the window, exactly as clicking the window would. It does that whether or
  not the window was already the focused one. **`scoot msg action
  focus-window-id N` does not do this** — it moves window focus the same way
  but does not take the keyboard back off a layer surface that holds it, so
  an agent driving focus over IPC while a clicked panel still has the
  keyboard will have its keystrokes delivered to the panel, not the window,
  with nothing in `scoot msg windows` to show the mismatch. See
  `docs/backlog/protocols/activation-leaves-the-keyboard-on-a-clicked-layer-surface.md`.
- **`close`** sends the window's `xdg_toplevel.close`. Whether the window
  actually goes is up to its own client, as the protocol says; `closed`
  follows if and when it does.
- **`set_maximized`, `unset_maximized`, `set_minimized`, `unset_minimized`,
  `set_fullscreen`, `unset_fullscreen` and `set_rectangle` are accepted and
  do nothing.** scoot has no concept of maximized, minimized or fullscreen
  at all, so the matching state bits are never sent either — a taskbar's
  minimise button is inert rather than lying. `set_rectangle` is a
  minimise-animation hint scoot reads nothing from; unlike wlroots, an
  invalid rectangle is ignored rather than answered with a protocol error,
  because disconnecting a shell over a number nothing looks at would be
  worse.

Worth knowing before you write against it:

- **`activate` needs a seat, and a headless shell may not have one.**
  Measured while verifying this: quickshell sources the `wl_seat` argument
  from Qt's last input device, so a `ShellRoot` with no window and no input
  event sends nothing at all when you call `activate()` — no request reaches
  the compositor. With a real `PanelWindow` and a real click it works. If you
  are testing this, test it with a window.
- **`output_leave` is never sent.** scoot has one output, a window is on it
  for its whole life, and switching workspaces does not move it — the same
  answer wlroots-based compositors give.
- **`parent` is never sent** (the version 3 event). scoot's layout has no
  parent/child relation; every `xdg_toplevel` is an independent column
  entry, dialogs included.
- **The list stays live while the session is locked, but the two requests
  are refused.** Same trust model as everything else here (see Screen
  locking below) — but a window you cannot see must not be focused or
  closed from behind the lock screen.

## Display information (`wlr-output-management-unstable-v1`)

scoot implements `zwlr_output_manager_v1` (version 4), which is what
`wlr-randr`, `kanshi` and a shell's Settings → Display page read the screen's
modes, position, scale and transform from. `wl_output` says what the screen
*is*; this is the management protocol on top of it. The global is available to
every client, no privilege or allow-list.

**It is read-only. `apply` and `test` always answer `failed`.** scoot has
exactly one output, whose mode, position, scale and transform are fixed for the
life of the process, so there is nothing a configuration could change. That is
a refusal, not a stub: a configuration that reported `succeeded` and changed
nothing would give you a Display page whose buttons appear to work.
`wlr-randr --output <name> --pos 100,100` prints `failed to apply
configuration` and exits non-zero, which is the honest answer. Reconfiguration
is closed as a deliberate refusal rather than deferred to multi-output — the
protocol has no authorization concept, the per-backend honest answers differ
(nested is host-constrained, custom modes are unhonorable, disabling the only
output has no state to land in), and neither probed shell attempts a write; see
`docs/backlog/resolved/output-management-reconfiguration-done.md`.

This is the wlr protocol rather than an `ext-` one only because no `ext-`
successor exists yet — unlike `ext-workspace-v1` and
`ext-foreign-toplevel-list-v1` above, there is nothing newer to prefer.

What a client sees:

- **One `zwlr_output_head_v1`**, since there is one output, carrying its
  `name` (the same one `wl_output` reports — a DRM connector name like
  `HDMI-A-1` under `--tty`, `headless` otherwise), `description`, `make`,
  `model`, `enabled`, `position`, `transform`, `scale` and `adaptive_sync`
  (always `disabled`; scoot has no VRR support).
- **A `zwlr_output_mode_v1` per mode the output knows**, with its size,
  refresh rate and whether it is preferred. Binding the global announces
  everything immediately, so registry order doesn't matter.
- **Changes arrive in batches closed by `done`**, which carries a serial that
  advances on every real change. The only thing that can trigger one today is
  a `--nested` host's *initial* configure at startup, if it proposes a size
  other than `--width`/`--height` — scoot applies that one and then ignores
  every later resize of the host window, so nothing changes again after
  startup.
- **`stop` is answered with `finished`**, after which the head and mode
  objects the client already has stay valid until it destroys them — the
  protocol's own teardown order.
- **Binding the manager counts against the same 8-bind per-client budget as
  the workspace manager above** — a ninth bind of any of the four globals is
  answered `done` and then `finished` rather than announced.

Worth knowing before you write against it:

- **The refresh rate is always 60 Hz**, including under `--tty` on a faster
  panel. `wl_output` already reports the same thing; this mirrors it rather
  than adding a second, differently-wrong number.
- **No `physical_size` and no `serial_number`.** scoot knows neither (its
  physical size is `0x0` on `wl_output` too, and its serial is a placeholder),
  and the protocol allows omitting both. A client that keys a saved
  per-monitor profile off a serial would otherwise match every scoot session
  on every machine.
- **The mode list only grows, never shrinks.** A new mode is added rather
  than replacing the old one, so every size the output has had stays
  advertised — matching what `wl_output` does with the same change. Under
  `--nested` that happens at most once (only the host's *initial* configure,
  if it proposes a size other than `--width`/`--height`; a host resizing the
  window afterwards does nothing). Under `--tty` it happens once per mode
  the display actually changes to, so a VM window moved between a 2x and a
  1x screen a few times leaves a mode for each distinct size it settled at.
  Bounded by the number of distinct sizes the connector has offered, not by
  how many hotplug events arrive.
- **A `--tty` VT switch changes nothing by itself.** The output does not go
  away when you switch to another VT, it just stops being drawn, so the head
  stays enabled with the same mode and no `done` is sent. The one thing a
  switch *back* can produce is a mode change, and the switch is not what
  caused it: a display plugged in or resized while scoot was on another VT
  could not be acted on then, so the switch back re-probes and applies
  whatever moved. Switch away and back with the display untouched and
  nothing is sent.
- **Multiple outputs will change the shape of this** — one head per output is
  what the protocol is built for — but scoot has exactly one output today.

## Screen capture for clients (`ext-image-copy-capture-v1`)

scoot implements `ext-image-copy-capture-v1` (version 1) together with
`ext-image-capture-source-v1` (version 1), which is what `grim`, a shell's
workspace-overview live preview, a screen recorder or a conferencing
screen-share uses to read the screen. Three globals are advertised:

| Global | What it is for |
| ------ | -------------- |
| `ext_image_copy_capture_manager_v1` | Creating a capture session and its frames |
| `ext_output_image_capture_source_manager_v1` | Turning a `wl_output` into a capture source |
| `zwp_linux_dmabuf_v1` | Real dmabuf import (GPU-rendering clients), plus format feedback (see below) |

All three are available to every client, no privilege or allow-list (scoot has no
security-context support to distinguish a privileged client from any other, so
an allow-list would be theatre — see the trust note under Screen locking
below). Screen capture is the most obviously sensitive protocol that applies
to, so it is worth saying plainly: **any process that can reach scoot's
Wayland socket can read your screen.**

This is separate from, and does not change, `scoot msg screenshot` — that
still goes over the privileged, owner-only IPC socket, is rate-limited per
connection, and is what an agent uses. This is the standard-protocol path, for
tools that will never speak scoot's own IPC.

`grim` works with no flags:

```sh
grim /tmp/screen.png          # the whole output
grim -t ppm - | ...           # or to stdout
```

What to know before pointing a client at it:

- **Output capture only.** A source can be made from a `wl_output`; there is
  no `ext_foreign_toplevel_image_capture_source_manager_v1`, so a *single
  window* cannot be captured on its own. That half was measured and closed as
  unreachable for the shell clients rather than stubbed here
  (`docs/backlog/resolved/screencopy-toplevel-capture-done.md` — stock
  quickshell routes per-window thumbnails exclusively to
  `hyprland-toplevel-export-v1`): the global is not advertised at all, so a
  client takes its fallback path immediately instead of discovering a refusal
  at runtime. The remaining thumbnail fallback (region crop out of the
  output) was measured and closed as needs-upstream, with the shell-side
  recipe proven live
  (`docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md`):
  a screen-source `ScreencopyView` in a clipped container at the window's
  rect needs no compositor work — DMS's `TileItem.qml` hard-requires a
  `Toplevel` source, so the shells must change.
- **`wl_shm` buffers only, `Xrgb8888` or `Argb8888`.** A capture session never
  offers to write into a dma-buf (`BufferConstraints::dma` is always `None`) —
  capture is the direction where scoot does the writing, and `wl_shm` already
  works everywhere at no extra cost to a CPU renderer. (The other direction,
  a *client's* dma-buf, is imported — see the next bullet.)
  `Xrgb8888` is offered first: if `[appearance]
  background_color` has an alpha below 1.0 then the framebuffer really is
  translucent, and an `Xrgb8888` capture forces the fourth byte opaque so you
  get a screenshot rather than a translucent image. With the default opaque
  background the framebuffer already is opaque everywhere, so that pass is
  skipped — same bytes either way. An `Argb8888` capture
  hands you the framebuffer's own alpha, which is what that format means.
- **`zwp_linux_dmabuf_v1` is advertised (version 6), and dmabufs really are
  imported — a GPU-rendering client works here, with no GPU on the
  compositor side.** The client renders with the GPU and hands over a
  dma-buf; scoot `mmap`s it and composites it with pixman, on the CPU, next
  to `wl_shm` clients in the same session. No `LIBGL_ALWAYS_SOFTWARE=1`
  needed, and nothing about the GPU-free requirement changes: the compositor
  still never touches a GPU.

  What is advertised: `Xrgb8888` then `Argb8888`, `LINEAR` only, single-plane
  only — exactly what the pixman renderer can map, and no more. A format in
  the table that could not then be imported would kill the client that
  believed it (`zwp_linux_buffer_params_v1.create_immed` has no soft
  refusal), so the table is pinned to the renderer's own importable set by
  test. A client that ignores the feedback and offers a multi-plane or
  non-`LINEAR` buffer is refused: `failed` on the asynchronous `create`,
  which it survives, and a protocol error on `create_immed`, which the
  protocol prescribes.

  `main_device` names this machine's **render node** (`/dev/dri/renderD128`,
  else `card0`, else `0` where no DRM node exists) — the device a client
  should allocate against, since it only needs to render, not to scan out.

  The global also gates screen capture for some shells: quickshell's buffer
  manager instantiates no capture context at all — not even the `wl_shm` one
  — until it has seen real dmabuf feedback, so without this advertisement
  every quickshell `ScreencopyView` stays blank despite the capture protocol
  above working. There is deliberately no empty table and no omitted device:
  a client maps the table Smithay always sends, and a zero-length map aborts
  it rather than merely not flipping it.
- **The buffer size is the framebuffer's, and it is re-advertised on a
  resize.** If `--tty` follows a hotplug to a new mode (see `--tty` follows
  the display above), every live session gets a fresh `buffer_size` +
  `done`. A capture whose buffer is now *too small* is answered
  `failed(buffer_constraints)`, asking the client to re-allocate — but one
  whose buffer is still large enough (the output shrank) succeeds into the
  oversized buffer, with whatever the client's own margin pixels already
  held left untouched outside the captured region. A client that resizes
  its buffer exactly to match `buffer_size` on every `done`, as the protocol
  expects, never sees this.
- **A session's first capture is served on the next frame; later ones wait
  for the screen to change.** The protocol allows exactly this ("the
  compositor may wait an indefinite amount of time for the source content to
  change"), and it is what keeps a live-preview client from costing a
  full-screen copy every frame on a desktop that is not moving. A capture
  parked this way is served the moment anything redraws.
- **At most one *outstanding capture* per session** — a second `capture`
  request before the first has been answered is failed rather than queued.
  And at most sixteen live frame *objects* per client, across all of its
  sessions: a further `create_frame` is refused with the protocol's own
  `duplicate_frame` error, which disconnects the client that overflowed. The
  protocol's own rule is stricter still (one frame object per *session*),
  which the pinned Smithay rev never enforces and exposes no hook to enforce
  per session — so the bound is per client, set far above anything legitimate
  (`grim` holds one frame per run; a persistent preview holds one per
  session). A client that never stockpiles unclaimed frames never sees it.
- **While the session is locked, a capture sees the lock screen** — never the
  windows behind it, and never a half-drawn transition. This is the same
  guarantee `scoot msg screenshot` gives, and it comes from the same place:
  both read back the framebuffer the compositor just drew, and a locked frame
  never has a window in it (see Screen locking below).
- **The `paint_cursors` option is accepted and has no effect**, which is a
  known deviation. Under `--headless` and `--nested` nothing draws a cursor at
  all, so a capture never contains one. Under `--tty` the cursor is part of
  the one framebuffer a capture is read out of, so a capture always contains
  it, flag or no flag. `scoot msg screenshot` behaves the same way.
- **Cursor capture sessions are refused.** `create_pointer_cursor_session`
  itself gets no event — the cursor-session object has no `stopped` of its
  own — but the `ext_image_copy_capture_session_v1` a client gets back from
  its `get_capture_session` is answered `stopped` immediately, which is
  where a client actually observes the refusal. Capturing the cursor
  *image* into its own buffer is a second render target nothing measured
  asks for.

`wlr-screencopy-unstable-v1` is deliberately **not** implemented alongside it,
unlike the two window-list protocols above. The clients that motivated this
already speak the `ext-` protocol: `grim` 1.5.0 carries
`ext_image_copy_capture_v1` and nothing else, and stock quickshell 0.3.1 — the
build both DMS and Noctalia run on — carries the `ext-` manager and both
`ext-` source managers.

## Screen locking (`ext-session-lock-v1`)

scoot implements `ext-session-lock-v1` (version 1), so a real locker
(`swaylock` 1.7+, `gtklock`, `hyprlock`, `waylock`) can lock the session with
the compositor enforcing it, rather than a layer surface asking politely. The
global is `ext_session_lock_manager_v1`, available to every client (scoot has
no security-context support to distinguish a privileged client from any
other, so an allow-list would be theatre — see the trust note below).

**What is guaranteed while the session is locked**, each of these verified on
real `--tty` hardware by screenshot and by what clients were actually sent:

- **Nothing but the lock client's own surfaces is drawn.** Not "drawn behind
  an opaque backdrop" — windows, layer surfaces (bars, wallpapers, launchers,
  on every layer including `overlay`) and the focus ring are not gathered into
  the frame at all. The screen is the lock surface, an opaque backdrop where
  it doesn't cover, and the pointer cursor. A `scoot msg screenshot` reads
  that same framebuffer, so a capture taken while locked shows the lock screen
  and nothing behind it.
- **Only the lock surface receives input.** Keyboard focus moves to it (or to
  nobody, if the client hasn't created one yet) the instant the lock request
  arrives, and pointer focus is moved with it, so a click can't land in the
  window that happened to be under the pointer. Pointer focus is re-derived
  again on the commit that maps the lock surface, so the first click lands on
  it without the mouse having to move first. Any grab in flight is
  dropped as well — not only when the lock is taken, but at every transition
  that changes which lock surfaces count (a lock surface destroyed, a lock
  given up, a locker that died), because a grab outlives focus changes by
  design and would otherwise keep steering input to whoever holds it. That
  covers an open popup menu, whose grab holds the *keyboard* and would
  otherwise swallow the focus change entirely and receive the password: the
  menu is dismissed, and a new grab asked for while locked is refused.
  A held-button grab and an open popup grab are each exercised by a test; the
  drag-and-drop case is still code-traced rather than exercised, since there
  is no drag-and-drop fixture here.
- **"The lock surface" means the *current* lock's.** A lock object can be
  destroyed while the client that made it stays connected and keeps the
  `wl_surface` underneath alive, so "is this surface alive" is not the same
  question as "does this surface still belong to the lock that owns the
  session". scoot asks the second one everywhere: a surface from a lock that
  was given up, or replaced, stops being drawn and stops receiving keyboard
  and pointer input immediately, without waiting for anything to replace it.
- **A destroyed lock surface falls back to the backdrop straight away.** The
  protocol's own rule ("the compositor must fall back to rendering a solid
  color"), and "straight away" means without waiting for anything else to
  change on screen — including when the client destroys only the
  `ext_session_lock_surface_v1` and keeps the `wl_surface` under it alive,
  which is what a locker does when an output is removed under it.
- **Keybindings that run an action don't fire.** `Super+Q`, a `spawn` bind,
  every layout motion: suppressed, and forwarded to the lock client as
  ordinary keystrokes instead. The one exception is deliberate: the `--tty`
  `Ctrl+Alt+F1`..`F12` VT switches still work. That is a session-level escape
  hatch, not a way in — the VT it switches to has its own login, and this
  session stays locked behind it (verified: switch away, switch back, the lock
  screen is pixel-identical).
- **`scoot msg action ...` is refused**, with an error saying why, for the
  same reason. So is an `ext-workspace-v1` client's `activate`.
- **Ordinary clients stop drawing.** They get no frame callbacks while
  locked, which is what the protocol asks for and also what keeps them from
  burning CPU behind a lock screen: measured on real hardware, a terminal
  running `while true; do date; done` costs the compositor 229 jiffies/10s
  unlocked and **2 jiffies/10s** with the same client still running behind a
  lock.

**If the lock client dies, the session stays locked.** That is the protocol's
rule and the point of it: a dead locker is not evidence that you want your
screen unlocked. scoot's recovery story, so a crashed locker isn't a dead
session:

- the screen turns **solid red**, so you can tell "my locker crashed" from "my
  locker is showing a black screen" — including when the locker had a surface
  up and drawing at the moment it went, which is the case where showing its
  last pixels instead would leave you with no way to tell;
- **run a lock client again and it takes over** — it puts its own surface up
  and can unlock once you authenticate. It is told `locked` immediately when
  the lock it replaces had already blanked the screen; if that lock went in
  the window *before* its first blanked frame, the replacement waits for one
  exactly like a fresh lock does, because until then your actual desktop is
  still what's on the display. Taking the lock over at all rather than
  refusing is what sway does too (`lock.c`'s `handle_session_lock` replaces an
  abandoned lock and refuses a live one, exactly as scoot does); the
  *conditional* confirmation is niri's (`Niri::lock` confirms immediately only
  from an already-locked state, never from a lock still waiting for its first
  frame). sway confirms unconditionally, fresh lock or takeover alike, so that
  half is deliberately not sway's behavior.

The same applies to a lock client that gives up without dying: destroying an
`ext_session_lock_v1` before `locked` arrives is legal (only
`unlock_and_destroy` is forbidden that early), and a locker that times out
waiting may well do it. The session stays locked and reads as abandoned — the
red screen, recovered by running a locker again — and the surfaces that client
had up stop being drawn and stop receiving input at that moment, even though
its connection and its `wl_surface`s are still perfectly alive.

**What is *not* guaranteed — read this before trusting it:**

- **A same-uid process is inside the boundary, and always was.** Anything that
  can reach scoot's wayland socket can take over a lock whose client has died
  and then unlock the session; anything that can reach its IPC socket can
  screenshot the lock screen and inject keystrokes into it (that is how an
  agent drives a lock screen, and it is refused for `action` requests only).
  Both sockets are owner-only. This lock keeps *someone at the keyboard* out,
  not a process already running as you — which could read your files anyway.
- **`scoot msg windows` still lists your windows while locked**, titles
  included, and `scoot msg outputs` still answers. Nothing is drawn from
  them, but the IPC surface is not blanked.
- **So do both foreign-toplevel protocols** (see Window lists for bars
  above): handles stay, titles keep updating, and a window opened behind the
  lock screen is still announced. Same boundary as the line above — a client
  that can reach the wayland socket is a same-uid process — and sending
  `closed` for windows that did not close would be a lie a taskbar could not
  recover from, since the `ext-` protocol forbids reusing their identifiers
  afterwards.
- **The wlr protocol's two *requests* are refused while locked**, which is
  the one thing that does change: `activate` will not move focus and `close`
  will not reach a window, because a window you cannot see must not be
  focused or closed from behind the lock screen. That is the same gate every
  `scoot msg action` request already sits behind.
- **The `locked` event is sent once a blanked frame is confirmed on screen,
  not merely rendered.** Under `--headless`/`--nested` the render is the
  confirmation — there is no scanout at all, and the framebuffer a screenshot
  reads *is* that frame. Under `--tty` the render hands the frame to the
  presenter, and `locked` waits for the vblank confirming the page flip that
  carries it, so the previous, possibly unlocked, frame can no longer outstay
  the event by a vblank. That costs up to one vblank of lock latency under
  contention (the lock raced a flip already in flight and its frame goes out
  on the next one). If no vblank can arrive at all — you switched VT away, a
  modeset discarded the flip — the lock is confirmed anyway after one second
  (logged as a warning) rather than hanging the locker forever: a locker left
  waiting might never prompt for the password at all, which is worse than the
  bounded staleness the fallback accepts.
- **Up to one frame of the unlocked screen can still be on the display**
  between the lock request and the first blanked frame. That is inherent (the
  protocol's `locked` ordering exists precisely because of it), not something
  scoot defers. Input is already captured during that frame — the keyboard
  and pointer leave the window the instant the request arrives — so nothing
  typed in that window reaches the unlocked session.
- **scoot blanks immediately rather than waiting for the lock client to
  draw.** Some compositors wait up to a second for lock surfaces so the
  transition doesn't flash black; scoot doesn't, deliberately — waiting means
  rendering the unlocked session for that whole second.
- **One output.** A lock surface is configured per `wl_output` and scoot has
  exactly one, so the one live surface a lock may hold is configured to that
  output's size, and the first blanked frame on it is what sends `locked`;
  a second `get_lock_surface` for the same output is refused with the
  protocol's `duplicate_output` error even when it names the output through
  a different `wl_output` bind (destroying the first surface frees the
  output for a rebuild). Multi-output support has to revisit all four halves.

## Idle detection (`ext-idle-notify-v1`, `idle-inhibit-unstable-v1`)

scoot implements `ext_idle_notifier_v1` (version 2), so a `swayidle`-style
daemon can learn the seat has been quiet N milliseconds and dim the screen,
lock it (see Screen locking above) or suspend the machine — verified live
with real swayidle: the timeout command fires after a quiet window, input
runs the resume command, and the next quiet window fires again. Every input
source resets the timers: real devices under `--tty`, host-forwarded input
under `--nested`, and IPC-injected input (`scoot msg type`/`key`/`pointer`
count as a user at the machine, which is also what keeps an agent's own
activity from looking like idleness). What does *not* reset them is the
compositor re-running its own hit test on a lock transition — that is not
input and doesn't claim to be.

`zwp_idle_inhibit_manager_v1` (version 1) is the reverse: a video player or
presentation app creates an inhibitor on one of its surfaces and `idled`
holds off until the inhibitor is gone — destroyed explicitly, or released
implicitly when the surface dies or the client disconnects. The
input-specific watch (`get_input_idle_notification`, for daemons with their
own inhibit policy) ignores inhibitors by design.

Two things to know: there is no built-in auto-locker — the timeouts and
commands are the daemon's config, not scoot's, the swayidle way — and an
inhibitor counts while its surface is *alive*, whether or not it is visible.
A client inhibiting from a surface it never maps holds off `idled`; that
client is local either way (same trust model as the session-lock global
above).

## Clipboard managers and primary selection

scoot exposes all three selection globals, on every backend, available to
every client (no security-context support here, so an allow-list would be
theatre — same trust model as the session-lock global: a same-uid process is
inside the boundary, see Screen locking above):

- **`zwlr_data_control_manager_v1`** (version 2): clipboard managers
  (`cliphist`, `clipman`). Set the clipboard without needing focus, read
  anything any client copies.
- **`ext_data_control_manager_v1`** (version 1): the successor protocol.
  Both generations are exposed side by side, as current compositors do, so a
  manager speaks whichever one it was written for.
- **`zwp_primary_selection_device_manager_v1`** (version 1): middle-click
  paste. Unlike the clipboard, this one is focus-gated — the compositor only
  accepts a `set_selection` from the client holding the keyboard, and only
  offers the selection to devices whose client holds it. A background client
  setting the primary selection is silently denied, not queued.

## Night light (`zwlr_gamma_control_manager_v1`)

scoot implements `zwlr_gamma_control_manager_v1` (version 1), so
`gammastep` and `wlsunset` work, on every backend, available to every client
(same trust note as above — there is no privileged seat to reserve this
for). One control per output, and scoot has exactly one output: a second
`get_gamma_control` transfers control, the old control gets `failed` and
stops affecting anything, and destroying the live control (or disconnecting
with one held) restores the default linear ramp.

What actually happens to the ramp depends on the backend:

- **Under `--tty`**, the ramp is pushed to the CRTC gamma LUT, so the screen
  really warms. The advertised `gamma_size` is the CRTC's own (256 on the
  hardware measured so far), re-read whenever a hotplug moves the session to
  a different CRTC; a live control hears `failed` over that move either
  way, so it re-reads `gamma_size` and re-pushes (nothing carries the old
  ramp to the new CRTC — a modeset moves planes, not LUT contents).
  Anything the DRM device refuses retires the
  control with `failed` and the session keeps running.
- **Under `--headless`/`--nested`** there is no hardware LUT, so the ramp is
  accepted but changes nothing on screen — and a `scoot msg
  screenshot` reads the framebuffer, which is pre-LUT, so captures show the
  unmodified frame either way. `gamma_size` is 256 there.

A `set_gamma` fd must hold exactly three ramps of `gamma_size`
little-endian `u16` entries (red, green, blue); anything else — short, long,
empty, unreadable — is an `invalid_gamma` protocol error. Gamma control keeps
working while the session is locked: it changes no pixel's content, only the
output's color temperature, so a daemon warming the screen over a lock screen
is the ordinary case.

No config option, keybinding, CLI flag or IPC surface comes with any of
this: clipboard, primary selection and gamma are pure Wayland protocols, used
by existing clients as-is.

## Cursor shapes (`wp-cursor-shape-v1`)

scoot implements `wp_cursor_shape_manager_v1` (version 2), so a client can
*name* the cursor it wants — `text`, `ew-resize`, `not-allowed` — instead of
loading an xcursor theme and uploading a surface of its own. Modern GTK4/Qt6
toolkits and `foot` prefer this when it exists; without it `foot` logs
"compositor does not implement server-side cursors".

A named shape is answered from **the cursor theme already installed on the
machine**: scoot resolves `[appearance] cursor_theme`, else `$XCURSOR_THEME`,
else `default`, and draws that theme's own artwork — the same pixels the
client would have loaded for itself. That is what makes the protocol a win
rather than a downgrade: without it, advertising cursor-shape would take a
correctly themed I-beam away from a client that had been uploading one and
replace it with line art.

scoot still ships no theme (niri's assets are GPL and Adwaita's aren't
MIT-clean — see License below), and it does not need to: reading the user's
own installed theme carries no such obligation, and is what sway, niri and
Hyprland do. Parsing is the MIT-licensed `xcursor` crate; nothing is
vendored.

**When no theme is installed** — a linuxserver webtop or any minimal
container, which is a first-class scoot target — named shapes fall back to
ten shapes scoot **draws procedurally itself**, in `cursor/shapes.rs`. The
mapping collapses the names a user cannot tell apart at 16 pixels:

| Drawn as | Named shapes it answers |
| --- | --- |
| arrow | `default`, plus every name with no row of its own — `help`, `wait`, `progress`, `pointer`, `zoom-in`, … |
| I-beam | `text` |
| sideways I-beam | `vertical-text` |
| crosshair | `crosshair`, `cell` |
| vertical double arrow | `n-resize`, `s-resize`, `ns-resize`, `row-resize` |
| horizontal double arrow | `e-resize`, `w-resize`, `ew-resize`, `col-resize` |
| diagonal double arrow | `ne-resize`, `sw-resize`, `nesw-resize` / `nw-resize`, `se-resize`, `nwse-resize` |
| four-way arrow | `move`, `all-scroll`, `all-resize`, `grab`, `grabbing` |
| circle with a slash | `not-allowed`, `no-drop` |

The drawn shapes use the same `[appearance]` `cursor_size`/`cursor_color` the
built-in arrow already did, are built once at startup (never per frame and
never per request), and the arrow stays byte-identical to what it has always
been. Theme images are loaded when a client first asks for that shape — an
event, not a frame — and cached from then on, including a negative cache so a
theme missing `zoom-in` is not re-searched on every hover.

A client that uploads its own cursor *surface* still gets its own pixels
drawn, exactly as before. scoot also exports `XCURSOR_THEME`/`XCURSOR_SIZE`
to everything it spawns, so a client that loads a theme itself (GTK3, and
anything predating this protocol) picks the same one the compositor draws —
which is the "consistent cursor across clients" half the protocol cannot
reach on its own.

Cursors are only drawn under `--tty`; `--headless` has no display and
`--nested` shows the host compositor's own cursor.

## Focus handoff between clients (`xdg-activation-v1`)

scoot implements `xdg_activation_v1` (version 1), so a launcher can hand
focus to the app it started and a notification daemon can focus the app its
popup came from. Without it the only way to move focus is scoot's own
keybindings and IPC — a client has no standard way to ask.

Honoring every such request unconditionally would be a focus-stealing
primitive, so scoot checks the token three ways (see
`compositor/activation.rs`):

- **The token must name a real, recent input event that went to the client
  asking.** `set_serial(serial, seat)` is how a client says which click or
  keypress caused it to ask, and a token that names none of them — no serial
  at all, a seat scoot doesn't own, or a stale or made-up number — is
  refused when it is created. What counts is a serial scoot issued for a key
  or button event (press *or* release; pointer motion never counts, and
  neither does a *focus* event, which every newly mapped window gets for
  free) within the last few input events **and the last 10 seconds**, **and
  that was delivered to that same client**. Both bounds are needed: an idle
  session — an agent driving scoot over IPC makes one, since injected
  actions are not input events — never rotates the event history, so without
  the clock a click from this morning would still be spendable tonight. The
  recipient half matters too: Wayland serials come from one
  process-wide counter shared with non-input events, so a number alone is
  cheap to observe and guess — pairing it with who actually received the
  event is what makes this a check on interaction. A launcher minting its
  token inside its own input handler passes; a background client that was
  never typed into or clicked has nothing to offer, which is exactly the
  focus-steal case the two bounds below do not stop.
- **A token is valid for 30 seconds.** Long enough for a cold-starting app to
  finish launching and redeem the token its launcher gave it; short enough
  that a token is still a receipt for something the user just did rather than
  a permit a client can sit on.
- **At most 64 unredeemed tokens exist at once**, across all clients, with
  expired ones swept first. `get_activation_token` is unauthenticated and
  unlimited, and nothing upstream prunes what it hands out; this is the same
  resource bound as the `wl_shm` pool cap.

The serial is checked when the token is *created*, never when it is redeemed:
a launcher hands its token to a process that may take seconds to start, by
which point plenty of newer input has happened, and re-checking then would
break the one case this protocol exists for. A client the user really did
interact with can still activate itself off that interaction — the user just
clicked it, which is the protocol working as intended.

An app scoot itself started — a keybinding or `msg action spawn` — gets its
token a different way: `State::spawn` mints one from the compositor and hands
it to the child in `$XDG_ACTIVATION_TOKEN`, so the child can activate its own
window when it maps one. That covers the slow cold start, where focus has
moved elsewhere before the window appears. The token lives under the same two
bounds above (30 seconds from spawn, one of the same 64 slots, swept the
same way); a spawn past a full table simply gets no token, and the window is
still focused on map.

What a refused token costs depends on what was being activated. For a **fresh
spawn** it is invisible: the app maps a window, and mapping focuses it, which
is where a launched app's focus came from before this protocol existed. For
**something already running** — a single-instance app (Firefox, Chromium,
anything on `GApplication`) re-invoked from a launcher, or the notification
daemon case above — nothing maps, so nothing else focuses it and the
activation simply does not happen. That is worth knowing because of the one
case scoot refuses that the protocol would allow: a launcher that mints its
token from a *focus* serial rather than a key or button one (fuzzel does this
when an entry is picked with the mouse) gets no activation, and for an
already-running target that is the whole outcome.

A redeemed token is removed whether or not it was honored, so one user action
cannot be replayed into focus later. Activation goes through the same action
path a keybinding does, which means it is refused while the session is locked
and it scrolls the activated column into view rather than only marking it
focused. A refused activation does nothing visible: scoot has no per-window
urgency state to raise instead.

## Window icons (`xdg-toplevel-icon-v1`)

scoot implements `xdg_toplevel_icon_manager_v1` (version 1), so a client can
say which icon belongs to its window. scoot draws no icons itself — it has
no titlebars, taskbar or window switcher — so this exists for the two
consumers outside it: a bar or dock showing a window list, and an agent
driving the session.

`scoot msg windows` therefore grew one field:

```json
{ "id": 1, "app_id": "foot", "title": "zsh", "icon": "foot", ... }
```

`icon` is the freedesktop icon name the client committed, or `null`. It is
read off the surface's own current state when asked, so it is never stale and
never reports an icon the client attached but has not committed. The field is
optional on the wire (an older server simply omits it), so it does not bump
`PROTOCOL_VERSION`. Two deliberate limits: no *preferred icon sizes* are
advertised, since nothing in scoot draws an icon and so it has no size to
prefer; and a client that supplies raw pixel buffers instead of a name reads
as having no icon, since handing those over IPC would mean re-encoding shm
buffers to PNG per query and no consumer has asked for it.

The buffer half, for the toolkit author: pixel buffers must be square and
`wl_shm`-backed (anything else is refused with `invalid_buffer`), and they
are ordinary live `wl_buffer`s under the 512-per-client bound above, so a
client already at its budget is refused further creations. There is no
`release` for icon buffers — the protocol leaves the event unused — and a
buffer destroyed while its icon still lives disconnects that client
(`no_buffer`); destroying the icon first makes destroying its buffers safe.
Pixels never leave the compositor: neither foreign-toplevel list protocol
has an icon event, and IPC carries the name only.

## Input methods (`text-input-v3`, `input-method-v2`)

scoot implements `zwp_text_input_manager_v3` (version 1) and
`zwp_input_method_manager_v2` (version 1) — the two halves of IME support,
neither of which is useful alone. An application binds the first to say
"there is a text field here"; an input method (fcitx5, ibus, an on-screen
keyboard) binds the second to compose into it. Without the first, `foot` logs
"text input interface not implemented by compositor; IME will be disabled"
and never sets the path up at all.

Which text field is focused follows keyboard focus automatically, so it works
for a layer-shell surface with a search field (a launcher) as well as for an
ordinary window. What scoot owns is the input method's **popup** — the
candidate window beside the text cursor — which is tracked against whichever
surface has the field and drawn with that surface's own popups, so it follows
the window, gets frame callbacks, and disappears when the field is disabled.
That includes a lock screen's password field: while the session is locked the
candidate window is drawn over the lock screen at the caret, with frame
callbacks — and only that popup is: background windows' popups stay hidden
and callback-starved until unlock.

Same trust note as the other privileged globals: there is no client filter on
`zwp_input_method_manager_v2`, because an allow-list would be theatre without
security-context support. An input method is more privileged than a clipboard
manager — it can grab the keyboard and inject text into the focused client —
so this is a deliberate consistency with scoot's existing trust model rather
than an oversight. That grab outranks an `xdg_popup.grab` in both orders:
a menu asked for while the IME holds the seat is refused, and a menu already
up is dismissed when the IME takes the keyboard (see the popup precedence
under Layer shell above) — so no context menu opens in a text field while an
IME is active there.

## Output scaling

A HiDPI panel needs the compositor to tell clients to render at a scale
greater than 1, or everything comes out physically tiny (text especially).
`[output] scale` sets that scale:

```toml
[output]
scale = 2.0        # 1.5, 1.25, ... all work; 1.0 is the default
```

It is advertised three ways, matching what clients actually support:

- **`wl_output.scale`** — the integer `ceil(scale)`. Every client that binds
  an output gets it automatically (re-sent on bind and whenever the output's
  state changes). A scale of `1.5` is therefore advertised as `2` here, which
  is what a client that only understands integer scaling should draw at.
- **`wp_fractional_scale_v1`** — the exact fractional value. A client that
  creates a `wp_fractional_scale_v1` for one of its surfaces is sent
  `preferred_scale` (`1.5`, not `2`), and can render a larger buffer and let
  the compositor scale it down. The `wp_viewporter` global is advertised
  alongside it, because that is the protocol a client uses to submit such a
  buffer (it sets the surface's logical destination size and scoot scales the
  buffer into it) — without it, a fractional client has no way to render.
- **`wl_surface.preferred_buffer_scale`** (needs client `wl_compositor` v6) —
  the integer preference that accompanies the fractional value, sent with the
  default `preferred_buffer_transform` (`normal`). It is a separate event on a
  separate object from the fractional one, so a client that opts into
  fractional scaling receives **both**: the exact `1.5` *and* the integer `2`.
  This is protocol completeness (it is what wlroots sends); it is not claimed
  to be what makes any particular toolkit render. A client below
  `wl_compositor` v6 is not sent the event and keeps the implicit default of 1,
  exactly as before.

Notes, because they are real limits rather than polish:

- **Startup only, and one output.** The scale is read once when scoot starts
  and never changes; there is no config reload and no per-output setting
  (scoot has exactly one output). Changing it means restarting scoot.
- **Clamped to `0.5..=4.0`**, warn-and-continue like every other config field
  (see Configuration below): a `scale` outside that range is brought into it
  and logged, and `nan`/`inf` fall back to `1.0`. A non-finite or zero scale
  would make the logical output size nonsense, so this is a correctness bound,
  not taste.
- **`--nested` is scale-1 only.** The host compositor owns the scale of the
  window scoot is drawn inside, so a non-1.0 `scale` there would double-count
  it; scoot logs a warning and uses `1.0`. `--headless` and `--tty` honour
  the setting.
- **Screenshots are physical pixels; layout coordinates are logical.**
  `scoot msg screenshot` captures the framebuffer at full physical
  resolution, while `scoot msg windows`/`outputs` report logical rectangles.
  An agent converts with `physical = logical * scale`, rounded down where a
   rectangle's edge lands mid-pixel (the logical size is `ceil(physical /
   scale)`, so a full-output `logical * scale` can overshoot by under one
   pixel). `scoot msg outputs` reports each output's `scale` for exactly that
   (older servers omit it, which decodes as `1.0`).

## Single-pixel buffers (`wp_single_pixel_buffer_manager_v1`)

scoot implements `wp_single_pixel_buffer_manager_v1` (version 1), so a
client can mint a solid-color 1x1 buffer straight from four `u32` channels
instead of allocating a shm pool for a single pixel — the cheap fill some
toolkits reach for. The buffer is what the channels say (the full `uint`
range is valid per channel, read as a percentage), reports 1x1, and renders
as a solid fill; a client that wants it bigger scales it through
`wp_viewporter` (advertised alongside, as the spec suggests) rather than by
uploading a larger buffer.

What to know before pointing a client at it:

- **No shm, no pool budget — but inside the buffer bound.** These buffers allocate nothing, so the
  per-client `wl_shm` pool count (see Status above) never moves for them —
  there is no fd, no mapping and no reservation to bound. They still count
  against the 512-live-`wl_buffer` bound above (uniform accounting — the
  hook can't observe buffer kind, and excluding them would let cheap
  destroys drain retaining units), so a client already holding 512 buffers
  of any kind is refused further creations with a bare code-0 error on the
  manager.
- **Destroying the manager leaves its buffers working.** The spec says the
  child objects are unaffected, and they are: a buffer outlives its manager
  and still attaches, commits and draws afterwards.
- **Destroying an attached buffer is legal and safe.** Wayland lets a client
  destroy a `wl_buffer` its surface still names; nothing panics and nobody is
  disconnected for it.

## Relative pointer (`zwp_relative_pointer_manager_v1`, `zwp_pointer_constraints_v1`)

scoot implements `zwp_relative_pointer_manager_v1` (version 1) together
with `zwp_pointer_constraints_v1` (version 1), the pair games and 3D apps
expect: the client locks or confines the pointer to its surface and reads
raw relative motion deltas off its relative-pointer object.

What to know before pointing a client at it:

- **Relative events are gated on pointer focus, not on the lock.** A client
  whose surface has pointer focus receives `relative_motion` whether or not
  it locked; a client without focus receives nothing. That is the protocol's
  own rule ("will only emit events when it has focus"), not a scoot policy.
- **Unaccelerated means pre-libinput-acceleration on `--tty`.** A `--tty`
  mouse reports both an accelerated and a raw device delta, and the relative
  event carries each as its own (`dx`/`dy` vs `dx_unaccel`/`dy_unaccel`).
  Every absolute source (IPC injection, `--nested` host motion, tablets)
  applies no acceleration of its own, so both pairs carry the same position
  change there.
- **Relative deltas are unclipped.** Motion stopped by the output edge, a
  lock, or a confinement still reports the full vector; only the absolute
  position stops.
- **A lock holds the absolute position; a confinement clamps it.** While a
  lock taken on the focused surface is active, the cursor does not move (the
  relative stream keeps flowing). A confinement keeps the pointer on its
  surface, clamped per axis to its region. A lock taken while unfocused
  stays inactive. Destroying a persistent lock or confinement is silent (no
  `unlocked`/`unconfined` event) and frees the pointer immediately.
- **A session lock deactivates a held lock or confinement.** Locking the
  session sends `unlocked`/`unconfined` to the holding client and moves
  pointer focus to the lock surface, so no pointer input -- deltas, buttons
  or scroll -- reaches the game while locked. The persistent entry stays
  registered: unlocking returns focus to the game surface and re-arms it
  there with no new request (the client sees `locked`/`confined` again),
  and the relative stream resumes with absolute still held. A lock
   requested while the session is already locked stays inactive until unlock
   engages it the same way.

## Drawing tablets (`zwp_tablet_manager_v2`)

scoot implements `zwp_tablet_manager_v2` (version 1 -- the most Smithay
carries at the pinned revision; the protocol's own version 2 only adds a
tablet bustype event and pad dials, neither of which scoot mints, see
below), so a drawing tablet works on `--tty` hardware: libinput tool
events reach tablet-aware clients (Krita, Xournal++), and every other
client gets a pen that moves the cursor and clicks.

What to know before pointing a client at it:

- **A pen moves the cursor and clicks; there is no second focus
  system.** Tool proximity and motion run the same path mouse motion
  does, so the cursor follows the pen and pointer focus lands where the
  tool is. A tip tap is a left click through the same click path: it
  focuses the window, mints activation like a click, and dismisses a
  menu tapped outside of.
- **Pressure, tilt, rotation, slider and wheel ride the tool's axis
  events**, announced with proximity and updated by motion. Only changed
  axes are sent: a hovering pen restates no pressure.
- **Stylus barrel buttons are tool-only.** The tool sees the exact button
  number; nothing is synthesized onto the pointer, because no mapping
  from a stylus button onto a mouse button exists to honour.
- **Pads, strips, rings and dials are not supported.** Smithay carries no
  pad objects at the pinned revision, so there is nothing for the
  compositor to drive and `pad_added` never fires. This is deferred
  upstream, not silently omitted.
- **A tool cursor is the cursor.** A shape or surface a client names for
  its tool lands in the same cursor the pointer uses, since the tool is
  what is driving it.
- **A tap on the lock screen reaches the locker**, through both the tool
  and the pointer halves, and moves nothing behind it -- the same two
  paths every other input takes under lock.

## Presentation-time feedback (`wp_presentation`)

scoot implements `wp_presentation` (version 2): a client requests feedback
on its surface and learns, per content update, either exactly when that
update reached the screen (`presented`, with a `CLOCK_MONOTONIC` timestamp,
the output's refresh, a frame sequence and flags) or that the update was
superseded before it ever got there (`discarded`).

What to know before pointing a client at it:

- **The timestamp is the frame handoff, and each backend hands off
  somewhere else.** With no presenter (`--headless` under IPC-only control)
  the framebuffer is the final image, so the timestamp is when the frame
  finished rendering. Under `--nested` it is when the frame was committed
  to the host compositor (when the host scans it out is the host's
  business). Under `--tty` it is when the page flip was issued to DRM, up
  to one vblank before the photons -- and the `vsync` flag is set there,
  because the flip is vblank-synchronized; the other backends report no
  flags, because there is no retrace to synchronize to and no zero-copy
  path behind a pixman copy.
- **`refresh` is the mode scoot advertises, not the panel's.** It is always
  60 Hz -- including under `--tty` on a faster panel, the same known
  inaccuracy as `wl_output`'s own mode (see Display information). A client
  pacing frames should trust the timestamps, not `refresh` plus arithmetic.
- **`seq` is zero except on `--tty`.** Headless has no vertical retrace to
  count and nested output is self-refreshing with no queryable count, so the
  protocol requires zero there; `--tty` reports the issued flip's number
  (a per-flip counter, not the kernel's refresh count).
- **Only displayed surfaces are stamped.** Mapped windows, layer surfaces,
  popups and the client cursor get feedback for a frame that showed them;
  while locked, only the lock surfaces do. Anything else keeps its feedback
  queued until it is shown or superseded (`discarded`).
- **A rendered-but-dropped frame stamps nothing.** A flip skipped for a
  busy CRTC, or a host commit dropped for lack of a free buffer, leaves
  pending feedback for the next presented frame rather than stamping a time
  nothing was shown at.

## Rendering hints (`wp_alpha_modifier_v1`, `wp_content_type_manager_v1`)

scoot implements `wp_alpha_modifier_v1` (version 1) and
`wp_content_type_manager_v1` (version 1), the two client-to-compositor
rendering hints. They are a pair only on this page: one of them works, and
the other is stored and honestly ignored.

- **Alpha does what it says.** A client names a `u32` multiplier on its
  surface (`0` transparent, `u32::MAX` opaque) and the compositor blends it
  -- windows, layer surfaces, lock surfaces and client cursor surfaces
  alike, all through the same render path. Destroying the modifier object
  is `set_multiplier(u32::MAX)` on the next commit, and destroying the
  manager leaves existing modifier objects working.
- **Content type is accepted and has no effect.** A client can label a
  surface `photo`, `video`, `game` or `none`, and the compositor stores the
  label and changes no pixel for it -- a CPU/pixman renderer with no
  adaptive-sync or GPU compositing story has no consumer for the hint, so
  ignoring it is the only truthful implementation. The label is already
  where a future GPU tier would look.
- **Neither touches any bound.** No pool, buffer or fd is created anywhere
  on either path, so the shm-pool count, the live-buffer count and the
  manager-bind budget never move for them.

## Configuration

`--config PATH` loads a TOML file explicitly. Without it, scoot looks for
`$XDG_CONFIG_HOME/scoot/config.toml`, falling back to
`~/.config/scoot/config.toml` if `$XDG_CONFIG_HOME` is unset or empty, and runs on
built-in defaults if neither exists. Five optional tables: `[layout]`,
`[appearance]`, `[output]`, `[tty]`, `[binds]`. Every field in every table is itself
optional and defaults independently, so a config that only sets `gap` leaves
everything else — including the rest of `[layout]` — at its built-in default.

**Failure semantics are deliberate, not an oversight.** An explicit
`--config PATH` that doesn't exist or can't be read is a hard startup
error — you pointed at it on purpose, so silently ignoring it would be worse
than failing loud. Every other problem falls back to defaults and logs
instead of blocking startup, with one deliberate exception (`[tty] gpu`,
below):

- No file at the *default* path: silent, not even a log line (a fresh
  install, not a mistake).
- The default path exists but can't be read (permissions): logged as an
  error, full defaults. (A broken symlink at that path resolves to "no such
  file," which is the silent case above, not this one.)
- Malformed TOML, an unknown/misspelled field name, or a field given the
  wrong type (a string where a number is expected, a negative number for a
  field that's unsigned) **anywhere in the file** (`[layout]`,
  `[appearance]`, `[tty]`, `[binds]`, or the top level): logged as an error, and the
  *entire* file is discarded for full built-in defaults — a single bad field
  in `[layout]` also throws away an otherwise-valid `[binds]` table
  elsewhere in the same file.
- One bad `[appearance]` color string, or one bad `[binds]` entry: logged as
  a warning, and only that field/bind falls back — every other field and
  bind in the file still applies.
- A set-but-unusable `[tty] gpu` (a wrong path, or an empty one): a hard
  startup error naming the key, not a silent fallback to the automatic
  pick — the one place failing loud wins over never blocking startup
  (see `[tty]` below for why falling back there would be fail-open).

This is deliberate: on `--tty`, the real deployment target, scoot *is* the
session — there's no other window manager to fall back to and often no easy
remote access. A compositor that refuses to boot over a config typo is a
hard lockout with no recovery, so it always starts with something usable and
says what's wrong in the log instead.

### `[layout]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `gap` | integer (pixels) | `12` | Gap between columns, between windows stacked in a column, and at output edges. Clamped into `0..=10000`: negatives become `0`, and anything above `10000` becomes `10000` — that is already wider than the long edge of an 8K display (so it insets any real output to nothing), and it keeps the layout's own integer arithmetic well away from overflow. A gap that large leaves no usable area, so windows end up 1x1; it's a guard against a typo or a probe, not a usable setting. |
| `column_widths` | array of floats | `[0.333…, 0.5, 0.666…]` (i.e. `1/3`, `1/2`, `2/3`) | Column widths as fractions of the output width, in the order `cycle-column-width` steps through. Non-finite or non-positive entries are dropped; an empty list falls back to the built-in three. |
| `default_column_width` | integer (unsigned) | `1` | Index into `column_widths` used for newly created columns (`1` selects `0.5`, i.e. half the output). Too large is clamped to the last valid index; negative isn't a valid value for this field at all, so it's a whole-file parse error (see Failure semantics above), not a clamp. |

### `[appearance]`

scoot draws no titlebars by design — a focused window gets a colored ring
drawn *around* it (in the layout's own gap), and there's a solid background
behind everything, instead of a per-window title bar with text or buttons.
That's why there's no titlebar-color/font option below: this table controls
the ring, the background, and the built-in pointer cursor — nothing else.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `focus_ring_width` | integer (pixels) | `3` | Ring thickness. Clamped at load time to at most half of `gap`, so it can never visually reach a neighboring window. |
| `focus_ring_active_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#6ba6fa` (accent blue) | Ring color around the focused window. |
| `focus_ring_inactive_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#595961` (muted gray) | Ring color around every other window. |
| `background_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#141419` (near-black) | Cleared behind all window content — there's no separate background render element, this is the frame clear color. |
| `cursor_size` | integer (pixels) | `16` | Both dimensions of the built-in pointer cursor (see below). Clamped into `4..=256`: under `4` the shape is left with at most one interior pixel (none at all below 3), and a pointer that small is indistinguishable from a dead pixel; over `256` it covers a quarter of a 1080p display's height and the bitmap it allocates stops being small. A value outside `i32` altogether (or a float) isn't a valid value for this field at all, so it's a whole-file parse error (see Failure semantics above), not a clamp. |
| `cursor_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#ffffff` (white) | Fill color of the built-in pointer cursor. Its 1px outline is always black, at this color's own alpha, and isn't separately configurable — the outline exists to keep the shape's edges visible against similarly-colored content. That doesn't help against a *dark* `cursor_color`: with black (or any near-black) fill, the outline blends into it and the pointer can be hard to spot against dark window content. An alpha of `00` makes the built-in cursor invisible; that's your call, not a clamped value. |
| `cursor_theme` | string | unset | Which installed xcursor theme named cursor shapes are drawn from (see Cursor shapes above). Unset means follow `$XCURSOR_THEME`, then `default` — i.e. whatever the rest of the desktop uses; an empty string means the same as unset. This only *names* a theme, it never makes scoot ship one, and a name that matches nothing installed is not an error: named shapes then come from scoot's own drawn set, exactly as on a machine with no themes at all. |
| `prefer_no_csd` | boolean | `true` | Whether to answer a client's `zxdg_toplevel_decoration_v1` request with `ServerSide`, so a well-behaved client stops drawing its own titlebar (which would otherwise double up with the ring). |

`cursor_size` and `cursor_color` apply to scoot's own drawn shapes — the
fallback used when the machine has no cursor theme installed, drawn only
under `--tty` (`--headless` has no display and `--nested` already shows the
host's cursor). `cursor_size` also picks which size is taken out of a real
theme's file, and `cursor_color` has no effect there: a theme's artwork
brings its own colors. None of the three affects a client that supplies its
own cursor *image* (a spinner, say): those pixels come from the client over
the wire, and scoot draws them at the size and hotspot the client chose.
All three are read once at startup, like every other setting here — there's
no config reload.

The first three hex values above are the actual rendered colors (pixel-sampled
from a real screenshot, and pasting any of them back into the matching
config field reproduces the default exactly). Internally those three built-in
defaults are stored as raw RGBA floats (`0.42, 0.65, 0.98`,
`0.35, 0.35, 0.38`, and `0.08, 0.08, 0.1`, each `1.0` alpha), and none of
those floats is exactly representable as an 8-bit `"#rrggbb"` string — that's
a storage detail, not a reason to distrust the hex above. Leave a color
field unset to get the real built-in default; only set it to a hex string if
you want to *change* it. (`cursor_color`'s `#ffffff` is the one exception:
pure white *is* exactly representable, so writing it out changes nothing at
all.)

### `[output]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `scale` | float | `1.0` | Output scale advertised to clients and rendered at. `1.0` renders identically to no setting at all; anything else advertises `ceil(scale)` on `wl_output` and `wl_surface.preferred_buffer_scale`, and the exact value through `wp_fractional_scale_v1`/`wp_viewporter` (see Output scaling above). Clamped into `0.5..=4.0` with a warning, and a non-finite value falls back to `1.0`; startup-only. `--nested` ignores a non-1.0 value with a warning. |

### `[renderer]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `backend` | string (`"pixman"` or `"gles"`) | `"pixman"` | Which renderer composites each frame — the config-file form of `--renderer` (see Which renderer draws the frames above). This is a different axis from `--headless`/`--nested`/`--tty`, which choose how the compositor *presents* what it drew; this chooses what draws it. `"pixman"` is the CPU renderer and needs no graphics device at all. `"gles"` draws with GLES on an EGL device. Under `--headless`/`--nested` that means compositing offscreen and reading the frame back; under `--tty`, in a build carrying the `gpu-scanout` Cargo feature, it means GPU scanout with no read-back at all (and without that feature, `--tty` warns and keeps pixman). `--renderer` wins when both name one, including `--renderer pixman` against a file asking for `gles`. A name that is neither is a warning and the default, like any other malformed value; but a name this build *knows* and then cannot build (`"gles"` with no working EGL) is a startup error on `--headless`/`--nested`, because silently drawing with the other renderer would be a session quietly different from the one you asked for. Under `--tty` that same failure is a warning and a pixman session instead: refusing to start there would leave you with no desktop. Startup-only, like every other setting here. |

### `[tty]`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `gpu` | string (device path) | unset | Which DRM device `--tty` drives, when the automatic choice is wrong — the config-file form of `--gpu PATH` (see Which DRM device `--tty` drives above). Unset means the automatic search picks: Smithay's primary GPU first, then every other DRM device on the seat until one works. Set means exactly that device, no fallback: a wrong path is a clean startup error naming the key and what failed, not a silent fall back to something else — falling back would mean silently driving a device the config explicitly ruled out, so this key is fail-closed where every other config field degrades gracefully. `--gpu` wins when both name one; an empty value (`gpu = ""`) is a startup error naming the key. Only means anything under `--tty`; on `--headless` or `--nested` a set non-empty value is ignored with a warning, exactly like `--gpu` (an empty value is a startup error on every backend — see above). Startup-only, like every other setting here. |

### `[binds]`

A table of `"key combination" = "action string"`. A combo is
`modifier+modifier+...+key` (e.g. `"super+shift+h"`), or a bare key with no
modifier at all (e.g. `"Return" = "close"` — legal, and it intercepts every
press of that key with none of Super/Shift/Ctrl/Alt held; `Shift+Return`,
for instance, still reaches the focused client normally). Whitespace around
`+` is ignored. Modifier names are
case-insensitive: `ctrl`/`control`, `shift`, `alt`, and `super`/`logo`/
`meta`/`cmd` (all four spellings mean the same modifier — scoot's own
tables and this doc call it "Super"). The key is an xkb keysym name (`h`,
`Return`, `F5`, ...), resolved by trying the name exactly as written first
and then case-insensitively — so `"return"` and `"RETURN"` both find
`Return`. A single ASCII letter is folded to lowercase before that lookup,
so `"H"` and `"h"` name the same key (only single letters fold: `"OE"` and
`"oe"` are distinct keysyms and stay that way).

Binds are matched against a key's *unshifted* symbol, with Shift tracked as
an ordinary modifier, so **a capital letter names the unshifted key, not
Shift plus that key**: `"A"` means plain `a`, exactly like `"a"` — write
`"shift+a"` for the Shift chord. Folding a bare capital logs a warning naming
the bind (config loading warns rather than silently reinterpreting what was
written); with Shift named there is nothing ambiguous, so `"shift+A"` folds
quietly. Note `scoot msg key A` is a different story on purpose: it keeps
refusing, because pressing `A` with nothing held would type a different
character — name `shift+a` there, or use `msg type`.

Action strings use exactly the grammar `scoot --help`'s ACTIONS section
documents — one parser handles both `scoot msg action ...` and a config
file's `[binds]` values:

```
focus-column|move-column|consume-or-expel   left|right
focus-window|move-window                    up|down
focus-workspace|move-window-to-workspace    up|down
focus-window-id ID | focus-workspace-index N | cycle-column-width | close | spawn COMMAND... | quit
```

e.g. `"focus-column left"`, `"close"`, or `"spawn foot -e htop"` (split on
whitespace, not run through a shell, so a path or argument containing a
space can't be expressed this way).

Two failure behaviors specific to `[binds]`, both worth knowing since they
fail silently rather than as a startup error:

- **A bind that doesn't parse** (unknown modifier, unknown key name, an
  invalid action string, or trailing text after the action) is skipped with
  a warning naming just that bind; every other bind in the file still loads.
- **Two different combo strings that resolve to the same actual key
  combination** — different modifier aliases, different modifier order, or
  different case (`"Super+H"` vs `"super+h"`) — are **both** skipped, with
  one warning naming all of them. `[binds]` is read into a `HashMap`, whose
  iteration order has no relationship to the order the keys were written in
  the file, so "the last one wins" isn't something this code can honor
  truthfully; rather than pick an arbitrary, run-to-run-unstable winner, the
  whole colliding group is dropped and whatever was bound to that combo
  before the file loaded (a default, or nothing) is left in place.

A user bind on a combo that already has a default simply replaces it; there
is no "unbind" action.

### Default keybindings

| Combo | Action |
|---|---|
| `Super+h` | Focus column left |
| `Super+l` | Focus column right |
| `Super+j` | Focus window down |
| `Super+k` | Focus window up |
| `Super+Shift+h` | Move column left |
| `Super+Shift+l` | Move column right |
| `Super+Shift+j` | Move window down |
| `Super+Shift+k` | Move window up |
| `Super+Alt+h` | Consume or expel column left |
| `Super+Alt+l` | Consume or expel column right |
| `Super+Ctrl+j` | Focus workspace down |
| `Super+Ctrl+k` | Focus workspace up |
| `Super+Ctrl+Shift+j` | Move window to workspace down |
| `Super+Ctrl+Shift+k` | Move window to workspace up |
| `Super+r` | Cycle column width |
| `Super+q` | Close focused window |
| `Super+Return` | Spawn `foot` |
| `Super+Shift+e` | Quit |

That's all 18 default bindings — vim motions (`h`/`j`/`k`/`l`) for direction,
Super as scoot's own modifier throughout. Quit is deliberately
`Super+Shift+e`, not `Super+Shift+q`: that combo is one slipped Shift away
from `Super+q` (close focused window), and a slip of the finger shouldn't be
able to end the whole session.

`--tty` additionally binds `Ctrl+Alt+F1` through `Ctrl+Alt+F12` to switching
to VT 1 through 12 — not present under `--headless`/`--nested`, since VT
switching is a Linux-session concept with no meaning there. These are added
*after* the config file loads and always win over a colliding config-file
bind (logging a warning naming whatever they displaced): on real hardware,
with no other window manager and often no easy remote access, Ctrl+Alt+Fn is
the one recovery path if the display ever gets wedged, so it can't be
allowed to silently lose to a config-file typo or a well-meaning rebind.

### Example `config.toml`

```toml
[layout]
gap = 8
column_widths = [0.25, 0.5, 0.75, 1.0]
default_column_width = 1

[appearance]
focus_ring_width = 4
focus_ring_active_color = "#ffaa00"
focus_ring_inactive_color = "#333333"
background_color = "#101014"
cursor_size = 24
cursor_color = "#ffcc66"
# Unset follows $XCURSOR_THEME, then "default" -- i.e. the rest of the
# desktop. Name one here only to override that.
# cursor_theme = "Adwaita"
prefer_no_csd = true

[output]
# 1.0 is correct for a non-HiDPI display; raise it (e.g. 2.0) on a HiDPI
# panel, or text and widgets render far too small. See the reference above.
scale = 1.0

# [renderer]
# Unset means "pixman", the CPU renderer -- the right answer on a GPU-less
# box and the default everywhere. "gles" is opt-in. On --headless/--nested it
# buys correctness parity rather than speed (the frame is still read back, and
# on a software rasteriser it is slower); on --tty, in a build carrying the
# `gpu-scanout` Cargo feature, it is GPU scanout with no read-back at all.
# --renderer wins over this when both name one. See the reference above
# before switching.
# backend = "gles"

# [tty]
# Uncomment only on hardware where the automatic DRM device search picks
# wrong. Apple Silicon under Asahi Linux -- where the 3D GPU and the display
# controller are separate devices -- was the motivating case, but the search
# was confirmed correct there on 2026-09-18 and needs no key. Unset means the
# automatic search picks; --gpu PATH on the command line wins over this when
# both name one. Name the *display controller*, never the render node, and
# prefer a stable /dev/dri/by-path/... alias over a cardN minor number.
# gpu = "/dev/dri/by-path/platform-soc:display-subsystem-card"

[binds]
"super+n" = "focus-column right"
"super+shift+n" = "move-column right"
"super+t" = "spawn foot"
"super+shift+t" = "spawn foot -e htop"
"ctrl+alt+space" = "spawn wofi --show drun"
```

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
