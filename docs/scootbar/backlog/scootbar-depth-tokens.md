---
title: "Shadow and gradient fill tokens for the bar and its popups"
status: "open"
area: "scootbar"
priority: "low"
blocked: "bar-opacity-benchmark"
---

# Shadow and gradient fill tokens for the bar and its popups

Filed 2026-10-31, child of GitHub epic
[#807](https://github.com/scoot-sh/scoot/issues/807) (section 4,
"scootbar (optional, later)"). Serves **daily-drive**: depth is what
lifts the bar and its popups off the windows behind them, reusing the
primitives scootnotify and scootlaunch already proved.

Depends on: #807 (the epic), `bar-opacity-benchmark` (this ticket's
gate — no shadow or fill work lands before the opacity numbers exist),
and the sibling `scootbar-popup-entrance` (which waits on this one for
shadow room).

## The gap

`crates/scootui` already has the shared primitives #807 section 1 asked
for, and both shell apps read them through the same look tokens:

- Soft drop shadow under a rounded rect, cached per size:
  `crates/scootui/src/paint/depth.rs` (`ShadowSpec`, `Shadow::new`,
  `Shadow::prepare` builds the tables outside any repaint,
  `Shadow::paint`; blur clamped to `MAX_BLUR = 128`). Shipped for
  notification cards in #810 and the launcher panel in #814.
- Two-stop vertical gradient fills (`FillStyle` in
  `crates/scootui/src/effects.rs`, `fill = "gradient"`), shipped in the
  same PRs.
- The look schema: the five depth and motion tokens in `[effects]`
  (`crates/scoot-look/src/allowlist.rs:194-204`, `EFFECTS_TABLE_KEYS`:
  `shadow`, `fill`, `motion`, `motion-duration-ms`, `motion-bounce`),
  with per-app overrides through `[apps.<app>]` dotted keys
  (`user_effect_key`, `allowlist.rs:208-229`). Two shipped looks already
  use them (`looks/ginger-night/look.toml:39-40`: `shadow = "soft"`;
  `looks/radial-burst/look.toml:41-46`: `shadow = "glow"`, spring
  motion, plus `[apps.scootlaunch]` and `[apps.scootnotify]` overrides).

scootbar reads none of it. The allowlist is explicit about why:
`reads_depth` (`allowlist.rs:231-236`) matches only `"scootnotify" |
"scootlaunch"`, so `user_effect_key("scootbar", "effects", "shadow")`
returns `None`, and `allows("scootbar", …)` (`allowlist.rs:238-249`)
accepts only `SCOOTBAR_ALLOWLIST` plus no depth keys. The bar's look
layer (`crates/scootbar/src/look.rs`, allowlisted keys enumerated in
`docs/scootbar/backlog/resolved/look-layer-scootbar-done.md:35-42`)
covers colors, radius, opacity, fonts, padding, spacing, separator and
the module glyph keys — no shadow, no fill. Defaults stay as they are
(no shadow, no gradient) unless a look opts in, so existing goldens
hold; `shadow = "none"` costs nothing (nothing is built or painted —
`crates/scootui/src/effects.rs:12-16`).

## What to do

1. **Read the depth tokens in scootbar** through `[effects]` with
   per-app overrides under `[apps.scootbar]`, reusing the scootui
   primitives (cached shadow tables per size via `Shadow::prepare`;
   `shadow = "none"` builds nothing). Extend `reads_depth`,
   `NOTIFY_DEPTH_KEYS`/`LAUNCH_DEPTH_KEYS` (or the scootbar equivalent),
   `user_effect_key` and `allows` in
   `crates/scoot-look/src/allowlist.rs` so the bar's user-file layer and
   look layer share the look's allowlist, never a copy of it. Unknown
   or refused tokens behave like today: reported and ignored, never
   fatal (#807 Constraints).
2. **Bar surface geometry.** The bar is a layer-shell exclusive-zone
   surface (`crates/scootbar/src/daemon/surfaces.rs:175`
   `set_exclusive_zone`). Pin down where a shadow draws: the bar has no
   margin to bleed into by default, so the ticket must answer room
   (reserve margin for the shadow or clip it), the input region (it must
   cover what is drawn and nothing more, the way #835 made the
   notification cards' region follow their rounded/pill outline —
   `crates/scootnotify/src/popup/region.rs`,
   `resolved/notify-input-region-follows-shape-done.md`), the exclusive
   zone interaction (the zone sizes clients' windows; the shadow must
   not push windows around), and per-edge placement (top and bottom
   bars shadow opposite sides). Cover both edges explicitly.
3. **Popup surfaces.** The popups (tray menu, network list, volume
   slider, power menu, looks picker — the five `popup()` implementers
   in `crates/scootbar/src/modules/`: `tray/mod.rs:665`,
   `network/mod.rs:2348`, `volume/mod.rs:1200`, `power/mod.rs:511`,
   `looks/mod.rs:377`, against the trait default in `mod.rs:217-230`)
   get the same shadow and fill tokens. Popups are their own
   layer-shell surfaces, so their shadows have room the bar lacks; say
   how much room each side gets and how it interacts with the popup
   placement (`crates/scootui/src/popup/`).
4. **Scale handling.** scootbar already follows the compositor's
   preferred scale (`daemon/surfaces.rs:23-24,707`: fractional scale
   with viewporter, else `preferred_buffer_scale`, else output scale —
   the same stack #839/#841 just fixed for scootlaunch/scootnotify).
   Shadow tables are per device-pixel size: rebuild them when the scale
   changes, cache them otherwise, and cover 1x and 2x in the bench.
5. **Validator, schema and docs.** Touch the look validator, the
   allowlist and the schema in `crates/scoot-look` together (they are
   checked against each other — see `allowlist/tests.rs:484-537`), the
   site look-format reference
   (`site/src/content/docs/reference/look-format.md`), and the scootbar
   config docs (`site/src/content/docs/scootbar/configure.md`,
   `theming.md`), in the same PR per the `CLAUDE.md` docs rule.

## Acceptance

- `shadow` (`none`/`soft`/`deep`/`glow`) and `fill` (solid/gradient)
  tokens reach the bar and all five popup surfaces from `[effects]` and
  `[apps.scootbar]`, reusing `scootui`'s cached tables (no per-frame
  recompute; `shadow = "none"` measurably costs nothing against the
  `bench_depth` pattern).
- Exclusive-zone, input-region and per-edge (top/bottom) behavior pinned
  by tests: the shadow never moves a client's window, and the input
  region covers the drawn shape.
- At least one shipped look opts in (the way ginger-night takes `soft`
  and radial-burst takes `glow` for notify), with golden coverage for
  the new styles and defaults unchanged everywhere else.
- The validator/allowlist/schema tests, the site reference and the
  scootbar config docs all updated in the same PR.

## Test plan

- Golden/fingerprint tests unchanged at defaults (defaults stay
  shadowless and solid); new goldens for each shadow style and the
  gradient fill.
- A live test on headless scoot with the bar top and bottom, plus a
  CPU-tier screenshot of the bar and one open popup at 1x and 2x.
- Warm-frame allocation check on the bar repaint path (no per-frame
  allocation once warm, tables cached per size).

## Not in this ticket

The opacity benchmark itself (`bar-opacity-benchmark` decides whether
translucency is affordable) and the popup entrance motion
(`scootbar-popup-entrance` reuses the room this ticket reserves). No
changes to which modules have popups.
