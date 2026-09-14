---
title: "`--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the libseat layer (LOW, informational)."
status: "open"
area: "tty"
priority: "info"
blocked: null
---

# `--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the libseat layer (LOW, informational).

`--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the
libseat layer (LOW, informational). `tty/mod.rs` requests
`OFlags::CLOEXEC` when opening the DRM device, but the pinned Smithay's
`LibSeatSession::open` discards the flags parameter entirely and just
calls `libseat::Seat::open_device` — so whether an `Action::Spawn`-launched
child inherits DRM master or input-device fds depends entirely on
libseat's own C-side behavior, not on flexwm's request. Measured
2026-09-13 with exactly the check this entry suggested, while an
SSH-started `--tty` ran on the dev VM at `0678765`: every one of flexwm's
seatd-obtained fds (`/dev/dri/card0` and all four `/dev/input/event*`)
reports `flags: 02504002`, which has the `02000000` `O_CLOEXEC` bit set. So
the outcome flexwm asked for does hold (an `Action::Spawn`ed child inherits
neither DRM master nor input fds), just for a different reason than
flexwm's own argument. Correction, caught by `flexwm-reviewer`: close-on-
exec is a property of the *receiving process's own fd table*, not of the
open file, and `SCM_RIGHTS` (how the fd crosses the seatd-flexwm socket)
never transfers it, so "identical to seatd's fd, since it's the same open
file" is the wrong reason for this one bit -- the rest of `02504002`
genuinely is shared `f_flags` from that open file
(`O_RDWR|O_NONBLOCK|O_NOFOLLOW|O_LARGEFILE`), just not this one. The actual
guarantor is libseat's own receive call:
`recvmsg(..., MSG_DONTWAIT | MSG_CMSG_CLOEXEC)`
(`libseat/common/connection.c:202`) sets close-on-exec the moment libseat
receives the fd into flexwm's own process, unconditionally, for every fd it
hands over. A real, deliberate contract -- just libseat's, not flexwm's
`OFlags::CLOEXEC` argument, which remains the no-op this entry originally
found. Still informational, still no code change: if a future libseat
version ever dropped `MSG_CMSG_CLOEXEC`, every `Action::Spawn`ed child
would silently inherit DRM master and every input device fd, and nothing
in flexwm's own code would catch or even notice it.
