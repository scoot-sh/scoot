#!/bin/sh
# Full-look screenshot: the default ginger-night look with the bar (and its
# notifications module showing an unread count), two terminals, scootlaunch
# open mid-search, and scootnotify cards — one frame, captured over scoot's
# own IPC screenshot path (`scoot msg screenshot`, the same path the docs'
# own screenshots use). Repeats docs/examples/ginger-night/screenshot.sh's
# rig (headless scoot + bar + two feet) with the launcher and the
# notification daemon added.
#
# Usage (on the Asahi M2, inside a private D-Bus session so the test daemon
# owns the bus name without touching the operator's session):
#
#   dbus-run-session -- sh scripts/full-look-shot.sh 1920 1080 /tmp/full-look.png
#
# Contract:
# - BIN_DIR (default $HOME/w39-shot): patched COPIES of the released v0.4.1
#   aarch64 binaries (originals SHA256SUMS-checked; interpreter + rpath
#   repointed at nixpkgs, the release bytes otherwise untouched). Provides
#   scoot, scootbar, foot? no — foot comes from PATH (the installed set);
#   BIN_DIR provides scoot/scootbar/scootlaunch/scootnotify copies under
#   those exact names (see the worktree report for the patchelf lines).
# - LOOK_SRC (default docs/examples/ginger-night in this checkout): the
#   look's example files (scoot.toml, bar.toml, foot.ini, load.sh, cpu.sh,
#   wallpaper.png). The committed files are never modified; the shot edits
#   a scratch copy (adds the notifications module to the bar).
# - Determinism: fresh XDG_*_HOME/RUNTIME per run (empty frecency, empty
#   daemon state), explicit --font files everywhere (no well-known-path
#   search), synthetic desktop entries (fixed bytes, no TryExec, no real
#   apps), staged terminal text (fixed prompt guest@scoot, no hostnames,
#   no secrets, no timestamps), notify timeouts pinned to one hour so the
#   cards cannot expire mid-shot. Live by design, like the existing
#   previews: the bar clock shows the run time, load/cpu show the box.
# - Cleanup kills exact captured PIDs only, never by pattern; the scratch
#   runtime dir is removed on exit (shot logs beside the output kept).
set -eu
W=${1:?width}; H=${2:?height}; OUT=${3:?out png}
BIN_DIR=${BIN_DIR:-$HOME/w39-shot}
LOOK_SRC=${LOOK_SRC:-$(cd "$(dirname "$0")/../docs/examples/ginger-night" && pwd)}

