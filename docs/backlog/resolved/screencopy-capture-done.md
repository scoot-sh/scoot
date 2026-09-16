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
(`ext-image-copy-capture-v1: the screen capture shell clients and grim read`)
on the dev VM (`ssh -p 2222 dev@localhost`, NixOS, QEMU), building through the
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

### VM state

Both VMs were up before this work and neither was started, stopped or
restarted by it (`nc -z localhost 31022`, `nc -z localhost 2222` both
succeeded first). No `--tty` seat was claimed at any point — every live run
here is `--headless`, so nothing here could collide with another agent's
hardware work. Disk before: 78% used / 3.3G free; after cleanup: 76% / 3.6G.
Every `flexwm` and `swaylock` process started here was killed and confirmed
gone (`pgrep -fa` empty) at the end of each run.
