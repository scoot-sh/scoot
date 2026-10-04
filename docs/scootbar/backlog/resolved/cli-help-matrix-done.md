---
title: "The help's module list is a combinatorial macro matrix that doubles with every module"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-04"
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

## Resolution (PR #417, 2026-10-04)

`crates/scootbar/src/cli.rs` went from 15,201 to 1,551 lines: the 519
`modules!` arms and the 18 `*_head!`/`*_tail!` fragment macros are gone,
replaced by `modules_section()`, which sorts the registry's ids and
assembles the `--left`/`--padding`/`--spacing`/`--clock-format` block at
run time (one small allocation on the cold `--help` path).
`DAEMON_HELP` is now `daemon_help()` (`DAEMON_PRE` + section +
`DAEMON_POST`); `Topic::text` returns `Cow<'static, str>`. Both help
tests assert the same properties (the `Modules:`-line test binds the
built string first). The two `gen_modules_arms.py` bench records are
deleted; the module notes say what a new module adds to the help.

Evidence: old tree `47c382fbe` vs new, 29 feature combinations
(none, each of 17 alone, default, `--all-features`, 9 pairs/triples
through the clock/workspaces/window-title variants), `diff -r` over
`--help`, `daemon --help` and `msg --help` empty (87 files). Release
default binary: file size unchanged (2,167,520 bytes), `.text` +1,632
bytes (+0.10%); idle RSS/wakeups/fds unchanged. fmt, the CI-exact clippy
matrix (35 runs), nextest (1,321 passed) and `cargo test` all clean.

**Maintainer's ruling (2026-10-04, given in chat): the `.text` growth
(+1,632 B, +0.10%, the run-time builder on the `--help` path) is waived.**
It covers that row only; the file does not grow. Nothing else is waived.
