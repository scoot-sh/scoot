---
title: "[nested] pointer clicks never reach layer-shell surfaces; toplevels fine — CLOSED, could not reproduce"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# [nested] pointer clicks never reach layer-shell surfaces — CLOSED, could not reproduce

## What it said

Filed as gh issue #182 (live field report, Selkies webtop deployment).
scoot `4f8707c` `--nested` (pixman) in pixelflux's compositor, single
output 430x744, noctalia-shell 4.7.7 bar (30px exclusive zone) + launcher
overlay. `scoot msg pointer click 30 15` on the bar's launcher icon:
nothing (screenshot-verified), repeated at other bar points and right
button — all no-ops. Toplevel clicks worked. Reporter's suspicion: the
layer hit-test/focus handoff (`layer_hit` and surroundings) never
establishes layer focus, so the button lands wherever focus already was.

## Verdict

**Could not reproduce, on current `main` (2b14661) or on the reported rev
itself (4f8707c). No compositor change made — deliberately.** The
click-delivery path (`Request::Click` move-then-press-release →
`surface_under` layer branch → `layer_hit` → Smithay `pointer.motion`
focus → `pointer_button` delivery to `pointer.current_focus`) delivers
every click shape from the report to a real layer client, live over
`--nested` and in-harness. The field failure was most likely environmental
(client-side input region / host-side event swallowing / a bar with
nothing mapped to hit — note the companion finding below), not a scoot
input defect. A pinning test lands so the path stays green.

## Evidence

Harness (dev VM, `cargo nextest run -p scoot --bin scoot
compositor::layer_shell::tests::input::a_click_reaches_the_bar_press_and_release`):
PASS, and green inside the full workspace run (1304 passed, 6 skipped).
The test asserts wire-level delivery — the client's own `wl_pointer`
`enter` plus a fresh `button` serial — for a 30px exclusive-zone bar
(left/right/middle, press and release) and an `on_demand` launcher
overlay, i.e. both layer surfaces the report names. No such assertion
existed before; every prior layer-click test stopped at the compositor's
hit test or keyboard focus.

Live `--nested`, current `main` (binary built from `2b14661` + this
branch's test-only change): cage (headless, pixman) as host, inner
`scoot --nested --width 430 --height 744` (pixman, `wayland-1`,
`SCOOT_SOCKET=/tmp/layer182-ipc.sock`), throwaway probe bar (Top layer,
full-width 30px, exclusive zone 30, keyboard None — the noctalia-bar
shape; `/tmp/layerprobe`, not committed) mapping 1280x30 after the host
resize. The issue's exact probes over the control socket:

```
$ scootctl pointer click 30 15    -> {"type": "ok", "locked": false}
$ scootctl pointer click 22 15    -> {"type": "ok", "locked": false}
$ scootctl pointer click 215 15 right -> {"type": "ok", "locked": false}
```

probe log (raw):

```
pointer: ENTER serial=7 x=30 y=15
pointer: BUTTON serial=9 time=23921 button=0x110 pressed
pointer: BUTTON serial=10 time=23921 button=0x110 released
pointer: MOTION x=22 y=15
pointer: BUTTON serial=13 time=24931 button=0x110 pressed
pointer: BUTTON serial=14 time=24931 button=0x110 released
pointer: MOTION x=215 y=15
pointer: BUTTON serial=17 time=25942 button=0x111 pressed
pointer: BUTTON serial=18 time=25942 button=0x111 released
```

Repeated in a clean room with a `foot` toplevel mapped (the field had
ghostty up): 6 → 10 `BUTTON` lines, left press/release at (30,15) and
right press/release at (215,15) both delivered. Screenshot
(`scootctl screenshot`) shows the probe bar painted across the top with
zero toplevels mapped.

Positive control, reported rev `4f8707c` (built from a `/tmp` copy,
private target dir, old `scoot msg` client grammar): the same three
clicks deliver `ENTER` + `0x110` press/release ×2 + `0x111`
press/release to the same probe. So the path was not broken at the
reported rev either — "fixed by earlier work" would overclaim; nothing
in this path needed fixing.

## Companion issue #183 (shared root?)

Refuted for the input path, with the mechanism: `surface_under`'s layer
branch never consults window state, and the live probes deliver clicks
with zero toplevels mapped *and* with a toplevel mapped. The screenshot
above (bar painted, no toplevels) additionally shows plain layer
surfaces render without windows, so "layer state keyed off window
existence" does not hold for input or for basic layer rendering —
#183's missing noctalia content lives elsewhere (client-side or a
different render path) and stays its own ticket.

## What did NOT change, and why

- No compositor code: there is no defect to fix, and manufacturing one
  would risk the keyboard-focus-after-click (data-loss class) behavior
  the existing suite pins.
- No benchmark: no hot path changed shape (`layer_hit` untouched).
- No README change: no user-facing surface changed; no documented
  behavior was wrong.
- Bug-bash edge cases from the ticket (exclusive vs overlay, right/middle,
  no-layer fallback, layer-then-window transitions, lock refusal) are
  covered by the kept test (first three) and the untouched existing
  suite (the rest) — verified green, not re-argued.

## Methodological notes (for the reviewer)

- Two stack collapses mid-probe were both the harness's own fault, not
  the compositor's: cage exits when its child `sleep N` finishes, and
  the first two runs used `sleep 300`/`sleep 600`. Every "broken pipe"
  death traces to a sleep expiry, confirmed by timestamps. Clean-room
  reruns used `sleep 3600`.
- One unexplained transient is recorded, not papered over: on a
  mid-session (post-cage-rebirth, multi-probe, foot mapped-then-killed)
  main-branch inner, two successive fresh probes mapped but received no
  pointer events while IPC stayed healthy. It did not reproduce in the
  clean room in any shape (with/without toplevel, all buttons), and the
  harness pin passes deterministically. If it ever recurs, the hunt
  starts with compositor-side tracing of `surface_under`/`motion`, as a
  separate ticket — not this one.

Full gate (dev VM, `2b14661` + test-only change): `cargo nextest run
--workspace` 1304 passed / 6 skipped; `cargo clippy --workspace
--all-targets -- -D warnings` clean; `cargo fmt --check --all` clean;
`scripts/smoke-test.sh` exit 0 headless and `MODE=--nested` (under the
standing cage host).
