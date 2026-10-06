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
  # The keyring lookup is best-effort: without the secrets slot
  # nothing answers it, so the picker says the one terminal command
  # that joins instead. Absolute when the secrets slot names its
  # client, bare otherwise (a missing `secret-tool` then simply finds
  # nothing: `command -v` guards it, so no new dependency).
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
  networkThemeFlags = lib.optionalString (themeLook.themed cfg.desktop "network") (
    fuzzelTheme (themeLook.lookFor cfg.desktop)
  );
  bluetoothThemeFlags = lib.optionalString (themeLook.themed cfg.desktop "bluetooth") (
    fuzzelTheme (themeLook.lookFor cfg.desktop)
  );

  # WiFi through `nmcli`, picked through the launcher's dmenu
  # contract (lines on stdin, selection on stdout -- the same shape
  # the clipboard picker and the launcher speak, which is what the
  # bar's `network` module calls as its `menu-command`, with
  # `connect` as its `connect-command`).
  #
  # Three entry points, one contract (stdout owns the picker, like
  # the audio slot's sink helper; everything else reports to
  # stderr):
  # - `list` prints candidate SSIDs (saved connections first, then
  #   the cached scan, deduplicated): a first join from a terminal
  #   stores a system connection, so the daily switch is one row.
  # - `pick` runs `list` through the menu below (the keymap's bind).
  # - `menu` runs the lines on stdin through the menu below: the
  #   bar's `network` module calls this as its `menu-command`,
  #   feeding the cached scan (so a hidden SSID stays hidden with
  #   `show-ssid = false`: an empty feed shows nothing instead of
  #   listing from the cache beside it).
  # - `connect <ssid>` joins: a saved connection goes up by UUID
  #   (no secret needed); an open network connects directly; a
  #   secured one reads its psk from the keyring (`secret-tool
  #   lookup scoot-wifi <ssid>`, stored once with `secret-tool
  #   store --label="WiFi <ssid>" scoot-wifi <ssid>`); anything else
  #   fails loud with the one terminal command that joins it (and
  #   the bar's `connect-command` calls exactly this, with the
  #   row's SSID as its last argument).
  # No WiFi device (a VM, a headless box) fails loud, naming it;
  # without NetworkManager running, nmcli's own error propagates
  # (never swallowed: a silent pick would join nothing while
  # looking like it did).
  #
  # nmcli's `-t` escapes `:` as `\:` (and `\` as `\\`), so the
  # parse splits on unescaped colons only: an SSID carrying a colon
  # (a captive portal's "Hotel: Lobby", say) survives the round
  # trip instead of splitting into two rows.
  networkPick = pkgs.writeShellScriptBin "scoot-network-pick" ''
    NMCLI=${lib.escapeShellArg nmcliBin}
    MENU=${lib.escapeShellArg networkMenu}
    THEME="${networkThemeFlags}"
    # shellcheck disable=SC2086
    nm() { $NMCLI "$@"; }
    unesc() {
      ${lib.getExe' pkgs.gawk "awk"} '
        function unesc(s,   out, i, c) {
          out = ""
          for (i = 1; i <= length(s); i++) {
            c = substr(s, i, 1)
            if (c == "\\" && i < length(s)) { i++; out = out substr(s, i, 1) }
            else { out = out c }
          }
          return out
        }
        { print unesc($0) }
      '
    }
    # Split one `-t` line on unescaped colons, printing field N (1-based).
    field() {
      n="$1"
      ${lib.getExe' pkgs.gawk "awk"} -v n="$n" '
        {
          f = 1; cur = ""; line = $0; done = 0
          for (i = 1; i <= length(line); i++) {
            c = substr(line, i, 1)
            if (c == "\\" && i < length(line)) { cur = cur c substr(line, i + 1, 1); i++ }
            else if (c == ":") { f++; if (f > n && !done) { print cur; done = 1 } cur = "" }
            else { cur = cur c }
          }
          if (f == n && !done) print cur
        }
      '
    }
    wifi_devs() {
      out="$(nm -t -f DEVICE,TYPE,STATE dev 2>&1)" || {
        echo "scoot-network-pick: nmcli cannot talk to NetworkManager ($out)" >&2
        return 1
      }
      printf '%s\n' "$out" | ${lib.getExe' pkgs.gawk "awk"} -F: '
        { dev = ""; type = ""; i = 1; cur = ""; line = $0
          n = split("", parts)
          f = 1; cur = ""
          for (i = 1; i <= length(line); i++) {
            c = substr(line, i, 1)
            if (c == "\\" && i < length(line)) { cur = cur substr(line, i, 2); i++ }
            else if (c == ":") { parts[f++] = cur; cur = "" }
            else { cur = cur c }
          }
          parts[f] = cur
          if (parts[2] == "wifi" && parts[3] !~ /unavailable|unmanaged/) print parts[1]
        }
      '
    }
    saved_ssids() {
      nm -t -f NAME,UUID,TYPE connection show 2>/dev/null | ${lib.getExe' pkgs.gawk "awk"} '
        { line = $0; f = 1; cur = ""; n = split("", parts)
          for (i = 1; i <= length(line); i++) {
            c = substr(line, i, 1)
            if (c == "\\" && i < length(line)) { cur = cur substr(line, i, 2); i++ }
            else if (c == ":") { parts[f++] = cur; cur = "" }
            else { cur = cur c }
          }
          parts[f] = cur
          if (parts[3] ~ /wifi|wireless/) print parts[1]
        }
      ' | unesc
    }
    scan_ssids() {
      nm -t -f SSID,SIGNAL,SECURITY dev wifi list --rescan no 2>/dev/null | field 1 | unesc | ${lib.getExe' pkgs.gawk "awk"} 'NF'
    }
    do_list() {
      devs="$(wifi_devs)" || exit 1
      if [ -z "$devs" ]; then
        echo "scoot-network-pick: no Wi-Fi device (this box has no wireless hardware, or it is unavailable)" >&2
        exit 1
      fi
      { saved_ssids; scan_ssids; } | ${lib.getExe' pkgs.gawk "awk"} '!seen[$0]++'
    }
    do_connect() {
      ssid="$1"
      if [ -z "$ssid" ]; then
        echo "usage: scoot-network-pick connect <ssid>" >&2
        exit 1
      fi
      devs="$(wifi_devs)" || exit 1
      if [ -z "$devs" ]; then
        echo "scoot-network-pick: no Wi-Fi device (this box has no wireless hardware, or it is unavailable)" >&2
        exit 1
      fi
      # A saved connection goes up by UUID (no secret needed: the
      # first join stored it system-side).
      uuid="$(nm -t -f NAME,UUID,TYPE connection show 2>/dev/null | while IFS= read -r line; do
        name="$(printf '%s\n' "$line" | field 1 | unesc)"
        if [ "$name" = "$ssid" ]; then printf '%s\n' "$line" | field 2; fi
      done | head -1)"
      if [ -n "$uuid" ]; then
        exec $NMCLI connection up uuid "$uuid"
      fi
      # Otherwise the cached scan says open or secured (an SSID
      # never seen scans as secured: better a keyring miss with
      # instructions than an open join attempted blind). The loop
      # prints `seen:` plus the security, so an open network (empty
      # security) still reads as seen -- an empty answer means the
      # scan never saw it at all.
      sec="$(nm -t -f SSID,SIGNAL,SECURITY dev wifi list --rescan no 2>/dev/null | while IFS= read -r line; do
        name="$(printf '%s\n' "$line" | field 1 | unesc)"
        if [ "$name" = "$ssid" ]; then printf 'seen:%s\n' "$(printf '%s\n' "$line" | field 3)"; fi
      done | head -1)"
      if [ -z "$sec" ]; then
        if command -v ${secretBin} >/dev/null 2>&1; then
          psk="$(${secretBin} lookup scoot-wifi "$ssid" 2>/dev/null)" || psk=""
        else
          psk=""
        fi
        if [ -n "$psk" ]; then
          exec $NMCLI dev wifi connect "$ssid" password "$psk"
        fi
        echo "scoot-network-pick: '$ssid' is new here: join it once from a terminal," >&2
        echo "  nmcli dev wifi connect '$ssid' --ask" >&2
        echo "or stash its key first: secret-tool store --label='WiFi $ssid' scoot-wifi '$ssid'" >&2
        exit 1
      fi
      if [ "$sec" = "seen:" ]; then
        exec $NMCLI dev wifi connect "$ssid"
      fi
      if command -v ${secretBin} >/dev/null 2>&1; then
        psk="$(${secretBin} lookup scoot-wifi "$ssid" 2>/dev/null)" || psk=""
      else
        psk=""
      fi
      if [ -n "$psk" ]; then
        exec $NMCLI dev wifi connect "$ssid" password "$psk"
      fi
      echo "scoot-network-pick: '$ssid' needs its key: secret-tool store --label='WiFi $ssid' scoot-wifi '$ssid'," >&2
      echo "then pick it again (or join once from a terminal: nmcli dev wifi connect '$ssid' --ask)" >&2
      exit 1
    }
    # The menu half: stdin lines through fuzzel into `connect`
    # (a cancel -- Escape, or an empty feed with `--no-run-if-empty`
    # -- joins nothing: the same contract as the clipboard picker's
    # cancel).
    menu_pick() {
      sel="$($MENU --dmenu --prompt='wifi: ' --no-run-if-empty --only-match $THEME)" || exit 0
      case "$sel" in
        "") exit 0 ;;
      esac
      do_connect "$sel"
    }
    case "''${1:-}" in
      list) do_list ;;
      pick) do_list | menu_pick ;;
      menu) menu_pick ;;
      connect) do_connect "''${2:-}" ;;
      *)
        echo "usage: scoot-network-pick {list|pick|menu|connect <ssid>}" >&2
        exit 1
        ;;
    esac
  '';

  # Bluetooth through `bluetoothctl`, picked the same way (which is
  # what the bar's `bluetooth` module calls as its device picker).
  # Pairing a never-seen device needs physical confirmation on the
  # device itself, so the picker only connects what is already
  # paired (a new device pairs from a terminal: `bluetoothctl pair
  # <mac>`); the daily connect/disconnect, the power toggle and the
  # audio-sink switch all work from the keyboard.
  #
  # - `list` prints paired devices (`MAC name`, a `*` first while
  #   connected): stdout owns the picker, everything else to
  #   stderr (the audio slot's contract).
  # - `pick` runs `list` through the menu below (the keymap's bind),
  #   with an `Audio output...` row delegating to the audio slot's
  #   sink helper.
  # - `menu` runs the lines on stdin through the menu below: the
  #   bar's `bluetooth` module calls this as its `menu-command`,
  #   feeding `Name` lines with a ` (connected)` suffix where
  #   connected (exactly what the bar shows, so the row a click
  #   offers is the row the picker toggles).
  # - `connect`/`disconnect <mac-or-name>` and `power <on|off>`
  #   run one action, loud on failure.
  # No controller (no hardware, no BlueZ) fails loud, naming it.
  bluetoothPick = pkgs.writeShellScriptBin "scoot-bluetooth-pick" ''
    BT=${lib.escapeShellArg blueBin}
    MENU=${lib.escapeShellArg bluetoothMenu}
    THEME="${bluetoothThemeFlags}"
    SINK=${lib.escapeShellArg sinkBin}
    check_ctl() {
      info="$($BT show 2>&1)" || {
        echo "scoot-bluetooth-pick: bluetoothctl cannot talk to BlueZ ($info)" >&2
        return 1
      }
      case "$info" in
        *"No default controller"*)
          echo "scoot-bluetooth-pick: no Bluetooth controller (no hardware, or BlueZ is not running)" >&2
          return 1
          ;;
      esac
    }
    is_connected() {
      $BT info "$1" 2>/dev/null | ${lib.getExe' pkgs.gawk "awk"} '/Connected: yes/ { found = 1 } END { exit !found }'
    }
    resolve() {
      want="$1"
      $BT devices Paired 2>/dev/null | ${lib.getExe' pkgs.gawk "awk"} -v want="$want" '
        /^Device / { mac = $2; $1 = ""; $2 = ""; sub(/^  /, ""); if (mac == want || $0 == want) print mac }
      ' | head -1
    }
    do_list() {
      check_ctl || exit 1
      $BT devices Paired 2>/dev/null | while IFS= read -r line; do
        mac="$(printf '%s\n' "$line" | ${lib.getExe' pkgs.gawk "awk"} '{ print $2 }')"
        name="$(printf '%s\n' "$line" | ${lib.getExe' pkgs.gawk "awk"} '{ $1 = ""; $2 = ""; sub(/^  /, ""); print }')"
        [ -n "$mac" ] || continue
        if is_connected "$mac"; then
          printf '* %s %s\n' "$mac" "$name"
        else
          printf '  %s %s\n' "$mac" "$name"
        fi
      done
    }
    do_toggle() {
      mac="$(resolve "$1")"
      if [ -z "$mac" ]; then
        echo "scoot-bluetooth-pick: no paired device matches '$1' (pair it first: bluetoothctl pair <mac>)" >&2
        exit 1
      fi
      if is_connected "$mac"; then
        exec $BT disconnect "$mac"
      else
        exec $BT connect "$mac"
      fi
    }
    case "''${1:-}" in
      list) do_list ;;
      pick)
        sel="$({ do_list; echo "Audio output..."; } | $MENU --dmenu --prompt='bluetooth: ' --no-run-if-empty --only-match $THEME)" || exit 0
        case "$sel" in
          "") exit 0 ;;
        esac
        case "$sel" in
          "Audio output...")
            sinks="$($SINK list)" || exit 1
            choice="$(printf '%s\n' "$sinks" | $MENU --dmenu --prompt='sink: ' --no-run-if-empty --only-match $THEME)" || exit 0
            case "$choice" in
              "") exit 0 ;;
            esac
            id="$(printf '%s\n' "$choice" | ${lib.getExe' pkgs.gawk "awk"} '{ print $1 }')"
            exec $SINK set "$id"
            ;;
          *)
            mac="$(printf '%s\n' "$sel" | ${lib.getExe' pkgs.gawk "awk"} '{ print $2 }')"
            do_toggle "$mac"
            ;;
        esac
        ;;
      # The bar-fed menu: stdin lines are `Name` with a ` (connected)`
      # suffix where connected (the bar's own row text), toggled by
      # name -- a fed device never paired here (visible but new) says
      # the pairing command instead of pairing blind.
      menu)
        sel="$($MENU --dmenu --prompt='bluetooth: ' --no-run-if-empty --only-match $THEME)" || exit 0
        case "$sel" in
          "") exit 0 ;;
        esac
        name="''${sel% \(connected\)}"
        do_toggle "$name"
        ;;
      connect)
        mac="$(resolve "''${2:-}")"
        [ -n "$mac" ] || {
          echo "scoot-bluetooth-pick: no paired device matches ''${2:-} (pair it first: bluetoothctl pair <mac>)" >&2
          exit 1
        }
        exec $BT connect "$mac"
        ;;
      disconnect)
        mac="$(resolve "''${2:-}")"
        [ -n "$mac" ] || {
          echo "scoot-bluetooth-pick: no paired device matches ''${2:-}" >&2
          exit 1
        }
        exec $BT disconnect "$mac"
        ;;
      power)
        case "''${2:-}" in
          on | off) exec $BT power "''${2}" ;;
          *)
            echo "usage: scoot-bluetooth-pick power <on|off>" >&2
            exit 1
            ;;
        esac
        ;;
      *)
        echo "usage: scoot-bluetooth-pick {list|pick|menu|connect <mac-or-name>|disconnect <mac-or-name>|power <on|off>}" >&2
        exit 1
        ;;
    esac
  '';

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

  # A null beside `enable` is the loud assertion below, not a throw
  # inside `getExe`: the same guard every other slot uses, since
  # standalone evals collect assertions without enforcing them.
  toolsReady =
    apps.network.cliPackage != null
    && apps.network.menuPackage != null
    && apps.bluetooth.cliPackage != null
    && apps.bluetooth.menuPackage != null;
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
    # the bar's modules, not an icon). Wanted by
    # `scoot-session.target` -- scoot's own session scope, never the
    # shared `graphical-session.target` -- and retried like the
    # bar's unit rather than conditioned (a skipped start is never
    # retried). Safe removal is `udiskie-umount` (beside the daemon
    # in the same package): unmount -- and detach (`-d`, powering
    # the drive off) -- before unplugging, or buffered writes die
    # with the yank (see site/src/content/docs/desktop/index.md#terminal-files-and-removable-media).
    # A null beside `enable` leaves the unit out (the loud assertion
    # above, not a throw inside `getExe`: the same guard the bar's
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
          ExecCondition = "${lib.getExe' pkgs.bash "bash"} -c '${busctlBin} --system --no-pager list | grep -q ^org.freedesktop.UDisks2'";
          ExecStart = "${lib.getExe' automount.package "udiskie"} -a -n -T";
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
