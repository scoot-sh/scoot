# scoot test VM

A NixOS VM (aarch64) for developing **scoot**, a Wayland compositor, from
a Mac. The point is to get a *real* DRM/KMS device, real libinput devices and a
real seat — the things a compositor needs and macOS cannot provide — while you
keep editing code on the Mac.

```
vm/
  flake.nix           nixosConfigurations.scoot-vm + the Mac-native launcher
  configuration.nix   the guest: qemu-vm settings, wayland/DRM stack, rust, dev user
  run-vm.sh           where VM state lives and what gets shared in
  linux-builder.sh    one-time: the Linux builder that builds the guest closure
  authorized_keys     dropped into the VM for dev@ and root@
```

## One-time setup

macOS cannot build Linux derivations, and the guest's system closure is one, so
a Linux builder VM has to exist first. This is the standard `nixpkgs` builder;
its own closure is prebuilt upstream, so it does not need a builder itself.

```sh
./vm/linux-builder.sh setup     # installs keys, /etc/nix/machines, ssh config (sudo)
./vm/linux-builder.sh run       # leave this running in its own terminal
./vm/linux-builder.sh status    # in another terminal: proves the daemon can reach it
```

`setup` also writes three download settings to `/etc/nix/nix.custom.conf`
(Determinate Nix rewrites `nix.conf`, so user settings live next door):

