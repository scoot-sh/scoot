---
item: "13"
title: "Config override for the fallback cursor's size and color"
status: "done"
area: "rendering"
pr: null
commit: null
---

# Config override for the fallback cursor's size and color

`[appearance]` gained `cursor_size` (integer, default `16`) and
`cursor_color` (`"#rrggbb"`/`"#rrggbbaa"`, default `#ffffff`), resolved
through the existing `AppearanceConfig`/`into_appearance` path — so a
malformed color degrades to that one field's default with a warning,
exactly like the three ring/background colors, rather than invalidating
`[appearance]`. `cursor.rs`'s `const SIZE` and its hardcoded
white/black pixels are gone: `generate_bitmap(size, fill, outline)` is
still pure and still returns the same bytes for the defaults, and
`Cursor::default()` became `Cursor::new(size, color)`, called once from
`State::new`. Built once at startup, never rebuilt — nothing in this
project reloads config, and this ticket deliberately did not invent a path
for it.

**No theme name, by decision, not omission.** Honoring
`CursorImageStatus::Named` properly needs a real xcursor asset or real
xcursor-file loading; niri's assets are GPL and Adwaita's aren't MIT-clean
(`CLAUDE.md`), so there is nothing in this repo to load and sourcing one is
its own concern. `cursor.rs`'s module doc now says that at the point
someone would look for the option.

Three decisions worth the reviewer's attention:

- **The outline is black at the fill's own alpha, not a second config
  field.** Its job is to keep the shape legible against similar-colored
  content, which a configurable outline could only undo; at the default
  opaque fill it is byte-identical to the fixed black outline this shape
  always had, and matching the alpha is what stops a translucent
  `cursor_color` from rendering as a solid black triangle outline around a
  see-through middle.
- **`MIN_CURSOR_SIZE = 4`, `MAX_CURSOR_SIZE = 256`, justified
  arithmetically** the way `Config::MAX_GAP` is, not by taste. Below 4 the
  shape has no interior pixels at all (the outline owns the left column and
  the diagonal, so the fill only exists where `0 < x < y`: one pixel at
  size 3, none below). The upper bound is about the allocation, not looks:
  the bitmap is `size * size * 4` bytes, which is 256 KiB at the cap but
  **overflows `i32` outright at `i32::MAX`** — a debug panic inside
  `generate_bitmap`, a wrapped length in release, and the same product
  appears again in Smithay's own `assert!(mem.len() >= stride * size.h)`
  in `MemoryBuffer::from_slice`. `Appearance::clamp_cursor_size` is the
  pure, separately-tested clamp (`Config::clamp_gap`'s shape);
  `Appearance::clamped` applies it and warns, and `Cursor::new` applies it
  again at the allocation itself, the way `ring_rects` re-checks
  `width <= 0` rather than trusting its caller.
- **`Color::to_argb8888` is a new single boundary for straight-alpha →
  premultiplied BGRA bytes**, next to the existing `From<Color> for
  Color32F` that does the same job for solid-color elements. Premultiplied
  because the pinned rev hands an `Argb8888` memory buffer to pixman as
  `a8r8g8b8` and composites with `Operation::Over`
  (`backend/renderer/pixman/mod.rs:389,605`), which is defined over
  premultiplied components.

**The test gap this closed, found while writing the tests rather than
after**: every pre-existing assertion about the cursor bitmap used white,
black or the clear color, and **white and black are symmetric under a
B↔R swap**, so nothing in the suite could have caught a byte-order
mistake in a color conversion. The new live read-back test uses `#ff8040`
(all three channels distinct) and deliberately *not* `#ff8000`, whose
reversed bytes are this harness's own `CLEAR_BGRA` exactly — a swap would
then draw the cursor in the background's color and the failure could not
tell "wrong color" from "nothing drawn".

**Tests: 20 new (221 total, against 201 on the merge base).**
`Color::to_argb8888`'s
byte order/premultiplication/saturation, the clamp at both bounds and at
`i32::MIN`/`i32::MAX`, `generate_bitmap` at a non-default size and with
non-default colors, a check that the defaults still describe the original
16x16 white-on-black shape, `Cursor::new`'s re-clamp measured through the
built element's real geometry, the config-file round trip (both fields
set, only the cursor fields set, a malformed color, an out-of-range size,
a size outside `i32`, and a gap small enough to clamp the ring but not the
cursor), and two live read-back tests through a real `State` and a real
`PixmanRenderer` — a 48px `#ff8040` cursor sampled at its outline,
diagonal, interior, last filled row, the row past it and outside the
triangle, plus a translucent `#ffffff80` one proving it blends with what
is behind it instead of replacing it.

