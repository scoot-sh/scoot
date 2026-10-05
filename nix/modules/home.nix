{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  tomlFormat = pkgs.formats.toml { };

  # The desktop profile's shared option subtree and look palettes.
  desktop = import ./desktop.nix { inherit lib; };
  # Null without a look; the `enum` type guarantees the name is one of
  # these, so the lookup cannot fail.
  look = if cfg.desktop.look == null then null else desktop.looks.${cfg.desktop.look};

  # Stylix is not an input of this flake: its presence is
  # `config.lib.stylix` (defined by its palette module whether or not it is
  # enabled), and `stylix.enable` (false by default in Stylix itself) says
  # whether it is asked to theme anything. Same detection as
  # `nix/modules/scootbar.nix`.
  stylix = config.lib ? stylix && (config.stylix.enable or false);

  # The base16 slots this module reads, as `#rrggbb`. Read here so the
  # mapping below names slots, not literals; lazy, so untouched without
  # Stylix (as is everything under the `mkIf` at the bottom).
  palette = config.lib.stylix.colors.withHashtag;

  # Rendered from the user's free-form settings. A value with no TOML
  # representation at all (e.g. a function) fails the option type-check
  # at evaluation time ("not of type 'TOML value'"), so the error aborts
  # before anything builds, let alone starts a session. A value that renders
  # but has the wrong *scoot* type (a string for `layout.gap`) reaches
  # the session and is refused there -- and the loader fails safe (whole
  # file discarded for defaults, session still boots), so a typo costs
  # the config, never the session. See
  # docs/configuration.md#failure-semantics.
  configFile = tomlFormat.generate "scoot-config.toml" renderedSettings;

  # `settings`, plus `wallpaper.command` pointing at the installed scootbg
  # when the settings have a `[wallpaper]` table and the module installs
  # the binary -- so the section works whatever is on PATH. A `command`
  # the user set wins. Only for a table: anything else under `wallpaper`
  # renders as written and is refused, by name, by scoot itself. The store
  # path changes with every upgrade; scootbg leaves `command` out of the
  # section's fingerprint, so an upgrade never re-applies the section over
  # a `scootbg set` pick.
  hasWallpaper = cfg.settings ? wallpaper;
  injectCommand =
    cfg.wallpaper.enable
    && cfg.wallpaper.package != null
    && hasWallpaper
    && builtins.isAttrs cfg.settings.wallpaper;
  renderedSettings =
    if injectCommand then
      cfg.settings
      // {
        wallpaper = {
          command = lib.getExe' cfg.wallpaper.package "scootbg";
        }
        // cfg.settings.wallpaper;
      }
    else
      cfg.settings;

  # Session script path: beside the rendered config, derived from
  # `configFile`'s directory, so a relocated config keeps its script
  # next to it (`scoot/session.sh` by default -- `dirOf` answers
  # `"scoot"`, i.e. the default path is unchanged). A bare filename
  # (`configFile = "config.toml"`) has no directory (`dirOf` answers
  # `"."`); the script then lands at the config root as `session.sh`.
  # `configFile` is relative to $XDG_CONFIG_HOME by contract (see the
  # option), so an absolute path is already user error on the config
  # half and stays one here.
  scriptPath =
    let
      d = builtins.dirOf cfg.configFile;
    in
    if d == "." || d == "" then "session.sh" else "${d}/session.sh";
