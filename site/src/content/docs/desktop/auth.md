---
title: Privilege prompts and the keyring
description: "A polkit agent to ask for passwords and a keyring to remember secrets."
---

Mount a disk from the file manager, change a network setting, open a
printer queue — without this slot every privileged GUI action fails
for want of someone to ask the password, and without the keyring every
app asks for its secrets again after each login. Both come on with
the profile: a polkit agent to ask, a keyring to remember.

```nix title="configuration.nix and home.nix"
programs.scoot.desktop = {
  enable = true;
  # No privilege prompts on this box (privileged actions refuse
  # naming the missing agent, as below):
  # auth.enable = false;
  # No keyring on this box (every app re-prompts for secrets):
  # secrets.enable = false;
  # A different agent (default polkit-gnome):
  # auth.daemon = "lxqt";
};
```

What runs: the login entry names the agent (`SCOOT_POLKIT_AGENT`
in its `Exec` line), and the session leader spawns it once the
display is known -- supervised, so a dead agent restarts after two
seconds, and stopped with the session. It must run there, as a
child of the leader: a polkit agent registers against its own logind
session, and a systemd user unit never joins one (measured live: the
unit's agent stays connected but unregistered, and registration
fails with "User of caller and user of subject differs"). The agent
holds nothing when no prompt is open: it blocks on D-Bus, keeps no
surface, and wakes nothing (measured below). With the prompt off the
entry is the plain launcher, and privileged actions refuse with
polkit's own no-agent error. A hand-written entry (a set
`session.command` replaces the whole line) carries the agent by
setting `SCOOT_POLKIT_AGENT` itself, to the agent's absolute path.
The keyring runs nothing at all until asked: gnome-keyring owns
`org.freedesktop.secrets` through D-Bus activation
(its service files
start the daemon `--components=secrets` on the first call, and it
holds the unlocked keyring until the session ends), with
`secret-tool` (from libsecret) on `PATH` for scripts and terminals:

```sh
secret-tool store --label='wifi' ssid MyNetwork   # any login...
secret-tool lookup ssid MyNetwork                  # ...reads back, no prompt
```
On a greetd login the login password unlocks
the keyring during login (below), so the first use — and every later
one till logout — is seamless. Off the greetd path the first use of
each login performs the one unlock (a graphical app prompts; the CLI
refuses loud until unlocked).

Why polkit-gnome, measured at the pinned rev (`8ce4ef6`,
`aarch64-linux`: full closures by `nix path-info --closure-size`,
marginals as new store paths over the profile's own tools, idle RSS as
VmRSS under Xvfb, wakeups as context-switch deltas over 60 s idle,
all on the M2):

| Tool | Version | Full closure | New over profile | Idle RSS | Wakeups idle | Why / why not |
|---|---|---|---|---|---|---|
| polkit-gnome | 0.105 | 335.9 MiB | 403 KiB (1 path: itself) | ~4.1 MB | 0 in 60 s | the pick: its whole GTK stack already rides with the portal backend, so the marginal is the agent alone; the oldest and most boring of the three |
| lxqt-policykit | 2.4.0 | 858.9 MiB | 385.9 MiB (39 paths) | ~4.1 MB | 0 in 60 s | a second toolkit (Qt6: qtdeclarative alone is 195 MB) for the same prompt |
| hyprpolkitagent | 0.1.3 | 1.7 GiB | 642.9 MiB (71 paths) | ~4.2 MB | 0 in 60 s | Qt6 *plus* KDE (kirigami, breeze-icons, ffmpeg) at ~1600× the marginal — and the youngest codebase of the three |

Every agent idles at zero wakeups (all three block on D-Bus, none
polls), so the pick is closure, not CPU: polkit-gnome's marginal is three orders of magnitude smaller than
either Qt agent's.

Why gnome-keyring, same method:

| Tool | Version | Full closure | New over profile | Idle RSS | Wakeups idle | Why / why not |
|---|---|---|---|---|---|---|
| gnome-keyring | 50.0 | 412.6 MiB | 34.6 MiB (11 paths) | ~4.1 MB | 0 in 60 s | the pick: D-Bus activated (no unit, starts on first use), and the only candidate the login password unlocks (below) |
| KeePassXC | 2.7.12 | 542.4 MiB | 169.5 MiB (26 paths) | — | — | needs its full GUI app running with the database open: a second toolkit (Qt5) for a second prompt by design — no PAM unlock path exists |

D-Bus activation, not an always-running daemon: the daemon starts on the first secrets call and persists only because an
unlocked keyring must live somewhere. The polkit agent cannot do the
same (no activation protocol exists for agents: polkitd needs one
registered before the prompt), so the session leader spawns it once
the display is known -- event-driven and idle-silent.

