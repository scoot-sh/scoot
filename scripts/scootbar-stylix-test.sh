#!/usr/bin/env bash
# homeModules.scootbar against a REAL Stylix and a REAL home-manager, not the stand-ins
# of nix/scootbar-tests.nix: evaluates both from their flakes (pinned revisions below),
# themes from a real image (Stylix runs its palette generator on it), builds the
# home-manager generation and then checks, on the file the generation renders:
#   T1  the six color tokens are the base16 palette's (background base00, foreground
#       base05, accent base0A, hover base0A like the bar's own default, dim base03,
#       urgent base08), and not the bar's own defaults
#   T2  bar.font is a store path to a real font file, bar.font-size Stylix's size in pixels
#   T3  the rendered bar.toml reads back as exactly the evaluated settings
#   T4  `scootbar daemon --check` accepts it
#   T5  a user value wins over Stylix's, one token at a time (the other tokens stay
#       Stylix's), in the evaluation and in the file
#   T6  on a headless scoot, the bar run with that file draws base00 as its background,
#       and a user's background in place of it when the user sets one
# Ends with RESULT: PASS n FAIL m (exit 1 on any FAIL, 2 on a setup problem).
#
#   scripts/scootbar-stylix-test.sh
#
# Needs Linux, Nix (flakes enabled, network for the two flakes and Stylix's own inputs),
# python3, and a built scoot and scootctl (SCOOT / SCOOTCTL, else target/release then
# target/debug of this tree). Overrides: SCOOTBAR_HM_REV, SCOOTBAR_STYLIX_REV. It touches
# no systemd, writes only a scratch directory (removed on exit, with its scoot) and the
# Nix store, and reads this checkout's flake through `git+file` (tracked files, including
# uncommitted edits; a NEW file needs `git add`). Not run in CI: it builds Stylix's
# palette generator (Haskell) and fetches two flakes.
set -u
die() { echo "error: $*" >&2; exit 2; }
ROOT=$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel 2>/dev/null) || die "run it from the scoot checkout"
command -v nix >/dev/null || die "nix is not on PATH"
command -v python3 >/dev/null || die "python3 is not on PATH"
case "$ROOT" in
    *[!A-Za-z0-9/_.+-]*) die "the checkout path '$ROOT' has characters the flake URL cannot carry here" ;;
esac
HM_REV=${SCOOTBAR_HM_REV:-efa3ccb4c3cc90d832eab232976379058fa75aa3}
STYLIX_REV=${SCOOTBAR_STYLIX_REV:-fb28acd59e2ac1984ec84fa496599d6b4bf3e690}
pick() { for f in "$@"; do [ -n "$f" ] && [ -x "$f" ] && { echo "$f"; return; }; done; }
SCOOT=$(pick "${SCOOT:-}" "$ROOT/target/release/scoot" "$ROOT/target/debug/scoot") || true
CTL=$(pick "${SCOOTCTL:-}" "$ROOT/target/release/scootctl" "$ROOT/target/debug/scootctl") || true
[ -n "$SCOOT" ] && [ -n "$CTL" ] || die "no scoot/scootctl binary (cargo build -p scoot -p scootctl, or set SCOOT and SCOOTCTL)"

