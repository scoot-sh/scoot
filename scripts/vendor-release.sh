#!/bin/sh
# vendor-release.sh: stage (and tar) a vendored-sources release tree that
# builds with no network, for distro packagers and for the tag-triggered
# release job ([release-artifacts](../docs/backlog/resolved/release-artifacts-done.md),
# which calls this script rather than reimplementing it).
#
# What it produces, in --out (default ./target/vendor-release), where
# <name> is `scoot-<label>-vendored` and the label is --label (the release
# job passes the version: `scoot-0.2.0-vendored.tar.gz`, the name the RPM
# specs' Source0 expects) or, by default, the short commit:
#   <name>/                  the exact HEAD tree (via `git archive`, so the
#                            tarball is reproducible from the commit)
#     vendor/                `cargo vendor` output: every crates.io crate
#                            plus the two scoot-sh git forks (Smithay and
#                            wayland-backend, whose source replacement is the
#                            part most likely to break)
#     .cargo/config.toml     the source-replacement snippet `cargo vendor`
#                            prints: crates-io and both git sources replaced
#                            with the vendored directory
#     THIRD-PARTY-LICENSES   `cargo deny list`: every locked crate and the
#                            license(s) cargo-deny attributes to it
#     OFFLINE-BUILD.txt      how to build from this tree with no network
#   <name>.tar.gz            the tarball, plus its sha256 on stdout
#
# With --check-offline the staging tree is also built
# (`cargo build --workspace --locked --offline` under an empty CARGO_HOME,
# so any network touch fails the run) before it is tarred. The packaging
# CI job runs exactly this, so a green main always has a proven tarball
# path; the release job adds signing and upload.
#
# Needs: git, cargo, cargo-deny, tar (all in `nix develop`).
# Generation needs the network (registry index, git forks, advisory DB is
# NOT consulted here); the tree it produces needs none.
set -eu

OUT=./target/vendor-release
CHECK_OFFLINE=0
LABEL=""

while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT="$2"; shift 2 ;;
        --check-offline) CHECK_OFFLINE=1; shift ;;
        --label) LABEL="$2"; shift 2 ;;
        -h|--help)
            echo "usage: scripts/vendor-release.sh [--out DIR] [--label X.Y.Z] [--check-offline]"
            exit 0 ;;
        *) echo "vendor-release.sh: unknown flag $1" >&2; exit 2 ;;
    esac
done

for tool in git cargo cargo-deny tar; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "vendor-release.sh: need '$tool' on PATH (nix develop has all four)" >&2
        exit 2
    fi
done

if [ -n "$(git status --porcelain)" ]; then
    echo "vendor-release.sh: refusing a dirty tree (the tarball must be \`git archive HEAD\`)" >&2
    exit 2
fi

REV=$(git rev-parse --short HEAD)
# The label lands in a file name: a version or a commit, nothing else.
case "${LABEL:-$REV}" in
    *[!0-9A-Za-z.+-]* | "") echo "vendor-release.sh: bad --label '${LABEL}'" >&2; exit 2 ;;
esac
NAME="scoot-${LABEL:-$REV}-vendored"
STAGE="${OUT}/${NAME}"

rm -rf "$STAGE"
mkdir -p "$STAGE/source" "$OUT"

echo "--- source: git archive HEAD"
git archive HEAD | tar -x -C "$STAGE/source"

echo "--- vendor: cargo vendor"
(cd "$STAGE/source" && cargo vendor vendor >"$STAGE/snippet.toml" 2>"$STAGE/vendor-stderr.log")
if [ ! -s "$STAGE/snippet.toml" ]; then
    echo "vendor-release.sh: 'cargo vendor' printed no snippet (needs a cargo that prints the source-replacement config to stdout)" >&2
    cat "$STAGE/vendor-stderr.log" >&2
    exit 1
fi
mkdir -p "$STAGE/source/.cargo"
cp "$STAGE/snippet.toml" "$STAGE/source/.cargo/config.toml"

echo "--- licenses: cargo deny list"
(cd "$STAGE/source" && cargo deny list >"$STAGE/THIRD-PARTY-LICENSES" 2>"$STAGE/deny-stderr.log")
if [ ! -s "$STAGE/THIRD-PARTY-LICENSES" ]; then
    echo "vendor-release.sh: 'cargo deny list' produced nothing" >&2
    cat "$STAGE/deny-stderr.log" >&2
    exit 1
fi
cp "$STAGE/THIRD-PARTY-LICENSES" "$STAGE/source/THIRD-PARTY-LICENSES"

echo "--- OFFLINE-BUILD.txt"
cat >"$STAGE/source/OFFLINE-BUILD.txt" <<EOF
scoot $REV: offline build from vendored sources
===============================================

This tree builds with no network. Every dependency is in vendor/
(including the two scoot-sh git forks: Smithay and wayland-backend),
and .cargo/config.toml replaces crates-io and both git sources with it.

Prerequisites (not vendored): a Rust toolchain >= 1.88
(workspace rust-version; rustup or a distro toolchain that new),
plus the system libraries in the docs site's packaging page
("package scoot offline": libinput, libseat, libudev, libpixman-1,
libxkbcommon). The GPU scanout tier is compiled in by default and
reaches libgbm at run time (dlopen, never linked), so no libgbm
package is needed to build.

Build (from this directory):

    cargo build --workspace --locked --offline

Features: the default build carries the GBM scanout tier and XWayland
support (needs the Xwayland binary on PATH at run time, not at build
time). --no-default-features is the minimal CPU-only build without
either. THIRD-PARTY-LICENSES lists every
locked dependency and its license (scoot itself is MIT).
EOF

if [ "$CHECK_OFFLINE" -eq 1 ]; then
    echo "--- check: offline build under an empty CARGO_HOME"
    export CARGO_HOME="$STAGE/empty-cargo-home"
    export CARGO_TARGET_DIR="$STAGE/target"
    export CARGO_NET_OFFLINE=true
    mkdir -p "$CARGO_HOME"
    (cd "$STAGE/source" && cargo build --workspace --locked --offline)
    rm -rf "$STAGE/target" "$CARGO_HOME"
    unset CARGO_HOME CARGO_TARGET_DIR CARGO_NET_OFFLINE
    echo "ok: the staged tree builds with no network"
fi

echo "--- tarball"
rm -f "$STAGE/snippet.toml" "$STAGE/vendor-stderr.log" "$STAGE/deny-stderr.log" \
    "$STAGE/THIRD-PARTY-LICENSES"
# Deterministic: sorted names, commit-time mtimes, root ownership, no gzip
# timestamp, so two runs from one commit give identical bytes (GNU tar).
tar --sort=name --mtime="@$(git log -1 --format=%ct HEAD)" --owner=0 --group=0 \
    --numeric-owner --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
    -cf - -C "$STAGE" source | gzip -n > "$OUT/${NAME}.tar.gz"
(cd "$OUT" && shasum -a 256 "${NAME}.tar.gz" 2>/dev/null || sha256sum "${NAME}.tar.gz")
echo "staged: $STAGE/source"
