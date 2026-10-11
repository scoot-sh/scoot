#!/bin/sh
# build-deb.sh: build the three scoot .debs from a source tree.
#
# Usage: packaging/deb/build-deb.sh --source DIR --out DIR [--revision N]
#
# --source is a clean checkout or a `scripts/vendor-release.sh` staged
# `source/` tree (the same layout the release tarball unpacks to, so
# after the first release this builds the published tarball with the
# same command). The build runs `cargo build --release --locked
# --offline`, so the tree must be vendored (or the registry warm);
# `CARGO_NET_OFFLINE=true` is exported to make any network touch fail.
# --out receives `scoot_<ver>_<arch>.deb` and friends.
#
# Per package the script: builds the crate, derives Depends from
# `dpkg-shlibdeps -O` over the staged binaries (checked against the
# ldd-derived allow-list in deb-meta.py -- a new or renamed system
# library fails here), renders DEBIAN/control from the crate manifests
# (no version is remembered anywhere), stages the installed files (the
# same set as packaging/arch: the session entry, the session user
# units, the portal selection, the bar's user unit,
# licenses including the deny-generated inventory, no writes to a user
# config), and runs `dpkg-deb --build`.
#
# Needs: cargo (a toolchain past the workspace rust-version floor --
# bookworm's own rustc is too old, so the container installs the pinned
# rustup toolchain, the same one release.yml uses), dpkg-dev
# (dpkg-shlibdeps), dpkg-deb, python3. THIRD-PARTY-LICENSES comes from
# the source tree when staged (vendor-release.sh generates it) and from
# `cargo deny list` otherwise.
set -eu

SOURCE=""
OUT=""
REVISION="1"

while [ $# -gt 0 ]; do
    case "$1" in
        --source) SOURCE="$2"; shift 2 ;;
        --out) OUT="$2"; shift 2 ;;
        --revision) REVISION="$2"; shift 2 ;;
        -h|--help)
            echo "usage: packaging/deb/build-deb.sh --source DIR --out DIR [--revision N]"
            exit 0 ;;
        *) echo "build-deb.sh: unknown flag $1" >&2; exit 2 ;;
    esac
done

if [ -z "$SOURCE" ] || [ -z "$OUT" ]; then
    echo "build-deb.sh: --source and --out are required" >&2
    exit 2
fi

for tool in cargo dpkg-deb dpkg-shlibdeps python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "build-deb.sh: need '$tool' on PATH" >&2
        exit 2
    fi
done

HERE=$(dirname "$0")
META="$HERE/deb-meta.py"

if command -v dpkg >/dev/null 2>&1; then
    ARCH=$(dpkg --print-architecture)
else
    case "$(uname -m)" in
        x86_64) ARCH=amd64 ;;
        aarch64) ARCH=arm64 ;;
        *) echo "build-deb.sh: unknown architecture $(uname -m)" >&2; exit 2 ;;
    esac
fi
echo "--- architecture: $ARCH"

if [ -f "$SOURCE/THIRD-PARTY-LICENSES" ]; then
    echo "--- THIRD-PARTY-LICENSES: from the staged source tree"
else
    echo "--- THIRD-PARTY-LICENSES: cargo deny list"
    (cd "$SOURCE" && cargo deny list >THIRD-PARTY-LICENSES)
fi

SCOOTBG_VERSION=$(python3 "$META" versions --source "$SOURCE" | sed -n 's/^scootbg //p')
echo "--- versions: scoot/scootbg $SCOOTBG_VERSION (lockstep), scootbar $(
    python3 "$META" versions --source "$SOURCE" | sed -n 's/^scootbar //p')"

export CARGO_NET_OFFLINE=true
STAGE_ROOT="$OUT/stage"
rm -rf "$STAGE_ROOT"
mkdir -p "$OUT" "$STAGE_ROOT"
# Honor an outside CARGO_TARGET_DIR (CI sets one); default to the
# source tree's own target/ otherwise.
BINDIR="${CARGO_TARGET_DIR:-$SOURCE/target}/release"

