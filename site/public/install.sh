#!/bin/sh
# scoot installer for any glibc Linux: https://www.scoot.sh/install.sh
#
#   curl -fsSL https://www.scoot.sh/install.sh | sh
#   curl -fsSL https://www.scoot.sh/install.sh | sh -s -- --system --look ginger-night
#
# What it does, and nothing else:
#   1. picks a release: the newest tagged one (a single `vX.Y.Z`, or the
#      newest `scoot-vX.Y.Z` / `scootbar-vX.Y.Z` while those exist);
#      with no tagged release it fails instead of installing main;
#   2. downloads scoot, scootbg and scootbar for this machine and checks
#      each binary against the release's SHA256SUMS before installing it
#      (the session files come from the same tagged commit);
#   3. installs them and the `scoot-session` launcher into a bin directory,
#      the session's systemd user units, and the portal config;
#   4. with --system, also adds "scoot" to your login screen;
#   5. with --look, installs the shipped looks under the prefix's share
#      dir and applies one with `scoot look apply` (your config files are
#      never written, only read; old look configs migrate only after the
#      apply succeeds, so a refused apply leaves them byte-identical).
#      Releases before v0.3.0 have no looks: --look then installs the
#      binaries and says so, once, instead of failing.
#   6. lists any system library or font scoot still needs, with the one
#      command to install them (it stays confirmation-based: add -y or
#      --noconfirm yourself for a non-interactive run);
#   7. records every file it installed, so --uninstall removes exactly
#      those (your config stays), plus what was installed
#      (`channel=release`, version, tag, the bin directory covering
#      --prefix and --system, date) beside the manifest, so a re-run
#      for the same destination can say it is already up to date
#      instead of downloading again (a different destination, or
#      binaries gone from the recorded one, reinstalls). A re-run
#      with --look still installs the catalog and applies the look when
#      already up to date, migrating old look configs only after the
#      apply succeeds.
#      It never replaces a file it did not
#      install itself, such as one from a distro package.
#
# On Nix or NixOS, do not use this: the flake and home-manager give you
# the whole desktop (https://www.scoot.sh/start/install/).

set -eu

REPO=scoot-sh/scoot
RELEASES_URL=${SCOOT_INSTALL_RELEASES_URL:-https://api.github.com/repos/$REPO/releases?per_page=100}
DOWNLOAD_URL=${SCOOT_INSTALL_DOWNLOAD_URL:-https://github.com/$REPO/releases/download}
RAW_URL=${SCOOT_INSTALL_RAW_URL:-https://raw.githubusercontent.com/$REPO}
LOOKS="ginger-night moonrise music-desk radial-burst vinyl-sunset"

system=0
uninstall=0
dry_run=0
check=0
force=0
with_bar=1
look=""
prefix=""
scoot_version=""
bar_version=""

say() { printf '%s\n' "$*"; }
note() { printf 'scoot-install: %s\n' "$*" >&2; }
die() {
    printf 'scoot-install: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Install scoot, scootbg and scootbar from GitHub releases.

Usage: install.sh [options]

  --system            install under /usr/local and add scoot to the login
                      screen (asks for sudo); default is ~/.local, started
                      with `scoot-session` from a text console
  --look NAME         install the shipped looks and apply one:
                      ginger-night, moonrise, music-desk, radial-burst
                      or vinyl-sunset
  --version V         install scoot V (a tag vV, else scoot-vV, must exist)
  --bar-version V     install scootbar V (a tag scootbar-vV must exist;
                      retired once releases share one vV tag: use --version)
  --no-bar            skip scootbar
  --check             compare the installed version with the newest tagged
                      release (or the --version you name) and print both;
                      exit 0 if current, 10 if an update exists, 1 on
                      error; installs nothing
  --force             reinstall even when the installed version is current
  --prefix DIR        binaries go to DIR/bin (default ~/.local, or
                      /usr/local with --system)
  --uninstall         remove what an earlier run installed (add --system
                      for a system install); your config is kept
  --dry-run           say what would happen, change nothing
  -h, --help          this text
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
    --system) system=1 ;;
    --nightly) die "the nightly channel was retired; the installer installs tagged releases only (to run main, use the Nix flake: https://www.scoot.sh/start/install/)" ;;
    --uninstall) uninstall=1 ;;
    --dry-run) dry_run=1 ;;
    --check) check=1 ;;
    --force) force=1 ;;
    --no-bar) with_bar=0 ;;
    --look)
        [ $# -ge 2 ] || die "--look needs a name"
        look=$2
        shift
        ;;
    --look=*) look=${1#--look=} ;;
    --prefix)
        [ $# -ge 2 ] || die "--prefix needs a directory"
        prefix=$2
        shift
        ;;
    --prefix=*) prefix=${1#--prefix=} ;;
    --version)
        [ $# -ge 2 ] || die "--version needs a version"
        scoot_version=$2
        shift
        ;;
    --version=*) scoot_version=${1#--version=} ;;
    --bar-version)
        [ $# -ge 2 ] || die "--bar-version needs a version"
        bar_version=$2
        shift
        ;;
    --bar-version=*) bar_version=${1#--bar-version=} ;;
    -h | --help)
        usage
        exit 0
        ;;
    *) die "unknown option: $1 (see --help)" ;;
    esac
    shift
done

if [ -n "$look" ]; then
    case " $LOOKS " in
    *" $look "*) ;;
    *) die "unknown look '$look' (one of: $LOOKS)" ;;
    esac
