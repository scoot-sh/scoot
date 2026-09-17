---
title: "Cursor frames stop while the `--tty` session is VT-paused — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Cursor frames stop while the `--tty` session is VT-paused — RESOLVED.

## The entry as filed

`docs/backlog/rendering/cursor-frame-callback-when-paused.md` (LOW):

> An animated cursor client keeps getting woken while the `--tty` session
> is VT-paused (LOW, inherited not introduced). `Tty::present` early-
> returns on `!active`, but `render()` and the frame-callback loops run
> regardless — identical to the pre-existing per-window `send_frame` loop;
> item 8 just makes the cursor share it. Not new, not specific to cursors.

## Resolution (2026-09-17)

`State::render` (`compositor/headless.rs`) returns early while the `--tty`
session holds no DRM master, skipping the render *and* the frame-callback
dispatch, via a unit-testable predicate:

```rust
fn tty_blocks_render(tty_active: Option<bool>) -> bool {
    tty_active.is_some_and(|active| !active)
}
```

called as `tty_blocks_render(self.tty.as_ref().map(Tty::is_active))` with a
new `Tty::is_active()` accessor (`compositor/tty/mod.rs`). The skipped
frame clears `needs_render` rather than leaving it set (see the audit).

### Audit (in code, before the fix)

- **Damage accumulation.** Skipping `render_output` entirely preserves
  damage: neither `OutputDamageTracker`'s history (advances only inside
  `render_output`, confirmed against the pinned Smithay source in
  `buffers.rs`'s own docs) nor the dumb-buffer generation (advances only in
  `advance_generation`, which runs post-`render_output`) moves while
  renders are skipped. The old behaviour was the worse shape — render ran,
  `present` dropped the pixels, damage consumed (the same hole as
  [present-skip-eats-frame-damage](../rendering/present-skip-eats-frame-damage.md),
  which this incidentally fixes for the paused case).
- **Reactivate correctness.** `reactivate()` calls `invalidate_scanout()`
  (ages invalidated → next render is a full repaint, `needs_modeset` →
  full commit, not page flip), and the `ActivateSession` arm maps even
  `Reconfigured::Nothing` to `Render`, so a successful reactivate
  unconditionally requests a render. Clearing `needs_render` on the skip is
  safe exactly because of that guarantee (stated in the gate comment).
- **`active` semantics per write site** (the 5b lesson). `Tty::active` has
  three write sites: `init` (`true`), `PauseSession` (`false`), and
  `reactivate` (`drm_active`). Both `false` states mean "no DRM master
  held, `present()` drops every frame" — paused and failed-reactivation
  alike — so gating on `!active` (the same predicate `present()` uses) is
  correct under each. Gated on `active`, deliberately *not*
  `session_paused`: the failed-reactivation state (session live again,
  master not reacquired) drops frames exactly like a paused one, and
  `reconfigure()` already gates on `active` for the same reason.
- **Frame-callback starvation.** Withheld callbacks only pause client
  pacing; no protocol promises a callback by any deadline (same shape as a
  minimized window in most compositors). Grab, focus and IME-composition
  state all live in the seat, untouched by the skipped tail, which contains
  no grab-dismissal path (those run in input/commit handlers, still live
  while paused).
- **Lock interplay.** A lock arriving while paused runs `lock()`
  (`pending=Some`) → `lock_transition()` → `request_render()`, but the
  timer's `render()` hits the gate and returns before the tail — so
  `await_vblank`, the sole site arming `blank_deadline`, never runs, and
  neither does the fallback-timer arming beside it. The wait stays
  bare-`pending` with no deadline until the switch-back, when the
  reactivation render confirms it via vblank or the fallback (both intact
  there — no wedge). Timing delta, stated honestly on this
  security-sensitive path: pre-PR the locker got `locked` via the fallback
  about a second after requesting it, with no blanked frame on scanout;
  post-PR `locked` waits for the switch-back, when one actually reaches
  scanout — arguably more protocol-correct, but a behaviour change, not a
  non-change. No live lock client exists on the dev VM (no swaylock), so
  this half is construction-verified only — stated, not papered over.
- **Headless/nested.** The gate reads `None` there (`tty_blocks_render(None)
  == false`), provably a no-op: no test harness can construct a `Tty`, so
  the entire existing suite pins the unblocked path.

### Composition with present-skip damage

