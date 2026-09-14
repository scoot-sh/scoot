---
title: "Suppress the same-VT no-op case of the IPC VT-switch warning (5c)."
status: "open"
area: "tty"
priority: "low"
blocked: null
---

# Suppress the same-VT no-op case of the IPC VT-switch warning (5c).

Suppress the same-VT no-op case of the IPC VT-switch warning (5c).
`change_vt`'s `VtSwitchOutcome::Requested` also fires — with a hedged
warning, per 5c — when the requested VT is the one the session is already
showing on, since libseat itself returns `Ok(())` for that request too
(confirmed on real hardware; see 5c's verification). A precise fix would
need `Tty` to track which VT it currently occupies and compare before
calling `session.change_vt`, so this case can be `Ignored` instead of a
hedged `Requested`. Not done in 5c because libseat exposes no query for
"what VT is this session on" at `init` time — the number would have to be
sourced some other way (the kernel's own active-VT ioctl on the console
fd, perhaps) and 5c's warning-not-guarantee wording already makes this a
false-positive risk, not a silent-failure one, so it wasn't worth blocking
5c on. Low priority: cosmetic (an agent gets told to be careful once for
no reason), not correctness-affecting.