for pkg in scoot scootbg scootbar; do
    echo "--- $pkg: cargo build --release --locked --offline -p $pkg"
    (cd "$SOURCE" && cargo build --release --locked --offline -p "$pkg")

    STAGE="$STAGE_ROOT/$pkg"
    mkdir -p "$STAGE/DEBIAN" "$STAGE/usr/bin" "$STAGE/usr/share/doc/$pkg"

    install -m755 "$BINDIR/$pkg" "$STAGE/usr/bin/$pkg"
    install -m644 "$SOURCE/THIRD-PARTY-LICENSES" "$STAGE/usr/share/doc/$pkg/THIRD-PARTY-LICENSES"
    install -m644 "$SOURCE/NOTICE" "$STAGE/usr/share/doc/$pkg/NOTICE"

    case "$pkg" in
        scoot)
            install -m755 "$SOURCE/resources/scoot-session" "$STAGE/usr/bin/scoot-session"
            install -Dm644 "$HERE/scoot.desktop" "$STAGE/usr/share/wayland-sessions/scoot.desktop"
            # The session units scoot-session starts (single-sourced from
            # resources/systemd/user/, the same files the NixOS module
            # installs): the service names the packaged binary, the two
            # targets name no binary. Plus the portal selection for the
            # vendor slot (see resources/scoot-portals.conf).
            install -Dm644 "$SOURCE/resources/systemd/user/scoot-session.target" "$STAGE/usr/lib/systemd/user/scoot-session.target"
            install -Dm644 "$SOURCE/resources/systemd/user/scoot-shutdown.target" "$STAGE/usr/lib/systemd/user/scoot-shutdown.target"
            sed 's|@SCOOT_BIN@|/usr/bin/scoot|g' "$SOURCE/resources/systemd/user/scoot.service" >"$STAGE/usr/lib/systemd/user/scoot.service"
            chmod 644 "$STAGE/usr/lib/systemd/user/scoot.service"
            install -Dm644 "$SOURCE/resources/scoot-portals.conf" "$STAGE/usr/share/xdg-desktop-portal/scoot-portals.conf"
            "$BINDIR/scoot" --print-default-config >"$STAGE/usr/share/doc/scoot/config.toml.example"
            install -m644 "$HERE/README.Debian.scoot" "$STAGE/usr/share/doc/scoot/README.Debian"
            ;;
        scootbar)
            install -Dm644 "$HERE/scootbar.service" "$STAGE/usr/lib/systemd/user/scootbar.service"
            install -m644 "$HERE/bar.toml.example" "$STAGE/usr/share/doc/scootbar/bar.toml.example"
            install -m644 "$HERE/README.Debian.scootbar" "$STAGE/usr/share/doc/scootbar/README.Debian"
            ;;
    esac

    # Derived, not remembered: shlibdeps versions, allow-list checked.
    # Only the binary: scoot-session is a shell script, and passing it
    # would only add a warning (it stays covered by inspection -- it
    # execs the compositor or degrades to it). dpkg-shlibdeps insists
    # on reading debian/control, so it gets a stub in the stage (real
    # metadata ships only in DEBIAN/control); the stub is removed
    # before dpkg-deb runs, never packaged.
    mkdir -p "$STAGE/debian"
    printf 'Source: %s\n\nPackage: %s\nArchitecture: %s\nDescription: shlibdeps stub\n' \
        "$pkg" "$pkg" "$ARCH" >"$STAGE/debian/control"
    if ! SHLIBS=$(cd "$STAGE" && dpkg-shlibdeps -O "usr/bin/$pkg" 2>"$OUT/shlibdeps-$pkg.log"); then
        echo "build-deb.sh: dpkg-shlibdeps failed for $pkg (log below)" >&2
        cat "$OUT/shlibdeps-$pkg.log" >&2
        exit 1
    fi
    rm -rf "$STAGE/debian"
    DEPENDS=$(python3 "$META" check-shlibdeps --package "$pkg" "$SHLIBS")
    echo "--- $pkg: Depends: $DEPENDS"

    python3 "$META" control --package "$pkg" --source "$SOURCE" \
        --arch "$ARCH" --depends "$DEPENDS" --revision "$REVISION" \
        --scootbg-version "$SCOOTBG_VERSION-$REVISION" \
        >"$STAGE/DEBIAN/control"

    # Non-native packages (our versions carry a Debian revision) must
    # ship changelog.Debian.gz or lintian fails with no-changelog. The
    # entry points at the per-package release notes (the user-visible
    # history) and the license inventory, which is where this tree's
    # history lives -- there is no Debian revision history to recount.
    # Both names come from the rendered control, so they cannot skew.
    UPSTREAM=$(sed -n 's/^Version: //p' "$STAGE/DEBIAN/control" | sed "s/-$REVISION\$//")
    CONTACT=$(sed -n 's/^Maintainer: //p' "$STAGE/DEBIAN/control")

    # DEP-5 machine-readable copyright (Format:/Upstream-/Files:/License:
    # stanzas). One stanza covers every shipped file: all of them are
    # upstream MIT. Third-party terms attach to code compiled *into* the
    # binaries, not to shipped files, so per-crate Files: stanzas would
    # describe sources that are not in the package; the inventory plus
    # NOTICE carry those terms instead, and the Comment says so. (DEP-5
    # is written for source packages; in a binary .deb the machine
    # reader gets the header and the human reader the Comment.)
    HOLDER=$(sed -n 's/^Copyright (c) //p' "$SOURCE/LICENSE" | head -1)
    {
        echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
        echo "Upstream-Name: scoot"
        echo "Upstream-Contact: $CONTACT"
        echo "Source: https://github.com/scoot-sh/scoot"
        echo
        echo "Files: *"
        echo "Copyright: $HOLDER"
        echo "License: MIT"
        sed -e 's/^/ /' -e 's/^ $/ ./' "$SOURCE/LICENSE"
        echo " ."
        echo " Comment: scoot itself is MIT (above). Every third-party crate"
        echo "  compiled into the binaries is listed with its license in"
        echo "  THIRD-PARTY-LICENSES (generated by 'cargo deny list' at"
        echo "  packaging time; the allow-list is enforced by"
        echo "  'cargo deny check' in CI); the full texts the inventory names"
        echo "  (Smithay's MIT and others) are in NOTICE and the vendored"
        echo "  sources."
    } >"$STAGE/usr/share/doc/$pkg/copyright"
    {
        echo "$pkg ($UPSTREAM-$REVISION) stable; urgency=medium"
        echo
        echo "  * Upstream release $UPSTREAM."
        echo "    User-visible changes are in the release notes at"
        echo "    https://github.com/scoot-sh/scoot/releases; every third-party"
        echo "    crate in the build is listed in"
        echo "    /usr/share/doc/$pkg/THIRD-PARTY-LICENSES."
        echo
        echo " -- $CONTACT  $(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S %z')"
    } >"$STAGE/usr/share/doc/$pkg/changelog.Debian"
    gzip -9n "$STAGE/usr/share/doc/$pkg/changelog.Debian"

    VER=$(sed -n 's/^Version: //p' "$STAGE/DEBIAN/control")
    # md5sums is a debhelper artifact dpkg-deb --build does not write;
    # without it lintian reports no-md5sums-control-file and debsums
    # has nothing to verify. Paths are stage-relative, sorted.
    (cd "$STAGE" && find usr -type f | LC_ALL=C sort | xargs md5sum >DEBIAN/md5sums)
    dpkg-deb --build "$STAGE" "$OUT/${pkg}_${VER}_${ARCH}.deb"
    ls -l "$OUT/${pkg}_${VER}_${ARCH}.deb"
done

echo "--- built:"
ls -l "$OUT"/*.deb
