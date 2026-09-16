---
title: "Screen capture for clients (`ext-image-copy-capture-v1`) for shell thumbnails and previews — RESOLVED (output half)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Screen capture for clients (`ext-image-copy-capture-v1`) for shell thumbnails and previews — RESOLVED (output half).

## The entry as filed

> Both shell probes hit this (DMS gap 6, Noctalia gap 6, 2026-09-14):
> launcher window thumbnails and workspace-overview live previews
> (`ScreencopyView`) have nothing to consume — neither global is
> advertised. Previously bundled in
> `docs/backlog/protocols/protocol-gaps-niche.md`.
>
> This is purely a shell-client gap, not an agent gap: flexwm's own
> screenshots for computer-use automation go over the privileged IPC
> socket (`flexwm msg screenshot`, owner-only, rate-limited), which stays
> regardless. The standard-protocol path is for third-party tools
> (`grim`, `wf-recorder`, shell thumbnails, conferencing screen-share)
> that will never speak flexwm IPC.
>
> Per the standing rule, prefer the standard protocol: evaluate
> `ext-image-copy-capture-v1` (with `ext-image-capture-source-v1`)
> first, `wlr-screencopy-unstable-v1` for legacy clients second. Note
> the privacy interaction with session lock: captures taken while
> `ext-session-lock-v1` holds the session must see only the lock
> framebuffer, never the windows behind it — the same guarantee
> `flexwm msg screenshot` already gives. Rough size: M.

## What shipped

`crates/flexwm/src/compositor/screencopy.rs`, against Smithay's
`wayland::image_copy_capture` and `wayland::image_capture_source` at the
pinned rev (`0ff00983`) — unlike the three protocol items before it, this one
had a maintained upstream implementation, so flexwm writes handlers rather
than wire format.

Two globals:

- `ext_image_copy_capture_manager_v1` (version 1)
- `ext_output_image_capture_source_manager_v1` (version 1)

Neither filtered by client, same trust model as every other global here (no
security-context support, so an allow-list would be theatre) — called out
explicitly in `README.md` because this is the protocol where "any process that
can reach the socket can read your screen" is most worth stating plainly.

### The `ext-` protocol, and no wlr protocol alongside it

`CLAUDE.md`'s standing rule is the `ext-` successor where one exists. PR #50
made the opposite call for the window list after measuring that the client
that mattered did not speak it, so the same measurement was taken here first
(dev VM nix store, 2026-09-16):

- `grim` 1.5.0: `ext_image_copy_capture_*` and `ext_image_capture_source_v1`
  symbols present; **no** `zwlr_screencopy_manager_v1`.
- `quickshell` 0.3.1 (the build DMS and Noctalia run on):
  `ext_image_copy_capture_manager_v1`,
  `ext_output_image_capture_source_manager_v1`,
  `ext_foreign_toplevel_image_capture_source_manager_v1` **and**
  `zwlr_screencopy_manager_v1` — i.e. it speaks the `ext-` protocol and keeps
  the older one as a fallback.

So nothing measured needs `wlr-screencopy-unstable-v1`, and it is not
implemented.

### Output capture only

The toplevel source manager is deliberately not advertised — not advertised
and refused, so a client takes its fallback path immediately rather than
discovering a `stopped` at runtime. Capturing one window in isolation needs a
second render target per session, a constraint-refresh path driven by window
resizes, mid-session teardown when the window closes, and a stated answer for
a locked session; that is this module again, so it is
[its own item](../protocols/screencopy-toplevel-capture.md) — the same split
PR #47 and PR #49 made.

### How a capture is served

A `capture` request does not copy pixels there and then. The frame is parked
on its session and served from `State::service_captures`, which runs on the
frame tick immediately after `State::render`:

- it bounds what a client can ask for — at most one capture per session per
  frame tick, the same reason `ipc.rs` spaces a connection's screenshots by
  `FRAME_INTERVAL`;
