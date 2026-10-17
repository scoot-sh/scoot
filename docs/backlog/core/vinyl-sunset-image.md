---
title: "Bundle the maintainer's vinyl-sunset image and retheme the look"
status: "open"
area: "core"
priority: "medium"
blocked: null
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
