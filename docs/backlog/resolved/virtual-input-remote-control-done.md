---
title: "Virtual pointer and keyboard protocols, so wayvnc gives full remote control"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Virtual pointer and keyboard protocols, so wayvnc gives full remote control

Filed 2026-10-04 on the maintainer's ask ("Does scoot support vnc, moonlight,
or other Remote Desktop methods" — yes, file it). Serves **daily-drive**
(reaching your own desktop from another machine) and **computer use** (a
standard remote-control path beside scoot's own IPC injection).

## The gap

scoot captures outputs over `ext-image-copy-capture-v1`
(`docs/protocols.md`), so a VNC server can see the screen, but it offers
neither `zwlr_virtual_pointer_manager_v1` nor
`zwp_virtual_keyboard_manager_v1`, so wayvnc cannot move the pointer or type.
Remote access today is view-only (the portal's ScreenCast) or the webtop
browser path. Nothing in the backlog covered it.

## What to do

- Implement both managers (check what the pinned Smithay fork already
  provides first; a `dev/forks.md` entry only as the last resort, per
  CLAUDE.md). Virtual keyboards bring their own keymap: handle a keymap that
  differs from the seat's without corrupting the physical keyboard's state.
- Security first, decided and written in `docs/protocols.md`: any client
  that binds these can type and click as the user. Gate them the way scoot
  already gates privileged globals (data-control, session lock): who may
  bind (e.g. off unless a config key enables it, or a per-binary allow
  list), and refuse all virtual input while the session is locked except
  what a lock client itself is owed. Never let a remote client unlock.
- Verify, not assume, which wayvnc version speaks `ext-image-copy-capture`
  (and not only `wlr-screencopy`, which scoot deliberately does not offer).
  If none does, say so: this ticket then needs a decision on capture too.
- Edge cases: injection at the real maximum rate (a remote drag), pointer
  across several outputs (absolute coordinates spanning the layout), a
  client disconnecting with keys held (release them), and a virtual device
  during VT switch-away.

## Acceptance

wayvnc against a `--headless` and a `--tty` scoot (the M2), driven from a
VNC client on another machine: pointer, clicks, scroll, typing with a
non-US layout, and a lock screen that refuses the remote client. Benchmark
the injection path (no allocation per event). Docs: `docs/protocols.md`
rows and a "Remote desktop" section, a config key in
`docs/configuration.md` if gated by one.

## Not in this ticket

RDP and the portal's RemoteDesktop interface (no wlroots-family portal
backend provides it); Sunshine/Moonlight (`moonlight-sunshine-on-m2`).

## Resolution (2026-10-05, PR #448)

Both managers implemented in `crates/scoot/src/compositor/virtual_input.rs`
(no Smithay fork change: the keyboard uses `KeyboardHandle::input_from_source`
with per-device auxiliary sources, the pointer is hand-rolled against the
re-exported `wayland-protocols-wlr` server bindings).

- Gating: `[virtual_input] enabled` (default off; globals unadvertised when
  off, restart-only with reload refusal). An allow-list was rejected as
  theatre per `docs/protocols.md`'s trust note; the threat answered is the
  network (wayvnc listens on TCP), not same-uid processes.
- wayvnc 0.10.1 verified required: without the managers it fails hard
  (`Virtual Pointer protocol not supported`), with them it starts clean;
  capture over `ext-image-copy-capture-v1` (in wayvnc since v0.9.0) already
  worked, including damage on window spawn.
- End to end (dev VM, `--headless`): VNC pointer/click/scroll/type into a
  client (byte-exact keycodes), German `z` through `wayvnc -k de` arriving
  as seat-`z`, absolute motion onto the named output across two outputs
  (cursor-bbox proof), lock screen refusing all virtual input (zero events,
  zero damage, VNC shows only the lock color), idle 0 ticks/10s both sides,
  ~29k keys/s sustained flood with zero kills.
- Tests: 26 protocol tests (real clients; lock-surface positive controls;
  failing-first proven on the lock gate), config + reload tests, paced
  flood benches (motion 27.5µs, key 24.8µs per event end to end, debug
  build). Full workspace nextest + clippy + fmt + smoke green; CI green on
  the head.
- Not done here: `--tty` on the M2 (seat coordination pending), RDP/
  RemoteDesktop/Sunshine (out of scope per the ticket).