Each of the three new live/clamp tests was confirmed non-vacuous against
two negative controls (raw output in PR #19): `Cursor::new` stubbed to
ignore the config entirely (all three fail), and `Color::to_argb8888`
returning `[R, G, B, A]` instead of `[B, G, R, A]` (the live pixel test
and the three byte-order unit tests fail, the rest pass — which is the
exact blind spot described above).

**Hardware verification** (dev VM, real `--tty` on its `virtio-gpu` KMS
device at 1600x1000, debug build — i.e. with integer overflow checks on —
all against `34514a9`, which is the whole of this item's executable code:
every later commit on the branch changes only prose and doc comments, which
`git diff 34514a9..HEAD -- '*.rs' | grep -E '^[-+]' | grep -vE '^(\+\+\+|---)' \
| grep -vE '^[-+]\s*(///|//!|//)'`
returning nothing confirms mechanically (the shorter, `grep -v`-only form
a first draft of this note used does *not* actually confirm this — it
still emits context lines and hunk headers, so seeing output from it is
not a sign the cache key is stale, only a sign of using the wrong
command; `flexwm-reviewer` caught this while reviewing item 13) — so the
evidence key still matches. Six scenarios, each a fresh compositor with its
own config, the pointer parked over the background by IPC and the frame
captured by IPC; exact commands and every raw sampled pixel are in PR
#19's description. Summary: `cursor_size = 48` + `cursor_color =
"#ff8040"` draws `rgb(255,128,64)` at its interior and black at its
hotspot/left edge/diagonal, with the last filled row at `+47` and
background at `+48`; the same binary with no cursor fields draws the
original 16x16 white shape with its boundary at `+15`/`+16`;
`cursor_size = 100000` logs the clamp warning and draws a 256px shape
(fill at `+255`, background at `+256`); `cursor_size = 0` logs it too and
draws a 4px one (its single interior pixel at `(1,3)`, background at
`+4`); `cursor_color = "not-a-color"` logs the per-field warning, draws
white, and still honors `cursor_size = 32`; and `#ffffff80` over a
`#203040` background reads `rgb(144,152,160)` at the fill and
`rgb(16,24,32)` at the outline — both exactly halfway, which is what makes
the premultiplication right rather than merely non-crashing.

Edge cases on the same hardware, same build: a 256px cursor with its
hotspot on the output's very last pixel `(1599,999)` draws its outline
there and nothing is cut wrong; at `(1500,900)` it is clipped against both
edges and still draws its interior and the diagonal that reaches the
corner; clipped against the right edge alone it fills out to column 1599.
Six adversarial absolute pointer positions (`-1`, `-100000`, `1e300`,
`-1e300`, `2147483647`, and a fractional `1599.9 999.9`) each return `Ok`,
each still capture a frame, leave the compositor alive with no panic or
error in its log, and an ordinary position still draws afterwards.

**Caveat on these logs, worth recording precisely rather than glossing
over**: every one of these runs, over SSH, logged smithay's own `Unable
to become drm master, assuming unprivileged mode` at device-open time —
`flexwm-reviewer` reproduced this independently and confirmed it isn't
specific to this item. This doesn't touch any claim above (the composited
frame is read back and pixel-sampled the same way regardless of DRM
master state, so the cursor-rendering claims hold either way), but it's
in real tension with `vm/README.md`'s claim that a `--tty` session over
SSH "takes real DRM/libseat ownership just fine." Nothing here confirms
or refutes whether master gets acquired later via libseat, only that it
isn't held at open time — an open question for whoever next needs to
trust a claim in this project that specifically depends on holding real
DRM master (VT-switch/scanout behavior, not pixel content), not
something this item's own evidence needed to resolve.
**Resolved 2026-09-13** — see the resolved DRM-master entry in the Backlog
below. Master *is* held, over SSH and from a real VT alike; the warning
means "this process may not call `SET_MASTER` itself", and the fd seatd
passes is already the master. One correction to the framing above, for the
record: master isn't acquired later by flexwm *making a successful call*
either — flexwm never itself issues a working `SET_MASTER` at all (its own
attempt always gets `EACCES`, by design, see the Backlog entry). It's
seatd's open-and-set that establishes it, and seatd's own pause/activate
logic that releases and re-establishes it across a VT switch (confirmed
live: master genuinely toggles `y → n → y` in step with a `chvt` cycle,
not held uninterrupted the whole time the process runs). `vm/README.md`'s
conclusion was right and only its stated mechanism (logind/PAM) was wrong;
both are fixed.

The item-8 client-cursor behavior is unchanged and is covered by its seven
tests passing untouched (they drive a real client and assert read-back
pixels, so "the client's image still wins over the configured fallback" is
a real assertion, not an inference): `element()` returns before it ever
touches the fallback buffer when a live cursor surface exists, and no
state is shared between the two paths.

**Benchmarked** (same jiffies-delta method as items 5/8; release builds of
`8446758` (base) and `34514a9` (new), three arms interleaved per rep so VM
drift hits all of them). The change cannot cost anything per frame in
principle — the bitmap is built once at startup and `element()` is
untouched — but a *bigger* bitmap composites more pixels every frame it is
drawn, so the cap's own cost was measured too, not just the default's.
200 pointer moves per rep, 6 reps:

| arm | base (16px) | new (16px) | new, `cursor_size = 256` |
|---|---|---|---|
| far: `(0,0)`↔`(1344,744)` | 29.67 (25-36) | 30.50 (27-36) | **42.00 (36-54)** |
| near: `(4,4)`↔`(8,8)` | 8.83 (7-11) | 9.50 (8-12) | **14.17 (12-16)** |

The default is unchanged either way (overlapping ranges, base vs new) —
also true analytically, not just by measurement: no per-frame allocation
was added, `element()` is untouched, and the persistent render buffer and
stable element `Id` are preserved. The largest cursor a config can ask for
costs *something* real: `flexwm-reviewer`'s own re-run (fixed arm order,
unrotated, so drift can load onto whichever arm runs last) found one
`new256` sample at 17 — inside `new`'s own range and below every reported
`new256` value — so the specific "+12 far / +5 near jiffies" figures above
are order-of-magnitude, not a tight measured delta; read them as "a
256px cursor costs single-digit-to-low-double-digit jiffies more per 200
moves," not as exact numbers. The direction and the reason (more pixels
composited per frame) are solid regardless, and bounded by
`MAX_CURSOR_SIZE`, which is the practical argument for having an upper
bound at all beyond the overflow one.

**A correction worth recording, because the first version of this
measurement was a no-op.** It drove the pointer with
`pointer move 100000 100000` / `-100000 -100000`, on the assumption that
IPC's `pointer move` takes a relative delta. It does not — it is absolute
and is *not* clamped to the output (that clamp belongs to
`pointer_move_relative`, libinput's path), so both endpoints were
off-screen, no cursor was composited in any arm, and all three came out
identical at ~21 jiffies. The numbers above use endpoints chosen so that
even a 256px shape is fully on screen at both (1344 + 255 = 1599 on a
1600x1000 output), and the script now screenshots both endpoints first and
prints the hotspot, an interior pixel and the pixel 255 rows down as a
witness that the thing being measured is actually being drawn.

Also at the same commit: `cargo test -p flexwm` 221/221 and
`-p flexwm-core` 45/45 on the dev VM, clippy `--workspace --all-targets
-D warnings` and `cargo fmt --all --check` clean on both the VM and
macOS, `cargo check --workspace --all-targets` clean on macOS (the
cross-platform build), `scripts/smoke-test.sh` green under `--headless`
(all 11 `ok:` checks, exit 0) against a release build of this branch, and
the README's example `config.toml` — now including the two new fields —
loaded by a real compositor with no warning or error in its log.
