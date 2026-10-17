# The shipped-looks agreement check: each `looks/<name>/look.toml`
# agrees with its example files (`docs/examples/<name>/`), over the
# fields both have (the style tokens and the foot hues). The look
# directories are the single source of truth -- the Nix side reads the
# same manifests at evaluation (see `nix/lib/look-shipped.nix`) -- so
# there is no registry counterpart left to compare against.
#
# Pure evaluation (file reads, `fromTOML`): no derivation output is
# read, so this runs on every platform, including the FlakeHub
# `--all-systems` eval gate. The cursor and icon theme come from the
# same helper the theme itself uses (`look-shipped.nix`), never a
# pasted copy; `tests.nix` forces the returned pins at eval, beside
# `_themePins`.

{ lib }:

let
  lookShipped = import ./lib/look-shipped.nix { inherit lib; };

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
      wall = t.wallpaper or { };
      image = wall.image or null;
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
      # The cursor and icon theme the theme helper gives every look.
      (
        assert t.cursor.theme == lookShipped.cursor.name;
        true
      )
      (
        assert t.cursor.size == lookShipped.cursor.size;
        true
      )
      (
        assert (if (t.icons or null) == null then "Adwaita" else t.icons.theme) == lookShipped.iconTheme;
        true
      )
    ]
    ++ (
      if image == null then
        [
          # vinyl-sunset: the illustration cannot be committed, so no
          # image on either side; the look shows the flat background.
          (
            assert (wall.mode or null) == null;
            true
          )
          (
            assert (wall.color or null) == t.palette.background;
            true
          )
        ]
      else
        [
          # The image lives in the look directory, beside its manifest.
          (
            assert image == "wallpaper.png";
            true
          )
          (
            assert builtins.pathExists (../looks/${name} + "/${image}");
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
lib.concatMap pinsFor lookShipped.shippedNames
