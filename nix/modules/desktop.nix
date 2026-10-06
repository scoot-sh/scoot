# The `programs.scoot.desktop` option subtree, shared by the NixOS module
# (`nixos.nix`) and the home-manager module (`home.nix`): one switch plus a
# look choice for essentially a full lightweight desktop, with a
# declared-but-off slot per future piece. Each side wires only what it owns
# (NixOS: system services, backend packages, the greeter; home-manager: user
# units, binds, theme files); either side alone degrades to what it can do.
# Later children fill the slot bodies without renaming options
# (the native-replacement contract: a scoot-native piece later replaces a
# slot without changing the user's config).
#
# Takes only `lib`: no `pkgs`, so the pure modules stay usable without the
# flake's overlay, and the look wallpapers resolve to store paths wherever
# the importing module is evaluated from.
{ lib }:

let
  # One boolean plus package override per slot that installs something.
  # All default off/null: declared now so later children only fill bodies,
  # never rename options. Enabling one today is accepted and inert (no
  # packages installed, no units or files written); the description names
  # the child ticket and the default tool it will wire.
  slot =
    {
      child,
      tool,
      extra ? "",
    }:
    {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Reserved for the `${child}` child (${tool}). Off: enabling it
          today is accepted and inert -- nothing is installed and nothing
          runs yet.${extra}
        '';
      };
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        description = ''
          The package the `${child}` child installs for this slot (${tool}).
          Null installs nothing.
        '';
      };
    };

  # A slot that installs nothing, only config when its child lands
  # (output wiring, an input-method setup): a boolean alone.
  # (`keys` used to be one of these; it now owns the shared keymap
  # below, so it declares its own subtree instead.)
  configSlot =
    { child, does }:
    {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Reserved for the `${child}` child (${does}). Off: enabling it
          today is accepted and inert.
        '';
      };
    };

  # The shared keymap's static half: per bind the `[binds]` combo,
  # which slot's `enable` gates it beside `keys.enable` (`null`
  # gates on nothing: the bind belongs to the keymap itself), and
  # the one-line blurb the option descriptions reuse. Actions live
  # in `keys-home.nix` (they need `pkgs` for store paths); the
  # combos live here so both sides and the tests read the one table.
  # Reserved-but-unbound combos (keyboard backlight, window capture,
  # power menu) are docs-only: see site/src/content/docs/desktop/index.md#xwayland-and-what-comes-next.
  keymap = {
    brightnessUp = {
      combo = "XF86MonBrightnessUp";
      slot = null;
      blurb = "panel brighter (`brightnessctl -e set +5%`)";
    };
    brightnessDown = {
      combo = "XF86MonBrightnessDown";
      slot = null;
      blurb = "panel dimmer (`brightnessctl -e set 5%-`)";
    };
    volumeUp = {
      combo = "XF86AudioRaiseVolume";
      slot = null;
      blurb = "default sink louder (`wpctl set-volume ... 5%+`)";
    };
    volumeDown = {
      combo = "XF86AudioLowerVolume";
      slot = null;
      blurb = "default sink quieter (`wpctl set-volume ... 5%-`)";
    };
    volumeMute = {
      combo = "XF86AudioMute";
      slot = null;
      blurb = "default sink mute toggle (`wpctl set-mute ... toggle`)";
    };
    micMute = {
      combo = "XF86AudioMicMute";
      slot = null;
      blurb = "default source mute toggle";
    };
    mediaPlay = {
      combo = "XF86AudioPlay";
      slot = null;
      blurb = "play/pause (`playerctl play-pause`)";
    };
    mediaPause = {
      combo = "XF86AudioPause";
      slot = null;
      blurb = "pause (`playerctl pause`)";
    };
    mediaStop = {
      combo = "XF86AudioStop";
      slot = null;
      blurb = "stop (`playerctl stop`)";
    };
    mediaNext = {
      combo = "XF86AudioNext";
      slot = null;
      blurb = "next track (`playerctl next`)";
    };
    mediaPrev = {
      combo = "XF86AudioPrev";
      slot = null;
      blurb = "previous track (`playerctl previous`)";
    };
    lock = {
      combo = "super+escape";
      slot = null;
      blurb = "lock through `idle.lock.command`";
    };
    launcher = {
      combo = "super+d";
      slot = "launcher";
      blurb = "launcher (`fuzzel` drun: XDG apps)";
    };
    launcherRun = {
      combo = "ctrl+alt+space";
      slot = "launcher";
      blurb = "run mode (`fuzzel --list-executables-in-path`: PATH executables beside apps)";
    };
    clipboard = {
      combo = "super+v";
      slot = "clipboard";
      blurb = "clipboard picker (`cliphist` through `fuzzel`)";
    };
    notifDismiss = {
      combo = "super+n";
      slot = "notifications";
      blurb = "dismiss visible notifications (`makoctl dismiss`)";
    };
    notifDnd = {
      combo = "super+shift+n";
      slot = "notifications";
      blurb = "do-not-disturb toggle (`makoctl mode -t do-not-disturb`)";
    };
    notifHistory = {
      combo = "super+ctrl+n";
      slot = "notifications";
      blurb = "show hidden notifications (`makoctl restore`)";
    };
    powerProfile = {
      combo = "super+p";
      slot = "power";
      blurb = "cycle the power profile (`scoot-power-profile cycle`: power-saver, balanced, performance)";
    };
    captureOutput = {
      combo = "print";
      slot = "capture";
      blurb = "screenshot every output to `~/Pictures` (`grim`)";
    };
    captureRegion = {
      combo = "shift+print";
      slot = "capture";
      blurb = "screenshot a picked region to `~/Pictures` (`grim` plus `slurp`)";
    };
    captureClipboard = {
      combo = "ctrl+print";
      slot = "capture";
      blurb = "screenshot a picked region into the clipboard (`grim` plus `slurp` plus `wl-copy`)";
    };
  };
  # Whether a notification state icon is shaped like the bar takes
  # it: empty (no icon for that state) or exactly one glyph. Nix
  # strings are byte strings and `lib.stringToCharacters` splits bytes
  # (a multibyte glyph reads as several "characters"), so this is a
  # byte rule: one printable ASCII byte, or up to four bytes none of
  # which is printable ASCII (meant as one UTF-8 sequence). Two holes
  # the byte rule cannot close without a codepoint counter Nix lacks:
  # two 2-byte glyphs ("éé", 4 bytes) and a control byte both pass
  # here, and the bar refuses such an icon per update (exactly one
  # non-control `char`), leaving that state's last value shown. Shared
  # by the feed and the bar half so both refuse the same values.
  isStateIcon =
    icon:
    icon == "" || (builtins.stringLength icon <= 4 && builtins.match "^[ -~]$|^[^ -~]+$" icon != null);
  # What logind may do on a lid or power-key event (logind.conf(5)
  # `Handle*=`): the benign set. `kexec` and `factory-reset` are left
  # out deliberately -- no lid or key should ever trigger those.
  logindAction = lib.types.enum [
    "ignore"
    "lock"
    "suspend"
    "hibernate"
    "hybrid-sleep"
    "suspend-then-hibernate"
    "poweroff"
    "reboot"
    "halt"
  ];
