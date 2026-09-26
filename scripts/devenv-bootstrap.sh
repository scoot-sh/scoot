#!/usr/bin/env bash
# Installs Nix and devenv on a fresh Linux box that has neither -- a Claude
# Code on the web container, or a throwaway VM -- so `devenv shell`
# works from this repo. Idempotent: rerunning skips whatever is already there.
#
#   scripts/devenv-bootstrap.sh
#   devenv shell -- cargo build --workspace
#
# Single-user Nix, no daemon: containers rarely run an init system, and a
# disposable box gains nothing from a multi-user install. Running as root
# (the usual container user) needs `build-users-group` emptied, since there
# is no `nixbld` group -- that is the one setting the stock installer trips
# on. Not meant for a workstation that already has Nix: install devenv there
# the way https://devenv.sh/getting-started/ says.
set -euo pipefail

NIX_BIN=/nix/var/nix/profiles/default/bin
[ -x "$HOME/.nix-profile/bin/nix" ] && NIX_BIN="$HOME/.nix-profile/bin"

if ! command -v nix >/dev/null 2>&1 && [ ! -x "$NIX_BIN/nix" ]; then
    if [ "$(id -u)" -eq 0 ] && [ ! -e /etc/nix/nix.conf ]; then
        mkdir -p /etc/nix
        cat >/etc/nix/nix.conf <<'EOF'
build-users-group =
experimental-features = nix-command flakes
sandbox = false
trusted-users = root
extra-substituters = https://devenv.cachix.org
extra-trusted-public-keys = devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=
EOF
    fi
    installer=$(mktemp)
    curl -fsSL https://nixos.org/nix/install -o "$installer"
    sh "$installer" --no-daemon --yes
    rm -f "$installer"
    NIX_BIN="$HOME/.nix-profile/bin"
fi

export PATH="$NIX_BIN:$HOME/.nix-profile/bin:$PATH"

if ! command -v devenv >/dev/null 2>&1; then
    nix --extra-experimental-features "nix-command flakes" profile add nixpkgs#devenv \
        || nix --extra-experimental-features "nix-command flakes" profile install nixpkgs#devenv
fi

# Put both on the default PATH so a non-login shell (an agent's tool shell)
# finds them without sourcing the Nix profile script. Link wherever each one
# actually resolved, and only if it did, so an install elsewhere (multi-user
# Nix, a devenv from another profile) never becomes a dangling link.
if [ -w /usr/local/bin ]; then
    for tool in nix devenv; do
        target=$(command -v "$tool" || true)
        case "$target" in
            /usr/local/bin/*|"") ;;
            *) [ -x "$target" ] && ln -sf "$target" "/usr/local/bin/$tool" ;;
        esac
    done
fi

nix --version
devenv version
