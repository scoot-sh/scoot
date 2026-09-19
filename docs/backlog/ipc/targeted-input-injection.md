---
title: "Input injection targeted at a specific window, without moving seat focus (research-backed idea, 2026-09-12 \u2014 a major goal per `CLAUDE.md`, not automatically ahead of daily-drivability items like layer-shell; see that file's current framing before assuming either wins by default)."
status: "research"
area: "ipc"
priority: "research"
blocked: null
---

# Input injection targeted at a specific window, without moving seat focus (research-backed idea, 2026-09-12 — a major goal per `CLAUDE.md`, not automatically ahead of daily-drivability items like layer-shell; see that file's current framing before assuming either wins by default).

Input injection targeted at a specific window, without moving seat
focus (research-backed idea, 2026-09-12 — a major goal per `CLAUDE.md`,
not automatically ahead of daily-drivability items like layer-shell; see
that file's current framing before assuming either wins by default).
The single most concrete gap identified while researching related
projects: an agent
driving a real desktop needs to act on a window that is not the one
currently focused, without stealing focus away from whatever a human (or
another agent) has in the foreground. A recent deep-dive on Linux
computer-use automation names this as one of the hardest unsolved
problems in the space — every existing tool hacks around it per-toolkit
(different tricks for GTK3/GTK4/Qt/Electron) because no compositor
exposes a clean primitive for it. A real automation project hit exactly
this wall trying to use niri: screenshot capture worked, but every input
action was refused, with the literal error "no trusted Wayland compositor
adapter can confirm exact target window" — niri's own IPC can identify
and activate a specific window, but nothing lets a caller inject input
*into* one without first making it the real focused window.

**scoot is unusually well-positioned to solve this properly**, since it
owns the entire Wayland dispatch stack itself rather than automation
being bolted onto a desktop environment that wasn't built for it, and
half the identity/targeting problem is already solved:
`Request::Action(FocusWindowId(id))` already proves by-ID window
targeting exists in the IPC surface today. What's missing is a sibling
request that dispatches input at a window ID *without* calling that real
focus-changing path.

**Why this isn't a trivial tweak, so scope it honestly.** Wayland's
keyboard/pointer protocols are focus-gated at the client level — a client
only processes key/pointer events after a matching `enter`, and most
toolkits track "am I focused" from that sequence, not the raw events
alone. So the real mechanism is not "inject with literally zero focus
interaction" — the same research above notes real implementations often
still need "synthetic focus events... without raising the app." The
actual design question is how to give the *target* surface a scoped
synthetic enter/key(or button)/leave sequence without touching the seat's
real global focus state (`Seat`/`KeyboardHandle`/`PointerHandle` in
Smithay) or visibly raising/activating that window — i.e. the target
briefly believes it is focused for exactly the injected event, while
every other client and the compositor's own idea of "what's actually
focused" is unaffected. This needs real investigation into what the
pinned Smithay revision's `KeyboardHandle`/`PointerHandle` actually expose
for this (a lower-level per-surface send path bypassing the seat's single
held-focus abstraction, if one exists) before committing to a design —
don't assume the API shape without checking, per this project's standing
rule on Smithay claims.

**Open question, not yet answered:** what does this mean for a window
that is not currently visible at all (scrolled off-screen in the
horizontally-scrolling layout, or on a different workspace/output)? Does
this project's render loop already process commits/frame-callbacks for
mapped-but-not-currently-visible windows (needed either way, independent
of this feature), or would that need its own fix first? Investigate
before scoping an implementation.

Concrete shape once designed: likely a new `Request` variant (naming
TBD — something like a `window` field added to the existing `Key`/
`Click`/`PointerButton` requests, or dedicated variants) in `scoot-ipc`,
implemented in `input.rs` alongside the existing focus-changing
dispatch it must *not* reuse.
