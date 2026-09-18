---
title: "One physical output can hold unboundedly many lock surfaces if the lock client binds `wl_output` more than once — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# One physical output can hold unboundedly many lock surfaces if the lock client binds `wl_output` more than once — DONE

## The entry as filed

> One physical output can hold unboundedly many lock surfaces if the lock
> client binds `wl_output` more than once (item 18, found by round three's
> review through source reading — not exercised, and not a privilege
> escalation). The protocol's one-surface-per-output rule is enforced by
> Smithay, whose `SessionLockState` keeps a `Vec<WlOutput>` of locked outputs
> and compares `WlOutput` resource identity (`locked_outputs.contains(&output)`
> in `session_lock/lock.rs`). A client may bind the same `wl_output` global
> any number of times, and each bind is a different resource, so `lock`,
> `get_lock_surface(bind_1)`, `get_lock_surface(bind_2)`, … all succeed for
> one physical output. flexwm's `new_surface` accepts each (they pass
> `is_current`: same lock, live surfaces) and `SessionLock::current`
> composites every mapped one, so the render list grows with the number of
> binds, and the first in creation order keeps the keyboard. Only reachable
> by whoever already holds the lock — they already own the whole screen, so
> there is nothing to escalate to, and the cost is their own memory plus the
> compositor's per-frame work over a list they control. Tearing that list
> down is quadratic (every surface destruction re-derives focus over what is
> left), which predates round three's hook and is not made worse by it: the
> `retain` in `State::forget_lock_surface` was already O(list) per destroyed
> `wl_surface`. The fix belongs in
> the same place multi-output does: resolve each `wl_output` to its
> `Output` (`Output::from_resource`) and treat *that* as the key, in
> `new_surface`, rather than trusting Smithay's resource-identity guard.

## Verify-first findings (2026-09-18 — refuse, no Smithay patch needed)

Drove nothing before deciding; the decision needed sources, not a harness.
Every claim below was re-derived against the pinned rev, not quoted from the
ticket:

**Where admission lives.** In Smithay, pre-delegation:
`0ff0098/src/wayland/session_lock/lock.rs:56-67` — `give_role`, then
`locked_outputs.contains(&output)` (resource identity: `PartialEq` on
`WlOutput` compares protocol object ids, so a second bind is a different
resource and is admitted), then `push`, then `data_init.init`, and only then
`state.new_surface(...)` followed by the initial `send_configure`. So
admission is Smithay's incomplete guard plus flexwm's accept-everything
`new_surface` — and flexwm *can* refuse without a Smithay patch:
`LockSurface::ext_session_lock()` hands back the lock object, on which
`post_error(Error::DuplicateOutput, ...)` is the same synchronous-kill shape
Smithay itself uses for the same-resource case and this codebase already
uses for every other wire refusal (`dispatch.rs`'s module doc states the
safety argument: `Client::kill` runs logging-only `disconnected` while
holding the backend mutex). The one asymmetry: Smithay's refusal fires
before the object is initialized, while flexwm's fires after — so Smithay
still sends the initial configure to a client it just killed, which is a
harmless write to a dead socket (proven: the kill lands, the session stays
locked, the compositor keeps serving).

**What the protocol says.** `ext-session-lock-v1.xml` (the
`wayland-protocols` 0.32.13 copy in the local registry):
"Attempting to create more than one lock surface for a given output is a
`duplicate_output` protocol error." *Output*, not resource — Smithay's
resource-identity check is an approximation, and the different-resource hole
is a protocol-letter violation, not a leniency. Admit-and-pin would pin a
violation; refuse is what the protocol mandates.

**The key exists.** `Output::from_resource` (`0ff0098/src/wayland/output/
mod.rs:214-217`) resolves any bind through per-resource user data to the
physical `Output`, and `Output: PartialEq` is `Arc::ptr_eq`
(`0ff0098/src/output.rs:437-441`) — physical identity, exactly the
granularity the error names. `LockSurface` itself exposes no output, so the
admitted output is recorded per surface in a sidecar map.

