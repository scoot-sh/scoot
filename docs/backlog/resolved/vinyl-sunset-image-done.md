---
title: "Bundle the maintainer's vinyl-sunset image and retheme the look"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-17"
---

# Bundle the maintainer's vinyl-sunset image and retheme the look

Filed 2026-10-17. Serves **daily-drive** (vinyl-sunset becomes a fully
bundled look with its own wallpaper, like moonrise, instead of a
download-it-yourself example).

## The gap

`looks/vinyl-sunset/look.toml` ships only `[wallpaper] color` because the
Pixabay illustration cannot be distributed standalone
(`docs/examples/vinyl-sunset/README.md`, NOTICE). The maintainer generated
their own sunset-desk image, so the look can bundle a wallpaper and retheme
to it.

## What to do

Bundle the image as `looks/vinyl-sunset/wallpaper.png` and
`docs/assets/wallpapers/vinyl-sunset.png`, point the example at it, retheme
the palette and leaves to sampled colors, replace the NOTICE Pixabay section
with the maintainer-image entry, regenerate previews and goldens, update the
nix pins and site docs.

## Not in this ticket

Other looks, RENDER_VERSION (renderer output shape is unchanged).

## Resolution (2026-10-17, PR #647)

Shipped the maintainer's 1672x941 desk-sunset image as
`looks/vinyl-sunset/wallpaper.png` + identical
`docs/assets/wallpapers/vinyl-sunset.png` (lossless, 1777527 B; quantized
sub-1 MB banded visibly), rethemed look.toml + all example/apps leaves to
scripted samples (accent #FE9949, bg #270E0A; validator clean, dim/urgent
stop warning), replaced the NOTICE Pixabay section with the MIT
maintainer-image entry, recaptured previews + golden on the dev VM (viewed),
updated nix greeter/palette/launcher pins + smoke pixels + site tables.
Evidence: `scoot look validate vinyl-sunset` → valid; headless
`look-apply` serves the image via scootbg (screenshotted); nextest
scoot-look+scootctl 485 pass, scoot look-suites pass under nextest and
cargo test; nix eval of all four check drvPaths + darwin scoot-modules
build green; site `verify` green. One known failure is pre-existing and
untouched: `xwayland refused_cost` (open ticket
xwayland-refused-cost-fd-leak; no compositor code in this diff).
