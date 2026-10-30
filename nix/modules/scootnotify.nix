# The part of `programs.scootnotify` that home-manager and NixOS share:
# the options, the rendered `notify.toml` and the package with its Cargo
# features. `scootnotify-home.nix` and `scootnotify-nixos.nix` import it
# and add only what their side has (where the file goes, the unit, the
# D-Bus activation file). Written for
# site/src/content/docs/scootnotify/index.md ("Overview").
#
# The precedence of a `settings` value, highest first:
#
#   1. what the user wrote (priority 100, the module system's default);
#   2. this module's plain default: nothing (an empty settings renders an
#      empty file, which the binary reads as its built-ins).
#
# The look is not a rung here at evaluation time at all: it arrives as a
# runtime layer (see `scoot look sync`) that the daemon reads itself, so
# the rendered file holds only the user's `settings`, verbatim -- with one
# exception: the desktop profile's `theme.targets.notifications.enable =
# false` adds `look = false` at `mkDefault` (rung 2), which tells the
# daemon to ignore the layer. The layer carries colors, font size,
# opacity and the icon theme; the file carries geometry, timeouts and
# rules (the card corner radius is drawn from the file's `radius`). So
# there is no per-key merge of the two: the switch is all-or-nothing,
# and the per-key override of a look value is the look's own
# `[apps.scootnotify]` table. The exception is the depth, motion and card
# tokens: the file's own `[effects]`, `[card]` and `[title]` tables win
# per key over the look at runtime, and the profile's
# `theme.overrides` fills them at `mkDefault`.
#
# Everything else the daemon reads has its own default in the binary,
# so an absent key is that default.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scootnotify;
  tomlFormat = pkgs.formats.toml { };

  # The desktop profile (`programs.scoot.desktop` in the scoot modules),
  # when those are imported beside this one. Each `or` keeps this
  # evaluating with the scoot module absent (as in this file's own
  # checks): nothing here requires it. The profile never sets across the
  # boundary in the other direction either (a conditional set of an
  # undeclared option fails eval whatever the condition is), so this read
  # is the whole notify half of the profile.
  desktopProfile = ((config.programs.scoot or { }).desktop or { });
  profileOn = desktopProfile.enable or false;
  # The notification slot beside this module: on with the profile
  # (still individually disable-able), standalone otherwise.
  slotOn = (desktopProfile.notifications or { }).enable or false;
  # `theme.targets.notifications.enable`: whether the look themes the
  # notification daemon. Read on its own, never through the look-set
  # check the other targets use: the look layer also reaches this daemon
  # from a look applied at runtime (`scoot look apply`) with none
  # declared here, and only the user's own `false` turns it off.
  lookTargetOn = (((desktopProfile.theme or { }).targets or { }).notifications or { }).enable or true;
  daemonIsNotify = (desktopProfile.notifications or { }).daemon or "scootnotify" == "scootnotify";
  # The slot's mako-shaped settings: flat `key = value` lines for mako's
  # own config dialect. They do not carry over to this daemon (whose file
  # is the user's `settings` here, below): a non-empty one beside the new
  # daemon warns, naming where the values go instead.
  slotSettings = (desktopProfile.notifications or { }).settings or { };
  # The profile's look-independent tokens (`theme.overrides`): written
  # into the file's own `[effects]`, `[card]` and `[title]` tables, which
  # win per key over whatever look is active. An empty table writes
  # nothing, so a profile that sets none renders the file it always did.
  overrides = (desktopProfile.theme or { }).overrides or { };
  overrideTables = lib.filterAttrs (_: table: table != { }) overrides;
