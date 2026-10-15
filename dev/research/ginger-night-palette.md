# Ginger-night palette sampling (dev note, unpublished)

How the hex values in `docs/examples/ginger-night/README.md` were derived
from `docs/assets/CatPeeking.png` (1672x941), so the method can be
reproduced or re-checked. User-facing page keeps the median method and
the R/G/B thresholds; this note keeps the reproducibility evidence.

Method: a stdlib PNG decoder (zlib + unfilter, no image library), not
eyedroppered.

Per role:

- `surface` near-black `#0E0E0E`: the field's center pixel (800,470);
  the 15 most common colors all sit in `#0C0C0C`–`#0F0E0E`.
- `ink` warm cream `#F5EAD6`: the two eye-catchlights (`#FFFFFF` at
  ~(1605,361) and ~(1506,418)), softened warm so body text does not
  glare on black.
- `accent` ginger `#E57F29`: the median of all 14,026 glyph pixels with
  R>150, 90≤G≤190, B<120 (mean `#DF8145`, median `#E57F29`).
- `ring` bright ginger `#FF9A30`: the most common glyph-stroke colors
  (`#FFA230`, `#FF972A`, …) — the stroke peaks the ring is drawn in.
- `glow` amber `#FFB14B`: a bright stroke variant from the same top-12
  list (hover, prompt success, highlights).
- `hush` dark umber `#4E2913`: the shadowed fur at (1500,470) (the
  inactive ring, entry fields).
- `sand` dusty sand `#C9A87A`: the pale-glyph mid-tones warmed (git
  branch, graph text).

Byte-size evidence: `wallpaper.png` is a byte-identical copy of
`docs/assets/CatPeeking.png` (926,588 bytes both; re-encoding the
pixels at zlib level 9 with sub/up/adaptive filtering all came out
larger: 927,754 / 985,838 / 933,955 bytes, so the source is already
optimal).
