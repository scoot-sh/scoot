---
title: "scoot look install from a path, export and save, behind the trust boundary"
status: "open"
area: "core"
priority: "low"
blocked: "look-authoring"
---

# scoot look install from a path, export and save, behind the trust boundary

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**. Low: the security-sensitive part, after the basics.

## The gap

There is no way to take a look from someone else or to hand one over, and
anything arriving from elsewhere crosses the trust boundary of the design
doc's section 8.3.

## What to do

- `scoot look install PATH [--trust]`: copies regular files only (a symlink,
  device, FIFO or socket refuses the install), every referenced path resolved
  inside the look directory, file count and total size capped, `license`
  required, a shipped or declared name refused, `apps/` static leaves dropped
  unless `--trust` (printed first).
- `scoot look export NAME`: a tarball of a source look, regular files only.
- `scoot look save NAME`: the user's own appearance and color values as a
  source look.

## Evidence to record

A test per refusal (symlink, `..`, absolute path, a shipped name, missing
license, an oversized image, `apps/` without `--trust`).

## Not in this ticket

Git URLs and `lookPacks` (`look-share`, later).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