fi
if [ "$check" = 1 ] && [ "$uninstall" = 1 ]; then
    die "--check and --uninstall do not mix"
fi
if [ "$check" = 1 ] && [ "$force" = 1 ]; then
    die "--check and --force do not mix"
fi

[ -n "${HOME:-}" ] || die "HOME is not set"
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
data_home=${XDG_DATA_HOME:-$HOME/.local/share}

# --- where things go ----------------------------------------------------

if [ "$system" = 1 ]; then
    bin_dir=${prefix:-/usr/local}/bin
    unit_dir=/usr/local/lib/systemd/user
    portal_dir=/etc/xdg-desktop-portal
    session_file=/usr/share/wayland-sessions/scoot.desktop
    manifest=/usr/local/share/scoot/install-manifest
    version_file=/usr/local/share/scoot/install-version
    if [ "$(id -u)" = 0 ]; then
        sudo=""
    elif command -v sudo >/dev/null 2>&1; then
        sudo=sudo
    elif command -v doas >/dev/null 2>&1; then
        sudo=doas
    else
        die "--system needs root: run as root, or install sudo or doas"
    fi
else
    bin_dir=${prefix:-$HOME/.local}/bin
    unit_dir=$config_home/systemd/user
    portal_dir=$config_home/xdg-desktop-portal
    session_file=""
    manifest=$data_home/scoot/install-manifest
    version_file=$data_home/scoot/install-version
    sudo=""
fi

run() {
    if [ "$dry_run" = 1 ]; then
        say "would run: $*"
    else
        "$@"
    fi
}

# Install SRC to DEST with MODE, as root for a system install, and record it.
place() {
    src=$1
    dest=$2
    mode=$3
    if [ "$dry_run" = 1 ]; then
        say "would install $dest"
        return 0
    fi
    $sudo mkdir -p "$(dirname "$dest")"
    $sudo install -m "$mode" "$src" "$dest"
    printf '%s\n' "$dest" >>"$tmp/manifest.new"
}

# True when DEST is ours to write: absent, or recorded by an earlier run.
ours() {
    [ ! -e "$1" ] && [ ! -L "$1" ] && return 0
    [ -f "$manifest" ] && grep -Fxq -- "$1" "$manifest"
}

# Merge what this run installed into the manifest. Runs on every exit,
# so a run that fails halfway still records what it placed.
write_manifest() {
    [ "$dry_run" = 0 ] || return 0
    [ -n "${tmp:-}" ] && [ -s "$tmp/manifest.new" ] || return 0
    if [ -f "$manifest" ]; then cat "$manifest" >>"$tmp/manifest.new"; fi
    sort -u "$tmp/manifest.new" >"$tmp/manifest.sorted"
    $sudo mkdir -p "$(dirname "$manifest")"
    $sudo install -m 644 "$tmp/manifest.sorted" "$manifest"
    : >"$tmp/manifest.new"
}

# Remove DIR when it is empty (a look directory the manifest emptied).
# Anything else in it keeps it: rmdir refuses a non-empty directory.
rmdir_empty() {
    if [ "$dry_run" = 1 ]; then
        say "would run: rmdir $1"
    else
        $sudo rmdir -- "$1" >/dev/null 2>&1 || true
    fi
}

have_systemd_user() {
    command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1
}

# --- uninstall ------------------------------------------------------------

if [ "$uninstall" = 1 ]; then
    [ -f "$manifest" ] || die "nothing to uninstall: no record at $manifest (installed with --system? add --system)"
    # The look entries, before the manifest itself is removed below.
    look_paths=$(grep '/share/scoot/looks/' "$manifest" || true)
    if have_systemd_user && systemctl --user is-enabled scootbar.service >/dev/null 2>&1; then
        run systemctl --user disable scootbar.service
    fi
    while IFS= read -r path; do
        [ -n "$path" ] || continue
        if [ -e "$path" ] || [ -L "$path" ]; then
            run $sudo rm -f -- "$path"
            say "removed $path"
        fi
    done <"$manifest"
    run $sudo rm -f -- "$manifest"
    # Drop the look directories the manifest emptied: only files were
    # recorded, and rmdir removes solely empty directories, so one
    # holding anything else stays.
    if [ -n "$look_paths" ]; then
        printf '%s\n' "$look_paths" | while IFS= read -r look_path; do
            [ -n "$look_path" ] || continue
            look_dir=$(dirname "$look_path")
            rmdir_empty "$look_dir"
            rmdir_empty "$(dirname "$look_dir")"
        done
    fi
    if [ -e "$version_file" ] || [ -L "$version_file" ]; then
        run $sudo rm -f -- "$version_file"
        say "removed $version_file"
    fi
    if have_systemd_user; then run systemctl --user daemon-reload; fi
    # Under sudo $HOME is root's, but the config the user cares about is
    # the invoking user's: name theirs when SUDO_USER says who that is.
    keep_config=$config_home
    if [ -n "${SUDO_USER:-}" ]; then
        keep_home=/home/$SUDO_USER
        if command -v getent >/dev/null 2>&1; then
            sudo_home=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6)
            if [ -n "$sudo_home" ]; then keep_home=$sudo_home; fi
        fi
        keep_config=$keep_home/.config
    fi
    say "scoot is uninstalled. Your config in $keep_config/scoot is untouched."
    exit 0
fi

# --- this machine ---------------------------------------------------------

