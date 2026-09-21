{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  tomlFormat = pkgs.formats.toml { };

  # Rendered from the user's free-form settings. A value with no TOML
  # representation at all (e.g. a function) fails the option type-check
  # at evaluation time ("not of type 'TOML value'"), so the error aborts
  # before anything builds, let alone starts a session. A value that renders
  # but has the wrong *scoot* type (a string for `layout.gap`) reaches
  # the session and is refused there -- and the loader fails safe (whole
  # file discarded for defaults, session still boots), so a typo costs
  # the config, never the session. See
  # docs/configuration.md#failure-semantics.
  configFile = tomlFormat.generate "scoot-config.toml" cfg.settings;

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

    # No default here: without an overlay there is no `pkgs.scoot`, and a
    # wrong guess would silently install someone else's build. The flake
    # wrapper (`homeModules.scoot`, still aliased as
    # `homeManagerModules.scoot`, in `flake.nix`) fills this with the
    # flake's own build via `mkDefault` on Linux and with null (files
    # only) on Darwin; direct-module users set it explicitly (see the
    # `example`), or leave it null for a files-only setup -- the module
    # manages files regardless, and asserts nothing about the package.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
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
    # `<package>/bin/scoot --tty -- ~/.config/<that path>` line so the
    # greeter entry runs this script instead of a bare compositor (see
    # docs/nix.md, which shows the pairing together). Null writes no file.
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
        `<package>/bin/scoot --tty -- ~/.config/<that path>` line so the
        login-screen entry runs it. Null writes no file.
      '';
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

  config = lib.mkIf cfg.enable {
    # No assertion that `package` is set: a files-only setup (binary
    # from elsewhere, e.g. a system package) is legitimate, and with
    # defaults this still manages a (minimal, valid) config plus the
    # portals file -- both harmless. `package`'s description states
    # that null installs no binary.

    home.packages = lib.optional (cfg.package != null) cfg.package;

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
  };
}
