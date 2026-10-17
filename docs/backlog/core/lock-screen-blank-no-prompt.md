---
title: "Idle lock shows a blank white screen with no prompt"
status: "open"
area: "core"
priority: "medium"
blocked: null
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
