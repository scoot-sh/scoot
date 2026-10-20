#!/bin/sh
# Install the two Nerd Font families the shipped looks name, for the
# packaging proof containers (deb.yml, rpm.yml, arch.yml).
#
# `scoot look apply vinyl-sunset` refuses a family `fc-match` resolves
# to a fallback (crates/scootctl/src/look/apply.rs): the looks name
# "DroidSansM Nerd Font Propo" and "FiraCode Nerd Font", which no Debian,
# Fedora or Arch package ships, so the smoke test's looks stage fails in
# the proof containers even though the packages are fine. Download the
# pinned nerd-fonts release zips, checksum them, install one file per
# family, and assert `fc-match` resolves both family names.
#
# Usage (as root, inside the container): packaging/install-nerd-fonts.sh
# Installs its own prerequisites (curl, unzip, fontconfig) through the
# distro's package manager. Idempotent: when both families already
# resolve, it prints that and changes nothing.
#
# The proof is the green deb/rpm/arch runs themselves (manual-only
# workflows, dispatched on the release branch); release.yml's lint job
# runs `sh -n` over this file.
set -eu

NERD_VERSION="v3.4.0"
BASE="https://github.com/ryanoasis/nerd-fonts/releases/download/$NERD_VERSION"
DROID_ZIP="DroidSansMono.zip"
DROID_SHA="030229341907c833862f00682f0f4bce65d0dc8d281fc9a40d80b7ee9d306e88"
DROID_FILE="DroidSansMNerdFontPropo-Regular.otf"
DROID_FAMILY="DroidSansM Nerd Font Propo"
FIRA_ZIP="FiraCode.zip"
FIRA_SHA="7cc4ffd8f7a1fc914cdab7b149808298165ff7a7f40e40d82dea9ebe41e8ca0b"
FIRA_FILE="FiraCodeNerdFont-Regular.ttf"
FIRA_FAMILY="FiraCode Nerd Font"
FONT_DIR="/usr/share/fonts/truetype/nerd-fonts"

resolves() {
    # $1 is the family: true when fc-match answers with that family name
    # (not a fallback like DejaVu Sans).
    command -v fc-match >/dev/null 2>&1 \
        && fc-match "$1" 2>/dev/null | grep -q "$1"
}

if resolves "$DROID_FAMILY" && resolves "$FIRA_FAMILY"; then
    echo "nerd fonts already installed: both look families resolve"
    exit 0
fi

if command -v apt-get >/dev/null 2>&1; then
    apt-get update -qq
    apt-get install -y -qq curl ca-certificates unzip fontconfig
elif command -v dnf >/dev/null 2>&1; then
    dnf install -y -q curl unzip fontconfig
elif command -v pacman >/dev/null 2>&1; then
    pacman -S --noconfirm --needed curl unzip fontconfig
else
    echo "error: no apt-get, dnf or pacman: install curl, unzip and fontconfig first" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
curl -fsSL --max-time 120 -o "$tmp/$DROID_ZIP" "$BASE/$DROID_ZIP"
curl -fsSL --max-time 120 -o "$tmp/$FIRA_ZIP" "$BASE/$FIRA_ZIP"
printf '%s  %s\n' "$DROID_SHA" "$tmp/$DROID_ZIP" | sha256sum -c -
printf '%s  %s\n' "$FIRA_SHA" "$tmp/$FIRA_ZIP" | sha256sum -c -
mkdir -p "$FONT_DIR"
unzip -j -o "$tmp/$DROID_ZIP" "$DROID_FILE" -d "$FONT_DIR"
unzip -j -o "$tmp/$FIRA_ZIP" "$FIRA_FILE" -d "$FONT_DIR"
fc-cache -f >/dev/null

for fam in "$DROID_FAMILY" "$FIRA_FAMILY"; do
    if ! resolves "$fam"; then
        echo "error: font family '$fam' still does not resolve after install:" >&2
        fc-match "$fam" >&2 || true
        exit 1
    fi
done
echo "nerd fonts installed: both look families resolve"
