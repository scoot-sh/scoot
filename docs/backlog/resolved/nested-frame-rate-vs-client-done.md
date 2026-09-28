---
title: "Nested scoot presents fewer frames than niri for a ~60 Hz client — RESOLVED (measured, don't build: own-timer pacing trade-off, client-limited)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Nested frame rate vs client — RESOLVED (don't build)

RESOLVED 2026-09-28 (verdict-only PR, no code change). **Don't build:
there is no defect.** The ~50-vs-~54 gap is the sum of two measured,
working-as-designed facts: scoot `--nested` paces presents off its own
16 ms frame timer rather than the host's frame callbacks, and the
benchmark client itself only produces ~54 commits/s. Under equal,
client-limited load on current main both compositors present every
client commit 1:1 (scoot 53.93, niri 53.62 frames/s, medians of three).
No README/protocols change: no user-facing behavior changed.

## Mechanism (measured)

- scoot `--nested` never asks the host for frame callbacks — 0
  `wl_surface.frame` requests in every host protocol log (three original
  DIAG sessions plus four new runs), against ~3000 per session for niri.
  Pacing is scoot's own frame timer (`headless.rs`: `FRAME_INTERVAL`
  16 ms, `frame_tick`), as `nested/gpu.rs` already documents.
- The timer drops at idle by design (`TimeoutAction::Drop` when
  `!needs_render`), and every client commit re-arms a full 16 ms wait
  (`handlers.rs` `commit` → `request_render` → `ensure_ticking`). So the
  steady-state present period is `max(client cadence, 16 ms + render)`,
  not a fixed 16 ms.
- Nested-side timestamps (new `WAYLAND_DEBUG=server` runs, current main
  `9794857` release) split the loop in two legs, stable across rounds:
  commit → scoot's `done` med 18.0–18.1 ms (16 ms timer + ~2 ms
  tick/render/present for one window), done → next client commit med
  0.34–0.38 ms (foot recommits almost immediately; its ~18.4 ms cadence
  comes from `sleep 0.016` + fork/exec per line, not from waiting).
- niri is host-callback-paced: median nested-commit interval 18.3 ms in
  the original logs (~16.7 ms host + render), just under the client's
  18.4 ms, so it also runs 1:1.

The original 49.8-vs-54.1 gap follows directly: with four windows at
1600x1000 the pixman render+present cost (~4 ms: 12.5% of a core at
50 fps) pushes scoot's loop to ~20.1 ms, just over the client's
18.4 ms, so ~8% of client commits coalesce (two commits share one
presented frame). niri's 18.3 ms loop fits under the cadence. Re-derived
from the original DIAG host logs: median inter-commit 20.0 ms
(scoot-pixman) vs 18.3 ms (niri-off), all three rounds.

## New numbers (dev VM, cage/pixman host, single `foot` anim client)

Focused rig (`/tmp/nfr-run.sh`, kept out of tree; methodology matches
`scripts/niri-ab-bench.sh`: same host, same 1600x1000, same anim
script, nested + host protocol logs), current-main release
(`9794857`, `/var/cargo-target/release/scoot`), three rounds each,
last-12 s window, host-side nested commits:

| run | frames/s | med interval |
|---|---|---|
| scoot 1 / 2 / 3 | 53.92 / 53.93 / 54.04 | 18.49 / 18.53 / 18.47 ms |
| niri 1 / 2 / 3 | 53.64 / 53.53 / 53.62 | 18.78 / 18.83 / 18.81 ms |

Nested-side (scoot): 1:1 commits:dones every round (e.g. 650/650);
commit→done med 18.0–18.1 ms, done→commit med 0.34–0.38 ms.
Both compositors track the client (~54 commits/s); neither is paced by
anything slower. Bench-order 4-window run (3 statics, anim last, as in
the original bench): 53.52, clean.

## GLES separation (the ticket's other half)

From the original DIAG logs, no new run needed: scoot-gles nested
commits at median 23.1 ms intervals (42.9–43.2 frames/s) with 0 host
frame requests — the period grows with render cost well past the
timer, i.e. llvmpipe-bound, exactly as the ticket suspected. The
run-main CPU (79% of a core vs 12.5% pixman) agrees. Pacing-timer
effects are second-order there; no conclusion about pacing should use
the gles figure.

## Fixes considered and rejected

