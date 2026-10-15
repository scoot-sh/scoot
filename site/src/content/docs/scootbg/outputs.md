---
title: One output or every output
description: "Set every output at once or one by name, kept across replugs."
---

`set` with no `--output` replaces every choice, per-output ones included; with `--output NAME` it sets that output only.

## One output or every output

`set` with no `--output` replaces every choice, per-output ones included;
with `--output NAME` it sets that output only, and the choice is kept by
name, so the monitor shows it again when it is unplugged and plugged back
in. A name no output has right now is an error (exit 1) and changes
nothing. `clear` does the same with nothing to show.