in
{
  # The shared keymap table above, beside the option subtree and the
  # look palettes: `keys-home.nix` reads it to render `[binds]`, so
  # the combos live in exactly one place.
  inherit keymap isStateIcon;
  # The option subtree, assigned to `options.programs.scoot.desktop` by
  # both modules.
  options = {
    enable = lib.mkEnableOption "the scoot desktop profile: the session wiring plus bar and wallpaper defaults, themed by `look`";

    # Null (the default) themes nothing: every value below is a default
    # the user or Stylix beats per key. An unknown value is an eval error
    # naming these four (the `enum` type's own message).
    look = lib.mkOption {
      type = lib.types.nullOr (
        lib.types.enum [
          "vinyl-sunset"
          "music-desk"
          "radial-burst"
          "moonrise"
        ]
      );
      default = null;
      example = "vinyl-sunset";
      description = ''
        One of `docs/examples/*`: applies that example's palette to every
        piece the flake owns today (the compositor `[appearance]` colors,
        the bar `colors`, the session wallpaper where one ships in the
        repository). Null themes nothing. Each value is a default a value
        you set in `settings` beats per key, and Stylix beats where
        present (Stylix stays the override path); see site/src/content/docs/scoot/theming.md#stylix.
      '';
    };

    # The bar half of `enable`: on, the profile enables and themes the bar
    # (through `programs.scootbar`, when that module is imported -- the
    # profile never requires it); off, the bar is entirely yours (enabled
    # or not, themed or not, the look leaves it alone). Read only with
    # `enable`: without it this does nothing.
    bar.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Whether `desktop.enable` enables and themes the bar
        (`programs.scootbar`, when imported). Set to `false` to own the
        bar yourself: the profile then neither enables it nor applies
        `look` colors to it. Read only with `desktop.enable`.
      '';
    };

    # The compositor half of XWayland support: on, the `[xwayland] enabled`
    # knob defaults on (read with `desktop.enable`). The package half stays
    # `programs.scoot.package`: point it at the flake's `scoot-xwayland`
    # (or `scoot-gpu-xwayland`) build for X11 apps -- see site/src/content/docs/scoot/xwayland.md.
    # The NixOS side accepts this and reserves it; the knob itself is
    # config-file (home-manager) wiring.
    xwayland.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Default `[xwayland] enabled` on (read with `desktop.enable`).
        Pair with an XWayland build as `programs.scoot.package` (the
        flake's `scoot-xwayland`); with the default package the knob
        warns and the session runs Wayland-only. See site/src/content/docs/scoot/xwayland.md.
      '';
    };

    # Idle policy (dim, lock, screens off, lock before sleep, media
    # inhibit) plus the locker behind it (over `ext-session-lock-v1`).
    # Filled by the `desktop-idle-lock` child: swayidle as a user unit
    # bound to `scoot-session.target`, the M2's measured timeouts as
    # defaults (dim to 10% at 2 min, lock at 4, screens off at 5, each
    # overridable, 0 disabling that step), lock-before-sleep through
    # logind, and an audio-driven idle inhibitor while media plays.
    # Each `package` and the lock `command` beside them are declared in
    # the side modules (`home.nix` installs for the user, `nixos.nix`
    # system-wide), which is also where their defaults live; everything
    # here is plain values, so this file stays `lib`-only.
    #
    # On with the profile (each still individually disable-able); without
    # it, `idle.enable` works standalone (unthemed: a look needs the
    # profile, and the user units need the home-manager side, the way the
    # themed config file does).
    idle = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Run the idle policy: dim, screens off and lock-before-sleep
          through swayidle (its package beside this), plus the audio
          inhibitor while media plays. The lock step inside it needs
          `lock.enable` below; without it the session still dims and
          sleeps unlocked, and sleep does not lock.
        '';
      };

      # Seconds of inactivity before the panel dims to `dimLevel`. 0
      # disables the step (swayidle never fires it).
      dimTimeout = lib.mkOption {
        type = lib.types.int;
        default = 120;
        example = 60;
        description = ''
          Seconds of inactivity before the panel dims to `dimLevel`
          percent (the M2's measured default: 2 min). 0 disables the
          step.
        '';
      };

      # Percent of full brightness the dim step sets (`brightnessctl -s
      # set <n>%`, restored with `-r` on activity). 1..100: 0 would be a
      # black panel with the session still unlocked, which is what the
      # screens-off step is for.
      dimLevel = lib.mkOption {
        type = lib.types.int;
        default = 10;
        example = 20;
        description = ''
          Brightness percent the dim step sets (the M2's measured
          default: 10%). 1 to 100.
        '';
      };

      # Seconds of inactivity before the session locks through
      # `lock.command` (so lid-close and manual locks share the path).
      # Before the screens-off step, so the lock is up before the panel
      # goes dark and no unlocked frame is ever visible on wake. 0
      # disables the step (sleep still locks through `before-sleep`
      # while `lock.enable` is on). Needs `lock.enable`.
      lockTimeout = lib.mkOption {
        type = lib.types.int;
        default = 240;
        example = 300;
        description = ''
          Seconds of inactivity before the session locks (default 4
          min: after dim, before screens off, so the lock is already up
          when the panel goes dark). 0 disables the step. Needs
          `lock.enable`.
        '';
      };

      # Seconds of inactivity before every output powers off (`wlopm`,
      # back on at the first input, locked or not). 0 disables it.
      offTimeout = lib.mkOption {
        type = lib.types.int;
        default = 300;
        example = 600;
        description = ''
          Seconds of inactivity before every output powers off (the
          M2's measured default: 5 min; back on at the first input,
          locked or not). 0 disables the step.
        '';
      };

      # Hold idle while audio plays, so music or a call never dims the
      # panel: `sway-audio-idle-inhibit` (any sink or source running)
      # holding a Wayland idle inhibitor. Needs PipeWire (or PulseAudio)
      # running; without an audio server the unit backs off and stays
      # stopped (see site/src/content/docs/desktop/index.md#idle-and-lock).
      mediaInhibit = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Hold idle while audio plays (`sway-audio-idle-inhibit`,
            its package beside this). Needs `idle.enable`.
          '';
        };
      };

      # The locker. `command` is the stable lock action the future
      # `desktop-keys` child binds (the `Super+Escape` class) without
      # renaming anything here; `daemon` names the locker behind it, so
      # a future scootlock widens that enum without changing the option
      # or the binds (the native-replacement contract).
      lock = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Lock the session: the locker behind `command` over
            `ext-session-lock-v1`, run from swayidle's `lock` event (so
            every path -- timeout, lid, manual, before-sleep -- lands on
            the same locker), themed by the look unless
            `theme.targets.lock.enable` is off. Needs `idle.enable`.
            Home-manager-only (non-NixOS) setups must provide a PAM
            service for the locker themselves, or no password will ever
            unlock.
          '';
        };

        daemon = lib.mkOption {
          type = lib.types.enum [ "swaylock" ];
          default = "swaylock";
          description = ''
            The locker behind `command`. Only swaylock today (smallest
            working closure at the pinned rev, plain-text config the
            look themes per leaf, CPU-only like the compositor); a
            future scootlock widens this enum, the option and the binds
            staying as they are.
          '';
        };

        # Free-form `key = value` lines merged over the look-themed
        # config (a value you set wins per key). An empty string renders
        # a bare flag (`{ show-failed-attempts = ""; }`). The look's
        # leaves stay when Stylix/the theme-look child arrives (user >
        # Stylix > look, per key); `theme.targets.lock.enable = false`
        # drops the themed block but keeps these.
        settings = lib.mkOption {
          type = lib.types.attrsOf lib.types.str;
          default = { };
          example = {
            font-size = "24";
            indicator-radius = "100";
          };
          description = ''
            Extra swaylock config lines, merged over the look-themed
            ones (a value here wins per key). Empty string renders a
            bare flag.
          '';
        };
      };
    };
    # mako now, scootnotify later without changing option names: the
    # daemon behind `daemon` is the only visible change when
    # scootnotify replaces it (same `enable`, same bar module, same
    # DND toggle). Filled by the `desktop-notifications` child: mako
    # as a user unit bound to `scoot-session.target`, its config
    # on the `overlay` layer (so popups show above fullscreen windows)
    # and themed by the look, plus the bar feed (DND state and unread
    # count into the bar's `push` module, a click toggling DND).
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (unthemed: a look needs the
    # profile, and the user unit needs the home-manager side, the way
    # the themed config file does).
    notifications = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Run the notification daemon: mako (its package beside this)
          owning `org.freedesktop.Notifications` on the session bus,
          with its popups on the `overlay` layer and a bar feed for
          DND state and the unread count. Without it nothing owns the
          name, and `Notify` calls fail.
        '';
      };

      daemon = lib.mkOption {
        type = lib.types.enum [ "mako" ];
        default = "mako";
        description = ''
          The daemon behind `enable`. Only mako today (a lean build
          without the GTK stack -- see `notifications-mako.nix`;
          X client libraries still ride along through cairo/pango,
          but mako has no X11 backend and cannot run on X11 -- a
          plain-text config the look themes per key); a future
          scootnotify widens this enum, the option and the bar module
          staying as they are.
        '';
      };

      # Free-form `key = value` lines merged over the generated config
      # (a value here wins per key). Rendered verbatim, so a `#rrggbb`
      # color keeps its leading `#` (mako's own format). The generated
      # sections (`[mode=do-not-disturb]`, `[urgency=critical]`) always
      # render; only global keys are overridable here.
      settings = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = { };
        example = {
          anchor = "bottom-right";
          max-visible = "3";
        };
        description = ''
          Extra mako config lines, merged over the generated ones (a
          value here wins per key). Rendered verbatim. The feed that
          surfaces DND state and the unread count into the bar watches
          the daemon over D-Bus, so every key stays overridable
          without breaking it. Overriding `layer` hides popups under
          fullscreen windows again (see site/src/content/docs/desktop/index.md#notifications).
        '';
      };

      # The bar face of the daemon: one icon per state in the bar's
      # `push` module (see `nix/modules/notifications-home.nix` for the
      # feed that sends them, `nix/modules/scootbar.nix` for the static
      # idle half). A future scootnotify keeps these names (the
      # native-replacement contract: the daemon changes, the options do
      # not). Each is empty (no icon for that state) or exactly one
      # glyph -- anything else fails evaluation, since the bar refuses
      # a multi-character icon per update. The defaults are all in
      # DejaVu Sans, the bar's default font, so they render with no
      # symbol font: a hollow circle for idle (empty), a solid dot
      # beside the count for unread (something here), and a crescent
      # moon for do-not-disturb (quiet hours -- DejaVu has no bell, so
      # no bell-slash; the envelope reads as a missing glyph at bar
      # size). A Nerd Font glyph works wherever `bar.fallback-fonts`
      # provides one.
      bar.icons = {
        idle = lib.mkOption {
          type = lib.types.str;
          default = "○";
          example = "✉";
          description = ''
            The icon the bar shows with no notifications and DND off
            (the push module's static icon). Empty shows nothing while
            idle.
          '';
        };

        unread = lib.mkOption {
          type = lib.types.str;
          default = "●";
          example = "✉";
          description = ''
            The icon the feed sends beside the unread count (a
            per-update icon, so it overrides `idle` while set). Empty
            sends no icon for this state, so the static `idle` icon
            shows beside the count (set `idle` empty too for the count
            alone).
          '';
        };

        dnd = lib.mkOption {
          type = lib.types.str;
          default = "☾";
          example = "Z";
          description = ''
            The icon the feed sends while do-not-disturb is on (a
            per-update icon, so it overrides `idle` while set). Empty
            sends no icon for this state, so the static `idle` icon
            shows beside the DND text (set `idle` empty too for the
            text alone).
          '';
        };
      };
    };
    # fuzzel now, scootlaunch later without changing option names: the
    # daemon behind `daemon` is the only visible change when
    # scootlaunch replaces it (same `enable`, same binds, same
    # `--dmenu` contract the clipboard picker and the bar's future
    # pickers call -- see `docs/scootbar/backlog/launcher.md`). Filled
    # by the `desktop-launcher` child: fuzzel on the keymap's `Super+d`
    # (drun: XDG apps from the user's and the system's `XDG_DATA_DIRS`)
    # and `Ctrl+Alt+Space` (run: PATH executables beside apps), on the
    # `overlay` layer (so it opens above fullscreen windows) with
    # exclusive keyboard, themed by the look.
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (unthemed: a look needs the
    # profile, and the themed flags need the home-manager side, the way
    # the clipboard picker does).
    launcher = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Open the launcher: fuzzel (its package beside this) listing
          XDG apps, most-launched first, with PATH executables on the
          run bind. The launcher holds nothing when closed (no daemon,
          no unit). Without it nothing answers the keymap's launcher
          binds, and they stay unbound.
        '';
      };

      daemon = lib.mkOption {
        type = lib.types.enum [ "fuzzel" ];
        default = "fuzzel";
        description = ''
          The program behind `enable`. Only fuzzel today (layer-shell
          `overlay` native, no toolkit, fastest cold start of the
          maintained set -- see site/src/content/docs/desktop/index.md#launcher
          for the measured pick); a
          future scootlaunch widens this enum, the option and the binds
          staying as they are.
        '';
      };
    };
    # Screenshots bound to keys, and screen sharing through portals.
    # Filled by the `desktop-capture` child: the portal backends
    # (`xdg-desktop-portal-wlr` for ScreenCast/Screenshot, `-gtk` for
    # the rest) behind `xdg.portal`, PipeWire running for the cast,
    # `grim` plus `slurp` for the keymap's three screenshot binds, and
    # the output chooser xdpw asks before each cast (a dmenu list
    # through fuzzel by default, themed by the look).
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (unthemed: a look needs the
    # profile, and the system backends need the NixOS side, the way
    # the themed config file does).
    capture = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Share the screen in calls and take screenshots: the portal
          backends (their packages beside this), PipeWire for the
          cast, and the keymap's three screenshot binds. Without it
          nothing answers ScreenCast/Screenshot and the capture binds
          stay unbound.
        '';
      };

      # Which output picker xdpw asks before each cast (its
      # `chooser_type`): `fuzzel` lists every output as a dmenu menu
      # through the profile's fuzzel (the same `--dmenu` contract the
      # clipboard picker and the launcher speak, themed by the look),
      # `slurp` picks by clicking a screen (xdpw's own `simple`
      # shape), `none` casts `outputName` (or any output) with no
      # picker at all. See site/src/content/docs/desktop/index.md#screenshots-and-screen-sharing.
      chooser = lib.mkOption {
        type = lib.types.enum [
          "fuzzel"
          "slurp"
          "none"
        ];
        default = "fuzzel";
        example = "slurp";
        description = ''
          The output chooser before each screencast. `fuzzel` (a dmenu
          list of outputs), `slurp` (click a screen), or `none` (no
          picker: cast `outputName`, or any output when that is null).
        '';
      };

      # The output `chooser = "none"` casts without asking (xdpw's
      # `output_name`, a connector name as `wayland-info` lists it,
      # e.g. `"eDP-1"`). Null casts any output. Read only with
      # `chooser = "none"`.
      outputName = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "eDP-1";
        description = ''
          The output cast with no picker (`chooser = "none"`). Null
          casts any output. Read only with `chooser = "none"`.
        '';
      };

      # Frames per second at most on a cast (xdpw's `max_fps`): 30 is
      # plenty for a call and bounds the compositor's copy cost; 0
      # means no limit. At least 0.
      maxFps = lib.mkOption {
        type = lib.types.int;
        default = 30;
        example = 60;
        description = ''
          Most frames per second on a screencast. 0 means no limit.
        '';
      };
    };
    # The privilege prompt.
    auth = slot {
      child = "desktop-auth-secrets";
      tool = "a polkit agent";
    };
    # The keyring.
    secrets = slot {
      child = "desktop-auth-secrets";
      tool = "a secrets service";
    };
    # Pipewire/wireplumber baseline, media keys, brightness keys and an OSD.
    audio = slot {
      child = "desktop-audio-osd";
      tool = "pipewire plus wireplumber, with the bar's volume, brightness, media and microphone modules as the display half";
    };
    # Clipboard persistence plus history and a picker bind. Filled by
    # the `desktop-clipboard` child: a lean cliphist (its contrib
    # pickers dropped -- see `clipboard-cliphist.nix`) watched by
    # `wl-paste`, `wl-copy`/`wl-paste` on PATH, and the history picker
    # on the keymap's `Super+v` (cliphist through fuzzel's dmenu mode,
    # themed by the look). Each `package` and the history bounds beside
    # them are declared in the side modules (`clipboard-home.nix`
    # installs and runs for the user, `nixos.nix` system-wide), which
    # is also where their defaults live; everything here is plain
    # values, so this file stays `lib`-only.
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (unthemed: a look needs the
    # profile, and the user units need the home-manager side, the way
    # the themed picker does).
    clipboard = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Keep the clipboard after its source closes: every copy lands
          in cliphist's history (its package beside this), restorable
          with the keymap's picker, with password-manager copies and
          anything copied while locked kept out (see site/src/content/docs/desktop/index.md#clipboard).
          `wl-copy`/`wl-paste` land on PATH beside it.
        '';
      };

      # How many entries the history keeps (cliphist's `max-items`,
      # oldest dropped first). 100 previews stay scannable in the
      # picker and bound the cache db (each entry at most 5 MB,
      # cliphist's own cap); raise it for a longer tail.
      maxItems = lib.mkOption {
        type = lib.types.int;
        default = 100;
        example = 250;
        description = ''
          History entries kept (oldest dropped first). At least 1.
        '';
      };

      # Where the history db lives. Null keeps cliphist's default
      # (`~/.cache/cliphist/db`, honoring `XDG_CACHE_HOME`): on disk,
      # so history survives reboots -- with secrets never landing in
      # it by construction (see site/src/content/docs/desktop/index.md#clipboard for the trade-off). Set it
      # to move the db: an absolute path without shell specials (no
      # `~`, spaces, quotes, `$`, backticks, `;` or backslashes -- the
      # store entry, the picker and the idle policy's lock wipe all
      # render it inside single quotes, so anything the shell would
      # expand or split names a different file on one line than the
      # others). Anything else fails evaluation.
      dbPath = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/home/you/.cache/cliphist/db";
        description = ''
          History db path (absolute; letters, digits and `._/+@-` only). Null keeps
          cliphist's default. Anything but an absolute path free of
          `~`, whitespace, quotes, `$`, backticks, `;` and backslashes
          fails evaluation.
        '';
      };
    };
    # Night light over `wlr-gamma-control-v1`.
    nightlight = slot {
      child = "desktop-nightlight";
      tool = "`wlsunset` (single purpose) or `gammastep` (undecided)";
    };
    # Power profiles, lid and low-battery suspend, charge limit.
    # Filled by the `desktop-power` child: `power-profiles-daemon` as
    # the system service (switched from the keymap's `Super+p` through
    # `scoot-power-profile`, or with `powerprofilesctl` directly), lid
    # and power-key policy plus low-battery suspend through logind and
    # UPower, and the M2's desk-aware charge-limit service (80%
    # default, full-once, trip) where the hardware has the sysfs node.
    # Each `package` lives beside this in the side modules (`nixos.nix`
    # installs system-wide, `power-home.nix` for the user), which is
    # also where their defaults live; everything here is plain values,
    # so this file stays `lib`-only.
    #
    # Opt-in (NOT on with the profile): lid-close suspend and an 80%
    # charge cap change what the machine does, with real consequences
    # on a remotely-driven box (suspend cuts SSH; the reference M2 is
    # driven over it), so the profile must not smuggle them in -- set
    # `power.enable` explicitly. Without it, `enable` works standalone
    # (unthemed: there is nothing the look themes here -- the charge
    # button inherits the bar's own colors -- and the daemons need the
    # NixOS side, the way the locker's PAM does).
    power = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Run the power policy: `power-profiles-daemon` (its package
          beside this) owning `org.freedesktop.UPower.PowerProfiles`
          on the system bus, the lid and power-key actions through
          logind, low-battery suspend through UPower, and the
          charge-limit service below. Opt-in, never with the profile:
          lid-close suspend and the charge cap change what the machine
          does (see site/src/content/docs/desktop/index.md#power).
        '';
      };

      # Whether the profiles daemon runs behind the keymap's switch.
      # On with the policy: `Super+p` cycles through
      # `powerprofilesctl`, and the daemon owns the bus name widgets
      # read. Off drops the daemon and its bind entirely -- for
      # hardware with no PPD driver (Apple silicon: the placeholder
      # lists power-saver + balanced as no-ops) the charge limit and
      # the lid policy below are what save battery, not profiles
      # (see site/src/content/docs/desktop/index.md#power).
      profiles = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          example = false;
          description = ''
            Run `power-profiles-daemon` behind the keymap's profile
            switch. Off drops the daemon and its bind entirely (the
            lid, low-battery and charge-limit policy still applies).
          '';
        };
      };

      # Which profile an AC transition selects (udev, so it covers boot
      # coldplug too: whichever matches the state at boot applies).
      # Null holds whatever is set (PPD boots to balanced and keeps the
      # last `set` across transitions -- PPD deliberately has no
      # auto-switch of its own: a profile is user intent, not power
      # state). Set both for the TLP-shaped behavior.
      profileOnAC = lib.mkOption {
        type = lib.types.nullOr (
          lib.types.enum [
            "performance"
            "balanced"
            "power-saver"
          ]
        );
        default = null;
        example = "performance";
        description = ''
          Profile to select when AC is connected (null keeps the
          current one). Needs `power.enable`.
        '';
      };

      # Which profile running on battery selects. Same shape as
      # `profileOnAC`; null keeps the current one.
      profileOnBattery = lib.mkOption {
        type = lib.types.nullOr (
          lib.types.enum [
            "performance"
            "balanced"
            "power-saver"
          ]
        );
        default = null;
        example = "power-saver";
        description = ''
          Profile to select when running on battery (null keeps the
          current one). Needs `power.enable`.
        '';
      };

      # What closing the lid does (logind's `HandleLidSwitch`). The
      # M2's proven value: suspend. Over SSH this suspends under you --
      # hold it off per session with
      # `systemd-inhibit --what=handle-lid-switch sleep 1d`, or set
      # `lock` (needs the idle policy's locker) or `ignore`.
      lidSwitch = lib.mkOption {
        type = logindAction;
        default = "suspend";
        example = "lock";
        description = ''
          What closing the lid does (logind's `HandleLidSwitch`).
          Suspends under SSH too unless inhibited -- see
          site/src/content/docs/desktop/index.md#power.
        '';
      };

      # What closing the lid does with a dock attached or a second
      # output connected (logind's `HandleLidSwitchDocked`, which fires
      # only then -- external power alone does not count). Lock, never
      # suspend: a closed lid on a multi-output box means the user
      # walked away to the external screen, not that the session should
      # die. The twin of the idle child's rule (same value, same path,
      # so the two merge); effective only while the idle policy's
      # locker listens for logind's Lock.
      lidSwitchDocked = lib.mkOption {
        type = logindAction;
        default = "lock";
        example = "ignore";
        description = ''
          What closing the lid does while docked or multi-output
          (logind's `HandleLidSwitchDocked`). Lock, never suspend: the
          session stays up behind the external screen.
        '';
      };

      # What closing the lid does on external power without a dock
      # (logind's `HandleLidSwitchExternalPower`). The M2's proven
      # value: suspend (a charger is not a screen).
      lidSwitchExternalPower = lib.mkOption {
        type = logindAction;
        default = "suspend";
        example = "lock";
        description = ''
          What closing the lid does on external power without a dock
          (logind's `HandleLidSwitchExternalPower`).
        '';
      };

      # What the power key does (logind's `HandlePowerKey`). The M2's
      # proven value: suspend (short press; long press is the
      # firmware's, not logind's).
      powerKey = lib.mkOption {
        type = logindAction;
        default = "suspend";
        example = "ignore";
        description = ''
          What the power key does (logind's `HandlePowerKey`).
        '';
      };

      # Low-battery suspend through UPower (which the M2 already runs:
      # its `macsmc-battery` is visible there with history and
      # statistics). No auto-suspend on idle timers -- the idle child
      # owns idle timing and deliberately suspends nothing (see its
      # reference: the box is reached over SSH) -- only this
      # battery-percentage trip, plus the lid and power-key paths
      # above. Hibernate is not wired: s2idle is the only sleep state
      # the reference hardware has, and its swap is zram (no
      # persistent image to hibernate into), so the UPower default
      # (`HybridSleep`) would fail there instead of sleeping -- the
      # default below suspends, which s2idle does.
      lowBattery = {
        # Battery percent that trips the action (UPower's
        # `PercentageAction`, against its `PercentageLow` 20 /
        # `PercentageCritical` 5, which stay at UPower's defaults:
        # anything above 5 breaks the descending order and UPower
        # silently falls back to its own triple, so the range ends
        # there).
        percentage = lib.mkOption {
          type = lib.types.int;
          default = 2;
          example = 5;
          description = ''
            Battery percent that trips `action` (UPower's
            `PercentageAction`). 0 to 5: above 5 UPower discards the
            whole triple for its defaults.
          '';
        };

        # What the trip does (UPower's `CriticalPowerAction`).
        # `Suspend` (the default) needs UPower's risky-action flag,
        # which the module sets beside it; `PowerOff` needs none.
        # `Hibernate` and `HybridSleep` are accepted for hardware with
        # persistent swap and a deeper sleep state, and unsupported on
        # the reference box (see above).
        action = lib.mkOption {
          type = lib.types.enum [
            "Suspend"
            "PowerOff"
            "Hibernate"
            "HybridSleep"
            "Ignore"
          ];
          default = "Suspend";
          example = "PowerOff";
          description = ''
            What low battery does (UPower's `CriticalPowerAction`).
            Suspend by default (s2idle); hibernate needs persistent
            swap the reference box has not got.
          '';
        };
      };

      # The desk-aware charge limit, ported from the M2's hand-wired
      # `charge.nix` (which this obsoletes when adopted): the battery
      # normally stops at `limit` percent (sitting at 100% on the
      # charger is what wears it most), charges to 100% once on demand
      # (`scoot-charge full-once`, until the next unplug), and refills
      # to 100% on its own after `fullAfter` seconds on battery (a
      # trip needs the range), dropping back after `tripEndsAfter`
      # seconds straight on the charger. Root re-syncs on every AC
      # change (udev) and every 5 minutes (a timer, which catches a
      # missed event); the bar button (`scoot-charge toggle` on the
      # push module's click) needs no password: the threshold file and
      # the state directory are group-writable, and every change is
      # pushed to the bar (which never polls).
      #
      # Where the hardware has no charge-control node the service is
      # inert, not refused: it logs one line and exits 0 (eval cannot
      # see the machine -- refusing there would break one shared
      # config across heterogeneous hardware). A set `limit` outside
      # 1..100, or a blank `battery`, fails evaluation.
      chargeLimit = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Cap the charge through the battery's
            `charge_control_end_threshold` node (the desk-aware policy
            above: limit, full-once, trip). On with `power.enable`
            (still individually disable-able); inert without the sysfs
            node.
          '';
        };

        # The percent the battery normally stops at (80: the M2's
        # measured default -- longevity over range, day to day).
        limit = lib.mkOption {
          type = lib.types.int;
          default = 80;
          example = 70;
          description = ''
            The percent the battery normally stops at. 1 to 100.
          '';
        };

        # Which battery to cap, as the kernel names it in
        # `/sys/class/power_supply` (`macsmc-battery` on Apple
        # silicon, `BAT0` on most laptops). Null takes the first
        # supply with a `charge_control_end_threshold` node, so one
        # config travels across machines.
        battery = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "BAT0";
          description = ''
            Which battery to cap (a `/sys/class/power_supply` name).
            Null auto-detects the first supply with a
            `charge_control_end_threshold` node.
          '';
        };

        # Seconds on battery before the next charge refills to 100%
        # (the trip: 30 min, the M2's value). 0 disables the trip
        # (full-once stays manual).
        fullAfter = lib.mkOption {
          type = lib.types.int;
          default = 1800;
          example = 3600;
          description = ''
            Seconds on battery before the next charge goes to 100%.
            0 disables the trip.
          '';
        };

        # Seconds straight on the charger before a trip drops back to
        # the limit (a day, the M2's value). 0 keeps the trip until
        # the next unplug.
        tripEndsAfter = lib.mkOption {
          type = lib.types.int;
          default = 86400;
          example = 43200;
          description = ''
            Seconds on the charger before a trip drops back to
            `limit`. 0 keeps it until unplug.
          '';
        };
      };
    };
    # GTK/Qt settings, dark-mode signal and a non-Stylix fallback.
    # Stylix stays the override path where present (as for `look`).
    # `targets` is the one per-target theme opt-out namespace
    # (Stylix-style): every themed piece gets
    # `theme.targets.<name>.enable` here (default on), so a user can
    # keep one piece's own style while the rest follows the look. The
    # theme-look child adds the rest; the locker's is first because it
    # lands here.
    theme =
      (slot {
        child = "desktop-theme-look";
        tool = "the non-Stylix GTK/Qt theme derivation";
      })
      // {
        targets.lock.enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Theme the locker from the look (screen and indicator colors
            from its palette). Set to `false` to keep swaylock's own
            style (`lock.settings` still applies).
          '';
        };
        targets.notifications.enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Theme mako from the look (popup background and text, ring
            and urgent leaves from its palette). Set to `false` to
            keep mako's own style (`notifications.settings` still
            applies).
          '';
        };
        targets.clipboard.enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Theme the history picker from the look (menu background
            and text, selection and border from its palette). Set to
            `false` to keep fuzzel's own style.
          '';
        };
        targets.launcher.enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Theme the launcher from the look (menu background and
            text, selection and border from its palette -- the same
            roles the history picker is themed from). Set to `false`
            to keep fuzzel's own style.
          '';
        };
        targets.capture.enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Theme the screencast chooser from the look (the dmenu
            list's background and text, selection and border, and the
            slurp picker's dim, border and selection -- the same
            roles the launcher and the history picker are themed
            from). Set to `false` to keep their own style.
          '';
        };
      };
    # The terminal the default binds spawn, and a file manager.
    # The manager is explicitly optional: nothing references one anywhere
    # today.
    apps = {
      terminal = slot {
        child = "desktop-apps";
        tool = "`foot` (already the default bind)";
      };
      fileManager = slot {
        child = "desktop-apps";
        tool = "a file manager (optional; nothing references one today)";
      };
    };
    # One keymap every other child registers into, each bind a `mkDefault`
    # so a user value wins. `enable` is the master switch (on with the
    # profile); `binds.<name>.enable` removes one bind at a time (a
    # user's own `[binds]` entry overrides one at a time -- plain
    # priority beats the keymap's `mkDefault`). The combos come from
    # `keymap` above, so this subtree cannot disagree with what
    # `keys-home.nix` renders.
    keys = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Render the shared keymap into `[binds]` (each bind a
          `mkDefault` a value you set in `settings.binds` wins
          over, one at a time). On with the profile. Binds gated on
          a future slot (launcher, clipboard, notifications,
          capture) appear only while that slot is enabled too;
          their keys stay reserved either way (see site/src/content/docs/desktop/index.md#hardware-keys-and-desktop-actions).
        '';
      };

      binds = lib.mapAttrs (name: spec: {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Bind ``${spec.combo}`` (${spec.blurb}). Set to `false`
            to leave that combo unbound.
          '';
        };
      }) keymap;
    };
    # Output policy (scale, placement) from the connected set.
    displays = configSlot {
      child = "desktop-displays";
      does = "output policy";
    };
    # Input-method wiring, off by default like everything here.
    inputMethod = configSlot {
      child = "desktop-input-method";
      does = "input-method wiring";
    };
    # Removable-media automount. In the paved-path inventory but with no
    # child filed yet; reserved here so the shell already covers it.
    automount = slot {
      child = "a future automount child (unfiled)";
      tool = "`udiskie`";
    };
  };

  # The palettes behind `look`, read from `docs/examples/*` (the
  # `scoot.toml` `[appearance]` and `bar.toml` `[colors]` each look ships).
  # `wallpaper` is null where the look has no image to ship
  # (`vinyl-sunset`: its illustration is license-barred from being
  # committed, and from being fetched for the user automatically, so the
  # session shows the flat `background_color` below
  # unless the user sets `settings.wallpaper` themselves -- see
  # site/src/content/docs/scootbg/index.md#the-wallpaper-section), else the in-repo file plus its mode. `image` is a path,
  # or `{ url, hash }` (a link fetched once with `pkgs.fetchurl` when the
  # profile applies it -- resolved in home.nix, since this module takes
  # only `lib`).
  looks = {
    vinyl-sunset = {
      appearance = {
        background_color = "#271A1F";
        focus_ring_active_color = "#E59560";
        focus_ring_inactive_color = "#423F51";
      };
      barColors = {
        background = "#271A1F";
        foreground = "#F1E3C6";
        accent = "#E59560";
        hover = "#FDC58B";
        dim = "#604F50";
        urgent = "#C76B47";
      };
      wallpaper = null;
    };
    music-desk = {
      appearance = {
        background_color = "#FCFBFB";
        focus_ring_active_color = "#3D579A";
        focus_ring_inactive_color = "#D5D7DD";
      };
      barColors = {
        background = "#FCFBFB";
        foreground = "#1A2032";
        accent = "#3D579A";
        hover = "#5D7AB0";
        dim = "#C9CBD0";
        urgent = "#EE6F5E";
      };
      wallpaper = {
        image = ../../docs/assets/wallpapers/music-desk.png;
        mode = "fill";
      };
    };
    radial-burst = {
      appearance = {
        background_color = "#241721";
        focus_ring_active_color = "#31a9e5";
        focus_ring_inactive_color = "#e36e38";
      };
      barColors = {
        background = "#241721";
        foreground = "#fdef1d";
        accent = "#31a9e5";
        dim = "#99911d";
        urgent = "#bf128d";
      };
      wallpaper = {
        image = ../../docs/assets/wallpapers/radial-burst.png;
        mode = "fill";
      };
    };
    moonrise = {
      appearance = {
        background_color = "#2B3648";
        focus_ring_active_color = "#FF9A49";
        focus_ring_inactive_color = "#5E4B5B";
      };
      barColors = {
        background = "#2B3648";
        foreground = "#F6EEDC";
        accent = "#FFA45C";
        hover = "#FFD54A";
        dim = "#9C8B95";
        urgent = "#E87F6A";
      };
      wallpaper = {
        image = ../../docs/assets/wallpapers/moonrise.png;
        mode = "fill";
      };
    };
  };
}
