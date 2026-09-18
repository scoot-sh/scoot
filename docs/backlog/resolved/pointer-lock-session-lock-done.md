---
title: "A held pointer lock survives a session lock, so a client keeps pointer input while the screen is locked — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A held pointer lock deactivates across a session lock — DONE

Resolves
`docs/backlog/protocols/pointer-lock-survives-session-lock.md`
(filed 2026-09-17 by a retrospective audit of PRs #73–#89; regression from
PR #89). That file is deleted by this change; this record replaces it.

## What was wrong

With a game holding an active pointer lock, locking the session left seat
pointer focus frozen on the game: `lock_transition`'s focus refresh is a
zero-delta move, which a held constraint resolves to `AbsoluteTarget::Held`
(delivers nothing), so device deltas, buttons and axis kept streaming to
the game across the lock while the lock surface never got `enter`. The
recovery chord the module doc cited (close the window) never runs while
locked -- every keybinding but `ChangeVt` is forwarded to the locker -- so
a VT switch was the only way out. Password-leak class.

## The fix, as filed

Deactivate (not bypass) the held constraint for the duration of the lock,
so the transition's focus refresh behaves like the unconstrained case.
`State::lock_transition` (`session_lock.rs`) now calls
`State::deactivate_pointer_constraint` (`relative_pointer.rs`) before the
focus refreshes. The client sees `unlocked`/`unconfined`; the persistent
entry stays registered; unlock re-derives focus onto the game surface and
the ordinary arrival path (`engage_pending_constraint`) re-arms it with no
new request -- the protocol's own leave/re-enter story, the same one the
regional-confinement test already pinned. The `Held`-arm alternative from
the ticket was rejected without building it: deactivation worked first
time, keeps the constraint state honest (no "active but focus elsewhere"
hybrid), and needs no care about relative events between a synthetic
leave/enter pair.

Only the focus surface's constraint is deactivated, and that is exhaustive
rather than approximate: activation is focus-gated at creation and on
arrival, Smithay deactivates on pointer-leave, and the seat holds a single
focus -- so an active constraint implies its surface holds focus. No
Smithay enumeration API exists (upstream `TODO` at the pinned rev), and
none is needed. Inactive constraints elsewhere are untouched (deactivating
one is a no-op that sends nothing), which is correct: they stream nothing,
and a lock requested while locked must stay inactive until unlock.

## The behavior correction, stated plainly

PR #89's review pinned the buggy behavior as intended in
`a_persistent_lock_survives_a_session_lock_round_trip`, on the false basis
above. That test is rewritten, not removed, as
`a_session_lock_deactivates_a_held_pointer_lock`: lock deactivates (game
sees `unlocked`), the lock surface gets `enter` at map-commit time,
motion/click/scroll reach only the locker while locked, unlock re-arms
(game sees `locked` with no new request) and the stream resumes with
absolute still held. The old assertion was wrong; this record is where
that is said.

## Edge cases, decided and pinned

- **Lock with no active constraint**: unchanged behavior (ordinary
  lock path, focus to the lock surface and back), pinned by
  `locking_with_no_active_constraint_changes_nothing`.
- **Unlock with the game client gone**: no event is owed afterwards; the
  disconnect mid-lock must neither panic nor wedge the session (a fresh
  lock afterwards confirms), pinned by
  `unlocking_with_the_locked_game_client_gone_does_not_panic`.
- **Constraint requested after the session locked**: lock wins. Activation
  is focus-gated and nothing but a lock surface can hold focus while
  locked, so the request sits inactive -- no `locked`, no stream -- and
  unlock engages it, pinned by
  `a_lock_requested_while_locked_stays_inactive_until_unlock`.
- **Multiple constrained clients**: only the focused lock is active, so
  only it is deactivated; the unfocused client's armed-but-inactive lock
  sees neither `unlocked` nor `locked`, and locked input reaches neither
  game client, pinned by
  `a_session_lock_deactivates_only_the_focused_constraint`.
- **Confinement**: the ticket's question answered NO. A held confine does
  *not* freeze focus at lock -- the lock-time refresh fail-opens (the
  origin re-derivation runs under the locked hit test, which finds
  nothing, so `absolute_target` returns `Free`), the refresh delivers a
  `leave`, and Smithay's own leave path deactivates it. The game already
  saw `unconfined` pre-fix. `a_session_lock_deactivates_a_held_confinement`
  pins that correct behavior (it passes with and without the fix) so the
  lock change cannot regress it. The uniform deactivation in
  `lock_transition` covers confine identically; the wire order
  (`unconfined` before `leave` rather than Smithay's `leave` then
  `unconfined`) is unspecified by the protocol either way.

## The adjacent allocation note, answered

Confirmed and fixed in the same change, as the ticket allowed: the pre-fix
`absolute_target` ran `constraint.region().cloned()` on every focused move
with an active constraint -- a `Vec` clone per event on the input path
(free only for regionless constraints, where the `Option` clone is empty).
The confine arm only ever asked `contains`, so the fix is a cheap
restructure, not a redesign: two short constraint-map borrows returning
only `Copy` data (phase one: locked-or-confined; phase two, against the
hit-tested origin: gate plus per-axis clamp as a delta), with the owned
`AbsoluteTarget` built outside afterwards.

One load-bearing subtlety found by bug-bash, not review: the first version
of the restructure ran the whole confine arm *inside* the map lookup and
hung every confinement test. `with_states` holds the surface's user-data
mutex (non-reentrant `MutexGuard`) for its closure, and the arm's hit
tests re-enter it -- a textbook deadlock Smithay's own commit-hook comment
warns about. The two-phase shape exists for that reason, and says so.

Measured on the dev VM (direct `absolute_target` calls, 200k x 5 reps over
an active regional confinement, temporary bench since removed -- direct
calls rather than `pointer_move` so socket buffers cannot wedge the
measurement). Debug: 8822-9093 ns/event before vs 7193-9576 after --
ranges overlapping, no latency claim made or needed: at this profile two
hit tests dominate and the clone was noise inside them. What changed is
structural: no heap allocation remains on the path (verifiable by
inspection -- both closures return `bool`/`Point` only), which is what the
standing bar asks for. Temporary bench removed before landing; ranges
above are the record.

## Bugs found in the bug-bash (both in the new tests, neither in the fix)

- The locker script acked `MapLockSurface` and `ReleaseSessionLock`
  without a trailing roundtrip, so wayland-client buffered the commit /
  `unlock_and_destroy` past the ack -- the map `enter` landed late and the
  unlock never left the client at all (found as "unlock doesn't move
  focus", root-caused on the wire with `WAYLAND_DEBUG=1`). Both arms now
  roundtrip before acking, mirroring the session-lock harness's
  bottom-of-loop flush.
- The locker's `locked_seen`/`finished_seen` flags were sticky across
  re-locks on one connection, so a second `TakeSessionLock` acked a lock
  that was never confirmed -- whose unlock the compositor then rightly
  refused with `InvalidUnlock`, killing the client (found as a flaky
  "Session is not locked" protocol error, ~2/3 of runs). Flags reset per
  `TakeSessionLock`, the session-lock harness's count-delta shape in
  miniature. 10/10 green after.

## Evidence

- Fail-first, dev VM (`ssh -p 2222 dev@localhost`, pre-fix tree
  `f3f7db9` plus the new tests uncommitted): the three fix-verifying
  tests fail -- `a_session_lock_deactivates_a_held_pointer_lock` ("the
  session lock left a held pointer lock active"),
  `a_session_lock_deactivates_only_the_focused_constraint`, and
  `unlocking_with_the_locked_game_client_gone_does_not_panic` -- while
  the three pins (confinement, no-constraint, requested-while-locked)
  pass, proving the harness observes the fixed behavior.
- Post-fix, same VM: `cargo test -p flexwm` 854 + 3 pass,
  `cargo nextest run --workspace` 959 pass 1 skipped,
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean (Mac-side),
  `scripts/smoke-test.sh` (`SMOKE_PREFIX=/tmp/smoke-lockfix`, `--headless`)
  exit 0 with zero `BUG` lines.
- No live `--tty` re-measurement: the changed paths run identically on
  every backend (one motion core, one transition function), and the
  evidence is harness wire assertions plus the constrained-path bench
  above. Stated as an environment call, not a gap.

## Docs

`README.md` in the same change: the Relative-pointer section documents
the lock-deactivates behavior for game clients, and the IPC agent note
corrects the PR #89 follow-up rule that assumed locks persist (an agent
must not assume an observed lock survives a session lock). No config,
keybinding, CLI or IPC wire change.

Original entry, left as written (file deleted by this change):

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
