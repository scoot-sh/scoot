---
title: "A `present()` skipped for an in-flight flip consumes that frame's damage — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A `present()` skipped for an in-flight flip consumes that frame's damage — DONE

## The entry as filed

> Found by code reading during the isolate-first probe that closed
> [`session-lock-surface-not-drawn-live`](./session-lock-surface-not-drawn-live-done.md)
> as unreproduced — observed never, live or in-harness. Filed so the trace
> doesn't evaporate.
>
> A `present()` skipped for an in-flight flip consumes that frame's damage
> in `render_output`, and the `present_skipped` retry re-renders with damage
> `None` (`headless.rs`, `tty/mod.rs`), so scanout keeps stale pixels until
> unrelated damage arrives. [...] The candidate fix (`invalidate_ages` on
> skip) would touch the hot present path [...] — correctly out of scope
> until observed.

## Verify-first: the filed shape self-heals; the adjacent one loses

Re-derived against the pinned Smithay source
(`src/backend/renderer/damage/mod.rs`, rev `0ff0098`) before touching
anything. `damage_output_internal` extends an unchanged frame's empty new
damage with `old_damage.take(age - 1)`, and only reports `None` when that
is empty too:

- **In-flight skip (the filed shape): no write happens**, so the slot's
  `last_written` is untouched and the retry's age is one *larger* than the
  skipped render's. At age ≥ 2 the history still holds the skipped frame's
  damage (`take(age-1)` includes it); at age 0 the tracker takes the
  full-redraw branch. Either way the retry re-presents. The `VBlank` for
  the still-out flip triggers it via `present_skipped`. This is why the
  ticket's shape was "observed never": the mechanism as filed does not
  lose. Pinned, not fixed: the retry must keep working this way, so no
  `invalidate_ages` on the in-flight path — forcing a full redraw + full
  dumb-buffer copy on every cursor-motion skip would throw away exactly
  the memcpy the per-slot ages were built to avoid.
- **Refused commit/page-flip (the adjacent shape, same symptom): the
  pixels were already copied into the slot** (`write_region` runs before
  the `commit`/`page_flip` call), and the error arm freed the slot while
  keeping its fresh `last_written`. The retry therefore reads as **age
  1**: empty new damage plus `take(0)` is empty, the tracker reports
  `None`, `render()` calls no presenter, and scanout stays stale. Worse,
  nothing is in flight, so no `VBlank` will ever arrive to consume
  `present_skipped` — at idle no retry render is even triggered. The
  reachable trigger is the one `hotplug.rs`'s `invalidate_scanout` doc
  already names: a discarded in-flight flip still out in the kernel makes
  the re-presented flip fail with EBUSY. So the ticket's "until unrelated
  damage arrives" was right about the symptom and wrong about which skip
  produces it.

## The fix

Only the commit-failure arm changes; the in-flight, paused and
size-mismatch paths are byte-for-byte what they were:

- `BufferPool::note_write_failed` (`tty/buffers.rs`): frees the failed
  slot *and* clears its age (per-slot, not whole-pool — the innocent
  slot keeps its history). The age counters moved into a `SlotAges`
  struct so this transition is unit-testable without a live DRM device,
  the same rationale `first_free`/`age`/`copy_region_rows` were extracted
  for. Zero-cost on the hot path: the success path is one extra method
  call resolving to the same stores.
- `Tty::present`'s error arm calls it (instead of `mark_free` + setting
  `present_skipped`, which stranded: no `VBlank` is owed), and arms a
  **timer-driven** retry via `retry_armed`, consumed once by the render
  tail *after* `needs_render` is cleared so the request survives it. The
  retries are bounded (`tty/present_retry.rs`: 3 consecutive refusals,
  then one `warn!` and quiet until genuine damage — a wedged device must
  not pin the loop at 60Hz full redraws); any issued flip resets the
  streak. No allocation anywhere on these paths.
- `present_skipped` keeps exactly its in-flight meaning (a `VBlank` is
  owed); the error arm no longer touches it.

Edge cases, all traced, none needing new code:

- **Skip-then-lock**: a refused blank frame keeps the lock waiting with
  its deadline armed (existing `await_vblank(None, …)` behavior); the
  timer retry now re-presents the blank ~16ms later and the vblank
  confirms promptly instead of only the 1s fallback confirming with a
  stale scanout. Strictly better than before, same guarantee shape as
  PR #84.
- **Skip-then-screencopy**: parked captures read the pixman framebuffer,
  which holds the failed frame's pixels either way, and `frame_serial`
  still advanced on its damage — nothing strands (PR #93 untouched).
- **Skip-then-resize**: `resize_output` replaces the backend (fresh
  tracker and pool) and re-renders; a pending timer retry just becomes
  one more full-damage render at the new size.
- **Give-up streak then recovery**: the first damage-driven render after
  the cap issues (or refuses and stays quiet); a success resets the
  streak. No latch-up.

## Evidence

- Fail-first, dev VM: `SlotAges::write_failed` + `PresentRetries` are new
  APIs (fail pre-fix by absence, per the PR #84 precedent); neuter checks
  recorded — `write_failed` as a no-op fails
  `a_write_that_never_reached_scanout_reads_as_age_zero` (retry would read
  age 1), `MAX_CONSECUTIVE_RETRIES = 0` fails all three retry tests.
- Contract pin, dev VM: `an_unchanged_frame_reports_no_damage_at_age_one_
  but_full_damage_at_age_zero` drives the real `OutputDamageTracker` +
  `PixmanRenderer` (unchanged solid element: age 1 → `None`, age 0 →
  full). Passes pre- and post-fix by design — it pins Smithay's contract
  the fix relies on, not flexwm behavior.
- Full set green post-fix, dev VM, branch `implementer/present-skip-damage`:
  `cargo test -p flexwm` (902 passed, 1 ignored), `cargo nextest run
  --workspace` (1007 passed, 1 skipped), `cargo clippy -p flexwm
  --all-targets -- -D warnings` clean, `cargo fmt --check -p flexwm`
  clean, `scripts/smoke-test.sh` (SMOKE_PREFIX=/tmp/smoke-skipdamage)
  exit 0, 17 ok.
- Benchmark, dev VM release, 500 headless renders 800×800: pre-fix
  28.1 / 32.8 / 35.2ms, post-fix 31.2 / 30.8 / 37.0ms — overlapping
  ranges, no measurable change, as expected (headless executes one extra
  never-taken branch). The `--tty` present path itself cannot be
  benchmarked without DRM hardware: stated, not papered over.
- Live `--tty` sanity, dev VM QEMU/virtio-gpu (no-regression only, not a
  repro — a deterministic EBUSY needs winning a ~16ms race):
  modeset on Virtual-1 1280×720, IPC `windows`, `msg screenshot`
  (1280×720 PNG), `spawn foot` + pointer move + `wait-idle` +
  screenshot with content, `action quit` exits clean, zero
  `flip failed` / `keeps failing` / render errors in the log. Shutdown
  logs a pre-existing Smithay device-drop `restore previous state:
  Permission denied` also present independent of this diff (drop path
  untouched).
- No README change: internal bug fix, no new config, keybinding, CLI
  flag, or IPC surface. Stated explicitly rather than skipped silently.

## What this is not

The in-flight skip path is deliberately *not* "fixed": verification
showed it recovers via damage history today, and invalidating there
would cost a full redraw on the retry of every throttled frame. If a
stale-scanout frame is ever captured live after a commit landed inside a
flip window, re-open against that evidence — the discriminating test is
still scanout differing from a same-instant `msg screenshot`.
