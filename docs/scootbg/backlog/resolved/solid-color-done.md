---
title: "Solid colors through single-pixel buffers"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Solid colors through single-pixel buffers — RESOLVED

Resolved 2026-09-27. What landed, where it departs from the plan, what
was verified where (and what could not be), and the measurements are in
[Resolution](#resolution) at the end; the original ticket follows
unchanged but for the spelling ("color", decided 2026-09-27 for all of
scootbg).

`scootbg set '#rrggbb'` sets a whole output to one color.

- With `wp_single_pixel_buffer_manager_v1` and `wp_viewporter`: one
  single-pixel buffer, viewport destination set to the surface size. No
  shared memory at all.
- Without them: a 1×1 `wl_shm` buffer and `wp_viewporter`, or, with no
  viewporter either, a full-size buffer filled once. Check the 1×1 path's
  pixels, not just that it maps: on `scoot --headless` (pixman) a
  viewport-upscaled 1×1 `wl_shm` buffer rendered as a bilinear fade to
  transparent at every edge, not a flat color
  ([observed](dependencies-done.md#incidental-finding-a-11-wl_shm-buffer-upscaled-on-scoot)).
  That is a compositor bug; scoot itself offers single-pixel buffers, so
  scootbg never takes this path there.
- Opaque region set to the whole surface.
- scoot's direct-scanout path (`docs/tty.md`) treats a covering, fully
  opaque, **black** single-pixel wallpaper as the background, so a
  fullscreen window with an *alpha* format still scans out directly over
  it (`a_black_single_pixel_wallpaper_hands_the_primary_to_the_window_above_it`).
  An opaque-format window scans out over any wallpaper. Only the
  single-pixel path keeps the alpha case: the `wl_shm` fallback or a
  full-size buffer blocks it. Verify on the `--tty` GPU tier with a black
  `scootbg set '#000000'` and an alpha-format fullscreen client.

This is also the cheapest way to prove the daemon, the socket and output
handling end to end before any image code exists.

From [ticket 3](outputs-and-layer-surfaces-done.md#for-the-next-tickets):
each output's surface already exists and, once `Surface::Configured`, has
its `configure` acked; attach at `Output::surface_size()`, commit, redraw on
a later `configure`, and fill `query`'s `shows`. The screenshot check of
scoot's second output (`scootctl screenshot --output 2`), deferred from
there because nothing was drawn, belongs here.

## Resolution

### What landed

- **`scootbg set '#rrggbb' [--output NAME]`, `scootbg clear [--output NAME]`**
  (`src/cli.rs`, protocol 1 requests `set` and `clear`, additive). Colors
  parse strictly (`src/color.rs`): `#` and six hex digits in either case,
  nothing around them, no `#rgb`, no alpha. Anything not starting with `#`
  is refused with "images come in a later version" (exit 2, as is a
  malformed color); ticket 5 turns that into a path. `set` and `clear`
  print nothing on success; an unknown `--output` is exit 1 and changes
  nothing. `--output=NAME` works too.
- **What each output should show** (`src/choices.rs`): one choice for every
  output, and choices by connector name. An every-output `set` or `clear`
  replaces all of them, so every output, and every output plugged in
  later, shows it. A named choice is only accepted for an output present
  now, and is then kept *by name*, so a monitor unplugged and plugged back
  in shows it again. The table holds one entry per name the compositor has
  shown, so its size is bounded by hardware, not by requests.
- **Three paths** (`src/paint.rs`, `src/daemon/canvas.rs`), chosen once
  from the globals: (1) `wp_single_pixel_buffer_manager_v1` + `wp_viewporter`:
  one single-pixel buffer, channels `v * 0x01010101` (exact: 0 → 0,
  255 → `u32::MAX`, every step `v/255`), viewport destination the surface
  size, no shared memory; (2) a 1×1 `XRGB8888` `ShmBuffer` under the
  viewport; (3) a full-size `ShmBuffer`, the configured surface size times
  `wl_output`'s integer scale, with `wl_surface.set_buffer_scale`.
  Opaque region the whole surface on every path. A 1×1 buffer stays at
  buffer scale 1 (a larger scale would make its size a protocol error).
  scoot gets path 1, which is what its direct-scanout rule for a black
  single-pixel wallpaper needs (`docs/tty.md`).
- **Buffers** (`ShmBuffer`'s first real use): sealed memfd, `i32`-bounded,
  never written while attached. A change reuses a released buffer of the
  right size in place; a held one is never touched, a second is made
  instead, at most two per output; with both held, the draw waits for a
  `wl_buffer.release` (unit-tested; never seen on scoot or sway, which
  release the older one as soon as the newer is committed). A released
  buffer of a size no longer drawn is dropped at once. The `wl_shm_pool`
  is destroyed right after its one buffer is made. Path 1 makes one
  single-pixel buffer per color and destroys the old one after the commit
  that replaces it.
- **Redraws** (`src/daemon/change.rs::reconcile`, idempotent): on a
  `set`/`clear`, on every `configure` (the first, and resizes: a new
  viewport destination, or a new full-size buffer), one round trip after a
  `wl_output.done` (below), and on a release a stalled draw waited for.
  Every commit follows the ack of the latest `configure`, which is sent
  as it arrives. A mapped surface commits after an ack even when nothing
  changed, so the ack takes effect.
- **Replies after the compositor has the change** (`src/waiters.rs`,
  `src/control/conn.rs`). A `set`/`clear` answers `Answer::Later`: the
  connection is neither read nor polled for anything while it waits, and
  requests behind it on the same connection are answered after it, in
  order. Each request stamps a generation on the outputs it targets; it is
  resolved once no output with that stamp or later is still waiting to show
  its choice. Those resolved in one loop turn go in flight behind one
  `wl_display.sync`, sent after their commits, and its callback releases
  their replies. Nothing blocks, and nothing is allocated per request
  beyond what `wayland-client` needs to send its requests (a name for a
  new `--output` choice aside).
  - *An output unplugged mid-request* is gone from the list, so the reply
    is `ok` for the outputs that remained. If the removal reaches the
    daemon *before* the request does, the name is simply unknown: the
    documented error, nothing changed. A race between the two gets one or
    the other, never a hang
    (`an_output_unplugged_during_a_set_still_gets_a_reply`, ten rounds).
  - *An output whose surface is not configured yet* (just plugged in,
    re-created after `clear` or a `closed`) is waited for: it will be
    configured within a round trip or two. One that gave up is not: it
    shows nothing, `query` says `gave-up`, stderr said so when it happened,
    and the reply is still `ok`.
  - *A draw that fails* (a buffer too large for `wl_shm`, out of memory)
    makes the reply an error, after the others are drawn, with the reason
    on stderr; it is not retried until the output is reconfigured or
    targeted again, so it cannot loop.
  - *A client that hangs up* before its reply loses only the reply.
- **`query`'s `shows`**: `{"color":"#rrggbb"}` (lowercase) for what the
  surface was last committed with, or `null`. Additive; every key still
  always present.
- **Forcing paths 2 and 3 for tests**: `SCOOTBG_DEBUG_PATH=viewport-shm|full-shm`,
  read only in a debug build (`cfg(debug_assertions)`); a release build
  compiles the reading out, so the shipped binary has no knob. Chosen over
  a cargo feature because both test runners and CI build debug by default,
  so every run covers the fallbacks with no extra build, and there is no
  feature combination to forget; the tests skip the forced cases, saying
  so, in a build without debug assertions. It can only pick a *worse* path
  the globals allow.

### Departures from the plan, and why

- **`clear` replaces the surface instead of attaching a null buffer.** Both
  are protocol-correct, but a null attach *unmaps* a layer surface, which
  then "returns to the state it had right after get_layer_surface": the
  pinned Smithay fork resets every layer attribute to its default on it
  (`wlr_layer/mod.rs`, `*guard_layer.pending() = Default::default()`), and
  a default anchor with size 0×0 is a protocol error on the next commit. So
  the unmap route has to set the surface up again anyway, and then relies
  on re-mapping, the path compositors exercise least. Destroying the layer
  surface (viewport, role, `wl_surface`, in that order) and making a fresh
  one, committed with no buffer, is the path every compositor already
  takes at start-up. The fresh surface is configured again, so a later
  `set` draws on it at once.
- **No cap on waiting requests; a bound by construction instead.** The
  first version refused a `set` past 32 waiting or in flight. The flood
  test (100 clients sending `set` and hanging up at once) showed that
  refusing a *legitimate* `set` afterwards: the cap was counting replies no
  one could receive. Now waiters whose connection is gone are dropped at
  the start of every loop turn, and a connection has at most one request
  waiting (it is not read meanwhile), so the lists hold at most one entry
  per live connection plus one per connection evicted in the current turn,
  2 × 16, which is the capacity they are made with. No request is refused
  for being one too many.
- **A redraw after `wl_output.done` waits one round trip.** The fallback
  test's trace on scoot showed `done` (scale 2) arriving *before* the
  `configure` for the new size, so a redraw at `done` drew a 3200×2000
  buffer at the stale surface size and the new scale, only to replace it
  (on a 4K output, a transient ~132 MB). The redraw now happens when a
  `wl_display.sync` sent at the `done` comes back: any `configure` from the
  same change has been handled by then, and if none came (a mode and a
  scale doubled together keep the logical size), the round trip redraws.
- **A per-output choice is kept by name across unplugging.** The ticket
  asked for the every-output choice to reach new outputs, and for a
  per-output choice for a missing output to be an error; both hold. Keeping
  a per-output choice for a monitor that is unplugged and plugged back in
  is what the README always promised ("by connector name"), costs one
  table entry per name, and forgetting it on a replug (or a monitor that
  drops off the bus when it sleeps) would be a visible surprise.
- **The 1×1 `wl_shm` path is exact on scoot**, not the edge fade this
  ticket warned about: that was fixed in the scoot-sh/smithay fork before
  this landed ([resolved](../../../backlog/resolved/shm-viewport-upscale-edge-fade-done.md)),
  and the fallback test asserts every pixel from the client side.
- **`set` of the color already shown still waits** for every targeted output
  to show it, and still round-trips. That makes "`set` returned" mean
  "it is on screen" without exception, which the hotplug test uses.

### Found along the way

- **A test that claimed too much** (caught by the stress run, iteration 5
  of 20): the unplug race test asserted `ok` "whichever way the race
  goes", but when sway's removal reaches the daemon before the request,
  the correct answer is the unknown-output error, and that is what came
  back. The daemon was right; the test (and a line of this record) now
  accept exactly those two outcomes and run the race ten times per run.
- **A pipelined `set` could hang** (fixed before commit, pinned by
  `a_set_pipelined_behind_a_set_is_answered`). The loop resolved waiters
  and *then* delivered replies; a delivered reply lets its connection
  handle the next request, and a `set` there that changed nothing sent the
  compositor nothing, so no event came back to wake the loop, and its
  reply waited for an unrelated event. The test first passed by accident,
  woken by the `delete_id` of the destroyed opaque-region object, until it
  was made to repeat the same color; then it timed out, and passes since
  the loop delivers first and resolves after.
- **The test harness now scrubs `WAYLAND_DEBUG` and `SCOOTBG_DEBUG_PATH`**
  from what it hands the daemon, so a developer's environment cannot
  change which path a test exercises; a test that wants either sets it.
- **Each shm buffer keeps its memfd open** (`ShmBuffer` owns the fd): +1 fd
  per buffer on paths 2 and 3, so +4 on two outputs after a change. Left
  for [memory-and-idle.md](../memory-and-idle.md): the fd is only needed
  for `create_pool`, and dropping it would be a change to `scootbg-mem`.
- **Path 3 keeps a released spare per output** after a change, as asked
  (reuse without reallocating): idle PSS 7.8 MB with one full-size buffer
  per 1600×1000 output, 14.1 MB with the spare. Only a compositor with no
  `wp_viewporter` takes that path; whether idle memory should win over
  reallocating on the next change is for memory-and-idle.md, measured.

### Verified where

All on a Claude Code web container (x86_64, 4 CPUs), no dev VM. Code at
`5179c09`; the docs came after, in a docs-only commit. The
screenshot and trace checks below were first run on `72e59a6`, and again
on the final code by the suite and the stress run.

- **scoot `--headless`, 1 and 2 outputs** (`tests/color.rs`): after `set`,
  scoot's own screenshot of each output (`output` 1 and 2 over its IPC,
  what `scootctl screenshot --output N` sends), 1600×1000, is one flat
  color, every pixel (not only the centre, corners and edge midpoints,
  which the failure message names); the trace shows one
  `create_u32_rgba_buffer(3233857728, 808464432, 538976288, 4294967295)`
  per output for `#c03020`, `set_destination(1600, 1000)`, an opaque region
  `add(0, 0, 1600, 1000)`, no `create_pool`, no `set_buffer_scale`, no
  `frame`; and no context switch over 1.5 s afterwards. `--output` changes
  one output only; `clear` shows scoot's own background again (sampled
  before any `set`); an unknown name is exit 1 and changes no pixel;
  `clear` then `set` works; `query` agrees at each step.
- **Never stale**: 30 rounds of six colors, both outputs screenshotted the
  moment `set` returns: never the old color.
- **Pipelining, hang-ups**: `set`, `query`, `version` in one write answer in
  order with the `query` already showing the color; `set`, same `set`,
  `clear`, `query` too; 100 clients sending `set` and closing at once, then
  a normal `set`: served, and the screen shows the last color.
- **Paths 2 and 3 forced on scoot**: six color changes each, every pixel
  exact each time, at most two buffers made (the trace shows two alternating
  `wl_buffer`s, released and refilled in place: `create_buffer(…, 1, 1, 4, 1)`
  or `(…, 1600, 1000, 6400, 1)`, format 1 = XRGB8888); after
  `scale = 2.0` through `scootctl reload`, exact again at 1600×1000
  physical, path 3 with one new 1600×1000 buffer at `set_buffer_scale(2)`
  and none larger. By hand, at `scale = 1.5`: path 1 sets the destination
  to 1067×667 and path 3 draws 2134×1334 at scale 2; both screenshots are
  1,600,000 pixels of `#c03020`.
- **sway 1.12 headless** (the pinned nixpkgs, pixman), read back through a
  wlr-screencopy client in the test harness, every path: exact after `set`;
  a new output (`create_output`) shows the every-output color once
  configured; `--output` on it; scale 2 redraws exact; a replacement output
  after an unplug gets the every-output color; `clear`; idle afterwards.
  sway uses path 1 by itself. An output unplugged right as a
  `set --output` for it is sent, ten rounds: that reply is `ok` or the
  unknown-output error, the every-output `set` pipelined behind it is
  always `ok`, and the remaining output is exact.
- **Unit**: 143 tests in the binary (colors: valid, uppercase, whitespace,
  shorthand, alpha, non-ASCII; channel scaling exact for all 256 values,
  both ways a compositor converts back; path choice and forcing; buffer
  choice over every slot combination; the model's plan and progress
  through configure, resize, scale, clear, close, give-up, failure; the
  waiters' generations, shared syncs, forgetting, and no reallocation;
  deferred replies on a connection: ordering, a hang-up while waiting, a
  half-closed client, delivery by id after eviction).
- **Commands**, at `5179c09`: `cargo nextest run --workspace` (2,218
  passed, 24 skipped, all of them the compositor's `#[ignore]`d tests) and
  `cargo test --workspace` (2,218 passed), both under `soft-egl`, with
  `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1` and
  `SCOOTBG_TEST_SWAY` from `nix build --inputs-from . nixpkgs#sway`;
  `cargo clippy -p scoot -p scootbg -p scootbg-mem --all-targets -- -D warnings`;
  `cargo fmt --all --check`; `actionlint` 1.7.12 on `ci.yml` reports one
  finding, SC2174 at line 180 (`mkdir -p -m 700`), identical on `main` and
  in a step this change does not touch.
- **No knob in a release build**: the Nix package (`nix build .#scootbg`,
  sandboxed) run with `SCOOTBG_DEBUG_PATH=full-shm` against headless scoot
  still made a single-pixel buffer and no shm pool, printed no debug line,
  and `strings` finds no `SCOOTBG_DEBUG_PATH` in the binary.
- **Nix**: `nix flake check` passes (sandbox on); `nix build .#scootbg`
  builds (779,624 B). In this container the sandboxed fetch of the
  smithay fork needed the agent proxy's CA
  (`NIX_GIT_SSL_CAINFO=/root/.ccr/ca-bundle.crt` and that one file in
  `extra-sandbox-paths`), an environment detail, not the flake's.
- **Stress**, at `5179c09`, all of `-p scootbg -p scootbg-mem` (188 tests),
  scoot and sway required: `cargo nextest run --stress-count 20 -j 16`
  20/20 idle (555 s) and 20/20 under eight busy loops (load average ~10 on
  4 CPUs, 1,344 s); `cargo test -- --test-threads 16` ×20, 20/20 idle and
  20/20 under the same load. An earlier stress run, at `72e59a6`, is what
  caught the over-claiming unplug test (above); it was stopped there, the
  test fixed, and every configuration run again from the start.
- **The unplug race, both ways**: idle, 50 rounds out of 50 went the `ok`
  way (the request taken before the removal); the unknown-output way was
  hit under the first stress run. Neither integration run can force the
  third order, the removal arriving while the reply is still waiting for
  the draw (a single-pixel draw resolves within the same turn); that case
  is the waiters' unit tests (an output gone from the list no longer holds
  a generation back).

### Review of PR #278

No blocking findings. Fixed:

- **A release-only warning**: `Path::name` and `Path::from_name` are used
  only by the debug knob, so `cargo build --release -p scootbg` warned they
  were unused. They (and their test) are now `cfg(debug_assertions)`, and
  CI's scootbg release build runs with `RUSTFLAGS="-D warnings"` so this
  class fails CI (lint levels only; the size step measures that same
  build).
- **`set` returning overstated**: an output that gave up is left out of the
  wait and shows nothing, while `set` exits 0. The README, `set --help`
  and the protocol notes now say so.
- **"An error reply changes nothing"** was contradicted by the next
  sentence (a failed draw is an error after the other outputs changed);
  reworded.
- **Opaque region, viewport destination and buffer scale are sent only for
  a new surface or a new size or scale** (`LayerObjects::sized`), not on
  every draw: they are persistent double-buffered state, and the opaque
  region cost a `wl_region` create and destroy per draw. Pinned in
  `a_color_covers_every_output_exactly_on_scoot`: two more color changes
  send neither again; `clear` then `set` sends each once more per output;
  a resize to 800×500 sends `set_destination(800, 500)` and
  `add(0, 0, 800, 500)` once per output, and every pixel stays exact.
  With the check removed the test fails (`left: 3, right: 1`).
- **A held-up reply** (an output that never configures, a buffer never
  released) is recorded in [memory-and-idle.md](../memory-and-idle.md).

Verified at `6b0fe92`, the review fixes' code: `cargo build --release -p
scootbg` and the Nix build's log show no warning, and the release binary
is 783,072 B, the same with or without `RUSTFLAGS="-D warnings"`;
`cargo nextest run --workspace` 2,218 passed (24 skipped, the compositor's
`#[ignore]`s), `cargo test --workspace` 2,218 passed and
`cargo test -p scootbg -p scootbg-mem` 188 passed, with scoot and sway
required; clippy (debug and `--release`) and fmt clean; `tests/color.rs`
×10 under eight busy loops (load average ~9.8 on 4 CPUs), 10/10. actionlint
reports only the SC2174 at line 180 that `main` has too.

### Not verified, and why

- **Direct scanout over a black single-pixel wallpaper** on the `--tty` GPU
  tier (the ticket's `scootbg set '#000000'` with an alpha-format
  fullscreen client): no dev VM or GPU is reachable from this container.
  What is verified is the precondition on scoot: path 1, one opaque
  single-pixel buffer (`a = u32::MAX`) with the viewport destination the
  full output and the whole surface opaque.
- **A compositor with no single-pixel buffers or no viewporter**: none was
  available; paths 2 and 3 were forced on compositors that have both.
- **A stalled draw** (both shm buffers held when a change comes): neither
  compositor holds the older buffer long enough to reach it; the choice is
  unit-tested, and the redraw on release is by reading.
- **The CI change** (`--test color` added to the integration job): runs
  first on this PR.
- **`--tty` in general, and macOS `cargo check`**: as for the earlier
  tickets.

### Measurements

Release (`cargo build --release -p scootbg`) at `5179c09`; before is
`3b2722f` (main), built from a separate worktree and target dir; against
`scoot --headless --outputs 2` (a debug build, 1600×1000 each), measured
by a Python client on the control socket and `/proc`, three rounds
interleaved (before, after without a color, after with one). An earlier
run of every row on `72e59a6` gave the same memory numbers and latencies
within a few percent.

| What | Before | After |
|---|---|---|
| Stripped binary (`cargo build --release`; the Nix package is 779,624 B) | 733,920 B | 783,072 B (+49,152). Symbols +43.6 KB: `reconcile` with the canvas drawing inlined 14.6 KB, `LayerObjects::create` now out of line 5.2 KB, the request handler 3.7 KB, the `set`/`clear` parser 3.4 KB, the rest Wayland dispatch and request code instantiated for `wl_buffer`, `wl_shm_pool`, `wp_viewport` and `wl_region` |
| `ldd` | `libgcc_s.so.1`, `libc.so.6` | `libgcc_s.so.1`, `libc.so.6` (unchanged) |
| `libc` crate in the normal tree | none | none (CI's check, `cargo tree -p scootbg -e normal --prefix none \| grep '^libc '`, finds 0 lines); `-sys`: `linux-raw-sys`, `wayland-sys` with no features (unchanged). The new dev-dependencies (`png`, `base64`) reach only the tests |
| Idle, no color, 3 s after start: RSS / PSS / `[heap]` Rss / threads / fds | 2,636 / 1,440 / 32 / 1 / 8; 2,628 / 1,432 / 32 / 1 / 8; 2,616 / 1,420 / 32 / 1 / 8 | 2,720 / 1,524 / 36 / 1 / 8, ×3 identical |
| Idle **with a color set** (path 1), 3 s after the `set` | n/a | 2,720 / 1,524 / 36 / 1 / 8, ×3 identical |
| Context switches, CPU ticks over 30 s, ×3 | 0, 0 (no color) | 0, 0 without a color; 0, 0 with one |
| First `set` after start, request to reply | n/a | 958 / 761 / 718 µs |
| 10,000 `set`s alternating four colors on one connection, ×3: median / p99 / max | n/a | 404.4 / 1,607.0 / 3,260.6 µs; 432.8 / 1,614.6 / 4,631.8 µs; 400.3 / 1,581.2 / 3,264.3 µs |
| Memory after those 10,000 (3 s later) | n/a | 2,720 / 1,524 / 36 kB / 8 fds, each round equal to its own start; 0 switches over the next 5 s |
| Per path, 2 outputs, a color set, ×3 (a release build with `debug-assertions` on, the only way to force a path in an optimised binary): PSS / RSS | n/a | single-pixel 1,556–1,560 / 2,752–2,756 kB, 8 fds; 1×1 shm 1,556–1,560 / 2,756–2,760 kB, 10 fds; full-size shm 7,807–7,808 / 15,252–15,256 kB, 10 fds, and 14,059–14,060 / 27,756–27,760 kB, 12 fds, after 20 changes (a released spare per output) |
| `set` median over 20 changes, per path | n/a | single-pixel 535–607 µs; 1×1 shm 399–467 µs; full-size shm 3,003–3,167 µs (6.4 MB filled per output per change) |

The `set` latency is a compositor round trip on top of the daemon's work,
against a *debug* scoot: `query`, which needs no round trip, took
27–38 µs median in ticket 3's record. "Heap" is the `[heap]` mapping's
resident pages; the +4 kB is the waiters' and the ready list's capacity,
made once. The fds are stdio, the lock, the listener, its spare and the
Wayland socket (7, checked by listing `/proc/PID/fd` under a plain
parent), plus the measuring client's own connection, which stayed open
(Python's `makefile` holds the socket past `close`), the same for both
builds; each shm buffer adds its memfd.
