---
title: "A systemd session for scoot: greeter-started sessions get graphical-session.target, the activation environment and a clean shutdown"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# A systemd session for scoot: greeter-started sessions get graphical-session.target, the activation environment and a clean shutdown

Filed 2026-10-04 from the maintainer's question: "Do we have whatever's
needed in our nix module to support a greeter too?" Serves
**daily-drive** (a greeter-started scoot is a whole desktop: the bar and
portals come up, and nothing leaks into the next login) and, second,
**computer use** (an agent driving a greeter-started session inherits the
same activation environment and target ordering a daily driver gets).

## The gap

`nix/modules/nixos.nix`'s opt-in `programs.scoot.session.enable` installs
`share/wayland-sessions/scoot.desktop` (via
`services.displayManager.sessionPackages`, never the default session)
running `scoot --tty` or `session.command`. Once a greeter starts it, three
things are missing:

1. Nothing imports `WAYLAND_DISPLAY`/`XDG_CURRENT_DESKTOP` into the
   systemd user manager and the D-Bus activation environment, and nothing
   starts `graphical-session.target`. So the home-manager scootbar unit
   (`nix/modules/scootbar-home.nix`: `WantedBy`/`PartOf`/`After`
   `graphical-session.target`) never starts, and D-Bus-activated portals
   miss the session. `docs/configuration.md` ("Portals and the D-Bus
   activation environment") leaves the import to the user's session script;
   scoot itself deliberately never touches the bus.
2. Nothing stops session-bound user services when scoot exits, so they
   leak into the next login.
3. `docs/nix.md` tells users to hand-roll a `scoot-session.target` in
   their session script (a `BindsTo` shim around a
   `RefuseManualStart=yes` target) with no teardown and no readiness gate
   — every user re-derives the same wiring.

## What to do

Follow niri's shape (`resources/niri-session`, `niri.service`,
`niri-shutdown.target` — read for the pattern, write our own: niri is
GPL-3.0, scoot is MIT), adapted to one hard constraint: scoot never
touches the bus, so there is no `--session` flag and no `Type=notify`;
the launcher does the readiness wait and the import:

1. **`resources/scoot-session`** (shell, shipped by the flake's `scoot`
   packages under `bin/`, installable outside Nix): refuses while a scoot
   session is active for the user (`scoot.service` or
   `graphical-session.target`); imports the login environment into the
   user manager (`systemctl --user import-environment`, then the scoped
   `dbus-update-activation-environment --systemd WAYLAND_DISPLAY
   XDG_CURRENT_DESKTOP` once the display is known — the scoped two-var
   form, not `--all`, so a login's `SSH_*`/`TERM`/etc. never reach
   D-Bus-activated apps); starts `scoot.service` (`--wait`, so an
   immediate failure aborts before anything else) and waits for readiness
   (`scoot msg version` against the IPC socket with a deadline, plus a
   service-liveness check each round — no blind sleep); learns
   `WAYLAND_DISPLAY` exactly (the service's `MainPID` process environment
   — scoot binds `wayland-1..32` ignoring the caller's `WAYLAND_DISPLAY`,
   so pre-setting the name is not a mechanism) and refuses to import a
   display whose socket does not exist; on exit (or on signal) starts
   `scoot-shutdown.target`, which `Conflicts=` the session target away so
   session-bound units stop, and unsets what was imported. Without a
   systemd user manager it degrades to `exec scoot --tty` with a note, not
   a failure.
2. **`resources/systemd/user/scoot.service`**: `BindsTo`/`Before`
   `graphical-session.target` (+ `Wants`/`After`
   `graphical-session-pre.target`), `ExecStart=<pkg>/bin/scoot --tty`,
   `Type=simple`. **`scoot-shutdown.target`**: `Conflicts=` the session
   targets, `StopWhenUnneeded=yes`. No `xdg-desktop-autostart.target`
   pull: scoot's `[autostart]` is the session's startup mechanism; the
   freedesktop autostart execution is a separate ticket if wanted.
3. **NixOS module**: `session.enable` installs the units
   (`systemd.user.units`, paths interpolated) and points the
   `wayland-sessions` entry at `<pkg>/bin/scoot-session`;
   `session.command` keeps verbatim semantics (null renders the launcher;
   a set value replaces the whole `Exec=` line — a `scoot --tty --
   <script>` value then runs *without* the session wiring, documented;
   the wiring-aware route for startup programs is `[autostart]`).
   `nix/tests.nix` pins the units (`BindsTo`/`Before`, `ExecStart`),
   the entry (`Exec=.../scoot-session`), and the off state.
4. **Home-manager**: no new option — the module manages files, it does
   not launch scoot, so there is nothing to bind; `scootbar-home.nix`'s
   `WantedBy`/`PartOf`/`After graphical-session.target` ordering is
   already the right half and now has a target that is actually reached
   (the hand-rolled target recipe in `docs/nix.md` is replaced by the
   launcher).
5. **Docs**: `docs/nix.md` (what starts what, greeters GDM/SDDM/greetd,
   the `session.command` mapping, manual install outside Nix),
   `docs/configuration.md` (the launcher owns the import on systemd; the
   manual two lines stay for non-systemd setups), `docs/tty.md` (pointer
   only — it describes no greeter flow today).

## Not in this ticket

- `programs.scoot.greeter.enable` (greetd + ReGreet-in-scoot, cage-style):
  decided **no**. A greeter option rewrites the machine's login path, and
  the never-strand rule plus the no-DM-change/no-reboot constraint on the
  dev VM mean it would ship untested; any greeter reading
  `wayland-sessions` already works with the entry above, so nothing is
  blocked without it.
- Launcher session-command arguments: decided **no**. Forwarding an exact
  argv through a static unit needs shell word-splitting (which breaks the
  quoting `session.command`'s verbatim design exists to preserve) or
  generated unit files (new failure modes in the login path);
  `[autostart]` already runs startup programs inside the wired session.
- `sd_notify`/`Type=notify` readiness in the compositor: not a mechanism
  that exists (no signal to reuse), and adding bus-adjacent startup
  protocol to the compositor cuts against the kept decision above.
- xdg-desktop-autostart execution, per above.
