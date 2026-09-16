---
title: "The pointer starts at the output's origin, so the cursor sits wedged in the top-left corner until the first motion."
status: "open"
area: "rendering"
priority: "low"
blocked: null
---

# The pointer starts at the output's origin, so the cursor sits wedged in the top-left corner until the first motion.

Nothing places the pointer at startup, so Smithay's seat leaves it at (0,0)
and `--tty` — the one backend that draws a cursor — draws the arrow in the
extreme top-left corner of the screen, hotspot exactly on the corner, until
the user moves the mouse. Centring it on an output is the more familiar
behaviour, and the likely fix, but which compositors actually do that has not
been checked against their source here — don't take it from this entry.

Purely cosmetic, and not a correctness bug: a cursor has to be *somewhere*
before the first motion event. Found while resolving
[`tty-background-not-painted-done.md`](../resolved/tty-background-not-painted-done.md),
where it was the reason a corner pixel the smoke test sampled as "background"
read as the cursor's black outline instead. That test now parks the pointer
before capturing, so this is no longer blocking anything.

Worth an item of its own rather than a drive-by, because "centre it" has real
decisions in it: on which output once there is more than one; whether the
placement counts as pointer motion for idle/activation purposes (it must not
— see `pointer_move_quietly`'s comment on exactly that distinction for
`refresh_pointer_focus`); and whether a `--tty` session that reactivates after
a VT switch re-places it or leaves it where the user left it.
