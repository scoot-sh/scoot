# `programs.scoot.desktop.apps` and `.automount` for home-manager: the
# terminal, the file manager plus the shared `xdg-open` plumbing, the
# WiFi/Bluetooth pickers, and the trayless automounter. The option
# shapes live in `./desktop.nix` (shared with the NixOS side); the
# `package` defaults live here because only this side has `pkgs`. See
# site/src/content/docs/desktop/index.md#terminal-files-and-removable-media
# ("Terminal, files, and removable media") and
# site/src/content/docs/desktop/index.md#wifi-and-bluetooth ("WiFi and
# Bluetooth").
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  apps = cfg.desktop.apps;
  automount = cfg.desktop.automount;

  # The desktop profile's shared option subtree and the one helper
  # that reads look-derived values (never hand-mapped here).
  themeLook = import ./theme-look.nix { inherit lib; };

  isLinux = pkgs.stdenv.hostPlatform.isLinux;
  linuxPkg = name: if isLinux then (pkgs.${name} or null) else null;

  # Absolute tool paths when the slot knows a package, bare names from
  # PATH otherwise (off Linux, or a direct-module setup without the
  # overlay): a missing tool then fails loudly in the script (its own
  # message on stderr, exit 1 -- never wedging input, since a bind is
  # one `spawn`), so the binds are always safe to render.
  binOr = name: pkg: if pkg != null then "${lib.getExe' pkg name}" else name;

  # The pickers' menus: absolute fuzzel when the slot names one (the
  # one fuzzel the launcher already themes: same derivation, no
  # second copy), bare name from PATH otherwise, with the same
  # fail-loud contract as above.
  networkMenu = binOr "fuzzel" apps.network.menuPackage;
  bluetoothMenu = binOr "fuzzel" apps.bluetooth.menuPackage;
  nmcliBin = binOr "nmcli" apps.network.cliPackage;
  blueBin = binOr "bluetoothctl" apps.bluetooth.cliPackage;
  # The keyring lookup is best-effort: a key stashed there (the
  # secrets slot's keyring) joins without a prompt, and a miss -- no
  # entry, no keyring, no `secret-tool` -- falls through to the
  # picker's masked prompt. Absolute when the secrets slot names its
  # client, bare otherwise.
  secretBin =
    if cfg.desktop.secrets.clientPackage != null then
      lib.getExe' cfg.desktop.secrets.clientPackage "secret-tool"
    else
      "secret-tool";
  # The audio slot's sink helper, which the Bluetooth picker
  # delegates its audio row to (`list` fills the menu, `set` runs
  # the pick -- see `audio-home.nix`). Absolute when the audio slot
  # built it (always, where that module is imported), bare otherwise.
  sinkBin =
    if (cfg.desktop.audio.scripts.sink or null) != null then
      "${cfg.desktop.audio.scripts.sink}/bin/scoot-audio-sink"
    else
      "scoot-audio-sink";

  # The pickers' theme: the look's roles as fuzzel CLI colors, the
  # same seven leaves the launcher and the clipboard picker carry
  # (one menu, one palette -- see `fuzzel-theme.nix`). Nothing
  # without a look (or opted out): fuzzel's own style stands.
  fuzzelTheme = import ./fuzzel-theme.nix { inherit lib; };
  themeFor =
    target:
    lib.optionalString (themeLook.themed cfg.desktop target) (
      fuzzelTheme (themeLook.lookFor cfg.desktop)
    );

  # What both pickers share: awk for the parsing, coreutils' `timeout`
  # bounding the keyring and notification calls, and `notify-send`
  # saying a failure where a keyboard user looks (a picker spawned by
  # a bind has no terminal; stderr still carries every message).
  shared = {
    "@AWK@" = binOr "awk" (if isLinux then pkgs.gawk else null);
    "@TIMEOUT@" = binOr "timeout" (if isLinux then pkgs.coreutils else null);
    "@NOTIFY@" = binOr "notify-send" (linuxPkg "libnotify");
  };
  # A picker script from its shell file, every `@NAME@` replaced by a
  # shell-quoted value (a tool path, the theme flags, a switch).
  pickerScript =
    name: file: values:
    let
      all = shared // values;
    in
    pkgs.writeShellScriptBin name (
      builtins.replaceStrings (builtins.attrNames all) (map lib.escapeShellArg (
        builtins.attrValues all
      )) (builtins.readFile file)
    );

  # WiFi through `nmcli` (see `apps-network-pick.sh` for the entry
  # points and the joining rules): the keymap's bind runs `pick`, the
  # bar's `network` module runs `menu` and `connect` (wired in
  # `scootbar.nix`).
  networkPick = pickerScript "scoot-network-pick" ./apps-network-pick.sh {
    "@NMCLI@" = nmcliBin;
    "@MENU@" = networkMenu;
    "@SECRET@" = secretBin;
    "@THEME@" = themeFor "network";
  };

  # Bluetooth through `bluetoothctl` (see `apps-bluetooth-pick.sh`):
  # the keymap's bind runs `pick`, the bar's `bluetooth` module runs
  # `menu` (wired in `scootbar.nix`). The audio row appears only with
  # the audio slot on, which is what builds the sink helper it runs.
  bluetoothPick = pickerScript "scoot-bluetooth-pick" ./apps-bluetooth-pick.sh {
    "@BT@" = blueBin;
    "@MENU@" = bluetoothMenu;
    "@SINK@" = sinkBin;
    "@AUDIO@" = if cfg.desktop.audio.enable or false then "1" else "0";
    "@THEME@" = themeFor "bluetooth";
  };

  # Whether any file-opening app is managed (the terminal counts:
  # `xdg-open` answers from a terminal even with no manager).
  filesInUse = apps.terminal.enable || apps.fileManager.enable;
  # Whether `xdg-open` must resolve (file opens, or the
  # automounter's Browse action).
  openInUse = filesInUse || automount.enable;

  # The bus probe the automounter's unit waits on: without udisks2
  # on the system bus (a home-manager-only setup without the NixOS
  # side) the unit skips instead of restart-looping every 2 s
  # forever -- a skipped start retries nothing and wakes nothing,
  # and `systemctl --user status` says why. The name only needs to
  # be KNOWN (active or activatable: udisks2 idles activatable on a
  # stock NixOS, and udiskie's first call wakes it); where the NixOS
  # side runs udisks2 the name is always there.
  busctlBin = binOr "busctl" (if isLinux then pkgs.systemd or null else null);

  # The automounter's own config (JSON, which udiskie reads beside
  # YAML): every device on the machine's own disks ignored, removable
  # media (no `HintSystem`) and loop images handled as udiskie ships.
  udiskieConfig = pkgs.writeText "scoot-udiskie.json" (
    builtins.toJSON {
      device_config = [
        {
          is_external = false;
          is_loop = false;
          ignore = true;
        }
      ];
    }
  );

