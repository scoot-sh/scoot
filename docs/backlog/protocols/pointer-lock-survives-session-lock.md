---
title: "A held pointer lock survives a session lock, so a client keeps pointer input while the screen is locked"
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# A held pointer lock survives a session lock, so a client keeps pointer input while the screen is locked

Found by a retrospective audit of PRs #73–#89 on 2026-09-17, traced against
`673c9ea` (the merge of PR #89, which introduced it).

`State::refresh_pointer_focus` (`input.rs:396`) exists for exactly one
reason, stated in its own doc: `wl_pointer.button` and `wl_pointer.axis` go
to whatever surface the pointer last *entered*, never to whatever is under
it now, so "without this, the first click after a lock would still land in
the window underneath". `lock_transition` (`session_lock.rs`) calls it at
every lock transition for that reason.

It cannot do that job against an active pointer lock. It re-derives focus by
moving the pointer to where it already is, and a zero-delta move with a held
constraint resolves to `AbsoluteTarget::Held` (`input.rs:223`), whose entire
body is `pointer.frame(self)` — no motion, no `leave`, no `enter`, so seat
pointer focus never moves off the locking surface. The relative stream is
then delivered on focus alone: "Focus-gated, not lock-gated ... whoever
holds pointer focus gets the deltas, locked or not" (`input.rs:192`).

So with a game holding a pointer lock (the use case README advertises) and
the user locking the session:

- device deltas keep streaming to the game at device rate, across the lock;
- `button` and `axis` keep going to it, since focus never left;
- the lock surface never receives an `enter`, so pointer-driven lock UI gets
  nothing.

Pre-#89 this was unreachable: nothing could freeze pointer focus, so the
lock-time refresh always landed on the lock surface. It is a new regression,
and it is the same class of exposure `session_lock.rs` already rejects in as
many words for the keyboard ("every keystroke of the user's password would
go to whatever had a menu open").

The escape hatch the pointer-constraints module claims does not apply here.
`relative_pointer.rs` justifies a persistent lock's freeze with "closing the
offending window frees the pointer with one chord, no VT switch needed" —
but `input.rs:747` forwards every bound keybinding except `Bound::ChangeVt`
to the lock client while the session is locked, by design. While locked, the
chord that would close the window is delivered to the locker instead of
executed, so a VT switch is the only recovery.

**Review saw this and pinned it rather than fixing it.** PR #89's
review-driven commit added
`a_persistent_lock_survives_a_session_lock_round_trip`
(`relative_pointer/tests.rs:1505`), whose own comment describes the
behaviour as intended: "the session lock's focus refresh delivers no `leave`
to the game surface ... the same lock is still active, still holding, still
streaming." Any fix therefore has to change that test, not just the code —
and the decision it records should be re-taken deliberately, because the
module doc's stated basis for accepting the freeze (keybinding recovery)
is false in the locked case.

Fix shape, not a plan: deactivate (or bypass) a held constraint for the
duration of a lock, so the lock transition's focus refresh behaves like the
unconstrained case — Smithay's `PointerConstraint::deactivate` exists and
already runs on other paths; the client then sees `unlocked` and re-arms on
unlock, which is the protocol's own story for losing a lock. The alternative
— special-casing the `Held` arm to still deliver `leave`/`enter` while
locked — keeps the constraint active and needs care that no relative event
is delivered between the two.

Adjacent, reported by the same audit but **not independently verified**:
`relative_pointer.rs:246` clones the constraint region
(`constraint.region().cloned()`, a `Vec` inside `RegionAttributes`) on the
focused constrained-motion path, which would be a per-event heap allocation
on an input path — against the standing bar in `CLAUDE.md` — and is
discarded unused on the locked branch. The measured before/after numbers in
PR #89's body cover the *unconstrained* focused path, so the constrained
path has no number either way. Worth confirming before acting on.
