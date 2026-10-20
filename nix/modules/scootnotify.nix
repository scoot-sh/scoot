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
# runtime layer under the user's own file (see `scoot look sync`), merged
# per key below every `settings` value above. So the rendered file holds
# only the user's `settings`, verbatim.
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
  daemonIsNotify = (desktopProfile.notifications or { }).daemon or "mako" == "scootnotify";
  # The slot's mako-shaped settings: flat `key = value` lines for mako's
  # own config dialect. They do not carry over to this daemon (whose file
  # is the user's `settings` here, below): a non-empty one beside the new
  # daemon warns, naming where the values go instead.
  slotSettings = (desktopProfile.notifications or { }).settings or { };
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
