# The shipped-looks agreement check (`docs/backlog/core/look-shipped.md`):
# each `looks/<name>/look.toml` agrees with its registry entry in
# `nix/modules/desktop.nix` until `look-nix-retire-registry`, over the
# fields both have (the 3 appearance and 6 bar colors, `isDark`/polarity,
# `fonts`, `nightTemp`, the wallpaper). `[shape]`, `[opacity]` and
# `[terminal]` have no registry counterpart and are compared with the
# example files instead (`docs/examples/<name>/scoot.toml`, `bar.toml`,
# `foot.ini`).
#
# Pure evaluation (file reads, `fromTOML`, one hash per image): no
# derivation output is read, so this runs on every platform, including
# the FlakeHub `--all-systems` eval gate. The registry comes through the
# same import the theme helper itself uses (`theme-look.nix`), never a
# pasted copy; the cursor and icon theme come from that helper too.
# `tests.nix` forces the returned pins at eval, beside `_themePins`.

{ lib }:

let
  themeLook = import ./modules/theme-look.nix { inherit lib; };
  registry = (import ./modules/desktop.nix { inherit lib; }).looks;

  toml = name: builtins.fromTOML (builtins.readFile ../looks/${name}/look.toml);
  exampleScoot =
    name: (builtins.fromTOML (builtins.readFile ../docs/examples/${name}/scoot.toml)).appearance;
  exampleBar = name: builtins.fromTOML (builtins.readFile ../docs/examples/${name}/bar.toml);

  # One foot hue out of the example's `foot.ini` (bare `key=rrggbb`
  # lines, always lowercase hex): the `[terminal]` table's source.
  footHue =
    name: key:
    let
      lines = lib.splitString "\n" (builtins.readFile ../docs/examples/${name}/foot.ini);
      pattern = "${key}=([0-9a-f]{6})";
      hits = lib.filter (line: builtins.match pattern line != null) lines;
    in
    assert builtins.length hits == 1;
    "#${builtins.head (builtins.match pattern (builtins.head hits))}";

  terminalKeys = [
    "regular0"
    "regular1"
    "regular2"
    "regular3"
    "regular4"
    "regular5"
    "regular6"
    "regular7"
    "bright0"
    "bright1"
    "bright2"
    "bright3"
    "bright4"
    "bright5"
    "bright6"
    "bright7"
  ];

  pinsFor =
    name:
    let
      t = toml name;
      reg = registry.${name};
      exRingInactive =
        (exampleScoot name).focus_ring_inactive_width or (exampleScoot name).focus_ring_width;
      exBar = exampleBar name;
      exInactive = exBar.workspaces."inactive-color" or null;
      exSeparator = exBar.bar.separator or null;
      barOverrides = t.apps.scootbar or { };
      lookInactive = barOverrides."workspaces.inactive-color" or null;
      lookSeparator = barOverrides."bar.separator" or null;
      hues = builtins.genList (i: footHue name (builtins.elemAt terminalKeys i)) 16;
    in
    [
      (
        assert t.schema == 1;
        true
      )
      (
        assert t.name == name;
        true
      )
      # The registry's values, no invented color.
      (
        assert t.palette.background == reg.appearance.background_color;
        true
      )
      (
        assert t.palette.focus == reg.appearance.focus_ring_active_color;
        true
      )
      (
        assert t.palette.border == reg.appearance.focus_ring_inactive_color;
        true
      )
      (
        assert t.palette.background == reg.barColors.background;
        true
      )
      (
        assert t.palette.foreground == reg.barColors.foreground;
        true
      )
      (
        assert t.palette.accent == reg.barColors.accent;
        true
      )
      (
        assert t.palette.dim == reg.barColors.dim;
        true
      )
      (
        assert t.palette.urgent == reg.barColors.urgent;
        true
      )
      # radial-burst names no hover: the look default (accent) reproduces it.
      (
        assert (t.palette.hover or t.palette.accent) == (reg.barColors.hover or reg.barColors.accent);
        true
      )
      (
        assert t.polarity == (if reg.isDark then "dark" else "light");
        true
      )
      (
        assert t.fonts.ui == reg.fonts.ui;
        true
      )
      (
        assert t.fonts.sans == reg.fonts.sans;
        true
      )
      (
        assert t.fonts.mono == reg.fonts.mono;
        true
      )
      (
        assert t.night.temperature == reg.nightTemp;
        true
      )
      # The cursor and icon theme the theme helper gives every look.
      (
        assert t.cursor.theme == themeLook.cursor.name;
        true
      )
      (
        assert t.cursor.size == themeLook.cursor.size;
        true
      )
      (
        assert (if (t.icons or null) == null then "Adwaita" else t.icons.theme) == themeLook.iconTheme;
        true
      )
    ]
    ++ (
      if reg.wallpaper == null then
        [
          # vinyl-sunset: the illustration cannot be committed, so no
          # image on either side; the look shows the flat background.
          (
            assert (t.wallpaper.image or null) == null;
            true
          )
          (
            assert (t.wallpaper.mode or null) == null;
            true
          )
          (
            assert t.wallpaper.color == t.palette.background;
            true
          )
        ]
      else
        [
          (
            assert t.wallpaper.image == "wallpaper.png";
            true
          )
          (
            assert t.wallpaper.mode == reg.wallpaper.mode;
            true
          )
          (
            assert
              (t.wallpaper.fill or t.palette.background) == (reg.wallpaper.fill or reg.barColors.background);
            true
          )
          # The shipped copy is the registry file, byte for byte.
          (
            assert
              builtins.hashFile "sha256" ../looks/${name}/wallpaper.png
              == builtins.hashFile "sha256" reg.wallpaper.image;
            true
          )
        ]
    )
    ++ [
      # Against the examples: the style values. The corner radius is the
      # example's (D15, raised once look-layer-scoot's pixman benchmark
      # passed: +5-6% per frame at 1, 10 and 50 windows); the bar opacity
      # stays at 1.0 until its blend cost is measured.
      (
        assert t.shape.radius == (exampleScoot name).corner_radius;
        true
      )
      (
        assert t.shape."ring-width" == (exampleScoot name).focus_ring_width;
        true
      )
      (
        assert (t.shape."ring-inactive-width" or t.shape."ring-width") == exRingInactive;
        true
      )
      (
        assert t.opacity.bar == 1.0;
        true
      )
      (
        assert barOverrides."bar.padding" == exBar.bar.padding;
        true
      )
      (
        assert barOverrides."bar.spacing" == exBar.bar.spacing;
        true
      )
      (
        assert lookSeparator == exSeparator;
        true
      )
      (
        assert lookInactive == exInactive;
        true
      )
      # foot's 16 hues, in ANSI order, lowercase as the files spell them.
      (
        assert t.terminal.colors == hues;
        true
      )
    ];

in
lib.concatMap pinsFor [
  "ginger-night"
  "moonrise"
  "music-desk"
  "radial-burst"
  "vinyl-sunset"
]
