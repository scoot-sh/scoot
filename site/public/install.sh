#!/bin/sh
# scoot installer for any glibc Linux: https://www.scoot.sh/install.sh
#
#   curl -fsSL https://www.scoot.sh/install.sh | sh
#   curl -fsSL https://www.scoot.sh/install.sh | sh -s -- --system --look ginger-night
#
# What it does, and nothing else:
#   1. picks a release: the newest tagged one, or `nightly` (main, rebuilt
#      on every merge) when asked or when no tag exists yet;
#   2. downloads scoot, scootbg and scootbar for this machine and checks
#      each binary against the release's SHA256SUMS before installing it
#      (the session files come from the same tagged or pinned commit);
#   3. installs them and the `scoot-session` launcher into a bin directory,
#      the session's systemd user units, and the portal config;
#   4. with --system, also adds "scoot" to your login screen;
#   5. with --look, writes that look's config to ~/.config/scoot, never
#      over a file you already have;
#   6. lists any system library scoot still needs, with the one command to
#      install them;
#   7. records every file it installed, so --uninstall removes exactly
#      those (your config stays). It never replaces a file it did not
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
nightly=0
uninstall=0
dry_run=0
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
  --look NAME         also write a look's config: ginger-night, moonrise,
                      music-desk, radial-burst or vinyl-sunset
  --nightly           install main's latest build instead of a tagged release
  --version V         install scoot V (a tag scoot-vV must exist)
  --bar-version V     install scootbar V (a tag scootbar-vV must exist)
  --no-bar            skip scootbar
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
    --nightly) nightly=1 ;;
    --uninstall) uninstall=1 ;;
    --dry-run) dry_run=1 ;;
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
if [ "$nightly" = 1 ] && { [ -n "$scoot_version" ] || [ -n "$bar_version" ]; }; then
    die "--nightly and --version/--bar-version do not mix"
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

have_systemd_user() {
    command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1
}

# --- uninstall ------------------------------------------------------------

if [ "$uninstall" = 1 ]; then
    [ -f "$manifest" ] || die "nothing to uninstall: no record at $manifest (installed with --system? add --system)"
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
    if have_systemd_user; then run systemctl --user daemon-reload; fi
    say "scoot is uninstalled. Your config in $config_home/scoot is untouched."
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
    die "needs curl or wget"
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

# Prints the newest tag for PACKAGE (scoot or scootbar), or nothing.
latest_tag() {
    grep -o '"tag_name": *"[^"]*"' "$tmp/releases.json" |
        sed 's/.*"\([^"]*\)"$/\1/' |
        grep -E "^$1-v[0-9]+\.[0-9]+\.[0-9]+\$" |
        head -n 1
}

if [ "$nightly" = 0 ]; then
    fetch "$RELEASES_URL" "$tmp/releases.json" || die "could not list releases at $RELEASES_URL"
    if [ -n "$scoot_version" ]; then
        scoot_tag=scoot-v$scoot_version
        grep -q "\"$scoot_tag\"" "$tmp/releases.json" || die "no release $scoot_tag"
    else
        scoot_tag=$(latest_tag scoot)
    fi
    if [ -n "$bar_version" ]; then
        bar_tag=scootbar-v$bar_version
        grep -q "\"$bar_tag\"" "$tmp/releases.json" || die "no release $bar_tag"
    else
        bar_tag=$(latest_tag scootbar)
    fi
    if [ -z "$scoot_tag" ]; then
        note "no tagged scoot release yet; installing the nightly build of main"
        nightly=1
    elif [ "$with_bar" = 1 ] && [ -z "$bar_tag" ]; then
        note "no tagged scootbar release yet; installing the nightly bar"
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
        die "$asset does not match its checksum (a nightly republish in progress? run this again in a few minutes)"
}

if [ "$nightly" = 1 ]; then
    get_asset nightly nightly.txt
    commit=$(sed -n 's/^commit=//p' "$tmp/nightly.txt")
    [ -n "$commit" ] || die "nightly.txt names no commit"
    scoot_asset=scoot-nightly-$arch
    bg_asset=scootbg-nightly-$arch
    scoot_tag=nightly
    source_ref=$commit
    rolling=$(sed -n 's/^version=//p' "$tmp/nightly.txt")
    label="nightly ${rolling:-$(printf '%.8s' "$commit")}"
