#!/usr/bin/env bash
# The scootbar systemd unit that nixosModules.scootbar (default, or --nixos) or
# homeModules.scootbar (--home) generates, run under the real `systemd --user`
# against a live (headless) scoot: does it do what docs/nix.md promises? Builds
# the unit, the bar.toml and the package from this checkout's flake (a real NixOS
# module evaluation, or a real home-manager evaluation, build and `activate`: not a
# hand-written unit), loads the unit into the user manager's RUNTIME directory
# ($XDG_RUNTIME_DIR/systemd/user: nothing persistent, gone at logout) and checks:
#   S0  started with no compositor, it keeps retrying (does not give up)
#   S1  once WAYLAND_DISPLAY is in the manager's environment, the next retry succeeds
#   S2  a SIGKILL is followed by a restart and a redrawn bar
#   S3  eight SIGKILLs in a row never leave it failed
#   S4  a SIGTERM (clean) stays stopped
#   S5  `scootbar msg kill` stays stopped
#   S6  `systemctl --user stop` stays stopped
#   S7  (INFO) the same retry with systemd's DEFAULT start limit, for the record
#   S8  graphical-session.target, started the way a session script does it (a
#       session target that BindsTo it, since the target itself refuses a manual
#       start): the unit's WantedBy= starts the bar, After= orders it, PartOf=
#       stops it with the session, and a clean stop stays stopped
#   S9  (--home only) the config changes: home-manager's own switch tool
#       (sd-switch, `--dry-run` first) restarts the bar when its X-Restart-Triggers
#       change, and leaves it alone when an unrelated part of the generation does
# Ends with RESULT: PASS n FAIL m (exit 1 on any FAIL, 2 on a setup problem).
#
#   scripts/scootbar-unit-test.sh [--nixos | --home]
#
# Needs: Linux with a running `systemd --user` manager and Nix (flakes enabled), a
# built scoot and scootctl (SCOOT / SCOOTCTL, else target/release then target/debug of
# this tree: `cargo build -p scoot -p scootctl`), python3 on PATH; --home also
# fetches home-manager (SCOOTBAR_HM_REV, default the revision in the script) and
# builds sd-switch. It uses a headless scoot, so nothing is drawn on a display and
# no VT is taken: before a compositor of ours exists the bar is pointed at a socket
# name that does not exist, never at whatever the session has. It REFUSES to run if
# a scootbar.service (or its drop-in directory) already exists in the user manager,
# if the manager's environment already holds a WAYLAND_DISPLAY (a real session's:
# this test would overwrite it), if graphical-session.target is active or has
# anything wanted by it (S8 starts and stops it), or if a unit it would create
# already exists. On exit, and on INT, TERM or HUP, it removes what it made: the
# units and their drop-ins and wants directory, the WAYLAND_DISPLAY it put in the
# manager's environment (only that), and its scratch directory. A SIGKILL of the
# script itself cannot be trapped and leaves them: `systemctl --user stop
# scootbar-test-session.target scootbar.service`, delete
# $XDG_RUNTIME_DIR/systemd/user/{scootbar.service{,.d},scootbar-test-session.target,
# graphical-session.target.wants}, `systemctl --user daemon-reload` and `systemctl
# --user unset-environment WAYLAND_DISPLAY`. The `nix build` writes the store, which
# stays. A bar the test SIGKILLs (S2, S3) leaves its control socket
# $XDG_RUNTIME_DIR/scootbar-wayland-N.sock and a lock file beside it, as any crashed bar
# does; the next bar on that display name recognises the dead lock and replaces them, and
# they are not removed here (the name may be a live session's).
#
# --home never touches your home-manager generation: `activate` of the generation
# runs for real, but with HOME and XDG_RUNTIME_DIR pointed into the scratch
# directory (so its profile, links and gcroots are scratch, and its systemd step
# finds no manager and skips), and the unit files it linked are what is loaded into
# the runtime directory. sd-switch is run on directories holding only the bar's unit.
#
# The deviations from the module's unit: the NixOS one names /etc/scootbar/bar.toml,
# which does not exist unless the module is enabled on this machine, so a drop-in
# points ExecStart at the same file by its store path; the home-manager one reads
# $XDG_CONFIG_HOME/scoot/bar.toml, so a drop-in sets XDG_CONFIG_HOME to the scratch
# home the generation was activated into, and ExecStart is the generated one. Not
# covered: NixOS's own switch (S9 is --home only: docs/nix.md says what the source of
# switch-to-configuration does with a user unit).
#
# The flake is this checkout read through `git+file`: tracked files, including
# uncommitted edits to them (a NEW file needs `git add`); the printed commit is HEAD.
set -u
die() { echo "error: $*" >&2; exit 2; }
MODE=nixos
case "${1:-}" in
    '' | --nixos) ;;
    --home) MODE=home ;;
    *) die "usage: scripts/scootbar-unit-test.sh [--nixos | --home]" ;;
