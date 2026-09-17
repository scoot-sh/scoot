---
title: "`--width`/`--height` are unbounded `i32`s (LOW, operator-supplied). — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--width`/`--height` are unbounded `i32`s (LOW, operator-supplied). — RESOLVED

## What it said

`--width`/`--height` are unbounded `i32`s (LOW, operator-supplied).
Found while bug-bashing item 12 and deliberately left out of it. Item 12(b)
bounds a *client's* `min_size` to the output's usable area, and 12(c) bounds
the gap, but the output's own size still comes straight from
`cli.rs`'s `number("--width", ..)` with no range check — so
`--width 2000000000` can overflow the same `x + width` in `arrange.rs`'s
on-screen test that 12(b) closes for minimums, plus `Rect::right()`/
`bottom()` wherever those are read. Lower priority than the four in item 12
because it needs the operator to pass an absurd flag to their own
compositor rather than a client or a config file to declare one, but it is
the same family of fix (clamp at the read site, with a documented bound)
and would close the last unbounded input to the layout's arithmetic. A
realistic bound is whatever DRM itself can report for a mode, with room to
spare. Two more facts `flexwm-reviewer` found while reviewing item 12,
worth fixing alongside this rather than separately: an absurd `--width`
doesn't just overflow directly — since 12(b)'s `min_size` limit is
*derived from* the output's usable area, a huge enough output makes that
clamp effectively vacuous (the limit becomes ~2×10⁹, which bounds nothing
real); and `Rect::inset`'s `self.x + by`/`self.y + by` are unguarded even
though its `w`/`h` arms already floor at 0 — the same function, only half
hardened.

## Resolution

**Bound: 65535 per axis, refused at parse.** `cli::MAX_OUTPUT_DIMENSION`
with a `dimension()` reader replacing the unbounded `number()` call for
both flags. Three legs to the number, all recorded rather than reasoned:
the kernel's uAPI `drm_mode_modeinfo` stores `hdisplay`/`vdisplay` in
`__u16` (nothing DRM can report exceeds it, verified against
torvalds/linux `include/uapi/drm/drm_mode.h`); real hardware sits far
below (8K is 7680 wide, the widest 16K prototype 15360 — the bound is ~4x
past that); and `--mode WxH` in the same file already parses `(u16, u16)`,
so one grammar for "a display size" covers every flag.

**Refuse, not clamp** — every other invalid flag in `cli.rs` is an
`Error::Invalid`, `--mode` refuses its own `0`, and a typo'd size
silently running at a different size is the worse surprise. `0`,
negatives, `65536` and `2000000000` are all `invalid --width` with exit 1
(proven live on the built binary); `1`, `800`, `7680`, `15360` and `65535`
parse untouched.

**The chain closes, with the worst case stated.** The largest flaggable
output is 65535x65535, doubled to 131070 a side at the `[output] scale`
floor of 0.5; `hint_limit` (item 12b) is therefore at most that per axis,
and the largest dimension-derived layout sum past it (`available + gap`)
sits ~15000x below `i32::MAX`. Pinned by a test asserting both ends: the
vacuous ~2e9 limit a 2e9-wide area yields (why the bound lives in the CLI,
not in `hint_limit`), and the real 65511 limit at the flaggable maximum.

**Fully hardened, not just the named sites.** The sibling audit found and
fixed three more of the same family, each fail-first (debug panics
pre-fix, all recorded): `Rect::right()`/`bottom()` now saturate (the
`intersection` doc's "unchecked add" wording corrected — the `i64` math
stays, since saturation still loses the true edge); `Rect::inset`'s origin
and doubled-margin arms saturate; `scroll_into_view`'s two edge sums
saturate; and `place_workspace`'s `usable.x + start - view_x` / `x + width`
saturate (reachable past the flag bound only through an absurd configured
proportion saturating `column_width`'s float cast first — pinned by its
own test). Zero behavior change for every in-range input: saturation is
the identity where nothing overflows.

**Audited and deliberately left alone.** `gap * (windows - 1)`,
`distribute`'s taken-sum and the `y` accumulation are window-*count*
derived, not dimension derived — bounded by live Wayland objects (memory
DoS arrives thousands of windows before the arithmetic does), the same
standing disclosure `Config::MAX_GAP`'s docs already carry. The other two
output-size sources: `--tty` hotplug is DRM-reported, hence `u16`-bounded
by construction; `--nested`'s first host configure trusts its host (the
user's own compositor, the same trust as every other configure value),
and an absurd size there dies loudly in `create_backend` before the core
ever hears it (`resize_output` returns `false` ahead of `OutputChanged`).

No hot-path benchmark: the CLI check runs once at startup, the saturating
ops are branchless single instructions on paths already doing the add.
`flexwm-core` stays platform-independent — no Wayland or I/O in the
change; its full suite (incl. the randomized invariant tests) is green
under both `cargo test` and `nextest`.
