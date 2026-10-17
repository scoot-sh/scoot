---
title: "install.sh: say clearly that --system adds the login-screen entry, and warn about a per-user copy"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-17"
---

# install.sh: say clearly that --system adds the login-screen entry, and warn about a per-user copy

Filed 2026-10-17, after the webtop and installer review. Serves
**daily-drive**: the first install is the first impression, and today a
per-user install says almost nothing about how to get scoot on the login
screen.

## The gap

`install.sh` without `--system` installs under `~/.local` and writes no
session entry, so scoot is not on the login screen. The only pointer is
one parenthesis on the last line (`site/public/install.sh`, "(Re-run with
--system to add scoot to your login screen.)"). The usage text and
`site/src/content/docs/start/install.mdx` do not lead with the choice.

Running `--system` after a per-user install is file-safe: the two installs
use different paths, manifests and version records, and a file the script
did not write is never overwritten. It is not announced, and the per-user
copy keeps shadowing the system one for that user (`~/.config/systemd/user`
beats `/usr/local/lib/systemd/user`; `~/.local/bin` is often first on
`PATH`), so a session picked from the login screen can run old user units
under a newer launcher.

## What to do

1. **Make the choice explicit.** The usage text, the install page and the
   end-of-run message say, in one plain sentence each: a plain run
   installs for you only and adds no login-screen entry; `--system` also
   adds the "scoot" session to the login screen (needs root, writes
   `/usr/share/wayland-sessions/scoot.desktop`) and leaves your greeter and
   its config alone. Print the `--system` command to run, copy-pasteable,
   with the same `--look` and `--no-bar` flags the user just used.
2. **`--system` detects a per-user install** (a per-user manifest exists
   for this user) and prints a note: the per-user copy may take precedence
   for you, with the exact `install.sh --uninstall` to remove it. Offer the
   removal behind an explicit flag or prompt; never remove it silently.
3. **The per-user run detects a system install** and says the system copy
   exists, so the user knows which one a login-screen session runs.
4. A test in `site/scripts/test-install.sh` for the notes in both orders,
   and the install page documents the two installs side by side.

## Not in this ticket

Installing or configuring any greeter, a scoot greeter for non-Nix systems,
or changing how the session file is written.

Landed in PR #654 (`feat(install)`): the usage header, the Any Linux
install page (both commands, side by side) and the end-of-run messages
state the choice; a per-user run prints its matching `--system` command
with `--look`/`--no-bar`; each direction warns when the other copy
exists (the `--system` side resolves the invoking user through
`SUDO_USER`/getent); removal stays an explicit `install.sh --uninstall`.
`test-install.sh` 10a-10e prove the command, both warnings, the quiet
case and the sudo path; mutation-proved (a dead note fails 10b, a
dropped `--no-bar` fails 10a). Verified end to end as root in an
ubuntu:24.04 container (real installs both ways, warnings in order,
uninstalls exact) plus the suite green under ShellCheck 0.11.0 on macOS
and Linux. The greeter is untouched.