esac
HM_REV=${SCOOTBAR_HM_REV:-efa3ccb4c3cc90d832eab232976379058fa75aa3}

ROOT=$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel 2>/dev/null) || die "run it from the scoot checkout"
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
[ -d "$XDG_RUNTIME_DIR" ] || die "no XDG_RUNTIME_DIR"
RT=$XDG_RUNTIME_DIR
UNITDIR=$RT/systemd/user
command -v nix >/dev/null || die "nix is not on PATH"
command -v python3 >/dev/null || die "python3 is not on PATH"
systemctl --user show-environment >/dev/null 2>&1 || die "no running systemd --user manager"
case "$ROOT" in
    *[!A-Za-z0-9/_.+-]*) die "the checkout path '$ROOT' has characters the flake URL cannot carry here; use a path of letters, digits and / _ . + -" ;;
esac
[ ! -e "$XDG_RUNTIME_DIR/systemd/user/scootbar.service.d" ] || die "$XDG_RUNTIME_DIR/systemd/user/scootbar.service.d already exists; remove it (it is not this test's)"
if systemctl --user show-environment | grep -q '^WAYLAND_DISPLAY='; then
    die "the user manager's environment already has a WAYLAND_DISPLAY (a live session's): this test sets and then unsets it. Run it where no session has imported one"
fi
for leftover in scootbar-test-session.target graphical-session.target.wants; do
    [ ! -e "$UNITDIR/$leftover" ] || die "$UNITDIR/$leftover already exists; remove it (it is not this test's)"
done
if [ "$(systemctl --user is-active graphical-session.target 2>&1)" != inactive ]; then
    die "graphical-session.target is not inactive: a session is using it, and S8 starts and stops it. Run it where none is"
fi
WANTED=$(systemctl --user show -p Wants -p RequiredBy -p WantedBy -p BoundBy --value graphical-session.target 2>/dev/null | tr -s ' \n' ' ')
[ -z "${WANTED// /}" ] || die "something is wired to graphical-session.target ($WANTED): S8 would start it. Run it where nothing is"
if systemctl --user cat scootbar.service >/dev/null 2>&1; then
    die "a scootbar.service already exists in your user manager; this test would shadow it. Stop and remove it first, or run on another account"
fi
pick() { for f in "$@"; do [ -n "$f" ] && [ -x "$f" ] && { echo "$f"; return; }; done; }
SCOOT=$(pick "${SCOOT:-}" "$ROOT/target/release/scoot" "$ROOT/target/debug/scoot") || true
CTL=$(pick "${SCOOTCTL:-}" "$ROOT/target/release/scootctl" "$ROOT/target/debug/scootctl") || true
[ -n "$SCOOT" ] && [ -n "$CTL" ] || die "no scoot/scootctl binary (cargo build -p scoot -p scootctl, or set SCOOT and SCOOTCTL)"

