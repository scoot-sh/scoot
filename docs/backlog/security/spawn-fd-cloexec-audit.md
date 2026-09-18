---
title: "Audit that every fd the compositor holds carries close-on-exec."
status: "open"
area: "security"
priority: "low"
blocked: null
---

# Audit that every fd the compositor holds carries close-on-exec.

Split out of [the `O_CLOEXEC` no-op
verdict](../resolved/tty-o-cloexec-noop-done.md) (2026-09-18), which removed
a dead flag and pinned the guarantee the flag appeared to give. What that
pin proved along the way -- and what stays open here -- is that
`State::spawn` (every keybinding and IPC `spawn`, and the only
`Command::new` in compositor code) inherits *any* fd without close-on-exec:
the pin test's plain marker arrives in the child, kept as the test's
permanent positive control. So spawn-time fd hygiene rests entirely on
every fd source setting the bit, and no single call site can cover a source
that doesn't.

Known-good, needing no re-verification: seatd-obtained DRM and input fds
(live-measured 2026-09-13, guarantor libseat's `MSG_CMSG_CLOEXEC` receive);
sockets Smithay and std create (`SOCK_CLOEXEC`, see `ipc.rs`'s accept
comment); memfds (`MemfdFlags::CLOEXEC` at every creation site, production
and test).

What the audit needs is the rest of the inventory, verified at creation --
atomically, not via a later `fcntl` (which races threads between open and
flag-set): the event-loop and notifier fds, the screenshot worker's
channels, anything the DRM/gamma paths hold open, and any file handle with
a lifetime reaching a spawn (config, cursor theme). Each source either sets
the bit at creation (fix the straggler there) or gets a reason recorded why
it cannot leak. Extend the spawn pin or add source-level pins for whatever
moves.

Low because every known source already sets the bit and the failure mode
needs a source that doesn't; revisit if a new fd source lands without one.
