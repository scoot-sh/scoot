---
title: "Desktop: display arrangement rules (kanshi-class) on the paved path"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# Desktop: display arrangement rules (kanshi-class) on the paved path

Filed 2026-10-04, child of `desktop-paved-path` (from PR #430's review).
Serves **daily-drive**: a laptop moving between a desk monitor and no monitor
needs its scale and placement to follow.

## The gap

scoot's `[[outputs]]` config sets scale and position per connector name, and
`wlr-output-management-v1` is read-only (`docs/protocols.md`), so a profile
switcher like kanshi cannot apply rules and there is no "this monitor set
means this layout" on the paved path. The M2 hand-sets DP-1 to 1x in its own
config today.

## What to do

Decide between scoot-native profiles (match a set of connected outputs by
name, make and model, then apply) and making `wlr-output-management-v1`
writable so kanshi works; prefer the standard protocol unless a concrete
reason says otherwise (CLAUDE.md), and say which. Expose it as a desktop
profile slot with eval pins.

## Acceptance

On the M2, plug and unplug DP-1 and show each profile applied (scale,
position) with `scoot msg outputs` before and after.

## Not in this ticket

A graphical display settings app.

## Resolution (PR #482, 2026-10-06)

Chose **scoot-native profiles** as a stopgap: `desktop.displays.profiles`
plus a `scoot-displays` watcher (home-manager user unit bound to
`scoot-session.target`), with eval pins and a `## Displays` site section.
Stock kanshi cannot drive scoot: its `exec` hooks run only after a
`succeeded` reply, which scoot's read-only write half never sends (proven
live: kanshi matched the profile, got `failed`, ran nothing). A writable
protocol is atomic-modeset surgery across three backends for a packaging
slot. **Its durable home is
[`output-management-reconfiguration-done.md`](./output-management-reconfiguration-done.md)**,
which is where positions, modes and kanshi interop land; this watcher is the
stopgap until then.

What profiles do, plainly: **scale and power, at hotplug.** No positions
(outputs pack left to right in connection order), no modes, and no make/model
keying (scoot's IPC reports connector names only). Two ticket premises were
corrected along the way: `[[outputs]]` carries scale and mode only, and
make/model cannot key a profile.

The watcher never writes the config file (the first round did, and review
found that under home-manager the file is a read-only store symlink: the
write either failed while reporting success or replaced the symlink and lost
to every rebuild). It applies over IPC only: a new
`scoot msg output-scale ID|NAME SCALE|reset` request (runtime state like
`output-power`: a successful reload or a restart goes back to the config's
scales) and `output-power`. It reports `applied` only when every call
succeeded and exits non-zero naming the failed call otherwise, and applies
run one at a time under `flock`. `mode` was dropped from profiles (a mode
cannot change live, and the watcher has no file to stage one in); a set
`mode` fails evaluation, pointing at `programs.scoot.settings.outputs`. A
home-manager rebuild never reloads the session, so it never drops an
applied scale; a manual reload does, and no IPC event reports it, so the
docs say to run `scoot-displays apply` after one.

Software proof is headless on the M2 (see the PR). Physical DP-1
plug/unplug steps are in the PR body for the maintainer (no scoot session
drives the real outputs right now).
