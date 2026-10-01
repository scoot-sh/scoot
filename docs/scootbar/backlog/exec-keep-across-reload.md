---
title: "Keep an unchanged exec command running across a reload"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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
