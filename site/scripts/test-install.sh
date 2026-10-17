#!/bin/sh
# Installer gate: runs public/install.sh against a fake release tree
# (file:// URLs, stub binaries, stub session files), so the script's
# paths, checksum refusal, no-tag failure, up-to-date rerun, --check,
# --force, look handling (a refused apply migrates nothing, symlinks are
# skipped, the swap is atomic, pre-looks releases degrade with one
# note), uninstall and missing-library/font report are proved without
# the network. shellcheck runs first when it is on PATH
# (CI provides it). `pnpm test-install`; CI's docs-site job.
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
stub() { printf '#!/bin/sh\necho "%s"\n' "$1" >"$2" && chmod +x "$2"; }
# The fake scoot answers --version and emulates `look apply`: it logs
# every call under the test HOME and applies (or refuses, when
# SCOOT_STUB_APPLY_FAIL names the reason), so the gate proves the
# installer's catalog, migration and apply flow without real binaries.
scoot_stub() {
    cat >"$1" <<EOF
#!/bin/sh
echo "scoot-stub \$*" >>"\$HOME/stub-scoot-calls"
if [ "\${1:-}" = look ] && [ "\${2:-}" = apply ]; then
    if [ -n "\${SCOOT_STUB_APPLY_FAIL:-}" ]; then
        echo "refused: \$SCOOT_STUB_APPLY_FAIL" >&2
        exit 1
    fi
    echo "applied: \$3"
    echo "shadowed keys: \${SCOOT_STUB_SHADOWED:-none}"
    exit 0
fi
echo "$2"
EOF
    chmod +x "$1"
}
mkdir -p "$f/dl/scoot-v0.1.0" "$f/dl/scootbar-v0.2.0" "$f/dl/v0.3.0" "$f/dl/v0.2.1"
scoot_stub "$f/dl/scoot-v0.1.0/scoot-0.1.0-$arch" "scoot 0.1.0"
stub "scootbg 0.1.0" "$f/dl/scoot-v0.1.0/scootbg-0.1.0-$arch"
stub "scootbar 0.2.0" "$f/dl/scootbar-v0.2.0/scootbar-0.2.0-$arch"
scoot_stub "$f/dl/v0.3.0/scoot-0.3.0-$arch" "scoot 0.3.0"
stub "scootbg 0.3.0" "$f/dl/v0.3.0/scootbg-0.3.0-$arch"
stub "scootbar 0.3.0" "$f/dl/v0.3.0/scootbar-0.3.0-$arch"
# v0.2.1 ships no looks and its scoot knows no `look apply`: plain stubs,
# so any apply attempt would show up as a missing stub-scoot-calls file
# check below rather than a refusal.
stub "scoot 0.2.1" "$f/dl/v0.2.1/scoot-0.2.1-$arch"
stub "scootbg 0.2.1" "$f/dl/v0.2.1/scootbg-0.2.1-$arch"
stub "scootbar 0.2.1" "$f/dl/v0.2.1/scootbar-0.2.1-$arch"
for d in "$f"/dl/*; do (cd "$d" && sha256sum -- * >SHA256SUMS); done

for ref in scoot-v0.1.0 v0.3.0; do
    r=$f/raw/$ref
    mkdir -p "$r/resources/systemd/user" "$r/packaging/deb" "$r/docs/assets/wallpapers"
    printf '#!/bin/sh\n' >"$r/resources/scoot-session"
    printf '[preferred]\n' >"$r/resources/scoot-portals.conf"
    printf '[Service]\nExecStart=@SCOOT_BIN@ --tty\n' >"$r/resources/systemd/user/scoot.service"
    printf '[Unit]\n' >"$r/resources/systemd/user/scoot-session.target"
    printf '[Unit]\n' >"$r/resources/systemd/user/scoot-shutdown.target"
    printf '[Desktop Entry]\nExec=/usr/bin/scoot-session\n' >"$r/packaging/deb/scoot.desktop"
    printf '[Unit]\nPartOf=graphical-session.target\nAfter=graphical-session.target\n[Service]\nExecStart=/usr/bin/scootbar daemon\n[Install]\nWantedBy=graphical-session.target\n' >"$r/packaging/deb/scootbar.service"
    for look in ginger-night moonrise music-desk radial-burst vinyl-sunset; do
        mkdir -p "$r/looks/$look" "$r/docs/examples/$look"
        # The catalog source: a small but shaped look.toml, with an
        # image for every look but vinyl-sunset (a flat color) and
        # per-app overrides for the migration to strip.
        {
            printf 'schema = 1\nname = "%s"\ntitle = "%s"\n' "$look" "$look"
            printf 'description = "Fake look for the installer gate."\n'
            printf 'polarity = "dark"\nlicense = "MIT"\n'
            printf '\n[palette]\nbackground = "#0E0E0E"\nforeground = "#F5EAD6"\n'
            printf 'dim = "#8F7A63"\naccent = "#E57F29"\nurgent = "#E05A4E"\n'
            if [ "$look" = vinyl-sunset ]; then
                printf '\n[wallpaper]\ncolor = "#271A1F"\n'
            else
                printf '\n[wallpaper]\nimage = "wallpaper.png"\nmode = "fill"\n'
                printf '%s wallpaper\n' "$look" >"$r/looks/$look/wallpaper.png"
            fi
            printf '\n[apps.scootbar]\n"bar.padding" = 10\n'
            if [ "$look" = radial-burst ]; then
                printf '"bar.spacing" = 14\n'
            else
                printf '"bar.separator" = 1\n"bar.spacing" = 8\n'
                printf '"workspaces.inactive-color" = "#8F7A63"\n'
            fi
        } >"$r/looks/$look/look.toml"
        # The old installer's template: whole example configs with one
        # look's values. The image line varies per look (a relative
        # file, a store-style path, a home path), and radial-burst's
        # bar names no helper scripts.
        case "$look" in
        ginger-night) image='image = "wallpaper.png"' ;;
        moonrise) image='image = "../../assets/wallpapers/moonrise.png"' ;;
        music-desk) image='image = "../../assets/wallpapers/music-desk.png"' ;;
        radial-burst) image='image = "../../assets/wallpapers/radial-burst.png"' ;;
        vinyl-sunset) image='image = "~/Pictures/wallpapers/vinyl-sunset.png"' ;;
        esac
        {
            printf '[layout]\ngap = 20\n'
            printf '\n[appearance]\nbackground_color = "#0E0E0E"\ncorner_radius = 14\n'
            printf 'focus_ring_width = 2\nfocus_ring_inactive_width = 2\n'
            printf 'focus_ring_active_color = "#FF9A30"\nfocus_ring_inactive_color = "#4E2913"\n'
            printf '\n[wallpaper]\n%s\nmode = "fill"\n' "$image"
        } >"$r/docs/examples/$look/scoot.toml"
        {
            printf 'left = ["workspaces"]\ncenter = ["clock"]\nright = ["cpu"]\n'
            printf '\n[bar]\nheight = 38\nmargin = "10,14"\nradius = 14\nopacity = 0.82\n'
            printf 'font-size = 13\npadding = 10\nspacing = 8\n'
            if [ "$look" != radial-burst ]; then printf 'separator = 1\n'; fi
            printf '\n[colors]\nbackground = "#0E0E0E"\nforeground = "#F5EAD6"\n'
            printf 'accent = "#E57F29"\nhover = "#FFB14B"\ndim = "#8F7A63"\nurgent = "#E05A4E"\n'
            printf '\n[workspaces]\npill-shape = "circle"\n'
            if [ "$look" != radial-burst ]; then printf 'inactive-color = "#8F7A63"\n'; fi
            printf '\n[exec.cpu]\n'
            if [ "$look" = radial-burst ]; then
                printf 'command = ["top"]\n'
            else
                printf 'command = ["cpu.sh"]\n'
            fi
        } >"$r/docs/examples/$look/bar.toml"
        # Static terminal leaves for one look only (the rest 404, which
        # the installer tolerates): the catalog install fetches them.
        if [ "$look" = ginger-night ]; then
            mkdir -p "$r/looks/$look/apps"
            printf '"ui.background" = {}\n"ui.text" = "#FF0000"\n"ui.selection" = "#FF0000"\n' >"$r/looks/$look/apps/helix.toml"
            printf '# fake\ntheme[main_fg]="#FF0000"\n' >"$r/looks/$look/apps/btop.theme"
            printf 'add_newline = false\n' >"$r/looks/$look/apps/starship.toml"
        fi
    done
    printf '#!/bin/sh\n' >"$r/docs/examples/ginger-night/cpu.sh"
    printf '#!/bin/sh\n' >"$r/docs/examples/ginger-night/load.sh"
    printf 'moon\n' >"$r/docs/assets/wallpapers/moonrise.png"
done
# v0.2.1 has no looks/ tree and no docs/examples: the session files only,
# like the real release before looks shipped in v0.3.0.
r=$f/raw/v0.2.1
mkdir -p "$r/resources/systemd/user" "$r/packaging/deb"
printf '#!/bin/sh\n' >"$r/resources/scoot-session"
printf '[preferred]\n' >"$r/resources/scoot-portals.conf"
printf '[Service]\nExecStart=@SCOOT_BIN@ --tty\n' >"$r/resources/systemd/user/scoot.service"
printf '[Unit]\n' >"$r/resources/systemd/user/scoot-session.target"
printf '[Unit]\n' >"$r/resources/systemd/user/scoot-shutdown.target"
printf '[Desktop Entry]\nExec=/usr/bin/scoot-session\n' >"$r/packaging/deb/scoot.desktop"
printf '[Unit]\nPartOf=graphical-session.target\nAfter=graphical-session.target\n[Service]\nExecStart=/usr/bin/scootbar daemon\n[Install]\nWantedBy=graphical-session.target\n' >"$r/packaging/deb/scootbar.service"
printf '[{"tag_name": "scootbar-v0.2.0"}, {"tag_name": "scoot-v0.1.0", "body": "compare scoot-v9.9.9 and scootbar-v9.9.9"}]' >"$f/tags.json"
printf '[{"tag_name": "v0.3.0"}, {"tag_name": "scootbar-v0.2.0"}, {"tag_name": "scoot-v0.1.0"}]' >"$f/tags-unified.json"
printf '[{"tag_name": "scoot-v0.1.1"}, {"tag_name": "scootbar-v0.2.0"}, {"tag_name": "scoot-v0.1.0"}]' >"$f/tags-new.json"
printf '[]' >"$f/none.json"
printf '[{"tag_name": "v0.2.1"}]' >"$f/tags-old.json"

export SCOOT_INSTALL_DOWNLOAD_URL="file://$f/dl" SCOOT_INSTALL_RAW_URL="file://$f/raw"
unset XDG_CONFIG_HOME XDG_DATA_HOME

# Portable uname/getconf: on macOS the installer would refuse Darwin and
# miss glibc, so shadow both (anything else passes through to the real
# tools). uname answers the real machine with a Linux kernel name; getconf
# answers the glibc floor the script requires. On a Linux box at or past
# that floor the stubs change nothing, so the suite proves the same
# behavior there.
real_uname=$(command -v uname)
real_getconf=$(command -v getconf || true)
mkdir -p "$work/bin"
# shellcheck disable=SC2016 # the $1 must stay literal: it runs in the stub, not here.
printf '#!/bin/sh\nif [ "${1:-}" = -s ]; then echo Linux; else exec "%s" "$@"; fi\n' "$real_uname" >"$work/bin/uname"
chmod +x "$work/bin/uname"
if [ -n "$real_getconf" ]; then
    # shellcheck disable=SC2016 # same: literal $1 for the stub.
    printf '#!/bin/sh\nif [ "${1:-}" = GNU_LIBC_VERSION ]; then echo "glibc 2.36"; else exec "%s" "$@"; fi\n' "$real_getconf" >"$work/bin/getconf"
    chmod +x "$work/bin/getconf"
fi
PATH="$work/bin:$PATH"
# Fake user manager: a CI runner has a real `systemctl --user`, which would
# enable units outside the test HOME (and fail when the unit is not in the
# real search path). Shadow it with a stub that records calls and tracks
# enabled units in a file, so the suite asserts the enable/disable contract
# without touching the host.
mkdir -p "$work/fake-systemd"
# shellcheck disable=SC2016 # every $ here runs in the stub, not in this file.
printf '#!/bin/sh\n# fake systemctl for the installer gate: log, never touch the host.\nlog="%s/systemctl.log"\nstatedir="%s/fake-systemd"\necho "systemctl $*" >>"$log"\nif [ "${1:-}" = --user ]; then shift; fi\ncase "${1:-}" in\nshow-environment) exit 0 ;;\nis-enabled) [ -f "$statedir/$2.enabled" ] ;;\nenable) touch "$statedir/$2.enabled" ;;\ndisable) rm -f "$statedir/$2.enabled" ;;\ndaemon-reload) exit 0 ;;\n*) exit 0 ;;\nesac\n' "$work" "$work" >"$work/bin/systemctl"
chmod +x "$work/bin/systemctl"
: >"$work/systemctl.log"
run_in() {
    (
        h=$1
        shift
        HOME="$h" SCOOT_INSTALL_RELEASES_URL=${releases:-file://$f/tags.json} sh "$installer" "$@" >"$work/out" 2>&1
    )
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
# --look installs the shipped catalog (recorded, so --uninstall removes
# it), writes no whole configs, and applies the look.
for l in ginger-night moonrise music-desk radial-burst vinyl-sunset; do
    [ -f "$h/.local/share/scoot/looks/$l/look.toml" ] || fail "no catalog $l"
done
[ -f "$h/.local/share/scoot/looks/moonrise/wallpaper.png" ] || fail "no catalog wallpaper"
[ ! -e "$h/.local/share/scoot/looks/vinyl-sunset/wallpaper.png" ] || fail "the color look fetched a wallpaper"
# The static terminal leaves: fetched where the release ships them,
# tolerated where it does not.
[ -f "$h/.local/share/scoot/looks/ginger-night/apps/helix.toml" ] || fail "no catalog helix leaf"
[ -f "$h/.local/share/scoot/looks/ginger-night/apps/btop.theme" ] || fail "no catalog btop leaf"
[ -f "$h/.local/share/scoot/looks/ginger-night/apps/starship.toml" ] || fail "no catalog starship leaf"
[ ! -e "$h/.local/share/scoot/looks/moonrise/apps/helix.toml" ] || fail "a missing leaf was invented"
grep -Fq "$h/.local/share/scoot/looks/ginger-night/apps/helix.toml" "$h/.local/share/scoot/install-manifest" || fail "leaf not in manifest"
[ ! -e "$h/.config/scoot/config.toml" ] && [ ! -e "$h/.config/scoot/bar.toml" ] || fail "whole configs written"
grep -q "look apply ginger-night" "$h/stub-scoot-calls" || fail "apply not invoked"
grep -q "applied: ginger-night" "$work/out" || fail "no apply reply"
grep -Fq "$h/.local/share/scoot/looks/ginger-night/look.toml" "$h/.local/share/scoot/install-manifest" || fail "catalog not in manifest"
grep -q "systemctl --user enable scootbar.service" "$work/systemctl.log" || fail "bar unit not enabled"
grep -q "systemctl --user daemon-reload" "$work/systemctl.log" || fail "no daemon-reload after install"
rec=$h/.local/share/scoot/install-version
grep -q "^channel=release$" "$rec" || fail "record has no channel"
grep -q "^version=0.1.0$" "$rec" || fail "record has no version"
grep -q "^tag=scoot-v0.1.0$" "$rec" || fail "record has no tag"
grep -q "^bar_tag=scootbar-v0.2.0$" "$rec" || fail "record has no bar tag"
grep -q "^date=[0-9-]*T[0-9:]*Z$" "$rec" || fail "record has no date"
ok "tagged install with a look and a version record"

# 1b. A re-run with nothing new says so and downloads nothing: point the
#     download and raw URLs at nowhere and it must still exit 0.
if ! run_in "$h"; then cat "$work/out"; fail "re-run"; fi
grep -q "already up to date (0.1.0)" "$work/out" || fail "re-run says no up-to-date"
SCOOT_INSTALL_DOWNLOAD_URL="file://$f/dl-gone" SCOOT_INSTALL_RAW_URL="file://$f/raw-gone" run_in "$h" || { cat "$work/out"; fail "re-run downloads"; }
export SCOOT_INSTALL_DOWNLOAD_URL="file://$f/dl" SCOOT_INSTALL_RAW_URL="file://$f/raw"
grep -q "already up to date (0.1.0)" "$work/out" || fail "offline re-run is not up to date"
ok "re-run is already up to date without downloading"

# 1b2. The version record is destination-aware: a re-run with a
#      different --prefix installs there instead of saying up to date.
pa=$work/prefix-a
pb=$work/prefix-b
run_in "$work/h-prefix" --prefix "$pa" || { cat "$work/out"; fail "prefix A install"; }
[ -x "$pa/bin/scoot" ] || fail "prefix A has no scoot"
grep -Fq "bin_dir=$pa/bin" "$work/h-prefix/.local/share/scoot/install-version" || fail "record has no bin_dir"
run_in "$work/h-prefix" --prefix "$pb" || { cat "$work/out"; fail "prefix B re-run"; }
[ -x "$pb/bin/scoot" ] || fail "prefix B got no install (prefix-blind up-to-date)"
ok "a different --prefix installs instead of claiming up to date"

# 1b3. A re-run whose binaries are gone reinstalls instead of claiming
#      to be current.
rm "$pb/bin/scoot" "$pb/bin/scootbg" "$pb/bin/scootbar"
if ! run_in "$work/h-prefix" --prefix "$pb"; then cat "$work/out"; fail "reinstall after binaries deleted"; fi
grep -q "Installing scoot" "$work/out" || fail "deleted binaries still claimed up to date"
[ -x "$pb/bin/scoot" ] || fail "deleted binaries were not reinstalled"
ok "missing binaries reinstall instead of claiming up to date"

# 1b4. An up-to-date re-run with --look still applies the look (and
#      downloads no binaries to do it): the catalog is already recorded,
#      so no look file downloads either.
h=$work/h-look
run_in "$h" || { cat "$work/out"; fail "look-gate install"; }
[ ! -e "$h/.config/scoot/config.toml" ] || fail "look-gate pre-state has config"
SCOOT_INSTALL_DOWNLOAD_URL="file://$f/dl-gone" run_in "$h" --look moonrise || { cat "$work/out"; fail "up-to-date --look"; }
export SCOOT_INSTALL_DOWNLOAD_URL="file://$f/dl"
grep -q "already up to date (0.1.0)" "$work/out" || fail "up-to-date --look says no up-to-date"
grep -q "look apply moonrise" "$h/stub-scoot-calls" || fail "up-to-date --look applied nothing"
[ ! -e "$h/.config/scoot/config.toml" ] || fail "up-to-date --look wrote a whole config"
h=$work/h1
ok "up-to-date re-run still applies --look"

# 1c. --check: exit 0 when current, 10 when an update exists, 1 on error.
if ! run_in "$h" --check; then cat "$work/out"; fail "--check when current"; fi
grep -q "installed: 0.1.0 (scoot-v0.1.0)" "$work/out" || fail "--check prints no installed version"
grep -q "latest: 0.1.0 (scoot-v0.1.0)" "$work/out" || fail "--check prints no latest version"
if releases=file://$f/tags-new.json run_in "$h" --check; then fail "--check missed an update"; else code=$?; fi
[ "$code" = 10 ] || fail "--check update exit is $code, not 10"
grep -q "latest: 0.1.1 (scoot-v0.1.1)" "$work/out" || fail "--check prints no newer latest"
unset releases
if run_in "$work/h-check-none" --check; then fail "--check with nothing installed exited 0"; else code=$?; fi
[ "$code" = 1 ] || fail "--check empty exit is $code, not 1"
grep -q "installed: none" "$work/out" || fail "--check empty prints no none"
ok "--check exit codes"

# 1d. --force reinstalls even when current.
run_in "$h" --force || { cat "$work/out"; fail "--force"; }
grep -q "Installing scoot scoot-v0.1.0" "$work/out" || fail "--force did not reinstall"
[ "$("$h/.local/bin/scoot")" = "scoot 0.1.0" ] || fail "--force broke the install"
ok "--force reinstalls"

# 1e. A refused apply fails the run but not the install: the binaries,
#     the catalog and the version record are all in place, and the error
#     says to fix the cause and re-run with --look.
h=$work/h-fail
export SCOOT_STUB_APPLY_FAIL="font Fake is not installed"
if run_in "$h" --look ginger-night; then fail "a refused apply exited 0"; fi
unset SCOOT_STUB_APPLY_FAIL
grep -q "was not applied" "$work/out" || fail "no refused-apply error"
grep -q "re-run with --look" "$work/out" || fail "no re-run pointer"
[ -x "$h/.local/bin/scoot" ] || fail "refused apply lost the binaries"
[ -f "$h/.local/share/scoot/looks/ginger-night/look.toml" ] || fail "refused apply lost the catalog"
grep -q "^tag=scoot-v0.1.0$" "$h/.local/share/scoot/install-version" || fail "refused apply lost the version record"
h=$work/h1
ok "a refused apply fails last, after the install"

# 1e2. A refused apply migrates nothing: configs still byte-identical to
#      an earlier --look run's output are left alone, with no migration
#      note, and the error still points at a re-run with --look.
h=$work/h-fail-mig
mkdir -p "$h/.config/scoot"
sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.png\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/scoot.toml" >"$h/.config/scoot/config.toml"
sed "s|\"cpu.sh\"|\"$h/.config/scoot/cpu.sh\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/bar.toml" >"$h/.config/scoot/bar.toml"
cp "$h/.config/scoot/config.toml" "$work/fail-mig-config.before"
cp "$h/.config/scoot/bar.toml" "$work/fail-mig-bar.before"
export SCOOT_STUB_APPLY_FAIL="font Fake is not installed"
if run_in "$h" --look moonrise; then fail "a refused apply with migratable configs exited 0"; fi
unset SCOOT_STUB_APPLY_FAIL
grep -q "was not applied" "$work/out" || fail "no refused-apply error with migratable configs"
grep -q "re-run with --look" "$work/out" || fail "no re-run pointer with migratable configs"
if grep -Fq "migrated $h/.config/scoot/config.toml" "$work/out"; then fail "refused apply migrated config.toml"; fi
if grep -Fq "migrated $h/.config/scoot/bar.toml" "$work/out"; then fail "refused apply migrated bar.toml"; fi
cmp -s "$h/.config/scoot/config.toml" "$work/fail-mig-config.before" || fail "refused apply touched config.toml"
cmp -s "$h/.config/scoot/bar.toml" "$work/fail-mig-bar.before" || fail "refused apply touched bar.toml"
h=$work/h1
ok "a refused apply leaves migratable configs byte-identical"

# 2. A second look never overwrites the first, and a re-run updates
#    its own files.
mkdir -p "$h/.config/scoot"
echo mine >"$h/.config/scoot/config.toml"
run_in "$h" --look moonrise --force || fail "second look run"
[ "$(cat "$h/.config/scoot/config.toml")" = mine ] || fail "user config was overwritten"
grep -q "look apply moonrise" "$h/stub-scoot-calls" || fail "apply not invoked on an existing install"
ok "existing config kept"

# 2d. A config still byte-identical to an earlier --look run's output has
#     its look keys removed (layout kept), so the layer shows through.
#     Moonrise exercises both rewrites (a relative wallpaper, helper
#     scripts by full path); radial-burst exercises neither (a store
#     path, no script references).
h=$work/h-mig
mkdir -p "$h/.config/scoot"
sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.png\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/scoot.toml" >"$h/.config/scoot/config.toml"
sed "s|\"cpu.sh\"|\"$h/.config/scoot/cpu.sh\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/bar.toml" >"$h/.config/scoot/bar.toml"
run_in "$h" --look moonrise || { cat "$work/out"; fail "migration run"; }
grep -q "migrated $h/.config/scoot/config.toml" "$work/out" || fail "no config migration note"
grep -q "migrated $h/.config/scoot/bar.toml" "$work/out" || fail "no bar migration note"
c=$h/.config/scoot/config.toml
if grep -q "^\[appearance\]" "$c" || grep -q "^\[wallpaper\]" "$c"; then fail "look sections left in $c"; fi
grep -q "^gap = 20$" "$c" || fail "layout lost from $c"
b=$h/.config/scoot/bar.toml
if grep -q "^\[colors\]" "$b"; then fail "colors left in $b"; fi
if grep -q "inactive-color" "$b" || grep -q "^opacity" "$b" || grep -q "^padding" "$b" || grep -q "^separator" "$b" || grep -q "^spacing" "$b"; then
    fail "look keys left in $b"
fi
grep -q "^height = 38$" "$b" || fail "geometry lost from $b"
grep -q "^pill-shape" "$b" || fail "module style lost from $b"
grep -q "command = \[\"$h/.config/scoot/cpu.sh\"\]" "$b" || fail "exec command lost from $b"
# Radial-burst exercises the other rewrites (a store-style image still
# rewritten, a bar with no helper scripts passed through untouched).
h=$work/h-mig2
mkdir -p "$h/.config/scoot"
sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.png\"|" "$f/raw/scoot-v0.1.0/docs/examples/radial-burst/scoot.toml" >"$h/.config/scoot/config.toml"
cp "$f/raw/scoot-v0.1.0/docs/examples/radial-burst/bar.toml" "$h/.config/scoot/bar.toml"
run_in "$h" --look radial-burst || { cat "$work/out"; fail "migration run without rewrites"; }
if grep -q "^\[appearance\]" "$h/.config/scoot/config.toml"; then fail "look sections left without rewrites"; fi
if grep -q "^\[colors\]" "$h/.config/scoot/bar.toml" || grep -q "^spacing" "$h/.config/scoot/bar.toml"; then
    fail "look keys left without rewrites"
fi
grep -q 'command = \["top"\]' "$h/.config/scoot/bar.toml" || fail "plain command lost without rewrites"
h=$work/h1
ok "byte-identical configs migrate their look keys away"

# 2d2. A migratable config that is a symlink is left alone, never written
#      through: the link still points where it did, its target keeps its
#      bytes, and the output says so. The regular migratable file beside
#      it still migrates.
h=$work/h-symlink
mkdir -p "$h/.config/scoot" "$work/sym-target"
sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.png\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/scoot.toml" >"$work/sym-target/real-config.toml"
cp "$work/sym-target/real-config.toml" "$work/sym-target-config.before"
ln -s "$work/sym-target/real-config.toml" "$h/.config/scoot/config.toml"
sed "s|\"cpu.sh\"|\"$h/.config/scoot/cpu.sh\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/bar.toml" >"$h/.config/scoot/bar.toml"
run_in "$h" --look moonrise || { cat "$work/out"; fail "symlink run"; }
[ -L "$h/.config/scoot/config.toml" ] || fail "symlink was replaced"
[ "$(readlink "$h/.config/scoot/config.toml")" = "$work/sym-target/real-config.toml" ] || fail "symlink retargeted"
cmp -s "$work/sym-target/real-config.toml" "$work/sym-target-config.before" || fail "symlink target was written through"
grep -Fq "left $h/.config/scoot/config.toml alone" "$work/out" || fail "no symlink note"
if grep -q "^\[colors\]" "$h/.config/scoot/bar.toml"; then fail "regular file beside a symlink did not migrate"; fi
h=$work/h1
ok "a symlinked config is left alone, never written through"

# 2d3. Migration swaps the stripped config in atomically: the migrated
#      file is a new directory entry (a truncating rewrite would keep the
#      inode), keeps its permission bits, and leaves no temp files.
h=$work/h-atomic
mkdir -p "$h/.config/scoot"
sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.png\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/scoot.toml" >"$h/.config/scoot/config.toml"
sed "s|\"cpu.sh\"|\"$h/.config/scoot/cpu.sh\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/bar.toml" >"$h/.config/scoot/bar.toml"
chmod 600 "$h/.config/scoot/config.toml"
ino_before=$(stat -c %i "$h/.config/scoot/config.toml" 2>/dev/null || stat -f %i "$h/.config/scoot/config.toml")
run_in "$h" --look moonrise || { cat "$work/out"; fail "atomic run"; }
ino_after=$(stat -c %i "$h/.config/scoot/config.toml" 2>/dev/null || stat -f %i "$h/.config/scoot/config.toml")
[ "$ino_before" != "$ino_after" ] || fail "migration rewrote in place instead of swapping"
mode_after=$(stat -c %a "$h/.config/scoot/config.toml" 2>/dev/null || stat -f %Lp "$h/.config/scoot/config.toml" 2>/dev/null)
[ "$mode_after" = 600 ] || fail "migration changed mode 600 to $mode_after"
if ls "$h/.config/scoot"/.scoot-migrate.* >/dev/null 2>&1; then fail "migration left temp files"; fi
if grep -q "^\[appearance\]" "$h/.config/scoot/config.toml"; then fail "atomic run left look sections"; fi
h=$work/h1
ok "migration swaps atomically, keeping mode and leaving no temp files"

# 2e. An edited file (one byte off the old output) is left alone, and
#     the apply reply names the keys shadowing the look.
h=$work/h-edit
mkdir -p "$h/.config/scoot"
sed "s|^image *= *\"[^\"]*\"|image = \"wallpaper.png\"|" "$f/raw/scoot-v0.1.0/docs/examples/moonrise/scoot.toml" >"$h/.config/scoot/config.toml"
printf '\n# my own note\n' >>"$h/.config/scoot/config.toml"
before=$(cat "$h/.config/scoot/config.toml")
export SCOOT_STUB_SHADOWED="appearance.background_color"
run_in "$h" --look moonrise || { cat "$work/out"; fail "edited-file run"; }
unset SCOOT_STUB_SHADOWED
[ "$(cat "$h/.config/scoot/config.toml")" = "$before" ] || fail "edited file was touched"
grep -q "shadowed keys: appearance.background_color" "$work/out" || fail "shadowing keys not named"
h=$work/h1
ok "edited files are left alone with their shadowing keys named"

# 2b. A file the script did not install is never replaced.
h=$work/h5
mkdir -p "$h/.local/bin"
echo foreign >"$h/.local/bin/scoot"
if run_in "$h"; then fail "a foreign scoot was replaced"; fi
grep -q "was not installed by this script" "$work/out" || fail "foreign-file message"
[ "$(cat "$h/.local/bin/scoot")" = foreign ] && [ ! -e "$h/.local/bin/scootbg" ] || fail "refusal wrote files"
ok "foreign files refused before anything is written"

# 2c. The refusal also fires when the paths contain spaces, and a clean
#     install works there too (the check never word-splits destinations).
h="$work/h space"
mkdir -p "$h/.local/bin"
echo foreign >"$h/.local/bin/scoot"
if run_in "$h"; then fail "a foreign scoot under a spaced HOME was replaced"; fi
grep -q "was not installed by this script" "$work/out" || fail "spaced-HOME refusal message"
[ "$(cat "$h/.local/bin/scoot")" = foreign ] || fail "spaced-HOME refusal wrote files"
rm "$h/.local/bin/scoot"
run_in "$h" || { cat "$work/out"; fail "install under a spaced HOME"; }
[ "$("$h/.local/bin/scoot")" = "scoot 0.1.0" ] || fail "spaced-HOME install is not the tagged build"
ok "spaced HOME: refusal fires, install works"
h=$work/h1

# 3. Uninstall removes what was installed (record included) and keeps
#    the config.
run_in "$h" --uninstall || fail "uninstall"
[ ! -e "$h/.local/bin/scoot" ] && [ ! -e "$h/.config/systemd/user/scoot.service" ] || fail "uninstall left files"
[ ! -e "$h/.local/share/scoot/install-version" ] || fail "uninstall left the version record"
[ -f "$h/.config/scoot/config.toml" ] || fail "uninstall removed the config"
grep -q "systemctl --user disable scootbar.service" "$work/systemctl.log" || fail "bar unit not disabled on uninstall"
ok "uninstall"

# 3c. Uninstall removes the look catalog too: no look files, and no
#     emptied look directories. A foreign file in the tree keeps its
#     directory (rmdir removes solely empty ones).
h=$work/h-look
mkdir -p "$h/.local/share/scoot/looks/foreign"
echo foreign >"$h/.local/share/scoot/looks/foreign/kept"
run_in "$h" --uninstall || { cat "$work/out"; fail "uninstall with looks"; }
[ ! -e "$h/.local/share/scoot/looks/ginger-night" ] && [ ! -e "$h/.local/share/scoot/looks/moonrise" ] || fail "uninstall left look files"
[ ! -e "$h/.local/share/scoot/looks/music-desk" ] && [ ! -e "$h/.local/share/scoot/looks/radial-burst" ] || fail "uninstall left look files"
[ ! -e "$h/.local/share/scoot/looks/vinyl-sunset" ] || fail "uninstall left look files"
[ "$(cat "$h/.local/share/scoot/looks/foreign/kept")" = foreign ] || fail "uninstall removed a foreign file"
h=$work/h1
ok "uninstall removes the look catalog"

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

# 4. No tag yet: a loud failure pointing at the Nix flake, never main.
h=$work/h2
if releases=file://$f/none.json run_in "$h" --look moonrise; then fail "tagless install succeeded"; fi
unset releases
grep -q "no tagged release yet" "$work/out" || fail "tagless failure names no missing tag"
grep -q "Nix flake" "$work/out" || fail "tagless failure names no Nix flake"
[ ! -e "$h/.local" ] || fail "tagless run wrote files"
ok "no tag fails loudly"

# 4b. --nightly is retired: one line, tagged releases only, Nix pointer.
if run_in "$h" --nightly; then fail "--nightly accepted"; fi
grep -q "nightly channel was retired" "$work/out" || fail "--nightly names no retirement"
grep -q "tagged releases only" "$work/out" || fail "--nightly names no tagged-only rule"
grep -q "Nix flake" "$work/out" || fail "--nightly names no Nix flake"
[ ! -e "$h/.local" ] || fail "--nightly wrote files"
ok "--nightly retired"

# 4c. One unified tag installs everything on that version, and
#     --bar-version is retired with it.
h=$work/h7
releases=file://$f/tags-unified.json run_in "$h" || { cat "$work/out"; fail "unified install"; }
unset releases
[ "$("$h/.local/bin/scoot")" = "scoot 0.3.0" ] || fail "unified scoot is not 0.3.0"
[ "$("$h/.local/bin/scootbar")" = "scootbar 0.3.0" ] || fail "unified bar is not 0.3.0"
grep -q "^tag=v0.3.0$" "$h/.local/share/scoot/install-version" || fail "unified record has no tag"
if releases=file://$f/tags-unified.json run_in "$h" --bar-version 0.3.0; then fail "unified --bar-version accepted"; fi
unset releases
grep -q "retired" "$work/out" || fail "unified --bar-version names no retirement"
ok "unified tag"

# 4d. A release from before looks (v0.2.1: unified tag, no looks/ tree,
#     a scoot with no `look apply`) degrades: --look installs the
#     binaries with one plain note and exits 0, attempting no catalog and
#     no apply.
h=$work/h-old
releases=file://$f/tags-old.json run_in "$h" --look ginger-night || { cat "$work/out"; fail "pre-looks install"; }
unset releases
[ "$("$h/.local/bin/scoot")" = "scoot 0.2.1" ] || fail "pre-looks scoot is not 0.2.1"
[ "$("$h/.local/bin/scootbar")" = "scootbar 0.2.1" ] || fail "pre-looks bar is not 0.2.1"
grep -q "has no looks" "$work/out" || fail "pre-looks run names no missing looks"
grep -q "v0.3 or later" "$work/out" || fail "pre-looks run names no newer release"
if grep -q "could not download the look catalog" "$work/out"; then fail "pre-looks run failed on the catalog"; fi
[ ! -e "$h/.local/share/scoot/looks" ] || fail "pre-looks run fetched a catalog"
[ ! -e "$h/stub-scoot-calls" ] || fail "pre-looks run invoked the look-less scoot"
grep -q "^tag=v0.2.1$" "$h/.local/share/scoot/install-version" || fail "pre-looks record has no tag"
h=$work/h1
ok "a pre-looks release installs binaries with one note"

# 5. A file that fails its checksum installs nothing.
echo tampered >>"$f/dl/scoot-v0.1.0/scootbg-0.1.0-$arch"
h=$work/h3
if run_in "$h"; then fail "a tampered download installed"; fi
grep -q "does not match its checksum" "$work/out" || fail "tamper message"
[ ! -e "$h/.local/bin/scoot" ] || fail "tampered run installed scoot"
ok "checksum refusal"

# 6. Dry run changes nothing; bad options are refused. The unified tree
#    is used because test 5 tampered with the tagged scootbg above.
h=$work/h4
releases=file://$f/tags-unified.json run_in "$h" --dry-run || { cat "$work/out"; fail "dry run"; }
[ ! -e "$h/.local" ] || fail "dry run wrote files"
if releases=file://$f/tags-unified.json run_in "$h" --look nope; then fail "unknown look accepted"; fi
if run_in "$h" --check --uninstall; then fail "--check --uninstall accepted"; fi
if run_in "$h" --check --force; then fail "--check --force accepted"; fi
# The release body mentions scoot-v9.9.9, but no such tag exists: an
# explicit --version must match a tag_name, not any mention.
if releases=file://$f/tags-unified.json run_in "$h" --version 9.9.9; then fail "a body-text version was accepted"; fi
unset releases
grep -q "no release scoot-v9.9.9" "$work/out" || fail "body-text version message"
# A --version with regex characters must not match spuriously: it is
# compared as a fixed string and refused with a clean "no release".
if run_in "$h" --version '.*'; then fail "a regex version was accepted"; fi
grep -q "no release" "$work/out" || fail "regex version message"
ok "dry run and refusals"

# 7. A missing font adds the distro's font package (F1), with the reason
#    line and the non-interactive note (F2); the printed command itself
#    stays confirmation-based. SCOOT_INSTALL_OS_RELEASE and
#    SCOOT_INSTALL_FONTS pin the distro and the bar's candidate list so
#    these run the same on any host; ldd and foot are shadowed so the
#    library half of the report is deterministic too. The unified tree is
#    used because test 5 tampered with the tagged scootbg above.
printf 'ID=arch\n' >"$f/os-arch"
printf 'ID=fedora\n' >"$f/os-fedora"
printf 'ID=ubuntu\nID_LIKE=debian\n' >"$f/os-ubuntu"
printf 'ID=opensuse-tumbleweed\nID_LIKE="opensuse suse"\n' >"$f/os-suse"
printf '#!/bin/sh\necho "libinput.so.10 => not found"\n' >"$work/bin/ldd"
chmod +x "$work/bin/ldd"
printf '#!/bin/sh\nexit 0\n' >"$work/bin/foot"
chmod +x "$work/bin/foot"
for os in arch fedora ubuntu suse; do
    case "$os" in
    arch) want_cmd="sudo pacman -S --needed libinput ttf-dejavu" ;;
    fedora) want_cmd="sudo dnf install libinput dejavu-sans-fonts" ;;
    ubuntu) want_cmd="sudo apt install libinput10 fonts-dejavu-core" ;;
    suse) want_cmd="sudo zypper install libinput10 dejavu-fonts" ;;
    esac
    export SCOOT_INSTALL_OS_RELEASE="$f/os-$os" SCOOT_INSTALL_FONTS=/nonexistent/scoot-test-font.ttf
    if ! releases=file://$f/tags-unified.json run_in "$work/h-font-$os" --dry-run; then
        cat "$work/out"
        fail "dry run on $os"
    fi
    unset SCOOT_INSTALL_OS_RELEASE SCOOT_INSTALL_FONTS releases
    if ! grep -Fq "  $want_cmd" "$work/out"; then
        cat "$work/out"
        fail "no '$want_cmd' on $os"
    fi
    if ! grep -q "none of the usual font files is usable" "$work/out"; then fail "no font reason on $os"; fi
    if ! grep -q "Non-interactive: use apt -y, dnf -y, or pacman --noconfirm" "$work/out"; then fail "no non-interactive note on $os"; fi
    cmdline=$(grep -F "  $want_cmd" "$work/out")
    if printf '%s\n' "$cmdline" | grep -q -- ' -y\|--noconfirm'; then fail "printed command takes -y on $os"; fi
    if [ -e "$work/h-font-$os/.local" ]; then fail "dry run wrote files on $os"; fi
done
ok "missing font names the distro package, with reason and note"

# 8. A usable font adds no package (and --no-bar needs none either),
#    while a dry run still lists the missing libraries (N1).
# (-s: an empty file is unusable, as the bar's own loader refuses it.)
printf 'fake-font\n' >"$work/have-font.ttf"
export SCOOT_INSTALL_OS_RELEASE="$f/os-ubuntu" SCOOT_INSTALL_FONTS="$work/have-font.ttf"
if ! releases=file://$f/tags-unified.json run_in "$work/h-font-ok" --dry-run; then
    cat "$work/out"
    fail "dry run with a font"
fi
unset SCOOT_INSTALL_OS_RELEASE SCOOT_INSTALL_FONTS releases
if grep -q "fonts-dejavu-core" "$work/out"; then
    cat "$work/out"
    fail "font package listed despite a usable font"
fi
if grep -q "none of the usual font files" "$work/out"; then fail "font reason despite a usable font"; fi
if ! grep -Fq "  sudo apt install libinput10" "$work/out"; then
    cat "$work/out"
    fail "dry run lists no libraries"
fi
ok "a usable font adds no package; dry run still lists libraries"

export SCOOT_INSTALL_OS_RELEASE="$f/os-fedora" SCOOT_INSTALL_FONTS=/nonexistent/scoot-test-font.ttf
if ! releases=file://$f/tags-unified.json run_in "$work/h-no-bar" --dry-run --no-bar; then
    cat "$work/out"
    fail "dry run with --no-bar"
fi
unset SCOOT_INSTALL_OS_RELEASE SCOOT_INSTALL_FONTS releases
if grep -q "dejavu-sans-fonts" "$work/out"; then
    cat "$work/out"
    fail "font package listed with --no-bar"
fi
if grep -q "none of the usual font files" "$work/out"; then fail "font reason with --no-bar"; fi
ok "--no-bar needs no font"

# 9. A system uninstall under sudo names the invoking user's config (N3),
#    not root's; without sudo the message is unchanged. Needs a writable
#    /usr/local/share/scoot for the manifest the uninstall reads.
sys_manifest=/usr/local/share/scoot/install-manifest
if mkdir -p /usr/local/share/scoot 2>/dev/null && printf '%s\n' "$work/nowhere" >"$sys_manifest" 2>/dev/null; then
    h=$work/h-sudo-user
    export SUDO_USER=scoot-probe-user
    if ! run_in "$h" --system --uninstall --dry-run; then
        cat "$work/out"
        fail "system uninstall dry run"
    fi
    unset SUDO_USER
    if ! grep -Fq "Your config in /home/scoot-probe-user/.config/scoot is untouched" "$work/out"; then
        cat "$work/out"
        fail "uninstall names no invoking-user config"
    fi
    if grep -Fq "/root/.config/scoot is untouched" "$work/out"; then fail "uninstall names root's config under sudo"; fi
    if ! run_in "$h" --system --uninstall --dry-run; then
        cat "$work/out"
        fail "system uninstall dry run without sudo"
    fi
    if ! grep -Fq "Your config in $h/.config/scoot is untouched" "$work/out"; then
        cat "$work/out"
        fail "plain uninstall message changed"
    fi
    rm -f "$sys_manifest"
    ok "system uninstall names the invoking user's config"
else
    ok "system uninstall message skipped (no /usr/local/share/scoot write)"
fi

# 9b. Under sudo the look applies as the invoking user: the dry run
#     prints the apply with their name in it (the command itself never
#     executes here, so no privilege is needed).
h=$work/h-sudo-look
export SUDO_USER=scoot-probe-user
# The unified tree: test 5 tampered with the tagged scootbg above.
releases=file://$f/tags-unified.json run_in "$h" --system --prefix "$work/syspre" --dry-run --look moonrise || { cat "$work/out"; fail "sudo dry run with --look"; }
unset releases SUDO_USER
grep -q "look apply moonrise" "$work/out" || fail "sudo dry run names no apply"
grep -q "scoot-probe-user" "$work/out" || fail "sudo dry run names no invoking user"
[ ! -e "$h/.local" ] || fail "sudo dry run wrote files"
h=$work/h1
ok "sudo applies the look as the invoking user"

# 10. --system vs per-user clarity: a per-user run prints the
#     copy-pasteable --system command with the same flags; each direction
#     warns when the other copy exists; and a --system run after a
#     per-user one leaves both sides' files alone. The system record and
#     login-screen entry point at a scratch tree (the
#     SCOOT_INSTALL_SYSTEM_* hooks beside the release-URL ones), so no
#     root is needed: --system runs are dry runs, per-user runs are real.
export SCOOT_INSTALL_SYSTEM_MANIFEST="$work/sys/install-manifest"
export SCOOT_INSTALL_SYSTEM_SESSION="$work/sys/scoot.desktop"

# 10a. A per-user install says it adds no login-screen entry and prints
#     the --system command with its flags (the unified tree: test 5
#     tampered with the tagged scootbg above). Nothing system-side is
#     touched.
h=$work/h-cl
releases=file://$f/tags-unified.json run_in "$h" --look moonrise --no-bar || { cat "$work/out"; fail "clarity per-user install"; }
unset releases
grep -Fq "This installed for you only and added no login-screen entry." "$work/out" || { cat "$work/out"; fail "no you-only sentence"; }
grep -Fq "curl -fsSL https://www.scoot.sh/install.sh | sh -s -- --system --look moonrise --no-bar" "$work/out" || { cat "$work/out"; fail "no copy-pasteable --system command"; }
if grep -q "a system install exists" "$work/out"; then cat "$work/out"; fail "phantom system note with no system copy"; fi
[ ! -e "$work/sys/install-manifest" ] || fail "per-user run touched the system record"
[ ! -e "$work/sys/scoot.desktop" ] || fail "per-user run wrote a login-screen entry"
ok "a per-user run prints the --system command with its flags"

# 10b. --system after that per-user install warns that the per-user copy
#     may shadow it, names the exact removal, and changes nothing: the
#     per-user record is byte-identical after, and no system file lands.
before_manifest=$(cat "$h/.local/share/scoot/install-manifest")
releases=file://$f/tags-unified.json run_in "$h" --system --dry-run --look moonrise --no-bar || { cat "$work/out"; fail "system dry run after per-user"; }
unset releases
grep -Fq "a per-user install exists ($h/.local/share/scoot/install-manifest)" "$work/out" || { cat "$work/out"; fail "no per-user shadow warning"; }
grep -Fq "To remove it, run: install.sh --uninstall" "$work/out" || { cat "$work/out"; fail "no per-user removal command"; }
grep -Fq "That entry is $work/sys/scoot.desktop; your greeter and its config are untouched." "$work/out" || { cat "$work/out"; fail "no --system login-screen sentence"; }
[ "$(cat "$h/.local/share/scoot/install-manifest")" = "$before_manifest" ] || fail "system dry run touched the per-user record"
if [ -e "$work/sys/install-manifest" ] || [ -e "$work/sys/scoot.desktop" ]; then fail "system dry run wrote system files"; fi
ok "--system warns about a per-user copy and stays file-safe"

# 10c. The other order: a per-user run with a system install present says
#     the login-screen session runs the system copy, and writes nothing
#     system-side.
mkdir -p "$work/sys"
printf '%s\n' "/usr/local/bin/scoot" >"$work/sys/install-manifest"
h2=$work/h-cl2
releases=file://$f/tags-unified.json run_in "$h2" --dry-run || { cat "$work/out"; fail "per-user dry run with a system copy"; }
unset releases
grep -Fq "a system install exists ($work/sys/install-manifest)" "$work/out" || { cat "$work/out"; fail "no system-exists note"; }
grep -q "a session picked from the login screen runs the system copy" "$work/out" || { cat "$work/out"; fail "no login-screen pointer"; }
grep -Fq "curl -fsSL https://www.scoot.sh/install.sh | sh -s -- --system" "$work/out" || { cat "$work/out"; fail "no --system pointer with a system copy"; }
printf '%s\n' "/usr/local/bin/scoot" | cmp -s - "$work/sys/install-manifest" || fail "per-user run touched the system record"
ok "a per-user run names the system copy a login-screen session runs"

# 10d. Neither note fires when the other copy is absent.
rm -f "$work/sys/install-manifest"
releases=file://$f/tags-unified.json run_in "$work/h-cl3" --dry-run || { cat "$work/out"; fail "per-user dry run with no system copy"; }
unset releases
if grep -q "install exists" "$work/out"; then cat "$work/out"; fail "phantom coexistence note"; fi
ok "no coexistence note without the other copy"

# 10e. Under sudo the --system run finds the invoking user's per-user
#      install through getent, not root's empty one. Needs getent; the
#      stub answers the probe user with a scratch home and delegates the
#      rest to the real tool.
if command -v getent >/dev/null 2>&1; then
    real_getent=$(command -v getent)
    # shellcheck disable=SC2016 # every $ but the two %s runs in the stub, not here.
    printf '#!/bin/sh\nif [ "${2:-}" = scoot-probe-user ]; then printf "scoot-probe-user:x:1001:1001::%s:\\n"; else exec "%s" "$@"; fi\n' "$work/probe-home" "$real_getent" >"$work/bin/getent"
    chmod +x "$work/bin/getent"
    mkdir -p "$work/probe-home/.local/share/scoot"
    printf '%s\n' "$work/probe-home/.local/bin/scoot" >"$work/probe-home/.local/share/scoot/install-manifest"
    export SUDO_USER=scoot-probe-user
    releases=file://$f/tags-unified.json run_in "$work/h-cl-sudo" --system --dry-run || { cat "$work/out"; fail "sudo system dry run"; }
    unset releases SUDO_USER
    grep -Fq "a per-user install exists ($work/probe-home/.local/share/scoot/install-manifest)" "$work/out" || { cat "$work/out"; fail "no invoking-user shadow warning"; }
    ok "--system finds the invoking user's per-user copy"
else
    ok "--system invoking-user check skipped (no getent here)"
fi
unset SCOOT_INSTALL_SYSTEM_MANIFEST SCOOT_INSTALL_SYSTEM_SESSION
h=$work/h1

printf 'test-install: all passed\n'
