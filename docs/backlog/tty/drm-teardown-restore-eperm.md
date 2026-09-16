---
title: "`--tty` quit sometimes logs a DRM \"restore previous state\" EPERM."
status: "open"
area: "tty"
priority: "low"
blocked: null
---

# `--tty` quit sometimes logs a DRM "restore previous state" `EPERM`.

Found independently twice: once during a reviewer's live `--tty` spot-check
of PR #41, once reproduced deliberately while testing VT-switch behavior on
the dev VM (2026-09-16). Not a crash, not a hang, and not something either
PR's diff touches — reproduced on `main` at both points in time.

## What happens

On a clean `flexwm msg action quit` under `--tty`, the process sometimes logs,
right at exit:

```
ERROR drm_atomic: smithay::backend::drm::device::atomic: Failed to restore
previous state. Error: Permission denied (os error 13)
```

This comes from the pinned Smithay rev's `impl Drop for AtomicDrmDevice`
(`backend/drm/device/atomic.rs`): if the device's own `active` flag was still
`true` when the struct is dropped, it issues one best-effort atomic commit to
put the CRTC/connectors back to whatever they were before flexwm took over —
"so that getty will be visible" again, per that impl's own comment, which
also states plainly that a failure here just means "the user will be
presented with a black screen if no display handler takes control again."
It is not propagated as an error; flexwm's own exit code and shutdown are
unaffected either way.

## Why it fails sometimes and not always

Confirmed via `journalctl -u seatd`: on the reproduction run, seatd logged
`Removed client 2 from seat0` / `Client disconnected` **before** this error
appeared in flexwm's own log — seatd had already torn down its side of the
session by the time `AtomicDrmDevice::drop` tried to commit, so the commit is
no longer master and gets `EPERM`.

What isn't confirmed: *why* seatd's teardown finishes before flexwm's own
process-exit sequence gets to the DRM device's `Drop`. `compositor::run`
(`crates/flexwm/src/compositor/mod.rs`) declares `event_loop` before `state`,
so by Rust's reverse-declaration drop order `state` (and the `Tty`/
`DrmDevice` inside it) drops *before* `event_loop` — and the actual strong
libseat connection lives in the `LibSeatSessionNotifier` registered as an
event-loop source (see `Tty::session`'s own doc on this), which only drops
with `event_loop`. So from a purely single-process, Rust-drop-order reading,
the seatd connection should still be alive when the DRM restore-commit runs.
Something closes it, or causes seatd to revoke master, earlier than that —
possibly `Libinput`'s own drop closing device fds through the same session
interface, possibly seatd revoking proactively on some other signal — not
traced further than this.

## Why this is low priority

- The failure mode Smithay's own comment describes (console left showing
  flexwm's last frame instead of getty) is cosmetic and transient: real
  hardware's console driver typically repaints on its own, and this VM's
  virtio-gpu console did too in both reproductions here — nothing was stuck.
- Not reliably reproducible on demand (it did not happen on every `--tty`
  quit tested for PR #41's own hardware bug-bash, only some), so it is not a
  regression a test suite could pin without first understanding the timing
  it depends on.
- Not attributable to any single PR's diff — the code at every site named
  above (`Tty`'s field order, `compositor::run`'s local order, and the
  Smithay dependency itself) predates both discoveries.

## What it would take

Trace the actual fd/socket lifecycle across a real `--tty` quit (e.g.
`strace -f -e trace=network,close` across the whole process, or instrumenting
`LibSeatSession`'s own drop) to find what closes the seatd connection, or
otherwise causes seatd to revoke master, ahead of `AtomicDrmDevice::drop`. If
it turns out to be flexwm's own drop order rather than something seatd
initiates independently, the fix is a field/declaration reorder so the
session's owning reference outlives the DRM device — cheap once the actual
mechanism is known, not cheap to guess at.
