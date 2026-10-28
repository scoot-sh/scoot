---
title: The login screen
description: "The ReGreet greeter, and starting the session from a TTY autologin."
---

## The greeter

**Passthrough, not profiled.**
`programs.scoot.desktop.greeter` is `programs.scoot.greeter` under the
profile's name (same options, same assertions, same forced session
entry). In particular `desktop.enable` never touches the login screen:
no default session, no autologin, nothing that could strand a login —
the greeter stays an explicit opt-in on top of the profile. Logging in
through it starts the same session [First session](../start/first-session.md)
describes.

**The login screen follows its own look.**
`programs.scoot.greeter.look` names the login screen's look: one entry
of `programs.scoot.looks`. It resolves at system build time into
ReGreet's CSS, backdrop, dark setting and sans — a live swap repoints
only the session's own state and never reaches the login screen. The
look also renders a `greet.toml` fragment, which nothing reads yet (it
is for the future scootgreet).

| Option | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `programs.scoot.greeter.look` | null or string | `null` (follow `programs.scoot.look`) | system rebuild | the login screen's look (e.g. `"moonrise"`); an explicit `null` themes nothing |

Set it to pin the login screen while the session starts elsewhere:

```nix title="configuration.nix"
programs.scoot.desktop.enable = true;
programs.scoot.greeter.enable = true;
programs.scoot.look = "moonrise";
programs.scoot.greeter.look = "ginger-night";  # or null for an unthemed login screen
```

The theming applies only with `greeter.enable` and `desktop.enable`
both on. An unknown name fails the evaluation, naming the catalog. A
custom catalog look themes from its manifest down the `extends` chain.
`desktop.theme.targets.greeter.enable = false` opts the login screen
out while the rest follows the look.

Writing your own ReGreet CSS instead of using a look's? Scope card
rules to `frame.background`, never bare `frame` — see
[Theming](../scoot/theming.md#what-stays-a-static-file-and-why).

**No buildup across logins.**
While the greeter is on, each greeter session's leftover processes
(the session bus, ReGreet's accessibility bus) are stopped with the
session through logind (`KillUserProcesses` scoped to the `greeter`
user only), so repeated logins leave no `closing` sessions behind.
An explicit `services.logind.settings.Login.KillUserProcesses` (or
`KillOnlyUsers`) of your own still wins. Two edges:
`KillUserProcesses = false` alone does not opt out: while
`KillOnlyUsers` lists `greeter`, logind kills the greeter's processes
without ever reading the kill switch. To restore stock behavior,
override `KillOnlyUsers` too (e.g.
`services.logind.settings.Login.KillOnlyUsers = [ ];`), but know that
an empty list with the defaulted `KillUserProcesses = true` kills
every user's processes. Second, your `KillOnlyUsers` replaces the
greeter's list instead of merging with it, so keep `"greeter"` in
your list to keep the fix. With the greeter off, logind is left
exactly as nixpkgs ships it.

## Without a greeter (TTY autologin)

No login screen: the console logs itself in and the shell profile
starts the session. Enable the profile as usual, then name who logs
in and let their user manager start at boot:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop = {
  enable = true;
};
```

```nix title="configuration.nix"
services.getty.autologinUser = "alice";
users.users."alice".linger = true;
```

On `tty1` that is the whole setup — the shell profile execs the
session on that console only, so SSH logins keep a shell:

```sh
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ "${XDG_VTNR:-}" = 1 ]; then
  exec scoot-session
fi
```

With the profile on and a console autologin without lingering, the
rebuild warns naming the user and the fix (a display-manager
autologin never warns: that path registers a full login, whose
manager starts).

Why lingering is the step: recent systemd gives TTY autologin
sessions the `user-light` class, which never starts
`user@UID.service` — so without it `scoot-session` finds no user
manager and falls back to a bare `scoot --tty` with no bar, no
notifications, no clipboard history, no portal and no session target.
The launcher names that cause on stderr and mirrors it into the
system journal (the compositor takes over the console on startup,
burying stderr), so the diagnosis survives the login. An autologin
is also one of the logins the keyring cannot prime from the login
password ([Privilege prompts](./auth.md)): the first
secrets use of each login unlocks once, then stays unlocked till
logout.

> **Symptom:** the session starts with no bar, no popups, no
> clipboard history — but the compositor itself runs.
>
> The session fell back to a bare `scoot --tty`: the user manager
> never started. Check the class (`loginctl show-session
> "$XDG_SESSION_ID" -p Class --value` prints `user-light` on an
> affected login) and the manager (`systemctl --user list-units`
> fails there), read the launcher's diagnosis (`journalctl -b -t
> scoot-session`), and set the autologin user's `linger` above.
> Logging in through the greeter instead starts the full session
> with no lingering.