- it is what the protocol describes ("unless this is the first successful
  captured frame performed in this session, the compositor may wait an
  indefinite amount of time for the source content to change"), so a
  session's first frame is served on the next tick whatever the screen is
  doing and every later one waits for `State::frame_serial` to move — a
  static desktop costs a `u64` comparison per tick and nothing else;
- it keeps the copy out of Wayland request dispatch, so nothing re-enters
  `render()` from inside a client's request.

The framebuffer read-back happens **once per tick**, shared by every session
that is due — `PixmanRenderer::copy_framebuffer` allocates and fills a fresh
image per call, so doing it per session would cost an extra full copy of the
screen for each client watching.

### The session-lock guarantee

Inherited, not re-derived: `render()` decides once per frame whether it is
drawing the lock screen or the desktop, and this reads back that same
framebuffer through the same `bind`/`copy_framebuffer`/`map_texture` sequence
`screenshot.rs` uses. The one thing reuse does not cover is the window between
a lock being accepted and its first blanked frame reaching the framebuffer
(`is_locked()` already true, desktop pixels still there) — reachable when a
render fails, since `render()` clears `needs_render` either way. Captures are
not served at all while a lock is pending (`SessionLock::awaiting_blank`);
the frame stays parked, which the protocol's "may wait indefinitely" allows.

### Bugs found and fixed while building it

- **Smithay leaks every session a client ever creates.** `SessionData::
  destroyed` calls the compositor's `session_destroyed` but does not remove
  the `SessionRef` from `ImageCopyCaptureState::sessions`; only `cleanup()`
  does, and nothing upstream calls it. The `capture` request then walks that
  list linearly. `session_destroyed` calls `cleanup()` here, and
  `a_destroyed_session_leaves_the_compositors_list` asserts on *both* lists so
  a version that swept only flexwm's own would fail.
- **Smithay never raises `duplicate_frame`.** The protocol allows at most one
  frame object per session at a time; the pinned rev pushes every
  `create_frame` onto an unbounded list. The parked-frame slot is the bound:
  a second outstanding capture is failed rather than queued.

### Known deviation, recorded rather than discovered

`paint_cursors` is accepted and has no effect. Under `--headless`/`--nested`
nothing draws a cursor, so a capture never contains one; under `--tty` the
cursor is an element in the one framebuffer a capture is read out of, so a
capture always contains it. Honouring the flag means a second full-output
render with the cursor dropped. `flexwm msg screenshot` has the same property.

### What is still unbounded

How many sessions one client may hold — N sessions with N parked frames cost
N full-screen copies on a tick the screen changes. Filed as
[its own item](../protocols/screencopy-session-cap.md) with the reasoning for
why it was not simply capped (the same per-frame work N mapped surfaces
already demand; and a *global* cap would let one client deny another, which
Smithay's handler API gives no per-client alternative to).

## Tests

`crates/flexwm/src/compositor/screencopy/tests.rs`, eleven real-client tests.
Every one drives a real `wayland-client` connection, allocates a real `wl_shm`
buffer over a real memfd, and **reads that memfd back** to compare the
client's own bytes with the framebuffer — the claim under test is about what
landed in a client's buffer, which a compositor-side assertion cannot make.

- constraints name the framebuffer size, both formats, opaque first, one
  `done`;
- a capture equals the framebuffer byte for byte, and contains the window that
  was on screen;
- an `Xrgb8888` capture is opaque everywhere over a deliberately translucent
  background, and differs from the framebuffer in nothing but the fourth byte;
- a capture while locked carries the lock surface and no pixel of the window
  behind it;
- a capture parked *before* a lock and served *after* it still carries no
  desktop pixel — the race the guarantee actually has to survive;
- a later capture of an unchanged screen is not served, and is served the
  moment something changes, carrying the new frame;
- an undersized buffer is `failed(buffer_constraints)`;
- a resized output re-advertises `buffer_size` with its own `done`;
- a second outstanding capture on one session is refused;
- a destroyed session leaves both session lists;
- a client disconnecting with a capture outstanding leaves both lists correct
  and a second client is still served.

## Verification

Everything below was captured against commit **`74557ec`**
(`ext-image-copy-capture-v1: the screen capture shell clients and grim read`).
Two doc comments in `screencopy.rs` were corrected afterwards (an
`#[allow(dead_code)]` on a field that does have a reader, and a
`session_destroyed` comment that credited `Session::drop` for failing the
parked frame when it is `Frame::drop` that does it — `Session::drop` returns
early on an already-dead object). **No executable statement changed**, and
`cargo build`/`clippy`/`fmt`/`test`/`nextest` were re-run clean after them, so
the live results below still describe this tree. Run on the dev VM (`ssh -p 2222 dev@localhost`, NixOS, QEMU), building through the
9p mount at `/mnt/flexwm` with `CARGO_TARGET_DIR=/var/cargo-target`. The
binaries under test were copied out of the shared target dir immediately after
each build (`~/screencopy-evidence/flexwm-74557ec`,
`~/screencopy-evidence/flexwm-main-37c44f1`) and every live run used those
copies, not `/var/cargo-target/debug/flexwm`.

### Build, test, lint

```
$ cargo clean -p flexwm && cargo build -p flexwm
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 33.15s

$ cargo fmt --check -p flexwm
FMT_CLEAN

$ cargo clippy -p flexwm --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 13.56s

$ cargo test -p flexwm
test result: ok. 643 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 18.67s

$ cargo nextest run --workspace
     Summary [  26.418s] 737 tests run: 737 passed, 1 skipped

$ bash scripts/smoke-test.sh     # default --headless
rc=0, 12 `ok:` assertions, no failures
```

The eleven screencopy tests were also run in isolation eight times in a row
(`cargo test -p flexwm --bin flexwm screencopy`), all `11 passed`, after a
flake in `a_later_capture_waits_for_the_screen_to_change` was traced to the
*test client* sharing one `xdg_surface.configure` serial slot between two
windows (mapping the second re-configures the first, so the second acked a
serial that was not its own →
`xdg_wm_base.wrong_configure_serial`). Fixed by keying the serial per surface,
which is what the other real-client suites here already do.

### `grim` end to end

`grim` 1.5.0 from the VM's nix store — checked first that it speaks this
protocol and not the older one:

```
$ strings $(readlink -f /run/current-system/sw/bin/grim) | grep -E 'ext_image_copy_capture|ext_image_capture_source|zwlr_screencopy'
ext_image_capture_source_v1
ext_image_copy_capture_frame_v1
ext_image_copy_capture_manager_v1
ext_image_copy_capture_session_v1
...            (no zwlr_screencopy_* symbol at all)
```

Both globals are advertised:

```
$ wayland-info | grep -iE 'image_c(opy|apture)'
interface: 'ext_output_image_capture_source_manager_v1', version:  1, name:  9
interface: 'ext_image_copy_capture_manager_v1',          version:  1, name: 10
```

Against `flexwm --headless --width 800 --height 600 -- foot`, with one real
`foot` window mapped (`flexwm msg windows` reports
`id 1, app_id foot, rect 12,12 382x576, focused true`):

```
$ for i in 1 2 3; do time grim shot$i.png; done
grim run 1: 0.05 s
grim run 2: 0.04 s
grim run 3: 0.04 s

$ file shot1.png
shot1.png: PNG image data, 800 x 600, 8-bit/color RGB, non-interlaced

$ sha256sum shot1.png shot2.png shot3.png
4699a0802ee70d15d9eea3bf78c38df6acc52c71d232170558eaad4f3eefa5cc  shot1.png
4699a0802ee70d15d9eea3bf78c38df6acc52c71d232170558eaad4f3eefa5cc  shot2.png
4699a0802ee70d15d9eea3bf78c38df6acc52c71d232170558eaad4f3eefa5cc  shot3.png
```

`8-bit/color RGB` with no alpha channel is `grim` taking the `Xrgb8888` this
compositor offers first, which is the point of that ordering. The three
captures of a static screen being byte-identical is the copy being the
framebuffer rather than anything reconstructed.

Artifact pulled to the Mac and viewed: the capture shows the `foot` terminal
with its prompt, flexwm's blue focus ring around it, and the configured
background — i.e. exactly what `flexwm msg screenshot` shows.
(`dev@flexwm-vm:~/screencopy-evidence/grim.png`, 7426 bytes, sha256 as above.)

### The session-lock guarantee, against a real locker

`swaylock` 1.8.6 (`nix shell nixpkgs#swaylock`, ephemeral) locking the same
session with `foot` still mapped, then `grim` capturing through the lock:

```
$ grim unlocked-grim.png                       # desktop, foot visible
$ swaylock -c 2060c0 -f &                      # real ext-session-lock-v1 client
   INFO flexwm::compositor::session_lock: locking the session
$ grim lock-grim.png

$ ls -l unlocked-grim.png lock-grim.png
-rw-r--r-- 1 dev users 7426 unlocked-grim.png
-rw-r--r-- 1 dev users 2791 lock-grim.png

$ sha256sum unlocked-grim.png lock-grim.png
4699a0802ee70d15d9eea3bf78c38df6acc52c71d232170558eaad4f3eefa5cc  unlocked-grim.png
10ba935a1d2561a54116e5666bd71d2edaae9746df65ec60e431d16eb5117a0a  lock-grim.png
```

`lock-grim.png` pulled to the Mac and viewed: **a solid `#2060c0` field, the
exact colour passed to `swaylock -c`, with no trace of the `foot` window, the
focus ring or the desktop background.** `flexwm msg windows` still listed the
window at that moment (the window did not go away; it simply is not in the
frame), which is what makes this a capture-path result rather than a
window-list one.

### `--tty`, and the cursor deviation measured rather than reasoned

The `paint_cursors` deviation is a `--tty`-only claim ("under `--tty` a
capture always contains the cursor, flag or no flag"), so it was taken on the
real backend rather than argued from the element list. The VT-bound seat was
confirmed free first (`pgrep -fa 'flexwm|sway'` empty), and the run was
`--tty` over ssh, which takes real DRM master and scans out to the QEMU
window.

```
$ ./flexwm-74557ec --tty --socket $XDG_RUNTIME_DIR/flexwm-tty.sock &
   INFO flexwm::compositor::tty: drm: driving this device
        path=/dev/dri/card0 connector=Virtual-1 width=1280 height=720

$ flexwm msg outputs
   { "id": 1, "name": "Virtual-1", "rect": { 0,0 1280x720 }, "scale": 1.0 }

$ flexwm msg pointer move 400 300
$ grim tty-grim.png            # no -c, i.e. the client did NOT ask for cursors

$ file tty-grim.png
tty-grim.png: PNG image data, 1280 x 720, 8-bit/color RGB, non-interlaced
```

Artifact pulled to the Mac and viewed: the capture is the configured
background at the connector's full 1280x720 mode, **with the compositor's own
cursor arrow drawn at exactly (400, 300)** — the pointer position just set.
`grim` was not given `-c`, so this is the deviation happening, not a client
asking for it. (`dev@flexwm-vm:~/screencopy-evidence/tty-grim.png`, 4390
bytes.) It also confirms the size half of the constraints on a real
connector: the buffer is the DRM mode, not the headless default.

The compositor was stopped and confirmed gone afterwards; nothing else was
holding the seat before or after.

Not measured: that `frame_serial` stays put when `--tty`'s damage tracker
returns `Ok` with `damage: None` (the `age > 0` path, unreachable on the
other two backends). Reasoned rather than observed, and the direction is the
safe one: `damage: None` from an age-based tracker means the framebuffer is
unchanged, so a parked capture that waits is waiting correctly. The failure
mode a wrong answer here produces is one redundant copy, not a frozen
preview.

### Benchmark: what the per-frame path costs

Method: `flexwm --headless --width 1920 --height 1080` with a client that
forces a redraw ~20x/s (`foot -- sh -c 'while :; do date; sleep 0.05; done'`),
6s of warm-up, then the compositor process's own `utime+stime` from
`/proc/<pid>/stat` over a 20s window. `CLK_TCK=100`, so 2000 jiffies is one
full core. Same workload, same VM, alternating runs; `main` is `37c44f1`, the
commit this branch is cut from.

| Scenario | Binary | jiffies / 20s | % of one core |
| --- | --- | --- | --- |
| Redrawing client, **nobody capturing** | main `37c44f1` | 147 | 7.35% |
| Redrawing client, **nobody capturing** | branch `74557ec` | 149 | 7.45% |
| ...repeat | main `37c44f1` | 148 | 7.40% |
| ...repeat | branch `74557ec` | 150 | 7.50% |
| No clients at all | main `37c44f1` | 0 | 0% |
| No clients at all | branch `74557ec` | 0 | 0% |

**+2 jiffies over 20s — 0.1% of one core, at the noise floor.** That is the
whole per-frame cost added to a compositor nobody is capturing: one
`Option::is_some` and one `wrapping_add` in `render()`, and one `Vec::iter`
over an empty list in `service_captures`. An idle compositor still uses
exactly zero, i.e. a parked capture does not keep the frame timer alive (the
`frame()` handler calls `ensure_ticking`, deliberately not `request_render`).

What a capture actually costs, same setup, loops run for 20s:

| Load | iterations in 20s | jiffies | % of one core |
| --- | --- | --- | --- |
| `grim -t ppm /dev/null` in a tight loop | 229 | 1230 | 61.5% |
| `flexwm msg screenshot` in a tight loop | 10 | 1979 | 99.0% |

229 whole `grim` runs — each one a fresh connection, registry round trip,
source, session, constraint batch, frame and 1920x1080 copy — in 20s, at 61.5%
of one core: **~54ms of compositor CPU per complete `grim` invocation in a
debug build.** The `flexwm msg screenshot` row is not a like-for-like
comparison and is included only as the existing reference point: it is
dominated by PNG encoding on the event-loop thread (already filed as
`docs/backlog/ipc/screenshot-encode-on-event-loop.md`), which this path does
not do — a screencopy client is handed raw pixels.

Not measured live, because no CLI client holds a session across captures:
the "a later capture waits for the screen to change" throttle. `grim` opens a
new session per run, so every one of the 229 above is a session's *first*
frame and is served unconditionally. The behaviour is covered at the wire
level by `a_later_capture_waits_for_the_screen_to_change`, which asserts a
repeat capture of a static screen is still unanswered after 300ms (~18 frame
ticks) and is served the moment a window maps.

### Round 2: what independent review changed, and what it measured

Two review findings needed code. Both were addressed and re-verified; this
section is keyed to the commit that carries them, not to `74557ec`.

#### `write_capture`'s `offset` and `stride` handling had no test that could fail

Correct, but untested in the only shape that matters: every test allocated one
buffer per pool at `offset 0` with `stride == width * 4`, so the `data.offset`
term and the padded-stride case were never exercised by anything that would
notice their removal. Two tests were added -- one capturing into the second of
two buffers carved from one pool, one into a buffer with 16 bytes of row
padding -- and both were **proved** to catch the corresponding mistake by
making it and watching them fail, then restoring:

```
# 1. drop the `data.offset` term from the destination pointer
-  let dst = ptr.add((data.offset as i64 + y * dst_stride) as usize);
+  let dst = ptr.add((y * dst_stride) as usize);
   => a_capture_lands_at_its_buffers_offset_inside_a_shared_pool ... FAILED
      (the only failure; the other 12 tests all still passed)

# 2. walk rows by pixel width instead of the client's stride
-  let dst = ptr.add((data.offset as i64 + y * dst_stride) as usize);
+  let dst = ptr.add((data.offset as i64 + y * row) as usize);
   => a_capture_honours_a_buffer_whose_rows_are_padded ... FAILED
      (the only failure; 12 passed)

# 3. drop the alpha forcing (checks the existing Xrgb test still bites)
   => an_xrgb_capture_is_opaque_even_over_a_translucent_background ... FAILED

# 4. neuter the tail loop the wide alpha step cannot cover
-  for x in steps * PIXELS_PER_STEP..width {
+  for x in steps * PIXELS_PER_STEP..steps * PIXELS_PER_STEP {
   => an_xrgb_capture_is_opaque_at_a_width_the_wide_step_cannot_divide ... FAILED
```

One term is deliberately **not** covered, and saying so is more useful than
implying it is: the `data.offset` term inside the `reach` bounds check. A
buffer that overhangs its pool cannot be created in the first place --
Smithay's own `create_buffer` refuses `offset > pool_size - stride * height`
-- and pools only ever grow, so there is no reachable input that distinguishes
the check with the term from the check without it. It stays as
defence-in-depth against a future upstream change, not as something a test can
pin.

#### The `Xrgb8888` alpha pass: the finding was right, the proposed fix was not

The review asked for the second, per-pixel alpha pass to be folded into the
row copy as a single pass. Measured first rather than assumed, and the single
pass is **six times slower** in the profile every test and dev-VM session
runs. Isolated over a 1920x1080 frame (`ms/frame`, median of runs, standalone
`rustc` at each level so nothing else is in the number):

| | `opt-level=0` | `opt-level=3` |
| --- | --- | --- |
| row `memcpy` alone (the `Argb8888` path) | 0.84 | 0.20 |
| ...plus one alpha byte per pixel (what shipped) | 30.4 | 1.56 |
| one pass, per pixel, copy and force together (as asked) | 186.4 | 1.29 |
| ...plus alpha two pixels at a time (`u64`) | 83.8 | 0.89 |
| **...plus alpha four pixels at a time (`u128`)** | **33.7** | **0.92** |

So the finding's substance held -- the alpha pass really is most of what this
path costs, 30.4 of 31.2 ms at `opt-level=0` -- but the fix that follows from
it does not. Two *wide* passes beat one narrow one: the copy stays a `memcpy`
intrinsic and the opacity pass does a quarter as many iterations. What shipped
is the `u128` row.

End to end, against real `grim`, both directions measured rather than
extrapolated. Method as before: `--headless` 1920x1080 with a client forcing
~20 redraws/s, `grim -t ppm /dev/null` in a tight loop for 20s, compositor's
own `utime+stime` from `/proc/<pid>/stat`, three runs each, alternating.

```
RELEASE build (opt-level=3, fat LTO -- the profile that ships)
  BEFORE per-byte alpha pass  rep1: jiffies=508 captures=493 => 10.30 ms/capture
  AFTER  u128 alpha step      rep1: jiffies=518 captures=508 => 10.20 ms/capture
  BEFORE per-byte alpha pass  rep2: jiffies=534 captures=501 => 10.66 ms/capture
  AFTER  u128 alpha step      rep2: jiffies=522 captures=506 => 10.32 ms/capture
  BEFORE per-byte alpha pass  rep3: jiffies=538 captures=504 => 10.67 ms/capture
  AFTER  u128 alpha step      rep3: jiffies=520 captures=507 => 10.26 ms/capture

DEV build (opt-level=0 -- what tests and the dev VM run)
  BEFORE per-byte alpha pass  rep1: jiffies=1262 captures=230 => 54.9 ms/capture
  AFTER  u128 alpha step      rep1: jiffies=1331 captures=208 => 64.0 ms/capture
  BEFORE per-byte alpha pass  rep2: jiffies=1270 captures=232 => 54.7 ms/capture
  AFTER  u128 alpha step      rep2: jiffies=1327 captures=211 => 62.9 ms/capture
  BEFORE per-byte alpha pass  rep3: jiffies=1277 captures=233 => 54.8 ms/capture
  AFTER  u128 alpha step      rep3: jiffies=1338 captures=212 => 63.1 ms/capture

Idle (nobody capturing), dev build: BEFORE 147, AFTER 148 jiffies over 20s
```

**~4% off a real capture in release, ~15% added in a dev build, and nothing at
all for a compositor nobody is capturing.** That is a trade, not a free win,
and it is taken on the grounds `Cargo.toml`'s own release-profile comment
states: "lightweight" is judged in release. The release build for this was a
real `cargo build -p flexwm --release` of both variants (4m13s cold, 2m33s for
the second), not an extrapolation; the debug target dir was removed first for
disk and rebuilt afterwards.

The much larger number this turned up is **not** in the shipped change: not
forcing the byte at all is ~13% off a release capture and ~77% off a debug
one, because `Xrgb8888`'s fourth byte is undefined and a conforming client
(`grim` demonstrably) never reads it. That is a behaviour question rather than
an optimization, so it is
[filed](../protocols/screencopy-xrgb-alpha-forcing.md) -- with the probable
answer, which is to force only when `background_color`'s alpha is actually
below 1.0 -- rather than decided mid-review.

#### Re-verification after both fixes

Force-clean rebuild (the whole `debug` target dir was removed for the release
experiment, so this is from scratch, not incremental):

```
$ rm -rf /var/cargo-target/debug && cargo build -p flexwm --all-targets
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2m 14s

$ cargo fmt --check -p flexwm                       FMT_CLEAN
$ cargo clippy -p flexwm --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 05s
$ cargo test -p flexwm
test result: ok. 646 passed; 0 failed; 1 ignored
$ cargo nextest run --workspace
     Summary [  26.419s] 740 tests run: 740 passed, 1 skipped
$ bash scripts/smoke-test.sh
rc=0, 12 `ok:` assertions
```

The fourteen screencopy tests were run six more times in isolation and the
full suite twice more, all clean -- the same flake check the first round's
`wrong_configure_serial` bug earned.

Not re-run after these two fixes, and named rather than implied: the live
`grim`, `swaylock` and `--tty` results above. Both changes are inside
`write_capture`'s row loop, which the `grim` benchmark above exercised
hundreds of times per run in both builds, and neither touches the lock guard,
the cursor path or the constraint negotiation those results cover.

### VM state

Both VMs were up before this work and neither was started, stopped or
restarted by it (`nc -z localhost 31022`, `nc -z localhost 2222` both
succeeded first). The VT-bound `--tty` seat was claimed exactly once, for the
cursor-deviation measurement above: it was confirmed free first
(`pgrep -fa 'flexwm|sway'` empty) and released immediately after. Every other
live run here is `--headless` and takes no seat. Every `flexwm` and `swaylock`
process started here was killed and confirmed gone (`pgrep -fa` empty) at the
end of each run.

Disk moved around more than usual because round 2 needed a release build:
78% used / 3.3G free at the start, 85% / 2.3G at its peak, then the debug
target dir was removed to make room (40% / 9.0G), release built twice, and
the debug tree rebuilt afterwards — 49% / 7.6G at the end, i.e. more free
space than this started with.
