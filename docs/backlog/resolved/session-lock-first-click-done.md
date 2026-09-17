---
title: "The first click on a fresh lock screen, before the mouse has moved, reaches nobody — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The first click on a fresh lock screen, before the mouse has moved, reaches nobody — DONE

## Resolution

Fixed as the ticket sketched: pointer focus is re-derived on the commit
that maps a lock surface. `new_surface`'s own re-derivation stays — it is
what moves the pointer *off* the window underneath at lock time — and the
keyboard half already worked there (keyboard focus goes to a lock surface
on liveness, not mapped-ness, arriving at +2ms in the original measurement).
What was missing was only the pointer half at map time.

Two call sites, both in the shape the codebase already uses:

- `handlers.rs::commit` claims a commit for a window (`id_of`) first, then
  for a layer surface (`commit_layer_surface`), and only when neither
  claims it asks `session_lock.rs::refresh_lock_pointer_focus`. Ordinary
  window commits and layer-surface commits therefore never reach the new
  code at all.
- `refresh_lock_pointer_focus` returns on one `Option::is_some` while
  unlocked, and while locked adds one `with_states` typemap probe for
  `LockSurfaceData` (a miss for every surface that never had a lock role;
  no allocation on any path) before running the same
  `refresh_pointer_focus` every lock transition runs.

The cost, stated explicitly as the ticket asked: one predictable branch
per non-window, non-layer commit while unlocked; one typemap probe more
while locked; the hit test and motion only for actual lock surfaces.
Zero new fields and zero new state — there is nothing that can drift out
of step the way item 5b's two-meaning field did.

Precedence needed no special case, and that is pinned rather than
assumed: the refresh runs the same locked hit test every other derivation
runs, which sees lock surfaces only, so an `overlay` layer surface mapped
before the lock cannot steal the enter (new test), an open popup grab is
dropped at lock time as before, and a grab asked for while locked is still
refused — the PR #44 password-leak guarantee, with the full session-lock
and popup suites green underneath.

The symmetric case the ticket did not name — lock surface *unmapping*
(unlock) — already worked: `unlock` re-derives pointer focus, and a new
test pins focus returning to the window underneath with no mouse move
(passing before and after, i.e. evidence, not a fix). True unmapping
mid-lock cannot arrive by commit at all: Smithay's pre-commit hook posts
`NullBuffer` for one, so the only unmap paths are role/surface
destruction and disconnect, each of which already runs `lock_transition`.
A lock surface that acked but attached no buffer takes no focus, also
pinned.

## Evidence

- Fail-first, dev VM, pre-fix tree (`8ae3b20` + uncommitted tests):
  `pointer_enter_reaches_the_lock_surface_at_map_time_without_any_pointer_movement`
  and `an_overlay_layer_surface_does_not_steal_the_map_commit_enter` fail
  with `left: None` (no enter without a mouse move — the bug);
  `an_unmapped_lock_surface_holds_no_pointer_focus` and
  `pointer_focus_returns_to_the_window_on_unlock_without_any_pointer_movement`
  pass. Post-fix all four pass; the toggle was re-validated by stashing
  just the two source files and watching the same two fail again.
- Benchmark (throwaway harness test, deleted after): 1000 bare commits on
  a mapped window, fixed tree 5824–5852us/commit vs pre-fix 5825–5831us
  across three reps each — ranges fully overlapping. Expected: the window
  path is byte-identical before/after (claimed by `id_of` first), and the
  per-commit harness round-trip noise (~ms) swamps a single branch (~ns)
  by six orders of magnitude either way.
- Full set green post-fix: `cargo test -p flexwm` (766 passed),
  `cargo nextest run --workspace` (865 passed), `cargo clippy -p flexwm
  --all-targets -- -D warnings` clean, `cargo fmt --check -p flexwm`
  clean, `scripts/smoke-test.sh` 15 ok / 0 fail.
- No live `--tty` re-measurement of the original +4975ms enter latency:
  no compositor held the dev-VM seat, but claiming DRM master and a VT
  from an ssh session to run a scripted lock client there risked leaving
  seat state behind for concurrent work, for a path with zero
  backend-specific code in it (commit dispatch → hit test → `enter` is
  identical on every backend). The harness wire evidence — `enter`
  arriving synchronously inside the map-commit dispatch with the pointer
  never moved — is the exact inversion of the original observation.
- README: the "What is *not* guaranteed" bullet describing this gap is
  removed, and the "Only the lock surface receives input" bullet carries
  the new sentence. User-observable behaviour changed, so this was a docs
  change, not an internal one.

Original entry, left as written:

# The first click on a fresh lock screen, before the mouse has moved, reaches nobody

The first click on a fresh lock screen, before the mouse has moved,
reaches nobody (item 18, found by round two's hardware bug-bash rather
than by any test). `new_surface` does re-derive pointer focus, but it runs
while the lock surface is still *unmapped*, so the hit test finds nothing;
the commit that maps it asks for a render and nothing else
(`handlers.rs::commit` falls through `id_of` to `commit_layer_surface`,
which a lock surface is not). Measured on real `--tty`: the locker gets
`wl_keyboard.enter` at +2ms and `wl_pointer.enter` only at +4975ms, when
the pointer was first moved. Safe direction -- the click goes nowhere, never
to something behind the lock screen -- and a locker is a keyboard-first
thing, so this is comfort rather than correctness, but it is the same class
as the bug `refresh_pointer_focus` was added for. The fix is presumably to
re-derive on the commit that maps a lock surface, which needs a cheap way to
recognise one in `commit` without making every ordinary window's commit pay
for the lookup.