in
{
  options.programs.scoot = {
    enable = lib.mkEnableOption "scoot, the scrolling-tiling Wayland compositor";

    # One switch plus a look choice for a working desktop (see
    # `desktop.nix` and docs/nix.md). Each side wires only what it owns;
    # this side owns the config file (the look's `[appearance]` and
    # `[wallpaper]`), the portal config and the `[xwayland]` knob.
    desktop = desktop.options;

    # `pkgs.scoot` when the flake's overlay (`overlays.default`) is
    # applied, else null: nothing is guessed, since a `scoot` from anywhere
    # else would silently install someone else's build. The flake wrapper
    # (`homeModules.scoot`, still aliased as `homeManagerModules.scoot`, in
    # `flake.nix`) fills this with the flake's own build via `mkDefault` on
    # Linux and with null (files only) on Darwin; direct-module users
    # without the overlay set it explicitly (see the `example`), or leave
    # it null for a files-only setup -- the module manages files
    # regardless, and asserts nothing about the package.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scoot or null;
      defaultText = lib.literalExpression "pkgs.scoot or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scoot";
      description = ''
        The scoot package to install. Null means install no binary --
        the module then only manages files.
      '';
    };

    # Free-form TOML value, NOT a typed schema per field -- deliberately.
    # The config moves fast (three tables landed this month); a typed
    # schema goes stale and then lies about what the compositor accepts,
    # while free-form never drifts. `[binds]` is arbitrary keys anyway,
    # so a schema would need an escape hatch exactly where most of the
    # user content lives. Typed options exist only where they carry
    # behavior beyond the file (`enable`, `package`, `configFile`,
    # `sessionScript`, `portals.enable`); everything the compositor
    # reads passes through here verbatim. Field names, types and
    # defaults are documented in docs/configuration.md, and the worked
    # example in docs/nix.md is pasted from a live
    # `scoot --print-default-config` emission, not hand-written.
    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = {
        layout.gap = 8;
        binds = {
          "super+t" = "spawn foot";
          "ctrl+alt+space" = "spawn wofi --show drun";
        };
        autostart.commands = [ "spawn waybar" ];
      };
      description = ''
        Free-form scoot configuration, rendered verbatim to TOML.
        An empty set renders a valid minimal file (the compositor runs
        on built-in defaults, exactly as with no file at all).
      '';
    };

    # Stylix, without depending on it. Same shape as
    # `programs.scootbar.stylix.enable` in `nix/modules/scootbar.nix`:
    # on by default, effective only when Stylix itself is in use (see
    # the `stylix` detection above); every value below is `lib.mkDefault`
    # at its own leaf, so a value the user wrote in `settings` wins.
    # Nothing here needs Stylix, and nothing changes without it.
    stylix = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Take defaults from Stylix when it is in use (`config.lib.stylix`
          exists and `stylix.enable` is on): the `[appearance]` ring and
          background colors from its base16 palette, `cursor_theme` and
          `cursor_size` from `stylix.cursor`, and `[wallpaper]` `image` and
          `mode` from `stylix.image` and `stylix.imageScalingMode` (the
          wallpaper pair further gated by `wallpaper.enable` below). A value
          you set in `settings` always wins; this only turns the defaults
          off. Nothing here needs Stylix, and nothing changes without it.
        '';
      };

      # Just the Stylix wallpaper defaults, so a solid-color background
      # stays reachable under Stylix: set this to `false` to choose your
      # own wallpaper color (`settings.wallpaper.color`) without Stylix's
      # `image` beside it (the two together are refused by scoot). The
      # themed `[appearance]` and cursor defaults stay. An explicit
      # switch, read here rather than the merged `settings.wallpaper`,
      # is what keeps the evaluation acyclic (see the `wallpaper` block
      # below).
      wallpaper.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Take the `[wallpaper]` `image` and `mode` defaults from Stylix
          (`stylix.image` and `stylix.imageScalingMode`) when theming is
          on. Set to `false` to choose your own wallpaper color instead:
          the Stylix `image` and `mode` are then not defaulted, so a
          `color` you set stands alone. The `[appearance]` and cursor
          defaults are unaffected.
        '';
      };
    };

    # Where the rendered TOML lands, relative to $XDG_CONFIG_HOME
    # (i.e. `xdg.configFile`). Overriding this is the only way to move
    # the config: scoot itself reads a fixed default path unless passed
    # `--config`, and the module does not launch scoot, so a path here
    # that scoot never reads would be a file to nowhere.
    configFile = lib.mkOption {
      type = lib.types.str;
      default = "scoot/config.toml";
      example = "scoot/config.toml";
      description = ''
        Location of the rendered config, relative to $XDG_CONFIG_HOME.
        Keep the default unless you also pass the same path via
        `--config` wherever you launch scoot.
      '';
    };

    # Behavior half of "Starting a session" (see
    # docs/configuration.md#starting-a-session): the config declares the
    # baseline, a script carries ordering/conditionals. When set, this
    # text is written executable beside the config (at
    # `<dirOf configFile>/session.sh` -- `scoot/session.sh` by default);
    # launch it with `scoot -- ~/.config/<that path>` (or exec it from
    # your greetd/startwm entry). On NixOS, the matching half is the
    # NixOS module's `programs.scoot.session.command`: set it to the full
    # `<package>/bin/scoot --tty -- /home/<user>/.config/<that path>`
    # line -- an absolute path, since `Exec=` lines get no shell
    # expansion -- so the greeter entry runs this script instead of the
    # `scoot-session` launcher (see docs/nix.md, which shows the pairing
    # together). That entry runs without the launcher's session wiring;
    # startup programs that want the wiring belong in `[autostart]`
    # instead. Null writes no file.
    sessionScript = lib.mkOption {
      type = lib.types.nullOr lib.types.lines;
      default = null;
      example = ''
        waybar &
        mako &
        exec foot
      '';
      description = ''
        Session startup script, written executable beside the rendered
        config at `<dirOf configFile>/session.sh` (`scoot/session.sh`
        with the default `configFile`). Launch it with
        `scoot -- ~/.config/scoot/session.sh` for the default (or
        `~/.config/<that path>` after a `configFile` override), or exec
        it from your greetd/startwm entry -- on NixOS, set the NixOS
        module's `programs.scoot.session.command` to the full
        `<package>/bin/scoot --tty -- /home/<user>/.config/<that path>`
        line (an absolute path -- `Exec=` lines get no shell expansion)
        so the login-screen entry runs it instead of the `scoot-session`
        launcher. That entry runs without the launcher's session wiring
        (no `graphical-session.target`, no activation import); for wired
        startup programs, use `[autostart]` instead. Null writes no file.
      '';
    };

    # scootbg, the wallpaper daemon a `[wallpaper]` section runs. Spelled
    # as in the NixOS module (`programs.scoot.wallpaper.*`), so either side
    # reads like the other.
    wallpaper = {
      # Follows the settings: a `[wallpaper]` table is what asks for the
      # binary, and nothing else here needs it.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = hasWallpaper;
        defaultText = lib.literalExpression "config.programs.scoot.settings ? wallpaper";
        description = ''
          Install `wallpaper.package` and point `settings.wallpaper.command`
          at it (unless you set `command` yourself). On whenever `settings`
          has a `wallpaper` table. Set it to `false` to leave both alone:
          `[wallpaper]` then runs `scootbg` from PATH (a system package,
          say: the NixOS module's `programs.scoot.wallpaper`).
        '';
      };

      # `pkgs.scootbg` with the flake's overlay, else null. The flake
      # wrapper injects the flake's own scootbg on Linux (the same revision
      # as `package`) and null on Darwin, where scootbg does not build: a
      # macOS config that edits a `[wallpaper]` for a Linux box renders it
      # as written, with no `command`, and installs nothing.
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = pkgs.scootbg or null;
        defaultText = lib.literalExpression "pkgs.scootbg or null";
        example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scootbg";
        description = ''
          The scootbg package to install when `wallpaper.enable` is set.
          Null installs nothing and leaves `command` alone.
        '';
      };
    };

    portals = {
      # On by default behind `enable`: `scoot-portals.conf` is inert
      # until a session names `XDG_CURRENT_DESKTOP=scoot` (which the
      # compositor exports to everything it spawns), and the per-user
      # path below is the highest-precedence lookup slot, so installing
      # it changes nothing outside a scoot session. Turn it off if you
      # manage portal backends some other way.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Install `scoot-portals.conf` to the per-user
          xdg-desktop-portal lookup path, so ScreenCast/Screenshot
          resolve to the `wlr` backend inside a scoot session.
        '';
      };
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.enable {
      # No assertion that `package` is set: a files-only setup (binary
      # from elsewhere, e.g. a system package) is legitimate, and with
      # defaults this still manages a (minimal, valid) config plus the
      # portals file -- both harmless. `package`'s description states
      # that null installs no binary.

      home.packages =
        lib.optional (cfg.package != null) cfg.package
        ++ lib.optional (cfg.wallpaper.enable && cfg.wallpaper.package != null) cfg.wallpaper.package;

      xdg.configFile.${cfg.configFile}.source = configFile;

      xdg.configFile.${scriptPath} = lib.mkIf (cfg.sessionScript != null) {
        executable = true;
        text = "#!/bin/sh\n" + cfg.sessionScript;
      };

      # Per-user slot from portals.conf(5), highest precedence; see
      # resources/scoot-portals.conf for what each backend line means and
      # why. This closes the remainder handed off by the session-environment
      # ticket, which this module now owns.
      xdg.configFile."xdg-desktop-portal/scoot-portals.conf" = lib.mkIf cfg.portals.enable {
        source = ../../resources/scoot-portals.conf;
      };
    })

    # The desktop profile's user half. `enable` owns the session-adjacent
    # defaults this side has (the portal config; the session entry and the
    # system packages are the NixOS side's, the bar is the bar module's,
    # which reads this profile). `look` themes the rendered config, each
    # leaf at `mkOptionDefault` so a value the user wrote wins and Stylix
    # (a `mkDefault` one level up) wins where present -- Stylix stays the
    # override path.
    (lib.mkIf cfg.desktop.enable {
      assertions = [
        {
          # A profile for a compositor that is not installed is
          # nonsense; without `enable` there is no session for the
          # profile's daemons to join.
          assertion = cfg.enable;
          message = ''
            programs.scoot.desktop.enable needs programs.scoot.enable:
            the profile themes and serves a scoot session, so scoot itself
            must be installed.
          '';
        }
      ];

      programs.scoot.portals.enable = lib.mkDefault true;
    })

    # The `[xwayland]` knob, in its own element: a nested `mkIf` on the leaf
    # would leave an empty `settings.xwayland` behind when off (the
    # condition empties the value, not the path), which a whole-element
    # gate does not.
    (lib.mkIf (cfg.desktop.enable && cfg.desktop.xwayland.enable) {
      programs.scoot.settings.xwayland.enabled = lib.mkDefault true;
    })

    # A look without the profile is a silent no-op; refuse it loudly
    # instead (kept outside `desktop.enable` so it still fires then, the
    # way the greeter's assertions sit outside `enable`).
    (lib.mkIf (cfg.desktop.look != null) {
      assertions = [
        {
          assertion = cfg.desktop.enable;
          message = ''
            programs.scoot.desktop.look needs programs.scoot.desktop.enable:
            the look is applied by the profile, so the profile must be on.
          '';
        }
      ];
    })

    # The look's compositor colors plus, where the look ships an in-repo
    # wallpaper, its image and mode (which is what turns `wallpaper.enable`
    # on and installs scootbg for it). A look without one (`vinyl-sunset`,
    # whose illustration cannot be committed) sets no `[wallpaper]` keys:
    # the session shows the flat `background_color` below, and a
    # `wallpaper` table the user sets themselves pairs with it untouched.
    # `toString` is the identity on the mode and renders the image path as
    # its store path.
    (lib.mkIf (cfg.desktop.enable && cfg.desktop.look != null) {
      programs.scoot.settings = {
        appearance = lib.mapAttrs (name: value: lib.mkOptionDefault value) look.appearance;
      }
      // lib.optionalAttrs (look.wallpaper != null) {
        wallpaper = lib.mapAttrs (name: value: lib.mkOptionDefault (toString value)) look.wallpaper;
      };
    })

    # The bar half lives in the bar module (`nix/modules/scootbar.nix`
    # reads this profile): a set of `programs.scootbar` here would need
    # that module imported, and a conditional set of an undeclared option
    # fails eval whatever the condition is, so the profile never sets
    # across the module boundary.

    # Stylix defaults, each at its own leaf (`settings.appearance.X`, not
    # `settings.appearance`), which is what lets the user's one value
    # replace one default and keep the rest from Stylix: a priority
    # applies to the whole value it wraps, so wrapping a table would make
    # the user's one key discard it all. Same rule as `scootbar.nix`.
    #
    # The mapping is what Stylix's own compositor targets use for the
    # same three things (checked against nix-community/stylix at fb28acd,
    # the rev `nix/scootbar-tests.nix` names): sway's
    # `modules/sway/hm.nix` (`focused = base0D`, every unfocused border
    # `base03`, `background = base00`), hyprland's
    # `modules/hyprland/hm.nix` (`col.active_border = base0D`,
    # `col.inactive_border = base03`, `misc.background_color = base00`)
    # and river's `modules/river/hm.nix` (`border-color-focused =
    # base0D`, `border-color-unfocused = base03`, `background-color =
    # base00`). There is no niri target at that rev (nor on current
    # master) to check against. `cursor_color` has no Stylix convention
    # (Stylix's cursor is name, size and package only), so it is left
    # alone.
    (lib.mkIf (cfg.enable && cfg.stylix.enable && stylix) {
      programs.scoot.settings = {
        appearance = {
          focus_ring_active_color = lib.mkDefault palette.base0D;
          focus_ring_inactive_color = lib.mkDefault palette.base03;
          background_color = lib.mkDefault palette.base00;
        }
        // lib.optionalAttrs (config.stylix.cursor != null) {
          # The theme's package needs no installing here: Stylix's own
          # cursor target (`home.pointerCursor`, set by Stylix itself on
          # the home-manager side) already installs it and puts its
          # `share/icons` on the lookup path (`~/.icons`,
          # `$XDG_DATA_HOME/icons`, `$XCURSOR_PATH`), which is where
          # scoot's `xcursor::CursorTheme::load` searches. This only
          # names the theme and the size.
          cursor_theme = lib.mkDefault config.stylix.cursor.name;
          cursor_size = lib.mkDefault config.stylix.cursor.size;
        };
      }
      // lib.optionalAttrs (cfg.stylix.wallpaper.enable && config.stylix.image != null) {
        # Gated on `image`: a `mode` without one is meaningless, and an
        # unconditional table would turn `wallpaper.enable` (which
        # follows `settings ? wallpaper`) on for every Stylix user,
        # installing scootbg where no wallpaper was asked for. The five
        # `imageScalingMode` values are exactly scootbg's five `mode`
        # values (`fill | fit | stretch | center | tile`), so the mode
        # maps one to one. Gated also on
        # `programs.scoot.stylix.wallpaper.enable` (on by default): set
        # it to `false` to choose your own wallpaper `color` -- a `color`
        # beside Stylix's `image` is refused by scoot (fail-safe, the
        # session carries on with `background_color`, the error in the
        # log naming it). The condition reads this explicit switch, not
        # the merged `settings.wallpaper` this block defines, which is
        # what keeps the evaluation acyclic (a home-manager assertion on
        # the merged section would cycle the same way).
        wallpaper = {
          image = lib.mkDefault (toString config.stylix.image);
          mode = lib.mkDefault config.stylix.imageScalingMode;
        };
      };
    })
  ];
}
