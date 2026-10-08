---
title: "Seamless in scoot: a `[bar]` section that starts, reloads and restarts the bar"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "ongoing"
resolved: "2026-10-08"
---

# Seamless in scoot

Filed 2026-09-29. Serves **daily-drive**. The bar ships standalone first
(launched from `[autostart]`, own config file); this is the optional polish,
the way [scootbg's `[wallpaper]`](../../scootbg/backlog/resolved/scoot-integration-done.md)
was.

## What it would add

- A `[bar]` section in scoot's `config.toml` that starts `scootbar` at
  session start and re-applies on `scootctl reload`, so no autostart line.
- **Restart on crash.** Scoot does not supervise clients today, and the bar's
  release profile aborts on a panic. Decide whether scoot restarts a bar that
  exits abnormally (bounded backoff, never a loop) or leaves that to a systemd
  unit ([robustness-and-limits](robustness-and-limits.md)).
- Scoot-side defaults (a bar height that matches its gap and ring settings).

## The real question

Coupling. The bar's config staying its own file is what lets it run on other
compositors and lets a bar change never break scoot. Any `[bar]` section must
be a thin launcher and pass-through (like `apply-config`), not a second schema
for the same keys. If the cost is a second place to look for options, the
standalone `[autostart]` line is the better default and this entry is a
deliberate refusal.

## Done when

Decided either way with the reasoning recorded; if built, `scootctl reload`
re-applies the bar and a crashed bar comes back at most a bounded number of times.

## Resolution — documented won't-do (the unit-based path already covers it)

Decided 2026-10-08 without building a `[bar]` section: the supervisor the
ticket asks scoot to become already exists, and building it again inside
scoot would duplicate it while adding a second scoot-to-bar coupling the
project deliberately avoids. Nothing was invented; the evidence is the
current tree.

- **Start is covered.** `nix/modules/scootbar-home.nix` and
  `nix/modules/scootbar-nixos.nix` run the bar as a systemd user service
  (`scootbar.service`): `PartOf`/`After` the session scope
  (`scoot-session.target` under the desktop profile, `graphical-session.target`
  standalone), `Restart = "on-failure"`, `RestartSec = 2`,
  `StartLimitIntervalSec = 0` (retries never give up), `KillMode = "process"`
  (a restart kills only the bar, not apps its buttons launched). The desktop
  profile already enables and themes the bar through `programs.scootbar`
  (`nix/modules/desktop.nix`: `bar.enable`, default true; `nix/modules/scootbar.nix`:
  profile turns the bar on at `mkDefault` and applies the look's colors).
  Without Nix, `scootbar daemon` runs from `[autostart]` (site
  `scootbar/index.md`: "Enable it" and "Without Nix"). No `[bar]` section
  exists in scoot's config today (`FileConfig` in
  `crates/scoot/src/compositor/config.rs` carries `[autostart]` and
  `[wallpaper]` only), so there is no half-wired code to remove.
- **Reload is covered.** A config change restarts the bar through the unit's
  triggers (`X-Restart-Triggers` / `restartTriggers` on the rendered
  `bar.toml`; the bar starts in milliseconds), and by hand
  `scootbar msg reload` re-reads the file and live-applies it
  (site `scootbar/cli.md`, `scootbar/configure.md`). A scoot-driven
  re-apply on `scootctl reload` would be a second supervisor for a daemon
  that already reloads itself.
- **Restart on crash is already decided — against scoot doing it.**
  [robustness-and-limits](robustness-and-limits.md) says it in as many
  words: "scoot does not supervise clients. Ship a systemd user unit /
  home-manager `Restart=` ... and say in the docs what happens without
  one." The unit's crash path is measured, not hoped for:
  `scripts/scootbar-unit-test.sh` S2 kills the bar with `SIGKILL` and
  asserts `Restart=on-failure` brings it back, and S7 pins the burst
  limit the 2 s retry stays under. The ticket's "bounded backoff, never
  a loop" is exactly what `RestartSec = 2` plus `StartLimitIntervalSec = 0`
  already is.
- **Scoot-side defaults are already a theming concern, not a launcher.**
  The look themes the bar's colors and font through the one theme helper
  (`nix/modules/scootbar.nix`, `theme-look.nix`; `desktop.bar.enable`
  keeps its meaning "manage the bar at all" and is never a theme switch).
  A `[bar]` height/gap default in scoot's config would be a second place
  to look for bar options — the exact cost the ticket's "real question"
  section names as grounds for refusal.
- **Coupling ([versioning](../versioning.md)).** `scootbar` / `scoot`
  speak standard protocols only, plus the optional IPC feature guarded by
  the IPC protocol number, so "a bar from one release works against a
  compositor from another" and each releases without the other. The
  wallpaper precedent cuts the other way: `[wallpaper]` exists because
  wallpaper state needs reconciling (fingerprint over a canonical
  encoding, per-backend profile, adopt-and-reconcile, a bounded 40 s run
  queue, reaper logging, an fd audit — `crates/scoot/src/compositor/wallpaper.rs`
  and the resolved scootbg entry). The bar has no equivalent state to
  reconcile: its config is its own file, its reload is idempotent, and a
  newer scoot sending a key an older bar does not know must fail loudly
  rather than silently drop it. A thin launcher would still be a second
  CLI coupling (binary path, version skew, exit-status logging, fd
  hygiene) with none of wallpaper's payoff.
- **The docs already say "pick one route".**
  `programs.scootbar.systemd.enable` (in `nix/modules/scootbar.nix`):
  "restarted when it fails (scoot does not supervise its clients). Turn
  it off to start `scootbar daemon` from your compositor's autostart
  instead." Site troubleshooting: "pick the unit *or* autostart, not
  both. The second refuses." This PR adds the one sentence that was
  missing: scoot's config deliberately has no `[bar]` section.

No behavior change, no test (docs-only close: nothing to fail first), no
`PROTOCOL_VERSION` bump, no CHANGELOG (nothing user-visible changed) —
the same shape as the `_NET_WM_ICON` verify-first close.

Revisit if and when the unit-based path stops covering a real setup: a
session without systemd where `[autostart]` ordering genuinely breaks the
bar (today autostart already starts it), or a bar setting that can only
be known inside the compositor at run time (none is on the table; static
look values already flow through the profile).