W=$(mktemp -d "$RT/scootbar-unit-test.XXXXXX")
pass=0; fail=0; SCOOT_PID=; SET_ENV=0
NOWHERE=scootbar-unit-test-no-such-socket
sc() { systemctl --user "$@"; }
ck() { local d=$1; shift; if "$@"; then echo "PASS  $d"; pass=$((pass+1)); else echo "FAIL  $d"; fail=$((fail+1)); fi; }
prop() { sc show -p "$1" --value scootbar.service; }
wait_for() { local n=$1; shift; local _; for _ in $(seq $((n*2))); do "$@" && return 0; sleep 0.5; done; return 1; }
active() { [ "$(prop ActiveState)" = active ]; }
inactive() { [ "$(prop ActiveState)" = inactive ]; }
not_failed() { [ "$(prop ActiveState)" != failed ]; }
restarts_ge() { [ "$(prop NRestarts)" -ge "$1" ]; }
# 0: the bar has reserved its zone; 1: it has not; 2: scoot could not be asked
bar_state() { SCOOT_SOCKET=$W/scoot.sock "$CTL" outputs 2>/dev/null | python3 -c '
import json,sys
try:
    o=json.load(sys.stdin)["outputs"][0]
except Exception:
    sys.exit(2)
sys.exit(0 if o["usable"]["y"]>0 or o["usable"]["height"]<o["rect"]["height"] else 1)'; }
bar_reserved() { bar_state; }
bar_gone() { bar_state; [ $? -eq 1 ]; }
# Signal the unit's own main process, and only a real one: MainPID is 0 when there is none,
# and `kill -KILL 0` would be the script's whole process group.
kill_main() { local p; p=$(prop MainPID); case "$p" in '' | *[!0-9]*) return 1 ;; esac; [ "$p" -gt 1 ] || return 1; kill "-$1" "$p"; }
stopped_after() { sleep "$1"; inactive; }
cleanup() {
    sc stop scootbar-test-session.target >/dev/null 2>&1
    sc stop scootbar.service >/dev/null 2>&1; sc reset-failed scootbar.service >/dev/null 2>&1
    rm -rf "$UNITDIR/scootbar.service" "$UNITDIR/scootbar.service.d" "$UNITDIR/scootbar-test-session.target" "$UNITDIR/graphical-session.target.wants"; sc daemon-reload
    [ "$SET_ENV" = 1 ] && sc unset-environment WAYLAND_DISPLAY
    [ -n "$SCOOT_PID" ] && kill "$SCOOT_PID" 2>/dev/null && wait "$SCOOT_PID" 2>/dev/null
    rm -rf "$W"
    # The bar makes a lock file named after the display it was told to connect to, and leaves it
    # when the connection fails: the one for the socket name that is not there.
    rm -f "$RT/scootbar-$NOWHERE.lock"
    echo "--- cleanup: unit loaded: $(sc is-enabled scootbar.service 2>&1 | head -1); graphical-session.target: $(sc is-active graphical-session.target 2>&1 | head -1); WAYLAND_DISPLAY in the manager: $(sc show-environment | grep -c WAYLAND_DISPLAY); scratch dir: $([ -e "$W" ] && echo left || echo gone)"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

if [ "$MODE" = nixos ]; then
cat > "$W/unit.nix" <<NIX
let
  flake = builtins.getFlake "git+file://$ROOT";
  sys = flake.inputs.nixpkgs.lib.nixosSystem {
    modules = [
      flake.nixosModules.scootbar
      ({ ... }: {
        nixpkgs.hostPlatform = builtins.currentSystem;
        boot.loader.grub.enable = false;
        fileSystems."/" = { device = "none"; fsType = "tmpfs"; };
        system.stateVersion = "25.11";
        programs.scootbar.enable = true;
      })
    ];
  };
  c = sys.config;
in {
  # The generated user-unit directory: the service and the wants link that WantedBy= makes.
  units = c.environment.etc."systemd/user".source;
  barToml = c.environment.etc."scootbar/bar.toml".source;
  package = c.programs.scootbar.finalPackage;
}
NIX
echo "== building the unit, bar.toml and package from the flake at $(git -C "$ROOT" rev-parse --short HEAD) (nix, NixOS module)"
NIXOUT=$(nix build --impure --no-link --print-out-paths -f "$W/unit.nix" units barToml package 2>"$W/nix.err") \
    || { cat "$W/nix.err" >&2; die "nix could not build the module's unit"; }
mapfile -t OUTS <<<"$NIXOUT"
[ "${#OUTS[@]}" -eq 3 ] || { cat "$W/nix.err" >&2; die "expected 3 outputs from nix, got ${#OUTS[@]}"; }
UNITS=${OUTS[0]}; TOML=${OUTS[1]}; PKG=${OUTS[2]}
echo "   units $UNITS"; echo "   toml  $TOML"; echo "   bar   $PKG"
else
# A real home-manager: its module system, its generation, its activation script.
# Three generations of one config: A the plain module, B with the bar's height
# changed (so bar.toml, and with it the unit's X-Restart-Triggers, change), C with an
# unrelated home-manager option added (so the generation changes and the bar's unit
# and file do not). The user name is the real one (home-manager refuses another);
# the home directory is the scratch one.
mkdir -p "$W/home" "$W/no-manager"
chmod 700 "$W/no-manager"
cat > "$W/hm.nix" <<NIX
let
  flake = builtins.getFlake "git+file://$ROOT";
  hm = builtins.getFlake "github:nix-community/home-manager/$HM_REV";
  pkgs = flake.inputs.nixpkgs.legacyPackages.\${builtins.currentSystem};
  gen = extra: (hm.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      flake.homeModules.scootbar
      ({ ... }: {
        home.username = "$(id -un)";
        home.homeDirectory = "$W/home";
        home.stateVersion = "25.11";
        programs.scootbar.enable = true;
      })
      extra
    ];
  }).activationPackage;
