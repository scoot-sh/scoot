---
title: "Advertise a minimal, honest `zwp_linux_dmabuf_v1` so quickshell's `ScreencopyView` leaves the readiness gate — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Advertise a minimal, honest `zwp_linux_dmabuf_v1` — RESOLVED.

## The entry as filed

Filed from the measurement that closed the gating question of
[`screencopy-shell-thumbnails-fallback.md`](./screencopy-shell-thumbnails-fallback-done.md)
as YES on 2026-09-17: advertising the global flips quickshell 0.3.1's
`WlBufferManager::isReady`, and the already-shipped ext output-capture path
then displays over shm — with an advertisement whose every datum is true but
whose import-capability implicature is overstated (that file's one-sentence
honesty verdict). This item is the small advertisement follow-up the ticket
names: no capture work, no protocol work beyond the one global.

## What to build

Essentially the probe, productionised: Smithay's `DmabufState` +
`DmabufHandler` at the pinned rev, `create_global_with_default_feedback`,
feedback = `main_device` (this machine's real scanout `dev_t`; `0` where no
DRM node exists, logged) + the formats the shm pipeline actually serves
(`Xrgb8888`, `Argb8888`) × `LINEAR`, and `dmabuf_imported` answering
`failed` — the protocol's own "cannot import for implementation-dependent
reasons", which is the only truthful answer a pixman/shm compositor has.
The probe diff below is the starting point, not the shape: it needs the
`PROBE` markings and `warn!` scaffolding replaced with the module's real
logging voice, a home inside `screencopy.rs` (or beside it, if it sprawls),
tests for the parts that are testable without a GPU (global advertised,
feedback well-formed, `create_params` answered `failed` rather than hung or
crashed), and the same trust-model note `screencopy.rs` already carries for
its capture globals.

## Acceptance (measured, not reasoned)

- Re-drive `/var/tmp/qs-toplevel-probe.sh` against the shipped tree:
  `outview hasContent=true`, one surviving `no recording context` (the
  `Toplevel` view — still no hyprland protocol, deliberately), zero
  `create_params`.
- The fallback matrix the probe did *not* run: at least one genuinely
  dmabuf-allocating client (video player, Qt dmabuf path) sharing a session
  with the global advertised — expected `failed` → shm fallback, proven on
  the wire rather than assumed. `foot` (shm) and `grim` already proven
  unaffected; they are the floor, not the matrix.
- The probe VM had `/dev/dri/card0`, so the `renderD128 → 0` fallback ladder
  never fired — yet `main_device=0` (no DRM node) is plausibly *the*
  production shape on GPU-less container targets (webtop). Exercise the
  no-node path explicitly, plus a `--nested` and a `--tty` re-drive of the
  acceptance above, before calling the advertisement done. (Added
  post-merge from PR #60's review.)
- `README.md`'s protocol inventory gains one row: what is advertised, what
  `failed` means here, and the one-sentence honesty statement from the
  measurement record. No config knob, no CLI flag, no IPC surface.

## Explicitly not in scope

- `hyprland-toplevel-export-v1`: still a compositor-specific protocol, still
  needs its own measured justification (the fallback entry's standing rule).
- Per-window thumbnails beyond readiness: that is the fallback entry's
  second half, possibly shell-side-only work, and it stays there.
- Empty format tables or omitted `main_device`: the measurement record
  explains why (client `mmap`/`qFatal` behavior); do not "simplify" into
  either without re-driving.

## Resolution (PR #60, merged 2026-09-17 as `6733905`)

Shipped as specified: new `compositor/dmabuf.rs` (device ladder
card0 → renderD128 → 0 with once-per-boot logging, feedback builder,
`DmabufHandler` answering `failed`) + 8 real-client wire tests;
`Screencopy` gains the `DmabufState` field (constructed once per process,
no mid-life teardown); trust-model note extended; README inventory row +
honesty statement (plus two stale lines the change caused); smoke test
asserts the advertisement via `wayland-info`. Global version 6 (Smithay
fixes it at 6 when default feedback exists); all feedback events are
v4-baseline, so the observed v5 (quickshell) and v4 (mesa) binds are both
well-formed.

Acceptance, all measured on the dev VM (shipped binary sha256
`07cdb714…2c38`, independently reproduced by the reviewer via a force-clean
rebuild):

| scenario | outview hasContent | no-recording-ctx | create_params | main_device |
|---|---|---|---|---|
| `--headless` | true 1200x800 | 1 (Toplevel view) | 0 | 57856 card0 |
| `--headless`, no DRM (`unshare -rm` + tmpfs) | true 1200x800 | 1 | 0 | 0 "no DRM node", once |
| `--nested` (cage) | true 1280x720 | 1 | 0 | 57856 card0 |
| `--tty` (seat checked free, released after) | true 1280x720 | 1 | 0 | 57856 card0 |

The `hasContent=true` sessions necessarily exercised the advertisement
(quickshell's `createContext` comes only from `onBuffersReady` ← `isReady`
← dmabuf `done()`). Fallback matrix, stated plainly: `foot` and `grim`
unaffected everywhere; quickshell binds v5 → `done` → shm display; mesa
binds v4; **no genuinely dmabuf-allocating third-party client can run on
this VM** (no 3D: kms_swrast/llvmpipe) — the `failed` path is proven by
wire tests (well-formed, garbage-modifier, garbage-format imports) instead
of a live fallback, an environment limit recorded here, not a refusal.
Full set green (697 / 793 at `599be4e`).

Independent review came back with no blocking findings and expressly
declined to gate on GPU hardware: a GPU run would execute flexwm's
byte-identical dispatch path and then test Mesa's fallback, not flexwm's
code; the scenario that would change that judgment is evidence of a real
client that dies or hangs on a well-formed `failed` event (a client bug,
not a flexwm one). Post-merge follow-ups on `main`, both comment/docs-only:
the `dmabuf.rs` "protocol reserves 0" attribution corrected to the
`dev_t`/kernel convention, and the no-DRM-node + nested/tty acceptance
note above added to the (already-shipped) follow-up entry at review time.
