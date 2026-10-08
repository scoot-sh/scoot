#!/bin/sh
# make-repo-index.sh: build the unsigned apt metadata for a directory of .debs.
#
# Usage: packaging/deb/make-repo-index.sh --debs DIR --out DIR \
#          --codename bookworm --component main
#
# Writes OUT/dists/<codename>/<component>/binary-<arch>/Packages{,.gz}
# plus OUT/dists/<codename>/Release (unsigned). Signing that Release
# into InRelease is the maintainer's step (packaging/deb/README.md):
# this script mints no key and signs nothing, so CI can run it without
# any secret. Needs: dpkg-scanpackages (dpkg-dev), gzip.
set -eu

DEBS=""
OUT=""
CODENAME=""
COMPONENT="main"

while [ $# -gt 0 ]; do
    case "$1" in
        --debs) DEBS="$2"; shift 2 ;;
        --out) OUT="$2"; shift 2 ;;
        --codename) CODENAME="$2"; shift 2 ;;
        --component) COMPONENT="$2"; shift 2 ;;
        -h|--help)
            echo "usage: packaging/deb/make-repo-index.sh --debs DIR --out DIR --codename NAME [--component main]"
            exit 0 ;;
        *) echo "make-repo-index.sh: unknown flag $1" >&2; exit 2 ;;
    esac
done

if [ -z "$DEBS" ] || [ -z "$OUT" ] || [ -z "$CODENAME" ]; then
    echo "make-repo-index.sh: --debs, --out and --codename are required" >&2
    exit 2
fi
if ! command -v dpkg-scanpackages >/dev/null 2>&1; then
    echo "make-repo-index.sh: need 'dpkg-scanpackages' on PATH (dpkg-dev)" >&2
    exit 2
fi

POOL="$OUT/pool/$COMPONENT"
mkdir -p "$POOL"
cp -a "$DEBS"/*.deb "$POOL/"

# One Packages file per architecture present (dpkg-scanpackages --arch).
ARCHES=$(dpkg-deb -f "$POOL"/*.deb Architecture 2>/dev/null | sort -u || true)
if [ -z "$ARCHES" ]; then
    echo "make-repo-index.sh: no .debs in $DEBS" >&2
    exit 1
fi
DISTDIR="$OUT/dists/$CODENAME"
for arch in $ARCHES; do
    BINDIR="$DISTDIR/$COMPONENT/binary-$arch"
    mkdir -p "$BINDIR"
    (cd "$OUT" && dpkg-scanpackages --arch "$arch" "pool/$COMPONENT" >"$BINDIR/Packages")
    gzip -9 -k -f "$BINDIR/Packages"
done

{
    echo "Origin: scoot-sh"
    echo "Label: scoot"
    echo "Codename: $CODENAME"
    echo "Date: $(date -u '+%a, %d %b %Y %H:%M:%S UTC')"
    echo "Architectures: $(printf '%s' "$ARCHES" | tr '\n' ' ')"
    echo "Components: $COMPONENT"
    echo "Description: scoot Wayland compositor packages"
} >"$DISTDIR/Release"
(cd "$DISTDIR" && sha256sum -- $COMPONENT/binary-*/Packages* >>Release)

echo "--- unsigned index for $CODENAME:"
find "$DISTDIR" -type f | sort
echo "NOTE: Release is unsigned. The maintainer signs it into InRelease"
echo "(see packaging/deb/README.md); until then this repo installs only"
echo "with Signed-By pointing at the published key or [trusted=yes]."
