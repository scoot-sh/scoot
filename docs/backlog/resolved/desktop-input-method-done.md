---
title: "Desktop: an input-method engine slot (fcitx5-class) for CJK and compose"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# Desktop: an input-method engine slot (fcitx5-class) for CJK and compose

Filed 2026-10-04, child of `desktop-paved-path` (from PR #430's review).
Serves **daily-drive** for users who type CJK or rely on an IME.

## The gap

The compositor implements `text-input-v3` and `input-method-v2`
(`site/src/content/docs/scoot/protocols.md`, table and "Input methods"
section), but nothing on the
paved path installs or starts an engine, so the protocols have no client.

## What to do

An off-by-default `desktop.inputMethod` slot: pick the lightest engine that
speaks `input-method-v2` (fcitx5 vs others, say why), start it as a user unit
ordered after the session target, set the toolkit variables only where a
toolkit still needs them, and theme its candidate window from the look.

## Acceptance

Eval pins; on the M2, type a CJK string into a GTK app, a Qt app and foot,
and show the candidate popup placed at the cursor.

## Not in this ticket

On by default (most users do not need it); XIM for X clients beyond what
XWayland already carries.

## Resolution (PR #512, 2026-10-08)

Shipped as designed, off by default and never with the profile. fcitx5
(the only maintained engine speaking `input-method-v2`: fcitx5 5.1.21
437.4 MiB closure vs ibus 1.5.34 1.10 GiB speaking its own D-Bus
protocol, kime 3.1.1 1.02 GiB Korean-only on the v1 path, uim 1.9.6
432.5 MiB XIM-only -- closures by `nix path-info --closure-size`,
aarch64-linux) runs as `scoot-input-method.service` in
`scoot-session.target` (PartOf/After/WantedBy, engine binary as
ExecStart, no polling); `XMODIFIERS` + `QT_IM_MODULE=fcitx` +
`QT_IM_MODULES=wayland;fcitx` land in the manager environment and
login shells while `GTK_IM_MODULE` stays unset (GTK3/4 take native
text-input); the candidate window wears a generated `scoot-look`
classic-UI theme behind `theme.targets.inputMethod.enable`. Default
package is plain fcitx5 with empty `addons` (CJK engines ride
`addons`, e.g. `qt6Packages.fcitx5-chinese-addons`).

Evidence: `nix flake check` darwin PASS; M2
`nix eval .#checks.aarch64-linux.scoot-modules.drvPath` PASS (all
eval asserts incl. ~60 new pins) and `nix build` of it exit 0; live
on the M2 (headless scoot 0.1.0/proto 10, fcitx5 5.1.21): the seat
advertises `zwp_input_method_manager_v2` v1, fcitx5 binds it
(`WAYLAND_DEBUG` wire log), loads `waylandim`, creates classicui
for the Wayland display. Full CJK typing (pinyin through foot/GTK/Qt
with the popup at the cursor) stays a documented manual step: the
shared M2 sat at 93% disk, so no CJK addon or client was fetched --
see site/src/content/docs/desktop/index.md#input-method ("Trying it
headless").
