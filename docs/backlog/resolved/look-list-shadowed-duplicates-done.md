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

## Resolution

Resolved 2026-10-29 (PR #799). The GUI/ISO listing itself was not
reproduced; the cause was found in the modules and reproduced with
`nix-instantiate` on the dev VM. The NixOS half links
`ln -s ${entry.src}`, which copies the source to `/nix/store/<hash>-<name>`.
The home-manager half set `source = entry.src` (a path value), and
home-manager's `sourceStorePath` then copies it again with `builtins.path`
under the name `hm_<name>`: a different store path with identical bytes.
The catalog saw two sources of one name and listed the second as shadowed.

Two changes. `home.nix` now interpolates the source (`"${entry.src}"`), so
it carries string context, home-manager links it as is, and both halves
link one store path (pinned in `nix/tests.nix`; reverting `home.nix` fails
that check). `scan_dir` then drops a later entry whose manifest directory
equals the winner's, since one look reached from two catalog directories is
not a shadow (the per-directory dedup cannot see it). Two different
sources of one name still shadow, even with identical bytes. Rust tests are
in `crates/scoot-look/src/catalog/tests.rs`.
