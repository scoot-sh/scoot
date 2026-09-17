---
title: "A lock surface mapped after the confirming frame never appears on live `--tty` — CLOSED UNREPRODUCED (isolate-first probe, no fix shipped)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A lock surface mapped after the confirming frame never appears on live `--tty` — CLOSED UNREPRODUCED.

## The entry as filed

Locking on dev-VM `--tty` and mapping a lock surface afterwards blanks
the screen, but the surface's own pixels never appear: locked IPC
screenshots stay backdrop-black (plus two stray magenta pixels at the
left edge, mechanism not isolated), while the session is genuinely
locked (`locked: true` on IPC replies, `locked` duly arriving over the
wire).

Proven pre-existing, not a regression from the vblank-confirm change:
the identical black screenshot (same two stray pixels) reproduces on the
pre-change binary, and the harness covers this path green — a surface
mapped after the lock appears through timer-driven renders alone when
checked there.

The harness/live delta points at damage under real buffer ages: with age
0 every render redraws everything, while under `--tty` a surface
committed after the confirming frame appears to get no damage redraw —
cursor sweeps across the whole screen do not reveal it either. Prime
suspect, not yet isolated: nothing feeds a lock surface commit's damage
to the tracker the way `Space`/`LayerMap` do for windows and layer
surfaces.

Why this is high priority next to the guarantee just closed: a lock
screen that never shows the locker leaves the user staring at black
with no visible password prompt. Keyboard focus still lands on the
surface (liveness-based), so blind entry may work, but that is not a
lock screen anyone can daily-drive.

## Resolution: unreproduced on `main`, suspect disconfirmed — no PR opened

Isolate-first probe on unmodified `main` (`2c0048e`, six live `--tty`
lock sessions, dev VM, 1280x720, force-clean builds with real `Compiling`
lines, private binary copies): a standalone lock client mapping a solid
**green** surface (so any magenta pixels are provably not the surface)
after `locked` is confirmed renders GREEN fullscreen + cursor, every
time — map-after-confirm, map-before-confirm, 20 frame-paced recommits,
cursor sweep, foot-underneath, 30 blind recommits at 5ms spacing. Pixel
censuses exact (e.g. 921464 green + 105 white + 31 black = 921600; the
non-green is the idle cursor at (0,0)). Zero stray pixels in 7
screenshots. Temporary per-frame logging (reverted) shows the post-confirm
map frame rendering at real buffer age 2 with fullscreen damage and a
flip ~2ms later; 52 renders, 51 flips + 1 modeset, 0 present-skips.

Mechanism audit (why the suspect is wrong, not just unreproduced):
`Window::on_commit` only refreshes a cached bbox — it feeds NO damage to
the tracker, and neither does `commit_layer_surface`. The ticket's "the
way `Space`/`LayerMap` do" premise is incorrect: ALL surface damage flows
through the generic `on_commit_buffer_handler` (every commit, lock
commits included) plus the tracker's new/changed-element rules (an
element ID the tracker never saw generates full-geometry damage). Those
rules provably fire for lock surfaces (log evidence above). There is no
lock-specific damage hole in this mechanism.

Magenta pixels: not reproduced, partially explained. Ruled out:
compositor defaults (no magenta anywhere), backdrop, fallback cursor
shapes, uninitialized buffers. Lead hypothesis: the deleted PR #84
probe's own buffer color (the harness `LOCK_BGRA` is magenta). The
original probe and its logs are gone, so this is likelihood, not proof.

Plausible artifact class for the original observation: the dev VM holds
hundreds of stale IPC sockets (`/run/user/1000/*.sock`, `/tmp/*.sock` —
`flexwm-lock.sock`, `flexwm-hwlock.sock`, `vblank-live.sock`, …) plus
stray Wayland sockets. A `msg screenshot` against any socket but the live
compositor's reads another session's state — black + `locked:true` with
no surface is exactly what a stale locked-with-nothing-mapped session
looks like, on both binaries, with the harness green. Unprovable
post-hoc, but it fits all four observations with zero code bugs. Dead
sockets were swept post-investigation (see below); future probes must pin
`--socket` + `WAYLAND_DISPLAY` and record both.

Adjacent finding, code-traced only (NOT observed live, NOT fixed, filed
separately as `docs/backlog/rendering/present-skip-eats-frame-damage.md`):
a `present()` skipped for an in-flight flip consumes that frame's damage
in `render_output`, and the retry re-renders with damage `None` — scanout
keeps stale pixels until unrelated damage arrives. Cannot explain black
screenshots (the image is always correct), needs a commit inside a ~16ms
flip window (a 30-commit burst produced 0 skips — frame timer and vblank
run phase-locked on this VM), self-heals on next damage. Left as a
low-priority entry, not a fix.

Re-open triggers (any one): a locked screenshot showing black with a
mapped lock surface from a session with recorded `--socket` +
`WAYLAND_DISPLAY` hygiene; a screenshot within the same second as the map
commit; a `wayland-info` cross-check proving the screenshotted compositor
is the locked one.
