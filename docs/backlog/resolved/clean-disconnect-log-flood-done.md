---
title: "A clean client disconnect logs at INFO, so an idle webtop session emits two lines a second forever — LANDED 2026-09-19 (PR #151)"
status: "resolved"
area: "ipc"
priority: "medium"
blocked: null
---

# A clean client disconnect logs at INFO, so an idle webtop session emits two lines a second forever — LANDED 2026-09-19 (PR #151)

**What landed is at the bottom of this file** ("What landed", below). The
diagnosis above it is the original entry, unchanged.

Issue #145, filed 2026-09-19, observed on `7e415b6` — the second thing from
the same webtop deployment as
[#144](./nested-follow-host-resize-done.md). Cosmetic in the sense that
nothing is broken, and not cosmetic in the sense that it makes the log
unusable for anything else.

```
INFO scoot::compositor::state: wayland client disconnected id=InnerClientId { id: 1, serial: 3 }
INFO scoot::compositor::state: wayland client disconnected id=InnerClientId { id: 1, serial: 4 }
...
```

Nothing is wrong: that is the clipboard working. Selkies' clipboard monitor
shells out to `wl-paste --list-types` on a **500 ms** loop, and each poll is
a fresh Wayland connection that opens and closes. **Every Selkies-based
deployment does this**, so it applies to the whole webtop use case rather
than to one setup.

The real cost is diagnostic: a genuinely useful log gets buried — the
reporter hit it while diagnosing something else — and `serial` incrementing
monotonically makes an idle session look busy at a glance.

## The site

`compositor/state.rs:1052`, verified at `2162da8`:

```rust
DisconnectReason::ConnectionClosed => {
    tracing::info!(?id, "wayland client disconnected");
}
DisconnectReason::ProtocolError(error) => {
    tracing::warn!(?id, ?error, "wayland client killed by a protocol error");
}
```

## The fix, and why this shape

Demote **only** the `ConnectionClosed` arm to `tracing::debug!`. Leave
`ProtocolError` at `warn!`.

That keeps the thing this logging exists for. The doc comment above it
records that a protocol error "used to leave no trace at all in scoot's
log, which is how a compositor-side kill of a layer-shell client stayed
undiagnosed" — and every bit of that diagnostic value is in the
`ProtocolError` arm. A client closing its own connection cleanly is the
uninteresting case, and it is the only one that repeats.

The issue offers rate-limiting as an alternative. Prefer the demotion: a
rate limiter adds state, a clock and a decision about the window to a log
line, and the information it would preserve is already available at
`debug!` for anyone who wants it. Reach for a limiter when the noisy thing
is something you cannot afford to lose — this is not that.

## Check while in here

Whether any *other* per-connection INFO line has the same property. The
same 500 ms loop opens a connection, binds globals and tears down, so
anything logging at INFO on bind or on global advertisement will flood
identically on this deployment and nowhere else — which is exactly why it
was not noticed before someone ran it under Selkies.

## What landed

Exactly the demotion this entry asked for: `state.rs`'s `ConnectionClosed`
arm is `tracing::debug!`, `ProtocolError` is untouched at `warn!`. No rate
limiter.

**The "check while in here" audit came back empty, and was checked twice.**
By reading: every logging site in the workspace is a fully-qualified
`tracing::` macro (there is no `use tracing::info` anywhere), so the whole
set is greppable, and `state.rs:1053` was the only INFO on a per-connection
path. The accept loop (`wayland_accept.rs`) logs only on failure, at
`warn!`; the bind budget logs at `debug!`; `dmabuf.rs`'s two INFO lines are
once-per-session by construction (a startup device probe, and a
first-import-only transition guarded by `imports_dmabufs`). Then by
measurement, which also covers any INFO a *bind* or a global advertisement
might have produced that reading missed: ten `wayland-info` runs against a
live `--headless` session — each a full connect, bind-every-global,
tear-down cycle, the same shape as Selkies' `wl-paste` poll — added **zero**
lines to an `info`-level log (7 lines before, 7 after), and the same ten
produced exactly **10** `DEBUG ... wayland client disconnected` lines under
`RUST_LOG=scoot=debug`. The information is still there for anyone who wants
it; it is just no longer in everyone's way.

**The audit did catch one line, and the sibling change is what made it
catchable.** `render/gles.rs` logs `the GLES renderer is up` at INFO on
every successful `GlesBackend::new`, which `State::resize_output` comes
back through — so once #144 made `--nested` follow host resizes, a
`--renderer gles` session dragged to resize would emit it at host frame
rate. Not per *connection*, which is why the reading pass above did not
reach it, but the same shape and newly reachable at rate. It logs once per
process now and `debug!`s the rebuilds, matching `dmabuf.rs`'s
first-import pattern — and the message was wrong on a rebuild anyway: "the
GLES renderer is up" is news exactly once.