in
{
  options.programs.scootnotify = {
    enable = lib.mkEnableOption "scootnotify, the notification daemon";

    # `pkgs.scootnotify` with the flake's overlay, else null; the flake's
    # wrappers (`homeModules.scootnotify`, `nixosModules.scootnotify`)
    # fill it with the flake's own build. Null with `enable` is refused:
    # the unit would have nothing to run.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scootnotify or null;
      defaultText = lib.literalExpression "pkgs.scootnotify or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.stdenv.hostPlatform.system}.scootnotify";
      description = ''
        The scootnotify package. Its Cargo features are `features`, below;
        a `package` you build yourself is used as given when `features`
        is null.
      '';
    };

    # The modules are Cargo features (none today: the daemon is one
    # build -- see nix/scootnotify.nix). Null builds what `package` is;
    # a list is exactly the features that build has.
    features = lib.mkOption {
      type = lib.types.nullOr (lib.types.listOf lib.types.str);
      default = null;
      example = [ ];
      description = ''
        The features to build the daemon with, as Cargo features:
        exactly the ones listed (`scootnotify.override {
        buildNoDefaultFeatures = true; buildFeatures = features; }`).
        Null keeps `package` as it is (its default features). Needs a
        `package` that takes those overrides, as the flake's does.
      '';
    };

    # Free-form: a new option never needs a module change first, and the
    # file's schema is the daemon's own (`crates/scootnotify` refuses
    # an unknown key by name, and the check in this flake runs the real
    # binary over the rendered file).
    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = {
        max_visible = 5;
        dnd_allows_critical = true;
      };
      description = ''
        The contents of `notify.toml`, rendered as TOML. The options are
        the daemon's own (`max_visible`, `dnd_allows_critical`, timeouts,
        rules): what is not set takes the binary's own default, unless a
        look layer carries it at runtime (the active look through
        `scoot look sync`).
      '';
    };

    # Run the daemon as a systemd user service. On to run it
    # (`scootnotify.service`, wanted by the session scope and restarted
    # when it fails -- a restart drops the cards and the history, so a
    # look swap reloads instead, never restarts). Off installs the
    # package and the file, and starts the daemon some other way.
    systemd.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Run the daemon as a systemd user service, `scootnotify.service`,
        wanted by the session scope (`scoot-session.target` under the
        desktop profile, `graphical-session.target` standalone). Turn it
        off to start `scootnotify daemon` some other way.
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
      description = "The rendered `notify.toml`.";
    };
  };

  config = lib.mkMerge [
    {
      programs.scootnotify = {
        finalPackage =
          if cfg.package == null || cfg.features == null then
            cfg.package
          else
            cfg.package.override {
              buildNoDefaultFeatures = true;
              buildFeatures = cfg.features;
            };
        configFile = tomlFormat.generate "scootnotify-notify.toml" cfg.settings;
      };
    }

    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = cfg.package != null;
          message = ''
            programs.scootnotify.package is null: apply the flake's overlay
            (inputs.scoot.overlays.default), import the flake's module
            (inputs.scoot.homeModules.scootnotify / nixosModules.scootnotify) or set
            the option, so the service has a daemon to run.
          '';
        }
        {
          assertion = cfg.features == null || cfg.package == null || cfg.package ? override;
          message = ''
            programs.scootnotify.features needs a package with `.override`
            (the flake's `scootnotify`, which takes buildFeatures); the given
            package has none. Leave features null, or build the package
            with the features you want.
          '';
        }
        {
          # The profile's mako escape hatch and this daemon own the same
          # bus name: enabling both leaves two units fighting for it
          # (the loser exits 3, mako respawns every 2 s with no
          # RestartPreventExitStatus). Refuse that combination loudly.
          assertion = !(profileOn && slotOn && !daemonIsNotify);
          message = ''
            programs.scootnotify is enabled together with programs.scoot.desktop.notifications.daemon = "mako": only one daemon can own org.freedesktop.Notifications. Disable one (programs.scootnotify.enable = false, or drop daemon = "mako").
          '';
        }
      ];
    })

    # The desktop profile's notify half: with the profile on, the slot
    # on, and `desktop.notifications.daemon = "scootnotify"`, this
    # daemon turns on too (at `mkDefault`, so an explicit value still
    # wins). The look reaches it as a runtime layer under the user's
    # own file (see `scoot look sync`), never as eval-time values here.
    (lib.mkIf (profileOn && slotOn && daemonIsNotify) {
      programs.scootnotify.enable = lib.mkDefault true;
    })

    # The look target off (`theme.targets.notifications.enable = false`)
    # beside the profile-managed daemon: the file carries `look = false`,
    # so the cards keep the built-in theme whatever look layer is on
    # disk. At `mkDefault`, so an explicit `settings.look` still wins.
    # The key rides the file, not the command line: toggling it
    # reloads the daemon in place (`X-Reload-Triggers`), where a changed
    # `ExecStart` would restart it and drop the cards and the history.
    (lib.mkIf (profileOn && daemonIsNotify && !lookTargetOn) {
      programs.scootnotify.settings.look = lib.mkDefault false;
    })

    # `theme.overrides`: the user's effect tokens in the file, per key at
    # `mkDefault` so an explicit `settings.<table>.<key>` wins. Unset (or
    # no profile) writes nothing.
    (lib.mkIf (overrideTables != { }) {
      programs.scootnotify.settings = lib.mapAttrs (_: lib.mapAttrs (_: lib.mkDefault)) overrideTables;
    })

    # The one slot option that does not survive the swap: mako's flat
    # `key = value` dialect has no meaning to this daemon (whose file is
    # `settings` above). A non-empty one beside the new daemon warns,
    # naming where the values go, instead of failing the switch or
    # silently dropping them.
    (lib.mkIf (profileOn && daemonIsNotify && slotSettings != { }) {
      warnings = [
        "programs.scoot.desktop.notifications.settings is mako's config dialect and has no effect with desktop.notifications.daemon = \"scootnotify\": move what still applies into programs.scootnotify.settings (see site/src/content/docs/scootnotify/configure.md)."
      ];
    })
  ];
}
