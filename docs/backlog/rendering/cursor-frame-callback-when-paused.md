---
title: "An animated cursor client keeps getting woken while the `--tty` session is VT-paused (LOW, inherited not introduced)."
status: "open"
area: "rendering"
priority: "low"
blocked: null
---

# An animated cursor client keeps getting woken while the `--tty` session is VT-paused (LOW, inherited not introduced).

An animated cursor client keeps getting woken while the `--tty` session
is VT-paused (LOW, inherited not introduced). `Tty::present` early-
returns on `!active`, but `render()` and the frame-callback loops run
regardless — identical to the pre-existing per-window `send_frame` loop;
item 8 just makes the cursor share it. Not new, not specific to cursors.
