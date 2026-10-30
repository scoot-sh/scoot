---
title: "nix test's scootnotify bind guard disagrees with the CLI"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# nix test's scootnotify bind guard disagrees with the CLI

Filed 2026-10-30 from the reviews of PRs #824 and #825. Serves
**daily-drive**: the guard exists so a rendered `scootnotify msg ...` bind is a
command the CLI accepts, and today it can be wrong both ways.

## The gap

The check in `nix/tests.nix` ("every `scootnotify` bind the profile renders is a
command the CLI accepts") matches each bind against

```
spawn [^ ]+/bin/scootnotify msg (dismiss (--all|[0-9]+)|dnd (on|off|toggle|status)|restore)
```

while `crates/scootnotify/src/cli.rs` accepts something different:

- **Looser:** `dismiss 0` and ids over `u32::MAX` pass the regex; the CLI parses
  `u32` and rejects 0.
- **Tighter:** a bare `msg dnd` (reads the mode) is valid in the CLI but fails
  the regex.
- **Blind spot:** the guard filters on `builtins.isString b`, so a bind written
  as an attrset (the table form) is skipped, not checked.

## What to do

Keep the guard but make it agree with the CLI: accept a bare `dnd`, bound the id
to 1..4294967295, and handle (or explicitly assert the absence of) non-string
binds. Better, drive the rendered binds through the real `scootnotify` parser
(an argument-check entry point or `--help`-style dry run) so the two cannot
drift, if the check can depend on the built binary.

## Not in this ticket

Changing the CLI's grammar.
