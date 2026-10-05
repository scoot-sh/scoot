---
title: "Research: native remote desktop that streams the scene, not pixels"
status: "research"
area: "protocols"
priority: "research"
blocked: null
---

# Research: native remote desktop that streams the scene, not pixels

Filed 2026-10-04 on the maintainer's ask: "if we wanted the fastest, lowest
resource method, would it be one we built?" Serves **daily-drive** and
**computer use** (an agent viewing and driving a remote session at almost
no cost). Research: no build until the design and a prototype's numbers say
it beats `virtual-input-remote-control` (wayvnc) by enough to carry its
own viewer.

## Why scoot could beat VNC, RDP and Sunshine

Every general remote desktop ships *composited pixels*: VNC and RDP send
damaged rectangles of the final frame, Sunshine encodes the whole frame as
video at a fixed rate. scoot knows more than the final frame: each surface's
buffer and damage, and an arrangement computed by `scoot-core` (events in,
arrangement out). A scrolling compositor is the worst case for pixel
remoting: one column scroll repaints the whole screen every animation frame,
so VNC re-sends and Sunshine re-encodes every pixel, though no window's
content changed.

## The shape to evaluate

- **Scene mode.** Send each surface's content once and then only its
  damage, plus the arrangement (positions, sizes, scroll offset, focus,
  decorations' state) as small messages. The viewer (`scootview`)
  composites locally with scoot's own renderers (pixman or GLES). A scroll,
  a window move or a workspace switch costs bytes, not frames; idle costs
  zero bytes and zero wakeups.
- **Frame mode,** as the fallback for thin viewers (a browser over WebRTC,
  a phone) and for surfaces where scene mode loses: a surface tagged video
  or game (scoot already accepts `wp-content-type-v1`) or one damaging most
  of itself at display rate is encoded as video (hardware where present,
  zero copy from the GPU tier's dmabuf; the M2 has no Asahi video encoder
  today, so CPU there), the rest stays scene.
- **Process split.** The compositor stays small and off the network. It
  hands buffers and damage (fds, not copies) to a separate `scootremote`
  helper over a privileged local channel, like scootbg is separate; codecs,
  crypto and sockets live in the helper, so a helper crash never takes the
  session. Off by default and free when off (no new wakeups, no allocation).
- **Input** enters scoot's own input path as a seat, the way IPC injection
  does (`docs/backlog/ipc/targeted-input-injection.md`), with no virtual-keyboard
  keymap round trip. Clipboard rides the channel under the same rules as
  data-control.
- **Security.** Authenticated transport (SSH tunnel or key-pinned TLS/Noise),
  the lock screen is what a remote viewer sees while locked, and nothing
  remote can unlock.

## What the research must produce

Measured, on the M2 and the dev VM, against wayvnc (once
`virtual-input-remote-control` lands) and Sunshine
(`moonlight-sunshine-on-m2`): bytes, CPU on both ends and latency for idle,
typing in foot, scrolling a column strip, a window move, a browser page
scroll and a playing video. Check prior art (waypipe forwards one client's
protocol; RDP's RemoteApp; X11 forwarding) for what scene mode must avoid,
and whether a standard covers any part (CLAUDE.md: prefer one where it
exists). Then a recommendation: build, build scene mode only for scoot
viewers, or stay with wayvnc.

## Not in this ticket

Any implementation; a viewer for macOS or the web (a later decision).
