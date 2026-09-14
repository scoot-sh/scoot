---
title: "The Nix package's `src = self` invalidates the whole build on any doc-only edit (LOW, non-blocking)."
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# The Nix package's `src = self` invalidates the whole build on any doc-only edit (LOW, non-blocking).

The Nix package's `src = self` invalidates the whole build on any
doc-only edit (LOW, non-blocking). Found by `flexwm-reviewer` reviewing
item 16: `src` is the whole flake tree — `CLAUDE.md`, `README.md`,
`ROADMAP.md`, `vm/`, `scripts/` included — none of which the compiler
reads, but a change to any of them still busts the derivation's cache and
forces a full ~3.5 minute rebuild. Demonstrated directly: appending one
newline to `README.md` changed the output path entirely. Since this repo
edits `ROADMAP.md` on essentially every PR, that's a real recurring cost
once this lands. Fix direction: a `lib.fileset` filter scoped to
`Cargo.toml`/`Cargo.lock`/`crates/` — but first confirm nothing the build
actually needs lives outside that set (a `build.rs`, an `include_str!` of
a root-level file, a license file read at build time). Two reads were
checked and are safe (`vm/compositor-deps.nix`'s import, and
`builtins.readFile ./Cargo.toml` for the version string, both of which
resolve against the flake tree rather than `src`), but that check should
be redone against whatever the tree looks like when this is picked up,
not assumed still true.
