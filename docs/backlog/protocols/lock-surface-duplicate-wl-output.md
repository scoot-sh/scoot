---
title: "One physical output can hold unboundedly many lock surfaces if the lock client binds `wl_output` more than once"
status: "open"
area: "protocols"
priority: "low"
blocked: "needs multi-output"
---

# One physical output can hold unboundedly many lock surfaces if the lock client binds `wl_output` more than once

One physical output can hold unboundedly many lock surfaces if the lock
client binds `wl_output` more than once (item 18, found by round three's
review through source reading — not exercised, and not a privilege
escalation). The protocol's one-surface-per-output rule is enforced by
Smithay, whose `SessionLockState` keeps a `Vec<WlOutput>` of locked outputs
and compares `WlOutput` resource identity (`locked_outputs.contains(&output)`
in `session_lock/lock.rs`). A client may bind the same `wl_output` global
any number of times, and each bind is a different resource, so `lock`,
`get_lock_surface(bind_1)`, `get_lock_surface(bind_2)`, … all succeed for
one physical output. flexwm's `new_surface` accepts each (they pass
`is_current`: same lock, live surfaces) and `SessionLock::current`
composites every mapped one, so the render list grows with the number of
binds, and the first in creation order keeps the keyboard. Only reachable
by whoever already holds the lock — they already own the whole screen, so
there is nothing to escalate to, and the cost is their own memory plus the
compositor's per-frame work over a list they control. Tearing that list
down is quadratic (every surface destruction re-derives focus over what is
left), which predates round three's hook and is not made worse by it: the
`retain` in `State::forget_lock_surface` was already O(list) per destroyed
`wl_surface`. The fix belongs in
the same place multi-output does: resolve each `wl_output` to its
`Output` (`Output::from_resource`) and treat *that* as the key, in
`new_surface`, rather than trusting Smithay's resource-identity guard.
