# The part of `programs.scootbar` that home-manager and NixOS share: the
# options, the Stylix defaults, the rendered `bar.toml` and the package
# with its Cargo features. `scootbar-home.nix` and `scootbar-nixos.nix`
# import it and add what only their side has (where the file goes, the
# systemd user unit). Written for docs/nix.md, "The status bar: scootbar".
#
# The precedence of a `settings` value, highest first:
#
#   1. what the user wrote (priority 100, the module system's default);
#   2. Stylix, when `config.lib.stylix` exists and `stylix.enable` is on
#      (`lib.mkDefault`, 1000);
#   3. this module's plain default (`lib.mkOptionDefault`, 1500): a font,
#      and only a font, since the bar refuses to start without one.
#
# Everything else the bar reads has its own default in the binary, so an
# absent key is that default. Each Stylix value is defined at its own
# leaf (`settings.colors.accent`, not `settings.colors`), which is what
# lets the user's `settings.colors.background` replace one token and keep
# the other four from Stylix: a priority applies to the whole value it
# wraps, so wrapping a table would make the user's one key discard it all.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scootbar;
  tomlFormat = pkgs.formats.toml { };

  # Stylix is not an input of this flake: its presence is
  # `config.lib.stylix` (defined by its palette module whether or not it is
  # enabled), and `stylix.enable` (false by default in Stylix itself) says
  # whether it is asked to theme anything.
  stylix = config.lib ? stylix && (config.stylix.enable or false);

  # Stylix hands over a font as a package and a family name, and scootbar
  # takes a file (there is no fontconfig). The file is found at build time
  # in the package's `share/fonts`: the regular face whose file name is the
  # family name, or that name plus `-Regular` ("DejaVu Sans" ->
  # DejaVuSans.ttf, "JetBrainsMono Nerd Font" ->
  # JetBrainsMonoNerdFont-Regular.ttf). Variable fonts (InterVariable.ttf),
  # collections (.ttc) and other naming (Ubuntu-R.ttf) are NOT resolved:
  # then the bar gets the plain default font (DejaVu Sans) instead of a
  # failed system build, and the derivation says so on stderr (in the build
  # log) and in `$out/warning` (the font itself is `$out/font`), naming the
  # family; set
  # `settings.bar.font` to a path to choose. A warning at evaluation time
  # (`lib.warn`) would need the font package built during evaluation
  # (import from derivation), so it is at build time. Only built when Stylix
  # is in use and the user has not set `bar.font`.
  fallbackFont = "${pkgs.dejavu_fonts.minimal}/share/fonts/truetype/DejaVuSans.ttf";
  fontFile =
    font:
    pkgs.runCommand "scootbar-font-${lib.strings.sanitizeDerivationName font.name}"
      {
        nativeBuildInputs = [ pkgs.findutils ];
        # Not `name`, which would rename the derivation.
        family = font.name;
        package = font.package;
        fallback = fallbackFont;
      }
      ''
        norm() { tr -d ' _-' | tr '[:upper:]' '[:lower:]'; }
        want=$(printf '%s' "$family" | norm)
        files=$(find -L "$package/share/fonts" -type f \( -iname '*.ttf' -o -iname '*.otf' \) 2>/dev/null | sort || true)
        pick=
        while IFS= read -r file; do
          [ -n "$file" ] || continue
          base=$(basename "$file"); base=''${base%.*}
          have=$(printf '%s' "$base" | norm)
          if [ "$have" = "$want" ] || [ "$have" = "''${want}regular" ]; then
            pick=$file
            break
          fi
        done <<<"$files"
        mkdir $out
        : > $out/warning
        if [ -z "$pick" ]; then
          msg="scootbar: no regular-face file named for \"$family\" in $package/share/fonts (variable fonts, .ttc and other naming are not resolved); using DejaVu Sans. Set programs.scootbar.settings.bar.font to a .ttf/.otf path to choose."
          echo "warning: $msg" >&2
          echo "$msg" > $out/warning
          pick=$fallback
        fi
        ln -s "$pick" $out/font
      '';

  # Stylix's size is in points; the bar's is logical pixels (the em). CSS
  # converts at 96 dpi, 1 pt = 4/3 px, and so do Stylix's own bar targets.
  # Integer division rounds to the nearest pixel; the bar takes 1 to 256.
  pixels = pt: lib.min 256 (lib.max 1 ((pt * 4 + 1) / 3));

  # base16: 00 background, 05 foreground, 0A yellow (the default accent,
  # Catppuccin's `f9e2af` in the bar's own palette), 03 comments (dim),
  # 08 red (urgent).
  palette = config.lib.stylix.colors.withHashtag;