else
    v=${scoot_tag#scoot-v}
    scoot_asset=scoot-$v-$arch
    bg_asset=scootbg-$v-$arch
    source_ref=$scoot_tag
    label=$scoot_tag
fi
if [ "$with_bar" = 1 ]; then
    if [ "$nightly" = 1 ] || [ -z "${bar_tag:-}" ]; then
        bar_tag=nightly
        bar_asset=scootbar-nightly-$arch
    else
        bar_asset=scootbar-${bar_tag#scootbar-v}-$arch
    fi
fi

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

# --- install ----------------------------------------------------------------

: >"$tmp/manifest.new"

# Refuse before writing anything if a destination holds a file this
# script did not install (a distro package, a source build).
dests="$bin_dir/scoot $bin_dir/scootbg $bin_dir/scoot-session $unit_dir/scoot.service
$unit_dir/scoot-session.target $unit_dir/scoot-shutdown.target $portal_dir/scoot-portals.conf"
if [ "$with_bar" = 1 ]; then dests="$dests $bin_dir/scootbar $unit_dir/scootbar.service"; fi
if [ -n "$session_file" ]; then dests="$dests $session_file"; fi
for d in $dests; do
    ours "$d" || die "$d exists and was not installed by this script (a distro package or a source build?). Remove it first, or install elsewhere with --prefix."
done

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

# --- a look -------------------------------------------------------------------

# Write SRC to the user's config path DEST unless a file is already there.
place_config() {
    if [ -e "$2" ]; then
        note "kept your $2 (the look's version was not written)"
        return 0
    fi
    if [ "$dry_run" = 1 ]; then
        say "would write $2"
        return 0
    fi
    mkdir -p "$(dirname "$2")"
    cp -p "$1" "$2"
}

if [ -n "$look" ]; then
    base=$RAW_URL/$source_ref/docs/examples/$look
    fetch "$base/scoot.toml" "$tmp/look-scoot.toml" || die "could not download the $look look"
    fetch "$base/bar.toml" "$tmp/look-bar.toml" || die "could not download the $look bar config"
    image=$(sed -n 's/^image *= *"\([^"]*\)".*/\1/p' "$tmp/look-scoot.toml" | head -n 1)
    case "$image" in
    "" | "~"* | /* | http*) ;;
    *)
        # A path relative to the look's directory: fetch it and keep it
        # next to the config, where a relative path resolves.
        ext=${image##*.}
        src=$(printf '%s' "docs/examples/$look/$image" |
            awk -F/ '{n=0; for(i=1;i<=NF;i++){ if($i==".."){n--} else if($i!="."){p[++n]=$i} } s=p[1]; for(i=2;i<=n;i++) s=s"/"p[i]; print s}')
        if fetch "$RAW_URL/$source_ref/$src" "$tmp/look-wallpaper.$ext"; then
            place_config "$tmp/look-wallpaper.$ext" "$config_home/scoot/wallpaper.$ext"
            sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.$ext\"|" "$tmp/look-scoot.toml" >"$tmp/look-scoot.toml.out"
            mv "$tmp/look-scoot.toml.out" "$tmp/look-scoot.toml"
        else
            note "could not download the $look wallpaper; set [wallpaper] image yourself"
        fi
        ;;
    esac
    place_config "$tmp/look-scoot.toml" "$config_home/scoot/config.toml"
    if [ "$with_bar" = 1 ]; then
        # The bar's helper scripts live beside its config (your files, like
        # the config), and the bar runs them by full path.
        for s in cpu.sh load.sh; do
            grep -q "\"$s\"" "$tmp/look-bar.toml" || continue
            if fetch "$base/$s" "$tmp/$s"; then
                chmod 755 "$tmp/$s"
                place_config "$tmp/$s" "$config_home/scoot/$s"
                esc=$(printf '%s' "$config_home/scoot/$s" | sed 's/[|&\\]/\\&/g')
                sed "s|\"$s\"|\"$esc\"|" "$tmp/look-bar.toml" >"$tmp/look-bar.toml.out"
                mv "$tmp/look-bar.toml.out" "$tmp/look-bar.toml"
            fi
        done
        place_config "$tmp/look-bar.toml" "$config_home/scoot/bar.toml"
    fi
fi

# --- record, enable --------------------------------------------------------------

write_manifest

if have_systemd_user; then
    run systemctl --user daemon-reload
    if [ "$with_bar" = 1 ]; then run systemctl --user enable scootbar.service; fi
else
    note "no systemd user manager here: scoot-session will run the compositor without session units"
fi

# --- what is still missing ----------------------------------------------------------

missing=""
if [ "$dry_run" = 0 ] && command -v ldd >/dev/null 2>&1; then
    missing=$(ldd "$bin_dir/scoot" 2>/dev/null | awk '/not found/ {print $1}' | tr '\n' ' ')
fi
need_foot=0
command -v foot >/dev/null 2>&1 || need_foot=1

distro=""
if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    distro=$(. /etc/os-release && printf '%s %s' "${ID:-}" "${ID_LIKE:-}")
fi
pkg_for() {
    case "$distro" in
    *arch*)
        case "$1" in
        libseat*) echo seatd ;; libinput*) echo libinput ;; libxkbcommon*) echo libxkbcommon ;;
        libudev*) echo systemd-libs ;; libpixman*) echo pixman ;; foot) echo foot ;;
        esac
        ;;
    *fedora* | *rhel*)
        case "$1" in
        libseat*) echo libseat ;; libinput*) echo libinput ;; libxkbcommon*) echo libxkbcommon ;;
        libudev*) echo systemd-libs ;; libpixman*) echo pixman ;; foot) echo foot ;;
        esac
        ;;
    *)
        case "$1" in
        libseat*) echo libseat1 ;; libinput*) echo libinput10 ;; libxkbcommon*) echo libxkbcommon0 ;;
        libudev*) echo libudev1 ;; libpixman*) echo libpixman-1-0 ;; foot) echo foot ;;
        esac
        ;;
    esac
}
pkgs=""
for lib in $missing; do pkgs="$pkgs $(pkg_for "$lib")"; done
if [ "$need_foot" = 1 ]; then pkgs="$pkgs $(pkg_for foot)"; fi
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
    else
        say "Install foot, scoot's default terminal:"
    fi
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