Unlocking takes no extra password entry on a greetd login: the
module adds two PAM rules confined to greetd's own service (an `auth`
rule caching the login password beside its `login` substack, a
`session` rule with `auto_start`, each ten past it), and a password
login through ReGreet unlocks there (the greeter log says `gkr-pam:
gnome-keyring-daemon started properly and unlocked keyring`). The
unlocked daemon lands in the *user's* session scope, not the
greeter's: `pam_systemd` runs inside the `login` substack, ahead of
the keyring rule, so the worker is already moved into the new scope
before the daemon forks — and it stays there till logout. The first
secrets use finds it through the control socket at its default path
(`$XDG_RUNTIME_DIR/keyring/control`): D-Bus activation runs
`gnome-keyring-daemon --start`, which hands the bus name to the
already-running unlocked daemon and exits, so no second daemon ever
serves. (An earlier session misread a greeter-scope kill of the
*greeter's own* daemon as the user's daemon dying, and measured a
locked session daemon on logins where the PAM pair was never active;
re-tested live, logout plus re-login reads back with no prompt — see
the ticket's landed notes.) Nixpkgs' own `enableGnomeKeyring` flag
cannot do even the priming
(probed at the pinned rev: it lives inside the default-rules block,
and greetd's service sets `useDefaultRules = false`, so the flag is
a silent no-op there); and the stock
`services.gnome.gnome-keyring.enable` switch is not used either (it
owns the `login` service's PAM, which would broaden the change past
greetd -- this slot wires the same daemon, bus files and IPC-lock
wrapper directly, and only greetd's stack).

What else prompts: anything that never puts the password through
greetd's PAM -- a TTY login, an SSH login, an autologin (no password
at all), or a login screen that is not greetd -- and a login
password that differs from the keyring's (changed one without the
other). The keyring never blocks the session: a locked keyring is
prompts, not a failure.

With neither half, failures stay loud, never silent: no agent, and a
privileged action refuses with polkit's own error naming the missing
agent (`pkexec` says no agent can authenticate); no keyring, and
`secret-tool lookup` fails naming the missing bus name. An SSH
session sharing the user manager gets the same loud refusal (no agent
is registered for its session). The agent dying mid-prompt fails that
one prompt (the client sees the dismissal); the leader restarts it
in two seconds and the next prompt works — retry it.

Every value is an option. A rebuild/switch re-renders the session
entry, but the running agent is the session's own child, so a change
to the agent takes effect at the next login:

| Option | Type | Default | Meaning |
|---|---|---|---|
| `desktop.auth.enable` | bool | `true` with the profile | name the agent in the session entry and run the authority beside it |
| `desktop.auth.daemon` | enum (`"gnome"`, `"lxqt"`, `"hyprpolkit"`) | `"gnome"` | the agent behind `enable` (a future scoot agent widens this without renaming anything) |
| `desktop.auth.package` | package or null | per-daemon (Linux-only: null off Linux) | point at your own agent build; null with the switch on fails evaluation naming it |
| `desktop.secrets.enable` | bool | `true` with the profile | publish the keyring's bus names, install daemon and client, unlock on greetd logins |
| `desktop.secrets.daemon` | enum (`"gnome-keyring"`) | `"gnome-keyring"` | the daemon behind `enable` (a future scoot keyring widens this without renaming anything) |
| `desktop.secrets.package` / `.clientPackage` | package or null | gnome-keyring / libsecret (Linux-only: null off Linux) | point at your own builds; null with the switch on fails evaluation naming it |

Troubleshooting, by symptom:

- *A privileged action says no authentication agent.* The leader never
  spawned it: `pgrep -a polkit-gnome` (or the lxqt/hypr binary for
  those daemons) empty means the prompt has nobody to ask — check
  the entry carries `SCOOT_POLKIT_AGENT` (`grep SCOOT_POLKIT_AGENT
  /run/current-system/sw/share/wayland-sessions/scoot.desktop`),
  then `loginctl` (an SSH session has no agent by design — run the
  action from the graphical session). With `auth.enable = false`
  this is the expected refusal, not an error.
- *The prompt appears but the password never works.* The agent shows
  the dialog, polkitd decides: check the action's policy
  (`pkaction --verbose`), and that the user is in the right group.
- *The keyring asks every login.* On a greetd login the login
  password unlocks during login, and the
  unlock holds till logout. If a use still prompts, the priming
  never happened — `journalctl` shows no `gkr-pam` line for the
  login — and the PAM pair never applied: `grep gnome_keyring
  /etc/pam.d/greetd` should show two lines. Off the greetd path
  (TTY, SSH, autologin, another greeter) the first secrets use of
  each login unlocks once (the login password), then stays unlocked
  till logout.
- *`secret-tool lookup` says no such service.* The bus names never
  published: `secrets.enable` without the NixOS side (a
  home-manager-only setup publishes nothing — the activation files
  are system files). `busctl --user list | grep -i secret` is empty
  until the first use wakes the daemon.
- *Two prompts stack.* Another agent runs beside this one (a desktop
  environment's own, or a hand-started one): only one agent per
  session should register — turn this one off
  (`auth.enable = false`) or stop the other.
- *Two keyrings fight.* Another secrets implementation owns the bus
  name (KeePassXC's secret-service with its app open, another
  gnome-keyring): whoever owns `org.freedesktop.secrets` answers —
  `busctl --user status org.freedesktop.secrets` names the owner.
