---
title: "scoot look apply: the atomic swap, scoped reloads, rollback, list, current and validate"
status: "open"
area: "core"
priority: "high"
blocked: "look-scoped-reload, look-layer-scootbar"
---

# scoot look apply: the atomic swap, scoped reloads, rollback, list, current and validate

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
the swap itself, and the end of the minimum slice.

## The gap

Nothing switches a look after evaluation. Each app's reload exists (scoot
`Request::Reload`, `scoot-ipc/src/request.rs:115-122`; scootbar's control
socket, `crates/scootbar/src/control/protocol.rs:1-17`), but nothing renders
a look, points the apps at it, or reloads them together.

## What to do

`scoot look list`, `current`, `show`, `validate`, `build`, `apply NAME`,
each with `--json`, per the design doc's sections 3.1, 4 and 7.1:

- The catalog: source looks searched in the design doc's section 3.1
  order (user-looks, installed-looks, home-manager, system, shipped;
  `installed-looks` is empty until `look-install`, but the search includes
  it from the start) and rendered looks kept apart; `apply` renders into
  `$XDG_CACHE_HOME/scoot/rendered/<name>-<hash>/` when no rendered entry
  exists.
- The effectful checks the pure crate leaves to the tool: fonts through
  their carrier, the wallpaper through `scootbg check-image` (new: byte and
  pixel caps from the header before decoding), `scootbar daemon --check`.
- The transaction of section 4.5 under one `flock` (`<state>/look.lock`):
  render and validate, write `look.json` then the link (repairing the link
  from `look.json` whenever they disagree), scoped reloads in order (scoot,
  waiting for the wallpaper; every scootbar of the session), skip what is not
  running, roll back on a refusal, report "rolled back with errors" when a
  rollback reload also refuses, and list shadowed keys.
- The render cache pruned under the lock (active, previous, four most
  recent; never a directory `look.json` or the link names).
- An unwritable `<state>` refuses loudly.
- **The one-key `color-scheme` write** (D11, R2): `apply` writes
  `org.gnome.desktop.interface color-scheme` from the target look's
  `polarity`, under D11's rules (skip a key the Nix list claims or someone
  else changed; adopt it when there is no record yet), and records it in
  `look.json`. Without it a live swap across polarity leaves dark/light
  stale until the next activation, because `look-toolkit-live`'s full
  dconf set lands later. A box without `dconf` gets a note, not a refusal.
  **Adoption overwrites a user's own choice on non-Nix (F3):** with no record
  and no Nix list, the first `apply` replaces a `color-scheme` the user set
  in a settings app. Ship the opt-out with it: `--no-toolkit` per apply,
  and `scoot look toolkit off` (persisted in `look.json`) to skip every
  dconf write; document both beside the first-apply description. Before
  `look-nix-catalog`, home-manager restores its own `color-scheme` at the
  next activation; the dogfood settings below stop that.
- **Dogfood note in the docs:** on a home-manager machine before
  `look-nix-catalog`, set `programs.scoot.desktop.look = null`,
  `programs.scoot.stylix.enable = false` and `programs.scootbar.stylix.enable
  = false`, or the Nix-written values in the user's own files shadow every
  look; `scoot look current` lists anything still shadowing. Say what that
  costs: with `desktop.look = null` the profile's theme slot is inert too
  (`themeOn = theme.enable && look != null`, `nix/modules/theme-home.nix:37`),
  so GTK, Qt, fontconfig and foot lose their theming while dogfooding,
  until `look-nix-catalog`.
- Docs in the same PR: a new site page for looks (pick, apply, what is live
  and what is next-launch), and the generated CLI pages.

## Evidence to record

Tests: atomicity (a reader racing a swap sees the old or the new directory),
refuse-and-keep, rollback and rollback-with-errors, the lock (two applies
serialize), the link repaired from `look.json` after a simulated crash, a
missing bar skipped, a shadowed key reported, a pending user edit not applied
by a swap. Benchmarks on headless and `--tty`: swap wall time, frame times
across a swap, zero new per-frame allocations after it. The smoke test
applies two looks and checks the background pixel of each.

**Every test and the smoke run set their own `XDG_STATE_HOME`,
`XDG_RUNTIME_DIR` and `XDG_CACHE_HOME`** (R3): until `look-session-state`
lands, a headless session shares the user's state, so a test run on a dev
box without private dirs would repoint the developer's real look. A
`color-scheme` test also runs against a private dconf profile (or a fake
`dconf` on `PATH`), never the developer's session bus.

## Not in this ticket

`next`/`prev`/`random`/`--for` (`look-cycle-preview`), IPC, Nix,
third-party apps, authoring.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