**Neither shell trips it.** Noctalia's real teardown is one
`get_lock_surface` per lock cycle (`noctalia-reprobe-done.md`: `lock()` →
`get_lock_surface(#90)` → single configure/ack/`locked`), and DMS's is the
same shape per cycle (`dms-reprobe-done.md`: one surface per
lock → auth → unlock cycle, `#57` then `#41`). Both destroy role +
null-commit on teardown and never hold two surfaces at once — the refusal
only fires on two *live* surfaces for one output.

## Resolution (refuse per physical output, 2026-09-18)

- `SessionLock::surface_outputs: HashMap<ObjectId, Output>` records what
  each admitted surface was admitted for, keyed by its `wl_surface`'s id
  like `acked`. Writers mirror `surfaces` minus the `retain` passes:
  `insert` in `new_surface`, `clear` in `lock`/`unlock` alongside
  `surfaces.clear()`, `remove` in `forget_lock_surface`.
- `SessionLock::surface_for_output(&Output) -> Option<&LockSurface>` asks
  through `current()`, so only live surfaces count. The signature is the
  guarantee: it answers with a live `&LockSurface`, so a sticky refusal (an
  entry with no live surface behind it) is unrepresentable — the
  forget-removal is bounded-hygiene, not load-bearing.
- `new_surface` refuses a second live surface for an already-covered
  output with `LockError::DuplicateOutput` ("Output already has a lock
  surface.") on the owning lock, after the existing `is_current` gate (a
  *non-owner's* surface is still ignored, not killed) and after output
  resolution (so the no-output fallback path is unchanged).
- Limits, pinned: at most one live lock surface per physical output per
  lock; with one output that is one live surface per lock. Same-client-only
  is unchanged and already held by `is_current` (only the owning lock's
  surfaces are ever admitted). Destroying a surface — role *and*
  `wl_surface`, so it is forgotten — frees its output for a rebuild.
  Residual, Smithay-side and out of scope: Smithay's `locked_outputs` never
  shrinks (no removal on surface destroy, unlock, or disconnect — re-verified
  by grep: five references total, none a removal), so a
  same-resource destroy-then-rebuild still dies in *its* guard before flexwm
  is asked. A locker rebuilding its UI must use a fresh bind the way the
  harness rebuild test does.
- Supersedes PR #103's two-surface suite: "first-created on top, keyboard
  on the first, resize reaching all" described admitted concurrent surfaces,
  which can no longer exist. The single-surface per-output pins stand
  (configured to the output's size, first-frame confirm, resize reaches
  the one surface — covered by `lifecycle`/`blanking`). A note is appended
  to `session-lock-per-output-done.md` saying so.

## Evidence

- Fail-first, dev VM: `a_second_live_surface_for_the_same_output_is_refused`
  fails pre-fix (`the client survived a request that should have been
  refused` — the second surface was admitted), passes post-fix with the
  exact refusal pinned: the client's own backend reports
  `Protocol error 3 on object ext_session_lock_v1@N: Output already has a
  lock surface` (object number varies, not pinned), the session stays
  locked, and a further `render()` proves the compositor keeps serving.
- `destroying_the_surface_frees_the_output_for_a_rebuild` passes pre- and
  post-fix (admission was permissive; the pin is against a sticky future).
  The live-only half is structural, not lifecycle-dependent: two separate
  neuter runs (skipping the forget-removal; consulting the map without the
  `current()` filter) both stay green, which is the expected outcome once
  stated plainly — refusal requires returning a live surface, so neither
  neuter can express stickiness.
- No hot path touched: `surface_for_output` runs only in `new_surface`
  (per lock-surface creation, off-frame) over at most the live surfaces
  (one, at one output), plus one `HashMap` insert per admission — so no
  before/after benchmark applies, stated not skipped.
- Full set post-fix, dev VM: `cargo test -p flexwm` (883 passed, 0 failed),
  `cargo nextest run --workspace` (988 passed, 1 skipped),
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh` (17 ok).
- Not regressed, all green untouched in the same runs: teardown (incl. both
  by-design kill pins), vblank-confirm (#84), blank-timing, and the
  per-output single-surface pins.
