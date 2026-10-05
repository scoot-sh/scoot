---
title: "Virtual pointer and keyboard protocols, so wayvnc gives full remote control"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
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
  provides first; a `docs/forks.md` entry only as the last resort, per
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
