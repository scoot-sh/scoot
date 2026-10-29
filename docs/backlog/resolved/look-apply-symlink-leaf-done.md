---
title: "apply follows an in-dir symlink for a leaf while build refuses all symlinks"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-29"
---

# apply follows an in-dir symlink for a leaf while build refuses all symlinks

Filed 2026-10-16, from the `look-leaves-terminal` review (PR #639).
Serves **daily-drive** (a look applies the same leaves whether it
arrives via `scoot-look build` or `scoot look apply`).

## The gap

`scoot-look build` refuses all symlinks (`main.rs` rejects them), but
`apply`'s leaf read (`contained_file` in `crates/scootctl/src/look/apply.rs`)
canonicalizes an in-directory symlink and reads through it — only
escapes outside the dir refuse. Same trust domain and the content is
still shape-checked, so this is a consistency nit, not a hole. Not
fixed in #639 to keep that PR additive-only.

## What to do

Decide one rule and enforce it in both places: either `apply`
refuses in-dir symlinks like `build` does, or `build` documents why
it is stricter (build-time sources versus already-installed dirs).
Pin with a test that applies/builds a look carrying a symlinked leaf
in both tools and asserts the same outcome.

## Not in this ticket

Symlink escapes (already refused), trust semantics (`--trust`
per-apply, documented in `site/src/content/docs/looks/index.md`).

## Resolution

The rule is the stricter one, enforced in both tools: no component of a
static leaf's path below the look directory may be a symlink (to a file or
directory inside the look, outside it, dangling or looping). The look
directory itself may be a link, which is how home-manager installs a look.

- `scoot_look::locate_leaf` (`crates/scoot-look/src/leaf_path.rs`) is the
  one walk: `symlink_metadata` per component, absent is `None`, a symlink
  or an unexpected stat error (permissions) is a loud refusal.
- `scoot-look build` and `scoot look apply` (and `validate`, which shares
  apply's path) both locate through it. Apply then reads `O_NOFOLLOW`
  (`read_capped`), so a link raced in after the walk fails with `ELOOP`.
  Before, apply canonicalized and followed an in-look link, and `build`
  followed a symlinked `apps/` directory (the kernel resolves parents).
- Wallpaper and font files still go through `contained_file` (in-look
  links allowed): `build` never reads them, so there is no rule to match.
- One difference stays on purpose: `build` refuses a hard-linked leaf
  (its existing check), `apply` accepts one, since a Nix store optimiser
  hard-links identical files and would otherwise break installed looks.
- A directory at a leaf path now refuses in apply (it was silently
  skipped), like build.
- Tests: `leaf_path::tests`, `crates/scoot-look/tests/leaf_symlinks.rs`
  (build) and `a_symlinked_static_leaf_refuses_in_every_shape_*` (apply)
  run the same six-case table.
