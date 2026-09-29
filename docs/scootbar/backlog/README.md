# scootbar backlog

scootbar's own backlog, kept apart from the compositor's
([`docs/backlog/`](../../backlog/README.md)) and scootbg's
([`docs/scootbg/backlog/`](../../scootbg/backlog/README.md)). Same format:
one file per item, YAML frontmatter (`title`, `status`, `area: "scootbar"`,
`priority`, `blocked`, and a `milestone`). Use `scripts/backlog` (or the
`backlog` skill):

```sh
scripts/backlog list --area scootbar --milestone M1   # what is in a milestone
scripts/backlog list --area scootbar --unblocked      # what can start now
scripts/backlog show workspaces-module
```

Items move to `resolved/` here when done, with a `-done` suffix.

## What scootbar is

A status bar for scoot, and for any compositor with `wlr-layer-shell-v1`:
the lightest bar that is still beautiful and configurable. It starts as a
clock, gains workspaces, and grows by modules. It is the first piece of a
composable shell; a launcher (`scootlaunch`) and a notification daemon
(`scootnotify`) follow, each its own small binary.

## How it ships: small steps, each one usable

**There is no big-bang release.** Work is cut into milestones, and each one is a
state of `main` that is useful on its own: any milestone can be the last one
shipped. The first is a clock you can `nix run` on day one; the second is a bar
you can live with; everything after adds a capability, one at a time.

A milestone (and each entry in it) is **done** when:

1. it went through the full per-feature cycle and its review (`CLAUDE.md`);
2. it is installable: `nix build .#scootbar` works and `nix flake check` is green
   ([nix-package](nix-package.md)), and CI covers it;
3. its docs are updated in the same PR (`docs/scootbar/`);
4. its numbers are published and pass the
   [resource ratchet](lightest.md): no row worse than the last milestone, and no
   competitor ahead at this scope;
5. the bounds for every new surface exist with a test that fails without them
   ([robustness-and-limits](robustness-and-limits.md)).

## Decisions the entries assume

Settled in planning (2026-09-29); an entry may reopen one with evidence.

- **One small daemon per component**, not one shell process. Each is optional
  and isolated.
- **Event-driven, no polling.** One single-threaded `poll(2)` loop; no async
  runtime. A module owns an fd the loop waits on, or costs nothing. Idle wakeups
  are a measured row, not a hope.
- **Modules are built in and chosen at compile time** (Cargo features), behind
  one small trait: a module reports what it shows, wakes on its own sources and
  handles clicks and scrolls. The bar owns layout, theme, drawing and damage. No
  dylib plugins, no embedded scripting, no CSS engine.
- **Extension is `exec` and `push`** (streamed lines, and a socket call), with the
  same interaction keys as a built-in; scootbar's own JSON, not Waybar's.
- **Standard protocols first**, so it runs on other compositors like scootbg;
  scoot IPC is an optional extra. The bar's own control socket is separate from
  scoot's IPC.
- **Its own config file, colors as semantic tokens**, so Stylix and any theme
  source map onto it. Until that file exists, options are command-line flags.
- **`wl_shm` only, real device pixels, damage-limited redraws**, GPU-free.
- **Decisions are made by the entry that needs them**, with a measurement, not
  months ahead. The shared ui crate is extracted when a second consumer appears.
