---
title: "No way to emit a default config file"
status: "open"
area: "config"
priority: "low"
blocked: "sequenced behind the `scootctl` split and milestone 6 (user, 2026-09-19) — not a technical block"
---

# No way to emit a default config file

Requested 2026-09-19. `--config PATH` reads a config; nothing writes one.
A new user's only route to a starting point is to copy the example out of
`docs/configuration.md` by hand, which goes stale the moment a default
changes and gives no signal when it has.

Verified against `cli.rs`'s `USAGE` at `056bba6`: there is no
`default-config`, `print-config`, `dump-config` or equivalent.

## Shape

`scoot msg` is the natural home given the IPC-first design, but **this
should not require a running compositor** — the whole point is to produce a
file before you have a session configured. So it wants to be a plain
subcommand rather than a socket request, alongside `--help`:

```sh
scoot --print-default-config > ~/.config/scoot/config.toml
```

Writing to stdout rather than to the path directly is the safer default:
it cannot clobber an existing config, it composes, and the user decides
where it lands. A `--write` convenience that refuses to overwrite could
follow if wanted.

## The part that makes it worth doing properly

Emitting a hand-maintained string would reintroduce exactly the drift this
solves. The output should be **generated from the same defaults the
compositor actually uses** (`Config::default()` and the `Appearance`,
`[binds]`, `[output]`, `[renderer]`, `[tty]` defaults), so that a changed
default changes the emitted file automatically.

The commented-out example in `docs/configuration.md` is the model for the
format — every key present, commented, with its default as the value — and
a test asserting the emitted file parses back to `Config::default()` closes
the loop cheaply.

## Related

- `docs/backlog/resolved/config-reload-done.md` — the other half of the
  config-editing experience. Emitting a starting file matters more once
  editing it does not require a session restart.
- If `scootctl` lands first (`docs/backlog/meta/rename-flex-family.md`),
  decide deliberately whether this belongs on the compositor binary or the
  client. It needs no running compositor, which argues for `scootctl` — but
  it needs the compositor's own defaults, which argues for `scoot`.
