---
title: "`scootui` surfaces: on-demand layer surfaces, a full-screen `xdg_toplevel` kind, memfd names"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
---

# `scootui`: on-demand surfaces

Filed 2026-10-12 in the #579 fix round, split out of the Wayland move so the move stays
mechanical. Serves **both**.

## What to do

- An **on-demand** layer-surface lifecycle: created when there is content, destroyed with its
  buffers unmapped when empty, so no surface is held at idle.
- A full-screen `xdg_toplevel` surface kind (scootgreet: cage 0.3.1 has no layer-shell).
- The memfd name parameter in `scootbg-mem`. Today every buffer is `scootbg-wallpaper`
  (`crates/scootbg-mem/src/shm.rs:173`).

## Acceptance

- A test creates and destroys an on-demand surface 1,000 times with fds and RSS flat.
- The `xdg_toplevel` kind maps fullscreen under headless scoot and sway.
- `/proc/PID/maps` shows buffers named per app.
- scootbar unchanged (its ratchet rows within noise).

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Keyboard input.

## Resolution

Shipped by `feat/scootui-surfaces` (this PR): new code, no moves, behind a
new `surfaces` feature on `scootui` (the CI scootui clippy/nextest loops
cover `surfaces` alone, and the scootbar-integration job runs the new
`--test surfaces` suite against headless scoot and sway with skips as
failures).

What landed, the minimal API the consumers need and nothing more:

- `scootui::surfaces` (new module, `crates/scootui/src/surfaces.rs` plus
  `surfaces/tests.rs`): the `LayerSpec` (overlay, anchor, size, margins,
  exclusive zone, exclusive-or-no keyboard) and `FullscreenSpec` (app id,
  title); the pure lifecycle `Demand` with `Effect`s (`Create`, `Ack`,
  `Destroy`, `DestroyAndRetry` once, `DestroyAndGiveUp`, `Destroy`),
  mirroring the bar's per-output model in miniature (stale-configure guard,
  past-`i32` sides taken as 0, output-removed with no retry, idempotent
  show/hide, a 3-strike draw-failure quieting); the glue that creates and
  destroys each kind with the fractional-scale/viewport plumbing (`LayerLive`,
  `ToplevelLive`, each with `prepare` for the viewport-destination-or-buffer-scale
  bookkeeping); and `Buffers`, a two-slot named-memfd pool (`take`,
  `released`, `clear`) keyed on size and `wl_shm` format, the popup's pool
  shape.
- `ShmBuffer::with_name` (`crates/scootbg-mem/src/shm.rs`): the memfd name
  is a parameter; `new` keeps `scootbg-wallpaper`, so scootbg is byte- and
  behavior-identical (no scootbg file touched).
- `test_support::shm_mappings_named` for the per-app maps check the popups
  ticket needs; `shm_mappings` delegates with the old name.
- `crates/scootui/tests/surfaces.rs` (new, `required-features =
  ["surfaces", "test-support"]`): the client drives the module exactly as
  the apps will. A layer surface cycles 1,000 times under headless scoot
  (create, configure, ack, two draws with the release path exercised, hide,
  destroy), plus a resize mid-run; a toplevel maps 1600x1000 fullscreen
  under headless scoot; under sway an exclusive-keyboard panel cycles and a
  toplevel maps fullscreen (`fullscreen_mode == 1` in sway's tree).
- No fuzz target was added: the tracker's hostile surface (u32 sizes,
  serials, event storms) is pinned deterministically instead (boundary
  values plus a 2,000-round seeded storm over show/hide/configure/closed/
  remove/retry asserting no panic, no double-Create, Ack echoing only a
  live serial, and hide leaving nothing live). The `#[path]` fuzz includes
  still resolve (`cargo check --locked` clean for both fuzz workspaces);
  the fixed-budget mutation runs are CI's leg on this head.

Two deviations from the first draft, both from what the compositors
actually do: the handler acks every configure at receipt (acking again in
the test body is a layer-shell protocol error, found live); and under sway
a fullscreen toplevel configures 0,0 until it maps, so the test maps a
provisional frame first, then draws at the arranged size (any real
fullscreen client does the same: draw at the arranged size once it
arrives). Sway keeps a disabled headless output bound, so an output
removal cannot be forced live headless: that edge stays unit-covered
(`output_removed`), and the attempt is recorded, not silent.

Acceptance, measured on the dev VM (aarch64, rustc 1.97.1) at the head
below:

- 1,000 layer cycles under headless scoot: fds identical before/after,
  RSS growth within the 512 KiB bound (raw: 3.35 s for the loop), maps
  show `/memfd:scootui-test` while drawn and zero lines after every
  destroy; releases arrive (the release path is exercised, not skipped).
- The toplevel maps fullscreen under headless scoot (configure
  1600x1000) and sway (nonzero configure, `fullscreen_mode == 1`), fds
  flat, maps clean after destroy.
- scootbar unchanged: no scootbar file in the diff; the release binary is
  byte-identical to main's (`cmp` clean, stripped 3019520 B both,
  `.text` 2450856 B both); idle under headless scoot (default bar with
  DejaVu, 14 s settle + 60 s window): main RSS 4340/4340 PSS 2808/2808
  vs head RSS 4360/4360 PSS 2839/2839, wakeups Δ0 both, fds 7 both
  (within one-run sharing noise; the identical bytes say the +20/+31 is
  environmental); snapshots 17 pass with no re-bless;
  `count_allocations == 0` unchanged (8 scootbar, 3 scootui + the new
  `a_hot_turn_allocates_nothing` making 4 with `surfaces` on).
- Mutation-proved: with the past-`i32` sane mapping reverted,
  `a_past_int32_configure_side_is_taken_as_zero` fails; with the
  retry-once `closed_once` reverted,
  `the_compositor_closing_retries_once_then_gives_up` fails.

What was not run: fixed-budget `cargo fuzz` mutation (no `cargo-fuzz` on
the VM; covered by CI green on this head); the scootbg rotation
integration tests fail on this VM on main too (pre-existing: the same 4
fail from pristine `origin/main`, `/dev/shm` 99% full shared-box
pressure; CI's clean runner is the gate); M2 rows (brief says not
needed); a live output-removal (cannot be forced headless, above).

No `unsafe` added anywhere (`#![forbid(unsafe_code)]` holds in `scootui`
and the test); no `unwrap`/`expect`/`panic!` in the library (only in the
test's assertions and a `debug_assert`-free no-panic path); `scootbar`'s
normal tree has no `libc`; nothing weakened to get CI green.
