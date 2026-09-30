#!/usr/bin/env bash
# The scootbar systemd unit that nixosModules.scootbar generates, run under the real
# `systemd --user` against a live (headless) scoot: does it do what docs/nix.md
# promises? Builds the unit, the bar.toml and the package from this checkout's flake
# (a real NixOS module evaluation, not a hand-written unit), loads the unit into the
# user manager's RUNTIME directory ($XDG_RUNTIME_DIR/systemd/user: nothing persistent,
# gone at logout) and checks:
#   S0  started with no compositor, it keeps retrying (does not give up)
#   S1  once WAYLAND_DISPLAY is in the manager's environment, the next retry succeeds
#   S2  a SIGKILL is followed by a restart and a redrawn bar
#   S3  eight SIGKILLs in a row never leave it failed
#   S4  a SIGTERM (clean) stays stopped
#   S5  `scootbar msg kill` stays stopped
#   S6  `systemctl --user stop` stays stopped
#   S7  (INFO) the same retry with systemd's DEFAULT start limit, for the record
# Ends with RESULT: PASS n FAIL m (exit 1 on any FAIL, 2 on a setup problem).
#
#   scripts/scootbar-unit-test.sh
#
# Needs: Linux with a running `systemd --user` manager and Nix (flakes enabled), a
# built scoot and scootctl (SCOOT / SCOOTCTL, else target/release then target/debug of
# this tree: `cargo build -p scoot -p scootctl`), python3 on PATH. It uses a headless
# scoot, so nothing is drawn on a display and no VT is taken: before a compositor of
# ours exists the bar is pointed at a socket name that does not exist, never at
# whatever the session has. It REFUSES to run if a scootbar.service (or its drop-in
# directory) already exists in the user manager, or if the manager's environment
# already holds a WAYLAND_DISPLAY (a real session's: this test would overwrite it).
# On exit, and on INT, TERM or HUP, it removes what it made: the unit and its
# drop-ins, the WAYLAND_DISPLAY it put in the manager's environment (only that), and
# its scratch directory. A SIGKILL of the script itself cannot be trapped and leaves
# them: `systemctl --user stop scootbar.service`, delete
# $XDG_RUNTIME_DIR/systemd/user/scootbar.service{,.d}, `systemctl --user
# daemon-reload` and `systemctl --user unset-environment WAYLAND_DISPLAY`. The
# `nix build` writes the store, which stays.
#
# The one deviation from the module's unit: it names /etc/scootbar/bar.toml, which does
# not exist unless the module is enabled on this machine, so a drop-in points ExecStart
# at the same file by its store path. Not covered: starting through
# graphical-session.target (a session script's job), and X-Restart-Triggers (NixOS
# switch and home-manager activation act on it).
#
# The flake is this checkout read through `git+file`: tracked files, including
# uncommitted edits to them (a NEW file needs `git add`); the printed commit is HEAD.
set -u
die() { echo "error: $*" >&2; exit 2; }

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
    sc stop scootbar.service >/dev/null 2>&1; sc reset-failed scootbar.service >/dev/null 2>&1
    rm -rf "$UNITDIR/scootbar.service" "$UNITDIR/scootbar.service.d"; sc daemon-reload
    [ "$SET_ENV" = 1 ] && sc unset-environment WAYLAND_DISPLAY
    [ -n "$SCOOT_PID" ] && kill "$SCOOT_PID" 2>/dev/null && wait "$SCOOT_PID" 2>/dev/null
    rm -rf "$W"
    echo "--- cleanup: unit loaded: $(sc is-enabled scootbar.service 2>&1 | head -1); WAYLAND_DISPLAY in the manager: $(sc show-environment | grep -c WAYLAND_DISPLAY); scratch dir: $([ -e "$W" ] && echo left || echo gone)"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

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
  unit = c.systemd.user.units."scootbar.service".unit;
  barToml = c.environment.etc."scootbar/bar.toml".source;
  package = c.programs.scootbar.finalPackage;
}
NIX
echo "== building the unit, bar.toml and package from the flake at $(git -C "$ROOT" rev-parse --short HEAD) (nix)"
NIXOUT=$(nix build --impure --no-link --print-out-paths -f "$W/unit.nix" unit barToml package 2>"$W/nix.err") \
    || { cat "$W/nix.err" >&2; die "nix could not build the module's unit"; }
mapfile -t OUTS <<<"$NIXOUT"
[ "${#OUTS[@]}" -eq 3 ] || { cat "$W/nix.err" >&2; die "expected 3 outputs from nix, got ${#OUTS[@]}"; }
U=${OUTS[0]}; TOML=${OUTS[1]}; PKG=${OUTS[2]}
echo "   unit $U"; echo "   toml $TOML"; echo "   bar  $PKG"

mkdir -p "$UNITDIR/scootbar.service.d"
cp "$U/scootbar.service" "$UNITDIR/scootbar.service"
cat > "$UNITDIR/scootbar.service.d/override.conf" <<DROP
[Service]
ExecStart=
ExecStart=$PKG/bin/scootbar daemon --config $TOML
DROP
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
sc set-environment WAYLAND_DISPLAY="$WL"; SET_ENV=1
rm -f "$UNITDIR/scootbar.service.d/nowhere.conf"; sc daemon-reload
ck "S1 the unit becomes active on the next retry" wait_for 10 active
ck "S1 the bar reserved its zone on scoot" wait_for 10 bar_reserved
PID1=$(prop MainPID); echo "   MainPID=$PID1 NRestarts=$(prop NRestarts)"
ck "S1 the running bar is the Nix-built one" sh -c "readlink /proc/$PID1/exe | grep -q '$PKG/bin/scootbar'"
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
echo
echo "RESULT: PASS $pass  FAIL $fail"
[ "$fail" -eq 0 ]