W=$(mktemp -d "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/scootbar-stylix.XXXXXX") || die "no scratch directory"
chmod 700 "$W"
pass=0; fail=0; SCOOT_PID=
ck() { local d=$1; shift; if "$@"; then echo "PASS  $d"; pass=$((pass+1)); else echo "FAIL  $d"; fail=$((fail+1)); fi; }
cleanup() {
    [ -n "$SCOOT_PID" ] && kill "$SCOOT_PID" 2>/dev/null && wait "$SCOOT_PID" 2>/dev/null
    rm -rf "$W"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

# The wallpaper: a seeded spread of many colors, so the palette is a real function of a real
# image and its base16 slots differ from one another (flat patches gave base03 = base05 and
# base08 = base0A, which could not tell a swapped slot from the right one).
python3 - "$W/wall.png" <<'PY'
import random, struct, sys, zlib
rnd = random.Random(7)
raw = b"".join(
    b"\x00" + b"".join(bytes((rnd.randrange(256), (x * 4) % 256, (y * 4) % 256)) for x in range(64)) for y in range(64)
)
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 2, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
open(sys.argv[1], "wb").write(png)
PY

cat > "$W/stylix.nix" <<NIX
let
  flake = builtins.getFlake "git+file://$ROOT";
  hm = builtins.getFlake "github:nix-community/home-manager/$HM_REV";
  stylix = builtins.getFlake "github:nix-community/stylix/$STYLIX_REV";
  pkgs = flake.inputs.nixpkgs.legacyPackages.\${builtins.currentSystem};
  gen = extra: hm.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      stylix.homeModules.stylix
      flake.homeModules.scootbar
      ({ ... }: {
        home.username = "$(id -un)";
        home.homeDirectory = "$W/home";
        home.stateVersion = "25.11";
        stylix.enable = true;
        # Only the module under test: no other target themed, so no GTK or Qt is built.
        stylix.autoEnable = false;
        stylix.image = $W/wall.png;
        stylix.polarity = "dark";
        programs.scootbar.enable = true;
      })
      extra
    ];
  };
  themed = gen { };
  # One token (and the size) from the user, one for the pixel test: Stylix keeps the rest.
  user = gen {
    programs.scootbar.settings.colors.accent = "#abcdef";
    programs.scootbar.settings.colors.background = "#123456";
    programs.scootbar.settings.bar.font-size = 20;
  };
  facts = c: {
    settings = c.config.programs.scootbar.settings;
    palette = builtins.mapAttrs (_: v: v) {
      inherit (c.config.lib.stylix.colors.withHashtag) base00 base03 base05 base08 base0A;
    };
    desktopSize = c.config.stylix.fonts.sizes.desktop;
    family = c.config.stylix.fonts.sansSerif.name;
    configFile = "\${c.config.programs.scootbar.configFile}";
  };
in {
  themedFacts = facts themed;
  userFacts = facts user;
  themed = themed.activationPackage;
  user = user.activationPackage;
  package = flake.packages.\${builtins.currentSystem}.scootbar;
}
NIX
echo "== real Stylix $STYLIX_REV and home-manager $HM_REV, scoot flake at $(git -C "$ROOT" rev-parse --short HEAD)"
nix eval --impure --json -f "$W/stylix.nix" themedFacts > "$W/themed.json" 2> "$W/eval.err" \
    || { cat "$W/eval.err" >&2; die "nix could not evaluate the themed configuration"; }
nix eval --impure --json -f "$W/stylix.nix" userFacts > "$W/user.json" 2>> "$W/eval.err" \
    || { cat "$W/eval.err" >&2; die "nix could not evaluate the user-override configuration"; }
echo "   building the palette (Stylix runs its generator on the image) and the generations"
NIXOUT=$(nix build --impure --no-link --print-out-paths -f "$W/stylix.nix" themed user package 2>"$W/build.err") \
    || { cat "$W/build.err" >&2; die "nix could not build the generations"; }
mapfile -t OUTS <<<"$NIXOUT"
[ "${#OUTS[@]}" -eq 3 ] || { cat "$W/build.err" >&2; die "expected 3 outputs from nix, got ${#OUTS[@]}"; }
THEMED_TOML=${OUTS[0]}/home-files/.config/scoot/bar.toml
USER_TOML=${OUTS[1]}/home-files/.config/scoot/bar.toml
BAR=${OUTS[2]}/bin/scootbar
[ -e "$THEMED_TOML" ] && [ -e "$USER_TOML" ] && [ -x "$BAR" ] || die "a generation has no bar.toml, or no bar"
echo "   themed file $(readlink -f "$THEMED_TOML")"; echo "   user file   $(readlink -f "$USER_TOML")"; echo "   bar         $BAR"
echo "   palette:  $(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["palette"])' "$W/themed.json")"
echo "   themed settings: $(python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1]))["settings"], sort_keys=True))' "$W/themed.json")"
echo "   user settings:   $(python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1]))["settings"], sort_keys=True))' "$W/user.json")"