in
{
  options.programs.scootbar = {
    enable = lib.mkEnableOption "scootbar, the status bar";

    # `pkgs.scootbar` with the flake's overlay, else null; the flake's
    # wrappers (`homeModules.scootbar`, `nixosModules.scootbar`) fill it
    # with the flake's own build. Null with `enable` is refused: the unit
    # would have nothing to run.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scootbar or null;
      defaultText = lib.literalExpression "pkgs.scootbar or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scootbar";
      description = ''
        The scootbar package. Its Cargo features are `features`, below;
        a `package` you build yourself is used as given when `features`
        is null.
      '';
    };

    # The modules are Cargo features (one per module, `clock` the default;
    # docs/nix.md). Null builds what `package` is; a list is exactly the
    # modules that build has, so `[ ]` is a plain bar that needs no font.
    features = lib.mkOption {
      type = lib.types.nullOr (lib.types.listOf lib.types.str);
      default = null;
      example = [
        "clock"
        "workspaces"
      ];
      description = ''
        The modules to build the bar with, as Cargo features: exactly the
        ones listed (`scootbar.override { buildNoDefaultFeatures = true;
        buildFeatures = features; }`), so an empty list is a bar with
        no modules. Null keeps `package` as it is (its default features).
        Needs a `package` that takes those overrides, as the flake's does.
        A module the layout names must be built in: the bar refuses to
        start otherwise, saying which are.
      '';
    };

    # Free-form: a new option never needs a module change first, and the
    # file's schema is docs/scootbar/cli.md's, checked by the bar itself
    # (an unknown key is a loud error naming it, and the check in this
    # flake runs the real binary over the rendered file).
    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = {
        bar = {
          height = 32;
          opacity = 0.9;
        };
        left = [ "workspaces" ];
        center = [ "clock" ];
        colors.accent = "#89b4fa";
        clock.format = "%H:%M";
      };
      description = ''
        The contents of `bar.toml`, rendered as TOML. The options are
        listed in docs/scootbar/cli.md ("The config file"). What is not
        set takes its value from Stylix, when `stylix.enable` is on, and
        then from this module's one plain default (a font).
      '';
    };

    stylix.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Take defaults from Stylix when it is in use (`config.lib.stylix`
        exists and `stylix.enable` is on): the five `colors` tokens from
        its base16 palette and `bar.font` and `bar.font-size` from its
        sans-serif font and desktop size. A value you set in `settings`
        always wins; this only turns the defaults off. Nothing here needs
        Stylix, and nothing changes without it.
      '';
    };

    # Read-only results the two sides use.
    finalPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      readOnly = true;
      internal = true;
      description = "`package` with `features` applied.";
    };
    configFile = lib.mkOption {
      type = lib.types.package;
      readOnly = true;
      internal = true;
      description = "The rendered `bar.toml`.";
    };
    systemd.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Run the bar as a systemd user service, `scootbar.service`, wanted
        by `graphical-session.target` and restarted when it fails
        (scoot does not supervise its clients). Turn it off to start
        `scootbar daemon` from your compositor's autostart instead.
      '';
    };
  };

  config = lib.mkMerge [
    {
      programs.scootbar = {
        finalPackage =
          if cfg.package == null || cfg.features == null then
            cfg.package
          else
            cfg.package.override {
              buildNoDefaultFeatures = true;
              buildFeatures = cfg.features;
            };
        configFile = tomlFormat.generate "scootbar-bar.toml" cfg.settings;
      };
    }

    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = cfg.package != null;
          message = ''
            programs.scootbar.package is null: apply the flake's overlay
            (inputs.scoot.overlays.default), import the flake's module
            (inputs.scoot.homeModules.scootbar / nixosModules.scootbar) or set
            the option, so the service has a bar to run.
          '';
        }
        {
          assertion = cfg.features == null || cfg.package == null || cfg.package ? override;
          message = ''
            programs.scootbar.features needs a package with `.override`
            (the flake's `scootbar`, which takes buildFeatures); the given
            package has none. Leave features null, or build the package
            with the features you want.
          '';
        }
      ];

      # A font only when something draws text; the bar refuses to start
      # for a font it cannot use and needs none for a bar with no modules.
      programs.scootbar.settings = lib.mkIf (cfg.features != [ ]) {
        bar.font = lib.mkOptionDefault fallbackFont;
      };
    })

    (lib.mkIf (cfg.enable && cfg.stylix.enable && stylix) {
      programs.scootbar.settings = {
        colors = {
          background = lib.mkDefault palette.base00;
          foreground = lib.mkDefault palette.base05;
          accent = lib.mkDefault palette.base0A;
          dim = lib.mkDefault palette.base03;
          urgent = lib.mkDefault palette.base08;
        };
        bar = {
          font-size = lib.mkDefault (pixels config.stylix.fonts.sizes.desktop);
        }
        // lib.optionalAttrs (cfg.features != [ ]) {
          font = lib.mkDefault "${fontFile config.stylix.fonts.sansSerif}/font";
        };
      };
    })
  ];
}
