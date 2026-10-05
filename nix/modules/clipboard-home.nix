# `programs.scoot.desktop.clipboard` for home-manager: the history
# daemons (one `wl-paste --watch` per selection), the store entry they
# run, and `wl-copy`/`wl-paste` plus the picker menu on PATH. The option
# shapes live in `./desktop.nix` (shared with the NixOS side); the
# `package` defaults live here because only this side has `pkgs`. See
# site/src/content/docs/desktop/index.md#clipboard.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  clip = cfg.desktop.clipboard;

  # The lean cliphist (nixpkgs' binary without its contrib pickers --
  # see `clipboard-cliphist.nix`), shared with the NixOS side's default
  # so either side alone names the same manager. Guarded off Linux like
  # the attribute it wraps: `pkgs.cliphist` is Linux-only
  # (`meta.platforms = linux`).
  leanClip = import ./clipboard-cliphist.nix { inherit pkgs; };

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # Absolute tool paths when the module knows a package, bare names from
  # PATH otherwise (off Linux, or a direct-module setup without the
  # overlay). The store entry then degrades to failing loudly at the
  # unit (a missing tool exits nonzero, which restarts it) rather than
  # failing evaluation; the picker degrades the keymap's usual way (a
  # missing tool fails quietly at runtime -- see `keys-home.nix`).
  # Explicit binary names (`getExe'`, not `getExe`): what runs is the
  # tool, whatever the derivation calls itself.
  cliphistBin =
    if clip.managerPackage != null then lib.getExe' clip.managerPackage "cliphist" else "cliphist";
  wlPasteBin =
    if clip.wlClipboardPackage != null then
      lib.getExe' clip.wlClipboardPackage "wl-paste"
    else
      "wl-paste";
  # The lock probe's compositor client: absolute when scoot itself is
  # known, bare otherwise (a files-only setup's probe then fails open in
  # the guard -- see `clipboard-guard.sh`).
  scootBin = if cfg.package != null then lib.getExe' cfg.package "scoot" else "scoot";

  # Extra cliphist flags, shared by the store entry (and the wipe on the
  # idle policy's lock line -- see `idle-home.nix`, which rebuilds the
  # db half from the same option). Null `dbPath` passes nothing, so a
  # default setup honors `XDG_CACHE_HOME` the way upstream does.
  dbFlags = lib.optionalString (clip.dbPath != null) "-db-path '${clip.dbPath}' ";

  # The lock probe both scripts embed (`@SCOOT_BIN@` replaced, the way
  # `nixos.nix` substitutes the session units).
  guard = builtins.replaceStrings [ "@SCOOT_BIN@" ] [ scootBin ] (
    builtins.readFile ./clipboard-guard.sh
  );

  # The watch command both store units run: stdin carries one selection's
  # bytes, `CLIPBOARD_STATE` its sensitivity (`wl-paste --watch` sets it
  # to `sensitive` for password-manager offers -- cliphist then stores
  # nothing -- and to `nil` for a cleared clipboard, which arrives on
  # empty stdin and is dropped by cliphist's whitespace check). The guard
  # in front refuses while locked (fail-open without IPC, so a broken
  # probe costs the lock guarantee rather than the history); `max-items`
  # bounds the db, oldest dropped first.
  storeEntry = pkgs.writeShellScriptBin "scoot-clipboard-store-entry" ''
    ${guard}
    if ! clipboard_unlocked; then
      exit 0
    fi
    exec ${cliphistBin} ${dbFlags}-max-items ${toString clip.maxItems} store
  '';

  # A null beside `enable` is the loud assertion below, not a throw
  # inside `getExe`: the same guard the idle policy uses, since
  # standalone evals collect assertions without enforcing them.
  toolsReady =
    clip.managerPackage != null && clip.wlClipboardPackage != null && clip.menuPackage != null;

  mkStoreUnit = description: extraArgs: {
    Unit = {
      Description = description;
      # `scoot-session.target`: scoot's own session scope (started by
      # the launcher past the display import) -- never the shared
      # `graphical-session.target`, which every other desktop reaches
      # too and would start these watchers inside someone else's
      # session.
      PartOf = [ "scoot-session.target" ];
      After = [ "scoot-session.target" ];
      # A new entry script restarts the watcher (it starts in
      # milliseconds).
      X-Restart-Triggers = [ "${storeEntry}" ];
    };
    Service = {
      # An activation can arrive before the session reaches the
      # session target: skip cleanly then -- no restart -- and the
      # wanted-by below starts it with the display in the common case
      # (the same condition the mako unit uses).
      ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '[ -n \"$WAYLAND_DISPLAY\" ]'";
      ExecStart = "${wlPasteBin} ${extraArgs}--watch ${storeEntry}/bin/scoot-clipboard-store-entry";
      Restart = "on-failure";
      RestartSec = 2;
      # Unending retries, like the bar's and idle units: a start
      # before the compositor is up must retry, not die at the burst
      # limit.
      StartLimitIntervalSec = 0;
    };
    Install.WantedBy = [ "scoot-session.target" ];
  };
