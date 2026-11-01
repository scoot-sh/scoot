---
title: "Shadow and gradient fill tokens for the bar and its popups"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-01"
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

## Not in this ticket

The opacity benchmark itself (`bar-opacity-benchmark` decides whether
translucency is affordable) and the popup entrance motion
(`scootbar-popup-entrance` reuses the room this ticket reserves). No
changes to which modules have popups.

## Resolution (2026-11-01, verdict: PASS)

Depth reaches the bar and all five popups from `[effects]` and
`[apps.scootbar]`, reusing `scootui`'s cached tables. Defaults stay
shadowless and solid, so existing goldens hold; new goldens cover each
shadow style and the gradient.

### Where the bar's shadow draws (room vs clip, with evidence)

- The bar is an exclusive-zone layer surface (`daemon/surfaces.rs:175`
  `set_exclusive_zone`). It has no margin to bleed into by default, so
  the surface reserves only the window-facing room: bottom for a top bar,
  top for a bottom bar (`Style::shadow_pad`, `Style::surface_height`).
  The anchored edge and both sides clip (width never grows).
- The exclusive zone stays the bar's own height
  (`Bar::exclusive_zone`, pinned by `bar.rs`'s zone test and by
  `render::tests::a_shadowed_bar_blends_and_keeps_its_zone`): a shadow
  overlaps windows instead of moving them. Top bars shadow below
  (`soft` `(0,4)` down), bottom bars mirror to above (`(0,-4)` up);
  `glow` (zero offset) is the same either way. Both edges explicitly
  pinned by the same test (soft/deep/glow x top/bottom pads).
- The input region is the bar's own rounded shape at its offset, never
  its shadow (`daemon/canvas.rs:set_input_region_bar`, keyed by
  `(surface, bar_y, bar_h, round)`): clicks in the shadow or in a cut
  corner fall through to the windows below, the way #835 made
  notification cards' region follow their outline. A square shadowless
  bar keeps the default (the whole surface, which is the bar); a square
  shadowed bar gets its own rect explicitly, so the shadow stays
  click-through.
- The bar needs an alpha channel when shadowed, rounded or translucent
  (`Style::translucent`), and blends everywhere then
  (`Style::opaque_inset` returns `None` for any shadowed bar, pinned by
  the same test).

### Popup surfaces' room and placement

- The five `popup()` implementers (`tray/mod.rs:665`,
  `network/mod.rs:2348`, `volume/mod.rs:1200`, `power/mod.rs:511`,
  `looks/mod.rs:377`, against `mod.rs:217-230`) get the same shadow and
  fill tokens. Popups are their own `xdg_popup` layer surfaces, so they
  have the room the bar lacks: the surface is the layout plus the
  shadow's pad on all four sides (`Shadow::new(dev_spec).pad()`), the
  popup sits at `(pad.left, pad.top)`, and the positioner is told the
  whole surface, so the compositor places it with its shadow
  (`constraint_adjustment` SlideX/FlipY still applies).
- Input is the popup's own rounded rect at its origin
  (`set_popup_input_region`), never its shadow; opaque is `None` when
  shadowed, rounded, translucent or gradient (an `ARGB8888` buffer),
  otherwise the previous cross/default stands, so tooltips (no shadow,
  solid, opaque, at the origin) paint exactly as before and their
  goldens hold.
- Scale handling reuses the notifier's pattern: logical specs scale to
  device (`dev_shadow`, blur via `device()`, offsets via
  `Scale::to_device`), tables built once on open via
  `Shadow::prepare(radius, [(w,h)])` and read every redraw, rebuilt only
  when the scale, size, radius, edge or style changes (cached otherwise;
  `shadow = "none"` builds nothing). The bar's `Scene` caches the same
  way (`shadow_for` keyed by scale, size, radius and edge).

### What was built

- `scoot-look`: `BAR_DEPTH_KEYS = ["bar.shadow", "bar.fill"]`,
  `reads_depth("scootbar")`, `user_effect_key("scootbar","effects",…)`
  mapping to `bar.*`, `allows` + `value_kind` for the two keys,
  `render_bar` emitting them only when the look sets them (validated
  twice, as the shell fragments are). Validator/allowlist/schema/tests
  together (`allowlist/tests.rs`, `render/tests.rs`, `tests/shipped.rs`).
- `scootbar`: `[bar] shadow`/`fill` plus a user `[effects]` table winning
  per key over `[bar]` and the look layer, both through the look's
  allowlist (`check_app_value` + `user_effect_key`, never a copy);
  `Style` carrying the two tokens, gradient stops, per-edge pads and
  surface heights; `Scene` shadow cache; `render::paint` with origin
  offsets, `Canvas::window`/`fill_rounded_gradient`, and gradient-forces-
  whole (solid spans stay damage-limited); daemon geometry (surface vs
  zone, input, opaque, restyle recreate on pad change); popup
  `paint_depth` with origin/shadow/fill/opacity and pool format gating.
- Goldens: `bar-shadow-soft/deep/glow-1x.pgm`, `bar-gradient-1x.pgm`;
  defaults unchanged (all 1086 bin tests pass, including the 21
  snapshots).
- Docs: `site/.../reference/look-format.md` (bar reads depth, per-app
  table), `scootbar/configure.md` (Shape, opacity and depth: room,
  zone, input, popups, scales, bench), `theming.md` (layer carries
  `shadow`/`fill`; ginger-night soft, radial-burst glow).
