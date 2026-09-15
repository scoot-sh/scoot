---
title: "`ext-idle-notify-v1` and `idle-inhibit-unstable-v1` — pairs naturally with session-lock — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `ext-idle-notify-v1` and `idle-inhibit-unstable-v1` — pairs naturally with session-lock — RESOLVED.

## Resolution (2026-09-15)

Both globals implemented flexwm-side, in a new `idle.rs`:

- **`ext_idle_notifier_v1` (version 2)**: Smithay's `IdleNotifierState`
  owns the globals and the per-notification calloop timers; flexwm adds
  the `IdleNotifierHandler` accessor and calls `notify_activity` from
  the four input choke points (`pointer_move`, `pointer_button`,
  `scroll`, `key` in `input.rs`), so libinput, host-forwarded, and
  IPC-injected input all reset idle timers -- including input a
  keybinding intercepted. The lock-transition focus refresh deliberately
  bypasses it (`pointer_move_quietly`): the compositor re-running its
  own hit test is not a user at the machine.
- **`zwp_idle_inhibit_manager_v1` (version 1)**: `IdleInhibitHandler`
  records inhibiting surfaces in a set (surfaces, not inhibitor
  objects -- a double-create releases with one destroy) and pushes the
  aggregate into the notifier. Death releases: `CompositorHandler::destroyed`
  forgets the surface, so a client that disconnects without destroying
  its inhibitors stops holding the session awake. Only *live* surfaces
  inhibit; visibility is deliberately not considered (see `idle.rs`'s
  scoping note -- re-deriving on every map/unmap commit would couple
  the hot path, and local clients are trusted per the README trust
  note).
- **No built-in auto-locker**: the protocol is the feature; policy
  (which timeout locks) belongs to the user's daemon, the swayidle way.
  No config key, no compositor timer.

Seven harness tests (`idle/tests.rs`, real protocol client, real
calloop timers): idle-then-resume-then-idle-again cycle with exact
`idled`/`resumed` counts (the protocol forbids doubles, so exactness is
meaningful), inhibit-holds-until-released, surface-destroy releases,
inhibit-while-idle resumes and holds, input-idle ignores inhibitors,
duplicate-inhibit single release, zero-timeout fires. The resume test
was verified to fail with the activity announcement disabled. One
bug-bash catch en route: a stale explicit destroy after a surface death
must *not* re-arm a second `idled` -- pinned by the destroy test.

Field proof with real swayidle 1.9.0 on the dev VM (headless): 5s
quiet fires the timeout command, IPC pointer input runs the resume
command, a further quiet window fires again -- daemon alive and
confirmed at every stage, zero protocol errors in the server log. (Its
`BlockInhibited`/logind complaint is VM-environment absence, same
category as the probe "not gaps" lists.)

Original entry, left as written:


# `ext-idle-notify-v1` and `idle-inhibit-unstable-v1` — pairs naturally with session-lock.

`ext-idle-notify-v1` and `idle-inhibit-unstable-v1` — pairs naturally
with session-lock. User request, 2026-09-13. `ext-idle-notify-v1` is
what lets an external tool (a `swayidle`-style daemon) learn "the user
has been idle N seconds" so it can dim the screen, lock it, or suspend
the machine — without it, `ext-session-lock-v1` above has no automatic
trigger, only a manual one. `idle-inhibit-unstable-v1` is the reverse:
lets a client (a video player, a presentation app) tell the compositor
not to consider the session idle while it's active. Neither has design
work done; natural to scope alongside session-lock since they're the
same feature area (idle/lock lifecycle), not before it. Now the obvious
next pick in that area: item 18 landed the lock itself, and without an
idle notification nothing can trigger it automatically — locking is
whatever the user runs by hand.
