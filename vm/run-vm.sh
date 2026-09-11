# Boot the flexwm dev VM. The heavy lifting is NixOS's generated launcher
# (config.system.build.vm, a native Mac script); this only decides where state
# lives and which directory gets shared into the guest.
#
# Not run directly -- flake.nix wraps it:
#
#   nix run ./vm                  boot it
#   nix run ./vm -- --reset       throw the VM's disk away and start clean
#   nix run ./vm -- -snapshot     anything else is passed straight to qemu
#
# Knobs: FLEXWM_SRC, FLEXWM_VM_STATE_DIR.

: "${FLEXWM_VM_RUNNER:?set by flake.nix}"

# Shared into the VM at /mnt/flexwm. Defaults to wherever you invoked nix run
# from, which is normally the repo root.
FLEXWM_SRC="${FLEXWM_SRC:-$PWD}"
export FLEXWM_SRC

STATE_DIR="${FLEXWM_VM_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/flexwm-vm}"

# The generated runner honours NIX_DISK_IMAGE; without it the disk lands in
# whatever directory you happened to be standing in.
NIX_DISK_IMAGE="$STATE_DIR/flexwm-vm.qcow2"
export NIX_DISK_IMAGE

if [ "${1:-}" = "--reset" ]; then
    echo "flexwm-vm: discarding $NIX_DISK_IMAGE"
    rm -f "$NIX_DISK_IMAGE"
    shift
fi

mkdir -p "$STATE_DIR"

echo "flexwm-vm: sharing $FLEXWM_SRC at /mnt/flexwm"
echo "flexwm-vm: ssh -p 2222 dev@localhost   (password: dev)"
echo "flexwm-vm: packing the store image, this takes a moment on each boot..."

exec "$FLEXWM_VM_RUNNER" "$@"