[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || { echo "full-look-shot: no session bus — run under dbus-run-session -- (refusing to touch the operator bus)"; exit 2; }
command -v busctl >/dev/null 2>&1 || { echo "full-look-shot: needs busctl on PATH"; exit 2; }
for b in scoot scootbar scootlaunch scootnotify; do
  [ -x "$BIN_DIR/$b" ] || { echo "full-look-shot: BIN_DIR=$BIN_DIR needs executable $b"; exit 2; }
done
command -v foot >/dev/null 2>&1 || { echo "full-look-shot: needs foot on PATH"; exit 2; }

FX=$HOME/w39-shot
RT=$(mktemp -d "$FX/rt-XXXXXX"); chmod 700 "$RT"
export XDG_RUNTIME_DIR=$RT
export SCOOT_SOCKET=$RT/scoot.sock
SCRATCH=$RT/home; mkdir -p "$SCRATCH"
export HOME=$SCRATCH
export XDG_CONFIG_HOME=$SCRATCH/.config XDG_DATA_HOME=$SCRATCH/.local/share
export XDG_STATE_HOME=$SCRATCH/.local/state XDG_CACHE_HOME=$SCRATCH/.cache
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME"

# Fonts: resolved explicitly, never by search.
BAR_FONT=$(fc-match --format='%{file}\n' 'DroidSansM Nerd Font Propo' | head -1)
SANS_FONT=$(fc-match --format='%{file}\n' 'DejaVu Sans' | head -1)
[ -n "$BAR_FONT" ] && [ -n "$SANS_FONT" ] || { echo "full-look-shot: fonts not found"; exit 2; }
echo "fonts: bar=$BAR_FONT sans=$SANS_FONT"

# Look files: scratch copy; the bar gains the notifications module.
G=$RT/look; mkdir -p "$G"
cp "$LOOK_SRC/scoot.toml" "$LOOK_SRC/bar.toml" "$LOOK_SRC/foot.ini" "$LOOK_SRC/wallpaper.png" "$LOOK_SRC/load.sh" "$LOOK_SRC/cpu.sh" "$G/"
chmod +x "$G/load.sh" "$G/cpu.sh"
# foot's face must resolve under the scratch HOME; fall back to DejaVu Mono.
fc-match 'FiraCode Nerd Font' >/dev/null 2>&1 || sed -i 's/^font=.*/font=DejaVu Sans Mono:size=10.5/' "$G/foot.ini"
# Add the notifications feed to the bar's right side + its push module.
sed -i 's/"battery", "terminal"/"battery", "notifications", "terminal"/' "$G/bar.toml"
printf '\n[push.notifications]\nplaceholder = "idle"\n' >>"$G/bar.toml"
grep -q 'push.notifications' "$G/bar.toml" || { echo "full-look-shot: bar edit failed"; exit 2; }

# Synthetic entries: fixed bytes, real enough to rank, icons where the
# system theme provides them (hicolor carries firefox/foot/mpv/nvim).
# Every entry carries a distinct Icon key (real or tile-fallback): the
# panel's icon cache keys (name, side) alone, so two icon-less entries
# would share one tile letter.
APPS=$RT/apps/applications; mkdir -p "$APPS"
mke() { # id, Name, Icon?, keywords, comment
  { echo "[Desktop Entry]"; echo "Version=1.0"; echo "Type=Application"; echo "Name=$2"; [ -z "${3:-}" ] || echo "Icon=$3"; echo "Comment=$5"; echo "Keywords=$4"; echo "Exec=/bin/true"; } >"$APPS/$1.desktop"
}
mke terminal Terminal foot "shell;prompt;console" "Terminal emulator"
mke firefox Firefox firefox "web;browser;internet" "Browse the web"
mke files Files files "files;folders;manager" "File manager"
mke mediaplayer "Media Player" mpv "video;music;play" "Play media"
mke editor "Text Editor" nvim "text;edit;code" "Edit text"
mke calculator Calculator calc "calc;math;numbers" "Calculator"
mke settings Settings prefs "prefs;system;config" "System settings"
mke mail Mail mail "email;inbox;post" "Mail client"
mke scanner Scanner scan "scan;document;page" "Scan documents"
mke alacritty Alacritty Alacritty "gpu;terminal;fast" "Alacritty terminal"
mke photos Photos photos "photo;image;gallery" "Photo gallery"
mke music Music music "audio;songs;play" "Music player"
mke notes Notes notes "notes;memo;text" "Notes"
# Icons resolve from the system themes, linked in (the app list itself
# stays exactly these entries: XDG_DATA_DIRS names only this dir).
mkdir -p "$RT/apps/icons"
ln -s /run/current-system/sw/share/icons/hicolor "$RT/apps/icons/hicolor"
ln -s /run/current-system/sw/share/icons/Adwaita "$RT/apps/icons/Adwaita"
export XDG_DATA_DIRS="$RT/apps"

# Notify timeouts pinned so the cards outlive the shot (defaults otherwise).
cat >"$G/notify.toml" <<'EOF'
timeout_low_ms = 3600000
timeout_normal_ms = 3600000
max_visible = 5
EOF

mkdir -p "$G/bin"; cp "$G/load.sh" "$G/cpu.sh" "$G/bin/"
export PATH="$G/bin:$BIN_DIR:$PATH"

scoot --headless --width "$W" --height "$H" --socket "$SCOOT_SOCKET" \
  --config "$G/scoot.toml" >"$RT/scoot.log" 2>&1 &
SCOOT_PID=$!
cleanup() {
  for p in ${LAUNCH_PID:-} ${NOTIFY_PID:-} ${BAR_PID:-} ${F1_PID:-} ${F2_PID:-}; do
    kill "$p" 2>/dev/null || true
  done
  if [ -n "${SCOOT_PID:-}" ]; then
    SCOOT_SOCKET=$SCOOT_SOCKET scoot msg action quit >/dev/null 2>&1 || kill "$SCOOT_PID" 2>/dev/null || true
  fi
  wait 2>/dev/null || true
  TAG=$(basename "$OUT" .png)
  mkdir -p "$FX/shot-logs"
  for f in "$RT"/*.log; do [ -f "$f" ] && cp "$f" "$FX/shot-logs/$TAG-$(basename "$f")" || true; done
  rm -rf "$RT"
}
trap cleanup EXIT

for i in $(seq 1 150); do
  [ -S "$SCOOT_SOCKET" ] && scoot msg outputs >/dev/null 2>&1 && break
  sleep 0.1
done
scoot msg outputs
WAYLAND_DISPLAY=$(grep 'scoot is up' "$RT/scoot.log" | grep -o 'wayland[^"]*"[^"]*"' | head -1 | grep -o 'wayland-[0-9]*' || true)
[ -n "$WAYLAND_DISPLAY" ] || { echo "full-look-shot: no Wayland socket in log"; tail -20 "$RT/scoot.log"; exit 1; }
export WAYLAND_DISPLAY
echo "wayland: $WAYLAND_DISPLAY"

scootbar daemon --font "$BAR_FONT" --config "$G/bar.toml" >"$RT/bar.log" 2>&1 &
BAR_PID=$!
foot --config "$G/foot.ini" sh -c "printf '%b' 'guest@scoot ~/projects/demo \$ ls\nCargo.lock  Cargo.toml  README.md  crates/  looks/  scripts/\nguest@scoot ~/projects/demo \$ git status -sb\n## main...origin/main\n M scripts/full-look-shot.sh\nguest@scoot ~/projects/demo \$ git log --oneline -3\na1b2c3d docs: full-look screenshot rig\ne4f5g6h feat(scootlaunch): icon-theme lookup\nc7d8e9f fix(scootnotify): unread count on the bar feed\nguest@scoot ~/projects/demo \$ git show --stat HEAD | head -6\ncommit a1b2c3d9 (HEAD -> main)\n    docs: full-look screenshot rig\n scripts/full-look-shot.sh | 120 +++++++++++++++++++++++++\n 1 file changed, 120 insertions(+)\n'; exec sleep 300" >"$RT/foot1.log" 2>&1 &
F1_PID=$!
foot --config "$G/foot.ini" sh -c "printf '%b' 'guest@scoot ~ \$ scoot --version\nscoot 0.4.1 (ipc protocol 12)\nguest@scoot ~ \$ scootnotify msg list\n1 [normal] Build: Build finished\n2 [normal] Calendar: Meeting in 5 minutes\nguest@scoot ~ \$ scootlaunch --query em --json\n[{\"id\": \"mail\", \"name\": \"Mail\"}, ...]\nguest@scoot ~ \$ cat notes.txt\n- launcher panel: rank Mail over tiles\n- cards: build + calendar, normal urgency\n- bar: unread count beside the dot\n'; exec sleep 300" >"$RT/foot2.log" 2>&1 &
F2_PID=$!
scootnotify daemon --config "$G/notify.toml" --font "$SANS_FONT" >"$RT/notify.log" 2>&1 &
NOTIFY_PID=$!

sleep 1
busctl --user call org.freedesktop.Notifications /org/freedesktop/Notifications org.freedesktop.Notifications Notify susssasa{sv}i \
  "Build" 0 "" "Build finished" "scoot 0.4.1 — 128 passed, 0 failed in 42 s" 0 0 3600000 >/dev/null
sleep 1
busctl --user call org.freedesktop.Notifications /org/freedesktop/Notifications org.freedesktop.Notifications Notify susssasa{sv}i \
  "Calendar" 0 "" "Meeting in 5 minutes" "Design review in the west room" 0 0 3600000 >/dev/null
sleep 1
scootnotify msg list
LIST_N=$(scootnotify msg list | wc -l)
[ "$LIST_N" -ge 2 ] || { echo "full-look-shot: want 2 live cards, have $LIST_N"; exit 1; }

# Launcher open mid-search: ginger-night roles as per-invocation flags.
# Query "em" ranks Mail (matched through its keywords, so the selected
# row carries no match spans and stays selection-text on selection) over
# six more rows: any name-matched selection would draw the match spans
# in urgent on the orange selection, which is unreadable in ginger-night
# (see docs/scootbar/backlog/launch-match-on-selection-contrast.md).
scootlaunch --font "$SANS_FONT" \
  --background-color 0e0e0eff --text-color f5ead6ff --border-color ff9a30ff \
  --selection-color e57f29ff --selection-text-color 0e0e0eff \
  --match-color ffb14bff --prompt-color 8f7a63ff >"$RT/launch.log" 2>&1 &
LAUNCH_PID=$!
sleep 2
kill -0 "$LAUNCH_PID" 2>/dev/null || { echo "full-look-shot: launcher died early"; cat "$RT/launch.log"; exit 1; }
scoot msg type "em"
sleep 1

# Settle, park the pointer top-center over the wallpaper, take the frame.
sleep 2
scoot msg windows
PX=$((W - 10)); PY=$((H - 10)); scoot msg pointer move $PX $PY || true
scoot msg screenshot --out "$OUT"
ls -l "$OUT"
echo "ok: $OUT"
