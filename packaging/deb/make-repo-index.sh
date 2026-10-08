#!/bin/sh
# make-repo-index.sh: build the unsigned apt metadata for a directory of .debs.
#
# Usage: packaging/deb/make-repo-index.sh --debs DIR --out DIR \
#          --codename bookworm --component main
#
# Writes OUT/dists/<codename>/<component>/binary-<arch>/Packages{,.gz}
# plus OUT/dists/<codename>/Release (unsigned, with real MD5Sum/SHA1/
# SHA256 stanzas -- hash plus size plus path, which is what apt parses;
# a bare `sha256sum` listing is not a Release). Signing that Release
# into InRelease is the maintainer's step
# (packaging/deb/README.md): this script mints no key and signs
# nothing, so CI can run it without any secret. Needs:
# dpkg-scanpackages (dpkg-dev) and gzip; md5sum/sha1sum/sha256sum/stat
# come with coreutils. (apt-ftparchive would write the same file, but
# bookworm-slim does not ship it, so the stanza is rendered here.)
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
for tool in dpkg-scanpackages gzip; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "make-repo-index.sh: need '$tool' on PATH" >&2
        exit 2
    fi
done

POOL="$OUT/pool/$COMPONENT"
mkdir -p "$POOL"
cp -a "$DEBS"/*.deb "$POOL/"

# One Packages file per architecture present (dpkg-scanpackages --arch).
# Per-file: dpkg-deb -f over several archives at once does not print
# one value per line.
DISTDIR="$OUT/dists/$CODENAME"
ARCHES=$(for d in "$POOL"/*.deb; do dpkg-deb -f "$d" Architecture; done | sort -u)
if [ -z "$ARCHES" ]; then
    echo "make-repo-index.sh: no .debs in $DEBS" >&2
    exit 1
fi
for arch in $ARCHES; do
    BINDIR="$DISTDIR/$COMPONENT/binary-$arch"
    mkdir -p "$BINDIR"
    (cd "$OUT" && dpkg-scanpackages --arch "$arch" "pool/$COMPONENT" >"$BINDIR/Packages")
    gzip -9 -k -f "$BINDIR/Packages"
done

{
    echo "Origin: scoot-sh"
    echo "Label: scoot"
    echo "Suite: $CODENAME"
    echo "Codename: $CODENAME"
    echo "Date: $(date -u '+%a, %d %b %Y %H:%M:%S UTC')"
    echo "Architectures: $(printf '%s' "$ARCHES" | tr '\n' ' ' | sed 's/ $//')"
    echo "Components: $COMPONENT"
    echo "Description: scoot Wayland compositor packages"
    for algo in MD5Sum SHA1 SHA256; do
        echo "$algo:"
        (cd "$DISTDIR" && find "$COMPONENT" -type f | sort | while read -r f; do
            case "$algo" in
                MD5Sum) h=$(md5sum "$f") ;;
                SHA1) h=$(sha1sum "$f") ;;
                SHA256) h=$(sha256sum "$f") ;;
            esac
            set -- $h
            printf ' %s %s %s\n' "$1" "$(stat -c '%s' "$f")" "$f"
        done)
    done
} >"$DISTDIR/Release"

echo "--- unsigned index for $CODENAME:"
find "$DISTDIR" "$POOL" -type f | sort
echo "NOTE: Release is unsigned. The maintainer signs it into InRelease"
echo "(see packaging/deb/README.md); until then this repo installs only"
echo "with Signed-By pointing at the published key or [trusted=yes]."
