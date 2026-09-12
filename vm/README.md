# flexwm test VM

A NixOS VM (aarch64) for developing **flexwm**, a Wayland window manager, from
a Mac. The point is to get a *real* DRM/KMS device, real libinput devices and a
real seat — the things a compositor needs and macOS cannot provide — while you
keep editing code on the Mac.

```
vm/
  flake.nix           nixosConfigurations.flexwm-vm + the Mac-native launcher
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

A QEMU window opens — that is the display flexwm will drive. The terminal you
launched from is the serial console (`Ctrl-a x` quits, `Ctrl-a c` opens the
qemu monitor).

- login: `dev` / `dev`, or `root` / `root`; `dev` is auto-logged-in on tty1
- `ssh -p 2222 dev@localhost` from the Mac
- the directory you ran `nix run` from is shared into the VM at `/mnt/flexwm`
  (override with `FLEXWM_SRC=/path/to/repo`)
- the VM's disk lives in `~/.local/state/flexwm-vm/`

## Is the stack alive?

In the QEMU window:

```sh
drm_info                    # what virtio-gpu exposes
kmscube                     # DRM/KMS + GLES, no compositor involved
sudo libinput list-devices  # keyboard and tablet(=absolute mouse) from qemu
sway                        # reference compositor; WLR_RENDERER=pixman is quicker
```

`sway` is there only as a known-good comparison. Nothing claims the display at
boot, so flexwm can take it.

## The flexwm loop

Edit on the Mac, build and run in the VM:

```sh
ssh -p 2222 dev@localhost
cd /mnt/flexwm && cargo build          # CARGO_TARGET_DIR is /var/cargo-target,
                                       # i.e. on the guest disk, not over 9p
```

Then run the binary from the QEMU window's tty1 to actually watch it on the
virtual display, or over an ssh session (logind's PAM stack registers those
with a real seat/session too, so `--tty` runs and takes real DRM/libseat
ownership just fine headlessly over ssh — only *watching* it happen needs the
QEMU window). `cargo build` works without `nix develop`: the
wayland/libinput/libxkbcommon/gbm/udev/seatd `.pc` files are in the system
profile and `PKG_CONFIG_PATH` points at it.

## Changing the guest config

Edit `configuration.nix` and either relaunch (`nix run ./vm`, which rebuilds
only the system closure — no image build) or push the change into the running
VM without rebooting:

```sh
NIX_SSHOPTS="-p 2222" nixos-rebuild switch \
  --flake ./vm#flexwm-vm --target-host root@localhost
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

- **Passwords in the config.** Throwaway local VM, not reachable off-host.

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
nix run ./vm </dev/null >/tmp/flexwm-vm.log 2>&1 &
```
This runs fine from an interactive shell and survives independent of it. The
same fix applies to `vm/linux-builder.sh run` if it ever tty-suspends the
same way.
