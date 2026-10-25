---
title: "Idle lock shows a blank white screen with no prompt"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-24"
---

# Idle lock shows a blank white screen with no prompt

Filed 2026-10-17. Serves **daily-drive** (a user returning to a locked
machine sees no indication it is locked or what to do).

## The gap

On a GUI-installed system (scoot-iso PR #6 ISO, aarch64/HVF walkthrough),
leaving the session idle past the 4-minute lock shows a plain white
fullscreen frame with no prompt, indicator, or instructions. Typing the
password blind + Enter unlocks, so the lock itself works. The generated
swaylock config says the background should be dark navy (`color=2B3648`
in `~/.config/swaylock/config`, written by
`programs.scoot.desktop.idle.lock`), so white is unexpected. After 5
idle minutes the output powers off (expected); waking returns to the
same white lock frame. Evidence: QMP screendumps `gui-57/58/60-62`
(white) and `gui-62-wake` (white after wake), all 1280x800, in the
T-iso-aarch64 task report; the compositor stays alive throughout
(`scoot msg version` answers, clients keep running).

## What to do

Find what renders the white frame (locker config vs. output-power
transition vs. screendump artifact) and make the lock screen show the
configured background plus a visible password prompt. Pin: idle
dim (2m), lock (4m), off (5m) each still fire; unlock by password and
by any other configured method still works; live media keeps never
locking.

## Not in this ticket

The idle timeout values themselves, and the installer-launch failure
found in the same walkthrough (fixed in scoot-iso PR #7).

## Resolution

The white frame was the locker's own default, not the compositor's:
swaylock paints `0xFFFFFFFF` (white) when no background is configured
(`set_default_colors` in its source; the man page says "instead of
white"), while scoot itself paints only black behind a live lock and
red behind an abandoned one -- so a white fullscreen frame is
necessarily lock-client pixels with no effective background. Any
generated config without a `color=` line locks white, and the lookless
config carried none at all. Fixed by always emitting a background (the
look's color, or a navy fallback), and the default lock now shows the
look's wallpaper blurred (pre-blurred at build time: this compositor
publishes no `wlr-screencopy`, so a live screenshot has nothing to
capture against it -- see dev/design/lock-blur.md), with the indicator
always visible and an unlock message over every output. The exact
themed-config-yet-white pixel path from the walkthrough (a one-day-old
locker wrapper at the time) is no longer reachable: no emitted config
depends on the client's white default.
