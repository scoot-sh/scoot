---
title: "Buffers, memory and zero idle cost"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Buffers, memory and zero idle cost — RESOLVED

Resolved 2026-09-27. What landed, where it departs from the plan, the
measured budget with its method and every raw number, and what could not
be verified are in [Resolution](#resolution) at the end; the original
ticket follows unchanged.

The resource budget is the feature. Targets to measure and publish in
`docs/scootbg/README.md` once the static path exists:

- **Idle:** no wakeups at all with a static wallpaper (no timers, no frame
  callbacks requested). Verify with `perf stat` / wakeup counts over a
  minute, the way `docs/benchmarks.md` measures the compositor.
- **Buffers:** one `XRGB8888` buffer per output in a `memfd` pool, opaque
  region set, reused in place on a same-size redraw. A 4K output is ~33 MB;
  two 4K outputs showing one image share nothing but the source decode.
- **RSS:** record resident memory for 1× 1080p, 1× 4K and 2× 4K, after the
  source has been dropped.
- **Startup:** time from `scootbg daemon` to the first committed buffer
  with a restored 4K JPEG.

No `release` event handling tricks until measured: a static wallpaper
commits once, so double buffering only matters for transitions.

**A reply can be held up** (from [solid-color-done.md](resolved/solid-color-done.md)):
an every-output `set` or `clear` waits for every output whose surface is
live, so an output whose surface never gets its `configure`, or a stalled
shm draw whose held buffer is never released, holds up the replies to
that request and later every-output ones until it resolves. Bounded by the
client's 30 s timeout (and the waiting lists by the connection limit), and
not reached on scoot or sway, whose surfaces configure within a round trip
and which release a buffer once the next is committed. If a compositor is
found that does, a per-output bound on the wait (or leaving outputs that
have never configured out of it) belongs here.

**From [ticket 6](resolved/images-decode-and-fit-done.md#measurements):**
after an image `set` on a 3840×2160 output (release, 12 sets of 6000×4000
JPEG, PNG and WebP): one 32.4 MB shm buffer per output (the old one is
dropped on release: a released buffer holding an image is never kept as
a spare), heap 372–568 kB anonymous, one thread, 0 wakeups over 30 s.
A rendered image no longer wanted (the choice changed before it could be
shown) is dropped at once.
Left for here:

- Two outputs of the same size showing one image get two buffers with
  the same pixels (the scale is done once, the second is a copy). One
  `wl_buffer` attached to both would save a buffer per extra output
  (32.4 MB at 4K); it needs release tracking per attach.
- A rendered image waiting for a held buffer's release (a stalled draw)
  is kept, one per output at most, beside the two slots.
- Each shm buffer keeps its memfd open (from ticket 4): +1 fd per image
  buffer.

## Resolution

### What landed

Code at `a581ffd` (`25dba06`, then `a581ffd`, which drops the spare
buffer); the docs, `set --help` and doc comments in the commit after.

- **Outputs of one size showing one image share its pixels**
  (`src/share.rs`, `daemon::canvas`, `daemon::images`, `daemon::worker`).
  The worker draws each distinct size once and hands back one buffer per
  size, not a copy per extra output. The loop wraps it for the compositor
  once, as `canvas::Pixels`: the mapping, and one `wl_shm_pool` over its
  memfd, kept (with no fd) for as long as the pixels are. Each output
  gets its own `wl_buffer` from that pool, attached and released on its
  own; the pool goes with the last share. 2× 4K showing one image: 36.7
  MB RSS where it was 69.2 MB ([below](#memory)).
- **Not one `wl_buffer` on every surface, and no count of attaches.**
  `wl_surface.attach` (wayland.xml, the copy in the pinned wayland-rs):
  "If a pending wl_buffer has been committed to more than one
  wl_surface, the delivery of wl_buffer.release events becomes
  undefined. A well behaved client should not rely on wl_buffer.release
  events in this case", and it suggests "creating multiple wl_buffer
  objects from the same backing storage" instead, which is what this
  does. Counting attaches against releases would not have saved one
  buffer either: Smithay (`RendererSurfaceState::update_buffer`, pinned
  fork `74edbf3`, line 168) keeps its wrapper, and sends no release, when
  the buffer attached is the one it already has, which scootbg does for
  every scale-only change. A count would never come back to zero, and
  the canvas would stall for good with both slots "held". Each buffer
  keeps the plain held flag it had, which is right on both compositors.
- **When shared pixels may be written** (`share::Slot::memory_mut`, the
  only way to them, unit-tested): this output's buffer released, no other
  `Rc` on the pixels (another output's buffer, held or free, or an image
  rendered for an output and waiting there), and no buffer over them ever
  dropped while still held. That last happens only when its surface is
  destroyed (`clear`, an output unplugged, a surface given up on), and
  the compositor may still be reading the pages for a frame it had
  started, so such pixels are frozen: shown again as they are, never
  written. Only a color on the full-size path ever writes into a buffer
  after it was first drawn, so the rule costs an image nothing.
  `paint::pick` now tells a free buffer (may be dropped) from a writable
  one (may be refilled): a free buffer whose pixels another output shows
  is replaced, never reused.
- **An output plugged in later, or a surface made again, shares pixels
  already drawn**: before the worker starts a render job, each target an
  output already has that image at that size for (on screen, kept, or
  waiting) is served from those pixels (`images::share`,
  `Jobs::satisfy`), with no decode. On sway, an output plugged in beside
  one showing the image gets a third buffer over the same pool and no new
  one (`tests/share.rs`); with this step removed it gets a new pool, a
  second decode, and the test fails (`left: [2, 1]`, `right: [3]`).
- **The memfd is closed as soon as its pool exists** (`ShmBuffer::close_fd`
  in `scootbg-mem`): `wayland-backend` `dup`s every fd it queues
  (`rs/wire.rs:126`), and the mapping keeps the memory on this side. No
  fd per buffer any more: 8 fds with any wallpaper, as with none (9 with
  one image buffer before, 10 with two). The seals stay with the file, so
  no one else holding it can shrink it under the mapping.
- **`scootbg-mem`'s `Attached` type state is gone.** One value per buffer
  cannot say that several buffers share pages; the rule it enforced
  moved to `share`, and `shm.rs`'s docs say where.
- **No spare buffer at rest.** A buffer released while another is on
  screen is dropped, whatever its size or content. It used to be kept on
  the shm color paths for the next change to reuse, which on the
  full-size path (a compositor with no `wp_viewporter`) held a second
  full-size buffer per output for as long as the wallpaper stayed up.
  [solid-color-done.md](solid-color-done.md) left that choice here. The
  cost is an allocation on the next color change on that path, 2–13 ms
  ([below](#the-spare-buffer)); the saving is a whole output's pixels per
  output. A buffer released because its surface went is still kept, so a
  re-created surface shows its image again without a decode.
- **A surface the compositor never configures no longer holds up
  replies** (the ticket's "a reply can be held up"). After each surface is
  made and committed with no buffer, a `wl_display.sync` goes out
  (`RoundTrip::Created`, numbered by `Output::creation` so a round trip
  about an older surface is ignored). A compositor answers that first
  commit with a `configure`, and scoot and sway both send it before the
  sync's reply. A surface still unconfigured when the reply comes is
  *late* (`Output::unanswered`, said once on stderr): replies stop waiting
  for it, and it is drawn when its `configure` comes. That bounds the
  wait at one round trip with no timer, costs one sync per surface made
  (start-up, hotplug, `clear`, a retry), and is never reached on scoot or
  sway: the whole suite passes unchanged on both, and `tests/share.rs`
  fails if the stderr line ever appears there.
- **A rendered image waiting for a held buffer's release** (the ticket's
  last item) is bounded as it was: `Canvas::ready` holds at most one,
  now often the same pixels as other outputs' (an `Rc`, not a copy), and
  it is dropped by any draw of something else, by `clear`, and at every
  `reconcile` where the output no longer wants that image. Neither
  compositor here stalls an image draw, so it is never reached in the
  tests.
- **Tests**: `share::tests` (8: one output, shared pixels under every
  combination of held and released, writable once the other output
  released and moved on, frozen after a drop under a held buffer, a
  waiting render blocking writes, the Smithay re-attach, drops in every
  order); `paint::tests` (a shared free buffer is replaced, never reused;
  every combination of held, free and shared); `jobs::tests` (targets
  served from shared pixels, never a trial, never the running job);
  `outputs::paint_tests` (the late surface: its own round trip only, a
  new surface each time, a `clear` unaffected, a failed draw still
  failed); `scootbg-mem` (closing the fd keeps the pages, frees the
  descriptor, keeps the seals); and end to end, `tests/share.rs` (3, in
  CI's integration job): one pool with a buffer per output and one
  mapping for two outputs on scoot, split by a per-output image and
  shared again, by screenshot; the full-size color path over shared
  pixels, three rounds, by screenshot on both outputs, one buffer each at
  rest; on sway, sharing through unplugging one output and plugging one
  in, and a per-output split. `tests/color.rs`'s fallback-path test now
  checks one buffer at rest instead of reuse of a spare.

### Departures from the plan, and why

- **One `wl_buffer` per output over shared storage, not one `wl_buffer`
  attached to both** (the ticket's wording): the protocol leaves releases
  of the latter undefined (above), and the memory, which is the saving,
  is the same.
- **No release counting per attach**, which the ticket suggested: with a
  buffer per output nothing needs counting, and a count would stall on
  Smithay (above).
- **Beyond the ticket: sharing with an output plugged in later**, and a
  surface made again. It is the same saving on a path the ticket's own
  checks exercise (hotplug), it also saves the decode (hundreds of ms of
  CPU at 4K), and it is one loop over the waiting renders at the one
  place every render passes through.
- **Beyond the ticket: the spare buffer**, a decision [ticket
  4](solid-color-done.md) handed to this one "measured".
- **Startup is measured without restore**: restore is
  [ticket 9](../restore-state.md). Measured instead: to the first
  answer, to a color on screen, and to a 4K image on screen, each asked
  for the moment the socket takes it. The image case found a double
  decode (below), left for ticket 9, whose restore is exactly that
  request.
- **A stalled shm draw is not bounded.** A draw stalls only while the
  compositor holds both of an output's buffers, and every compositor
  releases the older one once the newer is committed (Smithay at the
  replacing commit, wlroots once uploaded); none here ever stalled. A
  bound would have to either answer `ok` for something not on screen or
  allocate a third buffer, and nothing has shown it is needed. The
  client's 30 s timeout still bounds what a client waits.

### Found along the way

- **An image `set` sent before an output's surface is configured decodes
  the file twice**: the trial has no configured output to draw for, so it
  decodes only to validate the file; the `configure` then asks for a
  render, which decodes it again. Counted with `strace -f -e
  trace=%file,clone,clone3` on the daemon: 2 opens of the file for a
  `set` sent while starting, 1 for one sent after the `configure`, on
  both binaries. It costs 642–720 ms to the first image against about
  440 ms, and it is what a restore at login will do every time; recorded
  in [restore-state.md](../restore-state.md). (rustix opens files with
  `open(2)` on x86_64, not `openat`, which a first count missed.)
- The pool object ids in a `WAYLAND_DEBUG` trace are reused once a pool
  is destroyed, so `tests/share.rs` counts buffers against the latest
  pool with the id, not by id alone (the first version of the test
  counted two pools' buffers as one pool's four).

### Measurements

**Setup.** A Claude Code web container, x86_64, 4 CPUs, 15 GB, no GPU,
running as root. Release builds of scootbg, stripped, measured side by
side: **before** is `origin/main` at `2f822a2` (built in its own worktree
and `CARGO_TARGET_DIR`, sha256 `cbaefb85…`, 1,512,296 B), **after** is
`25dba06` (`ff454b46…`, 1,516,392 B), and **final** is `a581ffd` with
this commit's doc comments and help text (`20f468de…`, 1,516,392 B; the
spare change touches only the shm color paths, which no image or
single-pixel row below takes). The compositor is a debug build of
`scoot --headless --width W --height H --outputs N` with an empty config
(scale 1). The image is a 6000×4000 JPEG from the earlier records'
recipe (`magick -seed 1 -size 6000x4000 plasma:fractal -attenuate 0.5
+noise Gaussian -quality 92 -sampling-factor 4:2:0`, 8,851,735 B), set
with the default `fill`. Requests go over the socket from a Python
client. The scripts (`lib.py`, `mem.py`, `idle.py`, `control.py`,
`startup.py`, `trace.py`, `opens.py`, `spare.py`, `alloc.py`) and their
raw JSON lines are in the session's scratch record, not the repository.

#### Idle

60 s windows, after a `set` and once the daemon has held still (the same
context-switch count 1 s apart, one thread): `perf stat -e
task-clock,context-switches,cpu-migrations,page-faults,sched:sched_wakeup
-p PID -- sleep 60` (`perf` 7.2.8 from nixpkgs; the devenv shell has
none), and `/proc/PID/status` context switches and `/proc/PID/stat`
`utime + stime` read around the same window. One 3840×2160 output. Base
and after (or final) run side by side, one daemon each.

| | color `#1e1e2e` | the JPEG |
|---|---|---|
| perf, every event, each of 6 windows (base ×3, after ×2, final ×1) | `<not counted>` | `<not counted>` |
| `/proc` context switches, CPU ticks, each window | 0, 0 | 0, 0 |
| RSS / PSS kB at the end, base | 3,728, 3,684, 3,740 / 2,114, 2,064, 2,103 | 36,796, 36,748, 36,788 / 18,382, 18,300, 18,862 |
| RSS / PSS kB at the end, after, final | 3,612, 3,700; 3,660 / 1,991, 2,062; 2,039 | 36,744, 36,820; 36,828 / 18,345, 18,363; 18,912 |
| fds, threads | 8, 1 (both) | 9 before, 8 after; 1 |

`<not counted>` is perf's word for an event whose task never ran in the
window, that is 0. The positive control (`control.py`): the same perf
command around a 3 s window in which the final daemon answers 10
`query` requests counts `task-clock=2.78` ms, `context-switches=20`,
`page-faults=1`, `cpu-migrations=0`, `sched:sched_wakeup=0` (it counts
wakeups the task *makes*; the switches are the ones it received), and
`/proc` says 20 switches too. So the idle daemon, with a color or with
an image, makes no system call and is never woken in 60 s.

#### Memory

After a `set` of the JPEG, once idle (1 s still, one thread); peak is
`VmHWM` reset before the `set` (`/proc/PID/clear_refs` 5). The first
`set` of a daemon draws over nothing; the second holds the first's
buffer while it draws (the peak a change costs). Base and after 3
rounds, interleaved (before any `set`: RSS 3,536–3,776 kB, PSS
2,471–2,711 kB, `RssAnon` 196–204 kB, 8 fds, for both); final 2 rounds.
All numbers kB. `RssShmem` is the wallpaper's pages mapped by the
daemon; PSS halves them because the compositor maps them too.

| Outputs | | RSS | PSS | `RssAnon` | `RssShmem` | `[heap]` | fds | memfd maps / fds open |
|---|---|---|---|---|---|---|---|---|
| 1× 1920×1080 | base | 12,300, 12,404, 12,264 | 7,193, 7,281, 7,149 | 456 | 8,100 | 40 | 9 | 1 / 1 |
| | after | 12,320, 12,336, 12,328 | 7,221, 7,207, 7,203 | 456–460 | 8,100 | 40 | 8 | 1 / 0 |
| | final | 12,372, 12,280 | 7,281, 7,155 | 456 | 8,100 | 40 | 8 | 1 / 0 |
| 1× 3840×2160 | base | 36,688–36,804 | 19,417–19,525 | 564–572 | 32,400 | 40 | 9 | 1 / 1 |
| | after | 36,760–36,808 | 19,501–19,535 | 564–572 | 32,400 | 40 | 8 | 1 / 0 |
| | final | 36,672–36,740 | 19,405–19,465 | 564–568 | 32,400 | 40 | 8 | 1 / 0 |
| 2× 3840×2160 | base | 69,184–69,244 | 35,711–35,773 | 564–568 | **64,800** | 40 | 10 | 2 / 2 |
| | after | 36,808–36,836 | 19,531–19,571 | 564–572 | **32,400** | 40 | 8 | 1 / 0 |
| | final | 36,660–36,836 | 19,387–19,579 | 564–572 | **32,400** | 40 | 8 | 1 / 0 |

Every row: one thread. (Ranges are the 3 rounds × both `set`s, which
agree within 4 kB of each other on every row.)

The `set` itself, request to reply, CPU (`utime + stime` of the whole
process, worker included, in 10 ms ticks) and peak:

| Outputs | | reply ms, first `set` / second | CPU ms | peak kB, first / second |
|---|---|---|---|---|
| 1× 1080p | base | 341.6, 335.7, 395.3 / 351.5, 337.0, 342.5 | 330–380 | 74,636–74,840 / 83,128–83,332 |
| | after | 368.1, 351.1, 358.6 / 346.4, 383.2, 394.1 | 340–390 | 74,752–74,772 / 83,244–83,264 |
| | final | 382.1, 372.6 / 367.1, 393.6 | 360–370 | 74,664–74,700 / 83,200–83,236 |
| 1× 4K | base | 488.4, 445.9, 431.1 / 454.1, 430.8, 429.7 | 410–460 | 87,904–88,016 / 120,420–120,552 |
| | after | 427.5, 442.2, 452.2 / 431.7, 422.2, 439.0 | 410–440 | 87,952–88,080 / 120,528–120,656 |
| | final | 421.6, 458.5 / 426.6, 436.2 | 410–440 | 87,852–87,876 / 120,368–120,452 |
| 2× 4K | base | 448.7, 437.5, 472.3 / 443.9, 428.7, 464.4 | 420–460 | 87,992–88,088 / **152,892–153,048** |
| | after | 437.0, 462.2, 447.2 / 440.2, 423.0, 440.3 | 420–450 | 88,028–88,088 / **120,544–120,664** |
| | final | 435.1, 478.4 / 427.8, 464.9 | 420–470 | 87,872–87,980 / 120,448–120,496 |

So on 2× 4K the saving is one whole buffer, 32,400 kB of the daemon's
RSS (and of the compositor's: one pool, mapped once, where there were
two), and 32.4 MB off the peak of every change after the first. The
copy the worker used to make for the second output is gone too; its
time is within the runs' spread. One output's rows are unchanged but
for the fd. On a first `set` the peak is the decode (source plus scaled
image, before either buffer exists), the same with one output or two.

#### Startup

From `Popen` of `scootbg daemon` (on one 3840×2160 output) to: its
first answer (`version`); the reply to a `set` of a color sent the
moment the socket takes a connection; the same for the JPEG. A `set`'s
reply comes after its commit and a `wl_display.sync`, so it bounds "the
first committed buffer" from above. CPU is the whole process's at the
reply.

| | to first answer, ms | to a color on screen, ms | to the JPEG on screen, ms (CPU ms) |
|---|---|---|---|
| base ×5 | 1.9, 2.5, 2.0, 2.1, 2.1 | 3.1, 4.1, 3.0, 3.2, 3.8 | 655.9, 650.5, 661.6, 652.6, 707.0 (630–690) |
| after ×5 | 1.8, 1.9, 1.9, 1.8, 2.4 | 3.0, 3.5, 3.2, 3.1, 3.8 | 720.2, 666.7, 642.1, 680.1, 674.2 (630–700) |
| final ×3 | 2.2, 2.2, 2.1 | 3.0, 3.3, 3.5 | 660.4, 675.3, 689.5 (640–670) |

Cross-checked in a `WAYLAND_DEBUG=client` trace (`trace.py`; wall-clock
microseconds, as `wayland-backend` prints them): exec to the first
`wl_surface.commit` after an `attach`, color 4.2 ms (base) and 4.5 ms
(after), the reply 0.5 and 1.1 ms later; the JPEG 662.3 and 732.6 ms,
the reply 1.4 ms later (the trace slows the daemon by what it prints).
The JPEG's 650–720 ms is two decodes (the double decode
[above](#found-along-the-way)): the same `set` on a running,
configured daemon takes 421–488 ms. Ticket 9 measures again with a
restored JPEG.

#### The spare buffer

The full-size color path (`SCOOTBG_DEBUG_PATH=full-shm`, so **debug**
builds: the knob is compiled out of release; base `7830ff4b…`, final
`527d7f5d…`), two 1600×1000 outputs, three color changes, 3 rounds,
read once idle after each:

| | after change 1 | after changes 2 and 3 |
|---|---|---|
| base: RSS kB, memfd mappings, fds | 18,384–18,624, 2, 10 | **30,888–31,128, 4**, 12 |
| final | 18,468–18,496, 2, 8 | **18,468–18,496, 2**, 8 |

(PSS 11,065–11,285 kB then 17,317–17,537 kB for base; 11,139–11,179
kB throughout for final.) The debug builds' reply times, 82–111 ms, are
their unoptimized fill loop's and say nothing about release. What the
spare saved is an allocation, measured as the kernel sees it
(`alloc.py`, 20 runs: a new sealed memfd, mapped and filled, against
filling the same mapping again): **1600×1000, 2.82 ms against 0.47 ms;
3840×2160, 16.02 ms against 2.60 ms** (medians; 2.67–4.15 and
0.44–0.57, 14.82–21.69 and 2.44–3.96). So about 2.4 ms and 13 ms per
color change, on the one path that has it, for 6.4 MB and 32.4 MB per
output every moment the wallpaper is up. On wlroots the change reuses
the buffer on screen in place (released once uploaded), so it pays
nothing.

#### Binary

Release, stripped: 1,512,296 B before, 1,516,392 B after (+4,096; sizes
move in 4 KiB steps); links `libgcc_s`, `libm` and `libc` only.

### Verified where

On the container above; the code at `a581ffd` (the tree the final
binary was built from, with this commit's doc comments and help text).

- `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1
  SCOOTBG_TEST_SWAY=…sway-1.12/bin/sway devenv shell -- soft-egl cargo
  nextest run -p scootbg -p scootbg-mem`: 318 passed, 1 skipped (the
  `#[ignore]`d benchmark).
- `cargo test -p scootbg -p scootbg-mem` (same variables): 252 + 9 + 14
  + 2 + 7 + 3 + 7 + 3 + 17 + 4 passed, 1 ignored.
- `cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D warnings`
  and `cargo fmt --check -p scootbg -p scootbg-mem`: clean.
- `RUSTFLAGS="-D warnings" cargo build --release -p scootbg`: clean.
- **The end-to-end tests catch what they claim**, checked by breaking
  the code and running them: offering the second output a copy instead of
  the shared pixels fails `outputs_of_one_size_share_one_image_on_scoot`
  ("2 memfd mappings, want 1"); skipping the share step before a render
  fails `sharing_survives_hotplug_on_sway` (`left: [2, 1]`, `right:
  [3]`). Both restored.

### Not verified, and why

- **Writing into shared pixels, end to end.** The half of the gate that
  says "no other share" cannot be broken in safe Rust: `memory_mut`
  reaches the pixels only through `Rc::get_mut`, which refuses while any
  other `Rc` exists, and scootbg is `forbid(unsafe_code)`. Taking the
  count out of `is_writable` fails 3 `share` unit tests and no end-to-end
  test (the draw falls back to a new buffer when `memory_mut` refuses).
  The other halves (this output's buffer released, the pixels not
  frozen) are unit-tested; end to end, reaching them needs a compositor
  that releases a buffer still on screen while another output shares its
  pixels, and no run here showed one doing so.
  `full_size_colors_never_write_into_pixels_another_output_shows` runs
  the path on scoot and checks both outputs' pixels after every change.
- **Frozen pixels** (an output cleared or unplugged under a held buffer
  over shared pixels, then the other output reusing them): unit-tested
  only, for the same reason.
- **A compositor slow to configure**: none here is; the late surface is
  unit-tested in the model, and the glue by reading.
- **A kept image re-shown on a re-created surface**, now also served
  from shared pixels: by reading, as in ticket 6 (no compositor here
  closes the background surface of an output it keeps).
- **`--tty`, the dev VM, a GPU**: not reachable from this container.
- **`cargo nextest run --workspace`, `nix flake check`, macOS `cargo
  check`**: no compositor code changed; CI runs them.

### For the next tickets

- [restore-state.md](../restore-state.md): **the double decode.** An
  image `set` before the outputs are configured (a restore at login)
  decodes the file twice, 650–720 ms to the first image on one 4K output
  against 420–490 ms once configured. Make it one decode, then measure
  startup with a restored 4K JPEG. A color is on screen 3.0–4.1 ms after
  `scootbg daemon` starts.
- [lightest.md](../lightest.md): the idle rows are 0 wakeups and 0 CPU
  over 60 s, color and image. Idle memory: 12.3–12.4 MB RSS / 7.1–7.3
  MB PSS on 1× 1080p, 36.7–36.8 / 19.4–19.6 MB on 1× 4K **and on 2× 4K**
  (the floor once for both outputs: a daemon with a buffer per output
  holds it twice). With a color: 3.6–3.7 MB RSS, 2.0–2.1 MB PSS.
  Startup: 1.8–2.5 ms to the first answer.
- [transitions.md](../transitions.md) and
  [animated-images.md](../animated-images.md): a static wallpaper now keeps
  one buffer per output at rest and no spare; a transition's frames need
  their own buffers, allocated for it and dropped after (a 4K memfd costs
  16 ms to make and fill). Pixels shared between outputs must not be
  written: animate into buffers of the output's own, and share only the
  last frame.
- [scoot-integration.md](../scoot-integration.md): nothing new.