in {
  a = gen { };
  b = gen { programs.scootbar.settings.bar.height = 40; };
  c = gen { home.sessionVariables.SCOOTBAR_UNIT_TEST = "1"; };
  package = flake.packages.\${builtins.currentSystem}.scootbar;
  sdSwitch = pkgs.sd-switch;
}
NIX
echo "== building three home-manager generations and the package from the flake at $(git -C "$ROOT" rev-parse --short HEAD) (nix, home-manager $HM_REV)"
NIXOUT=$(nix build --impure --no-link --print-out-paths -f "$W/hm.nix" a b c package sdSwitch 2>"$W/nix.err") \
    || { cat "$W/nix.err" >&2; die "nix could not build the home-manager generations"; }
mapfile -t OUTS <<<"$NIXOUT"
[ "${#OUTS[@]}" -eq 5 ] || { cat "$W/nix.err" >&2; die "expected 5 outputs from nix, got ${#OUTS[@]}"; }
GEN_A=${OUTS[0]}; GEN_B=${OUTS[1]}; GEN_C=${OUTS[2]}; PKG=${OUTS[3]}; SDSWITCH=${OUTS[4]}/bin/sd-switch
echo "   A $GEN_A"; echo "   B $GEN_B"; echo "   C $GEN_C"; echo "   bar $PKG"; echo "   $($SDSWITCH --version)"
echo "== home-manager activate of A, for real, into the scratch home $W/home (no manager to reach: its systemd step must skip)"
env -i PATH="$PATH" HOME="$W/home" USER="$(id -un)" XDG_RUNTIME_DIR="$W/no-manager" "$GEN_A/activate" 2>&1 | sed 's/^/   /' | tee "$W/activate.out"
grep -q "User systemd daemon not running. Skipping reload." "$W/activate.out" || die "home-manager's activation did not skip its systemd step: it may have reached a manager"
UNITS=$W/home/.config/systemd/user; TOML=$W/home/.config/scoot/bar.toml
[ -e "$UNITS/scootbar.service" ] && [ -e "$TOML" ] || die "the activation linked no scootbar unit or bar.toml into $W/home"
echo "   linked: $(readlink "$UNITS/scootbar.service")"
fi

