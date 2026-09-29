# scootbar backlog

scootbar's own backlog, kept apart from the compositor's
([`docs/backlog/`](../../backlog/README.md)) and scootbg's
([`docs/scootbg/backlog/`](../../scootbg/backlog/README.md)). Same format:
one file per item, YAML frontmatter (`title`, `status`, `area: "scootbar"`,
`priority`, `blocked`). Use `scripts/backlog` (or the `backlog` skill) to
list and edit:

```sh
scripts/backlog list --area scootbar
scripts/backlog show module-api-and-clock
```

Items move to `resolved/` here when done, with a `-done` suffix.

## What scootbar is

A status bar for scoot, and for any compositor with `wlr-layer-shell-v1`:
the lightest bar that is still beautiful and configurable. It starts with
the time and the workspace numbers (the active one marked) and grows by
modules. It is the first piece of a composable shell; a launcher and a
notification daemon follow, each its own small binary.

## Decisions the entries assume

Settled in planning (2026-09-29); an entry may reopen one with evidence.

- **One small daemon per component**, not one shell process: `scootbar`,
  later `scootnotify` and a launcher. Each is optional and isolated.
- **Event-driven, no polling.** One single-threaded `poll(2)` loop over the
  Wayland fd, a timerfd, the control socket and any module fds; no async
  runtime. A module either owns an fd the loop waits on or costs nothing.
  Idle wakeups are a gated measurement, not a hope.
- **Modules are built in and chosen at compile time** (Cargo features), behind
  one small trait: a module reports what it shows (icon, text, state class),
  wakes on its own sources and handles clicks and scrolls. The bar owns
  layout, theme, drawing and damage. No dylib plugins, no embedded
  scripting, no CSS engine.
- **Extension is `exec` and `push`**: a module that streams lines from a
  command, and a control-socket call that sets any module's content, both
  with the same interaction keys as a built-in. scootbar's own JSON shape,
  not Waybar's.
- **Standard protocols first** (`ext-workspace-v1`, foreign-toplevel,
  layer-shell, `wl_output`), so it runs on other compositors like scootbg.
  Scoot IPC is an optional, feature-gated extra for what no standard covers
  (occupied workspaces, `quit` for log out). The bar's own control socket is
  separate from scoot's IPC.
- **Its own config file, colors as semantic tokens** (bg, fg, accent, dim,
  urgent), so Stylix and any other theme source map onto it directly.
- **Rendering**: `wl_shm` only, real device pixels, damage-limited redraws,
  no allocation per frame. GPU-free.
- **The shared ui crate is extracted, not designed up front**: it appears
  when a second consumer does.

## Order

Roughly in order; each is one PR through the full per-feature cycle. Every
entry's `blocked` field names what it waits on
(`scripts/backlog list --area scootbar --unblocked` shows what can start).

**Foundations**

1. [Baselines and spikes](baselines-and-spikes.md) (high): competitor numbers
   and four measured choices before any code
2. [Skeleton: a layer surface per output](skeleton-layer-surface.md) (high)
3. [Module API, layout, theme and the clock](module-api-and-clock.md) (high)
4. [Workspaces module](workspaces-module.md) (high): the v1 headline with the clock
5. [Testing and CI](testing-and-ci.md) (medium): harnesses, fuzz, path-filtered CI
6. [Robustness and limits](robustness-and-limits.md) (medium): bounds, hostile input, a dying compositor

**Interaction and extension**

7. [Pointer input and interactions](pointer-and-interactions.md) (medium)
8. [`exec`, `push` and `button` modules](exec-push-button-modules.md) (medium)
9. [Config file, control socket and reload](config-cli-and-reload.md) (medium)
10. [Agent interface](agent-interface.md) (medium): `query`, `invoke`, `layout`, `subscribe`

**Look and placement**

11. [Appearance](appearance.md) (medium)
12. [Icons and fonts](icons-and-fonts.md) (medium)
13. [Visibility and layering](visibility-and-layering.md) (medium)
14. [Multi-output policy](multi-output.md) (medium)

**Shipping**

15. [Nix package](nix-package.md) (high): ships with the first working milestone
    [Nix modules and Stylix](nix-modules-and-stylix.md) (medium)
16. [The lightest bar: the release gate](lightest.md) (high): v1 is not released until this passes

**Modules** ([umbrella](data-source-modules.md) holds the shared rules)

17. [Window title](window-title-module.md) (medium)
18. [Battery](battery-module.md) (medium)
19. [Volume](volume-module.md) (medium)
20. [Network](network-module.md) (medium)
21. [Brightness](brightness-module.md) (low)
22. [Media (MPRIS)](media-module.md) (low)
23. [System tray](tray.md) (low)
24. [Decision: no built-in CPU/memory/temperature/disk](system-stats-decision.md) (low)

**After v1**

25. [Popups](popups.md) (low)
26. [Tooltips](tooltips.md) (low)
27. [Extract `scootui`](extract-scootui.md) (low): when a second consumer appears
28. [A shared D-Bus client](dbus-client.md) (low)
29. [Seamless in scoot: a `[bar]` section](scoot-integration.md) (low)
30. [`scootlaunch`: the launcher](launcher.md) (low, pointer)
31. [`scootnotify`: the notification daemon](scootnotify.md) (low, pointer)

Scoot-side changes the bar wants are filed in the compositor's backlog, and
are built only when the workspaces module shows it needs them:
[workspace snapshot event](../../backlog/ipc/workspace-snapshot-event.md),
[output-targeted workspace switch](../../backlog/ipc/workspace-switch-targeted-output.md),
[`urgent` state bit](../../backlog/protocols/ext-workspace-urgent-state.md).
