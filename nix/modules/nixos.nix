{
  config,
  lib,
  pkgs,
  modulesPath,
  ...
}:

let
  cfg = config.programs.scoot;

  # The desktop profile's shared option subtree (the look palettes stay
  # where they are applied: the home-manager side and the bar module),
  # plus the one helper that reads look-derived values.
  desktop = import ./desktop.nix { inherit lib; };
  themeLook = import ./theme-look.nix { inherit lib; };
  lookNix = themeLook.lookFor cfg.desktop;

  # The login-screen entry. Built as a package exposing
  # `share/wayland-sessions/scoot.desktop` with `providedSessions`,
  # which is the shape `services.displayManager.sessionPackages`
  # requires (see the `addCheck` on that option in nixpkgs'
  # `nixos/modules/services/display-managers/default.nix` at the
  # pinned rev: an element without `providedSessions` is rejected at
  # eval). Consumed via XDG_DATA_DIRS by greetd/tuigreet, GDM, SDDM --
  # whichever reads wayland-sessions -- with no change to the default
  # session: this module never sets `services.displayManager.defaultSession`
  # (pre-select) or `services.greetd.settings.default_session` /
  # `initial_session` (what actually runs), so enabling it adds scoot
  # to the menu next to existing sessions and switches nothing.
  #
  # Auto-login caveat, verified against the same file: with no explicit
  # `defaultSession`, `autologinSession` falls back to the head of the
  # session list, so on a box that auto-logs-in, adding ANY session
  # package can move the autologin target. If you use autoLogin, pin
  # `services.displayManager.defaultSession` explicitly.
  #
  # The entry runs `scoot-session`, not `scoot --tty` directly: the
  # launcher (`resources/scoot-session`, shipped beside the binary)
  # imports the login environment into the user manager, starts
  # `scoot.service`, waits for IPC readiness, imports `WAYLAND_DISPLAY`,
  # `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE` into the manager and the
  # D-Bus activation environment, and only then starts `scoot-session.target` (which
  # pulls in `graphical-session.target`, so the target is reached with
  # the display already imported), and on exit stops the session
  # targets so session-bound units stop. See site/src/content/docs/desktop/index.md#the-greeter for the whole flow.
  #
  # With the privilege prompt on, the entry also names the polkit
  # agent (`SCOOT_POLKIT_AGENT`, an `env` prefix before the launcher):
  # the leader spawns it in-scope once the display is known, which a
  # user unit could never do (see `auth-home.nix`). A set
  # `session.command` replaces the whole line (agent env included),
  # so a hand entry carries the agent by setting the variable itself.
  sessionPackage =
    (pkgs.writeTextDir "share/wayland-sessions/scoot.desktop" ''
      [Desktop Entry]
      Name=scoot
      Comment=scoot scrolling-tiling Wayland compositor
      Exec=${
        if cfg.session.command == null then
          (lib.optionalString (
            cfg.desktop.auth.enable && cfg.desktop.auth.package != null
          ) "${lib.getExe' pkgs.coreutils "env"} SCOOT_POLKIT_AGENT=${agentCommand} ")
          + "${cfg.package}/bin/scoot-session"
        else
          cfg.session.command
      }
      Type=Application
      DesktopNames=scoot
    '').overrideAttrs
      (old: {
        # `writeTextDir` names the derivation after no real package;
        # give the session entry a stable name for the store path.
        name = "scoot-wayland-session";
        passthru = (old.passthru or { }) // {
          providedSessions = [ "scoot" ];
        };
      });

  # Whether Stylix's own regreet target would set ReGreet's background
  # (see the `background` assertion below): Stylix present, with an
  # image, and its regreet target enabled. Written defensively because
  # Stylix is an optional import that may be absent entirely (then
  # `stylix` is `{}`) or predate the regreet target (then `.regreet` is
  # `{}`) -- every level defaults to off.
  stylixRegreetBackground =
    stylix:
    (stylix.image or null) != null && (((stylix.targets or { }).regreet or { }).enable or false);

  # The user units behind `session.enable`, single-sourced from
  # `resources/systemd/user/`: `@SCOOT_BIN@` becomes this package's
  # binary, so the entry above and the service always launch the same
  # build (including the `scoot-xwayland` wrapper's `PATH` append, when
  # that is the package). Read at eval, like the desktop file: no
  # package rebuild when only the units change.
  sessionUnits = {
    "scoot.service".text = builtins.replaceStrings [ "@SCOOT_BIN@" ] [ "${cfg.package}/bin/scoot" ] (
      builtins.readFile ../../resources/systemd/user/scoot.service
    );
    # The session and shutdown targets name no binary: no substitution
    # to keep in step, plain text.
    "scoot-session.target".text = builtins.readFile ../../resources/systemd/user/scoot-session.target;
    "scoot-shutdown.target".text = builtins.readFile ../../resources/systemd/user/scoot-shutdown.target;
  };

  # The polkit agent's command for the session entry's
  # `SCOOT_POLKIT_AGENT` below (which the session leader spawns
  # in-scope -- a user unit could never register, see
  # `auth-home.nix`): absolute store path when the slot names its
  # package, bare name from PATH otherwise (off Linux, or a
  # direct-module setup without the overlay). A null beside `enable`
  # is the slot's own loud assertion, not a throw here: the entry
  # below only names it beside a non-null package.
  agentCommand =
    if cfg.desktop.auth.package != null then
      if cfg.desktop.auth.daemon == "gnome" then
        "${cfg.desktop.auth.package}/libexec/polkit-gnome-authentication-agent-1"
      else if cfg.desktop.auth.daemon == "lxqt" then
        lib.getExe' cfg.desktop.auth.package "lxqt-policykit-agent"
      else
        lib.getExe cfg.desktop.auth.package
    else if cfg.desktop.auth.daemon == "gnome" then
      "polkit-gnome-authentication-agent-1"
    else if cfg.desktop.auth.daemon == "lxqt" then
      "lxqt-policykit-agent"
    else
      "hyprpolkitagent";

  # The desk-aware charge-limit script, with the charge option values
  # baked in (see `power-charge.nix`): the service below runs its
  # `sync`, the bar button its `toggle`, and it installs on PATH
  # beside them. Lazy, like everything else here: referenced only
  # while `power.chargeLimit.enable` is on, so a config without the
  # policy never builds it.
  scootCharge = import ./power-charge.nix {
    inherit pkgs lib;
    inherit (cfg.desktop.power.chargeLimit)
      limit
      fullAfter
      tripEndsAfter
      battery
      ;
  };