mkdir -p "$UNITDIR/scootbar.service.d"
cp -L --no-preserve=mode "$UNITS/scootbar.service" "$UNITDIR/scootbar.service"
if [ "$MODE" = nixos ]; then
cat > "$UNITDIR/scootbar.service.d/override.conf" <<DROP
[Service]
ExecStart=
ExecStart=$PKG/bin/scootbar daemon --config $TOML
DROP
else
# The generated ExecStart runs as it is; only the config home moves to the scratch one.
cat > "$UNITDIR/scootbar.service.d/override.conf" <<DROP
[Service]
Environment=XDG_CONFIG_HOME=$W/home/.config
DROP
fi
# Before a compositor of ours exists the bar must not find the session's (libwayland falls back
# to wayland-0): this drop-in names a socket that is not there, and S1 takes it away.
cat > "$UNITDIR/scootbar.service.d/nowhere.conf" <<DROP
[Service]
Environment=WAYLAND_DISPLAY=$NOWHERE
DROP
sc daemon-reload
echo "== the unit as systemd $(systemctl --version | head -1 | cut -d' ' -f2) loads it"
sc cat scootbar.service | grep -E "^(ExecStart|Restart|RestartSec|StartLimit|After|Before|PartOf)" | sed 's/^/   /'
ck "systemd-analyze verify accepts the unit" systemd-analyze --user verify "$UNITDIR/scootbar.service"

echo "== S0: started with no compositor to connect to: it must keep retrying, not give up"
sc start --no-block scootbar.service
sleep 14
echo "   state=$(prop ActiveState)/$(prop SubState) NRestarts=$(prop NRestarts) Result=$(prop Result)"
ck "S0 still retrying after 14 s (not failed)" not_failed
ck "S0 restarted at least 4 times in 14 s" restarts_ge 4

echo "== S1: the compositor comes up and WAYLAND_DISPLAY reaches the manager: the next retry must succeed"
printf '[layout]\ngap = 12\n' > "$W/scoot.toml"
env -u SCOOT_SOCKET -u WAYLAND_SOCKET "$SCOOT" --headless --width 1600 --height 1000 --socket "$W/scoot.sock" --config "$W/scoot.toml" > "$W/scoot.log" 2>&1 &
SCOOT_PID=$!
WL=
for _ in $(seq 40); do WL=$(sed -e 's/\x1b\[[0-9;]*m//g' "$W/scoot.log" | grep 'scoot is up' | grep -o 'wayland-[0-9]*' | head -1); [ -n "$WL" ] && break; sleep 0.25; done
[ -n "$WL" ] || { cat "$W/scoot.log" >&2; die "scoot did not come up"; }
echo "   scoot is up on $WL"
# As a session script does it (`systemctl --user import-environment WAYLAND_DISPLAY`, docs/nix.md):
WAYLAND_DISPLAY="$WL" sc import-environment WAYLAND_DISPLAY; SET_ENV=1
rm -f "$UNITDIR/scootbar.service.d/nowhere.conf"; sc daemon-reload
ck "S1 the unit becomes active on the next retry" wait_for 10 active
ck "S1 the bar reserved its zone on scoot" wait_for 10 bar_reserved
PID1=$(prop MainPID); echo "   MainPID=$PID1 NRestarts=$(prop NRestarts)"
ck "S1 the running bar is the Nix-built one" sh -c "readlink /proc/$PID1/exe | grep -q '$PKG/bin/scootbar'"
ck "S1 and it runs the unit's own ExecStart" sh -c "systemctl --user show -p ExecStart --value scootbar.service | grep -q '$PKG/bin/scootbar'"
ck "S1 the bar answers its control socket" env WAYLAND_DISPLAY="$WL" "$PKG/bin/scootbar" msg version

