---
title: "`xdg-activation-v1`: flexwm's own `spawn` hands its child no activation token."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# `xdg-activation-v1`: flexwm's own `spawn` hands its child no activation token.

Follow-up from implementing the protocol
(`docs/backlog/resolved/foot-protocol-warnings-done.md`). The convention is
that whoever starts a process puts an activation token in the child's
`$XDG_ACTIVATION_TOKEN`, so the child can activate its own window when it
finally maps one. `State::spawn` (used by `Action::Spawn`, i.e. every
keybinding and IPC `spawn`) sets `WAYLAND_DISPLAY` and `FLEXWM_SOCKET` but no
token.

Consequence today: an app started *by flexwm* cannot ask to be focused the
way one started by a launcher client can. In practice this is invisible,
because flexwm already focuses a newly mapped window itself
(`add_window` passes `focus: true`) — which is exactly why it was not
bundled into the initial implementation. It starts to matter when a spawn
takes long enough that the user has moved focus elsewhere in the meantime,
and it is the sort of environment detail a toolkit may check for other
reasons.

What it would take: `XdgActivationState::create_external_token` (which
deliberately does *not* call `token_created`, so it bypasses the 64-token
cap and the freshness sweep would need thinking about for a long cold start),
and setting the variable in `spawn`'s `Command`. Both small; the design
question is which of `activation.rs`'s two bounds should apply to a token the
compositor minted itself.