in
{
  # nixpkgs' own ReGreet module, so `services.displayManager.regreet`
  # exists wherever this module is used -- including standalone
  # option-only evaluations that never import a full NixOS (like
  # `nix/tests.nix`): without it, the `greeter.enable` wiring below
  # would set an undeclared option. Already imported on real NixOS (it
  # is a default module there), where a repeated import is a no-op. The
  # path is pinned-rev: a consumer's nixpkgs predating
  # `services/display-managers/regreet.nix` fails the import loudly,
  # naming the file. Reached through `modulesPath`, never `pkgs`: on
  # real NixOS `pkgs` is a `_module.args` value, which `imports` cannot
  # read without infinite recursion (it is computed from the very
  # modules being imported); `modulesPath` is a `specialArgs` value
  # every NixOS evaluation provides, and the path it gives is the one
  # NixOS lists itself, so the repeat import dedups.
  #
  # Plus the desktop profile's greeter half as an alias: `desktop.greeter`
  # IS `programs.scoot.greeter` (passthrough, not a copy), so its
  # assertions, its forced session entry and its never-strand shape all
  # hold unchanged through the profile's name for it.
  imports = [
    "${modulesPath}/services/display-managers/regreet.nix"
    (lib.mkAliasOptionModule
      [ "programs" "scoot" "desktop" "greeter" ]
      [ "programs" "scoot" "greeter" ]
    )
  ];

  options.programs.scoot = {
    enable = lib.mkEnableOption "scoot, the scrolling-tiling Wayland compositor";

    # One switch plus a look choice for a working desktop (see
    # `desktop.nix` and site/src/content/docs/desktop/index.md). Each side wires only what it owns;
    # this side owns the session entry, the system packages and the
    # greeter (aliased above). The compositor config itself
    # (`[appearance]`, `[wallpaper]`, the `[xwayland]` knob, the
    # `[binds]` keymap) is the home-manager side's, and the bar is the
    # bar module's (which reads this profile): this side renders no
    # config file.
    desktop = desktop.options // {
      # The shared keymap's tool packages: the shapes are in
      # `desktop.nix` (shared with the home-manager side) and the
      # `[binds]` they run are that side's (`keys-home.nix`); this
      # side installs the tools system-wide. Same packages as there,
      # so either side alone names the same tools. Merged here (not
      # declared separately below) because one module cannot declare
      # the same option path twice. The tools are Linux-only: off
      # Linux each defaults to null and nothing installs.
      keys = desktop.options.keys // {
        brightnessPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.brightnessctl or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.brightnessctl or null else null";
          description = ''
            The backlight tool to install system-wide for the
            brightness binds. Null installs nothing. Linux-only:
            null off Linux.
          '';
        };

        volumePackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.wireplumber or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.wireplumber or null else null";
          description = ''
            The audio tool to install system-wide for the volume
            and mute binds (`wpctl`). Null installs nothing.
            Linux-only: null off Linux.
          '';
        };

        mediaPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.playerctl or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.playerctl or null else null";
          description = ''
            The MPRIS tool to install system-wide for the media
            binds. Null installs nothing. Linux-only: null off
            Linux.
          '';
        };
      };
      # The idle policy's tool packages: the shapes are in `desktop.nix`
      # (shared with the home-manager side) and the user units that run
      # them are that side's (`idle-home.nix`); this side installs the
      # tools system-wide. Same packages as there, so either side alone
      # names the same tools. Merged here (not declared separately
      # below) because one module cannot declare the same option path
      # twice. The tools are Linux-only: off Linux each defaults to
      # null (their attributes exist on Darwin but refuse evaluation
      # when forced), which the assertions below refuse loudly.
      idle = desktop.options.idle // {
        package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.swayidle or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.swayidle or null else null";
          description = ''
            The swayidle package to install system-wide for the idle
            policy. Null installs nothing. Linux-only: null off Linux.
          '';
        };

        dimPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.brightnessctl or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.brightnessctl or null else null";
          description = ''
            The backlight tool to install system-wide for the dim step.
            Null installs nothing. Linux-only: null off Linux.
          '';
        };

        offPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.wlopm or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.wlopm or null else null";
          description = ''
            The output-power tool to install system-wide for the
            screens-off step. Null installs nothing. Linux-only: null
            off Linux.
          '';
        };

        mediaInhibit = desktop.options.idle.mediaInhibit // {
          package = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.sway-audio-idle-inhibit or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.sway-audio-idle-inhibit or null else null";
            description = ''
              The audio inhibitor to install system-wide. Null installs
              nothing. Linux-only: null off Linux.
            '';
          };
        };

        lock = desktop.options.idle.lock // {
          package = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.swaylock or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.swaylock or null else null";
            description = ''
              The locker package to install system-wide (must speak the
              daemon's flags). Null installs nothing. Linux-only: null
              off Linux.
            '';
          };

          command = lib.mkOption {
            type = lib.types.str;
            default =
              if pkgs.stdenv.hostPlatform.isLinux then
                "${lib.getExe' pkgs.systemd "loginctl"} lock-session"
              else
                "loginctl lock-session";
            defaultText = lib.literalExpression ''if pkgs.stdenv.hostPlatform.isLinux then "''${lib.getExe' pkgs.systemd "loginctl"} lock-session" else "loginctl lock-session"'';
            example = "loginctl lock-session";
            description = ''
              The stable lock action: the same default and meaning as
              the home-manager side's `idle.lock.command` (the idle
              timeout, the lid and manual locks all share this path
              through logind).
            '';
          };
        };
      };

      # The notification daemon's package: the shape is in
      # `desktop.nix` (shared with the home-manager side) and the user
      # unit that runs it is that side's (`notifications-home.nix`);
      # this side installs it system-wide. The same package as there,
      # so either side alone names the same daemon. Merged here for
      # the same one-declaration reason as above. Linux-only: off
      # Linux it defaults to null, which the assertion below refuses
      # loudly.
      notifications = desktop.options.notifications // {
        package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default =
            if pkgs.stdenv.hostPlatform.isLinux then
              (if pkgs ? mako then import ./notifications-mako.nix { inherit pkgs; } else null)
            else
              null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then leanMako else null";
          description = ''
            The mako package to install system-wide for the
            notification daemon (a lean mako without the GTK stack,
            the same default as the home-manager side -- see
            `notifications-mako.nix`). Null installs nothing.
            Linux-only: null off Linux.
          '';
        };
      };

      # The launcher's package: the shape is in `desktop.nix`
      # (shared with the home-manager side) and the wrapper script that
      # runs it is that side's (`keys-home.nix`); this side installs it
      # system-wide. The same package as there, so either side alone
      # names the same launcher. Merged here for the same
      # one-declaration reason as above. Linux-only: off Linux it
      # defaults to null, which the assertion below refuses loudly.
      launcher = desktop.options.launcher // {
        package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null";
          description = ''
            The launcher package to install system-wide (fuzzel).
            Null installs nothing. Linux-only: null off Linux.
          '';
        };
      };

      # The clipboard's packages: the shapes are in `desktop.nix`
      # (shared with the home-manager side) and the watcher units that
      # run them are that side's (`clipboard-home.nix`); this side
      # installs them system-wide. The same packages as there, so
      # either side alone names the same tools. Merged here for the
      # same one-declaration reason as above. Linux-only: off Linux
      # each defaults to null, which the assertions below refuse
      # loudly.
      clipboard = desktop.options.clipboard // {
        managerPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default =
            if pkgs.stdenv.hostPlatform.isLinux then
              (if pkgs ? cliphist then import ./clipboard-cliphist.nix { inherit pkgs; } else null)
            else
              null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then leanCliphist else null";
          description = ''
            The clipboard manager to install system-wide (a lean
            cliphist without its contrib pickers, the same default as
            the home-manager side -- see `clipboard-cliphist.nix`).
            Null installs nothing. Linux-only: null off Linux.
          '';
        };

        wlClipboardPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.wl-clipboard or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.wl-clipboard or null else null";
          description = ''
            The copy/paste tools to install system-wide
            (`wl-copy`/`wl-paste`). Null installs nothing.
            Linux-only: null off Linux.
          '';
        };

        menuPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null";
          description = ''
            The dmenu-style menu to install system-wide for the
            history picker (`fuzzel`). Null installs nothing.
            Linux-only: null off Linux.
          '';
        };
      };

      # The capture slot's packages: the value shapes are in
      # `desktop.nix` (shared with the home-manager side) and the
      # chooser config that names them is that side's
      # (`capture-home.nix`); this side installs the portal backends
      # and the screenshot tools system-wide, so the D-Bus-activated
      # backends find them. The same screenshot packages as there, so
      # either side alone names the same tools. Merged here for the
      # same one-declaration reason as above. Linux-only: off Linux
      # each defaults to null, which the assertions below refuse
      # loudly.
      capture = desktop.options.capture // {
        portalWlrPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.xdg-desktop-portal-wlr or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.xdg-desktop-portal-wlr or null else null";
          description = ''
            The ScreenCast/Screenshot portal backend (0.8.4 or later:
            older releases need the wlr screencopy protocol scoot
            omits on purpose, and 0.8.3 stalls recordings). Null
            installs nothing. Linux-only: null off Linux.
          '';
        };

        portalGtkPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.xdg-desktop-portal-gtk or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.xdg-desktop-portal-gtk or null else null";
          description = ''
            The fallback portal backend (file chooser and the rest).
            Null installs nothing. Linux-only: null off Linux.
          '';
        };

        grimPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.grim or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.grim or null else null";
          description = ''
            The screenshot tool to install system-wide (1.5.0 or
            later, which speaks ext-image-copy-capture): the portal's
            Screenshot backend shells out to it. Null installs
            nothing. Linux-only: null off Linux.
          '';
        };

        slurpPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.slurp or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.slurp or null else null";
          description = ''
            The region picker to install system-wide. Null installs
            nothing. Linux-only: null off Linux.
          '';
        };

        menuPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null";
          description = ''
            The dmenu-style menu to install system-wide for the
            output chooser (`fuzzel`, unthemed on this side: the
            themed flags are the home-manager side's). Null installs
            nothing. Linux-only: null off Linux.
          '';
        };
      };

      # The apps slot's packages: the shapes are in `desktop.nix`
      # (shared with the home-manager side) and the binds, the
      # `xdg-open` plumbing and the picker scripts that run them are
      # that side's (`apps-home.nix` and the keymap); this side
      # installs them system-wide. The same packages as there, so
      # either side alone names the same tools. Merged here for the
      # same one-declaration reason as above. Linux-only: off Linux
      # each defaults to null, which the assertions below refuse
      # loudly. The WiFi/Bluetooth tools never touch their services:
      # installing the CLI is not a takeover (no
      # `networking.networkmanager`, no Bluetooth hardware switch --
      # pinned in `nix/tests.nix`).
      apps = desktop.options.apps // {
        terminal = desktop.options.apps.terminal // {
          package = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.foot or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.foot or null else null";
            description = ''
              The terminal to install system-wide (foot: what the
              compositor's built-in `Super+Return` bind spawns). Null
              installs nothing. Linux-only: null off Linux.
            '';
          };
        };

        fileManager = desktop.options.apps.fileManager // {
          package = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.pcmanfm or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.pcmanfm or null else null";
            description = ''
              The file manager to install system-wide (pcmanfm).
              Null installs nothing. Linux-only: null off Linux.
            '';
          };
        };

        network = desktop.options.apps.network // {
          cliPackage = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.networkmanager or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.networkmanager or null else null";
            description = ''
              The WiFi tool to install system-wide (`nmcli`: the
              picker lists and connects through it). Null installs
              nothing. Linux-only: null off Linux.
            '';
          };

          menuPackage = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null";
            description = ''
              The dmenu-style menu to install system-wide for the
              WiFi picker (`fuzzel`). Null installs nothing.
              Linux-only: null off Linux.
            '';
          };
        };

        bluetooth = desktop.options.apps.bluetooth // {
          cliPackage = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.bluez or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.bluez or null else null";
            description = ''
              The Bluetooth tool to install system-wide
              (`bluetoothctl`: the picker lists and connects through
              it). Null installs nothing. Linux-only: null off
              Linux.
            '';
          };

          menuPackage = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel or null else null";
            description = ''
              The dmenu-style menu to install system-wide for the
              Bluetooth picker (`fuzzel`). Null installs nothing.
              Linux-only: null off Linux.
            '';
          };
        };
      };

      # The automounter's package: the shape is in `desktop.nix`
      # (shared with the home-manager side) and the user unit that
      # runs it is that side's (`apps-home.nix`); this side installs
      # it system-wide and runs the udisks2 daemon it mounts
      # through. Merged here for the same one-declaration reason as
      # above. Linux-only: off Linux it defaults to null, which the
      # assertion below refuses loudly.
      automount = desktop.options.automount // {
        package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.udiskie or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.udiskie or null else null";
          description = ''
            The automounter to install system-wide (udiskie, run
            trayless by the home-manager side's unit). Null installs
            nothing. Linux-only: null off Linux.
          '';
        };
      };

      # The privilege prompt's agent package: the shape is in
      # `desktop.nix` (shared with the home-manager side) and the user
      # unit that runs it is that side's (`auth-home.nix`); this side
      # installs it system-wide. The same pick as there (following
      # `daemon`), so either side alone names the same agent. Merged
      # here for the same one-declaration reason as above.
      # Linux-only: off Linux it defaults to null, which the
      # assertion below refuses loudly.
      auth = desktop.options.auth // {
        package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default =
            if pkgs.stdenv.hostPlatform.isLinux then
              (
                {
                  gnome = if pkgs ? polkit_gnome then pkgs.polkit_gnome else null;
                  lxqt = if (pkgs.lxqt or { }) ? lxqt-policykit then pkgs.lxqt.lxqt-policykit else null;
                  hyprpolkit = if pkgs ? hyprpolkitagent then pkgs.hyprpolkitagent else null;
                }
                .${cfg.desktop.auth.daemon} or null
              )
            else
              null;
          defaultText = lib.literalExpression "per-daemon (polkit_gnome, lxqt-policykit or hyprpolkitagent)";
          description = ''
            The polkit agent package to install system-wide (following
            the `daemon` pick, the same default as the home-manager
            side). Null installs nothing. Linux-only: null off Linux.
          '';
        };
      };

      # The secrets slot's packages: the shapes are in `desktop.nix`
      # (shared with the home-manager side); this side installs the
      # daemon system-wide and publishes its D-Bus activation files,
      # so the daemon starts on the first secrets call. The same
      # packages as there, so either side alone names the same
      # daemon. Merged here for the same one-declaration reason as
      # above. Linux-only: off Linux each defaults to null, which
      # the assertions below refuse loudly.
      secrets = desktop.options.secrets // {
        package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.gnome-keyring or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.gnome-keyring else null";
          description = ''
            The keyring package to install system-wide (its daemon,
            its PAM module, its D-Bus activation files). Null installs
            nothing. Linux-only: null off Linux.
          '';
        };

        clientPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.libsecret or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.libsecret else null";
          description = ''
            The secrets client to install system-wide (`secret-tool`,
            from libsecret). Null installs nothing. Linux-only: null
            off Linux.
          '';
        };
      };

      # The night light's tool: the shape is in `desktop.nix`
      # (shared with the home-manager side) and the user unit that runs
      # it is that side's (`nightlight-home.nix`); this side installs it
      # system-wide. The same daemon-following default as there, so
      # either side alone names the same tool. Merged here for the same
      # one-declaration reason as above. Linux-only: off Linux it
      # defaults to null, which the assertion below refuses loudly.
      nightlight = desktop.options.nightlight // {
        package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default =
            if !pkgs.stdenv.hostPlatform.isLinux then
              null
            else if cfg.desktop.nightlight.daemon == "gammastep" then
              (if pkgs ? gammastep then pkgs.gammastep else null)
            else
              (if pkgs ? wlsunset then pkgs.wlsunset else null);
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then (wlsunset for daemon 'wlsunset', gammastep for 'gammastep') else null";
          description = ''
            The night-light tool to install system-wide (the daemon's
            own: wlsunset for `daemon = "wlsunset"`, gammastep for
            `daemon = "gammastep"` -- the unit calls it with that
            daemon's flags, so the two must agree). Null installs
            nothing. Linux-only: null off Linux.
          '';
        };
      };

      # The power policy's daemon package: the shape is in `desktop.nix`
      # (shared with the home-manager side) and the profile switch that
      # calls it is that side's (`keys-home.nix`); this side runs the
      # daemon system-wide. The same package as there, so either side
      # alone names the same daemon. Merged here for the same
      # one-declaration reason as above. Linux-only: off Linux it
      # defaults to null, which the assertion below refuses loudly.
      power = desktop.options.power // {
        profiles = desktop.options.power.profiles // {
          package = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.power-profiles-daemon or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.power-profiles-daemon or null else null";
            description = ''
              The power-profiles-daemon package to run system-wide
              (its `powerprofilesctl` is the profile switch's CLI).
              Null installs nothing. Linux-only: null off Linux.
            '';
          };
        };
      };
      # The audio slot's packages: the value shapes are in
      # `desktop.nix` (shared with the home-manager side) and the OSD
      # unit that runs them is that side's (`audio-home.nix`); this
      # side installs the OSD system-wide, so a hand-written setup
      # finds it the way it finds the launcher binary without its
      # wrapper. The same packages as there, so either side alone
      # names the same tools. Merged here for the same
      # one-declaration reason as above. Linux-only: off Linux each
      # defaults to null, which the assertions below refuse loudly.
      # `osd` is re-merged, not replaced: a shallow `//` would drop the
      # shared `osd.timeoutMs` from the NixOS-side declaration (the same
      # reason the idle slot re-merges `mediaInhibit` and `lock`).
      audio = desktop.options.audio // {
        osd = desktop.options.audio.osd // {
          package = lib.mkOption {
            type = lib.types.nullOr lib.types.package;
            default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.wob or null else null;
            defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.wob or null else null";
            description = ''
              The on-screen display to install system-wide (wob). Null
              installs nothing. Linux-only: null off Linux.
            '';
          };
        };

        dumpPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.pipewire or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.pipewire else null";
          description = ''
            The PipeWire package to install system-wide for the sink
            helper's `pw-dump`. Null installs nothing. Linux-only:
            null off Linux.
          '';
        };
      };
      # The theme's packages: the value shapes are in `desktop.nix`
      # (shared with the home-manager side) and the files that use
      # them are that side's (`theme-home.nix`); this side declares
      # the same paths (so either side alone names the same theme)
      # and installs what the greeter itself needs. The same packages
      # as there, so a combined setup agrees. Merged here for the
      # same one-declaration reason as above. Linux-only: off Linux
      # each defaults to null.
      theme = desktop.options.theme // {
        cursor.package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.vanilla-dmz or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.vanilla-dmz else null";
          description = ''
            The cursor theme to install system-wide for the look
            (Vanilla-DMZ). Null installs nothing. Linux-only: null
            off Linux.
          '';
        };

        icon.package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.adwaita-icon-theme or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.adwaita-icon-theme else null";
          description = ''
            The icon theme to install system-wide for the look
            (Adwaita). Null installs nothing. Linux-only: null off
            Linux.
          '';
        };

        fonts.uiPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default =
            if pkgs.stdenv.hostPlatform.isLinux then pkgs.nerd-fonts.droid-sans-mono or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.nerd-fonts.droid-sans-mono else null";
          description = ''
            The look's bar face to install system-wide. Null installs
            nothing. Linux-only: null off Linux.
          '';
        };

        fonts.sansPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then pkgs.dejavu_fonts.minimal or null else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.dejavu_fonts.minimal else null";
          description = ''
            The look's proportional UI sans to install system-wide
            (the greeter's font). Null installs nothing. Linux-only:
            null off Linux.
          '';
        };

        fonts.monoPackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default =
            if pkgs.stdenv.hostPlatform.isLinux then
              (
                if cfg.desktop.look == "radial-burst" then
                  (if pkgs ? dejavu_fonts then pkgs.dejavu_fonts else null)
                else
                  (pkgs.nerd-fonts.fira-code or null)
              )
            else
              null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then per-look monospace package else null";
          description = ''
            The look's terminal face to install system-wide.
            Null installs nothing. Linux-only: null off Linux.
          '';
        };

        qt.package = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = if pkgs.stdenv.hostPlatform.isLinux then (pkgs.qt6Packages.qt6ct or null) else null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.qt6Packages.qt6ct else null";
          description = ''
            The Qt config tool to install system-wide (no daemon:
            Qt reads its file at startup). Null installs nothing.
            Linux-only: null off Linux.
          '';
        };

        qt.stylePackage = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default =
            if pkgs.stdenv.hostPlatform.isLinux then
              (if pkgs ? adwaita-qt6 then pkgs.adwaita-qt6 else null)
            else
              null;
          defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.adwaita-qt6 else null";
          description = ''
            The Adwaita Qt style to install system-wide. Null
            installs nothing. Linux-only: null off Linux.
          '';
        };
      };
    };

    # `pkgs.scoot` when the flake's overlay (`overlays.default`) is
    # applied, else null: nothing is guessed, since a `scoot` from anywhere
    # else would silently install someone else's build. The flake wrapper
    # (`nixosModules.scoot` in `flake.nix`) fills this with the flake's own
    # build via `mkDefault` either way.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scoot or null;
      defaultText = lib.literalExpression "pkgs.scoot or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scoot";
      description = ''
        The scoot package to install system-wide and to launch from
        the login-screen session entry.
      '';
    };

    # scootbg, the wallpaper daemon a `[wallpaper]` config section runs.
    # The config file itself is per-user (the home-manager module); this
    # only guarantees the binary, on the system PATH, where the default
    # `[wallpaper] command = "scootbg"` finds it -- for any user, and for a
    # session the greeter starts.
    wallpaper = {
      # On whenever scoot is and there is a scootbg to install: a ~1.7 MB
      # binary that does nothing until a config asks for a wallpaper, so
      # installing it changes nothing -- unlike the login entry, which
      # stays opt-in. With no package (a direct-module user without the
      # flake's overlay) it stays off rather than failing evaluation, so
      # upgrading breaks nobody; `true` set explicitly with no package is
      # the loud assertion below. `false` opts out (another wallpaper
      # daemon, say).
      enable = lib.mkOption {
        type = lib.types.bool;
        default = cfg.enable && cfg.wallpaper.package != null;
        defaultText = lib.literalExpression "config.programs.scoot.enable && config.programs.scoot.wallpaper.package != null";
        description = ''
          Install `programs.scoot.wallpaper.package` system-wide, so a
          `[wallpaper]` section in a user's scoot config finds `scootbg`
          on PATH. On by default whenever `programs.scoot.enable` is and a
          package is available (the flake's modules and overlay provide
          one); setting it to `true` with no package fails evaluation.
        '';
      };

      # `pkgs.scootbg` with the flake's overlay, else null; the flake
      # wrapper injects the flake's own build (the same revision as
      # `package`, so the `apply-config` scoot speaks is the one scootbg
      # understands). A direct-module user who enables this with neither
      # gets the assertion below at eval, not a session with no wallpaper.
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = pkgs.scootbg or null;
        defaultText = lib.literalExpression "pkgs.scootbg or null";
        example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scootbg";
        description = ''
          The scootbg package to install when `wallpaper.enable` is set.
        '';
      };
    };

    session = {
      # Default OFF, explicitly. Adding the entry is safe (it never
      # replaces the default session -- see above), but a login-screen
      # change is still a change to the user's way back into their own
      # desktop, and per CLAUDE.md's never-strand rule that stays an
      # explicit opt-in, not something `enable = true` smuggles in.
      # `enable` alone installs the binary; the session entry is separate.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Add a scoot entry to the display-manager/greetd session menu,
          alongside existing sessions. Never replaces the default
          session. If you use autoLogin, also pin
          `services.displayManager.defaultSession` explicitly (see the
          autologin caveat in the module source).
        '';
      };
      # Full `Exec=` line for the session entry, NOT just a suffix
      # appended after `--` -- deliberately. The issue-#171 acceptance
      # shape is a wrapper script (`Exec=<wrapper>/bin/scoot-session`,
      # which itself runs `scoot --tty -- ...` plus stderr to a log),
      # and a plain `-- COMMAND` append cannot express a wrapper (it
      # would nest scoot inside scoot). A verbatim string expresses all
      # three shapes: the launcher default (null), the common append (write
      # the full `scoot --tty -- ...` line, e.g. pointing at the
      # home-manager module's `sessionScript` output), and a wrapper
      # path. It is a string rather than an argv list for the same
      # reason: an `Exec=` line is a command line, and auto-escaping
      # would fight wrapper paths and per the Desktop Entry Spec the
      # quoting is the author's anyway. The cost is stated in the option
      # description itself (not just here): a
      # set value replaces the whole line, so dropping `--tty` (or the
      # binary path) breaks the entry loudly at the greeter, not at
      # eval -- copy the example shape. The entry stays additive and
      # default-off whatever the value (see above), so a broken line
      # strands nobody: the other sessions remain.
      #
      # A set value bypasses `scoot-session`: only the null default runs
      # the launcher, so only it gets the session wiring (user-manager
      # import, `scoot-session.target` reaching `graphical-session.target`
      # past the display import, the activation environment,
      # teardown). A `scoot --tty -- <script>` value still runs exactly
      # what it says -- a session, just an unwired one -- and startup
      # programs that want the wiring belong in scoot's `[autostart]`,
      # which runs inside it (see site/src/content/docs/scoot/configure.md#autostart).
      command = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = lib.literalExpression ''
          "''${config.programs.scoot.package}/bin/scoot --tty -- /home/alice/.config/scoot/session.sh"
        '';
        description = ''
          Full `Exec=` command line for the login-screen session entry.
          Null (the default) renders `<package>/bin/scoot-session`, the
          session launcher, so existing configs gain the session wiring
          (user-manager import, `scoot-session.target` reaching
          `graphical-session.target` past the display import, the D-Bus
          activation environment, teardown on exit -- see site/src/content/docs/desktop/index.md#the-greeter).
          Set it to run something else instead: the usual shape is
          `<package>/bin/scoot --tty -- <command>`, e.g. the
          home-manager module's `sessionScript` output at
          `/home/alice/.config/scoot/session.sh` (for a user `alice`;
          see `programs.scoot.sessionScript` and site/src/content/docs/scoot/index.md#starting-a-session, which show
          the pairing together). A wrapper script path (logging,
          environment setup) works too -- that is the gh-issue-#171
          acceptance shape. A set value replaces the whole line and
          bypasses the launcher, so a set entry runs without the session
          wiring (for wired startup programs, use `[autostart]`
          instead); it also drops the privilege prompt's
          `SCOOT_POLKIT_AGENT` (a hand entry that wants prompts sets
          that variable itself, to the agent's absolute path, before
          running the launcher); dropping `--tty` (or the binary path) breaks the
          entry loudly at the greeter, not at eval -- copy the example
          shape. `Exec=` lines get no shell expansion (`~` and `$HOME`
          arrive literally), so always use an absolute path.
        '';
      };
    };

    greeter = {
      # Default OFF, explicitly, and more strongly than `session.enable`:
      # this replaces the login screen (greetd running ReGreet under
      # cage, via nixpkgs' own `services.displayManager.regreet`), so it
      # is only ever on when the user sets it. Roll back by turning it
      # off again (or by booting the previous generation): nothing about
      # the previous login screen is uninstalled while it is on, only
      # displaced. See site/src/content/docs/desktop/index.md#the-greeter.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Log in through ReGreet (greetd + cage, nixpkgs' own
          `services.displayManager.regreet`), with scoot in its session
          list. Only ever on when set: this replaces the login screen.
          Conflicts with GDM and SDDM (refused at eval); anything else
          that owns the login screen must be turned off by hand.
        '';
      };

      # ReGreet's backdrop, when the admin wants it to match the session.
      # Null (the default) leaves ReGreet's background alone -- which is
      # also what to do under Stylix with its regreet target enabled:
      # Stylix then sets the background from `stylix.image` itself, and
      # setting both is refused at eval. Otherwise, point this at an
      # image (it is copied to the store) -- usually the same file as
      # the session wallpaper -- and it becomes ReGreet's
      # `background.path`. No `fit` knob here: ReGreet's default stands
      # unless set through `services.displayManager.regreet.settings`
      # directly (which also wins over this path: leave it null then).
      background = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = "/home/alice/Pictures/hills.jpg";
        description = ''
          Background image for the ReGreet login screen
          (`background.path` in its config). Null leaves it alone (and
          is required under Stylix with its regreet target, which sets
          the background from `stylix.image` itself).
        '';
      };
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.wallpaper.enable {
      assertions = [
        {
          # Loud at eval, not a `[wallpaper]` that logs "scootbg was not
          # found" at every login. Reachable only by setting `enable = true`
          # explicitly: the default is off without a package.
          assertion = cfg.wallpaper.package != null;
          message = ''
            programs.scoot.wallpaper.enable is set to true but
            programs.scoot.wallpaper.package is null. Flake consumers get
            the flake's own scootbg through the nixosModules wrapper, and
            the flake's overlay provides pkgs.scootbg -- this fires only
            for direct-module use without either: set
            programs.scoot.wallpaper.package, apply the overlay, or leave
            programs.scoot.wallpaper.enable at its default.
          '';
        }
      ];
      # The `!= null` guard keeps a null out of the list, so the failure is
      # the assertion's message rather than a type error.
      environment.systemPackages = lib.optional (cfg.wallpaper.package != null) cfg.wallpaper.package;
    })
    (lib.mkIf cfg.enable {
      assertions = [
        {
          # Loud at eval, not a broken `.desktop` at login: `Exec=` with
          # a null package would interpolate to nothing and greet
          # whoever picks the entry with a greeter error.
          assertion = cfg.package != null;
          message = ''
            programs.scoot.enable is set but programs.scoot.package is null.
            Flake consumers get the flake's own build by default through the
            nixosModules wrapper, and the flake's overlay provides pkgs.scoot
            -- this fires only for direct-module use without either: set
            programs.scoot.package explicitly.
          '';
        }
        {
          assertion = cfg.session.enable -> cfg.package != null;
          message = ''
            programs.scoot.session.enable needs programs.scoot.package:
            a session entry with no binary to launch would fail at the login screen.
          '';
        }
        {
          # Loud at eval, not an empty `Exec=` at login: an explicitly
          # empty or whitespace-only command would render a .desktop with
          # nothing (or blanks) to launch. (`builtins.match` returns null
          # on no match, so the second disjunct is false exactly for
          # empty/blank strings.)
          # (The default null -- the launcher entry -- never reaches here.)
          assertion =
            cfg.session.command == null || builtins.match "^[[:space:]]*$" cfg.session.command == null;
          message = ''
            programs.scoot.session.command is set but empty or blank:
            either leave it null for the default
            `<package>/bin/scoot-session` launcher entry, or set the full
            Exec= command line to run.
          '';
        }
      ];

      environment.systemPackages = lib.optional (cfg.package != null) cfg.package;

      # The `cfg.package != null` conjunct is load-bearing, not redundant
      # with the assertions above: `sessionPackage` interpolates
      # `cfg.package` into `Exec=`, and a null there fails while
      # evaluating the config value itself -- before `assertions` are
      # checked -- with a bare "cannot coerce null to a string". The
      # conjunct keeps the null out of the interpolation so the failure
      # surfaces as the helpful assertion message instead.
      services.displayManager.sessionPackages = lib.optional (
        cfg.session.enable && cfg.package != null
      ) sessionPackage;

      # The launcher's units, beside the entry that starts them. Same
      # gate and same null guard as the entry: no entry, no units, and
      # never a unit naming a binary that is not there.
      systemd.user.units = lib.mkIf (cfg.session.enable && cfg.package != null) sessionUnits;
    })
    # The desktop profile's system half: `enable` turns on the session
    # wiring (the entry plus the launcher's units) and the wallpaper
    # defaults, each at `mkDefault` so an explicit value still wins. The
    # greeter stays opt-in through `desktop.greeter` (the alias above):
    # replacing the login screen is never smuggled in by this switch.
    # The `[xwayland]` knob itself is config-file (home-manager) wiring;
    # this side honors whatever `package` names (including the
    # `scoot-xwayland` builds) in the entry above. In its own element
    # (not under `cfg.enable`): with `enable` off and `desktop.enable`
    # on, the assertion below must still fire rather than going quiet.
    (lib.mkIf cfg.desktop.enable {
      assertions = [
        {
          assertion = cfg.enable;
          message = ''
            programs.scoot.desktop.enable needs programs.scoot.enable:
            the profile serves a scoot session, so scoot itself must be
            installed.
          '';
        }
      ];

      programs.scoot.session.enable = lib.mkDefault true;
      programs.scoot.wallpaper.enable = lib.mkDefault true;

      # The shared keymap on with the profile (each bind still
      # individually removable, the whole map disable-able at plain
      # priority): the tools below install, the `[binds]` come from
      # the home-manager side.
      programs.scoot.desktop.keys.enable = lib.mkDefault true;

      # The idle policy on with the profile (each still individually
      # disable-able at plain priority): the tools below install, the
      # lid rule locks docked lids, and the user units come from the
      # home-manager side.
      programs.scoot.desktop.idle.enable = lib.mkDefault true;
      programs.scoot.desktop.idle.lock.enable = lib.mkDefault true;
      programs.scoot.desktop.idle.mediaInhibit.enable = lib.mkDefault true;

      # The notification daemon on with the profile (still
      # individually disable-able at plain priority): the package
      # below installs, and the user unit comes from the
      # home-manager side.
      programs.scoot.desktop.notifications.enable = lib.mkDefault true;

      # The launcher slot on with the profile (still individually
      # disable-able at plain priority): the package below installs,
      # and the binds come from the home-manager side.
      programs.scoot.desktop.launcher.enable = lib.mkDefault true;

      # The clipboard slot on with the profile (still individually
      # disable-able at plain priority): the packages below install,
      # and the watcher units come from the home-manager side.
      programs.scoot.desktop.clipboard.enable = lib.mkDefault true;

      # The capture slot on with the profile (still individually
      # disable-able at plain priority): the backends and the tools
      # below install, PipeWire runs, and the chooser config comes
      # from the home-manager side.
      programs.scoot.desktop.capture.enable = lib.mkDefault true;

      # The privilege prompt on with the profile (still individually
      # disable-able at plain priority): the authority below runs,
      # the agent installs, and the unit comes from the
      # home-manager side.
      programs.scoot.desktop.auth.enable = lib.mkDefault true;

      # The keyring on with the profile (still individually
      # disable-able at plain priority): the daemon installs with
      # its D-Bus activation below, primed from the login password
      # on greetd logins (first use unlocks once).
      programs.scoot.desktop.secrets.enable = lib.mkDefault true;

      # The audio slot on with the profile (still individually
      # disable-able at plain priority): the OSD package below
      # installs, PipeWire runs, and the unit comes from the
      # home-manager side.
      programs.scoot.desktop.audio.enable = lib.mkDefault true;

      # The night light on with the profile (still individually
      # disable-able at plain priority): the tool below installs, and
      # the user unit comes from the home-manager side.
      programs.scoot.desktop.nightlight.enable = lib.mkDefault true;

      # The terminal on with the profile (still individually
      # disable-able at plain priority): the package below installs,
      # keeping the compositor's built-in `Super+Return` bind true.
      # The file manager stays optional: nothing references one.
      programs.scoot.desktop.apps.terminal.enable = lib.mkDefault true;

      # Both pickers on with the profile (each still individually
      # disable-able at plain priority): the tools below install,
      # and the binds come from the home-manager side. The CLIs
      # never touch their services (no takeover).
      programs.scoot.desktop.apps.network.enable = lib.mkDefault true;
      programs.scoot.desktop.apps.bluetooth.enable = lib.mkDefault true;

      # The automounter on with the profile (still individually
      # disable-able at plain priority): the daemon below runs and
      # the package installs, and the user unit comes from the
      # home-manager side.
      programs.scoot.desktop.automount.enable = lib.mkDefault true;
    })
    # System dconf for the look's dark-mode signal: the home-manager
    # side writes `dconf.settings` (the `color-scheme` leaf in
    # `theme-home.nix`), and home-manager's dconf activation needs the
    # system's dconf D-Bus service to land it. Gated on the profile,
    # not on this side's look or GTK target: those are separate copies
    # from the home configuration's (a look set only there, or a GTK
    # target off on one side only, would be missed), and upstream's
    # `gtk` module writes the same leaf when it owns it. Cost: nothing
    # started at boot (the dconf service is D-Bus-activated on a
    # write), the `dconf` tool and one GIO module path. A default, as
    # nixpkgs' own `programs/wayland/wayland-session.nix` sets it for
    # sway, niri, Hyprland and the rest, so a plain `false` wins.
    (lib.mkIf cfg.desktop.enable {
      programs.dconf.enable = lib.mkDefault true;
    })
    # The notification daemon's system half: its package on PATH. The
    # unit and the config are the home-manager side's
    # (`notifications-home.nix`): without it the daemon sits ready for
    # a hand-written setup, the way a `[wallpaper]` finds scootbg on
    # PATH without the home-manager side.
    (lib.mkIf cfg.desktop.notifications.enable {
      assertions = [
        {
          assertion = cfg.desktop.notifications.package != null;
          message = ''
            programs.scoot.desktop.notifications.enable is set but
            programs.scoot.desktop.notifications.package is null: set
            it explicitly (apply the overlay, or point at a mako).
          '';
        }
      ];

      environment.systemPackages = lib.optional (
        cfg.desktop.notifications.package != null
      ) cfg.desktop.notifications.package;
    })
    # The launcher's system half: its package on PATH. The wrapper
    # script and the binds are the home-manager side's
    # (`keys-home.nix`): without it the binary sits ready for a
    # hand-written setup, the way a `[wallpaper]` finds scootbg on PATH
    # without the home-manager side.
    (lib.mkIf cfg.desktop.launcher.enable {
      assertions = [
        {
          assertion = cfg.desktop.launcher.package != null;
          message = ''
            programs.scoot.desktop.launcher.enable is set but
            programs.scoot.desktop.launcher.package is null: set it
            explicitly (apply the overlay, or point at a fuzzel).
          '';
        }
      ];

      environment.systemPackages = lib.optional (
        cfg.desktop.launcher.package != null
      ) cfg.desktop.launcher.package;
    })
    # The clipboard's system half: its tools on PATH. The watcher units
    # and the picker are the home-manager side's (`clipboard-home.nix`
    # and the keymap): without it the tools sit ready for a hand-written
    # setup, the way a `[wallpaper]` finds scootbg on PATH without the
    # home-manager side.
    (lib.mkIf cfg.desktop.clipboard.enable {
      assertions = [
        {
          assertion = cfg.desktop.clipboard.managerPackage != null;
          message = ''
            programs.scoot.desktop.clipboard.enable is set but
            programs.scoot.desktop.clipboard.managerPackage is null: set
            it explicitly (apply the overlay, or point at a cliphist).
          '';
        }
        {
          assertion = cfg.desktop.clipboard.wlClipboardPackage != null;
          message = ''
            programs.scoot.desktop.clipboard.enable is set but
            programs.scoot.desktop.clipboard.wlClipboardPackage is null:
            set it explicitly (apply the overlay, or point at a
            wl-clipboard).
          '';
        }
        {
          assertion = cfg.desktop.clipboard.menuPackage != null;
          message = ''
            programs.scoot.desktop.clipboard.enable is set but
            programs.scoot.desktop.clipboard.menuPackage is null: set it
            explicitly (apply the overlay, or point at a fuzzel).
          '';
        }
      ];

      environment.systemPackages =
        lib.optional (cfg.desktop.clipboard.managerPackage != null) cfg.desktop.clipboard.managerPackage
        ++ lib.optional (
          cfg.desktop.clipboard.wlClipboardPackage != null
        ) cfg.desktop.clipboard.wlClipboardPackage
        ++ lib.optional (cfg.desktop.clipboard.menuPackage != null) cfg.desktop.clipboard.menuPackage;
    })
    # The capture slot's system half: the portal backends on their
    # bus names, PipeWire running for the cast, and the screenshot
    # tools on PATH. The chooser config and the screenshot binds are
    # the home-manager side's (`capture-home.nix` and the keymap):
    # without it the backends sit ready for a hand-written setup, the
    # way a `[wallpaper]` finds scootbg on PATH without the
    # home-manager side. The portals start on demand over D-Bus and
    # hold nothing until a cast asks.
    (lib.mkIf cfg.desktop.capture.enable {
      assertions = [
        {
          assertion = cfg.desktop.capture.portalWlrPackage != null;
          message = ''
            programs.scoot.desktop.capture.enable is set but
            programs.scoot.desktop.capture.portalWlrPackage is null:
            set it explicitly (apply the overlay, or point at an
            xdg-desktop-portal-wlr 0.8.4 or later).
          '';
        }
        {
          # Loud at eval, not a cast that binds nothing: older
          # releases need the wlr screencopy protocol scoot omits on
          # purpose (and 0.8.3 stalls recordings), so ScreenCast
          # would fail every time.
          assertion =
            cfg.desktop.capture.portalWlrPackage == null
            || lib.versionAtLeast (cfg.desktop.capture.portalWlrPackage.version or "0") "0.8.4";
          message = ''
            programs.scoot.desktop.capture.portalWlrPackage is too old
            (need 0.8.4 or later): ScreenCast needs
            ext-image-copy-capture (0.8.0 or later), and 0.8.3 stalls
            recordings -- take 0.8.4 or later.
          '';
        }
        {
          assertion = cfg.desktop.capture.portalGtkPackage != null;
          message = ''
            programs.scoot.desktop.capture.enable is set but
            programs.scoot.desktop.capture.portalGtkPackage is null:
            set it explicitly (apply the overlay, or point at an
            xdg-desktop-portal-gtk).
          '';
        }
        {
          assertion = cfg.desktop.capture.grimPackage != null;
          message = ''
            programs.scoot.desktop.capture.enable is set but
            programs.scoot.desktop.capture.grimPackage is null: set it
            explicitly (apply the overlay, or point at a grim 1.5.0 or
            later).
          '';
        }
        {
          assertion =
            cfg.desktop.capture.grimPackage == null
            || lib.versionAtLeast (cfg.desktop.capture.grimPackage.version or "0") "1.5.0";
          message = ''
            programs.scoot.desktop.capture.grimPackage is too old
            (need 1.5.0 or later): only grim 1.5.0 or later speaks
            ext-image-copy-capture, which is what scoot serves.
          '';
        }
        {
          assertion = cfg.desktop.capture.slurpPackage != null;
          message = ''
            programs.scoot.desktop.capture.enable is set but
            programs.scoot.desktop.capture.slurpPackage is null: set
            it explicitly (apply the overlay, or point at a slurp).
          '';
        }
        {
          assertion = cfg.desktop.capture.chooser != "fuzzel" || cfg.desktop.capture.menuPackage != null;
          message = ''
            programs.scoot.desktop.capture.chooser is "fuzzel" but
            programs.scoot.desktop.capture.menuPackage is null: set it
            explicitly (apply the overlay, or point at a fuzzel).
          '';
        }
      ];

      # The portal service with both backends. The `scoot` backend
      # selection mirrors `resources/scoot-portals.conf` (ScreenCast
      # and Screenshot to `wlr`, everything else to `gtk`): this
      # system file covers sessions without home-manager, and the
      # home-manager side's per-user file wins where both exist (it
      # is the higher-precedence lookup slot).
      xdg.portal.enable = lib.mkDefault true;
      # The `!= null` guards keep a null out of the list, so a
      # missing backend fails with the slot's own assertion message
      # above rather than a type error (the same guard the
      # `systemPackages` lists below use).
      xdg.portal.extraPortals =
        lib.optional (cfg.desktop.capture.portalWlrPackage != null) cfg.desktop.capture.portalWlrPackage
        ++ lib.optional (cfg.desktop.capture.portalGtkPackage != null) cfg.desktop.capture.portalGtkPackage;
      xdg.portal.config.scoot = {
        default = [ "gtk" ];
        "org.freedesktop.impl.portal.Screenshot" = [ "wlr" ];
        "org.freedesktop.impl.portal.ScreenCast" = [ "wlr" ];
      };

      # PipeWire running for the cast (the session manager rides
      # along: `wireplumber.enable` defaults to this). Plain
      # `mkDefault`, so an explicit value still wins -- and the audio
      # slot defaults the same switch, which merges rather than
      # conflicts.
      services.pipewire.enable = lib.mkDefault true;

      environment.systemPackages =
        lib.optional (cfg.desktop.capture.portalWlrPackage != null) cfg.desktop.capture.portalWlrPackage
        ++ lib.optional (cfg.desktop.capture.portalGtkPackage != null) cfg.desktop.capture.portalGtkPackage
        ++ lib.optional (cfg.desktop.capture.grimPackage != null) cfg.desktop.capture.grimPackage
        ++ lib.optional (cfg.desktop.capture.slurpPackage != null) cfg.desktop.capture.slurpPackage
        ++ lib.optional (cfg.desktop.capture.menuPackage != null) cfg.desktop.capture.menuPackage;
    })
    # xdpw's own config, system-wide: the same chooser the
    # home-manager side writes per desktop (unthemed here -- the look
    # is per-user, so the themed flags live only in that file, which
    # wins where both exist). Without home-manager this is what a
    # cast asks through; with it, the fallback nobody reads.
    (lib.mkIf cfg.desktop.capture.enable {
      environment.etc."xdg/xdg-desktop-portal-wlr/config".text =
        let
          cap = cfg.desktop.capture;
          binOr = name: pkg: if pkg != null then "${lib.getExe' pkg name}" else name;
          menuBin = binOr "fuzzel" cap.menuPackage;
          slurpBin = binOr "slurp" cap.slurpPackage;
          # A wrapper script beside the keymap's launcher script, for
          # the same reason as the home-manager side's: xdpw reads
          # its config through inih (200-character lines), which
          # would cut a fully-flagged `chooser_cmd` mid-flag. (This
          # side carries no theme -- the look is per-user, so the
          # themed flags live only in the per-desktop file, which
          # wins where both exist.)
          chooserScript = pkgs.writeShellScriptBin "scoot-screencast-chooser" ''
            exec ${menuBin} --dmenu --prompt='Share: '
          '';
          chooserCmd =
            if cap.chooser == "fuzzel" then
              "${chooserScript}/bin/scoot-screencast-chooser"
            else if cap.chooser == "slurp" then
              "${slurpBin} -f 'Monitor: %o' -or"
            else
              null;
        in
        lib.concatStringsSep "\n" (
          [
            "# Generated by programs.scoot.desktop.capture (system fallback: the per-user xdg-desktop-portal-wlr/scoot wins)."
          ]
          ++ [ "[screencast]" ]
          ++
            lib.optional (cap.chooser != "none")
              "chooser_type=${if cap.chooser == "fuzzel" then "dmenu" else "simple"}"
          ++ lib.optional (chooserCmd != null) "chooser_cmd=${chooserCmd}"
          ++ lib.optional (cap.chooser == "none") "chooser_type=none"
          ++ lib.optional (cap.chooser == "none" && cap.outputName != null) "output_name=${cap.outputName}"
          ++ [ "max_fps=${toString cap.maxFps}" ]
        )
        + "\n";
    })
    # The apps slot's system half: the terminal, the file manager and
    # both pickers' tools on PATH. The binds, the `xdg-open` plumbing
    # and the picker scripts are the home-manager side's
    # (`apps-home.nix` and the keymap): without it the tools sit ready
    # for a hand-written setup, the way a `[wallpaper]` finds scootbg
    # on PATH without the home-manager side. The WiFi/Bluetooth CLIs
    # never touch their services (no `networking.networkmanager`, no
    # Bluetooth hardware switch anywhere in this module -- pinned in
    # `nix/tests.nix`).
    (lib.mkIf cfg.desktop.apps.terminal.enable {
      assertions = [
        {
          assertion = cfg.desktop.apps.terminal.package != null;
          message = ''
            programs.scoot.desktop.apps.terminal.enable is set but
            programs.scoot.desktop.apps.terminal.package is null: set
            it explicitly (apply the overlay, or point at a foot).
          '';
        }
      ];

      environment.systemPackages = lib.optional (
        cfg.desktop.apps.terminal.package != null
      ) cfg.desktop.apps.terminal.package;
    })
    (lib.mkIf cfg.desktop.apps.fileManager.enable {
      assertions = [
        {
          assertion = cfg.desktop.apps.fileManager.package != null;
          message = ''
            programs.scoot.desktop.apps.fileManager.enable is set but
            programs.scoot.desktop.apps.fileManager.package is null:
            set it explicitly (apply the overlay, or point at a file
            manager).
          '';
        }
      ];

      environment.systemPackages = lib.optional (
        cfg.desktop.apps.fileManager.package != null
      ) cfg.desktop.apps.fileManager.package;
    })
    (lib.mkIf cfg.desktop.apps.network.enable {
      assertions = [
        {
          assertion = cfg.desktop.apps.network.cliPackage != null;
          message = ''
            programs.scoot.desktop.apps.network.enable is set but
            programs.scoot.desktop.apps.network.cliPackage is null:
            set it explicitly (apply the overlay, or point at a
            networkmanager).
          '';
        }
        {
          assertion = cfg.desktop.apps.network.menuPackage != null;
          message = ''
            programs.scoot.desktop.apps.network.enable is set but
            programs.scoot.desktop.apps.network.menuPackage is null:
            set it explicitly (apply the overlay, or point at a
            fuzzel).
          '';
        }
      ];

      environment.systemPackages =
        lib.optional (cfg.desktop.apps.network.cliPackage != null) cfg.desktop.apps.network.cliPackage
        ++ lib.optional (cfg.desktop.apps.network.menuPackage != null) cfg.desktop.apps.network.menuPackage;
    })
    (lib.mkIf cfg.desktop.apps.bluetooth.enable {
      assertions = [
        {
          assertion = cfg.desktop.apps.bluetooth.cliPackage != null;
          message = ''
            programs.scoot.desktop.apps.bluetooth.enable is set but
            programs.scoot.desktop.apps.bluetooth.cliPackage is null:
            set it explicitly (apply the overlay, or point at a
            bluez).
          '';
        }
        {
          assertion = cfg.desktop.apps.bluetooth.menuPackage != null;
          message = ''
            programs.scoot.desktop.apps.bluetooth.enable is set but
            programs.scoot.desktop.apps.bluetooth.menuPackage is null:
            set it explicitly (apply the overlay, or point at a
            fuzzel).
          '';
        }
      ];

      environment.systemPackages =
        lib.optional (cfg.desktop.apps.bluetooth.cliPackage != null) cfg.desktop.apps.bluetooth.cliPackage
        ++ lib.optional (
          cfg.desktop.apps.bluetooth.menuPackage != null
        ) cfg.desktop.apps.bluetooth.menuPackage;
    })
    # The automounter's system half: the udisks2 daemon it mounts
    # through, and its own package on PATH. The trayless unit is the
    # home-manager side's (`apps-home.nix`): without it the daemon
    # sits ready for a hand-written setup, the way a `[wallpaper]`
    # finds scootbg on PATH without the home-manager side.
    (lib.mkIf cfg.desktop.automount.enable {
      assertions = [
        {
          assertion = cfg.desktop.automount.package != null;
          message = ''
            programs.scoot.desktop.automount.enable is set but
            programs.scoot.desktop.automount.package is null: set it
            explicitly (apply the overlay, or point at a udiskie).
          '';
        }
      ];

      services.udisks2.enable = lib.mkDefault true;

      environment.systemPackages = lib.optional (
        cfg.desktop.automount.package != null
      ) cfg.desktop.automount.package;
    })
    # The night light's system half: its tool on PATH. The unit is the
    # home-manager side's (`nightlight-home.nix`): without it the tool
    # sits ready for a hand-written setup, the way a `[wallpaper]` finds
    # scootbg on PATH without the home-manager side.
    (lib.mkIf cfg.desktop.nightlight.enable {
      assertions = [
        {
          assertion = cfg.desktop.nightlight.package != null;
          message = ''
            programs.scoot.desktop.nightlight.enable is set but
            programs.scoot.desktop.nightlight.package is null: set it
            explicitly (apply the overlay, or point at a wlsunset for
            daemon "wlsunset" or a gammastep for "gammastep").
          '';
        }
      ];

      environment.systemPackages = lib.optional (
        cfg.desktop.nightlight.package != null
      ) cfg.desktop.nightlight.package;
    })
    # The privilege prompt's system half: the polkit authority, the
    # agent on PATH, and its command in the session entry (which the
    # session leader spawns in-scope -- a user unit could never
    # register, see `auth-home.nix`). Without the home-manager side
    # the authority answers and the agent binary sits ready for a
    # hand-written setup, the way a `[wallpaper]` finds scootbg on
    # PATH without the home-manager side.
    (lib.mkIf cfg.desktop.auth.enable {
      assertions = [
        {
          assertion = cfg.desktop.auth.package != null;
          message = ''
            programs.scoot.desktop.auth.enable is set but
            programs.scoot.desktop.auth.package is null: set it
            explicitly (apply the overlay, or point at a polkit
            agent).
          '';
        }
      ];

      # polkitd, the authority the agent answers to. Plain
      # `mkDefault`, so an explicit value still wins.
      security.polkit.enable = lib.mkDefault true;

      environment.systemPackages = lib.optional (
        cfg.desktop.auth.package != null
      ) cfg.desktop.auth.package;
    })
    # The secrets slot's system half: the daemon on PATH, its D-Bus
    # activation files on the session bus, the IPC-lock wrapper its
    # service files exec through, and the greetd PAM pair that
    # unlocks the login keyring from the login password. No unit:
    # the daemon starts on the first `org.freedesktop.secrets` call
    # (`--components=secrets`, per its own service files) and holds
    # the unlocked keyring until the session ends. The client is the
    # home-manager side's (`auth-home.nix` installs `secret-tool`):
    # without it the daemon sits ready for a hand-written setup,
    # the way a `[wallpaper]` finds scootbg on PATH without the
    # home-manager side.
    (lib.mkIf cfg.desktop.secrets.enable {
      assertions = [
        {
          assertion = cfg.desktop.secrets.package != null;
          message = ''
            programs.scoot.desktop.secrets.enable is set but
            programs.scoot.desktop.secrets.package is null: set it
            explicitly (apply the overlay, or point at a
            gnome-keyring).
          '';
        }
        {
          assertion = cfg.desktop.secrets.clientPackage != null;
          message = ''
            programs.scoot.desktop.secrets.enable is set but
            programs.scoot.desktop.secrets.clientPackage is null:
            set it explicitly (apply the overlay, or point at a
            libsecret).
          '';
        }
      ];

      environment.systemPackages =
        lib.optional (cfg.desktop.secrets.package != null) cfg.desktop.secrets.package
        ++ lib.optional (cfg.desktop.secrets.clientPackage != null) cfg.desktop.secrets.clientPackage;

      # The activation files (`org.freedesktop.secrets`,
      # `org.gnome.keyring`, and the Secret portal backend) resolve
      # through these. The `!= null` guard keeps a null out of the
      # list, so a missing daemon fails with the slot's own
      # assertion message above rather than a type error.
      services.dbus.packages = lib.optional (
        cfg.desktop.secrets.package != null
      ) cfg.desktop.secrets.package;

      # The wrapper the activation files exec through
      # (`/run/wrappers/bin/gnome-keyring-daemon`, with `cap_ipc_lock`
      # so the unlocked keyring is never swapped out): the stock
      # `services.gnome.gnome-keyring.enable` switch provides this
      # same wrapper, but that switch also owns the `login` PAM
      # service -- this slot confines its PAM change to greetd's own
      # service below, so it wires the wrapper directly instead.
      # The `!= null` guard is load-bearing the way the session
      # entry's is: the `source` interpolation fails while evaluating
      # the value itself (before `assertions` are checked), so
      # without it a null package would fail with a bare coercion
      # error instead of the assertion's message.
      security.wrappers.gnome-keyring-daemon = lib.mkIf (cfg.desktop.secrets.package != null) {
        owner = "root";
        group = "root";
        capabilities = "cap_ipc_lock=ep";
        source = "${cfg.desktop.secrets.package}/bin/gnome-keyring-daemon";
      };
    })
    # The login-keyring unlock, confined to greetd's own PAM service.
    # `security.pam.services.greetd` authenticates the login user when
    # a greeter (ReGreet, or the IPC login the live test drives)
    # creates the session -- so the password is available to exactly
    # this stack: `auth` caches it for the keyring, `session` starts
    # the daemon holding it (`auto_start`). This deliberately does
    # NOT use nixpkgs' per-service `enableGnomeKeyring` flag (probed
    # at the pinned rev: that flag lives inside the default-rules
    # block, and greetd's service sets `useDefaultRules = false` with
    # its own `login`-substack rules, so the flag is a silent no-op
    # there) and does NOT touch the `login` service (what the stock
    # `services.gnome.gnome-keyring.enable` switch owns): SSH logins,
    # TTY logins and autologins (no password through PAM) keep
    # working, and the keyring prompts once to unlock instead -- the
    # documented second prompt. Each `order` rides ten past the
    # `login` substack it belongs after (the documented relative
    # pattern: a constant would silently reorder under a nixpkgs
    # update). Gated on greetd itself: without the login path there
    # is no stack to extend. The package-null guard beside each keeps
    # a null out of the interpolation, so that failure stays the
    # slot's own assertion message (same reason as the wrapper
    # above).
    (lib.mkIf
      (cfg.desktop.secrets.enable && config.services.greetd.enable && cfg.desktop.secrets.package != null)
      {
        security.pam.services.greetd.rules.auth.gnome_keyring = {
          control = "optional";
          modulePath = "${cfg.desktop.secrets.package}/lib/security/pam_gnome_keyring.so";
          order = config.security.pam.services.greetd.rules.auth.login.order + 10;
          settings = { };
        };
        security.pam.services.greetd.rules.session.gnome_keyring = {
          control = "optional";
          modulePath = "${cfg.desktop.secrets.package}/lib/security/pam_gnome_keyring.so";
          order = config.security.pam.services.greetd.rules.session.login.order + 10;
          settings = {
            auto_start = true;
          };
        };
      }
    )
    # The power policy's system half: the profiles daemon, the lid and
    # power-key actions, low-battery suspend, and the charge-limit
    # service. The profile switch and the charge button's fill unit are
    # the home-manager side's (`keys-home.nix`, `power-home.nix`):
    # without it the daemon and the tools sit ready for a hand-written
    # setup, the way a `[wallpaper]` finds scootbg on PATH without the
    # home-manager side. Opt-in (never with the profile): lid-close
    # suspend and the charge cap are behavior changes, not defaults.
    (lib.mkIf cfg.desktop.power.enable {
      assertions = [
        {
          # Nothing to run the switch without (refused only while the
          # profiles daemon is wanted: with `profiles.enable = false`
          # a driverless box drops the daemon and needs no package).
          assertion = !cfg.desktop.power.profiles.enable || cfg.desktop.power.profiles.package != null;
          message = ''
            programs.scoot.desktop.power.enable is set but
            programs.scoot.desktop.power.profiles.package is null: set
            it explicitly (apply the overlay, or point at a
            power-profiles-daemon).
          '';
        }
        {
          # Loud at eval: UPower discards its whole percentage triple
          # for its own defaults unless low <= critical <= action is
          # descending, and this module leaves low (20) and critical
          # (5) at UPower's defaults -- so the action must stay at or
          # under 5, or the setting would silently do nothing.
          assertion =
            cfg.desktop.power.lowBattery.percentage >= 0 && cfg.desktop.power.lowBattery.percentage <= 5;
          message = ''
            programs.scoot.desktop.power.lowBattery.percentage is a
            battery percent at or under 5: UPower keeps `PercentageLow`
            20 and `PercentageCritical` 5, and anything above 5 breaks
            the descending order, so UPower would silently use its own
            triple instead.
          '';
        }
      ];

      # The charge cap rides with the policy (still individually
      # disable-able at plain priority, the way the lock rides with
      # the idle policy).
      programs.scoot.desktop.power.chargeLimit.enable = lib.mkDefault true;

      # The profiles daemon behind the keymap's switch. On hardware
      # with no PPD driver (Apple silicon: no `platform_profile`, no
      # EPP -- measured on the M2, whose cpufreq runs `apple-cpufreq`
      # with the `schedutil` governor and the `apple_idle` deep state)
      # the daemon still runs and still owns the bus name -- which is
      # what widgets read (the M2's own shell pulls it in for its
      # battery/power widgets) -- but changes no CPU behavior. That is
      # inert, not refused: eval cannot see the machine, and refusing
      # would break one shared config across heterogeneous hardware.
      # A `mkDefault`, like the file's siblings, so an explicit value
      # wins: `profiles.enable = false` drops the daemon (a driverless
      # box needs neither it nor its bind), and a user setting
      # `services.power-profiles-daemon.enable` directly wins over
      # both.
      services.power-profiles-daemon.enable = lib.mkDefault cfg.desktop.power.profiles.enable;
      services.power-profiles-daemon.package = lib.mkIf (
        cfg.desktop.power.profiles.enable && cfg.desktop.power.profiles.package != null
      ) cfg.desktop.power.profiles.package;

      # Low battery through UPower (which already sees the
      # `macsmc-battery` on the M2, with history and statistics).
      # Suspend, not the UPower default HybridSleep: s2idle is the
      # only sleep state the reference hardware has, and its swap is
      # zram (no persistent image to hibernate into), so HybridSleep
      # would fail there instead of sleeping. `Suspend` is a UPower
      # "risky" action (RAM stays powered on a dying battery), hence
      # the flag beside it -- set only while the action needs it.
      # Each a `mkDefault`, so an explicit value still wins.
      services.upower.enable = lib.mkDefault true;
      services.upower.percentageAction = lib.mkDefault cfg.desktop.power.lowBattery.percentage;
      services.upower.criticalPowerAction = lib.mkDefault cfg.desktop.power.lowBattery.action;
      services.upower.allowRiskyCriticalPowerAction = lib.mkIf (
        cfg.desktop.power.lowBattery.action == "Suspend" || cfg.desktop.power.lowBattery.action == "Ignore"
      ) true;

      # Lid and power-key actions through logind (the canonical
      # `settings.Login.*` path, not the renamed aliases). Each a
      # `mkDefault`, so an explicit value still wins. The docked rule
      # is the twin of the idle child's (same value, same path, so
      # the two merge): a closed lid on a multi-output box locks,
      # never suspends. `KillUserProcesses` is left to nixpkgs (its
      # own default is already false, for the tmux/mosh reason quoted
      # there): pinning it here would eval-conflict with anyone
      # setting it elsewhere, and a logout (or a dropped SSH session
      # sharing this user manager) never takes agents and
      # multiplexers with it either way -- which is what a
      # remotely-driven box needs.
      services.logind.settings.Login.HandleLidSwitch = lib.mkDefault cfg.desktop.power.lidSwitch;
      services.logind.settings.Login.HandleLidSwitchDocked =
        lib.mkDefault cfg.desktop.power.lidSwitchDocked;
      services.logind.settings.Login.HandleLidSwitchExternalPower =
        lib.mkDefault cfg.desktop.power.lidSwitchExternalPower;
      services.logind.settings.Login.HandlePowerKey = lib.mkDefault cfg.desktop.power.powerKey;

      environment.systemPackages =
        lib.optional (
          cfg.desktop.power.profiles.enable && cfg.desktop.power.profiles.package != null
        ) cfg.desktop.power.profiles.package
        ++ lib.optional (cfg.desktop.power.chargeLimit.enable) scootCharge;
    })
    # The charge and low-battery bounds, refused here as well as on
    # the home-manager side (same messages): a NixOS-only setup must
    # hear about a bad limit without the user module. Kept outside
    # `power.enable` so they still fire then, the way the idle
    # policy's refusals sit outside its own switch.
    (lib.mkIf cfg.desktop.power.chargeLimit.enable {
      assertions = [
        {
          # Loud at eval, not a threshold the kernel refuses at write:
          # `charge_control_end_threshold` takes a percent.
          assertion = cfg.desktop.power.chargeLimit.limit >= 1 && cfg.desktop.power.chargeLimit.limit <= 100;
          message = ''
            programs.scoot.desktop.power.chargeLimit.limit is a charge
            percent, 1 to 100.
          '';
        }
        {
          # Loud at eval, like `session.command`'s: the name renders
          # into sysfs and udev paths, so an explicitly empty or
          # whitespace-only value would address the wrong file.
          assertion =
            cfg.desktop.power.chargeLimit.battery == null
            || builtins.match "^[[:space:]]*$" cfg.desktop.power.chargeLimit.battery == null;
          message = ''
            programs.scoot.desktop.power.chargeLimit.battery is empty
            or blank: set the kernel's battery name (e.g. `BAT0`), or
            leave it null to auto-detect.
          '';
        }
      ];
    })
    # The charge-limit service itself: root re-syncs the threshold on
    # every AC change (udev, which also re-applies the group write on
    # the node) and every 5 minutes (a timer, which catches a missed
    # event). Kept in its own element (not under `power.enable`):
    # with `enable` on and `chargeLimit.enable` off, nothing here may
    # run -- and the udev AC rule below is what also carries the
    # opt-in profile auto-switch, which needs no charge hardware.
    (lib.mkIf (cfg.desktop.power.enable && cfg.desktop.power.chargeLimit.enable) {
      systemd.services.scoot-charge-sync = {
        description = "Apply the desk-aware battery charge policy";
        after = [ "systemd-udevd.service" ];
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${scootCharge}/bin/scoot-charge sync";
        };
      };
      systemd.timers.scoot-charge-sync = {
        wantedBy = [ "timers.target" ];
        timerConfig = {
          OnBootSec = "1min";
          OnUnitActiveSec = "5min";
          AccuracySec = "1min";
        };
      };
      systemd.tmpfiles.rules = [ "d /var/lib/scoot-charge 2775 root users -" ];

      # The threshold and the state are the `users` group's to change
      # (the bar button runs as the logged-in user); every power-supply
      # change re-runs the policy (the timer catches a missed event).
      # `TEST` matches whatever supply owns a threshold node
      # (`macsmc-battery`, `BAT0`, ...), and `Mains` whatever AC
      # adapter feeds it -- no per-machine kernel names in the rule.
      services.udev.extraRules = ''
        SUBSYSTEM=="power_supply", TEST=="charge_control_end_threshold", RUN+="${pkgs.coreutils}/bin/chgrp users /sys%p/charge_control_end_threshold", RUN+="${pkgs.coreutils}/bin/chmod g+w /sys%p/charge_control_end_threshold"
        SUBSYSTEM=="power_supply", ENV{POWER_SUPPLY_TYPE}=="Mains", RUN+="${pkgs.systemd}/bin/systemctl start --no-block scoot-charge-sync.service"
      '';
    })
    # The opt-in profile auto-switch on AC transitions (udev, which
    # fires on boot coldplug too, so the matching profile applies
    # from boot). Each half only while set: null holds whatever is
    # set (PPD's own behavior -- a profile is user intent, not power
    # state). Root on the system bus needs no policy exception.
    # The 5-minute charge timer does NOT re-apply these (only AC
    # transitions do): a manual switch mid-session stays until the
    # next plug event. A null package beside a set profile renders the
    # bare name (the refusal above still fires: eval must not throw
    # where it should refuse). Nothing without the profiles daemon
    # (`profiles.enable`): without it there is nothing to select.
    (lib.mkIf
      (
        cfg.desktop.power.enable
        && cfg.desktop.power.profiles.enable
        && cfg.desktop.power.profileOnAC != null
      )
      {
        services.udev.extraRules = ''
          SUBSYSTEM=="power_supply", ENV{POWER_SUPPLY_TYPE}=="Mains", ENV{POWER_SUPPLY_ONLINE}=="1", RUN+="${
            if cfg.desktop.power.profiles.package != null then
              lib.getExe cfg.desktop.power.profiles.package
            else
              "powerprofilesctl"
          } set ${cfg.desktop.power.profileOnAC}"
        '';
      }
    )
    (lib.mkIf
      (
        cfg.desktop.power.enable
        && cfg.desktop.power.profiles.enable
        && cfg.desktop.power.profileOnBattery != null
      )
      {
        services.udev.extraRules = ''
          SUBSYSTEM=="power_supply", ENV{POWER_SUPPLY_TYPE}=="Mains", ENV{POWER_SUPPLY_ONLINE}=="0", RUN+="${
            if cfg.desktop.power.profiles.package != null then
              lib.getExe cfg.desktop.power.profiles.package
            else
              "powerprofilesctl"
          } set ${cfg.desktop.power.profileOnBattery}"
        '';
      }
    )
    # The audio slot's system half: the OSD on PATH and PipeWire
    # running for the volume binds. The unit, its config and the
    # control scripts are the home-manager side's (`audio-home.nix`
    # and the keymap): without it the OSD sits ready for a
    # hand-written setup, the way a `[wallpaper]` finds scootbg on PATH
    # without the home-manager side. `wpctl` itself arrives with the
    # keymap's volume tool (WirePlumber's own CLI); the session
    # manager rides along with PipeWire, so the default-sink routing
    # the binds drive is there too.
    (lib.mkIf cfg.desktop.audio.enable {
      assertions = [
        {
          assertion = cfg.desktop.audio.osd.package != null;
          message = ''
            programs.scoot.desktop.audio.enable is set but
            programs.scoot.desktop.audio.osd.package is null: set it
            explicitly (apply the overlay, or point at a wob).
          '';
        }
        {
          assertion = cfg.desktop.audio.dumpPackage != null;
          message = ''
            programs.scoot.desktop.audio.enable is set but
            programs.scoot.desktop.audio.dumpPackage is null: set it
            explicitly (apply the overlay, or point at a pipewire).
          '';
        }
      ];

      # PipeWire with WirePlumber running for the binds (`wpctl` is
      # WirePlumber's CLI: a PipeWire-only shape would leave the binds
      # with nothing to call, for ~4.7 MiB saved -- measured, see
      # site/src/content/docs/desktop/index.md#sound-brightness-keys-and-the-on-screen-display).
      # Plain `mkDefault`, so an explicit value still wins -- and the
      # capture slot defaults the same switch, which merges rather
      # than conflicts.
      services.pipewire.enable = lib.mkDefault true;

      environment.systemPackages =
        lib.optional (cfg.desktop.audio.osd.package != null) cfg.desktop.audio.osd.package
        ++ lib.optional (cfg.desktop.audio.dumpPackage != null) cfg.desktop.audio.dumpPackage;
    })
    # The idle policy's system half: its tools on PATH, the docked-lid
    # rule, and the locker's PAM service. The timers and the locker
    # config are the home-manager side's (`idle-home.nix`): without it
    # the tools sit ready for a hand-written setup, the way a `[wallpaper]`
    # finds scootbg on PATH without the home-manager side.
    #
    # The shared keymap's system half just below it: its tools on
    # PATH (`keys-home.nix` owns the `[binds]`); the slot scripts
    # beside those binds are per-user, so they stay on that side.
    (lib.mkIf cfg.desktop.keys.enable {
      environment.systemPackages =
        lib.optional (cfg.desktop.keys.brightnessPackage != null) cfg.desktop.keys.brightnessPackage
        ++ lib.optional (cfg.desktop.keys.volumePackage != null) cfg.desktop.keys.volumePackage
        ++ lib.optional (cfg.desktop.keys.mediaPackage != null) cfg.desktop.keys.mediaPackage;
    })
    (lib.mkIf cfg.desktop.idle.enable {
      assertions = [
        {
          assertion = cfg.desktop.idle.package != null;
          message = ''
            programs.scoot.desktop.idle.enable is set but
            programs.scoot.desktop.idle.package is null: set it
            explicitly (apply the overlay, or point at a swayidle).
          '';
        }
        {
          assertion = cfg.desktop.idle.dimPackage != null;
          message = ''
            programs.scoot.desktop.idle.enable is set but
            programs.scoot.desktop.idle.dimPackage is null: set it
            explicitly.
          '';
        }
        {
          assertion = cfg.desktop.idle.offPackage != null;
          message = ''
            programs.scoot.desktop.idle.enable is set but
            programs.scoot.desktop.idle.offPackage is null: set it
            explicitly.
          '';
        }
      ];

      environment.systemPackages =
        lib.optional (cfg.desktop.idle.package != null) cfg.desktop.idle.package
        ++ lib.optional (cfg.desktop.idle.dimPackage != null) cfg.desktop.idle.dimPackage
        ++ lib.optional (cfg.desktop.idle.offPackage != null) cfg.desktop.idle.offPackage
        ++ lib.optional (
          cfg.desktop.idle.lock.enable && cfg.desktop.idle.lock.package != null
        ) cfg.desktop.idle.lock.package
        ++ lib.optional (
          cfg.desktop.idle.mediaInhibit.enable && cfg.desktop.idle.mediaInhibit.package != null
        ) cfg.desktop.idle.mediaInhibit.package;

      # Lid closed on a docked or multi-output box: lock, don't suspend
      # blindly. `HandleLidSwitchDocked` fires only when a dock is
      # detected or more than one display is connected (external power
      # alone does not count -- that is the separate
      # `HandleLidSwitchExternalPower`, ignored by default), so an
      # undocked laptop keeps the plain `HandleLidSwitch` behavior
      # (suspend; the suspend policy itself is the `desktop-power`
      # child's). The lock lands through logind's Lock, which the
      # policy's `lock` event listens on. The canonical
      # `settings.Login.*` path, not the renamed `lidSwitchDocked`
      # alias. `mkDefault`, so an explicit value still wins. Defers to
      # the power policy while it runs (nothing here then): the power
      # child's own docked rule owns the setting, so a user-set
      # `power.lidSwitchDocked` wins instead of eval-conflicting with
      # this twin.
      services.logind.settings.Login.HandleLidSwitchDocked = lib.mkIf (!cfg.desktop.power.enable) (
        lib.mkDefault "lock"
      );
    })
    # The locker's PAM service (password auth for the locker): without
    # it swaylock cannot validate, loud here instead of a locker that
    # never unlocks. Kept in its own element (not under `idle.enable`):
    # with `enable` off and `lock.enable` on, the refusal below must
    # still fire rather than going quiet.
    (lib.mkIf
      (
        cfg.desktop.idle.enable
        && cfg.desktop.idle.lock.enable
        && cfg.desktop.idle.lock.daemon == "swaylock"
      )
      {
        security.pam.services.swaylock = { };
      }
    )
    # Refusals that must fire whatever else is on (kept outside
    # `idle.enable` so they still fire then).
    (lib.mkIf cfg.desktop.idle.lock.enable {
      assertions = [
        {
          assertion = cfg.desktop.idle.enable;
          message = ''
            programs.scoot.desktop.idle.lock.enable needs
            programs.scoot.desktop.idle.enable: without the policy
            nothing listens for logind's Lock, and the locker would
            never start.
          '';
        }
        {
          assertion = cfg.desktop.idle.lock.package != null;
          message = ''
            programs.scoot.desktop.idle.lock.enable is set but
            programs.scoot.desktop.idle.lock.package is null: set it
            explicitly (apply the overlay, or point at a locker that
            speaks the daemon's flags).
          '';
        }
        {
          # Loud at eval, like `session.command`'s: the action renders
          # inside single quotes on the swayidle timeout line, so an
          # explicitly empty or whitespace-only value would run a no-op
          # (the session never locking while `lock.enable` says it
          # does), and a single quote would break out of the quoting
          # and corrupt the config line.
          assertion =
            builtins.match "^[[:space:]]*$" cfg.desktop.idle.lock.command == null
            && builtins.match ".*'.*" cfg.desktop.idle.lock.command == null;
          message = ''
            programs.scoot.desktop.idle.lock.command is empty, blank
            or contains a single quote: set the full lock action to run
            (e.g. `loginctl lock-session`).
          '';
        }
      ];
    })
    (lib.mkIf cfg.desktop.idle.mediaInhibit.enable {
      assertions = [
        {
          assertion = cfg.desktop.idle.enable;
          message = ''
            programs.scoot.desktop.idle.mediaInhibit.enable needs
            programs.scoot.desktop.idle.enable: an inhibitor with no
            policy holds nothing off.
          '';
        }
        {
          assertion = cfg.desktop.idle.mediaInhibit.package != null;
          message = ''
            programs.scoot.desktop.idle.mediaInhibit.enable is set but
            programs.scoot.desktop.idle.mediaInhibit.package is null:
            set it explicitly.
          '';
        }
      ];
    })
    # A look without the profile is a silent no-op; refuse it loudly
    # instead (kept outside `desktop.enable` so it still fires then).
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
    # The bar half lives in the bar module (`nix/modules/scootbar.nix`
    # reads this profile): a set of `programs.scootbar` here would need
    # that module imported, and a conditional set of an undeclared option
    # fails eval whatever the condition is, so the profile never sets
    # across the module boundary.
    # The login screen, in its own element (not under `cfg.enable`):
    # with `enable` off and `greeter.enable` on, the assertions below
    # must still fire rather than the whole element going quiet.
    (lib.mkIf cfg.greeter.enable {
      assertions = [
        {
          # A greeter for a compositor that is not installed is
          # nonsense; without `enable` there is no package for the
          # greeter's session list to offer.
          assertion = cfg.enable;
          message = ''
            programs.scoot.greeter.enable needs programs.scoot.enable:
            the greeter lists scoot sessions, so scoot itself must be installed.
          '';
        }
        {
          # The greeter lists scoot through the session entry (ReGreet
          # reads `wayland-sessions`, which is what `session.enable`
          # installs into). Forced on below by default, so this fires
          # only when the entry was explicitly turned back off.
          assertion = cfg.session.enable;
          message = ''
            programs.scoot.greeter.enable needs
            programs.scoot.session.enable: without the session entry the
            greeter has no scoot session to offer.
          '';
        }
        {
          # The login screen is single-owner: greetd/ReGreet and GDM
          # cannot both run it. Loud at eval, not two greeters fighting
          # over the first VT at boot.
          assertion = !config.services.displayManager.gdm.enable;
          message = ''
            programs.scoot.greeter.enable conflicts with GDM
            (services.displayManager.gdm.enable): turn one of them off.
            Rolling back is the previous NixOS generation, or this
            option set back to false.
          '';
        }
        {
          # Same, for SDDM. Anything else owning the login screen
          # (lemurs, ly, another greetd setup) must be turned off by
          # hand -- see site/src/content/docs/desktop/index.md#the-greeter.
          assertion = !config.services.displayManager.sddm.enable;
          message = ''
            programs.scoot.greeter.enable conflicts with SDDM
            (services.displayManager.sddm.enable): turn one of them off.
            Rolling back is the previous NixOS generation, or this
            option set back to false.
          '';
        }
        {
          # Two owners for one backdrop: an explicit `background` here
          # and Stylix's regreet target (which sets the same
          # `background.path` from `stylix.image`) would merge-conflict.
          # `config.stylix` is unset without Stylix, and `.regreet`
          # without that target -- both default to off (see the helper
          # above).
          assertion = cfg.greeter.background == null || !(stylixRegreetBackground (config.stylix or { }));
          message = ''
            programs.scoot.greeter.background is set, but Stylix's
            regreet target is also setting ReGreet's background from
            `stylix.image`: unset one of them (leave `background` null
            and Stylix owns the backdrop, or turn off
            `stylix.targets.regreet` image theming).
          '';
        }
      ];

      # `session.enable` on by default, so the greeter lists a scoot
      # session; an explicit `false` there trips the assertion above
      # instead of yielding a greeter with no scoot in it.
      programs.scoot.session.enable = lib.mkDefault true;

      # The login screen itself is nixpkgs' own ReGreet module (greetd
      # running ReGreet under cage) -- this only turns it on and
      # optionally names its backdrop at plain priority, so a backdrop
      # set directly under `services.displayManager.regreet.settings`
      # behaves like any other explicit-against-explicit conflict
      # instead of being silently shadowed. ReGreet runs under cage,
      # never inside scoot: hosting a pre-login greeter in the
      # compositor would need a locked-down scoot profile (no binds, no
      # IPC socket), which is out of scope -- see site/src/content/docs/desktop/index.md#the-greeter.
      services.displayManager.regreet = {
        enable = true;
        # One screen, not the whole layout: cage spans every output by
        # default (nixpkgs' `cageArgs` default `[ "-s" "-d" ]`), so on a
        # multi-output box ReGreet's window covers the layout bounding
        # box and the login card lands near one screen's edge (seen on a
        # laptop with an external monitor: 2560x1600 plus 1920x1080 drew
        # a 4480x1600 window). `-m last` confines cage to a single
        # output, which is also the shape nixpkgs documents as its
        # `cageArgs` example. Plain `mkDefault` priority: it beats
        # nixpkgs' option default, and anything the user sets beats it
        # (both halves pinned in `nix/tests.nix`). Back to spanning with
        # `services.displayManager.regreet.cageArgs = [ "-s" "-d" ]`.
        cageArgs = lib.mkDefault [
          "-s"
          "-d"
          "-m"
          "last"
        ];
        # The whole table or nothing: a `path`-only `mkIf` would leave
        # an empty `[background]` behind when `background` is null.
        settings.background = lib.mkIf (cfg.greeter.background != null) {
          path = toString cfg.greeter.background;
        };
      };

      # The greeter's processes end with its session: when the greeter
      # session stops, logind kills whatever is left in its scope. Today
      # the session-bus `dbus-daemon` that `dbus-run-session` starts (and
      # ReGreet's AT-SPI bus daemon beside it) survives the session, and
      # the scope sits `active (abandoned)` with the session `closing`
      # for good -- one leaked pair per login, greeter crash, and greetd
      # restart (see the `greeter-session-leak` backlog entry).
      # Scoped to the greeter user, so every other user's lingering
      # processes (tmux, ssh agents) are untouched: checked against
      # `manager_shall_kill` in the pinned systemd source, which with a
      # non-empty `KillOnlyUsers` kills only that user whatever
      # `KillUserProcesses` says. Both `mkDefault`, so an explicit user
      # setting still wins (this composes with the power policy, which
      # deliberately leaves `KillUserProcesses` alone). The greeter
      # user's manager (`user@greeter.service`: dbus-broker, pipewire)
      # is a separate unit outside the session scope, so it survives.
      services.logind.settings.Login.KillUserProcesses = lib.mkDefault true;
      services.logind.settings.Login.KillOnlyUsers = lib.mkDefault [ "greeter" ];
      # The switch must reload logind for the new config to take
      # effect: nixpkgs marks systemd-logind `reloadIfChanged` (reload,
      # never restart -- restarting logind breaks sessions), but that
      # only fires when the *unit file* changes, and a `logind.conf`
      # content change is an `environment.etc` change, not a unit
      # change. Without this line a switch that only flips the two keys
      # above leaves the running logind on its old in-memory config,
      # leaking exactly as before. Tying the settings' identity into
      # the unit (an `X-` key systemd itself ignores) makes the unit
      # differ exactly when the settings do, which
      # switch-to-configuration-ng resolves to a reload through the
      # `X-ReloadIfChanged` nixpkgs already sets on this unit: checked
      # against `compare_units` (any non-ignored `[Unit]` diff is
      # `UnequalNeedsRestart`) and `handle_modified_unit` (which reloads
      # on `X-ReloadIfChanged` instead of restarting) at the pinned
      # nixpkgs -- including via the `overrides.conf` drop-in this
      # unit's settings land in, which `parse_unit` merges before
      # comparing. `reloadTriggers` would do the same job but trips
      # nixpkgs' "both `reloadIfChanged` and `reloadTriggers`" eval
      # warning on every rebuild, so it is not used. Hashing the
      # `settings.Login` attrset (rather than the rendered file) keeps
      # the stub evaluations in `nix/tests.nix` free of new stubs; it
      # also reloads on any other `Login` change (lid-switch actions
      # included), which had the same latent gap.
      systemd.services.systemd-logind.unitConfig."X-ScootLogindSettings" = builtins.hashString "sha256" (
        builtins.toJSON config.services.logind.settings.Login
      );
    })
    # The look on the login screen (the `desktop-theme-look` child):
    # backdrop pairing, the dark/light GTK setting, the look's CSS
    # and font, each at plain `mkDefault` so an explicit value still
    # wins. Off with no look behind the profile, or while
    # `theme.targets.greeter` opts out. A look without a shippable
    # backdrop (`vinyl-sunset`, license-barred from being committed)
    # leaves the backdrop alone. No extra daemons: ReGreet reads its
    # config and CSS at startup, and the packages below put the
    # font, icons and cursor on the system lookup paths its session
    # reads.
    (lib.mkIf
      (
        cfg.greeter.enable
        && cfg.desktop.enable
        && lookNix != null
        && (cfg.desktop.theme.targets.greeter.enable or true)
      )
      {
        assertions = [
          {
            assertion = cfg.desktop.theme.fonts.sansPackage != null;
            message = ''
              programs.scoot.desktop.look needs a greeter font but
              programs.scoot.desktop.theme.fonts.sansPackage is null: set
              it explicitly (apply the overlay, or point at the look's
              UI sans).
            '';
          }
        ];

        # The backdrop pairing: the session wallpaper behind the login
        # card (null keeps ReGreet's own background, which is also
        # what a license-barred look does). An explicit `background`
        # wins over this default.
        programs.scoot.greeter.background = lib.mkIf (lookNix.wallpaper != null) (
          lib.mkDefault lookNix.wallpaper.image
        );

        services.displayManager.regreet = {
          # The dark/light half of the look (what the session's GTK
          # apps follow through `settings.ini`).
          settings.GTK.application_prefer_dark_theme = lib.mkDefault lookNix.isDark;
          # The look's CSS and proportional sans (12 pt, the measured
          # greeter size). An explicit `extraCss`/`font` wins over these.
          extraCss = lib.mkDefault lookNix.appFiles.regreetCss;
          font = lib.mkDefault {
            package = cfg.desktop.theme.fonts.sansPackage;
            name = lookNix.fonts.sans;
            size = 12;
          };
        };

        # The greeter's lookup paths: the UI sans (its `font` above),
        # the icon theme its widgets expect, and the cursor. The
        # `!= null` guards keep a null out of the lists, so a missing
        # package fails with the assertion above rather than a type
        # error (the same guard the portal backends use).
        fonts.packages = lib.optional (
          cfg.desktop.theme.fonts.sansPackage != null
        ) cfg.desktop.theme.fonts.sansPackage;
        environment.systemPackages =
          lib.optional (cfg.desktop.theme.icon.package != null) cfg.desktop.theme.icon.package
          ++ lib.optional (cfg.desktop.theme.cursor.package != null) cfg.desktop.theme.cursor.package;
      }
    )
  ];
}