echo "== S2: SIGKILL: Restart=on-failure must bring it back"
N0=$(prop NRestarts)
ck "S2 SIGKILL sent to the unit's own main process" kill_main KILL
ck "S2 a new bar is active again" wait_for 10 active
ck "S2 the bar is drawn again" wait_for 10 bar_reserved
PID2=$(prop MainPID); echo "   MainPID $PID1 -> $PID2, NRestarts $N0 -> $(prop NRestarts)"
ck "S2 a new process, restart counted" sh -c "[ '$PID2' != '$PID1' ] && [ '$(prop NRestarts)' -gt '$N0' ]"

echo "== S3: eight SIGKILLs in a row: it must never end up failed"
N0=$(prop NRestarts); ok=1
for _ in $(seq 8); do
    wait_for 8 active || { ok=0; break; }
    P=$(prop MainPID); kill_main KILL || { ok=0; break; }
    wait_for 8 sh -c "m=\$(systemctl --user show -p MainPID --value scootbar.service); [ \"\$m\" != '$P' ] && [ \"\$m\" != 0 ]" || { ok=0; break; }
done
wait_for 8 active && sa=1 || sa=0
echo "   after 8 kills: state=$(prop ActiveState) NRestarts $N0 -> $(prop NRestarts) Result=$(prop Result)"
ck "S3 still active after eight kills, never failed" sh -c "[ $ok = 1 ] && [ $sa = 1 ]"
ck "S3 NRestarts rose by at least 8" sh -c "[ '$(prop NRestarts)' -ge $((N0+8)) ]"

echo "== S4: SIGTERM (a clean stop): it must NOT come back"
wait_for 8 active; ck "S4 SIGTERM sent to the unit's own main process" kill_main TERM
ck "S4 stays stopped after SIGTERM" stopped_after 6
echo "   state=$(prop ActiveState)/$(prop SubState) Result=$(prop Result) ExecMainStatus=$(prop ExecMainStatus)"
ck "S4 the zone was released" wait_for 5 bar_gone

echo "== S5: started again, then scootbar msg kill: it must stay stopped"
sc start scootbar.service; wait_for 10 active
# "active" is reported as soon as the process is exec'd (Type=simple): wait until the bar is
# actually up (zone reserved, so its control socket exists) before talking to it.
ck "S5 the bar is up and has reserved its zone" wait_for 10 bar_reserved
WAYLAND_DISPLAY="$WL" "$PKG/bin/scootbar" msg kill > "$W/kill.out" 2> "$W/kill.err"; echo "   msg kill rc=$? stdout='$(cat "$W/kill.out")' stderr='$(cat "$W/kill.err")'"
ck "S5 stays stopped after msg kill" stopped_after 6

echo "== S6: systemctl --user stop: it must stay stopped"
sc start scootbar.service; wait_for 10 active; wait_for 10 bar_reserved; sc stop scootbar.service
ck "S6 inactive after stop" inactive

echo "== S7: a control: the same retry with systemd's DEFAULT start limit (5 in 10 s), nothing to connect to"
cat > "$UNITDIR/scootbar.service.d/nowhere.conf" <<DROP
[Service]
Environment=WAYLAND_DISPLAY=$NOWHERE
DROP
cat > "$UNITDIR/scootbar.service.d/limit.conf" <<DROP
[Unit]
StartLimitIntervalSec=10s
StartLimitBurst=5
DROP
sc daemon-reload; sc reset-failed scootbar.service >/dev/null 2>&1
sc start --no-block scootbar.service; sleep 22
echo "   state=$(prop ActiveState)/$(prop SubState) NRestarts=$(prop NRestarts) Result=$(prop Result)"
ck "S7 with the default limit a 2 s retry still does not end in failed (so StartLimitIntervalSec=0 is insurance, not what keeps it alive; systemd $(systemctl --version | head -1 | cut -d' ' -f2))" not_failed

