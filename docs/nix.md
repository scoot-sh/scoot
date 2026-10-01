# Using scoot from Nix: flake consumption, home-manager, NixOS

- [Consuming the flake](#consuming-the-flake)
- [Platform notes](#platform-notes)
- [GPU tiers from the flake](#gpu-tiers-from-the-flake)
- [XWayland from the flake](#xwayland-from-the-flake)
- [Home-manager module](#home-manager-module)
- [NixOS module](#nixos-module)
- [The wallpaper: scootbg](#the-wallpaper-scootbg)
- [The status bar: scootbar](#the-status-bar-scootbar)
- [The overlay](#the-overlay)
- [Migrating from a hand-rolled packaging](#migrating-from-a-hand-rolled-packaging)
- [Settings failure modes](#settings-failure-modes)
- [Reference: live defaults](#reference-live-defaults)

## Consuming the flake

In your own flake:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/<your-rev>";
    scoot.url = "github:scoot-sh/scoot";
  };
}
```

Then, in a module:

```nix
# The binary, anywhere a package list goes:
environment.systemPackages = [ inputs.scoot.packages.${pkgs.system}.scoot ];
# ... or build-and-run in one step, without installing:
# nix run github:scoot-sh/scoot -- --headless -- foot
```

`packages.<system>.scoot` is the compositor alone (`$out/bin` carries only
the `scoot` binary, with the `scoot msg` client alias kept on it),
`packages.<system>.scoot-gpu` the same binary with the `gpu-scanout` build
feature (see [GPU tiers](#gpu-tiers-from-the-flake) below),
`packages.<system>.scoot-xwayland` and `scoot-gpu-xwayland` those two with
the `xwayland` build feature and the X server on `PATH` (Linux only; see
[XWayland](#xwayland-from-the-flake) below),
`packages.<system>.scootctl` the standalone
remote-control client, `packages.<system>.scootbg` the wallpaper daemon
(Linux only; see [The wallpaper](#the-wallpaper-scootbg)),
`packages.<system>.scootbar` the status bar and `scootbar-demo` the bar
with a font (Linux only; see [The status bar](#the-status-bar-scootbar)),
and `packages.<system>.default` is whichever is
honest on that system (see [Platform notes](#platform-notes)). `apps`
mirrors six of them (`nix run . -- ...`, `nix run .#scootctl -- ...`,
`nix run .#scoot-gpu -- --tty -- ...`,
`nix run .#scoot-xwayland -- --headless --xwayland -- xterm`,
`nix run .#scootbar -- daemon --font F`, `nix run .#scootbar-demo`). The
same packages are also `pkgs.scoot`, `pkgs.scootctl`, `pkgs.scootbg` and
`pkgs.scootbar` through [the overlay](#the-overlay).

## Platform notes

The flake covers **Apple Silicon and Linux** (`aarch64-linux`,
`x86_64-linux`, `aarch64-darwin`). On **macOS** the build gives the
`scootctl` remote-control client only — enough to drive a compositor
running in a VM; there is no macOS compositor (see README's "No macOS
adapter"). An Intel Mac is outside what the flake provides but nothing
Intel-specific blocks the client: `cargo build -p scootctl` from source
works there.

The modules below follow the same split: the home-manager module
manages the config file on any system (useful on macOS too, to keep the
config you deploy to a Linux box next to the machine that edits it),
while the NixOS module's session entry only means anything on NixOS.
scootbg, the wallpaper daemon, is Linux-only: there is no macOS
`scootbg` package, and on macOS a `[wallpaper]` section in the
home-manager settings renders as written and installs nothing. scootbar,
the status bar, is Linux-only the same way: no macOS package.

## GPU tiers from the flake

Two packages, matching the two GPU tiers in
[tty.md](tty.md#which-renderer-draws-the-frames):

- `packages.<system>.scoot` (the default on Linux) composites on the CPU
  with pixman and needs no GPU at all. `--renderer gles` is available from
  this package -- the offscreen tier, still read back to main memory -- but
  it needs EGL drivers from the **host OS**, not from the derivation: the
  package ships libglvnd (the dispatch library Smithay `dlopen`s) while
  Mesa's vendor ICDs come from the system (on NixOS, `hardware.graphics`
  enabled). That split is the standard nixpkgs pattern, and it is
  deliberate: bundling Mesa would risk shadowing the host's drivers --
  notably Asahi's -- with wrong ones. On a box with no working EGL,
  `--renderer gles` is a loud startup error naming the pixman default, not
  a panic and not a silent downgrade (previously it panicked in Smithay's
  `dlopen`; gh #177).
- `packages.<system>.scoot-gpu` adds the `gpu-scanout` Cargo feature (the
  one that links libgbm): under `--tty --renderer gles` the frame is
  composited straight into the buffer the CRTC scans out, with no
  read-back; under `--nested --renderer gles` it is handed to the host
  compositor as a GPU buffer instead of being read back, where the host
  composites on the same GPU (see
  [tty.md](tty.md#which-renderer-draws-the-frames)). Same binary name (`scoot`), the
  same EGL-driver requirement as above, plus a real DRM seat. Scanout
  drives the primary plane only -- no overlay or cursor planes -- but it is
  measured rather than merely reasoned now: on an Apple M2 under Asahi Linux
  it costs 4--5x less compositor CPU under damage than the default tier, for
  7--16 MB more RSS (see [Asahi.md](../Asahi.md)'s Test 4, which settled
  that on 2026-09-21).

## XWayland from the flake

X11 applications need two things the packages above do not carry: the
`xwayland` Cargo feature, and the `Xwayland` binary on the compositor's
`PATH` (the server is started as a bare `Xwayland`, a `PATH` lookup and
nothing else). Two Linux-only packages carry both:

- `packages.<system>.scoot-xwayland` -- `scoot` with the `xwayland`
  feature;
- `packages.<system>.scoot-gpu-xwayland` -- `scoot-gpu` with it.

Each wraps `bin/scoot` (a small compiled wrapper, so `scoot` is still an
ELF binary and `scoot msg` pays no shell start) to *append* nixpkgs'
Xwayland to `PATH`. Appended, so an `Xwayland` already on the session's
`PATH` -- NixOS's `programs.xwayland.enable` puts one there -- is the one
started. Every program the session starts inherits that `PATH`; the
appended directory holds `Xwayland` and nothing else. The package's
closure grows by Xwayland's own (it links Mesa for its GL acceleration;
344 MiB in all on x86_64-linux, all but 11.8 MiB of it Xwayland's closure),
which is why it is its own package and never the default: the default
`scoot` stays the GPU-free, X-free build.

One visible side effect of the wrapper: `bin/scoot` execs the real binary
as `bin/.scoot-wrapped`, so the running compositor's process name (`comm`,
what `pgrep -x`, `top` and `ps -o comm` show) is `.scoot-wrapped`,
not `scoot`. `pgrep -x scoot` finds nothing under these two packages; use
`pgrep -x .scoot-wrapped` there. (`pidof scoot` still finds it: procps
`pidof` also matches `argv[0]`'s basename, which the wrapper keeps as
`scoot`.) `scripts/niri-ab/sample.sh session`
accepts both names.

The package is only half of it: XWayland still starts only when asked, by
`--xwayland` or `[xwayland] enabled` (`programs.scoot.settings.xwayland.enabled
= true` in the home-manager module). The NixOS module needs nothing extra
-- point `package` at the XWayland build and its session entry's `Exec=`
starts the wrapper:

```nix
programs.scoot = {
  enable = true;
  package = inputs.scoot.packages.${pkgs.system}.scoot-xwayland;
  session.enable = true;
};
```

Either half alone is loud rather than silent: `[xwayland] enabled` with the
default `scoot` package logs that the build has no XWayland support, and an
XWayland build whose `PATH` somehow lacks the binary (a hand-built one, say)
logs `` `Xwayland` must be on PATH `` with the `PATH` it searched. Both then
run a Wayland-only session; neither stops the session from starting. Read
[the trust model](protocols.md#xwayland-opt-in) before turning it on: any X
client can keylog and read other X clients by design.

## Home-manager module

```nix
# In your home-manager flake inputs: scoot.url = "github:scoot-sh/scoot";
# In your home configuration:
imports = [ inputs.scoot.homeModules.scoot ];
# (Legacy spelling `inputs.scoot.homeManagerModules.scoot` still resolves
# to the same module.)

programs.scoot = {
  enable = true;
  # `package` defaults to the flake's own build for your system on Linux
  # (the same per-system default as `packages`); on macOS it defaults to
  # null (files only — the module manages the config, installs no
  # binary). Set it only to override:
  # package = inputs.scoot.packages.${pkgs.system}.scoot;
  # # macOS, to also install the remote-control client:
  # package = inputs.scoot.packages.${pkgs.system}.scootctl;
  settings = {
    layout.gap = 8;
    binds = {
      "super+t" = "spawn foot";
      "ctrl+alt+space" = "spawn wofi --show drun";
    };
    autostart.commands = [ "spawn waybar" ];
  };
};
```

| Option | Default | Meaning |
|---|---|---|
| `enable` | `false` | Manage scoot files (and install `package`). |
| `package` | flake's own build (Linux), `null` (macOS) | The binary installed to your profile. `null` installs no binary (files only). |
| `settings` | `{ }` | Free-form config, rendered verbatim to TOML (see below). Empty renders a valid minimal file: the compositor runs it as pure defaults. |
| `configFile` | `"scoot/config.toml"` | Where the rendered TOML lands, relative to `$XDG_CONFIG_HOME`. Keep the default unless you pass the same path via `--config` wherever you launch scoot. |
| `sessionScript` | `null` | Startup script text, written executable beside the rendered config at `<dirOf configFile>/session.sh` (`scoot/session.sh` with the default `configFile`). Launch it with `scoot -- ~/.config/scoot/session.sh` for the default (or `~/.config/<that path>` after a `configFile` override), or exec it from your greetd/startwm entry — on NixOS, that launch line goes in the NixOS module's `session.command` (see below), which is what makes the greeter entry run this script instead of a bare compositor. `null` writes no file. |
| `portals.enable` | `true` | Install `scoot-portals.conf` to the per-user xdg-desktop-portal lookup path (`~/.config/xdg-desktop-portal/`), so ScreenCast/Screenshot resolve to the `wlr` backend inside a scoot session. Inert outside one (nothing reads it until `XDG_CURRENT_DESKTOP=scoot`). Turn off if you manage portal backends some other way. |
| `wallpaper.enable` | `settings ? wallpaper` | Install `wallpaper.package` and set `settings.wallpaper.command` to its store path (a `command` you set yourself wins). On whenever `settings` has a `wallpaper` table; `false` leaves both alone, so `[wallpaper]` runs `scootbg` from `PATH`. See [The wallpaper](#the-wallpaper-scootbg). |
| `wallpaper.package` | flake's own `scootbg` (Linux), `null` (macOS) | The scootbg to install. `null` installs nothing and sets no `command`. |

The example above renders byte-for-byte to:

```toml
[autostart]
commands = ["spawn waybar"]

[binds]
"ctrl+alt+space" = "spawn wofi --show drun"
"super+t" = "spawn foot"

[layout]
gap = 8
```

(That TOML is the module's own output for those settings, captured from
a real evaluation with the one invalid test bind removed — keys render
alphabetically, hence the table order.)

**Why free-form?** `settings` is a TOML value, not a typed schema per
field — deliberately. The config moves fast (three tables landed this
month); a typed schema goes stale and then lies about what the
compositor accepts, while free-form never drifts. `[binds]` is arbitrary
keys anyway, so a schema would need an escape hatch exactly where most
user content lives. Field names, types and defaults are documented in
[configuration.md](configuration.md); the full live-defaults reference
is at the bottom of this page, pasted from a real
`scoot --print-default-config` emission, not hand-written.

After `home-manager switch`, apply the new settings without re-logging in:
`scoot msg reload` (`scootctl reload` is the same client) re-reads the file
and re-applies what can move live; the two restart fields are refused with a
message naming that rather than silently kept (see
[configuration.md](configuration.md#reloading-the-config)).

## NixOS module

```nix
# In your NixOS flake inputs: scoot.url = "github:scoot-sh/scoot";
# In your system configuration:
imports = [ inputs.scoot.nixosModules.scoot ];

programs.scoot = {
  enable = true;
  # package defaults to the flake's own scoot build, like above.
  session.enable = true;   # opt-in login-screen entry (default false)
  # Run the home-manager session script from the greeter entry
  # (pairs with `sessionScript` above; default null is a bare `--tty`):
  # session.command = "${config.programs.scoot.package}/bin/scoot --tty -- /home/alice/.config/scoot/session.sh";
};
```

| Option | Default | Meaning |
|---|---|---|
| `enable` | `false` | Install `package` system-wide. |
| `package` | flake's own `scoot` build | The binary to install and to launch from the session entry. Set it to `scoot-xwayland` for X11 apps (see [XWayland](#xwayland-from-the-flake)). |
| `wallpaper.enable` | `enable` and a package available | Install `wallpaper.package` system-wide, so a user's `[wallpaper]` section finds `scootbg` on `PATH` (any user, and a session the greeter starts). On with `enable` whenever there is a package (the flake's modules and overlay provide one), off otherwise; `false` opts out (another wallpaper daemon). |
| `wallpaper.package` | flake's own `scootbg` build | The scootbg to install. Setting `wallpaper.enable = true` with no package (direct module use without the overlay) is an eval error naming this option; leaving it at its default is not. |
| `session.enable` | `false` | Add a scoot entry to the display-manager/greetd session menu. |
| `session.command` | `null` | Full `Exec=` line for the session entry. `null` renders the bare `<package>/bin/scoot --tty` (existing configs unchanged). Set it to run something inside the session — usually `<package>/bin/scoot --tty -- <command>`, e.g. the home-manager `sessionScript` output (`/home/alice/.config/scoot/session.sh` for a user `alice` with defaults), or a wrapper script path that launches scoot itself (logging, environment setup). |

The session entry **adds a session alongside existing ones, never
replacing the default**: the module writes a `scoot.desktop`
(`Exec=<package>/bin/scoot --tty` by default, `DesktopNames=scoot` — which is what
sets `XDG_CURRENT_DESKTOP=scoot` when launched from a display manager)
into `services.displayManager.sessionPackages`, the additive list
greetd/tuigreet, GDM and SDDM read. With `session.command` set, `Exec=`
is that string verbatim — the pairing with the home-manager module is
`session.command = "<package>/bin/scoot --tty --
/home/alice/.config/scoot/session.sh"` (for a user `alice`; substitute
your own username), so the greeter starts the compositor with
your session script instead of a bare one; a wrapper script path works
the same way (the wrapper launches scoot itself). The value renders
verbatim, so desktop-entry quoting (spaces, quotes, pipes) is yours to
get right — copy the example shape. `Exec=` lines get no shell
expansion (`~` and `$HOME` arrive literally — `~` is additionally
reserved by the Desktop Entry Spec), so always spell the script out as
an absolute path, quoted per the spec. It never sets
`services.displayManager.defaultSession` (the pre-select) or
`services.greetd.settings.default_session` / `initial_session` (what
actually runs), so enabling it puts scoot in the menu next to your
existing sessions and switches nothing. `session.enable` still defaults
to **off**: a login-screen change is a change to your way back into your
own desktop, and that stays an explicit opt-in.

One auto-login caveat, verified against the pinned nixpkgs source: with
no explicit `defaultSession`, NixOS falls back to the head of the
session list for the autologin target — so on a box that auto-logs-in,
adding *any* session package can move that target. If you use
`autoLogin`, pin `services.displayManager.defaultSession` explicitly.

A complete session, as one config — the home-manager side declares the
config and the startup script, the NixOS side points the greeter entry at
that script:

```nix
# Home configuration (user `alice`):
programs.scoot = {
  enable = true;
  settings = {
    output.scale = 2.0;
    binds."super+t" = "spawn foot";
  };
  sessionScript = ''
    waybar &
    exec foot
  '';
};

# System configuration:
programs.scoot = {
  enable = true;
  session.enable = true;
  session.command = "${config.programs.scoot.package}/bin/scoot --tty -- /home/alice/.config/scoot/session.sh";
};
```

The greeter starts scoot with the session script instead of a bare
compositor. The script path is absolute (`Exec=` lines get no shell
expansion), and it is the default `<dirOf configFile>/session.sh` — a
`configFile` override moves both halves, so keep them paired.

The config file itself is per-user, so it stays in the home-manager
module above — the NixOS module owns the binaries and the login entry,
nothing else. (Direct-module users, importing `nix/modules/*.nix` rather
than the flake's modules: apply [the overlay](#the-overlay), and
`package` and `wallpaper.package` default to its `pkgs.scoot` and
`pkgs.scootbg`; without it, set `package` explicitly — the NixOS module
fails loudly at eval, naming the option, rather than writing a session
entry with no binary — and `wallpaper.package` too if you want scootbg:
without one, `wallpaper.enable` defaults to off, and only an explicit
`wallpaper.enable = true` with no package fails at eval.)

## The wallpaper: scootbg

A `[wallpaper]` section in scoot's config (see
[configuration.md](configuration.md#wallpaper)) runs `scootbg`, the
wallpaper daemon, which is its own package. With either module, turning
scoot on is enough: nothing else to install, no path to write.

- **NixOS** installs `scootbg` system-wide whenever `programs.scoot.enable`
  is on (`programs.scoot.wallpaper.enable` follows it whenever a package
  is available, which the flake's modules and overlay provide; set it to
  `false` to opt out). The default `[wallpaper] command = "scootbg"` finds it on
  `PATH`, for every user and for a session the greeter starts, with or
  without home-manager. Installing it changes nothing until a config asks
  for a wallpaper, which is why it is on by default while the login
  entry stays opt-in.
- **home-manager** installs it whenever `settings` has a `wallpaper`
  table, and renders `command` as the package's store path, so the
  section works whatever is on `PATH`. The path changes with every
  upgrade, and that is harmless: scootbg leaves `command` out of the
  fingerprint that decides whether the section changed, so an upgrade
  never re-applies the section over a `scootbg set` pick.
- **One pinned pair.** Both modules default to the scoot and scootbg of
  the same flake revision, so the `apply-config` scoot speaks is the one
  the installed scootbg understands. Pin one without the other and a
  mismatch is reported in scoot's log (`apply-config` names both builds),
  never silently ignored.
- **Linux only.** On macOS `wallpaper.package` is `null`: the section is
  rendered as written (for the Linux box it deploys to) and nothing is
  installed.

The two together, next to the complete session above:

```nix
# Home configuration:
programs.scoot = {
  enable = true;
  settings.wallpaper = {
    image = "~/Pictures/hills.jpg";   # resolved against HOME by scoot
    mode = "fill";
    output."DP-2".color = "#101014";
  };
};

# System configuration (scootbg is installed by `enable` alone;
# the line below only says so):
programs.scoot = {
  enable = true;
  # wallpaper.enable = true;   # the default: follows `enable`
};
```

Either half alone works too: home-manager alone installs scootbg into the
user profile and points `command` at it; NixOS alone puts it on the
system `PATH` for a hand-written config.

No NixOS VM test boots a session to look at the wallpaper: CI proves the
same path end to end without one (`scripts/smoke-test.sh` and
`crates/scootbg/tests/scoot_config.rs` start a real scoot whose
`[wallpaper]` section runs scootbg, and check the pixels on each output),
and the module's own part, the package on `PATH` and the `command` it
renders, is pinned by the evaluation checks in `nix/tests.nix`. A
`nixosTest` would add a full NixOS system build and a VM boot to every
CI run for no path those do not already cover.

## The status bar: scootbar

`packages.<system>.scootbar` is [scootbar](scootbar/README.md), the status
bar, alone: one binary, linking nothing beyond glibc and libgcc_s, and
**no font in its closure**. Its installed closure is one of the bar's
measured rows ([the resource ratchet](scootbar/backlog/lightest.md)), and
the font is the user's to choose: even the one face the bar looks for first,
DejaVu Sans, is 742 KiB, nearly the size of the bar's own 826 KiB binary. It takes its font from `--font`, or from
the first of a short list of well-known files
([cli.md](scootbar/cli.md#fonts)), and with neither it refuses to start,
saying how to give one. On NixOS those files are usually absent, so name
one:

```sh
nix run github:scoot-sh/scoot#scootbar -- daemon \
  --font "$(nix build --no-link --print-out-paths nixpkgs#dejavu_fonts.minimal)/share/fonts/truetype/DejaVuSans.ttf"
```

or run **`scootbar-demo`**, the same binary behind a small script that
adds DejaVu Sans (`dejavu_fonts.minimal`, one 742 KiB file) as its
default `--font`:

```sh
nix run github:scoot-sh/scoot#scootbar-demo                          # scootbar daemon, a clock
nix run github:scoot-sh/scoot#scootbar-demo -- daemon --right clock  # any daemon flags
```

With no arguments it runs `daemon`; a `--font` you give wins; anything
other than `daemon` (`--help`, `--version`) passes through unchanged. It
is a demo, for trying the bar on a box with no fonts where it looks: the
overlay does not provide it, and a system should install `scootbar` and
give it a font its own font setup provides.

Modules are Cargo features (one per module, `clock` the default; see
`crates/scootbar/Cargo.toml`), reachable through `.override`:

```nix
# A bar with no modules: a plain bar, which needs no font.
scootbar.override { buildNoDefaultFeatures = true; }
# Exactly the modules listed.
scootbar.override { buildNoDefaultFeatures = true; buildFeatures = [ "clock" ]; }
# The default modules and the PNG decoder for `icon-image` (not a module and
# not a default: +115 KB, docs/scootbar/icons.md), built the same way.
scootbar.override { buildFeatures = [ "icon-image" ]; }
```

`programs.scootbar.features` (below) is the same list, so `features = [ "clock"
"workspaces" "icon-image" ]` builds the default bar with PNG icons.

### The modules: `programs.scootbar`

`homeModules.scootbar` (home-manager) and `nixosModules.scootbar` (NixOS)
are separate from the `programs.scoot` modules (`default` and `scoot`), so
importing one changes nothing about the other. Both take the same options;
the flake's own `scootbar` is the default `package`.

```nix
imports = [ inputs.scoot.homeModules.scootbar ];   # or nixosModules.scootbar
programs.scootbar = {
  enable = true;
  features = [ "clock" "workspaces" ];   # optional: build with exactly these modules
  settings = {                           # bar.toml, any key: see scootbar/cli.md#the-config-file
    left = [ "workspaces" ];
    center = [ "clock" ];
    bar.height = 32;
    colors.accent = "#89b4fa";
  };
};
```

| Option | Default | What it does |
| --- | --- | --- |
| `enable` | `false` | Installs the bar, writes its config and (unless `systemd.enable` is off) runs it. |
| `package` | the flake's `scootbar` (`pkgs.scootbar` with the overlay, else null) | The bar. Null with `enable` is refused, by name. |
| `features` | `null` | The modules as Cargo features, **exactly** the ones listed (`scootbar.override { buildNoDefaultFeatures = true; buildFeatures = features; }`); `[ ]` is a bar with no modules and no font. Null keeps `package` as it is. Needs a package with `.override`, as the flake's. |
| `settings` | `{ }` | Free-form: rendered as TOML to the bar's config, so a new option never needs a module change first. Unknown keys are the bar's own loud error, not the module's. |
| `stylix.enable` | `true` | Whether to take defaults from Stylix when it is in use (below). |
| `systemd.enable` | `true` | The user service, below. Off: start `scootbar daemon` from your compositor's autostart. |

**The file.** home-manager writes `~/.config/scoot/bar.toml`, the bar's
default path, so `scootbar daemon` from a shell reads what the service does
and `scootbar msg reload` re-reads it. NixOS writes `/etc/scootbar/bar.toml`
(the bar reads only `$XDG_CONFIG_HOME`, not `XDG_CONFIG_DIRS`), which the
system unit names with `--config`; a `scootbar daemon` started by hand reads
the user's own file instead.

**The service** is `scootbar.service` (a user unit): `WantedBy=` and
`PartOf=` `graphical-session.target`, `After=` it and `Before=tray.target`
(an ordering against a target the session does not define does nothing),
`Restart=on-failure` with `RestartSec=2` and `StartLimitIntervalSec=0` (no
burst limit: insurance, since systemd's default of 5 starts in 10 s is not
reached by a 2 s retry either, measured on systemd 261), so a crash, or a
start before the compositor's `WAYLAND_DISPLAY` is imported, retries, while
`scootbar msg kill` and a stop stay stopped (scoot does not supervise its
clients). `scripts/scootbar-unit-test.sh` runs this unit under a real
`systemd --user` against a live scoot and checks each of those. The unit
carries `X-Restart-Triggers` on the config file; home-manager's switch
restarts it on a new config, while whether NixOS restarts a user unit on
switch depends on the release (log in again or `systemctl --user restart
scootbar` otherwise; unverified here). scoot does not start
`graphical-session.target` itself: a scoot session script imports the
environment and starts it, as for any compositor:

```sh
dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
systemctl --user start graphical-session.target
```

**Stylix, without depending on it.** When `config.lib.stylix` exists and
`stylix.enable` is on, `settings` gets defaults from it: the five `colors`
tokens from the base16 palette (`background` base00, `foreground` base05,
`accent` base0A, `dim` base03, `urgent` base08), `bar.font` from
`stylix.fonts.sansSerif` and `bar.font-size` from `stylix.fonts.sizes.desktop`
(points, converted to the bar's pixels at 4/3 and clamped to 1 to 256). The
font is a **file path**: scootbar has no fontconfig, so a build step finds
the regular face in the font package's `share/fonts`: a file named for the
family, `DejaVuSans.ttf` for "DejaVu Sans", or `Family-Regular.ttf` (Noto
Sans, a Nerd Font). **Variable fonts** (Inter's `InterVariable.ttf`),
**`.ttc` collections** and **other naming** (Ubuntu's `Ubuntu-R.ttf`,
Cantarell, Fira Sans) are not resolved. The bar then gets the plain default
font (DejaVu Sans) rather than a failed system build, and the font's build
prints a warning naming the family (in its build log, and in `warning` beside
the font in the store; not at evaluation, which would need the font package
built then). Set `settings.bar.font` to a path to choose the face. The
module never imports Stylix; nothing changes without it, and
`programs.scootbar.stylix.enable = false` turns the defaults off with it
present.

**Precedence**, highest first, per key:

1. **A value you set in `settings`** (an ordinary definition).
2. **Stylix's** (`lib.mkDefault`).
3. **The module's plain default**: `bar.font` as DejaVu Sans (also what an
   unresolved Stylix font becomes)
   (`dejavu_fonts.minimal`, so a bar with modules always starts), and only
   that, unless `features = [ ]`. Every other key absent is the bar's own
   default.

So `settings.colors.background = "#123456"` replaces that one token and
keeps the other four from Stylix, and a value you set is never overridden by
theming (the trouble Waybar's users report). This is pinned by an
evaluation test, and checked against a real Stylix and home-manager/NixOS
(see `nix/scootbar-tests.nix`). A native `stylix.targets.scootbar` is not
provided: that is an upstream Stylix change.

**Checks.** `checks.<system>.scootbar-modules` (Linux; run by CI's
`nix flake check`) evaluates both modules with and without a Stylix stand-in,
pins the precedence, the font pick, `features`, the unit and the refusals,
and runs the **real `scootbar` binary** over each rendered file with
[`scootbar daemon --check`](scootbar/cli.md#--check-validate-without-running)
(the config, the flags over it, the modules and the font, as a start does
them, with no compositor and nothing written to the runtime directory); a
file with an unknown key is refused by name.

## The overlay

`overlays.default` adds `pkgs.scoot`, `pkgs.scootctl` and, on Linux,
`pkgs.scootbg` and `pkgs.scootbar`:

```nix
nixpkgs.overlays = [ inputs.scoot.overlays.default ];
```

They are the flake's own builds, the same derivations as
`packages.<system>.*`, not rebuilt against your nixpkgs: scoot and scootbg
stay one pinned pair, and nothing is built twice. With the overlay applied,
the modules' `package` and `wallpaper.package` default to these, which is
what makes the pure modules (`nix/modules/home.nix`, `nix/modules/nixos.nix`)
usable without the flake's wrappers. On macOS the overlay adds `scoot`
and `scootctl` (the client) and no `scootbg` or `scootbar`. It does not
add `scootbar-demo` (see [The status bar](#the-status-bar-scootbar)).

## Migrating from a hand-rolled packaging

Converting an existing `flexwm` setup to the flake is three renames, all
silent at build time:

- **`${pkg}/bin/flexwm` → `${pkg}/bin/scoot`** in any
  `writeShellScriptBin` wrapper or `Exec=` line. Nix interpolates the store
  path without checking the binary exists, so a stale path builds fine and
  fails at the login screen.
- **`xdg.configFile."flexwm/config.toml"` → `programs.scoot.settings`**
  (or `"scoot/config.toml"`). This is the dangerous one: scoot only reads
  the `scoot` path, and a missing file at the default path is silent by
  design (see
  [configuration.md](configuration.md#failure-semantics)) — so a stale path
  boots happily on built-in defaults, silently dropping scale and binds.
  Move the content into `settings` and delete the old entry, otherwise your
  real config sits orphaned at a path nothing reads while the module renders
  defaults where scoot looks.
- **The module split**: `homeModules.scoot` (the legacy spelling
  `homeManagerModules.scoot` still resolves) owns the config file;
  `nixosModules.scoot` owns the binaries (scoot, and scootbg for
  `[wallpaper]`) and the login-screen entry. A
  hand-rolled `xdg.configFile` next to the module manages a file scoot
  never reads — keep the module's and delete the hand-rolled one.

## Settings failure modes

Two cases, behaving differently by design:

- **A value with no TOML representation** (e.g. a Nix function in
  `settings`) fails the option type-check at evaluation time ("not of type
  'TOML value'"), so the error aborts before anything builds, let alone
  starts a session. Loud and early.
- **A value that renders but has the wrong scoot type** (a string for
  `layout.gap`) builds fine and is refused at session start — where the
  loader fails safe: the whole file is discarded for built-in defaults,
  the error is logged, and the session still boots. A typo costs the
  config, never the session. (This is why `settings` needs no
  build-time type schema to be safe: see
  [configuration.md](configuration.md#failure-semantics) for the full
  refusal rules.)

## Reference: live defaults

Every key below is present in `settings` exactly as shown (values are the
live defaults; the two commented-unset keys stay commented — uncommenting
either changes the session, as noted below). Pasted from a real
`scoot --print-default-config` emission on 2026-09-22 (dev-VM Linux
build from the tree at `e04d2b0`), not hand-written — regenerate rather
than edit by hand if it ever looks stale:

Two keys describe less than the packaged binary decides: setting
`backend = "gles"` needs EGL drivers from the host OS (on NixOS,
`hardware.graphics` enabled) and is a loud startup error without them,
and the `--tty` scanout tier needs `packages.scoot-gpu` instead of
`packages.scoot` — see [GPU tiers from the
flake](#gpu-tiers-from-the-flake). `[tty] gpu` is unaffected by packaging
either way: it names a host DRM device path, not a build feature.
Likewise `[xwayland] enabled = true` needs `packages.scoot-xwayland` (or
`scoot-gpu-xwayland`) -- see [XWayland from the
flake](#xwayland-from-the-flake); with the default package it logs a
warning and the session runs Wayland-only.

```toml
[layout]
# Gap between columns, between windows stacked in a column, and at output edges.
gap = 12
# Column widths as fractions of the output width, in cycle order.
column_widths = [0.3333333333333333, 0.5, 0.6666666666666666]
# Index into column_widths for newly created columns.
default_column_width = 1

[appearance]
# Ring thickness; clamped to at most half of gap.
focus_ring_width = 3
# Ring thickness around every window that is not focused; unset means the same
# as focus_ring_width (shown here with that default; uncommenting it pins the value,
# so it stops following focus_ring_width). Same clamp; 0 is no ring.
focus_ring_inactive_width = 3
focus_ring_active_color = "#6ba6fa"
focus_ring_inactive_color = "#595961"
background_color = "#14141a"
corner_radius = 0
cursor_size = 16
cursor_color = "#ffffff"
# Unset follows $XCURSOR_THEME, then "default"; name one here only to override that.
# cursor_theme = "Adwaita"
prefer_no_csd = true

[output]
# Output scale advertised to clients and rendered at.
scale = 1.0

[renderer]
# Which renderer composites each frame. --renderer wins over this when both name one.
backend = "pixman"

[tty]
# Unset means the automatic search picks; --gpu PATH wins over this when both name one.
# Name the display controller (prefer a stable /dev/dri/by-path/... alias):
# gpu = "/dev/dri/card0"

[xwayland]
# Run an XWayland server inside the session (opt-in X11 support, off by default). --xwayland wins when either names it: the two are OR-ed. Needs an `xwayland` Cargo-feature build; without one the knob warns and the session runs Wayland-only. Takes effect on restart.
enabled = false

[autostart]
# Action strings to run once each, in file order, at session startup. A reload runs entries the session has not seen yet (new spawns only).
commands = []

[binds]
"super+h" = "focus-column left"
"super+l" = "focus-column right"
"super+j" = "focus-window down"
"super+k" = "focus-window up"
"super+shift+h" = "move-column left"
"super+shift+l" = "move-column right"
"super+shift+j" = "move-window down"
"super+shift+k" = "move-window up"
"super+alt+h" = "consume-or-expel left"
"super+alt+l" = "consume-or-expel right"
"super+ctrl+j" = "focus-workspace down"
"super+ctrl+k" = "focus-workspace up"
"super+shift+ctrl+j" = "move-window-to-workspace down"
"super+shift+ctrl+k" = "move-window-to-workspace up"
"super+r" = "cycle-column-width"
"super+f" = "toggle-fullscreen"
"super+q" = "close"
"super+Return" = "spawn foot"
"super+shift+e" = "quit"
"super+comma" = "focus-output 1"
"super+period" = "focus-output 2"
"super+shift+comma" = "move-window-to-output 1"
"super+shift+period" = "move-window-to-output 2"
"super+1" = "focus-workspace-index 0"
"super+shift+1" = "move-window-to-workspace-index 0"
"super+2" = "focus-workspace-index 1"
"super+shift+2" = "move-window-to-workspace-index 1"
"super+3" = "focus-workspace-index 2"
"super+shift+3" = "move-window-to-workspace-index 2"
"super+4" = "focus-workspace-index 3"
"super+shift+4" = "move-window-to-workspace-index 3"
"super+5" = "focus-workspace-index 4"
"super+shift+5" = "move-window-to-workspace-index 4"
"super+6" = "focus-workspace-index 5"
"super+shift+6" = "move-window-to-workspace-index 5"
"super+7" = "focus-workspace-index 6"
"super+shift+7" = "move-window-to-workspace-index 6"
"super+8" = "focus-workspace-index 7"
"super+shift+8" = "move-window-to-workspace-index 7"
"super+9" = "focus-workspace-index 8"
"super+shift+9" = "move-window-to-workspace-index 8"
```

(Differences from the raw emission are mechanical and stated: the emission's
14-line file-header prose is dropped, comment lines unwrapped where the
emission's prose wraps, `#`-commented keys
uncommented with their default values filled in — except `cursor_theme`
and `tty.gpu`, which stay commented exactly as emitted because they are
unset by default (placeholders, not values: setting either changes the
session) — the 3-line `corner_radius` comment is dropped, and the trailing note
about `--tty` VT binds folded into
[configuration.md](configuration.md#binds). The values are untouched.)
