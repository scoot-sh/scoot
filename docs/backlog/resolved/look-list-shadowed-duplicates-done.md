---
title: "scoot look list prints every look twice (shadowed duplicates)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-29"
---

# scoot look list prints every look twice (shadowed duplicates)

Filed 2026-10-17. Serves **daily-drive** (CLI output noise; no behavior
impact found — looks apply and live-switch correctly).

## The gap

On a GUI-installed system (scoot-iso PR #6 ISO, aarch64/HVF walkthrough),
`scoot look list` prints each of the 5 looks twice: once
`(declared, dark/light)` and once `shadowed by an earlier catalog entry
(/nix/store/...)`. Raw output in the T-iso-aarch64 task report.
Presumably the system half and the home-manager half both register the
catalog. Harmless (apply/next work), but it reads like an error and
obscures real shadow warnings.

## What to do

Register the catalog once (or silence the duplicate when both halves
agree byte-for-byte), keeping a genuine shadow (two different sources)
loud. Pin with a test that asserts the list output for a system+home
setup identical to the installed target.

## Not in this ticket

Any change to look resolution order or the catalog format.
