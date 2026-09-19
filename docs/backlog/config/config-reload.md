---
title: "No config reload: every setting is read once at startup"
status: "open"
area: "config"
priority: "medium"
blocked: "sequenced behind the `scootctl` split and milestone 6 (user, 2026-09-19) — not a technical block"
---

# No config reload: every setting is read once at startup

Requested 2026-09-19. `README.md`'s "Not yet" list already states it: *"No
config reload. Settings are read once at startup."* Changing a keybinding,
a gap or a colour means quitting the session — which on `--tty` means
losing every client in it.

That is the sharp edge. On `--nested` a restart is an inconvenience; on a
daily-driven `--tty` session it is the difference between tweaking a colour
and closing your work.

## What "reload" has to mean, field by field

Not every setting can be re-applied, and pretending otherwise is worse than
refusing. Each `[section]` field is currently documented as *startup-only*
in `docs/configuration.md`, and the reasons differ:

- **Trivially re-appliable:** `[layout] gap`, `[appearance]` colours and the
  focus ring. Recompute the arrangement, request a render.
- **Re-appliable with work:** `[binds]`. Rebuilding the keybinding table is
  easy; the care is in what happens to a key currently held down, and in
  the `--tty` rule that `Ctrl+Alt+F1..F12` always win — a reload must not be
  able to strip the one recovery path.
- **Probably not re-appliable:** `[output] scale`, which is documented
  startup-only because clients are told the scale at bind time;
  `[tty] gpu`, which names the device already being driven; and
  `[renderer] backend`, which would mean tearing down a live renderer with
  client textures in it.

So the honest shape is likely **partial reload with an explicit list**, not
"reload the config". A reload that silently ignores half of what the user
changed is its own bug report.

## Two mechanisms, and they are separable

1. **A trigger.** An IPC request (`scoot msg reload`, natural given the
   IPC-first design and trivially scriptable) and/or `SIGHUP` and/or
   watching the file with inotify. The IPC one is the cheapest and fits how
   everything else in scoot is driven; file-watching is the most convenient
   and the most surprising.
2. **The re-application itself**, which is the real work, and which the
   field list above scopes.

Starting with the trigger plus the trivially-re-appliable set would deliver
most of the value — colours and gap are what people iterate on — and would
establish the mechanism before the hard fields need deciding.

## Failure semantics matter here more than at startup

`docs/configuration.md` records that a malformed config at startup falls
back to defaults and logs, while an explicit `--config PATH` that cannot be
read is a hard error. A *reload* must not do either of those: silently
reverting a live session to defaults because of a typo would be far worse
than at startup, where nothing is running yet. A failed reload should keep
the running configuration and say so loudly.

Related: `docs/backlog/config/default-config-command.md`, which is the
other half of the config-editing experience.
