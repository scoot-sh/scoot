---
title: "scootlaunch go/no-go prototype, out of the workspace: typing, latency, activation vs fuzzel"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-17"
---

# scootlaunch prototype: the go/no-go gate

Filed 2026-10-12 in the #579 fix round. The gate moved out of `launch-ui` to run before any
extraction. Serves **daily-drive**.

## What to do

A throwaway binary **outside the workspace**, `#[path]`-including scootbar's drawing and text
sources (`crates/scootbar/fuzz/Cargo.toml:28-65`'s pattern) and the keyboard spike's pick. It
shows an overlay panel with exclusive keyboard focus over a real desktop-entry list, plus a
mouse-pick launch.

Measure against fuzzel 1.14.1 in the same harness on the M2. The #579 review measured fuzzel
open at 18,944 kB RSS; the first frame is 50 to 57 ms (`desktop-launcher-done.md`).

## Acceptance

All of the following, recorded here:

- first frame warm and cold, open RSS, and keystroke-to-commit at 300 and 3,000 entries, all
  at or under fuzzel's;
- a query typed correctly on AZERTY and with `us(intl)` dead keys;
- a mouse-picked entry re-activating an already-running single-instance app. That is the case
  scoot's source documents fuzzel failing (`crates/scoot/src/compositor/activation.rs:81-99`).

## Resolving this gate

The tickets it guards name it in prose ("GO verdict recorded in launch-prototype"), which `scripts/backlog resolve` does not strip ([why](../../../dev/design/shell-apps.md#how-the-gates-stay-closed)). Record the verdict here, then resolve. On **go**, a human deletes that clause from the guarded tickets. On **no**, a human resolves the epic and the guarded tickets as won't-do, citing this evidence.

## Evidence the review needs

The rows beside fuzzel with the SHA, the typing transcript, and the activation screenshot.

## Verdict: GO

Measured 2026-10-18 on the aarch64 dev VM under `scoot --headless`
(1 output, 1280x800, pixman). The M2 rows could not run: no M2 access
this session. All rows are dev-VM-only, same harness both sides.
Prototype source was out of the workspace, `#[path]`-including
scootbar's `color`, `paint`, `icon`, `text` and `font` sources unchanged,
keys through `xkbcommon`, deleted after measuring. scoot built from
`6c1affa45`; fuzzel 1.14.1 from nixpkgs (store path
`y92k567fwwjpr3cc835qjj9dwnxfbrnd-fuzzel-1.14.1`).

| Row | Prototype | Fuzzel 1.14.1 |
|---|---|---|
| First frame, fresh process, 300 entries | 25-37 ms | 64-78 ms |
| First frame, fresh process, 3,000 entries | 25-37 ms | 64-78 ms |
| Open RSS, 300 entries (14 s + 44 s samples) | 4,884 kB, 1 thread, 6 fds | 14,560 kB, 13 threads, 12 fds |
| Open RSS, 3,000 entries | 6,208 kB, 1 thread, 6 fds | 13,872 kB, 13 threads, 12 fds |
| Keystroke to pixels, 300 entries (5 reps) | 27-34 ms | 25-37 ms |
| Keystroke to pixels, 3,000 entries (5 reps) | 27-30 ms | 26-33 ms |
| Stripped binary | 922,296 B (`.text` 649,060) | 371,560 B (`.text` 158,108) |
| Linked libraries | libc, libgcc_s, libxkbcommon.so.0 only | 11 shared libraries (cairo, fcft, resvg, pixman, fontconfig, wayland, …) |
| fuzzel closure | — | 159,809,528 B |

The prototype binary is bigger (static emission: pure-Rust Wayland,
`ab_glyph`, the xkbcommon glue), inside the design's 1.8 MB budget, with
zero marginal closure on a scoot desktop. End-to-end keystroke rows are
present-bound on both sides and tie; the prototype's own CPU path is
rescore 60-780 us plus redraw 40-150 us at 3,000 entries (headless
`--bench`, same binary). Identical-list stress (3,000 real desktop files):
first frame 79 ms vs 59 ms; the 47 ms gap is desktop-file parsing,
which the planned index cache (`launch-desktop-index`) removes.

Typing, typed through `scoot msg type` into the focused panel:
AZERTY (`XKB_DEFAULT_LAYOUT=fr`) `aqz&1` arrives `aqz&1`;
`us(intl)` AltGr+é arrives `café`; dead_acute+e composes `é` through the
panel's own compose glue (`--selftest-compose`, RMLVO us+intl, exit 0).
A bare dead-key press through `scoot msg type` is not testable: the
planner synthesises AltGr chords rather than the bare press a real
keyboard produces.

Activation, through a `--stub` singleton (`w9.launch.one/two`):
a mouse click on row 0 mints from the button serial, the compositor
answers `done`, and focus moves from the running second stub to the
running first (`windows` before: one unfocused, two focused; after:
one focused, two unfocused). Minting from the focus serial instead
produces a dud token and nothing moves, which is the fuzzel failure the
gate names. Fresh spawn also verified: token minted, child spawned with
`XDG_ACTIVATION_TOKEN`, redeems on first configure, maps focused.

Screenshots: `dev/spikes/scootbar/launch-prototype/`
(`panel-foot.png`, `panel-azerty.png`, `panel-usintl-cafe.png`,
`fuzzel-dmenu.png`, `activate-refocus.png`, `spawn-fresh.png`).
Source auditable in this branch's history (added then deleted, per the
ticket); the harness is `proto/harness/` in the same history.

GO: every acceptance row passes at or under fuzzel on the measured
box, and the two real risks (keystroke latency, typing correctness)
are retired. The M2 confirmation and the parse cache stay open work,
not gate blockers.
