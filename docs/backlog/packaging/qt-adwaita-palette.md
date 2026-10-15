---
title: "Qt palette under the Adwaita style ignores the look (measured)"
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# Qt palette under the Adwaita style ignores the look (measured)

Filed 2026-10-15, from [`look-toolkit-live`](look-toolkit-live.md)
("first, settle the Qt question with a screenshot"). Serves
**daily-drive** (Qt apps match the look's palette, not just its dark/light
variant).

## The gap

With `style=Adwaita-Dark` in `qt6ct.conf` and a vivid custom scheme
(magenta window, cyan highlight, yellow text), a QWidget app resolves
Adwaita's fixed dark palette instead: window `#ff353535`, highlight
`#ff15539e`, text `#ffeeeeec`. The same app under `style=Fusion`
resolves the scheme exactly. Measured live on the dev VM against the
pinned versions (qtbase 6.11.2, qt6ct 0.11, adwaita-qt 1.4.2, PyQt6
6.11.0); screenshots in the `look-toolkit-live` report. Matches the
design's inference (adwaita-qt's `polish` replaces the palette after
the platform-theme palette lands). Upstream fixes are out of scope
(no upstream PRs from here).

## What to do

Decide what a look promises Qt widget apps: style variant, icons and
fonts follow the swap today; the exact palette does not under Adwaita.
Candidates: keep Adwaita and document the limit (current state);
follow up if adwaita-qt ever honors `custom_palette`; or ship a
generated QSS for QWidget apps (new renderer, new semantics to pin).
Also open: whether a per-swap scheme file plus `qt6ct.conf` rewrite
(which re-palettes running Fusion-style apps live, measured) is worth
the config-dir churn — `look-toolkit-live` deliberately ships the
stable through-the-link path instead.

## Evidence to record

Screenshots of a QWidget app under each candidate, with the resolved
palette roles named.

## Not in this ticket

The GNOME portal (accent-color); Qt on macOS.