in
{
  options.programs.scoot.desktop.apps = {
    # The tools below are Linux-only: their attributes refuse
    # evaluation when forced on Darwin, so `or null` alone does not
    # save them (the Darwin `nix flake check` run reads every
    # default). Off Linux each defaults to null, which the
    # assertions below refuse loudly instead of installing nothing
    # silently.
    terminal.package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = linuxPkg "foot";
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.foot else null";
      description = ''
        The terminal the compositor's built-in `Super+Return` bind
        spawns (must speak `foot`'s invocation: the bind names it).
        Null installs nothing. Linux-only: null off Linux.
      '';
    };

    fileManager.package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = linuxPkg "pcmanfm";
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.pcmanfm else null";
      description = ''
        The file manager to install (pcmanfm: the smallest closure
        of the maintained graphical set -- 347 MiB against thunar's
        376 and the wrapped yazi's 525 at the pinned rev -- with no
        daemon when closed, so nothing wakes idle; point at
        `pkgs.yazi` for the terminal shape, adjusting
        `desktopEntry` beside it). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    # Which `.desktop` entry opens directories (`xdg-open` answers
    # through it, and so does the automounter's Browse action):
    # the file manager's own, so an override names its own.
    fileManager.desktopEntry = lib.mkOption {
      type = lib.types.str;
      default = "pcmanfm.desktop";
      example = "thunar.desktop";
      description = ''
        The `.desktop` entry that opens directories (the file
        manager's own: pcmanfm ships `pcmanfm.desktop`). Read only
        with `fileManager.enable`.
      '';
    };

    network.cliPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = linuxPkg "networkmanager";
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.networkmanager else null";
      description = ''
        The WiFi tool the picker lists and connects through
        (`nmcli`: read-only list, keyboard-driven connect -- the
        service itself is never touched). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    network.menuPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = linuxPkg "fuzzel";
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel else null";
      description = ''
        The dmenu-style menu the WiFi picker runs
        (`fuzzel --dmenu`, themed by the look -- the launcher,
        which reuses this same package). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    bluetooth.cliPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = linuxPkg "bluez";
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.bluez else null";
      description = ''
        The Bluetooth tool the picker lists and connects through
        (`bluetoothctl`: paired-device connect, power toggle --
        the service itself is never touched). Null installs
        nothing. Linux-only: null off Linux.
      '';
    };

    bluetooth.menuPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = linuxPkg "fuzzel";
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.fuzzel else null";
      description = ''
        The dmenu-style menu the Bluetooth picker runs
        (`fuzzel --dmenu`, themed by the look -- the launcher,
        which reuses this same package). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    # Internal wiring, not user options: the two picker scripts
    # above, built once here and read by the keymap
    # (`keys-home.nix` binds them while their slots are on). Set
    # unconditionally -- pure derivations, so they evaluate
    # everywhere (bare tool names off Linux) and are built only
    # where installed.
    scripts = lib.mkOption {
      type = lib.types.attrsOf lib.types.package;
      default = { };
      internal = true;
      visible = false;
      description = ''
        Internal: the apps slot's picker scripts (network,
        bluetooth), read by the keymap. Not for direct use.
      '';
    };
  };

  options.programs.scoot.desktop.automount = {
    # The tool below is Linux-only, guarded the way every other
    # slot's tool is. Off Linux it defaults to null, which the
    # assertion below refuses loudly.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = linuxPkg "udiskie";
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.udiskie else null";
      description = ''
        The automounter to run trayless (`udiskie -a -n -T`:
        automount on insert, a notification with a Browse action
        through `xdg-open`, no tray icon -- the bar, not a tray,
        is where state shows). Null installs nothing. Linux-only:
        null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The scripts, built once here for the keymap to read (see
    # `scripts` above): pure derivations, set without the slots so
    # the binds always have something to name.
    {
      programs.scoot.desktop.apps.scripts = {
        network = networkPick;
        bluetooth = bluetoothPick;
      };
    }

    # The terminal: its package on PATH (the compositor's
    # built-in `Super+Return` bind names `foot`, so installing it
    # keeps that bind true), and `TERMINAL` for everything that
    # asks (at `mkDefault`, so an explicit value still wins).
    (lib.mkIf apps.terminal.enable {
      assertions = [
        {
          assertion = apps.terminal.package != null;
          message = ''
            programs.scoot.desktop.apps.terminal.enable is set but
            programs.scoot.desktop.apps.terminal.package is null: set
            it explicitly (apply the overlay, or point at a foot).
          '';
        }
      ];

      home.packages = lib.optional (apps.terminal.package != null) apps.terminal.package;

      systemd.user.sessionVariables = {
        TERMINAL = lib.mkDefault "foot";
      };
    })

    # The file manager: its package on PATH, and the directory
    # association `xdg-open` answers through (gated on the slot, so
    # a disabled manager never leaves a dangling entry behind --
    # `xdg-open` then fails loud, never wedged on a missing file).
    (lib.mkIf apps.fileManager.enable {
      assertions = [
        {
          assertion = apps.fileManager.package != null;
          message = ''
            programs.scoot.desktop.apps.fileManager.enable is set but
            programs.scoot.desktop.apps.fileManager.package is null:
            set it explicitly (apply the overlay, or point at a file
            manager).
          '';
        }
        {
          assertion = apps.fileManager.desktopEntry != "";
          message = ''
            programs.scoot.desktop.apps.fileManager.desktopEntry is
            empty: name the `.desktop` entry that opens directories
            (e.g. `pcmanfm.desktop`).
          '';
        }
      ];

      home.packages = lib.optional (apps.fileManager.package != null) apps.fileManager.package;
    })

    # The shared `xdg-open` plumbing, while anything opens files (or
    # the automounter browses): `xdg-open` on PATH, the user dirs
    # (Downloads, Pictures -- where the capture slot writes -- and
    # the rest, created), the directory association above (only
    # with the manager: without it `xdg-open` fails loud on
    # directories instead of naming a file that is not installed),
    # and `BROWSER` for everything that asks (at `mkDefault`: a URL
    # opens through the scheme handler, not a hardcoded browser --
    # there is no browser slot).
    (lib.mkIf openInUse {
      home.packages = lib.optional (linuxPkg "xdg-utils" != null) (linuxPkg "xdg-utils");

      systemd.user.sessionVariables = {
        BROWSER = lib.mkDefault "xdg-open";
      };
    })
    (lib.mkIf filesInUse {
      xdg.userDirs = {
        enable = true;
        createDirectories = true;
      };

      xdg.mimeApps = lib.mkIf apps.fileManager.enable {
        enable = true;
        defaultApplications = {
          # Directories -- plus mount points, which the shared-mime
          # database types separately (`inode/mount-point`: /tmp, and
          # every automounted drive the Browse action opens -- without
          # this `xdg-open` on a mount falls through to the browsers).
          "inode/directory" = apps.fileManager.desktopEntry;
          "inode/mount-point" = apps.fileManager.desktopEntry;
        };
      };
    })

    # The WiFi picker: its tools on PATH, and the scripts beside
    # the binds that call them (Linux only: the keymap renders the
    # binds for the Linux box they deploy to and installs nothing
    # on Darwin, the way the other slot scripts do).
    (lib.mkIf apps.network.enable {
      assertions = [
        {
          assertion = apps.network.cliPackage != null;
          message = ''
            programs.scoot.desktop.apps.network.enable is set but
            programs.scoot.desktop.apps.network.cliPackage is null:
            set it explicitly (apply the overlay, or point at a
            networkmanager).
          '';
        }
        {
          assertion = apps.network.menuPackage != null;
          message = ''
            programs.scoot.desktop.apps.network.enable is set but
            programs.scoot.desktop.apps.network.menuPackage is null:
            set it explicitly (apply the overlay, or point at a
            fuzzel).
          '';
        }
      ];

      home.packages =
        lib.optional (apps.network.cliPackage != null) apps.network.cliPackage
        ++ lib.optional (apps.network.menuPackage != null) apps.network.menuPackage
        ++ lib.optionals isLinux [ networkPick ];
    })

    # The Bluetooth picker: same shape (Linux only, same reason).
    (lib.mkIf apps.bluetooth.enable {
      assertions = [
        {
          assertion = apps.bluetooth.cliPackage != null;
          message = ''
            programs.scoot.desktop.apps.bluetooth.enable is set but
            programs.scoot.desktop.apps.bluetooth.cliPackage is null:
            set it explicitly (apply the overlay, or point at a
            bluez).
          '';
        }
        {
          assertion = apps.bluetooth.menuPackage != null;
          message = ''
            programs.scoot.desktop.apps.bluetooth.enable is set but
            programs.scoot.desktop.apps.bluetooth.menuPackage is null:
            set it explicitly (apply the overlay, or point at a
            fuzzel).
          '';
        }
      ];

      home.packages =
        lib.optional (apps.bluetooth.cliPackage != null) apps.bluetooth.cliPackage
        ++ lib.optional (apps.bluetooth.menuPackage != null) apps.bluetooth.menuPackage
        ++ lib.optionals isLinux [ bluetoothPick ];
    })

    # The automounter: its package on PATH, and the trayless daemon
    # (`-a` automounts on insert, `-n` notifies with a Browse action
    # through `xdg-open`, `-T` keeps it off the tray: state shows in
    # the bar's modules, not an icon), wanted by `scoot-session.target`
    # -- scoot's own session scope, never the shared
    # `graphical-session.target`. Its config ignores the machine's own
    # disks (udisks2's `HintSystem`): udiskie's stock rules skip only an
    # internal disk's whole-disk device, so at startup it would try to
    # mount every unmounted internal partition -- a dual-boot box's
    # Windows or macOS volumes -- and each try is a polkit admin prompt
    # at login. Safe removal is `udiskie-umount -d` (beside the daemon
    # in the same package): unmount and power the drive off before
    # unplugging, or buffered writes die with the yank (see
    # site/src/content/docs/desktop/index.md#terminal-files-and-removable-media).
    # A null beside `enable` leaves the unit out (the loud assertion
    # below, not a throw inside `getExe`: the same guard the bar's
    # unit uses, since standalone evals collect assertions without
    # enforcing them).
    (lib.mkIf automount.enable {
      assertions = [
        {
          assertion = automount.package != null;
          message = ''
            programs.scoot.desktop.automount.enable is set but
            programs.scoot.desktop.automount.package is null: set it
            explicitly (apply the overlay, or point at a udiskie).
          '';
        }
      ];

      home.packages = lib.optional (automount.package != null) automount.package;

      systemd.user.services.scoot-automount = lib.mkIf (isLinux && automount.package != null) {
        Unit = {
          Description = "scoot removable-media automount (trayless udiskie over udisks2)";
          PartOf = [ "scoot-session.target" ];
          After = [ "scoot-session.target" ];
          # Unending retries, like the bar's unit
          # (`StartLimitIntervalSec` lives in `[Unit]`: systemd
          # ignores it in `[Service]`).
          StartLimitIntervalSec = 0;
        };
        Service = {
          # Skipped (not restarted) without udisks2 known on the
          # system bus: a skipped start wakes nothing, and the status
          # says why (see `busctlBin` above).
          ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '${busctlBin} --system --no-pager list | ${lib.getExe' pkgs.gnugrep "grep"} -q ^org.freedesktop.UDisks2'";
          ExecStart = "${lib.getExe' automount.package "udiskie"} -a -n -T -c ${udiskieConfig}";
          Restart = "on-failure";
          RestartSec = 2;
        };
        Install.WantedBy = [ "scoot-session.target" ];
      };
    })

    # The profile turns the terminal, both pickers and the
    # automounter on (each still individually disable-able at plain
    # priority, the way the clipboard slot works). The file manager
    # stays optional: nothing references one, and a graphical one
    # costs a toolkit -- enable it for mouse-driven browsing.
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.apps.terminal.enable = lib.mkDefault true;
      programs.scoot.desktop.apps.network.enable = lib.mkDefault true;
      programs.scoot.desktop.apps.bluetooth.enable = lib.mkDefault true;
      programs.scoot.desktop.automount.enable = lib.mkDefault true;
    })
  ];
}