pyck() { python3 - "$@"; }
t1() { pyck "$W/themed.json" <<'PY'
import json, sys
f = json.load(open(sys.argv[1])); s, p = f["settings"]["colors"], f["palette"]
want = {"background": p["base00"], "foreground": p["base05"], "accent": p["base0A"], "hover": p["base0A"], "dim": p["base03"], "urgent": p["base08"]}
bar_defaults = {"background": "#1e1e2e", "foreground": "#cdd6f4"}
assert s == want, (s, want)
assert all(s[k] != v for k, v in bar_defaults.items()), "the tokens are the bar's own defaults, not Stylix's"
assert len(set(s.values())) == 5, ("two tokens share a color beyond hover==accent, so a swapped slot would pass", s)
PY
}
ck "T1 the six color tokens are base16 base00/05/0A/0A/03/08 of the image's palette, not the bar's defaults" t1
t2() { pyck "$W/themed.json" <<'PY'
import json, os, sys
f = json.load(open(sys.argv[1])); bar = f["settings"]["bar"]
assert f["family"] == "DejaVu Sans", f["family"]
real = os.path.realpath(bar["font"])
assert real.startswith("/nix/store/") and real.endswith(".ttf") and os.path.getsize(real) > 1000, real
assert bar["font-size"] == (f["desktopSize"] * 4 + 1) // 3 == 13, (bar["font-size"], f["desktopSize"])
print("   font ->", real, " size", f["desktopSize"], "pt ->", bar["font-size"], "px")
PY
}
ck "T2 bar.font is a real font file in the store, bar.font-size Stylix's size in pixels" t2
t3() { pyck "$W/themed.json" "$THEMED_TOML" "$W/user.json" "$USER_TOML" <<'PY'
import json, sys, tomllib
for js, toml in ((sys.argv[1], sys.argv[2]), (sys.argv[3], sys.argv[4])):
    got = tomllib.load(open(toml, "rb"))
    want = json.load(open(js))["settings"]
    assert got == want, (toml, got, want)
PY
}
ck "T3 each rendered bar.toml reads back as exactly the evaluated settings" t3
check_ok() { [ "$("$BAR" daemon --check --config "$1" 2>"$W/check.err")" = ok ] && [ ! -s "$W/check.err" ]; }
ck "T4 scootbar daemon --check accepts the themed file" check_ok "$THEMED_TOML"
ck "T4 scootbar daemon --check accepts the user-override file" check_ok "$USER_TOML"
t5() { pyck "$W/themed.json" "$W/user.json" <<'PY'
import json, sys
t, u = (json.load(open(a)) for a in sys.argv[1:3])
tc, uc = t["settings"]["colors"], u["settings"]["colors"]
assert uc["accent"] == "#abcdef" and uc["background"] == "#123456", uc
for k in ("foreground", "dim", "urgent"):
    assert uc[k] == tc[k], (k, uc[k], tc[k])
assert u["settings"]["bar"]["font-size"] == 20 and u["settings"]["bar"]["font"] == t["settings"]["bar"]["font"]
PY
}
ck "T5 a user's values win per key (accent, background, size); Stylix keeps the other tokens and the font" t5