case "$(uname -s)" in
Linux) ;;
Darwin) die "scoot runs on Linux. On a Mac, try it in a browser: https://www.scoot.sh/start/install/" ;;
*) die "scoot runs on Linux, not $(uname -s)" ;;
esac

case "$(uname -m)" in
x86_64 | amd64) arch=x86_64-linux ;;
aarch64 | arm64) arch=aarch64-linux ;;
*) die "no prebuilt scoot for $(uname -m); build from source: https://www.scoot.sh/start/install/" ;;
esac

glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}') || glibc=""
[ -n "$glibc" ] || die "the prebuilt binaries need glibc (musl distros such as Alpine: build from source)"
if ! printf '%s\n' "$glibc" | awk -F. '{ exit !($1 > 2 || ($1 == 2 && $2 >= 36)) }'; then
    die "the prebuilt binaries need glibc 2.36 or newer; this system has $glibc"
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 2 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "needs curl or wget (e.g. sudo apt install curl)"
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha_check() { sha256sum -c "$1" >/dev/null 2>&1; }
elif command -v shasum >/dev/null 2>&1; then
    sha_check() { shasum -a 256 -c "$1" >/dev/null 2>&1; }
else
    die "needs sha256sum or shasum to verify the download"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/scoot-install.XXXXXX")
trap 'write_manifest; rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM

# --- which release ----------------------------------------------------------

# The installer installs tagged releases only: never main, no moving
# channel. Releases share one `vX.Y.Z` tag; while the per-package
# `scoot-vX.Y.Z` / `scootbar-vX.Y.Z` tags still exist those keep working,
# and a unified tag wins when both exist.
#
# Prints the newest unified tag (`vX.Y.Z`), or nothing.
latest_unified_tag() {
    grep -o '"tag_name": *"[^"]*"' "$tmp/releases.json" |
        sed 's/.*"\([^"]*\)"$/\1/' |
        grep -E "^v[0-9]+\.[0-9]+\.[0-9]+\$" |
        head -n 1
}

# Prints the newest tag for PACKAGE (scoot or scootbar), or nothing.
latest_tag() {
    grep -o '"tag_name": *"[^"]*"' "$tmp/releases.json" |
        sed 's/.*"\([^"]*\)"$/\1/' |
        grep -E "^$1-v[0-9]+\.[0-9]+\.[0-9]+\$" |
        head -n 1
}

has_tag() {
    # Fixed-string compare: the wanted tag must equal a tag_name value
    # exactly. A regex match here would accept odd --version values
    # (e.g. '.*') and fail later at download instead of with a clean
    # "no release" error. Whitespace is compacted first so the match
    # does not depend on how the JSON spaces its colons.
    tr -d ' \t\n\r' <"$tmp/releases.json" >"$tmp/releases.compact"
    grep -Fq -- "\"tag_name\":\"$1\"" "$tmp/releases.compact"
}

fetch "$RELEASES_URL" "$tmp/releases.json" || die "could not list releases at $RELEASES_URL"
unified_latest=$(latest_unified_tag)
unified=0
if [ -n "$scoot_version" ] && has_tag "v$scoot_version"; then
    unified=1
    scoot_tag=v$scoot_version
elif [ -n "$unified_latest" ] && [ -z "$scoot_version" ] && [ -z "$bar_version" ]; then
    unified=1
    scoot_tag=$unified_latest
fi
if [ "$unified" = 1 ]; then
    if [ -n "$bar_version" ]; then
        die "--bar-version is retired: the bar shares the single $scoot_tag version (use --version)"
    fi
    bar_tag=$scoot_tag
else
    if [ -n "$unified_latest" ] && [ -n "$bar_version" ] && [ -z "$scoot_version" ]; then
        die "--bar-version is retired: the bar shares the single $unified_latest version (use --version)"
    fi
    if [ -n "$scoot_version" ]; then
        scoot_tag=scoot-v$scoot_version
        has_tag "$scoot_tag" || die "no release $scoot_tag"
    else
        scoot_tag=$(latest_tag scoot)
    fi
    if [ -n "$bar_version" ]; then
        bar_tag=scootbar-v$bar_version
        has_tag "$bar_tag" || die "no release $bar_tag"
    else
        bar_tag=$(latest_tag scootbar)
    fi
    if [ -z "$scoot_tag" ]; then
        die "no tagged release yet; the installer installs tagged releases only (to run main, use the Nix flake: https://www.scoot.sh/start/install/)"
    elif [ "$with_bar" = 1 ] && [ -z "$bar_tag" ]; then
        die "no tagged scootbar release yet (re-run with --no-bar to skip the bar)"
    fi
fi

# Download ASSET from release TAG into $tmp, and check it against that
# release's SHA256SUMS (fetched once per tag).
get_asset() {
    tag=$1
    asset=$2
    sums=$tmp/SHA256SUMS.$tag
    if [ ! -f "$sums" ]; then
        fetch "$DOWNLOAD_URL/$tag/SHA256SUMS" "$sums" || die "could not download $tag/SHA256SUMS"
    fi
    fetch "$DOWNLOAD_URL/$tag/$asset" "$tmp/$asset" || die "could not download $tag/$asset"
    grep -E "^[0-9a-f]{64}  $asset\$" "$sums" >"$tmp/check.$asset" ||
        die "$asset is not listed in $tag/SHA256SUMS"
    (cd "$tmp" && sha_check "check.$asset") ||
        die "$asset does not match its checksum (try downloading again in a few minutes)"
}

