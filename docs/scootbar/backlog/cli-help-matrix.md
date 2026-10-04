---
title: "The help's module list is a combinatorial macro matrix that doubles with every module"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# The help's module list is a combinatorial macro matrix that doubles with every module

Filed 2026-10-04, from the review of PR #414 (the power module). Serves
**daily-drive** indirectly: every new module now costs a 1,000-arm,
script-generated change to `crates/scootbar/src/cli.rs` before it can land.

## The gap

`--help`'s `Modules:` line is built by `macro_rules! modules` with one
definition per complete combination of module features, each under a
`#[cfg]` naming that exact combination. The power module doubled it from
263 to 519 definitions (`cli.rs` 8,229 to 15,201 lines); the next module
doubles it again to about 1,031 (about 30,000 lines). Exactly one arm
compiles per build, so the binary cost is nil: the cost is source size,
review load, and a generator every module author has to run.

## What to do

Build the list at run time from per-module `#[cfg]`-gated fragments: a
small function pushes each enabled module's name, with the separators done
in code (the `*_head!`/`*_tail!` fragment macros are already the right
shape; only the fixed middle is combinatorial). `--help` is a cold path,
so one small allocation there costs nothing. The `clock`, `workspaces` and
`window-title` variants collapse the same way. Keep the
`the_help_matches_the_build` and `the_modules_line_is_the_registry_in_order`
tests passing, unchanged in what they assert, across the feature matrix.
Then delete the generator script.

## Not in this ticket

Restructuring the rest of `--help`.
