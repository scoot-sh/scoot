---
title: "`--nested` ignores every configure after the first, so a webtop session is stuck at its starting size"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# `--nested` ignores every configure after the first, so a webtop session is stuck at its starting size

Issue #144, filed 2026-09-19 from running scoot nested inside **webtop** —
the deployment target `README.md` names. Everything else worked; this is the
one thing that made the session feel wrong to use, which is why it is high
rather than medium: it is the named target, in normal use, every time the
browser window is resized.

Selkies resizes pixelflux's output on a browser resize and pixelflux sends
scoot's toplevel a fresh `xdg_toplevel::Configure`. scoot keeps its original
size and pixelflux letterboxes the difference, so the desktop never fills
the window at any size other than the one it started at.

## It is a documented scope boundary, not a bug

`compositor/nested_dispatch.rs:119-124`, verified still present at
`2162da8`:

```rust
if host.is_configured() {
    // Scope boundary (v1): only the first configure is acted on.
    return;
}
```

And the plumbing is already most of the way there, which is what makes this
worth doing now rather than later:

- `Dispatch<HostToplevel>` records **every** configure through
  `host.set_pending_size(width, height)` — later sizes are captured, just
  never consumed.
- `ack_configure` already happens before the early return, deliberately.
- `Host::apply_size` already does exactly what a resize needs, atomically:
  `resize_output` plus a new `BufferPool` at the new size, with the "never
  leave the render target and the host buffers at different sizes" guard,
  and the old pool destroyed rather than leaked.

So the change is roughly: on a later configure, if `take_pending_size()`
differs from `host.size`, call `apply_size` again instead of returning.

## The design question, answered

The issue asks for an opinion before anyone writes it, and the answer is
**yes, split the failure behaviour by when it happens** — this is the
interesting part of the item.

`apply_size`'s failure path currently stops the event loop:

```rust
tracing::error!(%error, "could not set up the nested backend's render target …; stopping");
state.loop_signal.stop();
```

That is right for the **first** configure and its doc argues the case well:
a mismatch there is permanent and silent, so failing loudly beats running
wrong.

It is wrong for a resize five hours into a session. `CLAUDE.md` treats a
plausible hang-or-crash path with the same severity as data loss, precisely
because a compositor going down takes every client's unsaved state with it —
and ending a live desktop with windows open in it because a *bigger* buffer
pool could not be allocated is that, for a failure the user did not cause
and cannot avoid.

The reason this is safe to make asymmetric is that `apply_size` already
guarantees the losing case is consistent: it restores `state.host` and
destroys the new pool before returning `Err`. So "log, keep the old size,
carry on" leaves a working session at the previous size, which is strictly
better than no session. Letterboxing is a visual annoyance; a dead
compositor is lost work.

**Write the asymmetry down where the function is**, not only here. One
function whose failure means "stop the process" on one call path and "keep
going" on another is exactly the shape `CLAUDE.md`'s worked example warns
about — a field meaning two things at two sites, invisible to tests, found
only by reading. Make the caller pass the intent, or split the entry point,
rather than leaving `apply_size` to guess.

## Repro (no webtop needed)

```sh
scoot --nested --width 1280 --height 800 -- foot
# resize the host window
scoot msg outputs   # rect is still 1280x800
```

## Worth checking while in here

`--nested` is one of the two backends that still reads every frame back
(see `docs/tty.md`); a resize reallocates the pool, so the interaction with
`render.rs`'s size handling wants a look rather than an assumption. And
`docs/backlog/core/multi-output-foundation.md` will turn `State.output`
into a collection — a resize path that assumes one output should not be
written in a way that has to be unpicked immediately afterwards.