if [ "$unified" = 1 ]; then
    v=${scoot_tag#v}
else
    v=${scoot_tag#scoot-v}
fi
scoot_asset=scoot-$v-$arch
bg_asset=scootbg-$v-$arch
source_ref=$scoot_tag
label=$scoot_tag
version=$v
if [ "$with_bar" = 1 ]; then
    if [ "$unified" = 1 ]; then
        bar_asset=scootbar-$v-$arch
        bar_version=$v
    else
        bar_asset=scootbar-${bar_tag#scootbar-v}-$arch
    fi
    if [ -z "$bar_version" ]; then
        bar_version=${bar_tag#scootbar-v}
    fi
fi

# Looks ship in v0.3.0 together: earlier unified releases have no looks/
# tree and no `scoot look apply`. Detect that here, before anything is
# downloaded or written, and degrade to a plain install with one note
# instead of failing after the binaries land. (Per-package tags keep
# the old behavior: their catalog fetch decides, since a per-package tag
# can point at a commit new enough to carry looks.)
if [ -n "$look" ] && [ "$unified" = 1 ]; then
    look_major=${version%%.*}
    look_rest=${version#*.}
    look_minor=${look_rest%%.*}
    if [ "$look_major" -eq 0 ] 2>/dev/null && [ "$look_minor" -lt 3 ] 2>/dev/null; then
        note "this release ($scoot_tag) has no looks; use v0.3 or later for --look (installing the binaries only)"
        look=""
    fi
fi

# What is already installed (the record written by an earlier run, beside
# the manifest). Absent before this change: then nothing is current.
# bin_dir pins the record to its destination: a re-run with a different
# --prefix (or --system vs user) is never "up to date" for this
# destination, and a run whose binaries are gone reinstalls instead of
# claiming to be current. Records written before bin_dir existed have
# no bin_dir line, so they reinstall once and heal.
installed_version=""
installed_tag=""
installed_bar_tag=""
installed_bin_dir=""
if [ -f "$version_file" ]; then
    installed_version=$(sed -n 's/^version=//p' "$version_file" | head -n 1)
    installed_tag=$(sed -n 's/^tag=//p' "$version_file" | head -n 1)
    installed_bar_tag=$(sed -n 's/^bar_tag=//p' "$version_file" | head -n 1)
    installed_bin_dir=$(sed -n 's/^bin_dir=//p' "$version_file" | head -n 1)
fi
wanted_bar_tag=""
if [ "$with_bar" = 1 ]; then wanted_bar_tag=$bar_tag; fi
up_to_date=0
if [ -n "$installed_tag" ] && [ "$installed_tag" = "$scoot_tag" ] && [ "$installed_bar_tag" = "$wanted_bar_tag" ] && [ "$installed_bin_dir" = "$bin_dir" ]; then
    up_to_date=1
    [ -f "$bin_dir/scoot" ] || up_to_date=0
    [ -f "$bin_dir/scootbg" ] || up_to_date=0
    if [ "$with_bar" = 1 ]; then [ -f "$bin_dir/scootbar" ] || up_to_date=0; fi
fi

if [ "$check" = 1 ]; then
    if [ -n "$installed_tag" ]; then
        say "installed: $installed_version ($installed_tag)"
    else
        say "installed: none"
    fi
    say "latest: $version ($scoot_tag)"
    if [ -z "$installed_tag" ]; then
        die "nothing installed at $version_file"
    elif [ "$up_to_date" = 1 ]; then
        exit 0
    else
        exit 10
    fi
fi

# An up-to-date re-run with --look still applies the look (its files
# are config, never binaries, and an existing file is never
# overwritten), then exits without downloading anything. Without --look
# there is nothing to do.
if [ "$up_to_date" = 1 ] && [ "$force" = 0 ]; then
    if [ -z "$look" ]; then
        say "already up to date ($version)"
        exit 0
    fi
    skip_install=1
    say "already up to date ($version); applying look $look"
else
    skip_install=0
fi

if [ "$skip_install" = 0 ]; then
    say "Installing scoot $label for $arch into $bin_dir"

    get_asset "$scoot_tag" "$scoot_asset"
    get_asset "$scoot_tag" "$bg_asset"
    if [ "$with_bar" = 1 ]; then get_asset "$bar_tag" "$bar_asset"; fi

    # The session files come from the same commit as the binaries.
    for f in resources/scoot-session resources/scoot-portals.conf \
        resources/systemd/user/scoot.service resources/systemd/user/scoot-session.target \
        resources/systemd/user/scoot-shutdown.target packaging/deb/scoot.desktop \
        packaging/deb/scootbar.service; do
        fetch "$RAW_URL/$source_ref/$f" "$tmp/$(basename "$f")" || die "could not download $f at $source_ref"
    done

    sed "s|@SCOOT_BIN@|$bin_dir/scoot|g" "$tmp/scoot.service" >"$tmp/scoot.service.out"
    # The packaged unit is generic (graphical-session.target, for any
    # compositor). Installed here it runs only inside a scoot session, the
    # way the home-manager profile's bar does, never in GNOME or KDE.
    sed -e "s|^ExecStart=.*|ExecStart=$bin_dir/scootbar daemon|" \
        -e 's|graphical-session\.target|scoot-session.target|g' \
        "$tmp/scootbar.service" >"$tmp/scootbar.service.out"
    sed "s|^Exec=.*|Exec=$bin_dir/scoot-session|" "$tmp/scoot.desktop" >"$tmp/scoot.desktop.out"
fi

# --- install ----------------------------------------------------------------

# Up-to-date with --look skips everything below up to the look section:
# no downloads, no writes, no refusals for a destination that is
# already installed.
if [ "$skip_install" = 0 ]; then
    : >"$tmp/manifest.new"

    # Refuse before writing anything if a destination holds a file this
    # script did not install (a distro package, a source build). One call
    # per destination: the paths may contain spaces (a $HOME with one), so
    # they are never joined into a word-split string.
    check_ours() {
        ours "$1" || die "$1 exists and was not installed by this script (a distro package or a source build?). Remove it first, or install elsewhere with --prefix."
    }
    check_ours "$bin_dir/scoot"
    check_ours "$bin_dir/scootbg"
    check_ours "$bin_dir/scoot-session"
    check_ours "$unit_dir/scoot.service"
    check_ours "$unit_dir/scoot-session.target"
    check_ours "$unit_dir/scoot-shutdown.target"
    check_ours "$portal_dir/scoot-portals.conf"
    if [ "$with_bar" = 1 ]; then
        check_ours "$bin_dir/scootbar"
        check_ours "$unit_dir/scootbar.service"
    fi
    if [ -n "$session_file" ]; then check_ours "$session_file"; fi

    place "$tmp/$scoot_asset" "$bin_dir/scoot" 755
    place "$tmp/$bg_asset" "$bin_dir/scootbg" 755
    place "$tmp/scoot-session" "$bin_dir/scoot-session" 755
    place "$tmp/scoot.service.out" "$unit_dir/scoot.service" 644
    place "$tmp/scoot-session.target" "$unit_dir/scoot-session.target" 644
    place "$tmp/scoot-shutdown.target" "$unit_dir/scoot-shutdown.target" 644
    place "$tmp/scoot-portals.conf" "$portal_dir/scoot-portals.conf" 644
    if [ "$with_bar" = 1 ]; then
        place "$tmp/$bar_asset" "$bin_dir/scootbar" 755
        place "$tmp/scootbar.service.out" "$unit_dir/scootbar.service" 644
    fi
    if [ -n "$session_file" ]; then
        place "$tmp/scoot.desktop.out" "$session_file" 644
    fi
fi

# --- a look -------------------------------------------------------------------

# The shipped catalog as plain files under the prefix's share dir: `scoot
# look` finds `<prefix>/share/scoot/looks` beside its own binary, so the
# installed scoot sees exactly this tree. Recorded in the manifest, so
# --uninstall removes it.
looks_dest=$(dirname "$bin_dir")/share/scoot/looks

# The user --look acts for: this user, or the invoking user under sudo
# (root's own config and look state are not the desktop being
# installed). The config dir honors XDG only for this user: under sudo
# the invoking user's variables did not survive, so it is the plain
# ~/.config.
look_home=$HOME
if [ -n "${SUDO_USER:-}" ]; then
    look_home=/home/$SUDO_USER
    if command -v getent >/dev/null 2>&1; then
        sudo_home=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6)
        if [ -n "$sudo_home" ]; then look_home=$sudo_home; fi
    fi
fi
user_config_home=$config_home
if [ -n "${SUDO_USER:-}" ]; then user_config_home=$look_home/.config; fi

# Fetch REL (a path under looks/) into the catalog, unless it is already
# there from this script: an up-to-date re-run with --look then needs no
# download, and a file this script did not install (a distro package, a
# hand copy) is never replaced.
place_look() {
    rel=$1
    dest=$looks_dest/$rel
    if [ -e "$dest" ] || [ -L "$dest" ]; then
        if ours "$dest"; then
            [ "$skip_install" = 1 ] && return 0
        else
            note "kept $dest (it was not installed by this script)"
            return 0
        fi
    fi
    fetch "$RAW_URL/$source_ref/looks/$rel" "$tmp/lookfile" || die "could not download the look catalog ($rel)"
    place "$tmp/lookfile" "$dest" 644
}

# The image a look.toml names under [wallpaper], when it is a file in
# the look directory (a color look names none; a URL is scootbg's to
# fetch, never the installer's).
look_image() {
    sed -n '/^\[wallpaper\]/,/^\[/s/^[ \t]*image[ \t]*=[ \t]*"\([^"]*\)".*/\1/p' "$1" | head -n 1
}

install_look_catalog() {
    for l in $LOOKS; do
        place_look "$l/look.toml"
        toml=$looks_dest/$l/look.toml
        # A dry run installs nothing: read the image name from the
        # download still sitting in $tmp instead (place_look's fetch is
        # the most recent write there whenever the destination was
        # missing).
        if [ ! -f "$toml" ] && [ -f "$tmp/lookfile" ]; then toml=$tmp/lookfile; fi
        [ -f "$toml" ] || continue
        image=$(look_image "$toml")
        case "$image" in
        "" | "~"* | /* | http* | *://*) ;;
        *) place_look "$l/$image" ;;
        esac
    done
}

# What an earlier --look run wrote for look L: the example's whole
# configs with the installer's rewrites (the wallpaper image pointed at
# the config dir, the bar's helper scripts by full path). Reproduced
# byte for byte: a user file still identical to one of these has its
# look keys removed so the layer shows through; anything else is left
# alone, and `scoot look apply` names the keys shadowing the look. A
# template that no longer downloads is skipped with a note, never a
# failure: the file is then simply left alone.
old_look_file() {
    l=$1
    which=$2
    out=$3
    fetch "$RAW_URL/$source_ref/docs/examples/$l/$which" "$out" || {
        note "could not download the $l example for comparison; its configs are left alone"
        return 1
    }
    if [ "$which" = "scoot.toml" ]; then
        image=$(sed -n 's/^image *= *"\([^"]*\)".*/\1/p' "$out" | head -n 1)
        case "$image" in
        "" | "~"* | /* | http*) ;;
        *)
            ext=${image##*.}
            sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.$ext\"|" "$out" >"$out.out"
            mv "$out.out" "$out"
            ;;
        esac
    else
        for s in cpu.sh load.sh; do
            grep -q "\"$s\"" "$out" || continue
            esc=$(printf '%s' "$user_config_home/scoot/$s" | sed 's/[|&\\]/\\&/g')
            sed "s|\"$s\"|\"$esc\"|" "$out" >"$out.out"
            mv "$out.out" "$out"
        done
    fi
}

# Delete a look's keys from a config that is still byte-identical to an
# earlier --look run's output: whole look-owned sections (`appearance`,
# `wallpaper`, `colors`), the look's `[bar]` style keys, and the look's
# per-app overrides (`"section.key" = ...` in the look.toml). Layout
# (module lists, binds, commands, geometry) stays. Reads the config on
# stdin, writes the stripped config on stdout.
strip_look_keys() {
    kind=$1
    toml=$2
    whole=""
    keys=""
    case "$kind" in
    scoot.toml) whole="appearance wallpaper" ;;
    bar.toml)
        whole="colors"
        keys="bar:radius bar:popup-radius bar:opacity bar:font bar:fallback-fonts bar:font-size"
        if [ -f "$toml" ]; then
            extra=$(awk '/^\[apps\.scootbar\]/{s=1;next} /^\[/{s=0} s' "$toml" |
                sed -n 's/^"\([^."]*\)\.\([^"]*\)".*/\1:\2/p')
            for e in $extra; do keys="$keys $e"; done
        fi
        ;;
    esac
    awk -v whole="$whole" -v keys="$keys" '
        function linekey(s) {
            sub(/^[ \t]+/, "", s)
            if (s == "" || substr(s, 1, 1) == "#" || substr(s, 1, 1) == "[") return ""
            p = index(s, "=")
            if (p == 0) return ""
            key = substr(s, 1, p - 1)
            sub(/[ \t]+$/, "", key)
            return key
        }
        BEGIN {
            split(whole, w, " ")
            for (i in w) { if (w[i] != "") WS[w[i]] = 1 }
            split(keys, ka, " ")
            for (i in ka) {
                if (ka[i] == "") continue
                p = index(ka[i], ":")
                KK[substr(ka[i], 1, p - 1) SUBSEP substr(ka[i], p + 1)] = 1
            }
            cur = ""
        }
        /^[ \t]*\[/ {
            h = $0
            sub(/^[ \t]*\[/, "", h)
            sub(/\].*$/, "", h)
            gsub(/[ \t]/, "", h)
            cur = h
            if (cur in WS) next
            print
            next
        }
        {
            if (cur in WS) next
            key = linekey($0)
            if (key != "" && ((cur SUBSEP key) in KK)) next
            print
        }
    '
}

migrate_look_configs() {
    for cfg in scoot.toml:config.toml bar.toml:bar.toml; do
        which=${cfg%%:*}
        name=${cfg##*:}
        file=$user_config_home/scoot/$name
        [ -f "$file" ] || continue
        for l in $LOOKS; do
            old_look_file "$l" "$which" "$tmp/old-$l-$which" || continue
            if cmp -s "$file" "$tmp/old-$l-$which"; then
                # A symlink (home-manager, dotfiles) is never written
                # through: its target is someone else's file. Leave it
                # alone and say so.
                if [ -L "$file" ]; then
                    note "left $file alone (it is a symlink; migrate it by hand)"
                    break
                fi
                if [ "$dry_run" = 1 ]; then
                    say "would migrate $file (still the $l example: remove its look keys)"
                    break
                fi
                # Swap the stripped config in atomically: a temp file in
                # the same directory plus rename, never a truncating
                # redirect into the live file. A failed strip leaves the
                # original untouched.
                tmp_new=$(mktemp "$(dirname "$file")/.scoot-migrate.XXXXXX") || {
                    note "could not migrate $file (no temp file); left alone"
                    break
                }
                if strip_look_keys "$which" "$looks_dest/$l/look.toml" <"$file" >"$tmp_new"; then
                    mode=$(stat -c %a "$file" 2>/dev/null || stat -f %Lp "$file" 2>/dev/null || true)
                    if [ -n "$mode" ]; then chmod "$mode" "$tmp_new" 2>/dev/null || true; fi
                    mv -f "$tmp_new" "$file"
                    note "migrated $file: it was still the $l example config, so its look keys were removed and the look layer shows through (layout kept)"
                else
                    rm -f "$tmp_new"
                    note "could not migrate $file; left alone"
                fi
                break
            fi
        done
    done
}

# Apply the look with the binary just installed (its helpers resolve
# beside it, so no PATH is needed). With SUDO_USER set the apply runs as
# the invoking user, whose config and state this is; the environment it
# gets is built from their home (see run_as_invoker). As root this drops
# privileges first: root-owned files would lock the user out of their
# own look state.
shquote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }

# Run the apply as the invoking user: $sudo -u (proven passwordless by
# the install above, or prompting like any sudo use on a look-only
# re-run), else sudo, su, doas, runuser or setpriv when root. Every
# variable is assigned, never unset or emptied: some toolchains lack
# `env -u`, and fontconfig reads an empty XDG_DATA_HOME as set, hiding
# ~/.local/share/fonts. The XDG variables name the standard defaults
# under the user's home (their own values did not survive sudo); the
# runtime dir points at their session, so a running compositor still
# reloads live. SCOOT_LOOK_STATE and SCOOT_SOCKET stay empty, which the
# tool reads as unset.
run_as_invoker() {
    tool=$1
    shift
    case "$tool" in
    sudo | doas) "$tool" -u "$SUDO_USER" "$@" ;;
    su)
        cmd=""
        for a in "$@"; do cmd="$cmd $(shquote "$a")"; done
        su -s /bin/sh "$SUDO_USER" -c "$cmd"
        ;;
    runuser) runuser -u "$SUDO_USER" -- "$@" ;;
    setpriv) setpriv --reuid="$SUDO_USER" --regid="$(id -g "$SUDO_USER")" --clear-groups "$@" ;;
    *) die "cannot apply the look as $SUDO_USER: no sudo, su, doas, runuser or setpriv here" ;;
    esac
}

