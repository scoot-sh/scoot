---
title: "kunifiedpush distributor runs in minimal sessions at 15 MB PSS"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# kunifiedpush distributor runs in minimal sessions at 15 MB PSS

Filed 2026-10-05 from the five-desktop idle benchmark (Asahi M2,
`dev/benches/benchmarks.md`). Serves **daily-drive** (dead weight in every
session's footprint: push notifications nobody on this box uses).

## The gap

`kunifiedpush-distributor` runs in the scoot, niri and KDE sessions
(started through the xdg-desktop-portal stack) at ~15 MB PSS and
~22 MB RSS, zero CPU at idle (`ev/scoot-c1-delta.tsv` and siblings:
`pss0=14989`). It is the single biggest "plumbing" process after the
user manager in the scoot session — bigger than mako, the bar and the
idle daemon combined.

It arrives as a portal dependency (the distributor backs the Push
portal some backends advertise). Nothing in a minimal scoot session
sends or receives push.

## What to do

Find which portal backend pulls it in and whether the Push portal can
be masked out of the session's portal config without breaking the
backends the session needs (file chooser, screencast, Secret,
settings). Expected saving: ~15 MB PSS per session. If it cannot go,
say which backend hard-requires it. Check the other portal fellow
travelers while there (geoclue, `ibus` under the gtk portal in niri
sessions at ~60 MB — that one is niri's recommended stack, not ours,
so note it in the benchmark, not here).

## Not in this ticket

wireplumber/pipewire (~40 MB together): a deliberate system choice,
identical everywhere measured, not portal-pulled.

## Resolution (2026-10-08, docs-only: no packaging change, PR #538)

The ticket's portal-dependency premise does not hold: no portal backend
pulls in `kunifiedpush-distributor`, and the scoot session never
installed it. The ~15 MB in the benchmark sessions came from the
benchmark harness enabling Plasma 6 system-wide. Evidence (Asahi M2):

- No installed backend advertises the Push portal. `grep -ri
  "impl.portal.Push"` over every installed
  `share/xdg-desktop-portal/portals/*.portal` (gtk, gnome, wlr 0.8.4,
  hyprland, kde 6.7.5) returns nothing: none lists
  `org.freedesktop.impl.portal.Push` in `Interfaces=`.
- Neither backend the scoot session installs references it.
  `nix-store --query --references` on the xdg-desktop-portal-wlr-0.8.4
  and -gtk-1.15.3 store paths reports 0 kunifiedpush references each,
  and the session's portal selection (`resources/scoot-portals.conf`,
  `xdg.portal.config.scoot` in `nix/modules/nixos.nix`) routes only
  Screenshot/ScreenCast to `wlr` with everything else to `gtk` — there
  is no Push route to mask, and masking would save 0.
- The distributor starts via `graphical-session.target.wants`, not D-Bus
  activation: its unit (`share/systemd/user/kunifiedpush-distributor.service`,
  `WantedBy=graphical-session.target`,
  `BusName=org.unifiedpush.Distributor.kde`) runs in *every* graphical
  session once the package is installed. It is installed because nixpkgs'
  `services.desktopManager.plasma6` lists `kunifiedpush` in
  `requiredPackages` — unconditional, since
  `environment.plasma6.excludePackages` only filters `optionalPackages`
  (verified at this flake's nixpkgs rev `8ce4ef6c`,
  `nixos/modules/services/desktop-managers/plasma6.nix`). The benchmark
  wrapper (`~/fx/cmpde-cerval/cmpde.nix`) set
  `services.desktopManager.plasma6.enable = true` system-wide, so the
  scoot, niri and KDE sessions all carried it. Plasma also explains the
  fellow travelers: it sets `services.geoclue2.enable = mkDefault true`
  and installs the kde portal backend.
- Before (benchmark `ev/scoot-c*-delta.tsv`): `kunifiedpush-distributor`
  at pss0=14989–17568 (~15–17 MB PSS), ~31 MB RSS, 0–1 ticks per 60 s,
  load 0.00–1.31 at sampling. After (pure scoot profile, no Plasma
  system-wide): no such process — `ps -eo comm | grep -i kunified`
  empty on the live M2 — and neither portal backend closure references
  it. Removing it from a Plasma-enabled box reclaims the full ~15 MB
  PSS per session.
- Fellow travelers checked: no geoclue/ibus rows in any scoot-session
  TSV; the ~60 MB ibus stack appears only in niri sessions (niri's
  recommended stack, as the ticket expected — noted in the benchmark,
  not here).

Conservative remedy, trivially reversible: change nothing in
`programs.scoot` — it never installs the distributor. On a box that
also enables Plasma 6, `systemctl --user mask
kunifiedpush-distributor.service` stops it (`unmask` restores it);
per-user, so it covers that user's Plasma sessions too, where push
would stop with it. Push has no backend in the scoot closure, so
nothing the session needs breaks. Recorded on the site (desktop
screenshots section) with the benchmark lead corrected.
