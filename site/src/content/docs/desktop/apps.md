---
title: Terminal, files, and removable media
description: "The terminal, file opening, the user directories and automounting."
---

The compositor's built-in `Super+Return` opens `foot`, so the profile
installs foot — a bind that names a missing binary is a dead key. The
look themes it (the [terminal target](./look.md#app-theme) writes `foot.ini`);
this slot owns the package, `TERMINAL=foot` for anything that asks,
and the plumbing every "open this" goes through:

- `xdg-open` (xdg-utils) on `PATH`, and `BROWSER=xdg-open`: a link
  opens through your `x-scheme-handler/https` default. The profile ships no
  browser.
- The XDG user dirs exist (`xdg.userDirs`, created on activation):
  `~/Downloads`, `~/Pictures` (where [screenshots](./screenshots.md)
  land), `~/Documents` and the rest.
- With the file manager on, directories open in it: `inode/directory`
  and `inode/mount-point`, the type of `/tmp` and of every mounted
  drive (without the second, `xdg-open` on a mount falls through to a
  browser).

The file manager is optional and off: nothing in the desktop needs
one, and a graphical one carries a toolkit. Turn it on for
mouse-driven browsing:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.apps.fileManager.enable = true;   # pcmanfm
```

Without it `xdg-open` keeps working for files and links. A directory
has no default then, and `xdg-open` falls back: to a web browser's file listing if one is installed, or "no
method available" if not. Nothing hangs.

A default text editor makes `git`, `xdg-open` on a text file and
every `EDITOR` reader agree:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.apps.editor.default = "nvim";   # or "helix"
```

Set: the editor installs (Neovim follows the look's colorscheme),
`EDITOR` and `VISUAL` name its binary (at `mkOverride 900`: beating
nixpkgs' `nano` default, losing to a plain assignment, merging with
`programs.neovim.defaultEditor` on the same value), and `text/plain`
(and `text/*`) opens through a generated entry running `foot -e
<editor> %F` (the terminal option's binary). Your own `EDITOR`,
`VISUAL` or text association wins outright over the profile's.
`null` (the default) installs and sets nothing; a null `package`
beside a set default fails evaluation naming it.

Removable media mounts on insert: `udiskie` runs trayless as a user
unit (`scoot-automount.service`, wanted by `scoot-session.target`)
over the udisks2 daemon the NixOS side enables. Insert a drive and it
mounts under `/run/media/$USER/`, with a notification whose *Browse*
action opens the mount through `xdg-open` — the file manager, when it
is on. An encrypted (LUKS) drive asks for its passphrase, then mounts
its unlocked filesystem the same way. It runs on udiskie's stock
device rules, which leave the machine's own disks alone: udisks2
marks an internal disk `HintSystem`, and udiskie skips it and every
partition on it, so a dual-boot box's Windows or macOS partitions
stay unmounted, with no admin prompt at login. A
`~/.config/udiskie/config.yml` of your own still applies (the unit
passes no `-c`; its `-a -n -T` flags win over the file's). A file
udiskie cannot parse makes every start fail, so the unit gives up
after 5 failed starts in a minute rather than retrying forever (see
troubleshooting below).
With home-manager's own `services.udiskie` on, that one runs instead
and this unit stays out: two automounters would race to mount every
drive and both ask for a passphrase. udiskie stays installed either
way, so `udiskie-umount` is there for safe removal.

**Safe removal: unmount before you unplug.** A yanked drive loses
whatever was still buffered. One command unmounts and powers it off:

```sh
udiskie-umount -d /run/media/$USER/LABEL
```

Wait for the command to return, then unplug. `udiskie-umount -a`
unmounts every drive udiskie handles at once; the machine's own disks
are not among them.

Why these, measured on the M2 (`aarch64-linux`): full closures at
the pinned rev (`8ce4ef6`) by `nix path-info --closure-size`;
"new over the profile" as the store paths the tool adds to the whole
desktop profile as deployed on the M2 (its system built with and
without the slot); idle cost under a headless scoot in its own
`dbus-run-session`, over every process the app brought
(D-Bus-activated helpers included): VmRSS, and context switches
summed over the threads alive across 60 s idle after a 30 s settle:

| File manager | Version | Full closure | New over the profile | Idle RSS | Idle wakeups | Left after closing | Why / why not |
|---|---|---|---|---|---|---|---|
| pcmanfm | 1.4.0 | 347.3 MiB | 5.3 MiB (4 paths: itself, libfm, libfm-extra, menu-cache) | ~50 MB | 0 in 60 s | nothing | the pick: the smallest closure and marginal, its GTK 3 already in the profile, silent at idle, gone when closed |
| thunar | 4.20.9 | 376.2 MiB | 27.0 MiB (13 paths) | ~52 MB | 2 in 60 s | nothing | as quiet and as light at runtime, five times the marginal (the xfce libraries) |
| yazi (in foot) | 26.9.1 | 524.7 MiB | 75.1 MiB (13 paths) | ~54 MB with its foot (~27 MB without) | 418 in 60 s | nothing | the terminal shape, but wrapped with its previewers it is the heaviest closure, and its async runtime wakes about seven times a second at idle |

pcmanfm is the pick: smallest everywhere it differs, and nothing
left running once its window closes (no `menu-cached`, `gvfsd` or
thumbnailer: checked live after closing it). Point `fileManager.package` at another (with its `desktopEntry`) to
choose differently.

Why udiskie, same method, measured live in the session (the
automounter's own unit, after the drive test below):

| Process | Idle RSS | Idle wakeups | Closure over the profile |
|---|---|---|---|
| udiskie, trayless (the unit) | ~58 MB | 0 in 60 s | 48.1 MiB (35 paths: Python, PyGObject, docopt, PyYAML) |
| `udisksctl monitor` (the floor a hand-rolled automounter would start from) | ~8 MB | 0 in 60 s | 0 (ships with udisks2) |
| udisksd (needed either way) | ~18 MB | 0 in 60 s, with `/etc/nvme` present (see below) | ~25 MiB more with its filesystem tools (the slot is 73.5 MiB, 70 paths, in all) |

Neither polls; udiskie costs about 50 MB more than the bare floor,
for per-partition handling, an encrypted-drive passphrase prompt,
the *Browse* action, the device rules above, and `udiskie-umount`
for safe removal. A scootmount on the floor's shape is the lighter
path if the memory ever matters more than that list.

The profile also creates `/etc/nvme` (an empty `/etc/nvme/.keep`,
through `environment.etc`, so it exists under an immutable `/etc`
too): udisksd watches that directory (NVMe-oF host identity), and
GLib retries a watch on a missing path every 4 seconds — 15 wakeups a
minute, forever, measured on the M2 (all
`inotify_add_watch("/etc/nvme") = ENOENT`), and 0 once the directory
exists.

Every value is an option, applied on rebuild/switch (the unit
restarts with the new config; a new `TERMINAL` or `BROWSER` reaches
apps started after the next login):

| Option | Type | Default | Meaning |
|---|---|---|---|
| `desktop.apps.terminal.enable` | bool | `true` with the profile | foot on `PATH` (left to home-manager's `programs.foot` when that is on: its package wins), `TERMINAL=foot` |
| `desktop.apps.terminal.package` | package or null | foot (Linux-only: null off Linux) | the terminal to install; the compositor's `Super+Return` still names `foot`, so another terminal also needs that bind |
| `desktop.apps.fileManager.enable` | bool | `false` | the file manager on `PATH`, directories and mounts opening in it |
| `desktop.apps.fileManager.package` | package or null | pcmanfm (Linux-only: null off Linux) | point at another file manager |
| `desktop.apps.fileManager.desktopEntry` | string | `"pcmanfm.desktop"` | the `.desktop` entry directories open through (`"thunar.desktop"` beside a thunar `package`) |
| `desktop.apps.editor.default` | null, `"nvim"` or `"helix"` | `null` | the default text editor: installs it, sets `EDITOR` and `VISUAL` (at `mkOverride 900`: beating nixpkgs' `nano` default, merging with `programs.neovim.defaultEditor`), and answers `text/plain` (and `text/*`) through a terminal entry running it (`foot -e <editor>`, following the terminal option); null installs and sets nothing |
| `desktop.apps.editor.package` | package or null | neovim or helix, following the default (Linux-only: null off Linux) | the default editor to install (left to home-manager's `programs.neovim`/`programs.helix` when that is on: its package wins) |
| `desktop.automount.enable` | bool | `true` with the profile | udisks2 on the NixOS side, udiskie and its trayless unit on the home-manager side (the unit left out while home-manager's `services.udiskie` is on: its unit runs instead) |
| `desktop.automount.package` | package or null | udiskie with its flaky keyring tests off (Linux-only: null off Linux) | point at your own udiskie build |

A null `package` beside its switch fails evaluation naming it.
`xdg-open`, `BROWSER` and the user dirs come on with the terminal or
the file manager (and `xdg-open`/`BROWSER` with the automounter, for
its *Browse* action). Your own `xdg.userDirs` and `xdg.mimeApps`
values win outright over the profile's: `xdg.userDirs.enable = false`
(no `~/Music` and friends) or a
`xdg.mimeApps.defaultApplications."inode/directory"` of your own
replaces the profile's, with no conflict and no merged list.

Troubleshooting, by symptom:

- *Activation stops: "Existing file '…/.config/user-dirs.dirs' would
  be clobbered"* (or `mimeapps.list`). A file you (or an
  app) wrote is in the way of the one the profile manages. Move it
  aside, or set home-manager's `backupFileExtension` so activation
  moves it for you.
- *A browser cannot make itself the default.* With the file manager
  on, `~/.config/mimeapps.list` is the profile's, read-only. Set the
  default there instead:
  `xdg.mimeApps.defaultApplications."x-scheme-handler/https" = "firefox.desktop";`
  (and `x-scheme-handler/http`, `text/html`).
- *A drive does not mount.* `systemctl --user status scoot-automount`
  first: *skipped* (condition failed) means no udisks2 on the system
  bus — a home-manager-only setup needs `services.udisks2.enable` on
  the system. Running but nothing mounted: `udisksctl status` lists
  what udisks2 sees, and `udisksctl mount -b /dev/sdX1` names the
  refusal. *failed* with `start-limit-hit` means udiskie failed 5
  starts in a minute: `journalctl --user -u scoot-automount` shows
  why, most often a `~/.config/udiskie/config.yml` it cannot parse.
  Fix (or move) the file, then
  `systemctl --user reset-failed scoot-automount` and
  `systemctl --user start scoot-automount`.
- *An internal partition does not automount.* By design (above):
  mount it by hand with `udisksctl mount -b /dev/...`, which asks for
  the admin password.