- `http2 = false` — parallel downloads freeze under HTTP/2 multiplexing
  ([NixOS/nix#3438](https://github.com/NixOS/nix/issues/3438)); here every
  frozen transfer died at the same byte offset, ~20 MiB in
- `stalled-download-timeout = 20` — a freeze costs 20s instead of 300s
- `builders-use-substitutes = false` — the Mac does all downloading and pushes
  to the builder over ssh, so those two fixes cover everything. The builder
  runs stock settings (see "Stall warnings" below for why they stay that way).

If you set up the builder before this, apply them by hand and restart the
daemon — the settings are read at daemon startup:

```sh
sudo sed -i '' -E '/^(http2|stalled-download-timeout|builders-use-substitutes)[[:space:]]*=/d' /etc/nix/nix.custom.conf
printf '%s\n' 'http2 = false' 'stalled-download-timeout = 20' 'builders-use-substitutes = false' \
  | sudo tee -a /etc/nix/nix.custom.conf
sudo launchctl kickstart -k system/systems.determinate.nix-daemon
nix config show | grep -E '^(http2|stalled-download-timeout|builders-use-substitutes) '
```

## Boot it

From the repo root, with the builder running:

```sh
nix run ./vm                 # first run builds the closure, then boots
nix run ./vm -- --reset      # throw the VM's disk away and start clean
nix run ./vm -- -snapshot    # extra args go straight to qemu
```

A QEMU window opens — that is the display scoot will drive. The terminal you
launched from is the serial console (`Ctrl-a x` quits, `Ctrl-a c` opens the
qemu monitor).

- login: `dev` / `dev`, or `root` / `root`; `dev` is auto-logged-in on tty1
- `ssh -p 2222 dev@localhost` from the Mac
- the directory you ran `nix run` from is shared into the VM at `/mnt/scoot`
  (override with `SCOOT_SRC=/path/to/repo`)
- the VM's disk lives in `~/.local/state/scoot-vm/`

Coming from a VM built before the `flexwm` → `scoot` rename: the share moved
from `/mnt/flexwm` to `/mnt/scoot` and the state directory from
`~/.local/state/flexwm-vm/`, so a VM already on disk needs its state moved
across or it boots from a fresh (empty) disk, losing the guest's cargo target
cache:

```sh
mv ~/.local/state/flexwm-vm ~/.local/state/scoot-vm
mv ~/.local/state/scoot-vm/flexwm-vm.qcow2 ~/.local/state/scoot-vm/scoot-vm.qcow2
```

A VM that is already *running* keeps the share it booted with (`/mnt/flexwm`)
until it is rebuilt and rebooted; `/mnt/scoot` is what the next boot mounts.

## Is the stack alive?

In the QEMU window:

```sh
drm_info                    # what virtio-gpu exposes
kmscube                     # DRM/KMS + GLES, no compositor involved
sudo libinput list-devices  # keyboard and tablet(=absolute mouse) from qemu
sway                        # reference compositor; WLR_RENDERER=pixman is quicker
```

`sway` is there only as a known-good comparison. Nothing claims the display at
boot, so scoot can take it.

## The scoot loop

Edit on the Mac, build and run in the VM:

```sh
ssh -p 2222 dev@localhost
cd /mnt/scoot && cargo build           # CARGO_TARGET_DIR is /var/cargo-target,
                                       # i.e. on the guest disk, not over 9p
```

Then run the binary from the QEMU window's tty1 to actually watch it on the
virtual display, or over an ssh session — an ssh-started `--tty` run does take
real DRM master and does really scan out to the QEMU window, so only
*watching* it happen needs that window.

The mechanism is **seatd, not logind** (an earlier version of this file
credited "logind's PAM stack", which is wrong — see the DRM-master entry under
Troubleshooting for how that was measured). libseat picks its seatd backend
here, and seatd's `seat0` is VT-bound: it binds any client that connects to
whichever VT is in the *foreground at that moment* — normally tty1's autologin
console — no matter how that client's own process was started. The ssh session
itself has no seat at all (`loginctl show-session $XDG_SESSION_ID` reports
`Seat=`, `VTNr=0`, `Remote=yes`), and forcing the logind backend instead fails
on the spot: `LIBSEAT_BACKEND=logind scoot --tty` over ssh dies with
`Failed to open session: No data available`.

Two consequences of that VT binding, worth knowing before you debug the
symptom instead of the cause:

- The run takes over the foreground VT's display, and seatd allows exactly one
  client on a VT-bound seat at a time. Start a second `--tty` (or sway, or
  anything else on libseat) while one is up and it exits 1 with
  ``scoot: the session refused every DRM device on seat `seat0`: the seat is
  what failed here, not the choice of device …`` and, under it, the device it
  tried and `Failed to open device: Operation not permitted (os error 1)`.
  `journalctl -u seatd` says `seat is VT-bound and has an active client`. See
  the concurrent-agents entry under Troubleshooting for why this is easy to
  misread as a bug in whatever you were testing.
- It needs `XDG_RUNTIME_DIR` for the wayland socket, which a login session
  provides — ssh and tty1's autologin both do, a bare `su dev -c ...` does
  not (`no wayland socket: $XDG_RUNTIME_DIR is not set or invalid`).

`cargo build` works without `nix develop`: the
wayland/libinput/libxkbcommon/gbm/udev/seatd `.pc` files are in the system
profile and `PKG_CONFIG_PATH` points at it.

## Changing the guest config

Edit `configuration.nix` and either relaunch (`nix run ./vm`, which rebuilds
only the system closure — no image build) or push the change into the running
VM without rebooting:

```sh
NIX_SSHOPTS="-p 2222" nixos-rebuild switch \
  --flake ./vm#scoot-vm --target-host root@localhost
```

## Why it is built this way

- **NixOS's own `qemu-vm` module, not a hand-rolled disk image.** Setting
  `virtualisation.host.pkgs` to darwin pkgs makes `config.system.build.vm` an
  *aarch64-darwin* derivation — a native Mac launcher. It packs the store into
  an erofs image on the host at launch and boots the kernel directly. This is
  the same mechanism `nixpkgs`' own darwin linux-builder uses.

  The first version of this flake used `make-disk-image` to produce a
  bootable GPT/ESP/systemd-boot qcow2 instead, on the assumption that the
  `build-vm` runner is a Linux script and could not run on a Mac. That is only
  true when `host.pkgs` is left at its default. The disk-image path cost a
  nested TCG VM inside the builder, a ~9G image, and a bootloader install — all
  of it avoidable. The trade-off of the current approach: there is no
  standalone bootable disk artifact, so it will not boot in UTM as-is.

- **QEMU rather than Apple's Virtualization.framework.** `vz` is faster, but
  nixpkgs' vz backend has no display support, and a window manager needs one.

- **Software rendering.** QEMU's Cocoa display has no host GL passthrough, so
  virtio-gpu runs without virgl and Mesa falls back to llvmpipe. KMS, atomic
  commits, input and seats are all real; only the GPU is not.

- **Passwords in the config.** Throwaway local VM, not reachable off-host --
  enforced by `forwardPorts`' `host.address = "127.0.0.1"` (added after a
  security audit found the port forward binding to every interface by
  default, i.e. reachable from the whole LAN with these exact credentials).
  This isn't just VM compromise: the shared `/mnt/scoot` directory is a 9p
  mount with no read-only option in this NixOS module's `sharedDirectories`
  schema, so anyone who reached the VM over the network had write access to
  the actual host checkout through it. Keep `host.address` set on every
  forwarded port; don't remove it to "simplify" the config.

## Making it faster

- **Boot time is dominated by packing the store into erofs**, which happens on
  every launch and scales with the closure. Keeping the guest lean matters:
  `documentation` is off, and anything added to `environment.systemPackages` is
  paid for on each boot.
- **For near-instant boots**, set `virtualisation.mountHostNixStore = true` and
  `useNixStoreImage = false` in `configuration.nix`. The guest then 9p-mounts
  the Mac's `/nix/store` and nothing is packed at all, at the cost of slower
  reads inside the VM.
- **Never rebuild to change config** — use the `nixos-rebuild --target-host`
  line above; it pushes only the closure delta.

## Troubleshooting

**Substitution crawls, `less than 1 bytes/sec … (attempt 1/5)`.** Nix's download
pool hung a stream. Check the network is actually fine with
`curl -o /dev/null -w '%{speed_download}\n' https://cache.nixos.org/nix-cache-info`,
then apply the download settings above. Note that passing them as
`--option` on the command line is silently ignored (`restricted setting … you
are not a trusted user`) — downloads happen in the daemon, so it must go in
`/etc/nix/nix.custom.conf`.

**Stall warnings still say `the last 300 seconds`.** Those are the builder's
downloads, relayed over ssh, which means `builders-use-substitutes` is still
`true` — the builder is fetching with its stock settings. Set it to `false` as
above. Do not try to fix the builder's own `nix.settings` through the
`linux-builder` override in `flake.nix`: that changes the builder's guest
closure, which then needs a Linux builder to build, so the builder can no
longer start (`Failed to find a machine for remote build!`). Only host-side
options like `diskSize` are safe there.

**The builder runs out of disk / garbage-collects mid-build.** Its stock disk is
20G and its `min-free` GC trips at 1G free. `flake.nix` asks for 60G, which
applies when the disk is created: stop the builder, delete
`~/.local/state/nix-linux-builder/*.qcow2`, and start it again.

**The VM window is black but the serial console shows a login prompt.** The
framebuffer only lights up once something drives KMS. Log in on tty1 in the
window and run `kmscube`.

**`--tty` logs `Unable to become drm master, assuming unprivileged mode`. Is it
really DRM master?** Usually yes — that warning is a red herring on its own,
and it is not specific to ssh. It is Smithay's own (`backend/drm/device/fd.rs`),
and on a modern kernel it fires for *every* non-root compositor whose DRM fd
was opened for it by a session daemon (seatd, and by the same mechanism
presumably logind, though this project has only measured seatd), over ssh and
from a real VT alike. It means "this process may not call `SET_MASTER` itself",
not "this process is not master" -- but it does **not** by itself distinguish
"master, held by someone else's open" from "master, held by ours", so verify
it rather than trust the log alone (recipe below):

- Master goes to whichever open file is *first* to open the device while
  nothing else already holds it, plus seatd's own explicit
  `DRM_IOCTL_SET_MASTER` call on that file right after. Root is not what
  grants master -- `drm_master_open` (`drivers/gpu/drm/drm_auth.c`) hands it
  to the first opener regardless of uid, and root cannot take master away
  from an existing holder either (`drm_setmaster_ioctl` returns `EBUSY`).
  Root is what lets seatd open the device node and manage VTs at all. Since
  seatd is normally the only thing that ever opens the GPU node on this VM,
  in practice it *is* first, and scoot inherits that already-master file
  along with the fd seatd hands over its socket -- but that is a fact about
  this VM's setup, not a property of "opened by seatd" in general.
- scoot's own `SET_MASTER` is then refused with `EACCES`, because the kernel
  only permits it from the process that owns the file:
  `drm_master_check_perm` (`drivers/gpu/drm/drm_auth.c`) wants
  `was_master && file->pid == current->tgid`, and `drm_file_update_pid`
  (`drm_file.c`) deliberately never re-owns a file that *was* master.
- Smithay's resulting `privileged = false` is the correct state here: it stops
  it issuing its own `SET_MASTER`/`DROP_MASTER` on session pause/resume, which
  seatd already does as root on every VT switch.
- **The identical warning also fires when master genuinely isn't held**: if
  something else already has it when seatd opens the device, seatd's own
  `SET_MASTER` gets `EBUSY` too, only logs it, and hands the fd over anyway --
  that case fails loudly at modeset instead of at open. "The fd was opened by
  seatd" doesn't tell the two cases apart; the verification below does.

To check it for real rather than trusting the log, while `--tty` is running:

```sh
sudo cat /sys/kernel/debug/dri/0/clients  # master=y, on the seatd-opened fd
sudo cat /sys/kernel/debug/dri/0/state    # plane fb=N, "allocated by = scoot"
```
`clients` lists the fd under `seatd`'s pid, not scoot's, for the same
`drm_file_update_pid` reason — one open file, two processes. `state` is the
scanout proof: the CRTC's plane points at a scoot-allocated framebuffer
(`[fbcon]`'s own fb is what it points at when nothing is driving KMS).
`DRM_IOCTL_MODE_ATOMIC` is master-gated by the kernel, so a `drm: modeset
(full commit)` line in scoot's log with no immediately-following `WARN …
drm commit/page flip failed` (its own error path, not a generic "error") is
itself proof master was held at that moment.

**Two agents (or two shells) verifying `--tty` at once, and one of them gets an
errno that has nothing to do with what it was testing.** The seat takes one
client at a time (see The scoot loop above), so *anything* holding it — a
benchmark script, a `timeout 30 scoot --tty` left running, sway — makes every
other `--tty` start fail at `Session::open` with `Operation not permitted`,
whatever that other run was actually exercising. This has really happened here:
a benchmark from an unrelated task held the seat while PR #27's `--gpu` cases
were being reproduced, and the EPERM read as a `--gpu` bug until the holder was
noticed. Check before you start, and again if an errno surprises you:

```sh
pgrep -a scoot; pgrep -a sway             # nothing should be holding it
sudo journalctl -u seatd -n 5 --no-pager  # "Removed client N" = seat free now
```

Either wait for the other run to finish or coordinate — do not "fix" the
symptom. (scoot's own error says the seat is what failed rather than blaming
device choice, but only when *every* candidate failed that way.)

**A change you just made doesn't show up in the binary you just ran.**
`CARGO_TARGET_DIR` is set to `/var/cargo-target` globally on this VM (guest
disk, not the 9p mount — that's deliberate, it's much faster), which means
every checkout built here writes to the *same* `debug/scoot`. Build two trees
concurrently, or build in one ssh session while running from another, and the
binary you run can be the other tree's. Symptom: a log line or error message
that matches neither the code you edited nor the code you reverted to. Check
the binary is newer than the edit before trusting any run:

```sh
ls -l --time-style=full-iso /var/cargo-target/debug/scoot
```

`scripts/smoke-test.sh` now does part of this for you: an unresolvable
`SCOOT`/`SCOOTCTL` default fails loudly before anything launches, and every
run's first lines print each binary's path, source and mtime — read them
and confirm the binary is your build, especially after a concurrent build
may have replaced it.

Same applies to `cargo test` output when two builds race — rerun it once the
other build has finished rather than debugging the result.

**`nix run ./vm` wants to build aarch64-linux paths and fails.** The builder is
not running or the daemon cannot reach it: `./vm/linux-builder.sh status`.

**The builder's own `error: unable to download '...narinfo': Problem with the
SSL CA cert ... error adding trust anchors from file:` on first boot from a
fresh shell.** `vm/linux-builder.sh run`'s generated `run-nixos-vm` launcher
only shares a CA bundle into the builder VM (for its own `cache.nixos.org`
downloads) when `NIX_SSL_CERT_FILE` points at an existing file *in the
invoking shell* — it does **not** read `/etc/nix/nix.conf`'s `ssl-cert-file`
(a separate, daemon-only setting). A plain interactive shell may not have it
set. Fix, once, in `~/.zshrc` (or the equivalent for your shell):
```sh
export NIX_SSL_CERT_FILE=/etc/nix/macos-keychain.crt
```
That file is Determinate Nix's own macOS-Keychain-exported PEM bundle
(auto-refreshed, world-readable, no sudo needed). Recent Determinate Nix
versions' own `nix-daemon.sh` also auto-set a fallback CA bundle when this is
unset, so this may already work without the export — but keeping it explicit
is more robust (picks up any custom/corporate root CAs the Keychain has, and
doesn't depend on that fallback behavior persisting across future Nix
updates).

**`nix build` fails with `error adding trust anchors from file:
/etc/ssl/certs/ca-certificates.crt`.** That is a `nix build` run *inside* the
builder VM (over ssh), where it downloads for itself with no CA setup. Don't
build there: run `nix build .#scoot .#scoot-gpu` on the Mac from a shell with
`NIX_SSL_CERT_FILE` exported (above). The daemon hands the build to the
builder and does the downloading itself. No builder restart is needed
(2026-09-24: an agent read this error as needing a relaunch; a fresh Mac shell
built both packages cleanly).

**`nix run ./vm &` from an interactive shell gets "suspended (tty output)",
then a second attempt fails with `Failed to get "write" lock ... Is another
process using the image?`.** qemu is invoked with `-serial mon:stdio`, so it
calls `tcsetattr` on stdio; if that stdio is still the shell's controlling
terminal when the job is backgrounded with a bare `&`, the kernel sends
SIGTTOU and stops it — and the stopped process still holds the disk lock,
which is what the second command's error is actually about (a misleading
symptom of the real cause). Fix: find and kill the stuck process
(`ps aux | grep qemu-system-aarch64`, `kill <pid>`), then relaunch with stdio
fully redirected instead of a bare `&`:
```sh
nix run ./vm </dev/null >/tmp/scoot-vm.log 2>&1 &
```
This runs fine from an interactive shell and survives independent of it. The
same fix applies to `vm/linux-builder.sh run` if it ever tty-suspends the
same way.