echo "== S8: graphical-session.target started by a session script: WantedBy, After and PartOf at work"
# What a session script would do, from the compositor's side: the environment first, then
# the target. graphical-session.target refuses a manual start (RefuseManualStart=yes), so
# the script starts a session target of its own that BindsTo it, as NixOS's own
# nixos-fake-graphical-session.target does; the bar's WantedBy= is the wants link
# that `enable` (or the module's Install section) makes.
sc stop scootbar.service; sc reset-failed scootbar.service >/dev/null 2>&1
rm -f "$UNITDIR/scootbar.service.d/nowhere.conf" "$UNITDIR/scootbar.service.d/limit.conf"
mkdir -p "$UNITDIR/graphical-session.target.wants"
ln -s ../scootbar.service "$UNITDIR/graphical-session.target.wants/scootbar.service"
cat > "$UNITDIR/scootbar-test-session.target" <<DROP
[Unit]
Description=scootbar-unit-test session
BindsTo=graphical-session.target
Wants=graphical-session-pre.target
After=graphical-session-pre.target
DROP
sc daemon-reload
echo "   manager environment holds WAYLAND_DISPLAY=$(sc show-environment | sed -n 's/^WAYLAND_DISPLAY=//p') (imported in S1, as a session script does)"
echo "   graphical-session.target wants: $(sc show -p Wants --value graphical-session.target)"
refused=$(sc start graphical-session.target 2>&1); rc=$?
echo "   a direct start of graphical-session.target: rc=$rc: $refused"
gst_is() { [ "$(sc is-active graphical-session.target)" = "$1" ]; }
refusal_ok() { [ "$rc" -ne 0 ] && [[ $refused == *"dependency only"* ]]; }
ck "S8 a direct start of graphical-session.target is refused (so a session script cannot use it)" refusal_ok
ck "S8 and nothing started" sh -c "! systemctl --user is-active --quiet graphical-session.target && ! systemctl --user is-active --quiet scootbar.service"
t0=$(date +%s.%N)
ck "S8 starting the session target succeeds" sc start scootbar-test-session.target
ck "S8 graphical-session.target is active" gst_is active
ck "S8 WantedBy=: the bar was started by the target, not by hand" wait_for 10 active
ck "S8 the bar reserved its zone" wait_for 10 bar_reserved
echo "   bar active $(awk -v a="$t0" -v b="$(date +%s.%N)" 'BEGIN{printf "%.2f", b-a}') s after the target start; NRestarts=$(prop NRestarts) MainPID=$(prop MainPID)"
ck "S8 it came up on the first try (WAYLAND_DISPLAY was imported first)" sh -c "[ '$(prop NRestarts)' = 0 ]"
GS=$(sc show -p ActiveEnterTimestampMonotonic --value graphical-session.target); BS=$(prop ActiveEnterTimestampMonotonic)
echo "   graphical-session.target active at $GS us, scootbar.service at $BS us (monotonic)"
ck "S8 After=: the bar was activated after graphical-session.target" sh -c "[ '$GS' -le '$BS' ]"
sc stop scootbar-test-session.target
ck "S8 stopping the session target stops graphical-session.target (StopWhenUnneeded)" wait_for 10 gst_is inactive
ck "S8 PartOf=: the bar stopped with it" wait_for 10 inactive
echo "   after the stop: state=$(prop ActiveState)/$(prop SubState) Result=$(prop Result)"
ck "S8 it stays stopped (a clean stop is not retried)" stopped_after 6
ck "S8 the zone was released" wait_for 5 bar_gone