- **Follow host frame callbacks (like niri).** A pacing redesign of the
  render loop, not a fix: the timer's idle-drop is a deliberate power
  feature (zero wakeups at idle, measured in the A/B), and host-paced
  presents would wake the compositor at host refresh even with nothing
  to draw unless request discipline is rebuilt around it. For ~4
  frames/s of one client kind in `--nested` only.
- **Shorten `FRAME_INTERVAL` or phase-lock the re-arm.** Global to all
  backends (headless suites, `--tty` draws) for a nested-only cosmetic
  gain; a sub-host loop would additionally beat against the host's
  60 Hz (judder) rather than converge. Same tiny payoff, wider blast
  radius.

## `--tty` check (by code trace, no new hardware run)

On `--tty` there is no sampling host to fall behind: `present()` issues
a page flip that completes on vblank, a second flip while one is in
flight sets `present_skipped` (never queues), and the owed `VBlank`
re-renders (`tty/mod.rs` `drm_event`, `dumb.rs`). Every drawn frame
reaches scanout (at most one vblank later); client callbacks follow
draws 1:1 through the same `send_frame` path. The nested shortfall —
commits landing between host refreshes — has no `--tty` analogue. No
live `--tty` frame counts were taken (the dev VM exposes only
virtio-gpu `card0`, no vkms; the `--tty` totals half is Asahi.md
Test 9). Nothing about this verdict asks `--tty` to change.

## Found on the way: scrolled-off windows go silent (correct, not a bug)

Four anim-first 4-window runs (three current-main, one `fe41921`)
showed host presents stopping ~1 s after the last window mapped, with
loop, IPC and screenshots alive and no panic — which first read as a
hang. It is occlusion culling: the first-mapped anim window ends up
scrolled off the strip (`windows` showed it `visible: false`, rect
`x=-782`, twice), unfocused, and stops receiving frame callbacks while
newer visible windows keep blinking. Bench order (anim last, focused,
visible) runs clean, and a refocus run revived the flow mid-session:
~17 s of silence while scrolled off, then 54–55 frames/s 1:1
(382 commits / 383 dones in 8 s) once scrolled back. Pre-existing
(`fe41921` shows the same shape), order-dependent, fully reversible —
no ticket filed.

Rig notes for whoever re-measures: capture both sides
(`WAYLAND_DEBUG=server` on cage *and* on the nested compositor; the
pinned wayland-backend fork logs `[UPTIME_MS][rs] <-/->`, not
`[HH:MM:SS]`/`#id`), give each run a private `XDG_RUNTIME_DIR` (stale
`wayland-N` sockets in the shared runtime cross-wire sessions),
size the cage keeper (`sleep 120` expired mid-probe more than once),
and never `pkill -f` a pattern that appears in the invoking command
(it kills the invoker first).

## Evidence

- Base for new runs: `9794857` (this branch is docs-only; `git status`
  clean apart from this ticket move), release binary
  `/var/cargo-target/release/scoot` (built on the VM 2026-09-28).
  Original binary: `~/evidence/niri-ab/bin/scoot` (`fe41921`).
- New logs (dev VM, `~/evidence/nested-frame-rate/`):
  `host/nested-scoot-{1,2,3}.log` (single-client),
  `host/nested-niri-{1,2,3}.log`,
  `host-scoot-4win-benchorder.log`,
  `host/nested-scoot-4win-refocus.log` (revival),
  `host-fe41921-4win.log`, plus `phase.pl`, `nested2.pl`, `host2.pl`
  (the analyses above).
- Original logs: `~/evidence/niri-ab/run-diag/r{1,2,3}-{scoot-pixman,niri-off,scoot-gles}/host.log.gz`
  with `results.tsv` frame counts (499/498/499, 543/541/541,
  429/431/430 per ~10 s).
- Compositor: niri 26.04 from
  `/nix/store/ww71z668r7kprqxwncl8xhsyjg6sxgr7-niri-26.04/bin/niri`
  (still present); foot 1.28.0; cage 0.3.1.

## Original ticket

Filed 2026-09-24 from the scoot/niri A/B (`docs/benchmarks.md`,
"animate" scene): nested pixman scoot presented 49.8 frames/s against
niri's 54.1 for a `foot` printing a line ~every 16 ms, far from
CPU-bound — suspected frame-callback pacing, uninvestigated, low
priority, with the `--tty` check and the gles separation as prerequisites.
Both prerequisites are answered above; the pacing question resolves to
the re-arm arithmetic, and there is nothing cheap and safe to fix.
