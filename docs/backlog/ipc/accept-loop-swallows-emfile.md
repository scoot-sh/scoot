---
title: "The IPC accept loop swallows `EMFILE` and can spin the event loop at 100% (LOW)"
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# The IPC accept loop swallows `EMFILE` and can spin the event loop at 100% (LOW)

Filed 2026-09-16, out of the connection-cap review. Pre-existing and
untouched by that change — named here because it was found while reading
that code, and because a latent whole-compositor busy-spin belongs written
down rather than in a PR description.

`ipc.rs`'s accept callback is:

```rust
while let Ok((stream, _)) = listener.accept() {
    ...
}
Ok(PostAction::Continue)
```

`while let Ok(..)` leaves the loop on *any* error, not just the
`WouldBlock` that means "no more connections pending", and says nothing
about it. The listener is registered `Mode::Level`, so a connection still
sitting in the backlog is reported again on the very next turn of the loop
— and if `accept` keeps failing for a reason that is not going to clear on
its own, the event loop takes that wakeup, fails again, and goes round
immediately. The compositor spins at 100% CPU with nothing to show for it.

The realistic way in is `EMFILE`/`ENFILE`: the process (or the machine)
is out of file descriptors. Note that this does *not* have to be IPC's
fault — every wayland client, every DRM device, every `wl_shm` pool maps
into the same table — so the connection cap that now bounds IPC's own fd
use makes this less reachable, not unreachable.

What a fix looks like, roughly: match on the error kind, treat
`WouldBlock`/`Interrupted` as "done for now" (the current behaviour),
log anything else, and for the fd-exhaustion kinds specifically, refuse the
pending connection in a way that actually consumes it — the usual trick is
to keep one spare fd open, close it, `accept` and immediately drop, reopen
it — or disable the listener source for a short back-off rather than
returning `Continue` into an immediate re-report. Worth a test that
deliberately exhausts the process's `RLIMIT_NOFILE` and asserts the loop
does not spin.
