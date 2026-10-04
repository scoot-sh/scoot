---
title: "A systemd session for scoot: greeter-started sessions get graphical-session.target, the activation environment and a clean shutdown"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
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
   user manager and the D-Bus activation environment together
   (`dbus-update-activation-environment --systemd --all`, falling back
   to `systemctl import-environment` without the bus half when the dbus
   tool is missing); starts `scoot.service` (plain `start`, not
   `--wait` — see the evidence record) and waits for readiness
   (`scoot msg version` against the IPC socket with a deadline, plus a
   service-liveness check each round — no blind sleep, and each attempt
   under a `timeout` so a mid-startup connection the loop never answers
   cannot wedge the wait); learns `WAYLAND_DISPLAY` by difference (the
   session's own new socket under `$XDG_RUNTIME_DIR` against a
   pre-start snapshot, compared by name *and* inode since a reused
   stale name is a replaced file — pre-setting the name is not a
   mechanism since
   scoot binds `wayland-1..32` ignoring the caller, and the service's
   `/proc` environment cannot work since `set_var` leaves the exec-time
   copy `/proc` shows untouched; zero or several new sockets stop the
   service instead of guessing); on exit (or on signal) starts
   `scoot-shutdown.target`, which `Conflicts=` the session target away so
   session-bound units stop, and restores the two display variables in
   the user manager to their pre-session values (the bus has no unset,
   so the activation environment keeps the last values like any
   session). Without a
   systemd user manager it degrades to `exec scoot --tty` with a note, not
   a failure, and it starts the service with plain `start` rather than
   `--wait` (`--wait` hangs forever on some user managers even for
   already-active units — measured on systemd 261, with no job queued;
   the readiness loop's liveness check covers stillborn services).
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
   `greeter.enable` (default false) is a thin opt-in over nixpkgs' own
   `services.displayManager.regreet` (greetd + cage hosting ReGreet):
   it turns that on, forces `session.enable` on so scoot is listed,
   and optionally names ReGreet's backdrop (`greeter.background` becoming
   `background.path`; null under Stylix, whose own regreet target owns
   the backdrop — setting both is refused). GDM/SDDM beside it are
   refused at eval; rollback is the previous generation. `nix/tests.nix`
   imports the real regreet module and pins on/off, the backdrop, and
   every refusal with its message.
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

- Hosting ReGreet (or any greeter) *inside scoot itself*: that needs a
  locked-down scoot profile (no binds, no IPC socket) and is out of
  scope -- a possible later step, said so in `docs/nix.md`. The
  `greeter.enable` above runs ReGreet under cage, the way nixpkgs runs
  it, and lists scoot as a session.
- A `nixosTest` booting greetd+ReGreet: measured against the dev VM's
  disk budget (see the evidence record) and skipped -- the eval pins
  above are the coverage, and this repo runs no `nixosTest` anywhere
  today for the same build-cost reason.
- Launcher session-command arguments: decided **no**. Forwarding an exact
  argv through a static unit needs shell word-splitting (which breaks the
  quoting `session.command`'s verbatim design exists to preserve) or
  generated unit files (new failure modes in the login path);
  `[autostart]` already runs startup programs inside the wired session.
- `sd_notify`/`Type=notify` readiness in the compositor: not a mechanism
  that exists (no signal to reuse), and adding bus-adjacent startup
  protocol to the compositor cuts against the kept decision above.
- xdg-desktop-autostart execution, per above.

## Resolution (PR #416, 2026-10-04)

Landed as designed, with five live-found fixes (each re-verified):

1. Display discovery by `/proc` MainPID environ does not work:
   `set_var` moves `environ` off the initial stack while
   `/proc/PID/environ` keeps the exec-time copy (a compositor answering
   IPC showed no `WAYLAND_DISPLAY` there). Discovery is socket
   difference instead.
2. The readiness poll had no per-attempt timeout and wedged on a
   mid-startup connection the loop never answers (launcher stuck in
   `do_wait` 01:14 live). Attempts run under `timeout 2`.
3. `systemctl --user --wait start` hangs forever on systemd 261 user
   managers even for already-active units, with no job queued (measured
   twice, including on a trivial active sleep unit). Plain `start` plus
   the loop's liveness check instead.
4. Name-only socket snapshots miss stale reuse (scoot rebinds a stale
   name, same name, new inode — a healthy session was stopped as "0 new
   sockets"). Snapshots compare name *and* inode, and a vanishing
   socket is its own refusal.
5. `die()` paths skipped env restore (a failed run's defaulted desktop
   leaked into the next run's "pre-session" values). Restore runs on
   every post-capture exit; the pre-capture refusal never touches
   another session's environment.

Evidence (dev VM, `ssh -p 2222 dev@localhost`, tree at PR #416 head
`07b1dfc25` unless noted):

- `nix flake check`: all checks passed (aarch64-linux; includes the new
  unit/entry/greeter eval pins and all pre-existing pins).
- Fail-first: new unit pins against the old module fail at eval on
  `osSession...units ? "scoot.service"` (thrown copy); greeter forcing
  reverted fails on `allAssertionsHold osGreeter.config`. Restored, both
  pass.
- Live login (prebuilt `/var/cargo-target/debug/scoot`, Oct 1 — no Rust
  changes in this ticket; disk 97%/1.2G free, no rebuild room): units
  installed to `~/.config/systemd/user`, launcher run over ssh.
  While running: `scoot.service`, `graphical-session.target` and a
  `WantedBy` probe unit all `active`; manager env carried
  `WAYLAND_DISPLAY=wayland-2` + `XDG_CURRENT_DESKTOP=scoot`; the D-Bus
  activation env carried both; a second launcher run refused with exit
  1. After `scoot msg action quit`: all four units `inactive`, both
  manager variables unset, launcher exited.
- `shellcheck -S warning resources/scoot-session`: clean.
  `nix fmt -- --check` on touched nix files: clean.
- nixosTest skipped: 1.2G free cannot build a VM image (brief made it
  conditional); eval pins are the coverage, matching repo practice (no
  `nixosTest` anywhere in CI).
- Greeter scope change mid-work (coordinator): the first "no greeter"
  decision was replaced by the thin `programs.scoot.greeter.enable`
  over nixpkgs' `services.displayManager.regreet` (default false, GDM /
  SDDM refused at eval, Stylix conflict refused at eval after verifying
  Stylix's own regreet target sets the same `background.path`). Hosting
  ReGreet inside scoot stays out of scope (locked-down profile).