The gate runs *before* `present()`: a paused frame never reaches
`present()`, so it never touches `present_skipped`, and a stale
`present_skipped=true` carried through a pause resolves the pre-existing
way (next vblank re-renders). The paused-render hole above (damage
consumed, scanout stale) is closed for the paused case; the in-flight-flip
case in `present-skip-eats-frame-damage` is untouched and still open.

### Tests

Three unit tests pin the predicate's truth table (`None`/`Some(true)` pass
through, `Some(false)` blocks). The blocking one was confirmed fail-first:
with the predicate neutered to `false`, `cargo test -p flexwm --bin
flexwm tty_blocks` fails on `a_masterless_tty_blocks_a_render` (1 failed,
814 filtered) and passes with the fix. The harness cannot simulate a pause
(no `Tty` without a DRM device — stated in the predicate's own doc
comment), so pause/reactivate is pinned by live VT-switch evidence below.

### Benchmark (dev VM, `--tty` at 1280x720, `foot` connected, 600 IPC
pointer moves per window, 5 reps each)

The driver is the realistic pressure: each motion marks the frame dirty
(cursor redraws under `--tty`), so a paused session renders at the frame
rate and drops every frame in `present()`. Paused-idle control (no driver)
is 0 jiffies/5s on both builds — the waste only materialises under damage,
which is why the driver is load-bearing to the methodology.

| profile | before (paused) | after (paused) | active control |
| --- | --- | --- | --- |
| release, jiffies/window | 10–12 / ~2.95s (3.4–4.1% CPU) | 7–8 / ~2.8s (2.4–2.9% CPU) | 13 / 1.6s (8.1%) |
| debug, jiffies/window | 31–39 / ~3.1s (10–12% CPU) | 21–24 / ~3.0s (6.6–7.4% CPU) | — |
| wakeups (ctxt)/window | ~1400 | ~1370 | ~700 |

Ranges are min–max across the 5 reps; `CLK_TCK=100`. Wakeups are parity by
design: they are the driver's own IPC round trips (~2.3/motion), identical
before/after — the gate removes the render slice, not the request path.
The remainder after the fix is IPC dispatch + pointer delivery, which no
compositor change can remove. What the table understates: the per-render
waste grows with scene cost (debug amplifies it ~3x), while the fix costs
one branch.

Discrimination proof (frozen framebuffer): while paused, screenshot →
move pointer across the screen → screenshot again. Before: the two differ
(the render ran and moved the cursor). After: byte-identical (no render
ran). Reactivation proof: every switch-back logs `session activated`
followed by `drm: modeset (full commit)`, and the post-reactivate
screenshot is byte-identical (22556 bytes) to the pre-pause one — the full
repaint, across 4 complete pause/activate cycles plus the benchmark
sessions.

### Bug bash

- Rapid pause/reactivate cycling (5× `chvt 2`/`chvt 1` at 0.5s pacing): 4
  complete cycles all show paused → activated → modeset → identical
  repaint. The 5th pause outran the pacing (its `chvt 1` landed before the
  switch completed) and its activate never arrived before teardown — a
  harness-pacing artifact; pause-without-resume is a safe steady state
  (IPC alive, screenshots serve the frozen frame). Foreground VT restored
  to tty1 afterwards.
- Log sweep across all live sessions: only known-benign startup lines
  (unprivileged-mode, legacy fbadd, gamma fallback, atomic blob cleanup).
- Seat discipline: `--tty` seat claimed only after `ps` showed it free,
  released after every session with `ps`-verified empty plus seatd's
  `Removed client`; no stray `foot` processes left behind.

### Not verified live (stated plainly)

- Lock-while-paused confirmation on real scanout (no lock client on the
  VM); the mechanism is traced — bare-`pending` with no deadline until the
  reactivation render confirms via vblank or fallback — and `locked` now
  waits for the switch-back instead of arriving via the fallback ~1s after
  the request.
- Popup grab / IME composition held across a pause (no menu/IME client on
  the VM); dismissal paths audited as outside the skipped tail.
- The failed-reactivation render skip (no way to fail `drm.activate` on
  demand); predicate shared with `present()`, recovery path unchanged.
- Per-client wakeup counts (no frame-callback-counting client on the VM);
  client-side savings follow by construction (no `send_frame` while
  paused).

No README change: a paused session burning less CPU is invisible, and a
screenshot taken while paused returns the last pre-pause frame — the last
thing anyone saw — which needs no documented contract.