invoker_tool() {
    if [ -n "$sudo" ]; then printf '%s' "$sudo"; return 0; fi
    if [ "$(id -u)" = 0 ]; then
        for t in sudo su doas runuser setpriv; do
            if command -v "$t" >/dev/null 2>&1; then printf '%s' "$t"; return 0; fi
        done
    fi
    return 1
}

run_look_apply() {
    bin=$bin_dir/scoot
    tool=""
    if [ -n "${SUDO_USER:-}" ]; then tool=$(invoker_tool || true); fi
    if [ "$dry_run" = 1 ]; then
        if [ "$tool" = sudo ] || [ "$tool" = doas ]; then
            say "would run: $tool -u $SUDO_USER env HOME=$look_home (standard XDG defaults) $bin look apply $look"
        elif [ -n "$tool" ]; then
            say "would run: $bin look apply $look (as $SUDO_USER via $tool)"
        else
            say "would run: $bin look apply $look"
        fi
        return 0
    fi
    [ -x "$bin" ] || die "$bin is not executable"
    if [ -n "${SUDO_USER:-}" ]; then
        if [ -n "$tool" ]; then
            run_uid=$(id -u "$SUDO_USER" 2>/dev/null || true)
            run_runtime=
            if [ -n "$run_uid" ]; then run_runtime=/run/user/$run_uid; fi
            run run_as_invoker "$tool" env HOME="$look_home" XDG_CONFIG_HOME="$look_home/.config" XDG_DATA_HOME="$look_home/.local/share" XDG_CACHE_HOME="$look_home/.cache" XDG_STATE_HOME="$look_home/.local/state" XDG_RUNTIME_DIR="$run_runtime" SCOOT_LOOK_STATE= SCOOT_SOCKET= "$bin" look apply "$look"
        else
            note "cannot reach $SUDO_USER here: applying the look as this user"
            run "$bin" look apply "$look"
        fi
    else
        if [ "$(id -u)" = 0 ]; then note "no invoking user found: applying the look for root"; fi
        run "$bin" look apply "$look"
    fi
}

