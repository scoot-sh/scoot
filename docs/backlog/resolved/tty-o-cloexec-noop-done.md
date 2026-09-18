---
title: "`--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the libseat layer — RESOLVED (flag removed; spawn non-inheritance pinned)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the libseat layer — RESOLVED (flag removed; spawn non-inheritance pinned).

## The entry as filed

`docs/backlog/tty/tty-o-cloexec-noop.md` (LOW, informational):

> `--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the
> libseat layer (LOW, informational). `tty/mod.rs` requests
> `OFlags::CLOEXEC` when opening the DRM device, but the pinned Smithay's
> `LibSeatSession::open` discards the flags parameter entirely and just
> calls `libseat::Seat::open_device` — so whether an `Action::Spawn`-launched
> child inherits DRM master or input-device fds depends entirely on
> libseat's own C-side behavior, not on flexwm's request. Measured
> 2026-09-13 with exactly the check this entry suggested, while an
> SSH-started `--tty` ran on the dev VM at `0678765`: every one of flexwm's
> seatd-obtained fds (`/dev/dri/card0` and all four `/dev/input/event*`)
> reports `flags: 02504002`, which has the `02000000` `O_CLOEXEC` bit set. So
> the outcome flexwm asked for does hold (an `Action::Spawn`ed child inherits
> neither DRM master nor input fds), just for a different reason than
> flexwm's own argument. Correction, caught by `flexwm-reviewer`: close-on-
> exec is a property of the *receiving process's own fd table*, not of the
> open file, and `SCM_RIGHTS` (how the fd crosses the seatd-flexwm socket)
> never transfers it, so "identical to seatd's fd, since it's the same open
> file" is the wrong reason for this one bit -- the rest of `02504002`
> genuinely is shared `f_flags` from that open file
> (`O_RDWR|O_NONBLOCK|O_NOFOLLOW|O_LARGEFILE`), just not this one. The actual
> guarantor is libseat's own receive call:
> `recvmsg(..., MSG_DONTWAIT | MSG_CMSG_CLOEXEC)`
> (`libseat/common/connection.c:202`) sets close-on-exec the moment libseat
> receives the fd into flexwm's own process, unconditionally, for every fd it
> hands over. A real, deliberate contract -- just libseat's, not flexwm's
> `OFlags::CLOEXEC` argument, which remains the no-op this entry originally
> found. Still informational, still no code change: if a future libseat
> version ever dropped `MSG_CMSG_CLOEXEC`, every `Action::Spawn`ed child
> would silently inherit DRM master and every input device fd, and nothing
> in flexwm's own code would catch or even notice it.

## Resolution (2026-09-18, PR #114)

Remove + pin, exactly the shape the entry's last paragraph asked for: the
dead flag is gone, and the spawn side of the chain is now pinned; the
seatd-fd premise itself remains measurement-backed (see Residuals).

### Verify-first: what the pinned rev actually says

All re-verified in source at `0ff0098`, not relayed from the entry:

- Smithay's `LibSeatSession::open` takes `_flags: OFlags` and never reads
  it; the body forwards only the path to `seat.open_device(&path)`
  (`src/backend/session/libseat.rs:118`). The flag flexwm passed
  (by now in `tty/gpu.rs`'s `open`, moved out of `tty/mod.rs` by the DRM
  device-selection work) was a dead argument.
- `State::spawn` is the only compositor spawn path -- the only
  `Command::new` in compositor code is `state.rs:921` -- and Rust std's
  spawn inherits every fd *without* close-on-exec. Proven, not assumed:
  the new pin test's plain marker arrives in the child. So the
  close-on-exec bit is load-bearing, not belt-and-braces, and the entry's
  "would silently inherit" warning was exactly right about the mechanism.

### What landed

- `tty/gpu.rs`: `OFlags::RDWR | OFlags::CLOEXEC` becomes `OFlags::RDWR`,
  with a comment naming the real guarantor (libseat's receive path, plus
  the 2026-09-13 live measurement) and the pin test. `RDWR` stays: the
  call needs flags, and it states the access intent.
- New test `a_spawned_child_inherits_no_close_on_exec_fd`
  (`activation/tests/spawn.rs`): the real `State::spawn` runs a real `sh`
  child that lists `/proc/self/fd`. A close-on-exec marker (the seatd-fd
  shape, opened via `libc` since std sets the bit unconditionally) must be
  absent; a second marker deliberately without the bit must be PRESENT --
  the permanent positive control proving the probe observes real
  inheritance; fd 1 (the child's own redirected stdout) must be present as
  the vacuity guard. Premise asserts check each marker carries exactly the
  expected flag state (fail loud, not weak), and a trailing liveness check
  keeps both markers open across the spawn (without that last use they
  could drop, closing the fds, before the child even starts).
- Fail-first: first written asserting *both* markers absent, it FAILED
  exactly on the plain marker -- the child's table `[0, 1, 13, 2, 3]` held
  it -- which is both the sensitivity record and why the control asserts
  presence permanently.

### Residuals, stated plainly

- `State::spawn` inherits ANY non-close-on-exec fd, so hygiene rests on
  every fd source setting the bit. Seatd's are proven, std/Smithay-created
  ones are close-on-exec-by-construction (`SOCK_CLOEXEC` per `ipc.rs`,
  `MemfdFlags::CLOEXEC` at every creation site); the systematic per-source
  audit is filed separately as [a low-priority security
  item — since completed, see
  [the audit record](./spawn-fd-cloexec-audit-done.md) — not sprawled into
  this one-flag change.
- No live `--tty` re-verification: behavior-identical change (a dead
  argument removed), and the seatd-fd premise reuses the entry's 2026-09-13
  live measurement as cache per `CLAUDE.md`. Full standard set green on
  the dev VM (exact commands and raw outputs in the PR).
- No README change: no user-facing surface (an internal flag removal plus
  a test).
