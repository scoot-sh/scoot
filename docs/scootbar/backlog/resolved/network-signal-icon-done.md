---
title: "Network: WiFi signal as a strength icon, not text bars"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
---

# Network: WiFi signal as a strength icon, not text bars

Filed 2026-10-03. Serves **daily-drive**: the bar's WiFi indicator should
look like an icon, not a ragged staircase of text in a proportional font.

## The gap

The maintainer, looking at real screenshots of the bar (2026-10-03): "The
Wi-Fi stairstep thing is super ugly." The network module appends text bars
(`Wimbly ▂▄▆█`) for WiFi signal; in a proportional font they render as a
ragged staircase. Decision: **the icon shows the strength; the text is just
the SSID.**

Evidence: `crates/scootbar/src/modules/network/mod.rs` `write_text`
(appends `BAR_GLYPHS[..bars]`), `crates/scootbar/src/modules/network/netlink.rs`
`bars_for`/`BAR_GLYPHS`, popup rows in `mod.rs` `popup` (`{line} ▂▄▆█`),
`docs/scootbar/cli.md` Network section (the bars sentence, `show-ssid`).

## What to do

1. `icon-wifi` takes four glyphs as well as one:
   `icon-wifi = ["\U000f091f", "\U000f0922", "\U000f0925", "\U000f0928"]`
   (weakest to strongest; Nerd Font wifi-strength-1..4) picks the glyph by
   the existing `bars_for(signal)` level (1 to 4). A single string keeps
   working exactly as today (one glyph for every level). Validate loudly:
   an array must have exactly 4 entries, each one glyph, as the other icon
   keys validate theirs.
2. The text bars go. WiFi text is just the SSID (`Wimbly`), and
   `show-ssid = false` shows `WiFi`. Strength stays in the tooltip
   (`Wimbly · −49 dBm on wlan0`, unchanged) and in `query` (`bars` and
   `signal` unchanged). With no icon configured at all, the bar shows the
   SSID alone.
3. The popup list (`on-click = "popup"`): rows lose the text bars too. If
   `icon-wifi` is a 4-glyph array, each row starts with its network's
   strength glyph (the popup's text rows draw with the bar's fonts, so a
   glyph from the fallback font must render there: check, and if the popup
   cannot draw a fallback-font glyph, leave rows as plain SSIDs). With a
   single glyph or none, rows are plain SSIDs.
4. Everything else (`show-text = false`, ethernet/VPN/offline icons,
   `icon-path`/`icon-image` static icons) unchanged. If `icon-path` or
   `icon-image` is set, it stays static: per-level SVG/PNG is out of scope.

Pin in tests: level selection at each `bars_for` boundary, the
single-glyph case unchanged, a bad array refused naming the key,
`show-ssid = false`, popup rows with and without the array, `query`
unchanged. Update every test and doc example showing `▂▄▆█`/`▂▄▆`.

## Not in this ticket

Per-level SVG/PNG icons; nl80211/VPN detection changes; layout beyond the
icon swap.

## Resolution (2026-10-03, PR #408)

Landed as `feat(scootbar): WiFi signal as a strength icon, not text bars`.
`icon-wifi` takes one glyph or four (`WifiIcon::One`/`Levels`, weakest to
strongest, picked by `bars_for`); a 4-array with the wrong length or a bad
entry is refused naming `network.icon-wifi`. WiFi text is the SSID alone
(`WiFi` with `show-ssid = false`); tooltip and `query` (`bars`, `signal`)
unchanged; `netlink::BAR_GLYPHS` removed. Popup rows start with the
strength glyph with a 4-glyph `icon-wifi` (verified live: the popup draws
with the bar's fonts, so the fallback Nerd Font glyph renders), else plain
SSIDs. Static `icon-path`/`icon-image` stays static.

Evidence: new tests fail-before proven on the dev VM (5 tests fail against
the text-bars behavior, 6 pass with the fix); `nextest -p scootbar` 1250
passed + `cargo test -p scootbar` 21 suites clean on the Asahi M2 with
`SCOOTBAR_REQUIRE_SCOOT=1 SCOOTBAR_REQUIRE_DBUS_DAEMON=1` (sway absent
there; sway-path popup/tooltip integration 31 passed on the dev VM);
clippy matrix clean (default, none, each module alone, popup+each,
network+each, all-features); live on real `wlan0` (−55 dBm, level 4):
`wsig-icon.png` (strength glyph + SSID), `wsig-ssid-only.png` (SSID alone),
`wsig-popup.png` (glyph row, selected). Ratchet: release `.text`
1,595,848 → 1,595,792 (−56 B), file size unchanged; idle RSS 5936 vs
5872–5888 kB and wakeups ~10–16/20 s both before and after (same band:
the 10 s signal timer plus the box's netlink chatter).
