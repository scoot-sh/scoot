---
title: "`Cursor::element`'s fallback path allocates a one-element `Vec` every rendered frame (LOW) — CLOSED DELIBERATE with measured numbers."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Cursor fallback one-element `Vec` — CLOSED DELIBERATE, measured

PR #106 triaged this as a deliberate non-entry on the grounds that the
allocation matches local convention and "measured nothing when the review
filed it". Re-derived independently with fresh numbers rather than relaying
that claim — and the re-derivation agrees, with the rate analysis below as
the record. No compositor code changed; two pin tests lock the shape.

## Which allocation fires at what rate, in bytes

`Cursor::element` (`crates/flexwm/src/compositor/cursor.rs`) has four
shapes, and only one of them allocates flexwm-side:

- **Fallback** (default cursor, drawn shapes, or themed image — all three go
  through the same `vec![CursorElement::Fallback(element)]` line): exactly
  one allocation of exactly one element per call. Measured
  `size_of::<CursorElement<PixmanRenderer>>() == 448` bytes, returned `Vec`
  has `len == 1, capacity == 1`, so 448 bytes of payload per dirty frame
  (+ the allocator's own header), no slack, no over-allocation.
- **Themed** (real xcursor artwork): the same single line, the same 448
  bytes. Not a second allocation.
- **Hidden**: `Vec::new()`, `len == 0, capacity == 0` — zero bytes,
  pinned.
- **Surface** (client-supplied cursor tree): Smithay's own
  `render_elements_from_surface_tree` returns a `Vec`, allocated inside
  Smithay per call. Unavoidable from this side without forking Smithay's
  surface-tree walk; out of scope by construction.

The rate is **not** "60Hz". `render()` runs only when `needs_render` is set
(`headless.rs::render` early-returns otherwise), the frame timer drops
itself at idle (`frame_tick` → `Drop`), and pointer motion only marks the
frame dirty (`input.rs::move_pointer_to` → `request_render`) — motion at
500–1000Hz coalesces to at most one render per 16ms tick. So the fallback
allocation fires **0 times/sec at idle** and at most **~62.5 times/sec
during continuous pointer motion or other damage under `--tty`** (the only
backend that draws a cursor). Worst case: 62.5 × 448 B ≈ **28 KiB/s of
transient garbage**, each allocation living for one frame assembly.

## What it costs, against the frame budget

Dev VM, release, 20,000 fallback `element()` calls (two runs):

| run | mean (raw) | p50 (raw) | p99 (raw) | loop+clock overhead |
|-----|-----------|-----------|-----------|---------------------|
| 1   | 193ns     | 167ns     | 375ns     | 95ns                |
| 2   | 180ns     | 167ns     | 250ns     | 60ns                |

Net per call ≈ **~90–120ns** (alloc + import-element build + free). Against
a 16ms frame budget that is ~0.0007% of one frame; against the full pixman
render the element feeds (~44–47µs mean for an 800x600 full redraw of that
same element, measured alongside on the same machine/mode) it is ~1/400th.
At the maximum 62.5 fires/sec the allocation costs ~7µs of CPU per second.

## Why the ticket's cheaper shape loses, measured

The ticket names appending into a caller-owned `&mut Vec`. A local
caller-owned `Vec` does **not** eliminate the allocation — it moves it — and
it allocates *more* bytes: one push into a fresh `Vec` grows by Rust's
minimum non-zero capacity (4 for a 448-byte element type), i.e. **1792 bytes
vs the current 448**, measured in the same probe run
(`push-into-fresh-Vec cap = 4`). Only a persistent buffer reused across
frames would truly go to zero, and that is the larger refactor the ticket
already names: a new field (on `Backend` — `Cursor` itself is generic over
`R` and cannot hold `Vec<CursorElement<R>>`), a signature change to
`element()`, and borrow reasoning inside `render()`'s hottest assembly — to
save ~7µs/s during active motion only, while `render()` keeps allocating
several per-frame `Vec`s regardless (`Decorations::elements` returns a fresh
owned `Vec` every frame, `window_elements` and the final `elements` list are
built per frame, and the surface path allocates inside Smithay on every
frame). The ripple genuinely costs more than it saves, which is exactly the
ticket's own close condition.

## Pins

Two tests in `cursor/tests.rs`, both confirmed fail-first against a
deliberately wasteful fallback (`Vec::with_capacity(4)` + push fails the
capacity pin with `left: 4, right: 1`):

- `fallback_element_is_one_exactly_sized_allocation` — `len == 1`,
  `capacity == 1`.
- `hidden_cursor_allocates_nothing` — empty, `capacity == 0`.

## Revisit conditions

- `render()` goes allocation-free as a whole (then the cursor Vec is no
  longer 1-of-N and a persistent buffer has company worth sharing it with).
- A second renderer backend lands and element Immix/pooling arrives with it.
- Anyone reproduces a frame-time percentile delta from this line with real
  numbers — the prior (12+4 interleaved reps, no difference) and this
  pass (~1/400th of the frame it feeds) both say they won't.

## Evidence

- Probe runs (dev VM, release, uncommitted working tree on top of `523bbb0`):
  `XDG_RUNTIME_DIR=/tmp/xdgrt cargo test --release -p flexwm
  probe_cursor_element_alloc -- --nocapture` — raw `PROBE` lines in the PR
  report (sizes, capacities, both timing runs, 800x600 render context).
  The probe test was removed after measuring; only the two pins above ship.
- Fail-first (dev VM, debug): wasteful-fallback mutation → new pin fails
  (`left: 4, right: 1`); reverted → green.
- Full standard set on the final tree: `cargo test -p flexwm`, `cargo
  nextest run --workspace`, `cargo clippy -p flexwm --all-targets -- -D
  warnings`, `cargo fmt --check -p flexwm` (fmt Mac-side per the 9p
  constraint), `scripts/smoke-test.sh` — exact outputs in the PR report.
- No `README.md` change: no user-facing surface (internal allocation shape
  only, no config/keybinding/CLI/IPC/behavior delta).