if [ "$MODE" = home ]; then
echo "== S9: a changed config restarts the bar, an unchanged one does not (home-manager's sd-switch)"
# What a `home-manager switch` does after it links the new generation: sd-switch compares the
# old and new unit directories and acts on the manager. Here the directories hold only the
# bar's unit and its wants link (the generation also has tray.target, which is not this
# test's), and the "link" of a new generation is the same few steps by hand: its
# bar.toml link, its unit file into the manager's directory, a daemon-reload.
gendir() { # NAME GEN: the bar's unit and wants link out of a generation
    mkdir -p "$W/gen/$1/graphical-session.target.wants"
    cp -L --no-preserve=mode "$2/home-files/.config/systemd/user/scootbar.service" "$W/gen/$1/scootbar.service"
    ln -s "$(readlink "$2/home-files/.config/systemd/user/graphical-session.target.wants/scootbar.service")" "$W/gen/$1/graphical-session.target.wants/scootbar.service"
}
gendir a "$GEN_A"; gendir b "$GEN_B"; gendir c "$GEN_C"
echo "   unit A vs C: $(cmp -s "$W/gen/a/scootbar.service" "$W/gen/c/scootbar.service" && echo identical || echo DIFFERENT); A vs B: $(cmp -s "$W/gen/a/scootbar.service" "$W/gen/b/scootbar.service" && echo identical || echo different)"
diff "$W/gen/a/scootbar.service" "$W/gen/b/scootbar.service" | sed 's/^/   | /'
export DBUS_SESSION_BUS_ADDRESS=${DBUS_SESSION_BUS_ADDRESS:-unix:path=$RT/bus}
usable_y() { SCOOT_SOCKET=$W/scoot.sock "$CTL" outputs 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["outputs"][0]["usable"]["y"])'; }
switch_to() { # NAME GEN OLD: link the generation as home-manager does, then sd-switch
    ln -sfn "$2/home-files/.config/scoot/bar.toml" "$W/home/.config/scoot/bar.toml"
    cp -L --no-preserve=mode "$W/gen/$1/scootbar.service" "$UNITDIR/scootbar.service" || die "could not link generation $1 unit"
    sc daemon-reload
    echo "   loaded X-Restart-Triggers: $(sc show -p XRestartTriggers scootbar.service 2>/dev/null; sc cat scootbar.service | grep X-Restart-Triggers)"
    echo "   sd-switch --dry-run $3 -> $1:"; "$SDSWITCH" --dry-run -v --old-units "$W/gen/$3" --new-units "$W/gen/$1" 2>&1 | sed 's/^/   | /'
    echo "   sd-switch $3 -> $1:"; "$SDSWITCH" -v --old-units "$W/gen/$3" --new-units "$W/gen/$1" 2>&1 | sed 's/^/   | /'
}
invocation() { prop InvocationID; }
sc start scootbar-test-session.target; wait_for 10 active; wait_for 10 bar_reserved
I0=$(invocation); P0=$(prop MainPID); Y0=$(usable_y)
echo "   running generation A: InvocationID=$I0 MainPID=$P0 zone height=$Y0"
echo "   -- A to C (the generation changes, the bar's unit and file do not)"
switch_to c "$GEN_C" a
sleep 3
I1=$(invocation); P1=$(prop MainPID)
echo "   after: InvocationID=$I1 MainPID=$P1 state=$(prop ActiveState)"
ck "S9 unchanged: the bar was NOT restarted (same invocation, same process)" sh -c "[ '$I0' = '$I1' ] && [ '$P0' = '$P1' ]"
echo "   -- C to B (bar.height changed: bar.toml and X-Restart-Triggers change)"
switch_to b "$GEN_B" c
restarted_from() { [ "$(invocation)" != "$1" ] && active; }
zone_is() { [ "$(usable_y)" = "$1" ]; }
ck "S9 changed: the bar was restarted (new invocation, active again)" wait_for 10 restarted_from "$I1"
ck "S9 and it drew the new config (zone height $Y0 to 40)" wait_for 10 zone_is 40
echo "   after: InvocationID=$(invocation) MainPID=$(prop MainPID) zone height=$(usable_y)"
sc stop scootbar-test-session.target; wait_for 10 inactive
fi
echo
echo "RESULT: PASS $pass  FAIL $fail"
[ "$fail" -eq 0 ]
