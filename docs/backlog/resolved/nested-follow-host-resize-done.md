---
title: "`--nested` ignores every configure after the first, so a webtop session is stuck at its starting size — LANDED 2026-09-19 (PR #151)"
status: "resolved"
area: "core"
priority: "high"
blocked: null
---

# `--nested` ignores every configure after the first, so a webtop session is stuck at its starting size — LANDED 2026-09-19 (PR #151)

**What landed is at the bottom of this file** ("What landed", below). The
diagnosis above it is the original entry, unchanged — including one claim
it made that turned out not to hold, which the bottom section corrects.

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

## What landed

The early return is gone. Every `xdg_surface::Configure` is now classified
by one pure function, `nested::configure_action(configured, proposed,
current)`, into one of three outcomes, and the dispatch file holds no
failure policy of its own:

- `FirstConfigure` → `Host::apply_first_configure`, whose failure is fatal
  and stops the loop, as before.
- `Resize` → `Host::apply_resize`, whose failure logs at `warn!` and keeps
  the session at the size it was.
- `Nothing` → a configure proposing the size scoot is already at.

That third outcome is not an optimisation. Hosts re-send a configure at an
unchanged size on every state change that is not a resize (activation,
maximize, a tiling-edge update), so without it every focus change in the
host would throw away a working render target and buffer pool to build an
identical pair — and under `--renderer gles` that is a whole new EGL
context and shader set per focus change.

### The asymmetry, written where the function is

Two entry points over one private `replace_render_target`, rather than one
function reading `is_configured()` to decide what its own failure means.
`apply_resize` returns no `Result` at all, so "fatal" is not a thing a
caller can accidentally do with it. This is the shape `CLAUDE.md`'s worked
example (a field meaning two things at two sites) argues for.

### One claim in the diagnosis above was wrong, and it mattered

The entry says `apply_size` "already guarantees the losing case is
consistent: it restores `state.host` and destroys the new pool before
returning `Err`". That is true of *one* of its two failure branches. In the
other — `resize_output` succeeded and `BufferPool::new` then failed, which
is precisely the "a bigger buffer pool could not be allocated" case the
whole non-fatal argument rests on — the render target was left at the **new**
size with the host buffers at the **old** one. `present()`'s size guard
drops every frame in that state, silently and for good: under the old code
the loop stopped so nobody noticed, and under the new policy it would have
been a live session whose window never updated again. Worse than either
failure policy was meant to allow.

Fixed by reversing the order rather than by adding a rollback: allocate the
new pool first, while nothing is committed, and only touch the render target
once it is in hand. Now `Err` really does mean nothing moved.

`State::resize_output` had a smaller version of the same shape, shared with
`--tty`'s hotplug path: it calls `set_mode` (which tells every bound
`wl_output` client the new mode *synchronously*) before `Backend::new`, so a
failure left clients believing a size nothing would ever render at, with
nothing to resend it. It now puts the advertised mode back on that path.
Only the metadata is restored, never the render target — which was never
torn down, so the restore cannot itself fail, which is what distinguishes
this from the "a failure at one size is not evidence the old size would
work" note that function already carried.

### Host-proposed sizes are now bounded

A host configure is acted on only within `1..=65535` per axis —
`cli::MAX_OUTPUT_DIMENSION`, the same range `--width`/`--height` parse into,
for the same reason (DRM stores a mode axis in a `u16`). Out of range is
ignored with a `debug!`, not a `warn!`: a host that proposes it once
proposes it on every configure, which is the flood the sibling entry
(#145) is about. Separately, `BufferPool::new` now checks its own
arithmetic instead of `as i32`-ing the pool's byte count — 65535x8192 is
inside the per-axis bound and past what `wl_shm.create_pool` can name, and
handing the host a truncated (possibly negative) size is a protocol error,
i.e. a compositor killed by a size the host itself proposed.

### The two "worth checking" items

- **`render.rs`**: nothing needed. `--nested` always passes buffer age `0`,
  so the damage bbox is the whole framebuffer, and `Backend::new` brings a
  fresh damage tracker — the first frame after a resize is a full redraw at
  the new size. `resize_output` ends in `apply()`, which ends in
  `request_render()`, so that frame is asked for exactly once.
- **Multi-output**: no new single-output assumption. The resize path reaches
  the output only through `State::resize_output`, which is where the
  `State.output` work already has to land; `nested.rs` gained no reference
  to `state.output` at all.

### What it cost, and what grows

A resize rebuilds the render target and the pool, so it is not free.
`headless/bench.rs` gained a `resize_cost` scene for it (`#[ignore]`d, like
`render_frame_cost`); measured on the dev VM at 800x800 with 8 windows,
best of five runs of 40 resizes:

| renderer | per resize |
| --- | --- |
| pixman (the default) | **37.3 µs** |
| gles (llvmpipe on this VM) | **16.6 ms** |

pixman is 0.2% of a 60 Hz frame — a drag is free. `gles` is a whole frame
apiece, because each rebuild is a new EGL context and shader set, so a
`--nested --renderer gles` window dragged to resize will stutter until you
let go. It is once per *distinct* size, not per event, and `gles` is opt-in
under a backend the CHANGELOG already describes as buying "correctness
parity rather than speed" — recorded in `docs/tty.md` rather than fixed
here, with the fix named (resize the GLES target in place instead of
rebuilding it) if it ever matters.

The one line that *did* flood as a result was fixed: see the sibling entry's
"What landed" for `render/gles.rs`'s "the GLES renderer is up".

The mode list grows by one per *distinct* size the host has configured the
window to (`Output::modes` dedupes by value), so a drag through many sizes
leaves one mode each; `Nothing` keeps same-size configures out of it
entirely. `docs/protocols.md` and `output_management.rs`'s module doc both
say so now, where they used to say `--nested` grew the list "at most once".
