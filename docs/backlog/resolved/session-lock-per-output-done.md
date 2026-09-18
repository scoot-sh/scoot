---
title: "Lock surfaces are per-output and flexwm has one output — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Lock surfaces are per-output and flexwm has one output — DONE

## The entry as filed

> Lock surfaces are per-output and flexwm has one output (item 18).
> `new_surface` honours the `wl_output` the client named but falls back to
> the single output; `configure_all` resizes them all together. One more
> site for the multi-output list `headless.rs`'s `OUTPUT_ID` doc keeps.

## Verify-first findings (2026-09-18, pins + one doc paragraph, no behavior change)

Drove the lock sequence in-harness before writing anything, taking the
ticket's per-output question as the thing to verify: what the protocol
requires per output, and what happens today when a locker creates surfaces.

**What the protocol requires per output.** `locked` must not be sent until a
locked frame has been presented on *all* outputs. flexwm confirms on the
first blanked frame (`confirm_lock` on headless/nested, the carrying flip's
vblank under `--tty` per PR #84) with no surface-count gate anywhere on the
path — with exactly one output that single frame *is* "all outputs", so the
requirement holds trivially. The zero-surface half was already pinned
(`the_locked_event_waits_for_a_blanked_frame`,
`locking_an_empty_session_blanks_and_keeps_blanking` — every `Step::Lock`
waits for `locked`, so each is a confirm-with-no-surfaces proof); no new
test repeats it.

**What happens today with several surfaces.** One empirical correction to
the entry's framing, found by running rather than reading: Smithay refuses
`get_lock_surface` naming the *same* `wl_output` resource twice
(`already_locked`), so "several surfaces" cannot arise the obvious way —
only by binding the global again and naming the same physical output
through a different resource, which is the shape the sibling ticket
(`lock-surface-duplicate-wl-output.md`, still open) owns. The admission
itself is unchanged by this item; what lands here is what every admitted
surface is then configured and drawn as:

1. each is configured to the single output's size (`new_surface` resolves
   the named output, falls back to the one output, configures from
   `logical_size`);
2. all are drawn onto that output at its origin, first-created on top, with
   the keyboard on the first current surface;
3. a resize reconfigures every one of them together (`configure_all`).

So the honest resolution is the ticket's option (a): pin single-output
correctness, record multi-output as the revisit condition. Multi-output
itself is a separate epic and is not built here.

## Resolution (pin + document, 2026-09-18)

- New suite `session_lock::tests::per_output` (2 tests), plus two harness
  steps in the session-lock client: `LockSurfaceSecondBind` (names the one
  output through the up-front second `wl_output` bind — every test client
  now holds two binds of the one global) and `RedrawLockSurface` (acks the
  latest configure and redraws at it, what a real locker does on resize;
  `ReattachLockBuffer` replays the already-acked size and a post-resize
  commit through it is rightly killed for commit-before-ack).
  - `every_lock_surface_is_configured_to_the_single_output`: two surfaces
    from one lock (magenta, then green) render whole-screen magenta
    (first-created on top), keyboard on `Lock(0)`; destroying the first and
    letting the compositor redraw by itself reveals the second whole and
    green, keyboard on `Lock(1)` — both were configured to this output and
    composited all along.
  - `resizing_the_output_reconfigures_every_lock_surface`: shrink to half
    canvas, white-box assert on both surfaces' pending sizes (the harness
    readback covers the old canvas, so a missed surface is invisible in
    pixels — documented in the test), then ack-and-redraw both at the new
    size with the client still alive and `locked: 1`.
  - Fail-first, dev VM: with `new_surface` neutered to keep only the first
    surface, test 1 fails at the reveal (`pixel (0, 0) is [0, 0, 0, 255],
    expected [32, 224, 32, 255]`); with `configure_all` neutered to
    `.take(1)`, test 2 fails at the white-box leg (`surface 1 ... left:
    120x120, right: 60x60`). Both restored, both green.
- `headless.rs`'s `OUTPUT_ID` doc now enumerates the four session-lock
  per-output sites (named-output-with-fallback, single-size configure-all,
  first-frame-is-all-outputs confirm, single-origin composite + first-surface
  keyboard) with the per-output shape each one needs — the list the ticket
  asked this site to join.
- README's "One output" lock bullet sharpened to the pinned semantics
  (shared size, first-on-top + keyboard, first-frame confirm).
- Not touched: teardown, vblank-confirm (#84) and blank-timing (#101) pins
  all still green untouched; the duplicate-bind admission question stays
  with its own open ticket; no hot path touched (production diff is a doc
  comment), so no before/after benchmark applies.

## Evidence

- New tests post-fix, dev VM: `cargo test -p flexwm --bin flexwm
  session_lock::tests::per_output` — 2 passed. Neuter runs as above (1
  failed each, at the stated legs, nothing else run in those states).
- Full set post-fix, dev VM: `cargo test -p flexwm` (883 passed, 0 failed),
  `cargo nextest run --workspace` (988 passed, 1 skipped),
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh` (17 ok).