# T6: pixels. A headless scoot, the bar run with each file, a screenshot of the output.
RUN=$W/run; mkdir -p "$RUN"; chmod 700 "$RUN"
printf '[layout]\ngap = 12\n' > "$W/scoot.toml"
env -u SCOOT_SOCKET -u WAYLAND_SOCKET XDG_RUNTIME_DIR="$RUN" "$SCOOT" --headless --width 800 --height 600 --socket "$W/scoot.sock" --config "$W/scoot.toml" > "$W/scoot.log" 2>&1 &
SCOOT_PID=$!
WL=
for _ in $(seq 40); do WL=$(sed -e 's/\x1b\[[0-9;]*m//g' "$W/scoot.log" | grep 'scoot is up' | grep -o 'wayland-[0-9]*' | head -1); [ -n "$WL" ] && break; sleep 0.25; done
[ -n "$WL" ] || { cat "$W/scoot.log" >&2; die "scoot did not come up"; }
zone_reserved() { SCOOT_SOCKET=$W/scoot.sock "$CTL" outputs 2>/dev/null | python3 -c '
import json,sys
try: o=json.load(sys.stdin)["outputs"][0]
except Exception: sys.exit(2)
sys.exit(0 if o["usable"]["y"]>0 or o["usable"]["height"]<o["rect"]["height"] else 1)'; }
shoot_pixel() { # TOML EXPECTED_HEX: the bar's background pixel, from a screenshot of the output
    XDG_RUNTIME_DIR=$RUN WAYLAND_DISPLAY=$WL "$BAR" daemon --config "$1" > "$W/bar.log" 2>&1 &
    local bp=$! n ok=1
    for n in $(seq 40); do zone_reserved && { ok=0; break; }; sleep 0.25; done
    if [ $ok -eq 0 ]; then
        SCOOT_SOCKET=$W/scoot.sock "$CTL" screenshot --out "$W/shot.png" --no-cursor >/dev/null 2>&1 || ok=1
    fi
    kill $bp 2>/dev/null; wait $bp 2>/dev/null
    [ $ok -eq 0 ] || { cat "$W/bar.log" >&2; return 1; }
    python3 - "$W/shot.png" "$2" <<'PY'
import struct, sys, zlib
data = open(sys.argv[1], "rb").read()
pos, idat, ihdr = 8, b"", None
while pos < len(data):
    n, kind = struct.unpack(">I4s", data[pos:pos + 8]); body = data[pos + 8:pos + 8 + n]
    if kind == b"IHDR": ihdr = struct.unpack(">IIBBBBB", body)
    elif kind == b"IDAT": idat += body
    pos += 12 + n
w, h, depth, ctype, _, _, interlace = ihdr
assert depth == 8 and ctype in (2, 6) and interlace == 0, ihdr
bpp = 3 if ctype == 2 else 4
raw = zlib.decompress(idat); stride = w * bpp
rows, prev = [], bytearray(stride)
for y in range(h):
    ft = raw[y * (stride + 1)]; line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
    for i in range(stride):
        a = line[i - bpp] if i >= bpp else 0; b = prev[i]; c = prev[i - bpp] if i >= bpp else 0
        if ft == 1: line[i] = (line[i] + a) & 255
        elif ft == 2: line[i] = (line[i] + b) & 255
        elif ft == 3: line[i] = (line[i] + (a + b) // 2) & 255
        elif ft == 4:
            p = a + b - c; pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    rows.append(line); prev = line
def px(x, y): return "#%02x%02x%02x" % tuple(rows[y][x * bpp:x * bpp + 3])
got, want = px(2, 2), sys.argv[2].lower()
print("   bar pixel (2,2) = %s, wanted %s; the wallpaper-less desktop at (400,300) = %s" % (got, want, px(400, 300)))
sys.exit(0 if got == want else 1)
PY
}
BASE00=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["palette"]["base00"])' "$W/themed.json")
ck "T6 the bar run with the themed file draws base00 ($BASE00) as its background" shoot_pixel "$THEMED_TOML" "$BASE00"
ck "T6 with the user's background (#123456) in the file, that is what is drawn" shoot_pixel "$USER_TOML" "#123456"
echo
echo "RESULT: PASS $pass  FAIL $fail"
[ "$fail" -eq 0 ]
