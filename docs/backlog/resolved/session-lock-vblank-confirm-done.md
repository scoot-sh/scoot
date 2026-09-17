---
title: "`locked` is sent once a blanked frame has been *rendered*, not once a vblank has confirmed it — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `locked` is sent once a blanked frame has been *rendered*, not once a vblank has confirmed it — DONE

## Resolution

`locked` now waits for the vblank confirming the flip that carries the
blanked frame under `--tty`, instead of going out with the render.
Headless and nested confirm on render exactly as before — there is no
scanout there, so the framebuffer a screenshot reads *is* the frame.

### Which-flip tracking

`Tty::present` issues at most one flip at a time (it skips while one is in
flight, since a second would fail with EBUSY), so "the in-flight flip" is
at most one number and matching a completion is an equality check, not a
search. The pieces:

- `tty/flip_tracker.rs` (new, ~60 lines plus its own unit tests): a
  monotonic `u64` sequence per issued flip and the in-flight number, if
  one is out. One field for both facts — `is_busy()` *is* "a flip is in
  flight", which is what `present` gates on — so the two can never
  disagree the way item 5b's two-meaning field did. `present` returns the
  issued number (`None` on every skip and on commit failure); only an
  issued flip can carry the blanked frame, so a skip records no number.
- `SessionLock::blank_flip`: the tracked number, set only when a blanked
  frame's `present` issued a flip. A vblank for the *previous* frame (the
  lock raced a flip already in flight, so `present` skipped and nothing
  was recorded) matches nothing and confirms nothing; the re-render the
  skip armed presents the blanked frame as the next flip and *that*
  vblank confirms. A stale vblank for a flip the scanout bookkeeping has
  since discarded (VT switch back, hotplug modeset — both discard the
  in-flight number without a completion) settles to `None` and likewise
  matches nothing. `DrmEvent::Error` settles the buffers but deliberately
  drops the number: an untrackable completion must not confirm a lock.
- Invariant, stated on the field: `blank_flip` is `Some` only while
  `pending` is `Some`. Both are installed and cleared together at every
  site that writes `pending` — fresh `lock`, the dead-`pending` sweep,
  `unlock`, and the two confirm paths — so a wait can never outlive its
  lock and confirm a later one. In particular the sweep cancels the dead
  lock's wait, which a same-connection destroy-then-relock would otherwise
  leave matchable.

### No hang (the load-bearing property)

A vblank that can never arrive — switched away, a discarded flip, a
driver that never delivers — must not hang the locker forever, which is
worse than the one-vblank gap this closes (a locker waiting for `locked`
may be waiting to prompt for the password at all). So every unconfirmed
`--tty` frame also arms `blank_deadline` (`LOCK_VBLANK_TIMEOUT`, pinned
at one second: ~60 vblanks of grace at 60Hz, far short of any locker's
give-up timescale), watched by a one-shot calloop timer that confirms
anyway with a `warn!` log. Skipped frames (no flip issued) arm the
deadline without recording a number; a failed timer insert logs `error!`
(the vblank path still works; only the fallback is gone). The timeout
takes the wait exactly once; a vblank that already confirmed leaves the
fired timer a no-op.

### Cost

Per issued flip: one `wrapping_add` and two `Option` stores. Per render:
one `awaiting_blank()` plus one `tty.is_some()` at the tail (the old
code already asked `pending` there). No allocation on any path; the timer
exists only while a lock awaits confirmation. Measured render throughput
(throwaway harness loop, deleted after): 500 headless renders 42–62ms
after vs 48–51ms before across three reps each — overlapping ranges on a
noisy VM, i.e. no measurable change, as expected for a counter and two
branches.

## Evidence

- Fail-first, dev VM: the eight `vblank_confirm` harness tests plus five
  `flip_tracker` unit tests fail pre-fix (the API does not exist), pass
  post-fix. Neuter check (temporarily `confirm_on_vblank → false`,
  `poll_blank_timeout → false`): the five confirm/timeout-dependent tests
  fail, the three no-confirm guards still pass — the expected split.
- Vblank feasibility, dev VM QEMU/virtio-gpu: 1 modeset + 2 page flips
  for two pointer-move renders — the second flip proves the first's
  completion event arrived and cleared the in-flight state. Completion
  events are usable there; no "needs real hardware" caveat on the event
  path itself.
- Live lock on dev-VM `--tty` (temporary `examples/tty_lock_probe.rs`,
  deleted after — a minimal lock client: lock, map a solid surface, wait
  for `locked`, unlock): three consecutive locks each log `session lock
  confirmed: the blanked frame reached scanout` ~20ms after the request,
  the fallback `without its vblank` never fires, and a locked IPC
  screenshot plus `locked: true` on replies confirm the session state.
- Full set green post-fix: `cargo test -p flexwm` (807 passed: 794 + 13
  new), `cargo nextest run --workspace` (912 passed), `cargo clippy -p
  flexwm --all-targets -- -D warnings` clean, `cargo fmt --check -p
  flexwm` clean, `scripts/smoke-test.sh` exit 0, 17 ok.
- README's lock section rewritten: vblank confirmation under `--tty` (up
  to one vblank of added lock latency under contention), the one-second
  fallback, headless/nested unchanged.

## What this is not, found live while verifying it

Locking live and mapping a surface afterwards blanks the screen but the
surface's own pixels never appear: locked screenshots stay
backdrop-black (plus two stray magenta pixels at the left edge —
mechanism not isolated) on the pre-change binary identically, so this is
pre-existing, not a regression. The harness stays green because with
buffer age 0 every render redraws everything; under `--tty`'s real ages
a surface committed after the confirming frame appears to get no damage
redraw — not even cursor sweeps across the whole screen reveal it. Filed
as its own item (`docs/backlog/protocols/session-lock-surface-not-drawn-live.md`):
a lock screen that never shows the locker is a daily-driving blocker
standing next to the guarantee this item just closed.
