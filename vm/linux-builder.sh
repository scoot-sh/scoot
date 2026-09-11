#!/usr/bin/env bash
#
# macOS cannot build Linux derivations, and the flexwm VM image is one. This
# manages the standard nixpkgs Linux builder VM (a small headless NixOS VM that
# the nix daemon delegates aarch64-linux builds to).
#
#   ./vm/linux-builder.sh setup    one-time: keys, /etc/nix/machines, ssh config (uses sudo)
#   ./vm/linux-builder.sh run      start it; leave this running while you build
#   ./vm/linux-builder.sh status   check that the daemon can actually reach it
#
# The builder VM's own config is prebuilt upstream, so `run` substitutes it
# rather than needing a Linux builder to build the Linux builder.
set -euo pipefail

FLAKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_DIR="${LINUX_BUILDER_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/nix-linux-builder}"
KEYS="$STATE_DIR/keys"

# Well-known host key of the builder VM, from nixpkgs
# (nixos/modules/profiles/keys/ssh_host_ed25519_key.pub), base64 encoded.
HOST_KEY_B64="c3NoLWVkMjU1MTkgQUFBQUMzTnphQzFsWkRJMU5URTVBQUFBSUpCV2N4Yi9CbGFxdDFhdU90RStGOFFVV3JVb3RpQzVxQkorVXVFV2RWQ2Igcm9vdEBuaXhvcwo="
MACHINE_LINE="ssh-ng://builder@linux-builder aarch64-linux /etc/nix/builder_ed25519 4 1 kvm,benchmark,big-parallel - $HOST_KEY_B64"

cmd_setup() {
    mkdir -p "$KEYS"
    if [ ! -e "$KEYS/builder_ed25519" ]; then
        echo "==> generating $KEYS/builder_ed25519"
        ssh-keygen -q -t ed25519 -N "" -C builder@localhost -f "$KEYS/builder_ed25519"
    fi

    # The nix daemon runs as root, so its copy of the key lives in /etc/nix.
    echo "==> installing the builder key into /etc/nix (sudo)"
    sudo install -g nixbld -m 600 "$KEYS/builder_ed25519" /etc/nix/builder_ed25519
    sudo install -g nixbld -m 644 "$KEYS/builder_ed25519.pub" /etc/nix/builder_ed25519.pub

    echo "==> teaching ssh about the linux-builder alias"
    sudo mkdir -p /etc/ssh/ssh_config.d
    sudo tee /etc/ssh/ssh_config.d/100-linux-builder.conf >/dev/null <<'EOF'
Host linux-builder
  Hostname localhost
  HostKeyAlias linux-builder
  Port 31022
  User builder
  IdentityFile /etc/nix/builder_ed25519
  IdentitiesOnly yes
  StrictHostKeyChecking accept-new
EOF

    # /etc/nix/nix.conf already points `builders` at this file; append rather
    # than overwrite so any builders you already have keep working.
    if [ -e /etc/nix/machines ] && grep -q 'linux-builder' /etc/nix/machines; then
        echo "==> /etc/nix/machines already lists linux-builder, leaving it alone"
    else
        echo "==> adding linux-builder to /etc/nix/machines"
        printf '%s\n' "$MACHINE_LINE" | sudo tee -a /etc/nix/machines >/dev/null
    fi

    # Determinate Nix rewrites /etc/nix/nix.conf; user settings go next door.
    #  - http2 = false: parallel downloads freeze under HTTP/2 (NixOS/nix#3438)
    #  - stalled-download-timeout = 20: a freeze costs 20s, not the 300s default
    #  - builders-use-substitutes = false: the Mac, which has the two fixes
    #    above, does all the downloading and pushes to the builder over ssh.
    #    The builder runs stock settings, and changing them means rebuilding
    #    its guest closure -- which needs a builder. The Mac also needs the
    #    guest closure locally anyway to pack the VM's store image.
    for setting in 'http2 = false' 'stalled-download-timeout = 20' 'builders-use-substitutes = false'; do
        key="${setting%% =*}"
        if ! grep -q "^$key" /etc/nix/nix.custom.conf 2>/dev/null; then
            echo "==> $setting"
            echo "$setting" | sudo tee -a /etc/nix/nix.custom.conf >/dev/null
        fi
    done

    echo "==> restarting the nix daemon"
    sudo launchctl kickstart -k system/systems.determinate.nix-daemon

    echo
    echo "setup done. Start the builder with: $0 run"
}

cmd_run() {
    if [ ! -e "$KEYS/builder_ed25519" ]; then
        echo "no keys yet -- run '$0 setup' first" >&2
        exit 1
    fi
    mkdir -p "$STATE_DIR"
    cd "$STATE_DIR"
    echo "==> builder state in $STATE_DIR (ssh on localhost:31022)"
    echo "==> leave this running; Ctrl-a x or 'sudo poweroff' inside stops it"
    KEYS="$KEYS" exec nix run "$FLAKE_DIR#linux-builder"
}

cmd_status() {
    if nc -z localhost 31022 2>/dev/null; then
        echo "builder VM: listening on localhost:31022"
    else
        echo "builder VM: not running ($0 run)"
        exit 1
    fi
    echo "==> asking the nix daemon to run a trivial aarch64-linux build"
    # shellcheck disable=SC2016  # $out is a Nix expression, not shell
    nix build --no-link --impure --expr \
        'derivation { name = "linux-builder-smoke-test"; system = "aarch64-linux"; builder = "/bin/sh"; args = [ "-c" "echo ok > $out" ]; }'
    echo "builder VM: reachable from the nix daemon"
}

case "${1:-run}" in
    setup) cmd_setup ;;
    run) cmd_run ;;
    status) cmd_status ;;
    *)
        echo "usage: $0 [setup|run|status]" >&2
        exit 1
        ;;
esac