in
{
  options.programs.scoot.desktop.clipboard = {
    # The tools below are Linux-only: their attributes refuse evaluation
    # when forced on Darwin, so `or null` alone does not save them (the
    # Darwin `nix flake check` run reads every default). Off Linux each
    # defaults to null, which the assertions below refuse loudly instead
    # of installing nothing silently.
    managerPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? cliphist then leanClip else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then leanClip else null";
      description = ''
        The clipboard manager to keep history in (must speak
        `store`/`list`/`decode`/`wipe`). Defaults to a lean cliphist
        without its contrib pickers (see `clipboard-cliphist.nix`).
        Null installs nothing. Linux-only: null off Linux.
      '';
    };

    wlClipboardPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? wl-clipboard then pkgs.wl-clipboard else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.wl-clipboard else null";
      description = ''
        The copy/paste tools (`wl-copy`/`wl-paste` on PATH, and the
        watcher behind the store units). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    menuPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? fuzzel then pkgs.fuzzel else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel else null";
      description = ''
        The dmenu-style menu the history picker runs
        (`fuzzel --dmenu`, themed by the look -- the launcher, which
        reuses this same package). Null installs nothing. Linux-only:
        null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The history: its tools on PATH, and one watcher per selection
    # (regular clipboard plus primary, into the same history -- the
    # picker restores either to the regular clipboard; the live primary
    # stays compositor-native, middle-click pasting what it always did).
    (lib.mkIf clip.enable {
      assertions = [
        {
          assertion = clip.managerPackage != null;
          message = ''
            programs.scoot.desktop.clipboard.enable is set but
            programs.scoot.desktop.clipboard.managerPackage is null: set
            it explicitly (apply the overlay, or point at a cliphist).
          '';
        }
        {
          assertion = clip.wlClipboardPackage != null;
          message = ''
            programs.scoot.desktop.clipboard.enable is set but
            programs.scoot.desktop.clipboard.wlClipboardPackage is null:
            set it explicitly (apply the overlay, or point at a
            wl-clipboard).
          '';
        }
        {
          assertion = clip.menuPackage != null;
          message = ''
            programs.scoot.desktop.clipboard.enable is set but
            programs.scoot.desktop.clipboard.menuPackage is null: set it
            explicitly (apply the overlay, or point at a fuzzel).
          '';
        }
        {
          assertion = clip.maxItems >= 1;
          message = ''
            programs.scoot.desktop.clipboard.maxItems is ${toString clip.maxItems}:
            keep at least 1 entry (0 would store nothing while claiming
            to keep history).
          '';
        }
        {
          # The db path renders inside single quotes in three shell
          # contexts that must agree on one file: the store entry, the
          # picker's `list`/`decode`, and the idle policy's lock line
          # (swayidle wordexp-parses that line, then `sh -c` runs it --
          # an unquoted `~` there would expand to `$HOME` while the
          # scripts keep a literal `~`, wiping a different db than the
          # history lives in; a space would field-split the wipe, and a
          # glob, pipe, redirect or paren would be read by swayidle's
          # wordexp/`sh -c` too). So an allowlist, not a blocklist: an
          # absolute path of letters, digits and `._/+@-` only
          # (`builtins.match` needs the whole value).
          assertion = clip.dbPath == null || builtins.match "^/[A-Za-z0-9._/+@-]*$" clip.dbPath != null;
          message = ''
            programs.scoot.desktop.clipboard.dbPath must be an absolute
            path made only of letters, digits and `._/+@-` (e.g.
            `/home/you/.cache/cliphist/db`), or null for cliphist's
            default. A `~` or relative path would name a different file
            on the lock line than in the store entry and the picker.
          '';
        }
      ];

      # The entry script runs from its store path in the watcher units
      # below, not from PATH: installing it would put a second copy of
      # the unit's shape into every profile user's profile for no
      # runtime need (the same reason the notify-sync bridge is not
      # installed).
      home.packages =
        lib.optional (clip.managerPackage != null) clip.managerPackage
        ++ lib.optional (clip.wlClipboardPackage != null) clip.wlClipboardPackage
        ++ lib.optional (clip.menuPackage != null) clip.menuPackage;

      systemd.user.services.scoot-clipboard-store = lib.mkIf toolsReady (
        mkStoreUnit "scoot clipboard history (regular selection)" ""
      );

      systemd.user.services.scoot-clipboard-primary-store = lib.mkIf toolsReady (
        mkStoreUnit "scoot clipboard history (primary selection)" "--primary "
      );
    })

    # The profile turns the slot on (still individually disable-able at
    # plain priority, the way the notification daemon works).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.clipboard.enable = lib.mkDefault true;
    })
  ];
}
