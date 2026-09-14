---
title: "A config-file key for `--tty`'s DRM device, so `--gpu` doesn't have to be retyped on every launch."
status: "open"
area: "tty"
priority: "medium"
blocked: "blocked on user's Asahi hardware confirming --gpu"
---

# A config-file key for `--tty`'s DRM device, so `--gpu` doesn't have to be retyped on every launch.

A config-file key for `--tty`'s DRM device, so `--gpu` doesn't have to be
retyped on every launch. Suggested by the review of PR #27, 2026-09-13;
deliberately *not* in that PR. On hardware where the automatic search picks
wrong (the Apple Silicon case item 17 exists for), `--gpu PATH` is the fix,
and a fix you must remember to type is not the daily-drive form of one — a
display manager, a `.desktop` entry or a shell alias each have to carry it
separately. Shape: a new `[tty]` section (the config file has
`[layout]`/`[appearance]`/`[binds]` today, none of which fits a backend
device path) with a `gpu = "/dev/dri/card1"` key, and `--gpu` overriding it
the way an explicit flag should. Small: `config.rs` already has the
parse-and-warn-per-key machinery, and `tty::init` already takes the path as
an `Option<&Path>` argument, so nothing below `compositor::run` changes.
Gate on the user confirming `--gpu` actually fixes their Asahi machine
first — if it doesn't, the right shape of the persistent setting may not
be a device path at all, and this would be a config key shipped for a
workaround that didn't work.
