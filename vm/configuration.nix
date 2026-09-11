# Guest configuration for the flexwm development VM.
#
# One job: hand a Rust/Wayland compositor a real DRM/KMS device, real libinput
# devices and a seat to sit on -- inside a VM that boots on macOS under QEMU/HVF.
#
# This imports NixOS's own qemu-vm module. With `virtualisation.host.pkgs` set
# to darwin pkgs (see flake.nix), `system.build.vm` is a *native Mac* launcher:
# it packs the store into an erofs image on the host at launch and boots the
# kernel directly. No bootloader, no disk image build, no nested VM.
{ config, lib, pkgs, modulesPath, ... }:

let
  # gbm moved out of the mesa attribute in newer nixpkgs; accept either.
  gbm = pkgs.libgbm or pkgs.mesa;

  # Libraries a Smithay/wlroots-style compositor links against. Their `.dev`
  # outputs are installed too, so a plain `cargo build` finds the .pc files via
  # PKG_CONFIG_PATH below -- no `nix develop` needed inside the VM.
  compositorDeps = [
    pkgs.wayland
    pkgs.wayland-protocols
    pkgs.wayland-scanner
    pkgs.libxkbcommon
    pkgs.libinput
    pkgs.libdrm
    pkgs.libdisplay-info
    pkgs.seatd
    pkgs.systemdLibs # libudev
    pkgs.pixman
    pkgs.libglvnd # EGL/GLES
    pkgs.dbus
    gbm
  ];
in
{
  imports = [ "${modulesPath}/virtualisation/qemu-vm.nix" ];

  # ------------------------------------------------------------------ vm ---
  virtualisation = {
    # Pack the store into erofs on the Mac at launch instead of building a
    # disk image on a Linux builder. Rebuilding after a config change is then
    # just the system closure, not a 9G image.
    useNixStoreImage = true;
    # ...but keep the store writable (overlay on the VM's own disk) so
    # `nix develop` and friends work inside the VM, and survive a reboot.
    writableStore = true;
    writableStoreUseTmpfs = false;

    diskSize = 16 * 1024; # writable root; cargo target dirs live here
    memorySize = 4096;
    cores = 4;
    graphics = true;

    forwardPorts = [
      {
        from = "host";
        host.port = 2222;
        guest.port = 22;
      }
    ];

    # $FLEXWM_SRC is expanded by the launcher at runtime, not at build time.
    sharedDirectories.flexwm = {
      source = "\"$FLEXWM_SRC\"";
      target = "/mnt/flexwm";
    };

    qemu.options = [
      # aarch64 `virt` has no default display adapter, and this is also the DRM
      # device flexwm will drive. Deliberately no virgl: Cocoa has no host GL
      # passthrough, so Mesa lands on llvmpipe. KMS and input are still real.
      "-device virtio-gpu-pci,xres=1600,yres=1000"
      # Boot log and a root shell in the terminal you launched from, alongside
      # the graphical console in the QEMU window.
      "-serial mon:stdio"
    ];
  };

  # ------------------------------------------------------------- graphics ---
  hardware.graphics.enable = true;

  # seatd hands a non-root compositor DRM master and input devices.
  # (logind can do the same; both are present, libseat picks one.)
  services.seatd.enable = true;

  # Nothing owns the display by default -- flexwm is meant to. sway and cage are
  # installed below purely as an "is the stack alive?" reference.
  services.xserver.enable = false;

  # ------------------------------------------------------------- accounts ---
  users.mutableUsers = false;
  users.users.dev = {
    isNormalUser = true;
    description = "flexwm developer";
    password = "dev";
    extraGroups = [ "wheel" "video" "render" "input" "seat" ];
    openssh.authorizedKeys.keyFiles = lib.optional (builtins.pathExists ./authorized_keys) ./authorized_keys;
  };
  users.users.root = {
    password = "root";
    openssh.authorizedKeys.keyFiles = lib.optional (builtins.pathExists ./authorized_keys) ./authorized_keys;
  };
  security.sudo.wheelNeedsPassword = false;

  # Log straight into a VT in the QEMU window so a compositor can be started.
  services.getty.autologinUser = "dev";

  users.motd = ''
    flexwm dev VM -- the Mac checkout is at /mnt/flexwm (9p, edit on the host)

      kmscube                 DRM/KMS + GLES smoke test (Ctrl-C quits)
      drm_info                what virtio-gpu exposes
      sudo libinput list-devices
      sway                    reference compositor; WLR_RENDERER=pixman is quicker here
      cd /mnt/flexwm && cargo build      (CARGO_TARGET_DIR is already off 9p)

    ssh dev@localhost -p 2222 from the Mac.
  '';

  # ------------------------------------------------------------- services ---
  services.openssh = {
    enable = true;
    settings.PermitRootLogin = "yes";
    settings.PasswordAuthentication = true; # throwaway local VM
  };

  networking.hostName = "flexwm-vm";
  networking.firewall.enable = false;
  time.timeZone = "UTC";

  nix.settings = {
    experimental-features = [ "nix-command" "flakes" ];
    trusted-users = [ "root" "dev" ];
  };

  # ---------------------------------------------------------- dev toolbox ---
  environment.systemPackages =
    (with pkgs; [
      # basics
      git vim htop tmux curl wget file tree ripgrep fd jq pciutils usbutils

      # wayland / drm smoke tests
      kmscube drm_info libinput wayland-utils wev wlr-randr
      sway cage foot

      # rust
      rustc cargo clippy rustfmt gcc gnumake pkg-config
    ])
    ++ compositorDeps
    ++ map (p: p.dev) (lib.filter (p: p ? dev) compositorDeps);

  # Make the system profile a usable pkg-config prefix, so `cargo build` works
  # in the VM without a dev shell.
  environment.pathsToLink = [ "/lib/pkgconfig" "/share/pkgconfig" ];
  environment.variables = {
    PKG_CONFIG_PATH = "/run/current-system/sw/lib/pkgconfig:/run/current-system/sw/share/pkgconfig";
    EDITOR = "vim";
    # Keep build artifacts on the guest disk instead of writing them back
    # through 9p, which would be painfully slow.
    CARGO_TARGET_DIR = "/var/cargo-target";
  };
  systemd.tmpfiles.rules = [ "d /var/cargo-target 1777 root root -" ];

  # Every megabyte here is repacked into the erofs image on each launch.
  documentation.nixos.enable = false;
  documentation.man.enable = lib.mkDefault false;

  # Disposable VM: always track the nixpkgs it was built from.
  system.stateVersion = lib.trivial.release;
}
