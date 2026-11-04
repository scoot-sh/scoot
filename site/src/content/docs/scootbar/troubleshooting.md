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
- *Out of memory: the bar says `out of memory: cannot allocate N bytes`
  naming an icon image or a font, or an icon is blank.* The bar runs under
  an address-space limit (`ulimit -v`, systemd's `LimitAS=`) or strict
  overcommit (`vm.overcommit_memory=2`), and the allocator refused a block.
  A refusal does not end the bar: a font or `icon-image`
  PNG that does not fit is refused with that error, naming the file; a
  tray icon is left out; an icon or glyph
  that cannot be drawn this frame is blank and tried again on the next
  one. The PNG decoder's and the glyph rasterizer's own working memory is
  not fallible, so a limit tighter than they
  need still ends the bar. Give it
  address space rather than shrinking the config:

```sh
prlimit --as --pid "$(pidof scootbar)"   # the bar's address-space limit
cat /proc/sys/vm/overcommit_memory       # 2 means strict overcommit
```

  Raise or drop the `LimitAS=` line that starts it (or start it with
  `ulimit -v unlimited`).
- *Bar vanished after the compositor restarted.* The bar exits with
  status 1 when the compositor goes away and does not reconnect. The
  unit starts it again after two seconds; run by hand or from
  `[autostart]`, it stays gone until started again. See
  [When the bar exits](./index.md#when-the-bar-exits).
- *Crash on start before the compositor.* The unit retries every two
  seconds until `WAYLAND_DISPLAY` is imported — a failed first start is
  normal, not an error.

## Diagnose without running

```sh
scootbar daemon --check
```

Validates the config, the flags over it, the modules and the font —
as a start does them, with no compositor and nothing written. A file
with an unknown key is refused by name. (Full detail in [CLI
reference](./cli.md#--check-validate-without-running).)
