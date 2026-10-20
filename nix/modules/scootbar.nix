# The part of `programs.scootbar` that home-manager and NixOS share: the
# options, the rendered `bar.toml` and the package
# with its Cargo features. `scootbar-home.nix` and `scootbar-nixos.nix`
# import it and add what only their side has (where the file goes, the
# systemd user unit). Written for Written for site/src/content/docs/scootbar/index.md, "The status bar: scootbar".
#
# The precedence of a `settings` value, highest first:
#
#   1. what the user wrote (priority 100, the module system's default);
#   2. this module's plain default (priority 1600): a font, and only a
#      font, since the bar refuses to start without one — and only while
#      no look layer covers it (see below).
#
# The look is not a rung here at evaluation time at all: it arrives as a
# runtime layer under the user's own file (see `scoot look sync`), merged
# per key below every `settings` value above. Without scoot (no look
# catalog), `stylix.enable` renders the `stylix` look's bar fragment at
# build time and pins it as a fixed `[look] layer`; the plain-default
# font is then left out, so the layer's own face shows instead of
# shadowing it. With a catalog present the option does nothing and
# warns. No app module reads Stylix itself: the reads are in
# `../lib/scootbar-stylix-layer.nix`, beside the scoot-side synthesis.
#
# Everything else the bar reads has its own default in the binary, so an
# absent key is that default.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scootbar;
  tomlFormat = pkgs.formats.toml { };

  # The bar-only Stylix path (see `../lib/scootbar-stylix-layer.nix`):
  # whether the `stylix` look's bar fragment renders, the fixed `[look]
  # layer` pointing at it, and the warning when a catalog present makes
  # the option a no-op. Only `active`, `layer` and
  # `warning` are read here; the Stylix reads stay in that file.
  barOnly = import ../lib/scootbar-stylix-layer.nix { inherit lib pkgs; } {
    inherit config cfg;
    scootLook = cfg.lookPackage;
    fallbackFont = fallbackFont;
  };

  # The module's one plain default: DejaVu Sans as a file (there is no
  # fontconfig). Left out while the bar-only layer covers the font (see
  # above), so the layer's own face shows.
  fallbackFont = "${pkgs.dejavu_fonts.minimal}/share/fonts/truetype/DejaVuSans.ttf";

  # The desktop profile (`programs.scoot.desktop` in the scoot modules),
  # when those are imported beside this one. Each `or` keeps this
  # evaluating with the scoot module absent (as in this file's own
  # checks): nothing here requires it. The profile never sets across the
  # boundary in the other direction either (a conditional set of an
  # undeclared option fails eval whatever the condition is), so this read
  # is the whole bar half of the profile.
  desktopProfile = ((config.programs.scoot or { }).desktop or { });
  profileOn = desktopProfile.enable or false;
  # `bar.enable = false` leaves the bar entirely to the user: neither
  # enabled nor themed by the profile.
  profileManagesBar = desktopProfile.bar.enable or true;

  # The notification daemon (`programs.scoot.desktop.notifications`
  # in the scoot modules), when those are imported beside this one.
  # Each `or` keeps this evaluating with the scoot module absent or
  # older than the notifications slot. The daemon behind it is the
  # only half that changes when scootnotify replaces mako (the module
  # id, the feed and the toggle stay); the toggle below names the
  # daemon's own command per `daemon`.
  notifProfile = (desktopProfile.notifications or { });
  notifOn = (notifProfile.enable or false);
  notifDaemon = (notifProfile.daemon or "mako");
  notifPkg = (notifProfile.package or null);
  # This daemon's build, when its module is imported beside this one
  # (the `or { }` keeps this evaluating without it, the way the bar
  # half reads the profile). The click calls it by absolute path then;
  # otherwise by name from PATH, tolerating a missing daemon.
  scootNotifyPkg = (config.programs.scootnotify or { }).finalPackage or null;
  scootNotifyBin = if scootNotifyPkg != null then lib.getExe scootNotifyPkg else "scootnotify";
  # The per-state icons (`desktop.notifications.bar.icons` in the scoot
  # modules). Only `idle` is read here (the push module's static icon,
  # shown until the first update and whenever an update names none);
  # `unread` and `dnd` ride the feed's per-update icons (see
  # `nix/modules/notifications-home.nix`). Each `or` keeps this
  # evaluating with the scoot module absent or older than the icons
  # (the defaults then come from this file's own `desktop.nix`, so the
  # two cannot drift).
  notifIcons =
    let
      iconDefaults = (import ./desktop.nix { inherit lib; }).options.notifications.bar.icons;
    in
    {
      idle = iconDefaults.idle.default;
      unread = iconDefaults.unread.default;
      dnd = iconDefaults.dnd.default;
    }
    // (notifProfile.bar.icons or { });
  notifPushNeeded =
    profileOn
    && profileManagesBar
    && notifOn
    && notifPkg != null
    && (notifDaemon == "mako" || notifDaemon == "scootnotify");

  # The WiFi and Bluetooth pickers (`programs.scoot.desktop.apps` in
  # the scoot modules, home-manager side: only that side builds the
  # scripts), when those are imported beside this one. A click on
  # either module then opens the same picker the keyboard binds open,
  # fed the bar's own list: the network module's scan into
  # `scoot-network-pick menu` (and its popup rows into `connect`), the
  # bluetooth module's devices into `scoot-bluetooth-pick menu`. Wired
  # only where the bar has the module built in: a `[network]` or
  # `[bluetooth]` key in a build without it is refused at startup.
  appsProfile = desktopProfile.apps or { };
  appsScripts = appsProfile.scripts or { };
  builtIn = feature: cfg.features == null || builtins.elem feature cfg.features;
  pickerWired =
    slot: feature:
    profileOn
    && profileManagesBar
    && (appsProfile.${slot}.enable or false)
    && (appsScripts.${slot} or null) != null
    && builtIn feature;
  networkPicker = "${appsScripts.network}/bin/scoot-network-pick";
  bluetoothPicker = "${appsScripts.bluetooth}/bin/scoot-bluetooth-pick";
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
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.stdenv.hostPlatform.system}.scootbar";
      description = ''
        The scootbar package. Its Cargo features are `features`, below;
        a `package` you build yourself is used as given when `features`
        is null.
      '';
    };

    # The modules are Cargo features (one per module, `clock` the default;
    # site/src/content/docs/scootbar/index.md). Null builds what `package` is; a list is exactly the
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
    # file's schema is site/src/content/docs/scootbar/cli.md's, checked by the bar itself
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
        listed in site/src/content/docs/scootbar/cli.md ("The config file"). What is not
        set takes the binary's own default, unless a look layer carries
        it at runtime: the `stylix` look's fragment while `stylix.enable`
        renders one, else the active look through `scoot look sync`.
      '';
    };

    # The `scoot-look` binary that renders the bar-only Stylix fragment
    # at build time. Null keeps that path off: with Stylix in use and no
    # catalog, the bar is then unthemed.
    lookPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scoot-look or null;
      defaultText = lib.literalExpression "pkgs.scoot-look or null";
      description = ''
        The scoot-look package that renders the bar-only Stylix fragment
        (`stylix.enable`, below) at build time. Null keeps that path off.
      '';
    };

    stylix.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        The bar-only Stylix path: with Stylix present and enabled, and
        no scoot look catalog in this configuration, render the `stylix`
        look's bar fragment at build time (the same `scoot-look` binary
        the catalog uses) and pin it as a fixed `[look] layer` in the
        rendered `bar.toml`. A value you set in `settings` still wins per
        key at runtime; this only turns the layer off. With a catalog
        present the option does nothing and warns. Nothing here needs
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
        by the session scope (`scoot-session.target` under the desktop
        profile, `graphical-session.target` standalone) and restarted
        when it fails (scoot does not supervise its clients). Turn it off
        to start `scootbar daemon` from your compositor's autostart instead.
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
        {
          # The notification feed writes this module: a build without
          # it refuses the table at startup, naming it. Loud here
          # instead, at eval. Null `features` keeps the default build,
          # which has it.
          assertion = !notifPushNeeded || cfg.features == null || builtins.elem "push" cfg.features;
          message = ''
            programs.scoot.desktop.notifications.enable needs the push
            module: add "push" to programs.scootbar.features (or leave
            features null for the default build, which has it).
          '';
        }
        {
          # The static idle icon obeys the same one-glyph rule the
          # feed's per-update icons do (the bar refuses anything else
          # where it is drawn).
          assertion = !notifPushNeeded || (import ./desktop.nix { inherit lib; }).isStateIcon notifIcons.idle;
          message = ''
            programs.scoot.desktop.notifications.bar.icons.idle is
            empty (no icon while idle) or exactly one glyph: got ${notifIcons.idle}.
          '';
        }
      ];

      # A font only when something draws text; the bar refuses to start
      # for a font it cannot use and needs none for a bar with no modules.
      # Priority 1600: below the user, above the binary's own default —
      # and only while no look layer covers the font (the bar-only layer
      # carries its own face, which a default in the file would shadow
      # per key at runtime; see the precedence note up top).
      programs.scootbar.settings = lib.mkIf (cfg.features != [ ] && !barOnly.active) {
        bar.font = lib.mkOverride 1600 fallbackFont;
      };
    })

    # The bar-only Stylix layer (see `../lib/scootbar-stylix-layer.nix`):
    # the `stylix` look's bar fragment, pinned as a fixed `[look] layer`
    # under the user's own file. A `look.layer` set in `settings` wins
    # (plain beats `mkDefault`).
    (lib.mkIf barOnly.active {
      programs.scootbar.settings.look.layer = lib.mkDefault barOnly.layer;
    })

    # With a catalog present the option does nothing, and says so.
    (lib.mkIf (barOnly.warning != null) {
      warnings = [ barOnly.warning ];
    })

    # The desktop profile's bar half: the profile turns this bar on (at
    # `mkDefault`, so an explicit value still wins). The look reaches
    # the bar as a runtime layer under the user's own file (see
    # `scoot look sync`), never as eval-time values here.
    (lib.mkIf (profileOn && profileManagesBar) {
      programs.scootbar.enable = lib.mkDefault true;
    })

    # The notification daemon's bar half: the `push` module its feed
    # writes (DND state and the unread count -- with mako through
    # `scoot-notify-sync`, with scootnotify through the daemon's own
    # push: see `nix/modules/notifications-home.nix`), with a click
    # toggling do-not-disturb through the daemon's own command, by
    # absolute path, so it works whatever is on PATH. Each leaf a priority
    # below the user's: a value set in `settings` wins
    # per key. Placement stays yours (the
    # module is defined, not placed -- add `"notifications"` to a list
    # to show it; see site/src/content/docs/scootbar/theming.md). Like the
    # other profile halves, a bar the user turned off stays unthemed too. The idle icon stands while
    # the text is empty (a quiet desktop keeps a clickable indicator,
    # not a hole); it is in DejaVu Sans, the bar's own default font,
    # and empty (`bar.icons.idle = ""`) sets no static icon at all, so
    # a quiet desktop shows nothing. The unread and DND states arrive
    # as per-update icons from the feed, overriding this while set.
    (lib.mkIf (notifPushNeeded && cfg.enable) {
      programs.scootbar.settings.push.notifications = {
        icon = lib.mkIf (notifIcons.idle != "") (lib.mkOptionDefault notifIcons.idle);
        on-click = lib.mkOptionDefault {
          exec =
            if notifDaemon == "scootnotify" then
              [
                scootNotifyBin
                "msg"
                "dnd"
                "toggle"
              ]
            else
              [
                "${lib.getExe' notifPkg "makoctl"}"
                "mode"
                "-t"
                "do-not-disturb"
              ];
        };
      };
    })

    # The apps pickers' bar half (see `pickerWired` above): each
    # command a priority below the user's, so a command
    # set in `settings` wins per key. Like the other profile halves, a
    # bar the user turned off stays unwired.
    (lib.mkIf (pickerWired "network" "network" && cfg.enable) {
      programs.scootbar.settings.network = {
        menu-command = lib.mkOptionDefault [
          networkPicker
          "menu"
        ];
        connect-command = lib.mkOptionDefault [
          networkPicker
          "connect"
        ];
      };
    })
    # The bluetooth module's click opens the whole picker (`pick`: the
    # paired devices, pairing, audio output, power) in place of the
    # module's own default, which toggles the adapter's power: one
    # stray click would drop every Bluetooth device, a keyboard or
    # mouse among them. A click is a launched command, so the bar
    # never waits on it, and fuzzel's lock keeps a second click from
    # opening a second menu. `menu-command` stays wired for the
    # module's `menu` action (its own device list).
    (lib.mkIf (pickerWired "bluetooth" "bluetooth" && cfg.enable) {
      programs.scootbar.settings.bluetooth = {
        menu-command = lib.mkOptionDefault [
          bluetoothPicker
          "menu"
        ];
        on-click = lib.mkOptionDefault {
          exec = [
            bluetoothPicker
            "pick"
          ];
        };
      };
    })
  ];
}
