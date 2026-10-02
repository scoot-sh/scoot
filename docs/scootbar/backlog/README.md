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
   ([nix-package](resolved/nix-package-done.md)), and CI covers it;
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
- [**Nix package**](resolved/nix-package-done.md) — RESOLVED 2026-09-29: `packages.scootbar` (no font in its closure; 49 MB, glibc and libgcc_s) and `scootbar-demo` (DejaVu Sans by default), `pkgs.scootbar`, `nix run`, features by `.override`, eval pins and main-only CI builds ([docs/nix.md](../../nix.md#the-status-bar-scootbar))
- [**Testing and CI**](resolved/testing-and-ci-done.md) — RESOLVED 2026-09-29: snapshot tests of the canvas and the bar at 1x and fractional scales, fake events through the module harness and a contract every registered module is held to, the fuzz corpus replayed on stable and both targets run in CI for a fixed budget, a per-module feature matrix, and `scripts/scootbar-bench` on scootbg's runner, with M1's run as the ratchet's baseline ([testing.md](../testing.md))

### M2 — Workspaces: the first bar you can live with
- [Workspaces module](resolved/workspaces-module-done.md) (high): `ext-workspace-v1`, click to switch — RESOLVED 2026-09-29
- [M1 review follow-ups](resolved/m1-review-followups-done.md) (medium): a clock that retries a failed draw, and four hardening gaps (the fuzz crate's CI build done) — RESOLVED 2026-09-29

### M3 — Configurable
- [Config file, control socket and reload](resolved/config-cli-and-reload-done.md) (medium): `scootbar msg`, `query` — RESOLVED 2026-09-29
- [Nix modules and Stylix](resolved/nix-modules-and-stylix-done.md) (medium): `programs.scootbar` for NixOS and home-manager, Stylix defaults with pinned precedence, the restarting user unit, `scootbar daemon --check`; run under a real user manager and a real Stylix — RESOLVED 2026-10-01
- [Appearance](resolved/appearance-done.md) (medium): rounded corners, opacity, spacing, pill and circle workspaces, the corner-shaped input region, measured on hardware — RESOLVED 2026-09-30
- [Icons and fonts](resolved/icons-and-fonts-done.md) (medium): the fallback chain, a glyph, a path or a PNG icon on the clock — RESOLVED 2026-09-30
- [Multi-output policy](resolved/multi-output-done.md) (medium): `outputs`, `[output."NAME"]` overrides — RESOLVED 2026-09-30
- [Visibility and layering](resolved/visibility-and-layering-done.md) (medium): layer, edge, zone or float, `msg hide|show|toggle` — RESOLVED 2026-09-30

### M4 — Interactive, and readable by agents
- [Pointer input and interactions](resolved/pointer-and-interactions-done.md) (medium): click, scroll and hover, `exec` and `scoot = "quit"` actions — RESOLVED 2026-09-30
- [`exec`, `push` and `button` modules](resolved/exec-push-button-modules-done.md) (medium): launcher and power buttons — RESOLVED 2026-09-30
- [Keep an unchanged `exec` command across a reload](resolved/exec-keep-across-reload-done.md) (low): unchanged tables keep their child — RESOLVED 2026-10-01
- [Agent interface](resolved/agent-interface-done.md) (medium): `invoke`, `layout`, `subscribe` — RESOLVED 2026-09-30
- [Appearance follow-ups](resolved/appearance-followups-done.md) (low): hover token, per-module state colors, dot indicators, inactive-workspace colors — RESOLVED 2026-10-01
- [Bring M4's idle memory and CPU back down](resolved/m4-usage-optimization-done.md) — RESOLVED 2026-10-01, not pursued: the maintainer accepted the idle memory as it stands; the measured cause and levers are kept in the entry

### M5 — Daily-driver modules, one release each
The [umbrella](data-source-modules.md) holds the rules they share.
- [Window title](resolved/window-title-module-done.md) (medium): the focused window's title per output, click to focus — RESOLVED 2026-10-01
- [Volume](resolved/volume-module-done.md) (medium): default sink level and mute, scroll to change, click to mute — RESOLVED 2026-10-02
- [Volume scan_names test passes for the wrong reason](volume-scan-names-test.md) (medium): review follow-up from PR #376
- [Volume re-probe a present socket](volume-reprobe-present-socket.md) (medium): review follow-up from PR #376
- [Network](resolved/network-module-done.md) (medium): link state, WiFi name and signal, click to pick — RESOLVED 2026-10-02
- [Battery](resolved/battery-module-done.md) (medium): charge level and state, warn and urgent classes, the low hook — RESOLVED 2026-10-02
- [Brightness](resolved/brightness-module-done.md) (low): backlight level, scroll to adjust — RESOLVED 2026-10-02
- [Battery: measure unplug and capacity-step uevents](battery-unplug-uevent-measure.md) (medium): needs a human at the Asahi box; review follow-up from PR #383

### M6 — Infrastructure and the tray
- [A shared D-Bus client](dbus-client.md) (low): its own spike first
- [System tray](tray.md) (medium): the watcher is core infrastructure
- [Popups](popups.md) (low), [Tooltips](tooltips.md) (low)
- [Media (MPRIS)](media-module.md) (low), [Bluetooth](bluetooth-module.md) (low)

### M7 — The rest of the shell (separate products, own backlogs when they start)
- [Extract `scootui`](extract-scootui.md) (low): when a second consumer appears
- [`scootlaunch`: the launcher](launcher.md) (low, pointer)
- [`scootnotify`: the notification daemon](scootnotify.md) (low, pointer)

### Unscheduled (until someone asks)
- [Gate integration tests on module features, tidy the unit-test script](scootbar-test-gating-and-script-hygiene.md) (low)
- [drive_placed pidfile-vs-pipe race flakes the exec keep tests](exec-keep-pidfile-race.md) (low): from the battery review (#378)
- [Per-output font size](multi-output-font-size.md) (low): deferred 2026-10-01 by the maintainer; left M3

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
