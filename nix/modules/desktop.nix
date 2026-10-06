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
  # A slot that installs nothing, only config when its child lands
  # (output wiring, an input-method setup): a boolean alone.
  # (`keys` used to be one of these; it now owns the shared keymap
  # below, so it declares its own subtree instead. `slot` used to
  # sit beside it -- one boolean plus package override per future
  # piece -- until the last reserved piece (`desktop-apps`)
  # landed and every slot carried its own options.)
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
    network = {
      combo = "super+w";
      slot = "apps.network";
      blurb = "WiFi picker (`scoot-network-pick` through `fuzzel`)";
    };
    bluetooth = {
      combo = "super+b";
      slot = "apps.bluetooth";
      blurb = "Bluetooth picker (`scoot-bluetooth-pick` through `fuzzel`)";
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
    # Filled by the `desktop-auth-secrets` child: a polkit
    # authentication agent spawned in-scope by the session leader
    # (named through the login entry's `SCOOT_POLKIT_AGENT` -- a user
    # unit could never register, see `auth-home.nix`), so GUI
    # privilege prompts (disks, network, printers) work instead of
    # failing with no agent.
    # `daemon` names the agent (polkit-gnome by default: the smallest
    # closure of the maintained set, measured against lxqt-policykit
    # and hyprpolkitagent -- see site/src/content/docs/desktop/index.md#privilege-prompts-and-the-keyring);
    # a future scoot agent widens that enum without changing the
    # option (the native-replacement contract).
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (the authority still needs the
    # NixOS side's polkitd, the way the portal backends need that
    # side).
    auth = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Run the polkit authentication agent: the daemon behind
          `daemon` (its package beside this) owning
          `org.freedesktop.PolicyKit1.AuthenticationAgent` on the
          session bus. Without it privilege prompts fail with
          polkit's own no-agent refusal.
        '';
      };

      daemon = lib.mkOption {
        type = lib.types.enum [
          "gnome"
          "lxqt"
          "hyprpolkit"
        ];
        default = "gnome";
        example = "lxqt";
        description = ''
          The agent behind `enable`. `gnome` (polkit-gnome: the
          smallest closure of the maintained set), `lxqt`
          (lxqt-policykit-agent) or `hyprpolkit` (hyprpolkitagent);
          a future scoot agent widens this enum, the option staying
          as it is.
        '';
      };
    };
    # The keyring.
    # Filled by the `desktop-auth-secrets` child: gnome-keyring with
    # the `secrets` component, D-Bus activated (no unit: the daemon
    # starts on the first `org.freedesktop.secrets` call and holds
    # the unlocked keyring until the session ends), auto-unlocked
    # from the login password on greetd logins (a PAM pair confined
    # to greetd's own service -- see `nixos.nix`).
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (the D-Bus activation files need
    # the NixOS side, the way the portal backends do). First use per
    # login unlocks once; the rest is seamless.
    secrets = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Provide the secrets service: gnome-keyring (its package
          beside this) owning `org.freedesktop.secrets` on the
          session bus, D-Bus activated on first use. Without it
          every app re-prompts for secrets.
        '';
      };

      daemon = lib.mkOption {
        type = lib.types.enum [ "gnome-keyring" ];
        default = "gnome-keyring";
        description = ''
          The daemon behind `enable`. Only gnome-keyring today (the
          only candidate with a login-password unlock path -- see
          site/src/content/docs/desktop/index.md#privilege-prompts-and-the-keyring
          for the measured pick); a future scoot keyring widens this
          enum, the option staying as it is.
        '';
      };
    };
    # PipeWire baseline, media keys, brightness keys and an OSD.
    # Filled by the `desktop-audio-osd` child: PipeWire with
    # WirePlumber running for the keymap's volume binds (`wpctl` is
    # WirePlumber's own CLI, so a PipeWire-only shape would leave the
    # binds with nothing to call), the keymap's volume, brightness
    # and mic-mute binds routed through small scripts that also poke
    # the OSD, a sink helper (`list`, `set`, `cycle`) the future
    # Bluetooth picker in `desktop-apps` calls, and the OSD itself:
    # wob on the `overlay` layer (so it shows above fullscreen
    # windows), themed by the look.
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (unthemed: a look needs the
    # profile, and PipeWire itself needs the NixOS side, the way the
    # portal backends do).
    audio = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Run the audio baseline: PipeWire with WirePlumber (their
          packages beside this), the keymap's volume, brightness and
          mic-mute binds through the OSD scripts, and the OSD on the
          `overlay` layer. Without it nothing answers the volume
          binds' `wpctl` calls and the hardware binds run silent.
        '';
      };

      daemon = lib.mkOption {
        type = lib.types.enum [ "wob" ];
        default = "wob";
        description = ''
          The program behind the OSD. Only wob today (a single-purpose
          overlay bar with no toolkit -- see site/src/content/docs/desktop/index.md#sound-brightness-keys-and-the-on-screen-display
          for the measured pick); a future scoot OSD widens this
          enum, the option and the binds staying as they are.
        '';
      };

      osd = {
        # Milliseconds the OSD stays mapped after the last key press
        # (wob's `timeout`: it destroys its surface on expiry, so a
        # hidden OSD holds no surface and wakes nothing). At least 0
        # (0 hides at once: useful only to prove the bind fires).
        timeoutMs = lib.mkOption {
          type = lib.types.int;
          default = 1500;
          example = 2500;
          description = ''
            Milliseconds the OSD stays visible after the last step.
            At least 0.
          '';
        };
      };
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
    # Night light over `wlr-gamma-control-v1`. Filled by the
    # `desktop-nightlight` child: `wlsunset` as a user unit bound to
    # `scoot-session.target` (single purpose, tiny -- see
    # site/src/content/docs/desktop/index.md#night-light for the measured
    # pick), manual sunrise/sunset by default (no location, no geoclue,
    # no network), `gammastep` for location-based sunrise/sunset.
    # Each `package` lives beside this in the side modules (`nixos.nix`
    # installs system-wide, `nightlight-home.nix` for the user, each
    # defaulting to the daemon's own tool), which is also where its
    # default lives; everything here is plain values, so this file
    # stays `lib`-only.
    #
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (unthemed: a look needs the
    # profile, and the user unit needs the home-manager side, the way
    # the themed picker does).
    nightlight = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Warm the screen at night: the daemon behind `daemon`
          (its package beside this) driving every output's gamma ramp
          through `wlr-gamma-control-v1`, one control per output (the
          compositor retires a control with `failed` on CRTC moves, so
          the daemon re-reads `gamma_size` and re-pushes -- see
          site/src/content/docs/desktop/index.md#night-light). Without
          it the ramp stays linear day and night.
        '';
      };

      # `wlsunset` warms on a fixed schedule with no location (neither
      # geoclue nor network: `-S`/`-s`), `gammastep` on sunrise/sunset
      # computed from `latitude`/`longitude` (which it then requires:
      # without either it would reach for geoclue, which is not wired
      # -- see site/src/content/docs/desktop/index.md#night-light). A
      # future scoot-native widens this enum, the option and the unit
      # staying as they are (the native-replacement contract).
      daemon = lib.mkOption {
        type = lib.types.enum [
          "wlsunset"
          "gammastep"
        ];
        default = "wlsunset";
        example = "gammastep";
        description = ''
          The program behind `enable`. `wlsunset` (single purpose: one
          small binary warming on a fixed schedule) or `gammastep`
          (sunrise/sunset from `latitude`/`longitude`).
        '';
      };

      # Day color temperature in Kelvin (the neutral point: 6500 changes
      # nothing, higher is bluer). 1000 to 10000, either daemon's range.
      dayTemp = lib.mkOption {
        type = lib.types.int;
        default = 6500;
        example = 6000;
        description = ''
          Day color temperature in Kelvin. 1000 to 10000.
        '';
      };

      # Night color temperature in Kelvin (lower is warmer). 1000 to
      # 10000, at or under `dayTemp` (above it fails evaluation: a
      # night bluer than the day is a typo). Without a look (or opted
      # out below) the default below; with a look each look warms to
      # its own default (its palette's warmth: the espresso look
      # warmest, the paper look brightest), a value here winning per
      # key the way `settings` wins over a look.
      nightTemp = lib.mkOption {
        type = lib.types.int;
        default = 3500;
        example = 3000;
        description = ''
          Night color temperature in Kelvin (lower is warmer). 1000 to
          10000, at or under `dayTemp`.
        '';
      };

      # Manual sunrise/sunset as 24-hour `HH:MM` (read only while no
      # location is set: a set `latitude`/`longitude` pair switches the
      # daemon to location mode instead). `07:00`/`19:00`: a fixed
      # schedule that works with neither geoclue nor network.
      sunrise = lib.mkOption {
        type = lib.types.str;
        default = "07:00";
        example = "06:30";
        description = ''
          Manual sunrise as 24-hour `HH:MM` (when the schedule warms
          back up). Read only without `latitude`/`longitude`.
        '';
      };

      sunset = lib.mkOption {
        type = lib.types.str;
        default = "19:00";
        example = "21:30";
        description = ''
          Manual sunset as 24-hour `HH:MM` (when the schedule warms
          down). Read only without `latitude`/`longitude`.
        '';
      };

      # Where on earth the session is (decimal degrees). Null (the
      # default) keeps the manual schedule above; set both for
      # location mode (sunrise/sunset computed, the manual pair unread
      # -- and `-d` unread with it: wlsunset's duration applies to
      # manual times only). One without the other fails evaluation.
      # `gammastep` needs both (geoclue is not wired, so nothing else
      # can locate it).
      latitude = lib.mkOption {
        type = lib.types.nullOr lib.types.number;
        default = null;
        example = 37.33;
        description = ''
          Latitude in decimal degrees (-90 to 90). Null keeps the
          manual schedule; set beside `longitude` for location mode.
        '';
      };

      longitude = lib.mkOption {
        type = lib.types.nullOr lib.types.number;
        default = null;
        example = -121.89;
        description = ''
          Longitude in decimal degrees (-180 to 180). Null keeps the
          manual schedule; set beside `latitude` for location mode.
        '';
      };

      # Seconds the warm-up/warm-down takes (manual schedule only:
      # location mode follows the sun, not this). 900 (15 min: gentle,
      # over long before either boundary matters); 0 snaps, 7200 (2 h)
      # is the most gradual. Outside 0..7200 fails evaluation.
      duration = lib.mkOption {
        type = lib.types.int;
        default = 900;
        example = 1800;
        description = ''
          Seconds the day/night transition takes (manual schedule
          only). 0 snaps, at most 7200.
        '';
      };

      # Extra gamma multiplier (1.0 is neutral). 0.1 to 10, either
      # daemon's range.
      gamma = lib.mkOption {
        type = lib.types.number;
        default = 1.0;
        example = 0.8;
        description = ''
          Gamma multiplier (1.0 is neutral). 0.1 to 10.
        '';
      };
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
    # Stylix stays the override path where present (as for `look`):
    # every value below is a default a value you set wins over per
    # key, and Stylix wins where present (user > Stylix > look).
    # `targets` is the one per-target theme opt-out namespace
    # (Stylix-style): every themed piece gets
    # `theme.targets.<name>.enable` here (default on), so a user can
    # keep one piece's own style while the rest follows the look.
    # Filled by the `desktop-theme-look` child; without a look behind
    # the profile the slot is inert (there is nothing to derive
    # from), the way every other standalone slot runs unthemed.
    theme = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Derive the app theme (fonts, cursor, GTK/Qt settings, the
          dark-mode signal, the look's app files) from `desktop.look`.
          On with the profile (still individually disable-able);
          without a look it is inert.
        '';
      };
      # Extra GTK `settings.ini` keys, merged over the generated
      # ones (a value here wins per key). Rendered verbatim.
      settings = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = { };
        example = {
          gtk-xft-hintstyle = "hintslight";
        };
        description = ''
          Extra GTK `settings.ini` keys, merged over the look-derived
          ones (a value here wins per key). Rendered verbatim.
        '';
      };
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
      targets.osd.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Theme the on-screen display from the look (bar
          background and text, fill and border from its palette,
          a washed style while muted and an urgent fill past
          100%). Set to `false` to keep wob's own style.
        '';
      };
      targets.nightlight.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Warm to the look's own night temperature (each look's
          `nightTemp`: the espresso look warmest, the paper look
          brightest). Set to `false` to keep the plain `nightTemp`
          default (or a value you set) while the rest follows the
          look.
        '';
      };
      targets.gtk.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Theme GTK apps from the look (Adwaita or Adwaita-dark by
          its polarity, the look's UI font and cursor, the
          dark-mode preference). Set to `false` to keep GTK's own
          style (`theme.settings` still applies).
        '';
      };
      targets.qt.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Theme Qt apps from the look (the Adwaita Qt style in the
          look's polarity, the Adwaita icon theme, through qt6ct).
          Set to `false` to keep Qt's own style.
        '';
      };
      targets.cursor.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Theme the cursor from the look (Vanilla-DMZ, 24 px: the
          compositor cursor plus `XCURSOR_THEME`/`XCURSOR_SIZE`
          for X11 apps). Set to `false` to keep the cursor alone
          (a value you set in `settings.appearance` still wins).
        '';
      };
      targets.fonts.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Install the look's fonts and default them through
          fontconfig (UI face for sans-serif, terminal face for
          monospace). Set to `false` to keep your own fonts (the
          bar then falls back to its DejaVu default font).
        '';
      };
      targets.greeter.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Theme the login screen from the look (backdrop pairing,
          dark/light GTK setting, the look's CSS and font). Set to
          `false` to keep ReGreet's own style.
        '';
      };
      targets.terminal.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Apply the look's foot config (`foot.ini`) from the flake.
          Set to `false` to keep your own (enabling home-manager's
          `programs.foot` skips it too: that module owns the same
          file).
        '';
      };
      targets.network.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Theme the WiFi picker from the look (menu background
          and text, selection and border from its palette -- the same
          roles the launcher is themed from). Set to `false` to keep
          fuzzel's own style.
        '';
      };
      targets.bluetooth.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Theme the Bluetooth picker from the look (menu background
          and text, selection and border from its palette -- the same
          roles the launcher is themed from). Set to `false` to keep
          fuzzel's own style.
        '';
      };
      targets.shell.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Apply the look's starship prompt from the flake. Set to
          `false` to keep your own (enabling home-manager's
          `programs.starship` skips it too: that module owns the
          same file).
        '';
      };
      targets.editor.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Apply the look's Helix config and theme from the flake.
          Set to `false` to keep your own (enabling
          home-manager's `programs.helix` skips them too: that
          module owns the same files).
        '';
      };
      targets.monitor.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Apply the look's btop config and theme from the flake.
          Set to `false` to keep your own (enabling
          home-manager's `programs.btop` skips them too: that
          module owns the same file).
        '';
      };
    };
    # The terminal the default binds spawn, a file manager, and the
    # network/Bluetooth pickers. The terminal is on with the profile
    # (the compositor's built-in `Super+Return` spawns `foot`, so the
    # profile installs what the bind names); the pickers are on with
    # the profile too (keyboard-driven join/switch through the
    # launcher's dmenu contract, completing what the bar's `network`
    # and `bluetooth` modules display); the manager stays optional
    # (nothing references one, and a graphical one costs a toolkit).
    # Filled by the `desktop-apps` child. Each `package` lives beside
    # this in the side modules (`apps-home.nix` installs for the
    # user, `nixos.nix` system-wide), which is also where their
    # defaults live; everything here is plain values, so this file
    # stays `lib`-only.
    #
    # On with the profile (each still individually disable-able);
    # without it, each `enable` works standalone (unthemed: a look
    # needs the profile, and the menus need the home-manager side,
    # the way the clipboard picker does).
    apps = {
      # `foot` installed, the look's `foot.ini` applied through the
      # theme child's `terminal` target, `TERMINAL` set for
      # everything that asks, and the `Super+Return` bind true.
      terminal = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Install the terminal: `foot` (its package beside this),
            keeping the compositor's built-in `Super+Return` bind
            true, with `TERMINAL` set beside it. Without it the bind
            names a binary that is not installed.
          '';
        };
      };
      # A graphical file manager, explicitly optional: `xdg-open`
      # answers directories through it while it is on (the directory
      # association), and fails loud -- never wedged on a missing
      # file -- without it. `xdg.userDirs` keeps Downloads, Pictures
      # (where the capture slot writes) and the rest present while
      # anything here opens files.
      fileManager = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Install the file manager (its package beside this:
            pcmanfm, the smallest closure of the maintained
            graphical set -- see site/src/content/docs/desktop/index.md#terminal-files-and-removable-media
            for the measured pick). Off: nothing references one.
          '';
        };
      };
      # WiFi through `nmcli`, picked through the launcher's dmenu
      # contract (a saved connection switches at once; a new secured
      # one reads its psk from the keyring's `scoot-wifi` entries, or
      # says the one terminal command that joins it). On with the
      # profile (still individually disable-able); without it,
      # `enable` works standalone (unthemed: a look needs the
      # profile, and the menu needs the home-manager side, the way
      # the clipboard picker does). Never touches the networking
      # service itself (no takeover: it only calls the CLI).
      network = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Pick WiFi from the keyboard: the picker behind
            `scoot-network-pick` (its packages beside this) listing
            saved connections first, then the cached scan, joining
            through `nmcli`. Without it the keymap's WiFi bind stays
            unbound.
          '';
        };
      };
      # Bluetooth through `bluetoothctl`, picked the same way (paired
      # devices connect/disconnect, power toggles, the audio sink
      # switch delegating to the audio slot's helper). On with the
      # profile (still individually disable-able); without it,
      # `enable` works standalone (same unthemed shape as above).
      # Never touches the Bluetooth service itself.
      bluetooth = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Pick Bluetooth from the keyboard: the picker behind
            `scoot-bluetooth-pick` (its packages beside this)
            toggling paired devices and power, with an audio-sink
            row through the audio slot's helper. Without it the
            keymap's Bluetooth bind stays unbound.
          '';
        };
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
          capture, the apps pickers) appear only while that slot is
          enabled too;
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
    # Output policy (scale and power) from the connected set.
    # Filled by the `desktop-displays` child: scoot-native arrangement
    # profiles (kanshi-class matching, without kanshi: stock kanshi gates
    # its `exec` hooks on the output-management `succeeded` reply, which
    # scoot never sends -- its write half answers every configuration
    # with `failed` -- so kanshi matches and then fails every profile;
    # making the protocol writable would be atomic-modeset surgery across
    # three backends for a packaging ticket -- its durable home is
    # `docs/backlog/resolved/output-management-reconfiguration-done.md`,
    # and this watcher is the stopgap until then -- while subscribe/
    # outputs/output-scale/output-power already give matching plus
    # applying). Each profile names the exact connected set it answers
    # (connector names, the same key `[[outputs]]` matches on -- make
    # and model are not visible over scoot's IPC, so they cannot key a
    # profile), with a scale and optional power-off per output. No
    # positions (outputs pack left to right in connection order) and no
    # modes (a mode cannot change live; `settings.outputs` sets one
    # statically). The watcher (`displays-home.nix`) re-matches on every
    # output event (an add included, so a first plug is heard) and on
    # the idle policy's resume, and applies over IPC only
    # (`output-scale`, `output-power`, both live runtime state), never
    # writing the config file; the only offs it ever undoes are its
    # own. Each `package`
    # lives beside this in the side modules, which is also where its
    # default lives; everything here is plain values, so this file
    # stays `lib`-only.
    #
    # On with the profile (still individually disable-able at plain
    # priority, the way the clipboard slot works); without it, `enable`
    # works standalone (unthemed: there is nothing the look themes
    # here).
    displays = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Match the connected output set and apply the profile's scale
          and power: the watcher behind `profiles` re-matches on every
          output event (each plug and unplug), once at session start,
          and on the idle policy's resume, and applies live over IPC
          (`scoot msg output-scale`, `scoot msg output-power`), never
          writing the config file. Without it the outputs keep
          whatever `settings` (or the compositor defaults) say.
        '';
      };

      # Arrangement profiles, first match wins, in list order. A profile
      # matches when the connected connector-name set is exactly its
      # `outputs` (no subset rule: "this monitor set means this layout",
      # so a typo'd extra monitor falls through instead of half
      # applying). No match resets every connected output's scale to
      # the static config's (`scoot msg output-scale NAME reset`) and
      # powers back on only what the watcher itself powered off.
      profiles = lib.mkOption {
        type = lib.types.listOf (
          lib.types.submodule {
            options = {
              # The profile's name, as `scoot-displays status` prints it
              # and the watcher logs it. Unique across `profiles`.
              name = lib.mkOption {
                type = lib.types.str;
                example = "docked";
                description = ''
                  The profile's name (unique across `profiles`).
                '';
              };

              # The exact connected set this profile answers, as
              # connector names (`scoot msg outputs` lists them:
              # `eDP-1`, `DP-1` on real hardware, `headless-N`
              # headless). Non-empty, and unique across `profiles`
              # (two profiles for one set would never reach the
              # second).
              outputs = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                example = [
                  "eDP-1"
                  "DP-1"
                ];
                description = ''
                  The exact connected connector-name set this profile
                  answers. Non-empty, and unique across `profiles`.
                '';
              };

              # The scale each named output runs at under this profile
              # (the `[[outputs]]` entry's `scale`, 0.5 to 4.0 like the
              # compositor's own range). Applied live through
              # `scoot msg output-scale`. Every key names an output in
              # `outputs` (a scale for an output this profile never
              # matches is a typo, refused here). An output with no
              # entry runs at the static config's scale (its live scale
              # is reset, so another profile's never lingers).
              scale = lib.mkOption {
                type = lib.types.attrsOf lib.types.number;
                default = { };
                example = {
                  "DP-1" = 1.0;
                  "eDP-1" = 2.0;
                };
                description = ''
                  The scale each named output runs at (0.5 to 4.0),
                  applied live. An output in `outputs` with no entry
                  runs at the static config's scale. Every key names an
                  output in `outputs`.
                '';
              };

              # Not a profile field: kept only so a set `mode` fails
              # evaluation with the way out (`settings.outputs`), not
              # as an unknown option -- see `displays-home.nix`.
              mode = lib.mkOption {
                type = lib.types.attrsOf lib.types.str;
                default = { };
                visible = false;
                description = ''
                  Not supported: a mode cannot change live. Set a
                  static mode in `programs.scoot.settings.outputs`.
                '';
              };

              # Outputs to power off under this profile (through
              # `scoot msg output-power`, live). Empty disables
              # nothing: no output ever powers off unless named here.
              # Every entry names an output in `outputs`, and at least
              # one output of `outputs` stays out of it (a profile
              # that darkens its whole set is refused). The watcher
              # records each off it makes and powers it back on once
              # the matched profile stops disabling it, or nothing
              # matches (the clamshell undock); an output something
              # else turned off (the idle policy) is never turned on.
              disabled = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                default = [ ];
                example = [ "eDP-1" ];
                description = ''
                  Outputs to power off under this profile (live), and
                  back on once it no longer applies. Empty disables
                  nothing. Every entry names an output in `outputs`,
                  and at least one output of `outputs` stays out of it.
                '';
              };
            };
          }
        );
        default = [ ];
        example = [
          {
            name = "docked";
            outputs = [
              "eDP-1"
              "DP-1"
            ];
            scale = {
              "DP-1" = 1.0;
              "eDP-1" = 2.0;
            };
            disabled = [ ];
          }
          {
            name = "undocked";
            outputs = [ "eDP-1" ];
            scale = {
              "eDP-1" = 2.0;
            };
          }
        ];
        description = ''
          Arrangement profiles, first match wins in list order. Empty
          matches nothing (the watcher sets no scale and turns nothing
          off, and only turns back on an output it turned off itself under
          an earlier list). Profiles set scale and power only: no
          positions, no modes, no make/model matching.
        '';
      };
    };
    # Input-method wiring, off by default like everything here.
    inputMethod = configSlot {
      child = "desktop-input-method";
      does = "input-method wiring";
    };
    # Removable-media automount over udisks2. Filled by the
    # `desktop-apps` child: `udiskie` trayless (`-a -n -T`, browsing
    # through `xdg-open` so the file manager association answers) as
    # a user unit, with `udiskie-umount` for safe removal. The
    # `package` lives beside this in the side modules
    # (`apps-home.nix` installs for the user, `nixos.nix`
    # system-wide with the udisks2 daemon it mounts through), which
    # is also where its default lives; everything here is a plain
    # value, so this file stays `lib`-only.
    # On with the profile (still individually disable-able); without
    # it, `enable` works standalone (the daemon needs the NixOS
    # side's udisks2, the way the portal backends need that side).
    automount = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Automount removable media on insert: trayless `udiskie`
          (its package beside this) over the udisks2 daemon, with a
          notification carrying a Browse action. Without it nothing
          mounts on insert.
        '';
      };
    };
  };

  # The palettes behind `look`, read from `docs/examples/*` (the
  # `scoot.toml` `[appearance]` and `bar.toml` `[colors]` each look ships).
  # `nightTemp` is each look's own night warmth for the nightlight slot
  # (the espresso look warmest, the paper look brightest -- a value the
  # user sets in `nightlight.nightTemp` wins per key).
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
      nightTemp = 3200;
      # Dark or light: what the GTK/Qt theme, the dark-mode signal
      # and the greeter's dark setting follow.
      isDark = true;
      # The three faces the theme carries everywhere text is drawn:
      # `ui` is the bar's face (Droid Sans Mono Nerd Font Propo: the
      # module icons below need its Nerd glyphs, so the bar keeps this
      # mono face); `sans` is the proportional UI sans for GTK/Qt
      # apps, the greeter and fontconfig sans-serif (DejaVu Sans: the
      # only proportional face the profile already ships -- the bar's
      # own fallback file -- so it adds zero closure; its humanist
      # warmth pairs with this look's espresso-and-orange vinyl dusk,
      # and its open apertures stay legible on dark surfaces); `mono`
      # is the terminal face (foot, fontconfig monospace). Names,
      # resolved through fontconfig; the packages beside them live in
      # the side modules (which own `pkgs`), so this file stays
      # `lib`-only.
      fonts = {
        ui = "DroidSansM Nerd Font Propo";
        sans = "DejaVu Sans";
        mono = "FiraCode Nerd Font";
      };
      # The example's own app files, applied from the flake instead of
      # copy-paste (see `theme-home.nix`): foot, starship, Helix and
      # btop stay static files -- their palettes are hand-tuned per
      # look, not derivable from the six bar roles without inventing
      # colors -- while GTK/Qt/cursor/greeter settings generate from
      # the roles above. Null where the example ships none
      # (`radial-burst` has no shell, editor or monitor files): the
      # target is then inert for that look.
      appFiles = {
        foot = ../../docs/examples/vinyl-sunset/foot.ini;
        starship = ../../docs/examples/vinyl-sunset/starship.toml;
        helixConfig = ../../docs/examples/vinyl-sunset/helix/config.toml;
        helixTheme = ../../docs/examples/vinyl-sunset/helix/themes/scoot-vinyl.toml;
        btopConf = ../../docs/examples/vinyl-sunset/btop/btop.conf;
        btopTheme = ../../docs/examples/vinyl-sunset/btop/themes/vinyl.theme;
        regreetCss = ../../docs/examples/vinyl-sunset/regreet.css;
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
      nightTemp = 4000;
      isDark = false;
      # `sans` is DejaVu Sans here too: bookish and paper-neutral, so
      # the ink-blue accent carries the look while the type stays
      # quiet -- and already shipped, so zero closure (see the
      # vinyl-sunset note above for the `ui`/`sans`/`mono` split).
      fonts = {
        ui = "DroidSansM Nerd Font Propo";
        sans = "DejaVu Sans";
        mono = "FiraCode Nerd Font";
      };
      appFiles = {
        foot = ../../docs/examples/music-desk/foot.ini;
        starship = ../../docs/examples/music-desk/starship.toml;
        helixConfig = ../../docs/examples/music-desk/helix/config.toml;
        helixTheme = ../../docs/examples/music-desk/helix/themes/scoot-light.toml;
        btopConf = ../../docs/examples/music-desk/btop.conf;
        btopTheme = null;
        regreetCss = ../../docs/examples/music-desk/regreet.css;
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
      nightTemp = 3500;
      isDark = true;
      # `sans` is DejaVu Sans: the sturdiest of the shipped faces at
      # poster sizes, holding against this look's plum-and-yellow
      # burst -- and this look already ships `dejavu_fonts` as its
      # `mono`, so the sans adds nothing (see the vinyl-sunset note
      # above for the `ui`/`sans`/`mono` split).
      fonts = {
        ui = "DroidSansM Nerd Font Propo";
        sans = "DejaVu Sans";
        mono = "DejaVu Sans Mono";
      };
      appFiles = {
        foot = ../../docs/examples/radial-burst/foot.ini;
        starship = null;
        helixConfig = null;
        helixTheme = null;
        btopConf = null;
        btopTheme = null;
        regreetCss = ../../docs/examples/radial-burst/regreet.css;
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
      nightTemp = 3400;
      isDark = true;
      # `sans` is DejaVu Sans: open apertures stay legible on this
      # look's translucent slate, while the cream text and amber disc
      # carry the night mood -- already shipped, so zero closure (see
      # the vinyl-sunset note above for the `ui`/`sans`/`mono` split).
      fonts = {
        ui = "DroidSansM Nerd Font Propo";
        sans = "DejaVu Sans";
        mono = "FiraCode Nerd Font";
      };
      appFiles = {
        foot = ../../docs/examples/moonrise/foot.ini;
        starship = ../../docs/examples/moonrise/starship.toml;
        helixConfig = ../../docs/examples/moonrise/helix/config.toml;
        helixTheme = ../../docs/examples/moonrise/helix/themes/scoot-moonrise.toml;
        btopConf = ../../docs/examples/moonrise/btop/btop.conf;
        btopTheme = ../../docs/examples/moonrise/btop/themes/moonrise.theme;
        regreetCss = ../../docs/examples/moonrise/regreet.css;
      };
      wallpaper = {
        image = ../../docs/assets/wallpapers/moonrise.png;
        mode = "fill";
      };
    };
  };
}