- **Distribution is part of the product**: Nix from the first milestone, distro
  packages and a binary cache later
  ([project packaging backlog](../../backlog/README.md#packaging-and-releases-filed-2026-09-29)).

What people actually complain about in bars (workspace fragility, the tray,
memory growth, resume and hotplug crashes, polled scripts) shaped the order and
the bounds: [research notes](../research.md).

## Milestones

`blocked` on each entry names what it waits on. Priorities are inside a milestone.

### M0 — Measure (small, then stop)
- [**Baselines, and the two spikes the first milestone needs**](resolved/baselines-and-spikes-done.md) — RESOLVED 2026-09-29: `ab_glyph` over a mapped font, a timerfd clock with a hand-rolled TZif reader, and the [baselines](../README.md#baselines). The record later entries extend: [dependencies-done](resolved/dependencies-done.md)

### M1 — A clock you can `nix run`
Walking skeleton: every piece exists once, end to end, so later steps only add.
- [**Skeleton: a layer surface per output**](resolved/skeleton-layer-surface-done.md) — RESOLVED 2026-09-29: `scootbar daemon`, a solid bar per output across hotplug, device pixels at fractional scales, zero idle wakeups; flags, no config file ([cli.md](../cli.md))
- [**Module API, layout, theme tokens and the clock**](resolved/module-api-and-clock-done.md) — RESOLVED 2026-09-29: the module contract, three-section layout, color tokens, `ab_glyph` text with a bounded cache, and a timerfd clock (`3:07 pm` by default) redrawing only its own span; `--font` or a well-known file, else a refusal; fonts mapped only when root-owned, unwritable and on a read-only mount, as in the Nix store ([cli.md](../cli.md))
- [Nix package](nix-package.md) (high): installable from the start
- [Testing and CI](testing-and-ci.md) (medium): harnesses, path-filtered CI, first benchmark

### M2 — Workspaces: the first bar you can live with
- [Workspaces module](workspaces-module.md) (high): `ext-workspace-v1`, click to switch

### M3 — Configurable
- [Config file, control socket and reload](config-cli-and-reload.md) (medium): `scootbar msg`, `query`
- [Nix modules and Stylix](nix-modules-and-stylix.md) (medium)
- [Appearance](appearance.md) (medium)
- [Icons and fonts](icons-and-fonts.md) (medium)
- [Multi-output policy](multi-output.md) (medium)
- [Visibility and layering](visibility-and-layering.md) (medium)

### M4 — Interactive, and readable by agents
- [Pointer input and interactions](pointer-and-interactions.md) (medium)
- [`exec`, `push` and `button` modules](exec-push-button-modules.md) (medium): launcher and power buttons
- [Agent interface](agent-interface.md) (medium): `invoke`, `layout`, `subscribe`

### M5 — Daily-driver modules, one release each
The [umbrella](data-source-modules.md) holds the rules they share.
- [Window title](window-title-module.md) (medium)
- [Volume](volume-module.md) (medium)
- [Network](network-module.md) (medium)
- [Battery](battery-module.md) (medium)
- [Brightness](brightness-module.md) (low)

### M6 — Infrastructure and the tray
- [A shared D-Bus client](dbus-client.md) (low): its own spike first
- [System tray](tray.md) (medium): the watcher is core infrastructure
- [Popups](popups.md) (low), [Tooltips](tooltips.md) (low)
- [Media (MPRIS)](media-module.md) (low), [Bluetooth](bluetooth-module.md) (low)

### M7 — The rest of the shell (separate products, own backlogs when they start)
- [Extract `scootui`](extract-scootui.md) (low): when a second consumer appears
- [`scootlaunch`: the launcher](launcher.md) (low, pointer)
- [`scootnotify`: the notification daemon](scootnotify.md) (low, pointer)

### Ongoing (no milestone, applies to all)
- [The resource ratchet](lightest.md) (high)
- [Robustness and resource limits](robustness-and-limits.md) (medium): a standing checklist, delivered bound by bound with each surface
- [Decision: no built-in CPU/memory/temperature/disk](system-stats-decision.md) (low)
- [Seamless in scoot: a `[bar]` section](scoot-integration.md) (low)

## Scoot-side changes the bar wants

Filed in the compositor's backlog, and built only when the module that needs them
hits the wall:
[workspace snapshot event](../../backlog/ipc/workspace-snapshot-event.md),
[output-targeted workspace switch](../../backlog/ipc/workspace-switch-targeted-output.md),
[`urgent` state bit](../../backlog/protocols/ext-workspace-urgent-state.md),
[a real maximize](../../backlog/core/maximize.md),
[persistent workspaces (a decision)](../../backlog/core/persistent-workspaces.md),
[keyboard layout event](../../backlog/ipc/keyboard-layout-event.md).
