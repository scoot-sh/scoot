---
title: "Repin Smithay from the scoot-sh fork back to upstream once the syncobj Drop lands"
status: "open"
area: "core"
priority: "low"
blocked: "needs an upstream Smithay rev carrying a Drop for the imported syncobj timeline; nothing is filed upstream from this project (the maintainer decides later, see docs/forks.md)"
---

# Repin Smithay from the scoot-sh fork back to upstream

Filed 2026-09-24 by PR #233. `crates/scoot/Cargo.toml` pins Smithay to
`github.com/scoot-sh/smithay` rev `43f50eb2`. That is upstream `0ff00983`
plus exactly one commit, a `Drop` for `DrmTimelineDeviceSpecific`, which
stops each syncobj timeline import leaking a kernel handle
([resolved record](../resolved/syncobj-handle-leak-done.md)).

The fork is debt:

- every Smithay bump now needs the commit rebased onto the new rev;
- CLAUDE.md's "verify against the pinned source" means the fork rev;
- `flake.nix`'s `outputHashes` entry is the fork tree's hash.

## What to do

Once an upstream Smithay rev carries the fix (the user is filing it):

1. Pin `crates/scoot/Cargo.toml` back to `github.com/Smithay/smithay` at a
   rev that has it. Take whatever other upstream changes that rev brings
   through the usual bump review, since `dispatch.rs`'s maintenance note
   applies.
2. Update `Cargo.lock` (smithay's source line only, if the rev allows) and
   the flake's `outputHashes`.
3. Re-run the leak measurement (`~/evidence/sync/run-g*.sh` on the dev VM:
   the import-and-destroy loop and the abandoned-wait loop) and confirm
   both stay flat.
4. Drop the fork mentions from `crates/scoot/Cargo.toml`, `flake.nix`,
   CLAUDE.md, `docs/protocols.md` and `drm_syncobj.rs`'s module doc.

If upstream fixes it differently (for example `Drop` on `DrmTimelineInner`),
check that `update_device` and `invalidate` still cannot double-destroy.
