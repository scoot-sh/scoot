# Boot the scoot dev VM. The heavy lifting is NixOS's generated launcher
# (config.system.build.vm, a native Mac script); this only decides where state
# lives and which directory gets shared into the guest.
#
# Not run directly -- flake.nix wraps it:
#
#   nix run ./vm                  boot it
#   nix run ./vm -- --reset       throw the VM's disk away and start clean
#   nix run ./vm -- -snapshot     anything else is passed straight to qemu
#
# Knobs: SCOOT_SRC, SCOOT_VM_STATE_DIR.

: "${SCOOT_VM_RUNNER:?set by flake.nix}"

# Shared into the VM at /mnt/scoot. Defaults to wherever you invoked nix run
# from, which is normally the repo root.
SCOOT_SRC="${SCOOT_SRC:-$PWD}"
export SCOOT_SRC

STATE_DIR="${SCOOT_VM_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/scoot-vm}"

# The generated runner honours NIX_DISK_IMAGE; without it the disk lands in
# whatever directory you happened to be standing in.
NIX_DISK_IMAGE="$STATE_DIR/scoot-vm.qcow2"
export NIX_DISK_IMAGE

if [ "${1:-}" = "--reset" ]; then
    echo "scoot-vm: discarding $NIX_DISK_IMAGE"
    rm -f "$NIX_DISK_IMAGE"
    shift
fi

mkdir -p "$STATE_DIR"

echo "scoot-vm: sharing $SCOOT_SRC at /mnt/scoot"
echo "scoot-vm: ssh -p 2222 dev@localhost   (password: dev)"
echo "scoot-vm: packing the store image, this takes a moment on each boot..."

exec "$SCOOT_VM_RUNNER" "$@"
