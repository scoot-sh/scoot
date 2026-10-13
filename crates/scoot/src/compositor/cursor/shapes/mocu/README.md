# Mocu cursor art (vendored)

These SVG files are the source art for most of scoot's built-in cursor
shapes. They come unchanged from **Mocu**, "a modest xcursor theme" by
Severin Meyer:

- Repository: <https://github.com/sevmeyer/mocu-xcursor>
- Commit: `efdd71279b79bf1e2566c38becc5e7c4ab2d900c` (release 1.1,
  2022-02-02), files from `src/svg/`
- License: CC0 1.0 Universal (public domain dedication), reproduced in
  [`LICENSE.txt`](LICENSE.txt). CC0 asks for no attribution; scoot gives it
  anyway, here and in the repository's `NOTICE`.

## How scoot uses them

scoot does not render these files and does not ship them in the binary.
The `d` path strings and hotspots of the shapes it uses are copied
verbatim into `../mocu.rs`, and a unit test checks that each copied string
still appears in its file here, so the two cannot drift apart. At startup
scoot flattens those paths at the bitmap's own pixel size and fills them
with its own rasterizer, painting them in the configured
`cursor_color`, its contrasting rim and the ginger `cursor_accent`. The
accent marks (the arrow's tail tip, the resize shafts, the hand's cuff and
so on) are scoot's own additions, clipped from Mocu's outlines.

Mocu's three placeholder colors map to scoot's paints: the fill
(`#fafbfc`) is `cursor_color`, the stroke (`#1a1b1c`) is the auto-contrast
rim, and the 10% drop shadow (`#0a0b0c`) is not drawn.

Shapes Mocu has no art for (the paw-print busy cursor and the paw badge on
the progress pointer) are drawn by `../paw.rs` in scoot's own geometry.

To update: copy the new `src/svg/*.svg` files over these, update the
commit above, and copy any changed `d` strings into `../mocu.rs`. The
provenance test names any string that no longer matches.
