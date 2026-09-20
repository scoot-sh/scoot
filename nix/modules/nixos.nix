{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;

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
  sessionPackage =
    (pkgs.writeTextDir "share/wayland-sessions/scoot.desktop" ''
      [Desktop Entry]
      Name=scoot
      Comment=scoot scrolling-tiling Wayland compositor
      Exec=${cfg.package}/bin/scoot --tty
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
in
{
  options.programs.scoot = {
    enable = lib.mkEnableOption "scoot, the scrolling-tiling Wayland compositor";

    # Same null-default shape as the home-manager module (see its
    # comment): no overlay means no `pkgs.scoot` to default to, and the
    # flake wrapper (`nixosModules.scoot` in `flake.nix`) fills this
    # with the flake's own build via `mkDefault`.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scoot";
      description = ''
        The scoot package to install system-wide and to launch from
        the login-screen session entry.
      '';
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
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        # Loud at eval, not a broken `.desktop` at login: `Exec=` with
        # a null package would interpolate to nothing and greet
        # whoever picks the entry with a greeter error.
        assertion = cfg.package != null;
        message = ''
          programs.scoot.enable is set but programs.scoot.package is null.
          Flake consumers get the flake's own build by default through the
          nixosModules wrapper -- this fires only for direct-module use:
          set programs.scoot.package explicitly.
        '';
      }
      {
        assertion = cfg.session.enable -> cfg.package != null;
        message = ''
          programs.scoot.session.enable needs programs.scoot.package:
          a session entry with no binary to launch would fail at the login screen.
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
  };
}
