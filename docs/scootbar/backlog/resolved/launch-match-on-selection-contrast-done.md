---
title: "launcher match highlight unreadable on the selection row in ginger-night"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-21"
---

# launcher match highlight unreadable on the selection row in ginger-night

Filed 2026-10-21. Serves **daily-drive**: the launcher's selected row is
the row the user is about to launch, and its matched characters must
stay legible.

## The gap

`crates/scootui/src/entry/paint.rs` (`paint_rows`) draws match spans in
`theme.urgent` on the selected row and `theme.hover` elsewhere, while
`crates/scootlaunch/src/panel.rs` (`apply_colors`) maps the
per-invocation `--match-color` onto `theme.hover` only. So the selected
row's match color is always the look's `urgent`, never the match role:
in ginger-night (`looks/ginger-night/look.toml`) that is urgent
`#E05A4E` (salmon) on selection accent `#E57F29` (orange), roughly
1.3:1, unreadable. Evidence: the `docs/assets/full-look.png` take with
query `ter` (PR #707 round 1) shows `Te` of `Terminal` as a pink smear
on the orange selection bar; queries topping Firefox (`fire`) or
Alacritty (`a`) garble the same way, since any name-matched selection
takes the urgent ink.

## What to do

Decide where the selected-row match ink comes from (a dedicated
`match-on-selection` role vs reusing `selection-text` for matched spans
on the selected row vs mapping `--match-color` onto both `hover` and
`urgent`), then check every shipped look's selected-row match contrast
and extend the look render tests (`launch-look-selected-row-contrast`
covers selection vs row background, not match vs selection). Pin:
dmenu mode shares `paint_rows`; `--match-color` passthrough must keep
working for unselected rows.

## Not in this ticket

The README hero workaround (PR #707 picks query `em`, whose top hit
Mail matches through keywords with no name spans, so the selected row
is plain selection-text). Changing ginger-night's urgent or accent
hues.

## Resolution

Resolved 2026-10-21. The selected row no longer inks matches in
`urgent`: `scootui`'s `paint_rows` resolves the selected-row match ink
through `Theme::match_on_selection`, measured against the actual theme
roles — the `hover` ink when it contrasts at least 3:1 with the
selection fill (so a readable `--match-color` keeps meaning what its
name says on both rows), else the inverted chip (row-text ink behind,
`accent` over it), else black or white ink. Measured: `urgent` on
`accent` was 1.29:1 in ginger-night (1.29-2.33 everywhere shipped);
`hover` on `accent` is 1.0-1.61 everywhere shipped, so every shipped
look resolves to the inverted chip (row-text on `accent` is 6.2-12.9).
Pinned by `match_on_selection_*` unit tests over all five shipped
palettes plus the built-in default, and the re-pinned
`golden_list_selection_highlights` (no `urgent` in the selected row).
Dmenu mode shares the path, so it gains the same fix. Docs in
`site/.../scootlaunch/{panel,cli}.md`, the `look.rs` role table and
`panel.rs apply_colors`.
