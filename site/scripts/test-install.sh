#!/bin/sh
# Installer gate: runs public/install.sh against a fake release tree
# (file:// URLs, stub binaries, stub session files), so the script's
# paths, checksum refusal, nightly fallback, look handling and uninstall
# are proved without the network. shellcheck runs first when it is on
# PATH (CI provides it). `pnpm test-install`; CI's docs-site job.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
installer=$here/../public/install.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/scoot-install-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

fail() {
    printf 'test-install: FAIL: %s\n' "$*" >&2
    exit 1
}
ok() { printf 'test-install: ok: %s\n' "$*"; }

if command -v shellcheck >/dev/null 2>&1; then
    shellcheck -s sh "$installer" "$0"
    ok "shellcheck"
fi
sh -n "$installer"

case "$(uname -m)" in
aarch64 | arm64) arch=aarch64-linux ;;
*) arch=x86_64-linux ;;
esac

# --- a fake release tree ------------------------------------------------------
f=$work/fake
commit=0123456789abcdef0123456789abcdef01234567
stub() { printf '#!/bin/sh\necho "%s"\n' "$1" >"$2" && chmod +x "$2"; }
mkdir -p "$f/dl/scoot-v0.1.0" "$f/dl/scootbar-v0.2.0" "$f/dl/nightly"
stub "scoot 0.1.0" "$f/dl/scoot-v0.1.0/scoot-0.1.0-$arch"
stub "scootbg 0.1.0" "$f/dl/scoot-v0.1.0/scootbg-0.1.0-$arch"
stub "scootbar 0.2.0" "$f/dl/scootbar-v0.2.0/scootbar-0.2.0-$arch"
for b in scoot scootbg scootbar; do stub "$b nightly" "$f/dl/nightly/$b-nightly-$arch"; done
printf "version=0.1.42+rev-0123456\ncommit=%s\n" "$commit" >"$f/dl/nightly/nightly.txt"
for d in "$f"/dl/*; do (cd "$d" && sha256sum -- * >SHA256SUMS); done

for ref in scoot-v0.1.0 "$commit"; do
    r=$f/raw/$ref
    mkdir -p "$r/resources/systemd/user" "$r/packaging/deb" "$r/docs/examples/ginger-night" "$r/docs/assets/wallpapers" "$r/docs/examples/moonrise"
    printf '#!/bin/sh\n' >"$r/resources/scoot-session"
    printf '[preferred]\n' >"$r/resources/scoot-portals.conf"
    printf '[Service]\nExecStart=@SCOOT_BIN@ --tty\n' >"$r/resources/systemd/user/scoot.service"
    printf '[Unit]\n' >"$r/resources/systemd/user/scoot-session.target"
    printf '[Unit]\n' >"$r/resources/systemd/user/scoot-shutdown.target"
    printf '[Desktop Entry]\nExec=/usr/bin/scoot-session\n' >"$r/packaging/deb/scoot.desktop"
    printf '[Unit]\nPartOf=graphical-session.target\nAfter=graphical-session.target\n[Service]\nExecStart=/usr/bin/scootbar daemon\n[Install]\nWantedBy=graphical-session.target\n' >"$r/packaging/deb/scootbar.service"
    printf '[wallpaper]\nimage = "wallpaper.png"\n' >"$r/docs/examples/ginger-night/scoot.toml"
    printf 'command = ["cpu.sh"]\n' >"$r/docs/examples/ginger-night/bar.toml"
    printf 'png\n' >"$r/docs/examples/ginger-night/wallpaper.png"
    printf '#!/bin/sh\n' >"$r/docs/examples/ginger-night/cpu.sh"
    printf '#!/bin/sh\n' >"$r/docs/examples/ginger-night/load.sh"
    printf '[wallpaper]\nimage = "../../assets/wallpapers/moonrise.png"\n' >"$r/docs/examples/moonrise/scoot.toml"
    printf 'left = []\n' >"$r/docs/examples/moonrise/bar.toml"
    printf 'moon\n' >"$r/docs/assets/wallpapers/moonrise.png"
done
printf '[{"tag_name": "nightly"}, {"tag_name": "scootbar-v0.2.0"}, {"tag_name": "scoot-v0.1.0"}]' >"$f/tags.json"
printf '[]' >"$f/none.json"

export SCOOT_INSTALL_DOWNLOAD_URL="file://$f/dl" SCOOT_INSTALL_RAW_URL="file://$f/raw"
unset XDG_CONFIG_HOME XDG_DATA_HOME
run_in() {
    h=$1
    shift
    HOME=$h SCOOT_INSTALL_RELEASES_URL=${releases:-file://$f/tags.json} sh "$installer" "$@" >"$work/out" 2>&1
}

# 1. Tagged release, a look, user install.
h=$work/h1
run_in "$h" --look ginger-night || { cat "$work/out"; fail "tagged install"; }
[ "$("$h/.local/bin/scoot")" = "scoot 0.1.0" ] || fail "scoot is not the tagged build"
[ "$("$h/.local/bin/scootbar")" = "scootbar 0.2.0" ] || fail "scootbar is not its own tagged build"
grep -q "ExecStart=$h/.local/bin/scoot --tty" "$h/.config/systemd/user/scoot.service" || fail "scoot.service binary path"
grep -q "ExecStart=$h/.local/bin/scootbar daemon" "$h/.config/systemd/user/scootbar.service" || fail "scootbar.service binary path"
grep -q "WantedBy=scoot-session.target" "$h/.config/systemd/user/scootbar.service" || fail "bar not bound to scoot's own session"
! grep -q "graphical-session" "$h/.config/systemd/user/scootbar.service" || fail "bar would start in other desktops"
[ -f "$h/.config/xdg-desktop-portal/scoot-portals.conf" ] || fail "portal config"
[ -f "$h/.config/scoot/wallpaper.png" ] && [ -x "$h/.config/scoot/cpu.sh" ] || fail "look files"
grep -q "command = \[\"$h/.config/scoot/cpu.sh\"\]" "$h/.config/scoot/bar.toml" || fail "bar script not by full path"
[ ! -e "$h/.local/bin/cpu.sh" ] || fail "look script dropped into the bin directory"
grep -q 'image = "wallpaper.png"' "$h/.config/scoot/config.toml" || fail "look config"
ok "tagged install with a look"

# 2. A second look never overwrites the first, and a re-run updates
#    its own files.
echo mine >"$h/.config/scoot/config.toml"
run_in "$h" --look moonrise || fail "second look run"
[ "$(cat "$h/.config/scoot/config.toml")" = mine ] || fail "user config was overwritten"
ok "existing config kept"

# 2b. A file the script did not install is never replaced.
h=$work/h5
mkdir -p "$h/.local/bin"
echo foreign >"$h/.local/bin/scoot"
if run_in "$h"; then fail "a foreign scoot was replaced"; fi
grep -q "was not installed by this script" "$work/out" || fail "foreign-file message"
[ "$(cat "$h/.local/bin/scoot")" = foreign ] && [ ! -e "$h/.local/bin/scootbg" ] || fail "refusal wrote files"
ok "foreign files refused before anything is written"
h=$work/h1

# 3. Uninstall removes what was installed and keeps the config.
run_in "$h" --uninstall || fail "uninstall"
[ ! -e "$h/.local/bin/scoot" ] && [ ! -e "$h/.config/systemd/user/scoot.service" ] || fail "uninstall left files"
[ -f "$h/.config/scoot/config.toml" ] || fail "uninstall removed the config"
ok "uninstall"

# 3b. A run that fails halfway still records what it placed.
h=$work/h6
mkdir -p "$h/.config/systemd"
echo blocker >"$h/.config/systemd/user"
if run_in "$h"; then fail "install into a blocked unit dir succeeded"; fi
grep -Fxq "$h/.local/bin/scoot" "$h/.local/share/scoot/install-manifest" || fail "partial run left no record"
rm "$h/.config/systemd/user"
run_in "$h" --uninstall || fail "uninstall after a partial run"
[ ! -e "$h/.local/bin/scoot" ] || fail "partial run's files not removed"
ok "partial run recorded and removable"

# 4. No tag yet: nightly, and a relative wallpaper outside the look dir.
h=$work/h2
releases=file://$f/none.json run_in "$h" --look moonrise || { cat "$work/out"; fail "nightly fallback"; }
grep -q "installing the nightly build" "$work/out" || fail "nightly fallback not announced"
grep -q "Installing scoot nightly 0.1.42+rev-0123456" "$work/out" || fail "nightly not labeled with its rolling version"
[ "$("$h/.local/bin/scoot")" = "scoot nightly" ] || fail "fallback is not nightly"
[ "$(cat "$h/.config/scoot/wallpaper.png")" = moon ] || fail "relative wallpaper path"
ok "nightly fallback"

# 5. A file that fails its checksum installs nothing.
echo tampered >>"$f/dl/scoot-v0.1.0/scootbg-0.1.0-$arch"
h=$work/h3
if run_in "$h"; then fail "a tampered download installed"; fi
grep -q "does not match its checksum" "$work/out" || fail "tamper message"
[ ! -e "$h/.local/bin/scoot" ] || fail "tampered run installed scoot"
ok "checksum refusal"

# 6. Dry run changes nothing; bad options are refused.
h=$work/h4
releases=file://$f/none.json run_in "$h" --dry-run || fail "dry run"
[ ! -e "$h/.local" ] || fail "dry run wrote files"
if run_in "$h" --look nope; then fail "unknown look accepted"; fi
if run_in "$h" --nightly --version 1.0.0; then fail "--nightly --version accepted"; fi
ok "dry run and refusals"

printf 'test-install: all passed\n'
