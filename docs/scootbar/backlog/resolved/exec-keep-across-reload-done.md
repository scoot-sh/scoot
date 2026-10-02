---
title: "Keep an unchanged exec command running across a reload"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-01"
---

# Keep an unchanged exec command running across a reload

Filed 2026-10-01, from the review of the `exec` PR. Serves **daily-drive**: a
config reload (a Stylix switch, a theme edit, `scootbar msg reload`) should
not restart a weather script or kill the state of a long-lived watcher it did
not touch.

## The gap

A reload builds every module again (`daemon/respond.rs` `apply`, then
`modules::start`), so **every `exec` command is killed and started afresh**,
whether or not its table changed (decision 5 of
[exec-push-button-modules-done](resolved/exec-push-button-modules-done.md)).
For a command that prints once a minute that means the module shows its
placeholder, and runs the script's first iteration again, after every reload.
Cheap to live with (a reload is rare, and a print-first script shows its line
at once) and not a leak, so it is a refinement, not a defect.

## What to do

`exec::Settings` already derives `PartialEq`. On a reload, hand the old
placed modules to `modules::start` and let an `exec` whose id and `Settings`
are equal take the old instance (its child, pipe, timer and what it showed)
instead of starting a new one. The module trait has no way to say "what are
you configured with", so it needs one small hook (`Module::keeps(&self,
&Kind) -> bool`, or a `Box<dyn Any>` downcast), plus tests for: an unchanged
table keeps its pid; a changed `command`, `format`, `placeholder` or restart
key replaces it; a module removed from the lists is dropped (and its group
killed); one moved between sections or outputs keeps running; a refused
reload keeps everything as it is.

Measured size of the change when the review asked for it: not small, because
it touches `modules::start`'s signature, the trait and `custom.rs`, and every
other module kind has to say it never keeps anything. That is why it is a
ticket and not part of the PR that found it.

## Not in this ticket

Keeping `push` values across a reload (a `push` is cleared by a reload today,
by design: its value is a state somebody set), and keeping a `button`'s or the
clock's state (they have none that matters).

## What landed

[`docs/scootbar/cli.md`](../../cli.md) documents the behavior (the `exec`
module's "Children" bullet, and the multi-output reload bullet).

- **`Module::keeps(&self, &Custom) -> bool`** (`crates/scootbar/src/modules/mod.rs`):
  the one small hook, defaulting to false. Only `exec` overrides it
  (`modules/exec/mod.rs`): an equal [`Settings`] (command, format,
  placeholder and restart key, all in the derived `PartialEq`) keeps the
  instance. Every other module kind (clock, workspaces, button, push) takes
  the default and starts fresh.
- **`modules::start` takes the old bar** (`&mut Vec<Placed>`): a placed id
  whose custom table's `keeps` holds moves over with its child, pipe, timer,
  shown output and revision (bindings refreshed), wherever the lists place
  it now — a move between sections, or between outputs (the policy merges
  before `start`), keeps it running. What is left in `old` is for the caller
  to drop, which kills a removed `exec`'s whole group.
- **The font is validated before anything moves** (`daemon/respond.rs`
  `Responder::stage`, called by `apply` with the live modules): a refused
  reload (bad file, flag clash, vanished font) leaves the running bar
  exactly as it was — nothing started, nothing moved, nothing killed.
  Whether a font is needed is the merged layout's answer, not the started
  modules': every placed module starts unless the process is out of file
  descriptors, and then the bar is past refusing a reload.

## Evidence

Captured on the dev VM (`ssh -p 2222 dev@localhost`, tree at `/mnt/scoot`,
the 9p mount of this checkout), at the commit carrying this entry:

- `cargo nextest run -p scootbar` — 730 run: 729 passed, 1 failed:
  `agent::layout_rectangles_are_where_a_click_lands_on_two_outputs_at_two_scales`
  wants headless-2 at scale 1.5 and finds 1.0. Pre-existing and
  environmental, proven by running it with this lane's changes stashed
  (same failure on the clean tree); nothing in this change touches outputs,
  scales, layout or agent code.
- New tests, all passing: `modules::keep_tests` (an unchanged table keeps
  its child, bindings and revision; a changed command, format, placeholder
  or restart key replaces it; a removed module is dropped and its group,
  worker included, is killed; a move between sections and a move between
  outputs through `to_start` keep it running; `keeps` unit cases and the
  default-decline pins for a silent module, `button` and `push`) and
  `daemon::respond::tests::a_font_that_vanished_refuses_before_anything_moves`.
- `cargo clippy -p scootbar --all-targets -- -D warnings` — clean.
- `cargo fmt --check -p scootbar` — clean.
- `cargo check -p scootbar --no-default-features` and `--features exec` —
  both build (the per-module feature matrix).

No hot path is touched (the render loop, input dispatch and IPC dispatch
are unchanged; `start` runs at start-up and on reload only), so there is
no before/after benchmark: the added work per reload is one short scan of
the old modules per placed custom id plus one small `Settings` comparison.
