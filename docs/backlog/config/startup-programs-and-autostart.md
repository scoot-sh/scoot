---
title: "Nothing says how to start a bar, a launcher or a browser at session start — and `--` takes exactly one command"
status: "open"
area: "config"
priority: "medium"
blocked: null
---

# Nothing says how to start a bar, a launcher or a browser at session start — and `--` takes exactly one command

Asked by the user, 2026-09-19: *"How do we handle startup exec of other
processes like Waybar, fuzzel, browsers, etc? Do we make it easy for startup
desktop-env type things?"*

The answer today is "a shell script", and it works — but the project has
never written that down, and the one place that comes close doesn't say it.
`docs/protocols.md:72-80` shows exactly the right commands:

```sh
swaybg -c '#123456' &     # a wallpaper, on the background layer
waybar &                  # a bar, on the top layer
fuzzel                    # a launcher, on the overlay layer
```

…under the sentence *"Start one the same way you start anything else inside
the session"* — which never says **where** that shell runs, or how it gets
started, or that `--` is how you get one. Someone coming from sway
(`exec`), niri (`spawn-at-startup`) or Hyprland (`exec-once` in the hyprlang
era; `hl.exec_cmd` since it moved to Lua) will look in the config file
first, find nothing, and reasonably conclude scoot can't do this.

## What actually exists

- **One startup command.** `cli.rs:309-312`: the `"--"` arm does
  `options.command = args.by_ref().collect()` and breaks — so everything
  after `--` is *one* program and its arguments, not a list. `scoot
  --headless -- foot -e sh` runs `foot`; there is no second slot.
- **Fire and forget.** `mod.rs:239-242` calls `state.spawn(&command)` and
  moves on. scoot does not wait on it and does not exit when it exits.
- **Placement is already right.** That spawn happens *after* `ipc::init` and
  after the `set_var` block at `mod.rs:218-237`, so the child inherits
  `WAYLAND_DISPLAY`, `SCOOT_SOCKET` and the cursor variables for free. Any
  list added at the same site inherits the same thing — that placement is
  the whole design, and it is already correct.

So a session script is the only mechanism, and it is a good one: `scoot
--tty -- /path/to/session.sh`, with the script launching the bar, the
wallpaper and the notification daemon and then `wait`-ing.

## The principle worth stating: config is *state*, the script is *behavior*

scoot's config describes what the compositor **is** — layout, appearance,
binds, renderer, output, tty. The `--` command describes what the session
**does** at startup. Keeping those apart is why `config.toml` has no `exec`
and arguably shouldn't grow one as a special form.

What makes this more than taste here is a property scoot has and most peers
don't. `compositor/config.rs:558-565`:

```rust
/// … `value` is an action string in exactly the grammar `scoot msg action
/// ...` uses (`"focus-column left"`, `"close"`, `"spawn" "foot"`, ...) --
/// see `cli::action`, reused here rather than duplicated.
let mut tokens = value.split_whitespace().map(str::to_owned);
let action = crate::cli::action(&mut tokens).map_err(|error| error.to_string())?;
```

A `[binds]` value and a `scoot msg action …` argument list are parsed by the
**same function**. One vocabulary, three doors — keybind, IPC, startup.
That is what makes the config agent-legible: an agent that reads
`config.toml` already knows what every value means, because it is the
grammar the agent itself speaks. Anything added here should preserve it.

Two other rules the code already follows and the docs should state, so the
next key doesn't have to guess:

- **Fail-open by default, fail-closed by named exception.** A bad value logs
  and falls back; `[tty] gpu` and `[renderer] backend` refuse, because
  silently rendering with the wrong one is worse. (`docs/configuration.md`'s
  "Failure semantics" has the specifics; what's missing is the rule behind
  them.)
- **Defaults are derived, not maintained** — see
  [`default-config-command.md`](./default-config-command.md).

## Shape

**1. Document the startup model.** The cheapest and largest win. A short
"Starting a session" section in `README.md`/`docs/configuration.md` with a
real `session.sh` — bar, wallpaper, notification daemon, `exec`-ing or
`wait`-ing at the end — plus a webtop `/defaults/startwm.sh` variant, since
that is the deployment target `README.md` names. Nothing to build.

**2. Optionally, `[autostart]` as a list of the same action strings.**

```toml
[autostart]
commands = [
    "spawn waybar",
    "spawn fuzzel --daemonize",
]
```

Deliberately the *action* grammar, not a bare argv list, so rule one holds:
every string here is something `scoot msg action` would accept and a
`[binds]` value could contain. It covers the 80% case that needs no ordering
or conditionals, it costs almost nothing on top of the existing `spawn`, and
it goes at `mod.rs:239` where the environment is already right. The script
stays for the 20% — ordering, `sleep`, conditionals, `wait`.

The honest argument against: it is a second way to do something that already
works, and river deliberately has only the script. The argument for is that
three of scoot's four named peers put it in the config, so that is where
people will look.

**3. Home-manager mapping** (see
[`flake-consumer-and-home-manager.md`](../packaging/flake-consumer-and-home-manager.md)):
`programs.scoot.settings` renders the TOML, `programs.scoot.sessionScript`
carries the behavior half. The state/behavior split maps onto it cleanly,
which is some evidence the split is the right one.

## What is out of scope: supervision

Restarting a bar that died, backing off a crash loop, telling a deliberate
quit from a crash — that is a service manager's job. No peer compositor
supervises: sway's `exec`, niri's `spawn-at-startup`, Hyprland's and river's
`init` are all fire-and-forget. The systemd route is the usual answer —
niri ships a session script that imports the environment into the user
manager, Hyprland ships `example/hyprland.service`, and waybar ships
`resources/waybar.service.in`. (sway does *not* ship a unit in-tree; that is
the third-party `sway-systemd`, which is worth knowing before citing sway as
the precedent.) Reimplementing systemd badly inside a compositor is a
well-known trap, and it would sit directly in the render loop's process.

The wrinkle worth recording: **the webtop target has no systemd.** Its init
is s6-overlay and scoot's `--` command is the container entrypoint, so on
that target supervision belongs to the container, where linuxserver images
already put it. If a scoot-native supervisor is ever wanted it should be a
separate tool, the same way a status bar is.

What *is* in scope, and is a real bug found asking this question:
[`spawned-children-never-reaped.md`](../core/spawned-children-never-reaped.md)
— every child scoot spawns becomes a zombie, confirmed live. Reaping is the
compositor's job whether or not anything ever supervises.

Also related:
[`session-environment-and-portals.md`](../core/session-environment-and-portals.md)
— a session script is also where the D-Bus activation environment would get
updated, so the two entries share a mechanism.
