---
title: "Under `--tty`, the background color is not painted where no window covers it — RESOLVED: it always was. The smoke test's background sample sat under the cursor."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Under `--tty`, the background color is not painted where no window covers it — RESOLVED

~~Under `--tty`, the background color is not painted where no window covers
it — the uncovered area stays black. `MODE=--tty scripts/smoke-test.sh`
fails one assertion: `the background pixel at (3,3) is rgb(0,0,0), expected
#123456`. No diagnosis done: the obvious place to look first is the
damage/buffer-age path, which `--tty` engages
(`Tty::next_buffer_age`/`advance_generation`) and `--headless` does not, so a
never-damaged region on the first frame would keep whatever the render target
was initialized to.~~ — **RESOLVED 2026-09-16**. Both halves of that guess
were wrong: the background is painted, over the whole uncovered area, and the
`--tty` damage/buffer-age path is doing its job. The one black pixel is the
**cursor**.

This entry was also a duplicate. `docs/backlog/testing/smoke-test-background-cursor-sample.md`
(filed from item 15's bug-bash, which had pixel-mapped the corner and got it
right) recorded the same failure with the correct diagnosis; item 17's
bug-bash filed this one independently, under `tty/`, as an undiagnosed
compositor bug. The `tty/` framing is what got picked up as the work item, so
the wrong hypothesis outlived the right one for three days. Both entries are
closed by the same one-line change; the `testing/` file is removed in favour
of this record.

## What was actually happening

`--tty` is the one backend that draws a cursor (`cursor.rs`'s module doc:
headless has no display, `--nested` shows the host's own). Nothing in the
smoke test ever moves the pointer, so it sits where it starts — the output's
origin — and `Cursor::element` draws the built-in arrow there with its
hotspot at the top-left corner. `generate_bitmap`'s arrow is
`outline` where `x == 0 || x == y`, `fill` where `x < y`, transparent where
`x > y`, and that outline is opaque black by construction (`Cursor::new`
derives it as black at the fill's own alpha). The sample point **(3,3) lies
exactly on the `x == y` diagonal**, so the check read the cursor's outline,
not the background — `rgb(0,0,0)`, exactly, with no blending, which is itself
a tell that this was never a half-painted or stale region.

`--headless` and `--nested` pass the identical assertion because neither
draws a cursor at all, which is also why this looked backend-specific enough
to blame on the one other thing `--tty` does differently (buffer age).

## The fix

`scripts/smoke-test.sh` parks the pointer over the focused window's middle
(`msg pointer move`, then `wait-idle`) before capturing, and keeps sampling
(3,3). Deliberately that way round rather than moving the sample somewhere
the cursor isn't: (3,3) is the output *corner*, which is where a real
background/edge-clamping bug would show first, and a moved sample point would
have silently encoded today's cursor placement and size into a coordinate
that has no reason to know either. The focused window (not the unfocused one)
so that even a hypothetical focus-follows-mouse could not change which ring
colour is which; flexwm has none today (`input.rs`'s `pointer_move_quietly`
never touches keyboard focus), and the comment in the script says so.

No compositor code changed. The cursor-side facts the diagnosis rests on are
already unit-tested where they live — `cursor/tests.rs`'s
`bitmap_honors_the_requested_colors` asserts the diagonal is the outline
colour, and `bitmap_hotspot_pixel_is_the_outline_color` the corner.

## Evidence

Dev VM (`ssh -p 2222 dev@localhost`, NixOS aarch64, real virtio-gpu
DRM/KMS). Binary `/var/cargo-target/debug/flexwm`, `cargo build` at
`2026-09-16 04:27:35 +0000`, from `6602808` (`main`, the commit this branch
forked from) — the fix is shell-only, so the same binary produced both the
failing and the passing run.

**1. Reproduced, at `6602808`:**

```
$ SHOT=/tmp/tty-repro.png MODE=--tty scripts/smoke-test.sh
ok: the focused window's ring pixel at (403,9) matches #ff00ff
ok: the unfocused window's ring pixel at (1197,9) matches #00ffff
BUG: the background pixel at (3,3) is rgb(0,0,0), expected #123456 (rgb(18,52,86))
```

**2. It is the cursor, proved by moving it** — flexwm `--tty` with only
`background_color = "#123456"` configured and *no client at all*, screenshot
before and after one `msg pointer move 800 500`:

```
before, pointer never moved:        after moving the pointer to (800,500):
  (0,0) -> srgba(0,0,0,1)             (0,0)     -> srgba(18,52,86,1)
  (3,3) -> srgba(0,0,0,1)             (3,3)     -> srgba(18,52,86,1)
  (8,8) -> srgba(0,0,0,1)             (8,8)     -> srgba(18,52,86,1)
  (3,8) -> srgba(255,255,255,1)       (3,8)     -> srgba(18,52,86,1)
  (8,3) -> srgba(18,52,86,1)          (8,3)     -> srgba(18,52,86,1)
  (20,20) -> srgba(18,52,86,1)        (800,500) -> srgba(0,0,0,1)
                                      (803,503) -> srgba(0,0,0,1)
```

The "before" column is `generate_bitmap`'s arrow byte for byte: outline on
the left edge and the diagonal (`0,0`/`3,3`/`8,8`), white fill below it
(`3,8`), transparent above it (`8,3`, which reads the background through the
cursor). The "after" column is the same three pixels once the arrow leaves —
so the background was under it the whole time, and the vacated region
repaints correctly, which incidentally exercises exactly the buffer-age path
this entry suspected. Artifacts: `/tmp/cursorproof-before.png`,
`/tmp/cursorproof-after.png` on the dev VM.

A full-frame sweep of the failing capture says the same thing: `(30,3)`,
`(3,30)`, `(5,500)` and every other uncovered point read `srgba(18,52,86,1)`.
Only the 16x16 box at the origin was ever dark.

**3. After the fix**, same VM, same binary:

```
$ SHOT=/tmp/tty-fixed.png MODE=--tty scripts/smoke-test.sh   # EXIT=0, 12 ok, 0 BUG
ok: the focused window's ring pixel at (403,9) matches #ff00ff
ok: the unfocused window's ring pixel at (1197,9) matches #00ffff
ok: the background pixel at (3,3) matches #123456

$ SHOT=/tmp/headless-fixed.png MODE=--headless scripts/smoke-test.sh  # EXIT=0, 12 ok, 0 BUG
ok: the focused window's ring pixel at (303,9) matches #ff00ff
ok: the unfocused window's ring pixel at (897,9) matches #00ffff
ok: the background pixel at (3,3) matches #123456
```

(Exit status captured directly, not through a pipe — the original repro run
printed `EXIT=0` for a *failing* script because `$?` was `tail`'s.)

`cargo test --workspace`: 509 passed, 1 ignored, plus 68 + 10 + 3 + 13 in the
other targets, 0 failed. `cargo clippy --workspace --all-targets -- -D
warnings` and `cargo fmt --check --all`: clean. No Rust changed, so these are
freshness checks on the tree the hardware evidence was captured against, not
coverage of the fix.

## Follow-up this deliberately does not do

**The pointer starts at the output's origin, not centred.** That is why the
cursor was in the corner at all. It is defensible (a cursor has to be
somewhere before the first motion event), but on a real `--tty` session it
means the arrow sits wedged in the top-left until the user moves the
mouse. Changing it is a behaviour change
with its own edge cases (which output, what about multi-output, does it count
as motion for idle/activation purposes) and belongs in its own item, not in a
test fix: `docs/backlog/rendering/pointer-starts-at-origin.md`.
