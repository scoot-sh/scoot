---
title: "apply follows an in-dir symlink for a leaf while build refuses all symlinks"
status: "open"
area: "packaging"
priority: "low"
blocked: null
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
