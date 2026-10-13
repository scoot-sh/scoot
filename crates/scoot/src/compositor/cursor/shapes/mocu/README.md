# Mocu cursor art (vendored)

These SVG files are scoot's built-in cursors. They come unchanged from
**Mocu**, "a modest xcursor theme" by Severin Meyer:

- Repository: <https://github.com/sevmeyer/mocu-xcursor>
- Commit: `efdd71279b79bf1e2566c38becc5e7c4ab2d900c` (release 1.1,
  2022-02-02), files from `src/svg/`
- License: CC0 1.0 Universal (public domain dedication), reproduced in
  [`LICENSE.txt`](LICENSE.txt). CC0 asks for no attribution; scoot gives it
  anyway, here and in the repository's `NOTICE`.

## How scoot uses them

The files are compiled into scoot (`include_str!`) and read at startup by
`../mocu.rs`, which takes what Mocu's own build draws from each one: the
outlines, their stroke widths, the hotspot, the drop shadow and, for the two
busy cursors, the spinner mask (its first animation frame). scoot then fills
them with its own rasterizer at each output's pixel size, grid-fitting them
so the 16px default stays crisp, in Mocu's own paints: the fill (`#fafbfc` in
the sources) is `cursor_color`, Mocu's white `#fcfcfc` by default, and the
stroke (`#1a1b1c`) is Mocu's pairing for that fill (`#222222`, or `#f9f9f9`
around a dark fill, as in Mocu's black theme). Mocu's left-handed variants
are not used.

Every CSS cursor name draws the Mocu cursor of the same name, except `move`
(Mocu's own alias for it: `grabbing`) and `all-resize` (which Mocu does not
name: `all-scroll`).

To update: copy the new `src/svg/*.svg` files over these and update the
commit above. The tests read every file and fail on one that no longer
yields its outlines and hotspot.