- Bench + allocation: `render/tests/bench.rs:depth_repaint_cost`
  (bench_depth pattern, 1x and 2x, 6 combos, full+span, best of 2) and
  `warm_frames_allocate_nothing_once_tables_are_cached`
  (`scootbg_mem::count_allocations`, 0 on 10 warm full repaints per
  combo/scale; `none` builds no tables).
- Screenshots (CPU-tier, headless scoot, dev VM): bar top/bottom and one
  open (power) popup at 1x and 2x, copied to `/tmp/depth-shots/` on the
  Mac (see below).

### How it was measured

Worktree `/tmp/wt-depth` on `feat/scootbar-depth-tokens` from
`origin/main` `51b16eb11` (uncommitted when measured; VM tree
`/dev/shm/depth/tree` tarred from it, `COPYFILE_DISABLE=1`, own
`CARGO_TARGET_DIR=/dev/shm/depth/target`, `-j2`, scratch deleted
after). `cargo fmt` on the Mac; no VM restarts.

- `cargo test -p scoot-look` (Mac): 226 passed.
- `cargo nextest run -p scoot-look` (VM): 250 passed.
- `cargo clippy -p scootbar --all-targets -- -D warnings` (VM): clean.
- `cargo nextest run -p scootbar --bin scootbar` (VM): 1086 passed,
  1 skipped (including 4 new goldens + `no_snapshot_is_left_over`).
- `cargo test -p scootbar --bin scootbar warm_frames…` (VM): pass (0
  allocations on 10 warm full repaints per combo/scale).
- `SCOOTBAR_BLESS=1 cargo test -p scootbar --bin scootbar -- snapshots::tests::bar_shadow… snapshots::tests::bar_gradient…`
  (VM): blessed the 4 new `.pgm` files (sizes: soft 160x36, deep
  160x58, glow 160x36, gradient 160x20 at 1x for a 160x20 bar).
- `cargo test -p scootbar --bin scootbar --release -- --ignored --nocapture depth_repaint_cost`
  (VM, `taskset` left off: shared box, interleaved per round, best of 2,
  median+p5): raw lines below.
- Headless screenshots (VM, Nix-built `scoot`/`scootbar` 0.6.28 from the
  same tree, CPU tier, 800x600, DroidSansMNerdFontPropo-Regular,
  `[bar] height=38 radius=14 opacity=0.82 shadow=soft fill=gradient`,
  power popup): `bar-top-1x.png`, `bar-bottom-1x.png`, `popup-1x.png`,
  `bar-top-2x.png`, `bar-bottom-2x.png`, `popup-2x.png` (via
  `output-scale 1 2` for 2x, reopening the popup after the scale closes
  it) in `/tmp/depth-shots/` on the Mac.
- `python3 scripts/backlog check`: clean (after resolve).
- Site gates you can run: `nix build .#docs-site` not run here (site
  job runs it in CI); `cargo fmt --check` clean.

### Numbers (release, dev VM, best of 2 passes; surface px includes the shadow)

```
full 1x none-solid 69120 10.9 8.9
span 1x none-solid 69120 1.0 0.9
full 1x soft-solid 99840 64.9 49.2
span 1x soft-solid 99840 1.0 0.9
full 1x deep-solid 142080 116.8 100.5
span 1x deep-solid 142080 1.0 0.9
full 1x glow-solid 99840 44.0 38.0
span 1x glow-solid 99840 1.0 0.8
full 1x none-gradient 69120 10.6 9.1
span 1x none-gradient 69120 10.7 9.2
full 1x soft-gradient 99840 57.1 48.8
span 1x soft-gradient 99840 57.5 49.3
full 2x none-solid 276480 46.1 29.3
span 2x none-solid 276480 2.8 2.0
full 2x soft-solid 399360 299.5 185.5
span 2x soft-solid 399360 3.3 2.0
full 2x deep-solid 568320 617.2 394.8
span 2x deep-solid 568320 2.5 1.9
full 2x glow-solid 399360 262.8 140.4
span 2x glow-solid 399360 2.8 2.0
full 2x none-gradient 276480 58.0 29.7
span 2x none-gradient 276480 58.1 29.8
full 2x soft-gradient 399360 276.2 186.5
span 2x soft-gradient 399360 259.4 185.1
```

`shadow = "none"` costs nothing measurable against the bench pattern:
`none-solid` full 10.9/8.9 vs `soft-solid` span 1.0/0.9 (span never
paints the shadow; solid spans stay damage-limited, 1.0us at 1x at every
shadow). Gradient forces whole (span == full, e.g. 10.7 vs 10.6), as
designed and documented. Warm full repaints allocate 0 (all combos,
1x and 2x); `none` builds no tables (`scene.shadow.is_none()`).

Screenshots: `/tmp/depth-shots/bar-top-1x.png`,
`/tmp/depth-shots/bar-bottom-1x.png`,
`/tmp/depth-shots/popup-1x.png`,
`/tmp/depth-shots/bar-top-2x.png`,
`/tmp/depth-shots/bar-bottom-2x.png`,
`/tmp/depth-shots/popup-2x.png` (800x600 headless, CPU tier; popup shots
larger by the popup: 1x 19K vs 12K, 2x 31K vs 13K).

At least one shipped look opts in without a look change: ginger-night
already sets `[effects] shadow = "soft"`, which now renders
`bar.shadow = "soft"` (radial-burst's `glow` likewise). The screenshots
use the ginger-night-like translucent+shadow+gradient interplay
(`opacity 0.82`, `radius 14`, `soft`, `gradient`) to show it holds
together. The sibling `scootbar-popup-entrance` is unblocked (its shadow
room now exists).
