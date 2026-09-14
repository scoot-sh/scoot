---
title: "flexwm blanks the screen immediately on a lock request rather than waiting for the lock client's first surface"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# flexwm blanks the screen immediately on a lock request rather than waiting for the lock client's first surface

flexwm blanks the screen immediately on a lock request rather than
waiting for the lock client's first surface (item 18, deliberate). niri
waits up to a second for lock surfaces so the transition doesn't flash
black; the cost of that is rendering the *unlocked* session for that whole
second, which is the wrong half of the trade to take first. Worth
revisiting as a comfort feature if the black flash proves annoying in
daily use — with a hard deadline, as the protocol requires.
