---
title: "flexwm blanks the screen immediately on a lock request rather than waiting for the lock client's first surface — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# flexwm blanks the screen immediately on a lock request rather than waiting for the lock client's first surface — DONE

## The entry as filed

> flexwm blanks the screen immediately on a lock request rather than
> waiting for the lock client's first surface (item 18, deliberate). niri
> waits up to a second for lock surfaces so the transition doesn't flash
> black; the cost of that is rendering the *unlocked* session for that whole
> second, which is the wrong half of the trade to take first. Worth
> revisiting as a comfort feature if the black flash proves annoying in
> daily use — with a hard deadline, as the protocol requires.

## Verify-first findings (2026-09-18, no behavior change)

Drove the lock sequence in-harness before writing anything, taking the
ticket's timing question — when the screen blanks vs when the session
reports locked vs when input is captured — as the thing to verify rather
than assuming the deliberate choice was still correctly implemented. The
ordering, as observed on `main`, is:

1. **Input is captured the moment the request is accepted.**
   `SessionLockHandler::lock` sets `owner` and runs `lock_transition()`
   synchronously: both focuses re-derived (keyboard to the first current
   lock surface, or nobody when none exists yet), grabs dropped, a held
   pointer constraint deactivated, render requested. No frame involved.
2. **Pixels blank on the first frame after that.** `headless.rs::render`
   builds the element list from `is_locked()` alone, so the first frame
   after the accept is backdrop-black (plus any mapped lock surfaces).
3. **`locked` goes out only once the blanked frame exists.**
   Headless/nested confirm on render (`confirm_lock`); `--tty` additionally
   waits for the carrying flip's vblank with a one-second fallback (PR
   #84). Captures parked across the window stay parked (`awaiting_blank`
   in `service_captures`, PR #93), and an IPC screenshot renders first and
   reads back after (`capture_pixels`), so neither can serve desktop
   pixels from the window.

So there *is* a frame in which the desktop is still in the framebuffer
while input is already locked — between the accept and the first frame —
and that direction is the deliberate, secure one. The reverse (blank
pixels while keystrokes still reach the window underneath) would route
the password into the unlocked session, and the niri shape (keep drawing
the desktop until the locker is ready, up to a second) keeps the
sensitive pixels on screen for the whole wait while input is already
dead. Neither alternative survives the comparison; the ticket's own
framing ("the wrong half of the trade to take first") still holds, and
no daily-use complaint about the black flash has been filed since, so
the revisit condition has not fired.

Interactions re-checked, none disturbed (this change adds no production
code, so this is audit, not re-verification):

- **vblank-confirm (PR #84):** the headless confirm-on-render path this
  ticket's window lives on is unchanged; the new test asserts
  `pending.is_none()` after the headless confirming render.
- **First-click (PR #76):** pointer focus is re-derived at accept (this
  window) *and* on the commit that maps the lock surface; the two are
  complementary, not competing.
- **Screencopy-parked (PR #93):** untouched; the parked-across-blank
  suite still pins the re-arm on both confirm paths.
- **Pointer-lock deactivation (PR #92):** the deactivation runs inside
  `lock_transition()`, i.e. at accept — the same synchronous point this
  record pins — so a held lock cannot freeze the focus refresh this
  ordering depends on.

## Resolution (decide + pin, 2026-09-18)

Immediate blank stands; the niri-shape wait is declined, not deferred —
revisit only if the black flash proves annoying in daily use, which is
the ticket's own condition, with the protocol-mandated hard deadline it
already names. What lands is the pin the ordering was missing: no single
test asserted the three events' order together (input at accept, pixels
at first frame, `locked` after), only each leg separately.

- New test
  `session_lock::tests::blanking::lock_captures_input_before_the_first_blanked_frame_confirms_it`:
  maps a window, parks the pointer over it, renders the desktop, then
  takes the render target away so no frame tick can blank or confirm
  early. After `LockNoWait` it asserts the session is locked and pending,
  both focuses already `None`, and `locked`/`finished` still 0; restores
  the target and asserts the untouched framebuffer still holds desktop
  pixels; then renders and asserts a whole-screen black frame, nothing
  pending, and `locked` 1.
- Fail-first: with `lock_transition()` neutered out of
  `SessionLockHandler::lock`, the new test fails at the keyboard
  assertion (`Some(Window(0))` vs `None`); restored, it passes. The
  pixels and `locked` legs overlap the existing blanking/lifecycle pins
  and are asserted here for the combined order, not as new behavior.
- No production code changed, so no hot path is touched and no
  before/after benchmark applies (one test, run on dispatch, not on any
  per-event path).

## Evidence

- New test post-fix, dev VM: `cargo test -p flexwm --bin flexwm
  session_lock::tests::blanking` — 7 passed (6 existing + 1 new).
- Neuter run (same filter, `lock_transition()` call removed): 1 failed
  at the keyboard-at-accept assertion, as above; no other test run in
  that state.
- Full set post-fix, dev VM, this commit (re-run after the amend; the
  amend touched only this record's prose, no code):
  `cargo test -p flexwm` (881 + 3 passed, 0 failed),
  `cargo nextest run --workspace` (986 passed, 1 skipped),
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh` exit 0
  (17 ok).
- README's Screen-locking section already stated the immediate blank and
  the one-frame window; the one-frame bullet now also says input is
  already captured during it, so the ordering reads in one place.
