---
title: "`--tty` quit sometimes logs a DRM \"restore previous state\" EPERM — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--tty` quit sometimes logs a DRM "restore previous state" `EPERM` — RESOLVED.

## The entry as filed

`docs/backlog/tty/drm-teardown-restore-eperm.md` (LOW):

> Found independently twice: once during a reviewer's live `--tty` spot-check
> of PR #41, once reproduced deliberately while testing VT-switch behavior on
> the dev VM (2026-09-16). Not a crash, not a hang, and not something either
> PR's diff touches — reproduced on `main` at both points in time.
>
> On a clean `flexwm msg action quit` under `--tty`, the process sometimes logs,
> right at exit:
>
> ```
> ERROR drm_atomic: smithay::backend::drm::device::atomic: Failed to restore
> previous state. Error: Permission denied (os error 13)
> ```
>
> This comes from the pinned Smithay rev's `impl Drop for AtomicDrmDevice`
> (`backend/drm/device/atomic.rs`): if the device's own `active` flag was still
> `true` when the struct is dropped, it issues one best-effort atomic commit to
> put the CRTC/connectors back to whatever they were before flexwm took over —
> "so that getty will be visible" again, per that impl's own comment, which
> also states plainly that a failure here just means "the user will be
> presented with a black screen if no display handler takes control again."
> It is not propagated as an error; flexwm's own exit code and shutdown are
> unaffected either way.
>
> Confirmed via `journalctl -u seatd`: on the reproduction run, seatd logged
> `Removed client 2 from seat0` / `Client disconnected` **before** this error
> appeared in flexwm's own log — seatd had already torn down its side of the
> session by the time `AtomicDrmDevice::drop` tried to commit, so the commit is
> no longer master and gets `EPERM`.
>
> What isn't confirmed: *why* seatd's teardown finishes before flexwm's own
> process-exit sequence gets to the DRM device's `Drop`. `compositor::run`
> (`crates/flexwm/src/compositor/mod.rs`) declares `event_loop` before `state`,
> so by Rust's reverse-declaration drop order `state` (and the `Tty`/
> `DrmDevice` inside it) drops *before* `event_loop` — and the actual strong
> libseat connection lives in the `LibSeatSessionNotifier` registered as an
> event-loop source (see `Tty::session`'s own doc on this), which only drops
> with `event_loop`. So from a purely single-process, Rust-drop-order reading,
> the seatd connection should still be alive when the DRM restore-commit runs.
> Something closes it, or causes seatd to revoke master, earlier than that —
> possibly `Libinput`'s own drop closing device fds through the same session
> interface, possibly seatd revoking proactively on some other signal — not
> traced further than this.
>
> Why this is low priority:
>
> - The failure mode Smithay's own comment describes (console left showing
>   flexwm's last frame instead of getty) is cosmetic and transient: real
>   hardware's console driver typically repaints on its own, and this VM's
>   virtio-gpu console did too in both reproductions here — nothing was stuck.
> - Not reliably reproducible on demand (it did not happen on every `--tty`
>   quit tested for PR #41's own hardware bug-bash, only some), so it is not a
>   regression a test suite could pin without first understanding the timing
>   it depends on.
> - Not attributable to any single PR's diff — the code at every site named
>   above (`Tty`'s field order, `compositor::run`'s local order, and the
>   Smithay dependency itself) predates both discoveries.
>
> What it would take:
>
> Trace the actual fd/socket lifecycle across a real `--tty` quit (e.g.
> `strace -f -e trace=network,close` across the whole process, or instrumenting
> `LibSeatSession`'s own drop) to find what closes the seatd connection, or
> otherwise causes seatd to revoke master, ahead of `AtomicDrmDevice::drop`. If
> it turns out to be flexwm's own drop order rather than something seatd
> initiates independently, the fix is a field/declaration reorder so the
> session's owning reference outlives the DRM device — cheap once the actual
> mechanism is known, not cheap to guess at.

## Resolution (2026-09-18)

Traced, root-caused, and fixed as the ticket's "what it would take" asked.
`Tty` now has an explicit `Drop` impl (`compositor/tty/mod.rs`) that calls
`self.drm.pause()` before any of its pieces drop, so Smithay's
restore-on-drop never fires and shutdown is deterministically quiet.

### The mechanism (strace-proven, not reasoned)

The ticket's drop-order puzzle resolves once one Smithay fact is in view:
`DrmDevice.internal` is an `Arc<DrmDeviceInternal>`, cloned three ways at
steady state — into `Tty.drm`, into `Tty.surface` (via `create_surface`),
and into the `DrmDeviceNotifier` that `tty::init` registers with the event
loop. So the restore-on-drop does **not** run when `Tty` drops; it runs
when the *last* clone drops, which is the notifier's, during event-loop
teardown — after the libseat notifier in that same loop has dropped and
closed the seatd socket. No local/field reorder in flexwm can fix that: any
order still drops the loop's notifier clone last.

`strace -f -y -e trace=network,close,ioctl` across a whole `--tty` quit
shows the exact sequence in our own process: IPC `quit` reply → final
`DRM_IOCTL_MODE_ATOMIC = 0` → dumb-buffer destruction (`DESTROY_DUMB` /
`RMFB`, i.e. `BufferPool` dropping with `Tty`) → one libseat
`CLOSE_DEVICE` handshake → `close()` of the seatd socket → *then*
`DRM_IOCTL_MODE_ATOMIC = -1 EACCES (Permission denied)` (the restore) →
`close()` of `/dev/dri/card0`. seatd revokes master on disconnect, so the
restore races seatd's async disconnect handling of our own socket close —
which is why it was "sometimes": whichever side wins the race decides
whether the `ERROR` line appears. (Two corrections to the ticket's
shorthand, both from the trace: the errno is `EACCES`, not `EPERM` —
non-master callers fail atomic commits with `EACCES`, os error 13 reads
"Permission denied" either way — and the cross-log "seatd first" ordering
compared two different clocks and proves nothing on its own; the strace
does.)

### The fix

`DrmDevice::pause()` is Smithay's supported "don't touch the fd on drop"
(its own doc: "This will cause the `DrmDevice` to avoid making calls to the
file descriptor e.g. on drop" — here just `set_active(false)` plus surface
bookkeeping, since this backend runs unprivileged and issues no master
ioctls). Pausing in `Tty::drop` makes the skip deterministic rather than
raced, and the hardware outcome is byte-for-byte the already field-observed
failure case: the last frame stays scanned out until the VT switch on
session close repaints, which the dev VM's console does on its own. The
alternative — preserving the sometimes-working getty restore — is
unreachable without a synchronous way to kill the loop's notifier clone
before the seat closes (calloop removals only take effect on a dispatch
that never comes post-`run`), so it would mean keeping the race, not fixing
it. Idempotent with the `PauseSession` arm's own `drm.pause()` (quit while
paused was already quiet), and panic-safe (no allocation, no ioctls,
nothing fallible — the same severity bar as data loss, applied to a Drop
that runs on every exit path including unwinds).

Fail-first: pre-fix, 6/6 clean `--tty` quits logged the restore error
(`/tmp/eperm-repro-{1..6}.log` on the dev VM, 10:30 UTC); post-fix, 6/6
quit with zero hits on the identical harness. The harness cannot construct
a `Tty` (no DRM device outside `--tty` — same stated limit as the
cursor-paused record), so there is no unit test for a three-line Drop impl;
the pin is the live pre/post pair plus the code-level guarantee (once
`Tty::drop` pauses, no restore can ever fire — nothing can set the flag
again without `&mut Tty`).

### Bug bash (dev VM, `--tty`, fixed tree)

- Quit while VT-paused (`sudo chvt 2`, IPC `quit`, `sudo chvt 1` back):
  exit 0, no restore line, foreground VT restored to tty1. (Pre-fix this
  shape was already quiet via the pause arm — no-regression check.)
- Full pause → activate cycle then quit: `session activated` + second
  `drm: modeset`, post-reactivate screenshot byte-identical to pre-pause
  (19665 bytes both) — the reactivation repaint is untouched by a Drop impl
  that only runs at teardown.
- Log sweep across all post-fix quit logs: only the known-benign
  `Failed to destroy old mode property blob` startup WARN (called out as
  benign in the cursor-paused record); shutdown ends at `Dropping device`
  with no restore attempt. No panics, no new ERRORs.

No hot-path impact (a `Drop` that runs once per process exit) — no
benchmark, stated plainly. No README change: a log line disappearing is not
a config option, keybinding, CLI flag, or IPC surface.

### Not verified live (stated plainly)

- Getty repaint timing on real hardware after a clean quit (the restore that
  used to do it instantly is now skipped by design; recovery relies on the
  VT switch on session close, as the already-observed failure case did).
  Dev-VM console recovers on its own in every run above.
- The `EPERM`-while-operating shapes the ticket's prompt mentions
  (VT-switched-away flips, foreign master): audited, not reproduced —
  `present()` gates on `active`, `reactivate()` degrades to
  paused/no-master with the keyboard path kept alive, and gamma failures
  surface as protocol `failed` events, so the Drop-time restore was the only
  unhandled EPERM-shaped path.
