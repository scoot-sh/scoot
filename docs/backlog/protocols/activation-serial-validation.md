---
title: "`xdg-activation-v1` has no input-serial gate, so an unfocused client can self-activate."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# `xdg-activation-v1` has no input-serial gate, so an unfocused client can self-activate.

Found by independent review of `docs/backlog/resolved/foot-protocol-warnings-done.md`
(the four-protocols PR). Not a crash and not a regression — before that work
there was no way for a client to ask for focus at all — but the two bounds
`compositor/activation.rs` documents (`TOKEN_LIFETIME`, `MAX_TOKENS`) were
being described as the answer to focus-stealing when they are resource bounds
only, and don't stop the actual case.

A client with no keyboard or pointer focus, and no user interaction at all,
can call `get_activation_token`, then immediately `activate(token,
its_own_surface)`. The token is milliseconds old (well under
`TOKEN_LIFETIME`) and the token table is nowhere near `MAX_TOKENS`, so both
checks pass and focus moves to a surface the user never touched. Nothing
about this is rate-limited or attributable to a specific misbehaving client
after the fact.

The protocol's own answer to this is the token's optional seat and input
serial (`xdg_activation_token_v1.set_serial`): a compositor can refuse to
honor a token that wasn't created against a recent, real input event. flexwm
does not check it today — `activation.rs`'s module doc previously justified
skipping it with reasoning that doesn't hold up (a client that can "just
click first" already has focus and gains nothing from self-activation; the
interesting case is the client that never gets clicked at all). The doc has
been corrected to state this as a known gap rather than a considered
trade-off; the behavior itself is unchanged.

What it would take: track the seat's last input serial (something already
needed for other protocols that reference serials), and in `request_activation`
require the token's stored serial to be recent/valid before honoring
`activate` — refusing (silently, per the existing policy) otherwise. The
care needed: real launchers create a token from a keyboard-driven selection
and hand it to a process that takes a second or more to cold-start before
redeeming it, so "serial is recent" has to mean "was valid when the *token*
was minted," not "is still the current serial when `activate` is called" —
otherwise the one case this protocol exists for breaks.
