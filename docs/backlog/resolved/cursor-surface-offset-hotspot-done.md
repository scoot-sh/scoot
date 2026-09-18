---
title: "`wl_surface.offset` on a cursor surface moves the hotspot — DONE (flexwm-side fix; overturns the PR #106 needs-upstream triage)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `wl_surface.offset` on a cursor surface moves the hotspot — DONE

PR #106 triaged this as NEEDS-UPSTREAM ("Smithay never adjusts it", "the
fix belongs in Smithay's set_cursor/commit path"). Re-derived
independently for this ticket, that triage was wrong about where the fix
belongs: Smithay core indeed never adjusts the hotspot, but Smithay's own
reference compositor does it compositor-side, and so can flexwm. The
NEEDS-UPSTREAM verdict is overturned; the fix landed here.

## Protocol (cited, not assumed)

`wayland.xml`, `wl_pointer.set_cursor`:

> On wl_surface.offset requests to the pointer surface, hotspot_x and
> hotspot_y are decremented by the x and y parameters passed to the
> request. The offset must be applied by wl_surface.commit as usual.

So: `hotspot -= offset`, applied at commit. (`wl_surface.offset` itself is
double-buffered state — "Surface location offset is double-buffered
state, see wl_surface.commit" — and role-specific in effect: "The exact
semantics of wl_surface.offset are role-specific.")

## Pinned-rev sources (all re-verified, not inherited)

- Smithay core writes `CursorImageAttributes.hotspot` only from
  `set_cursor` (`src/wayland/seat/pointer.rs`, `SetCursor` arm: converts
  with `to_logical(client_scale)` and stores). Nothing in core touches it
  on offset or commit — the half of PR #106's claim that is true.
- Smithay core stores the offset as `SurfaceAttributes::buffer_delta`:
  `wl_surface::Request::Offset` writes `pending().buffer_delta`
  (`src/wayland/compositor/handlers.rs`, converted with
  `to_logical(client_scale)` — the same conversion as the hotspot, so the
  two operands already agree in Logical space at any scale); `attach(x, y)`
  maps the same way; `Cacheable::commit`/`merge_into` move it to `current`
  on commit. Nothing in `src/backend` or `src/desktop` reads it — at this
  rev the renderer does *not* shift content by the delta; every consumer
  is compositor-side.
- The compositor-side consumer exists: `anvil/src/shell/mod.rs`
  (`commit_override`) does `cursor_image_attributes.hotspot -=
  buffer_delta`, taking the delta from `current().buffer_delta`, gated on
  the committed surface being the active `CursorImageStatus::Surface`.
  That is the whole fix shape, and it lives outside Smithay core — which
  is why NEEDS-UPSTREAM was the wrong disposition.

## What landed

- `Cursor::note_surface_commit` (`compositor/cursor.rs`): the anvil hook
  for flexwm. Gated on the committed surface being the active cursor
  image, takes this commit's `buffer_delta` from `current`, and
  decrements the hotspot. Two deliberate deviations from anvil's two
  lines, both pinned by tests:
  - saturating subtraction, not `-=`: both operands are
    client-controlled `i32`, so `i32::MIN - 1` is one malicious commit
    away, and a plain `-=` panics a debug build — taking every client's
    unsaved state down with the compositor.
  - lock-poisoning and missing-data read as "no adjustment", the same
    stance as the existing `surface_hotspot`.
- One call site: the end of `CompositorHandler::commit`
  (`compositor/handlers.rs`), unconditional — the method itself returns
  after one enum match plus a pointer comparison unless this commit is
  the active cursor surface's, so ordinary commits pay one predictable
  branch and no allocation. The redraw is already covered: `commit()`
  requests a render unconditionally.
- Four harness tests (`compositor/cursor/tests.rs`), driving a real
  client (`surface.offset` + `commit`) and asserting rendered pixels:
  basic offset + accumulation incl. a negative hotspot; offset after a
  re-`set_cursor` applying to the *new* hotspot; `i32::MIN` + offset
  saturating instead of panicking/wrapping; and the negative control
  (offset on the focus surface leaves the cursor unmoved).
- Harness change the tests needed: the test client bound `wl_compositor`
  at version 4, whose surfaces have no `offset` (a version-5 request).
  Now `version.min(5)`; Smithay advertises version 5 for the global.

## Evidence

Fail-first (dev VM, new tests against unfixed code,
`XDG_RUNTIME_DIR=/tmp/xdgrt cargo test -p flexwm cursor::tests`): 27
passed, 2 failed — `a_surface_offset_moves_the_hotspot_with_the_image`
and `an_offset_after_a_new_hotspot_applies_to_the_new_hotspot`, both on
the stale image position (e.g. `left: [255, 128, 0, 255], right: [32,
64, 224, 255]` at the new bottom-right: clear where the client color
should be). The saturation and negative-control tests passed pre-fix as
expected — they pin behavior that only discriminates against a naive
(unadjusted) or wrapping implementation, not against the old ignore.

Post-fix (dev VM, same command): 29 passed, 0 failed; broader `cursor`
filter 60 passed, 1 ignored.

Full standard set on the dev VM at the implementation commit:
`cargo test -p flexwm` (905 passed, 1 ignored, plus 3 passed),
`cargo nextest run --workspace` (1010 passed, 1 skipped),
`cargo clippy -p flexwm --all-targets -- -D warnings` (clean),
`cargo fmt --check -p flexwm` (clean — one local `cargo fmt` reflow of
long assert lines; the 9p mount is read-only from the VM side),
`scripts/smoke-test.sh` (17 `ok`, no failures).

## Edge cases, decided

- **Offset after cursor set / re-set**: the hook reads `current` every
  commit and `set_cursor` rewrites the hotspot outright, so each offset
  applies to whatever is current — pinned by the re-set test.
- **Hotspot at the surface edge / negative / extreme**: pure `i32`
  arithmetic, saturated; off-canvas in either direction rather than
  wrapped — pinned, including the still-works-afterwards shape.
- **Fractional-scale interplay**: no live test, by construction — Smithay
  converts *both* operands with `to_logical(client_scale)` at request
  time, so the subtraction is in Logical space at any scale, and the
  output-scale half (`element_location` scaling pointer and hotspot
  together) is already pinned by the existing pure tests, including the
  fractional one. The harness fixes `output_scale` at 1.0, so a live
  fractional test would pin nothing the two halves don't already cover.
- **Subsurfaces / non-cursor surfaces**: gated on identity with the
  active cursor surface, mirroring anvil — pinned by the focus-surface
  negative test.
- **Tablet `set_cursor`**: writes the same `CursorImageSurfaceData` the
  hook reads, so the same commit path covers it; flexwm has no tablet
  input path to exercise it with (see `tablet-v2`).
- **Hot path**: one enum match + pointer comparison per surface commit,
  no allocation; the render loop is untouched. Suite timing before/after
  (0.69–0.80s vs 0.79–0.84s for the cursor filter) overlaps — noise, as
  expected for a single predictable branch.

## What this deliberately leaves open

- No README change: a protocol-correctness fix with no new config,
  keybinding, CLI flag, or IPC surface — stated explicitly rather than
  silently skipped.
- `docs/backlog/protocols/protocol-gaps-niche.md` sub-item 9 keeps its
  history (closed NEEDS-UPSTREAM in PR #106) with a forward pointer to
  this record; the bundle's other eight dispositions stand.
