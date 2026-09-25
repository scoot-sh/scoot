---
title: "Repin Smithay from the scoot-sh fork back to upstream once its fixes land"
status: "open"
area: "core"
priority: "low"
blocked: "needs an upstream Smithay rev carrying a Drop for the imported syncobj timeline and equivalents of the seven XWayland selection/drag commits; nothing is filed upstream from this project (the maintainer decides later, see docs/forks.md)"
---

# Repin Smithay from the scoot-sh fork back to upstream

Filed 2026-09-24 by PR #233. `crates/scoot/Cargo.toml` pins Smithay to
`github.com/scoot-sh/smithay` rev `0d281abf` (branch
`scoot/xwayland-selection-dnd`, since XWayland Phase 4; `43f50eb2` before).
That is upstream `0ff00983` plus eight commits: a `Drop` for
`DrmTimelineDeviceSpecific`, which stops each syncobj timeline import
leaking a kernel handle
([resolved record](../resolved/syncobj-handle-leak-done.md)), and seven
XWayland selection and drag fixes and hooks, each listed with its
measurement in [`docs/forks.md`](../../forks.md).

The fork is debt:

- every Smithay bump now needs the commit rebased onto the new rev;
- CLAUDE.md's "verify against the pinned source" means the fork rev;
- `flake.nix`'s `outputHashes` entry is the fork tree's hash.

## What to do

Once an upstream Smithay rev carries the fixes (nothing is filed upstream from this project; see `docs/forks.md`) -- all eight, or a subset with the rest re-carried on a fresh fork branch:

1. Pin `crates/scoot/Cargo.toml` back to `github.com/Smithay/smithay` at a
   rev that has it. Take whatever other upstream changes that rev brings
   through the usual bump review, since `dispatch.rs`'s maintenance note
   applies.
2. Update `Cargo.lock` (smithay's source line only, if the rev allows) and
   the flake's `outputHashes`.
3. Re-run the leak measurement (`~/evidence/sync/run-g*.sh` on the dev VM:
   the import-and-destroy loop and the abandoned-wait loop) and confirm
   both stay flat.
4. For the XWayland commits: `cargo nextest run -p scoot --features
   xwayland` with `SCOOT_REQUIRE_XWAYLAND=1` -- the clipboard and drag
   suites were written against each bug and fail without its fix -- and
   port `X11Wm::selection_owner` / `XwmHandler::allow_drag` callers to
   whatever upstream names them.
5. Drop the fork mentions from `crates/scoot/Cargo.toml`, `flake.nix`,
   CLAUDE.md, `docs/protocols.md` and `drm_syncobj.rs`'s module doc.

If upstream fixes it differently (for example `Drop` on `DrmTimelineInner`),
check that `update_device` and `invalidate` still cannot double-destroy.
