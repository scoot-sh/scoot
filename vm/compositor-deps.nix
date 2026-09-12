# The libraries a Smithay compositor links against. One list, used by both the
# VM's system profile (configuration.nix) and the dev shell (../flake.nix), so
# the two cannot drift apart. Order matters for the VM: changing it changes
# the system closure.
pkgs:
[
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
  # gbm moved out of the mesa attribute in newer nixpkgs; accept either.
  (pkgs.libgbm or pkgs.mesa)
]
