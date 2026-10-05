---
title: scootbar troubleshooting
description: "Bar symptoms and their fixes — placement, fonts, modules, the daemon."
---

## Symptoms

- *Bar missing on the second monitor.* Check `--outputs` (default
  `all`; a name no output has is refused at start). Then check the
  exclusive zone: with `--exclusive true` (default) the bar reserves
  its height per output — a bar that reserved nothing floats over
  windows instead. Then `scootbar msg layout`: where each module is,
  per output.
- *Bar won't start: font.* With modules placed the bar needs a font:
  `--font` a path, or the first well-known file found; with neither it
  refuses, saying how to give one. A bar with no modules needs none.
  See [Fonts](./configure.md#fonts).
- *Bar won't start: unknown module / unknown key.* A module id the
  build doesn't carry (a `features` list without it) is a usage error
  naming it; a config table for an unbuilt `button`/`push`/`exec` is
  refused the same way. Rebuild with the feature or drop the table.
- *Config change did nothing.* Flags win over the file, at start-up
  and on reload — a flag you forgot still overrides the file's value.
  `scootbar msg reload` re-reads the file; the NixOS/Nix unit restarts
  on a changed config automatically.
- *Two bars / second one refuses.* One daemon per display holds the
  control socket: pick the unit *or* autostart, not both. The second
  start is refused; the unit then retries every two seconds.
- *Module shows nothing / click does nothing.* `scootbar msg query`
  shows every placed module's state; `scootbar msg layout` shows where
  each is. A `push` the bar refuses names why (usually a module not
  placed yet). Pointer behavior per module is in
  [Pointer input](./modules.md#pointer-input).
- *Popup won't open / tooltip never shows.* Popups come from the
  module's own state ([Popups](./modules.md#popups)); tooltips wait
  `tooltip-delay` (default 500 ms, `0` is off — [Tooltips](./modules.md#tooltips)).
- *Crash on start before the compositor.* The unit retries every two
  seconds until `WAYLAND_DISPLAY` is imported — start clean is normal,
  not an error.

## Diagnose without running

```sh
scootbar daemon --check
```

Validates the config, the flags over it, the modules and the font —
as a start does them, with no compositor and nothing written. A file
with an unknown key is refused by name. (Full detail in [CLI
reference](./cli.md#--check-validate-without-running).)
