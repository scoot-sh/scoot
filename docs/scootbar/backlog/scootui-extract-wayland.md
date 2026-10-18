---
title: "Extract `scootui`'s Wayland half: outputs, layer surfaces, shm, the poll loop (move only)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M7"
---

# Extract `scootui`: the Wayland half (move only)

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#scootui-library-cratesscootui).
Serves **both**.

## What to do

A mechanical move behind a `wayland` feature, with **nothing new**:

- `outputs.rs`, `density.rs`, `region.rs`, `daemon/surfaces.rs`, `daemon/canvas.rs`;
- the Wayland half of the `poll` loop;
- `tests/common/` (the headless scoot and sway harness and the screenshot readers), as a
  test-support module.

New lifecycle code is `scootui-surfaces-on-demand`, a separate PR, so this one stays a
no-drift move.

## Acceptance

- scootbar's integration suites pass on headless scoot and sway.
- Ratchet rows within noise.
- `git diff -M` shows renames with only visibility and path edits.
- scootbg not moved (its release gate).

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

On-demand surfaces, the `xdg_toplevel` kind, the memfd name parameter.

## Resolution

Shipped by `refactor/scootui-extract-wayland` (PR #TBD). A mechanical move
with no logic edits, verified by unchanged snapshots, unchanged
`count_allocations == 0` tests, and ratchet rows within noise (below).

Moved, each as a `git mv` rename with only visibility and path edits:

- `region.rs`, `density.rs`, the per-output model `outputs.rs` and the bar
  geometry `bar.rs` into `scootui` (always built, no new dependencies);
  `Size` joined `OutputId` in `scootui::output`, and the output-name bound
  `MAX_NAME` moved with the model from `policy` (which re-exports it).
- The poll loop's Wayland primitives (`flush`, `connection_gone`,
  `read_after_poll`) into `scootui::wayland` (new `wayland` feature, adding
  only `wayland-client` and `rustix/event`, both already in the tree).
- `tests/common/` into `scootui::test_support` (new `test-support`
  feature), with `scootbar/tests/common.rs` a ten-line shim supplying the
  app binary. The harness takes the app binary as a parameter now
  (`Session::scoot/sway` and `Scratch::command` take `app: &Path`), because
  `CARGO_BIN_EXE_scootbar` only exists in a test target, never in a
  library; the `scootbar` method names, the `sbart-` prefix and the
  `SCOOTBAR_*` variables stay verbatim for a consumer to rename.
- The W10 scale stand-ins in `scootui`'s paint and popup tests now use the
  real `density::Scale` (the local `device` helper stays, standing in for
  `render::device`, which stays in scootbar).

Deviations from the file list, all forced by the no-logic-edits rule:

- `bar.rs` moved too (unlisted): `outputs.rs` takes `&Bar` in
  `surface_size`/`plan`, `Bar` is dependency-free, and moving it keeps the
  move verbatim where re-signaturing those functions would invent API.
  `bar.rs` keeps no Wayland; the design doc's "bar.rs stays" line is
  superseded for the geometry (its parsing rides along to avoid inventing
  a split).
- `daemon/surfaces.rs`, `daemon/canvas.rs`, `daemon/wayland.rs` (`State`,
  `connect`) and the poll loop's body stay in scootbar: they are written
  against the bar's content and render types (`Scene`, `Style`, `Record`,
  `Placed`, `OutputView`, `Placement`, `Pointer`, the popup and event
  state). Moving them verbatim is impossible (a library cannot use the
  binary crate's types), and parameterizing them is new abstraction, which
  this ticket forbids. They move when `scootui-surfaces-on-demand`
  generalizes the lifecycle; this PR moves the pure model and the loop
  primitives that lifecycle will build on.
- `scootui` gains a `test-support` feature alongside the ticket's
  `wayland` one, and CI's scootui clippy/nextest loops cover both.
- One lint allow, forced by the move: `Outputs::len` (test-only) newly
  trips `len_without_is_empty` now that `outputs` is a public module.

Ratchet (M6 method, dev VM aarch64, release builds, default bar with
DejaVu under headless scoot, 14 s settle, 60 s window; against `main` =
`2b0f9ce74`, crates-identical to the rebase base):

- Stripped size: main 3019488 B vs head 3019488 B (identical).
- `.text`: main 2444168 B vs head 2443400 B (-768 B, -0.03%).
- Idle: main RSS 4456/4632 PSS 2924/3088 wakeups Δ2 fds 8;
  head RSS 4456/4632 PSS 3000/3088 wakeups Δ2 fds 8. RSS identical at
  both ends, PSS within sharing noise, wakeups and fds identical.
- No waiver needed. scootbg untouched (`git diff --name-only` shows no
  `scootbg` file).
