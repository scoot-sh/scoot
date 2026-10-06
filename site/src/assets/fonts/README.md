# Display face: League Spartan 900

`league-spartan-latin-900.woff2` (12,648 bytes) is the display face for
the docs site's wordmark, hero and headings (see `site/OUTLINE.md`,
brand decision).

- Face: **League Spartan** Black (weight 900), by Micah Rich and
  Tyler Finck (The League Spartan Project Authors).
- License: **SIL Open Font License 1.1** (https://openfontlicense.org;
  license stub in the font's name table points at
  https://scripts.sil.org/OFL).
- Source: Google Fonts (`fonts.google.com/specimen/League+Spartan`),
  self-hosted here so the site makes no third-party request.
- Subset: the `latin` unicode-range file Google Fonts serves
  (`U+0000-00FF, U+0131, U+0152-0153, ...`), covering the wordmark,
  headings and hero copy. Re-download the same subset if the file ever
  needs replacing; do not vendor the full character set.
- Why this face: compared side by side against the logo wordmark
  (`docs/assets/logo.png`) at matched t-height with League Spartan 900,
  Montserrat 900, Poppins 900, Outfit 900, Urbanist 900, Lexend 900 and
  the previous Archivo Black (see `brand.font-compare.png` beside the
  brand reports). League Spartan is the only geometric that matches the
  logo's short `t` with its flat angled terminal, with the most circular
  `o` counter (0.917 vs 1.02 measured on the logo) and a wide `s`
  (0.853 of the `o` advance vs 0.815 on the logo) to go with it.
