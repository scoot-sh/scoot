---
title: "relative-pointer-unstable-v1 + pointer-constraints advertisement: raw deltas for constrained-pointer clients — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `relative-pointer-unstable-v1` (+ `pointer_constraints`) — DONE

~~Raw, unaccelerated pointer deltas; pairs with the `pointer_constraints`
support already present, and games/3D apps expect both together.~~ — DONE,
one of the bundle children of
`docs/backlog/protocols/protocol-gaps-general.md` (which now marks it done
the same way). The one remainder there (`presentation-time`) is a separate
design and stays open.

## What landed

`zwp_relative_pointer_manager_v1` (version 1) and `zwp_pointer_constraints_v1`
(version 1), both advertised to every client with no filter.

Two findings on the way in, both against the bundle entry's assumptions,
both verified in source rather than assumed:

- **Smithay carries both protocols at the pinned rev** (`0ff0098`:
  `src/wayland/relative_pointer.rs` and `src/wayland/pointer_constraints.rs`),
  so this is the PR #88 shape: two hold-alive fields on `State`
  (`relative_pointer_manager_state`, `pointer_constraints_state`), no
  hand-rolled handler, no Smithay patch. The blanket `Dispatch` in
  `dispatch.rs` forwards everything to Smithay's own `Dispatch2` impls, so
  that file is untouched.
- **The `pointer_constraints` "support already present" was not.**
  What was in-tree was one empty `impl PointerConstraintsHandler for State`
  (the trait bound `WlSurface`'s `PointerTarget` impl needs to compile at
  all) -- no `PointerConstraintsState`, no global, no lock path. A client
  could not lock, so the "existing constraint path" the item assumed did
  not exist. This item advertises the global too, with anvil's policy
  (activate at creation when the surface already has pointer focus), plus
  the half anvil runs post-motion that a creation-time-only policy misses:
  a lock taken before first focus engages when focus arrives
  (`engage_pending_constraint`, enter-gated so steady-state motion never
  pays for it). Advertising a lock that can never activate would have been
  the worse lie; the small scope excess over "no pointer-constraints
  changes" is recorded here rather than hidden.
- **Relative events are focus-gated, not lock-gated.** The bundle entry
  assumed they flow only while locked. The protocol XML says otherwise
  (`zwp_relative_pointer_v1`: "will only emit events when it has focus"),
  and Smithay's routing matches (no constraint check anywhere on the
  `WlSurface::relative_motion` path). Gating on lock state would have been
  a bespoke deviation that breaks real clients reading relative motion
  without locking, so the compositor emits on every focused motion and the
  tests pin the correction explicitly: focused-without-lock receives,
  unfocused receives nothing.

The motion core (`State::move_absolute` in `input.rs`, the one funnel every
pointer source reaches) now resolves each move against the pre-move focus
surface's constraint (`absolute_target` in the new `relative_pointer.rs`):
an active lock holds absolute motion, an active confinement clamps per axis
to its region and refuses surface-leaving moves, and the relative event --
emitted first, with the full unclipped vector -- flows in all three cases.
`--tty` libinput threads both delta pairs through
(`pointer_move_relative` grew `dx_unaccel`/`dy_unaccel`), so
`dx_unaccel`/`dy_unaccel` are the pre-acceleration device values; every
absolute source applies no acceleration of its own, so both pairs carry the
position change there.

## Edge cases (all pinned by tests)

- **Teleport onto a surface reports nothing.** Nobody had focus when the
  motion began; the `enter` arrives, no `relative_motion` does.
- **A focus-changing teleport is credited to the surface it leaves.**
  Forced by Smithay's dispatch (`PointerInnerHandle::relative_motion`
  routes by seat focus, ignoring the passed focus), so the emission runs
  before the absolute motion -- anvil's order. Pinned with the exact switch
  vector, not just presence.
- **Lock holds absolute, keeps relative.** Deltas while held are measured
  from the held point (pinned: `(70,70)` then `(100,100)`, not `(30,30)`).
- **Unlock resumes absolute and keeps relative.** Destroying a `Persistent`
  lock is silent (no `unlocked` event -- Smithay only sends those from an
  explicit deactivate), pinned as absence.
- **Confinement holds past the surface, lands inside it**, and resumes
  when released (destroy likewise silent, no `unconfined`).
- **Unclipped at the output edge.** A 5000px device delta from a focused
  window reports the whole vector while absolute clamps at 1599.
- **Pre-accel pair threading.** `(10, 0)` accelerated with `(3, 0)` raw
  reports exactly that, far enough apart to fail loudly on a mix-up.
- **Disconnect with live lock + relative pointer is clean.** Smithay tears
  both down from its own destruction hooks; a fresh client afterwards locks
  and streams normally.
- **Two clients get separate streams**, each hearing only its own focused
  motion (Smithay's `same_client_as` routing).
- **A lock taken before focus engages on arrival** (the
  `engage_pending_constraint` half; proven fail-first by neutering the
  call).

## Evidence

- Fail-first, dev VM (`ssh -p 2222 dev@localhost`, branch
  `feat/relative-pointer-v1`, `state.rs` stashed to a pre-fix tree): 10 of
  11 fail -- `both_manager_globals_are_advertised` panics on `no
  zwp_relative_pointer_manager_v1 -- the global is missing`, the
  delta/lock/confine/stream tests on empty streams or client lock errors;
  `a_teleport_onto_a_surface_reports_no_relative_motion` passes trivially
  (it asserts emptiness). The engage test additionally proven to pin its
  own call: with the call neutered it fails on `arriving focus did not
  engage the waiting lock`.
- Post-fix, same VM: `cargo test -p flexwm` 831 + 3 pass (12 new),
  `cargo nextest run --workspace` 936 pass 1 skipped,
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean (Mac-side),
  `scripts/smoke-test.sh` (`SMOKE_PREFIX=/tmp/smoke-relptr`, `--headless`)
  exit 0 with zero `BUG` lines.
- Live, same VM: `wayland-info` against a headless flexwm lists
  `zwp_pointer_constraints_v1, version: 1` and
  `zwp_relative_pointer_manager_v1, version: 1`; real `foot` sees both in
  its registry (`WAYLAND_DEBUG` sighting) and binds neither -- foot uses
  neither protocol, which is expected. No game client exists on the VM, so
  bind/create/event flow is harness-proven, not live-proven; stated as an
  environment limit.
- Benchmark (the motion hot path is touched, so measured before/after on
  the dev VM, debug build, 200k `pointer_move` events x 5 reps, temporary
  bench since removed): unfocused 3129-3426 ns/event after vs 3415-3623
  before -- ranges overlapping, no measurable regression. Focused with no
  relative pointers bound 10688-11501 vs 8555-9090 before -- one
  `current_location` read, one constraint-map lookup and Smithay's
  empty-list lock per event, all debug-inflated; ~2.2ms/s at a real 1000Hz
  device rate, beside the per-object socket writes a live relative
  pointer pays anyway.

## What this deliberately leaves open

- Nothing on relative-pointer itself. Its bundle neighbour
  (`presentation-time`) is untouched and stays in
  `docs/backlog/protocols/protocol-gaps-general.md`.
- Constraint re-arming after pointer-leave deactivation is not implemented
  (Smithay deactivates; nothing re-arms): a lock whose surface loses focus
  stays disarmed until the client re-locks. Anvil-equivalent behavior
  would re-engage on re-entry; filed implicitly here rather than as its own
  item, since no shipped client has asked for it.