look_error=""
if [ -n "$look" ]; then
    install_look_catalog
    # Apply before migrating: a refused apply (a missing font, usually)
    # must leave the user's configs byte-identical, so the look keys are
    # stripped only after the apply succeeds.
    run_look_apply || look_error="the look was not applied (see above); the binaries and the catalog are installed, so fix the cause and re-run with --look"
    if [ -z "$look_error" ]; then
        migrate_look_configs
    fi
fi

# A look-only re-run stops here: the record is already current, and
# there is nothing to enable or report.
if [ "$skip_install" = 1 ]; then
    if [ -n "$look_error" ]; then die "$look_error"; fi
    exit 0
fi

# --- record, enable --------------------------------------------------------------

write_manifest

# What this run installed: channel is always release. A re-run and
# --check read it (up to date or not), --uninstall removes it. Written
# here, not in the EXIT trap: a run that fails halfway must not claim
# the install is complete.
if [ "$dry_run" = 0 ]; then
    {
        printf 'channel=release\n'
        printf 'version=%s\n' "$version"
        printf 'tag=%s\n' "$scoot_tag"
        printf 'bar_version=%s\n' "${bar_version:-}"
        printf 'bar_tag=%s\n' "${wanted_bar_tag:-}"
        printf 'bin_dir=%s\n' "$bin_dir"
        printf 'date=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$tmp/install-version"
    $sudo mkdir -p "$(dirname "$version_file")"
    $sudo install -m 644 "$tmp/install-version" "$version_file"
fi

if have_systemd_user; then
    run systemctl --user daemon-reload
    if [ "$with_bar" = 1 ]; then run systemctl --user enable scootbar.service; fi
else
    note "no systemd user manager here: scoot-session will run the compositor without session units"
fi

# --- what is still missing ----------------------------------------------------------

# The downloaded scoot binary is already in $tmp, so even a dry run can
# say which system libraries it still needs: ldd only reads.
ldd_target=$bin_dir/scoot
if [ "$dry_run" = 1 ]; then ldd_target=$tmp/$scoot_asset; fi
missing=""
if command -v ldd >/dev/null 2>&1; then
    missing=$(ldd "$ldd_target" 2>/dev/null | awk '/not found/ {print $1}' | tr '\n' ' ')
fi
need_foot=0
command -v foot >/dev/null 2>&1 || need_foot=1
# The font files scootbar tries without --font (crates/scootbar/src/font.rs
# WELL_KNOWN), overridable for tests the way the release URLs are.
font_candidates=${SCOOT_INSTALL_FONTS:-"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf
/usr/share/fonts/TTF/DejaVuSans.ttf
/usr/share/fonts/truetype/DejaVuSans.ttf
/usr/share/fonts/dejavu/DejaVuSans.ttf
/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf
/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf
/usr/share/fonts/noto/NotoSans-Regular.ttf"}
# Whether scootbar would find a font: any one of its well-known files
# present means it starts, so only none at all adds a font package below.
need_font=0
if [ "$with_bar" = 1 ]; then
    need_font=1
    for f in $font_candidates; do
        if [ -s "$f" ]; then
            need_font=0
            break
        fi
    done
fi

os_release=${SCOOT_INSTALL_OS_RELEASE:-/etc/os-release}
distro=""
if [ -r "$os_release" ]; then
    # shellcheck disable=SC1090,SC1091
    distro=$(. "$os_release" && printf '%s %s' "${ID:-}" "${ID_LIKE:-}")
fi
pkg_for() {
    case "$distro" in
    *arch*)
        case "$1" in
        libseat*) echo seatd ;; libinput*) echo libinput ;; libxkbcommon*) echo libxkbcommon ;;
        libudev*) echo systemd-libs ;; libpixman*) echo pixman ;; font) echo ttf-dejavu ;; foot) echo foot ;;
        esac
        ;;
    *fedora* | *rhel*)
        case "$1" in
        libseat*) echo libseat ;; libinput*) echo libinput ;; libxkbcommon*) echo libxkbcommon ;;
        libudev*) echo systemd-libs ;; libpixman*) echo pixman ;; font) echo dejavu-sans-fonts ;; foot) echo foot ;;
        esac
        ;;
    # openSUSE names its libraries the Debian way (libseat1, libinput10,
    # ...: verified against Tumbleweed's own metadata), so it shares the
    # default below; this branch only documents that the fallthrough is
    # deliberate, not a gap.
    *suse*)
        case "$1" in
        libseat*) echo libseat1 ;; libinput*) echo libinput10 ;; libxkbcommon*) echo libxkbcommon0 ;;
        libudev*) echo libudev1 ;; libpixman*) echo libpixman-1-0 ;; font) echo dejavu-fonts ;; foot) echo foot ;;
        esac
        ;;
    *)
        case "$1" in
        libseat*) echo libseat1 ;; libinput*) echo libinput10 ;; libxkbcommon*) echo libxkbcommon0 ;;
        libudev*) echo libudev1 ;; libpixman*) echo libpixman-1-0 ;; font) echo fonts-dejavu-core ;; foot) echo foot ;;
        esac
        ;;
    esac
}
pkgs=""
for lib in $missing; do pkgs="$pkgs $(pkg_for "$lib")"; done
if [ "$need_foot" = 1 ]; then pkgs="$pkgs $(pkg_for foot)"; fi
if [ "$need_font" = 1 ]; then pkgs="$pkgs $(pkg_for font)"; fi
if [ -n "$pkgs" ]; then
    case "$distro" in
    *arch*) cmd="sudo pacman -S --needed$pkgs" ;;
    *fedora* | *rhel*) cmd="sudo dnf install$pkgs" ;;
    *suse*) cmd="sudo zypper install$pkgs" ;;
    *debian* | *ubuntu*) cmd="sudo apt install$pkgs" ;;
    *) cmd="install with your package manager:$pkgs" ;;
    esac
    say ""
    if [ -n "$missing" ]; then
        say "scoot needs a few system libraries (and foot, its default terminal):"
    elif [ "$need_foot" = 1 ]; then
        say "Install foot, scoot's default terminal:"
    fi
    if [ "$need_font" = 1 ]; then
        say "scootbar needs a font to draw text (none of the usual font files is usable)."
    fi
    say "Non-interactive: use apt -y, dnf -y, or pacman --noconfirm."
    say "  $cmd"
fi

# --- next ---------------------------------------------------------------------------

say ""
case ":${PATH:-}:" in
*":$bin_dir:"*) ;;
*) say "Add $bin_dir to your PATH first (e.g. in ~/.profile)." ;;
esac
say "Try it in a window inside your current desktop:"
say "  scoot --nested -- foot"
if [ -n "$session_file" ]; then
    say "Then log out and pick \"scoot\" on your login screen."
else
    say "Or run a whole session from a text console (Ctrl+Alt+F3, log in):"
    say "  scoot-session"
    say "(Re-run with --system to add scoot to your login screen.)"
fi
say ""
say "Next: https://www.scoot.sh/start/first-session/  ·  Remove: install.sh --uninstall$([ "$system" = 1 ] && printf ' --system')"

# A refused `scoot look apply` never stopped the install above: the
# binaries, the catalog, the units and the library report are all done.
# Fail here instead, so the cause (a missing font, usually) and its fix
# print first.
if [ -n "$look_error" ]; then die "$look_error"; fi
