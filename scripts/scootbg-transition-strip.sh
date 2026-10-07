#!/bin/sh
# Frame strip for the transitions docs page: four frames each of a fade, a
# wipe and a grow, from red to blue, taken from a real headless scoot
# through scoot's own screenshot path, tiled with stdlib python3 only.
#
# Usage (on a Linux box with a built tree):
#   scripts/scootbg-transition-strip.sh [out.png]
#
# Defaults to site/src/assets/scootbg-transition-strip.png. Needs the
# release or debug `scoot` and `scootbg` beside each other: set SCOOT_BIN
# and SCOOTBG_BIN, or pass a target dir with both in it. Nothing here
# touches your session: its own XDG_RUNTIME_DIR, HOME and scoot config.
set -eu

OUT="${1:-site/src/content/../assets/scootbg-transition-strip.png}"
SCOOT_BIN="${SCOOT_BIN:-target/debug/scoot}"
SCOOTBG_BIN="${SCOOTBG_BIN:-target/debug/scootbg}"
PYTHON3="${PYTHON3:-python3}"
RED="#c03020"
BLUE="#101014"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/trans-strip-XXXXXX")"
trap 'kill "$scoot" 2>/dev/null || true; rm -rf "$scratch"' EXIT INT TERM
mkdir -p "$scratch/run" "$scratch/home"
chmod 700 "$scratch/run"
export XDG_RUNTIME_DIR="$scratch/run" HOME="$scratch/home" XDG_STATE_HOME="$scratch/home/state"
unset WAYLAND_DISPLAY WAYLAND_SOCKET SCOOT_SOCKET

"$SCOOT_BIN" --headless --outputs 1 --width 480 --height 270 \
    --socket "$scratch/run/scoot.sock" --config "$scratch/empty.toml" \
    > "$scratch/scoot.log" 2>&1 &
scoot=$!
: > "$scratch/empty.toml"
for i in $(seq 1 100); do
    if [ -S "$scratch/run/scoot.sock" ]; then
        break
    fi
    sleep 0.1
done
# The compositor names its Wayland socket; the daemon and `msg` need it.
for i in $(seq 1 100); do
    WAYLAND_DISPLAY="$(ls "$scratch/run" | grep '^wayland-' | head -1)"
    if [ -n "$WAYLAND_DISPLAY" ]; then
        break
    fi
    sleep 0.1
done
export WAYLAND_DISPLAY

shot() {
    "$SCOOT_BIN" msg screenshot --output 1 --no-cursor --out "$scratch/$1.png"
}

"$SCOOTBG_BIN" daemon --profile strip > "$scratch/scootbg.log" 2>&1 &
bgd=$!
for i in $(seq 1 100); do
    "$SCOOTBG_BIN" query > /dev/null 2>&1 && break
    sleep 0.1
done

"$SCOOTBG_BIN" set "$RED"
frame=0
for kind in fade wipe grow; do
    #Fade the default timing, wipe from the left, grow from the center,
    # over three seconds each.
    case "$kind" in
        fade) extra="" ;;
        wipe) extra="--angle 0" ;;
        grow) extra="--position 0.5,0.5" ;;
    esac
    # shellcheck disable=SC2086
    "$SCOOTBG_BIN" set "$BLUE" --transition "$kind" --duration-ms 3000 $extra &
    changing=$!
    sleep 0.8; shot "f$frame"; frame=$((frame + 1))
    sleep 0.8; shot "f$frame"; frame=$((frame + 1))
    sleep 0.8; shot "f$frame"; frame=$((frame + 1))
    wait "$changing"
    shot "f$frame"; frame=$((frame + 1))
    "$SCOOTBG_BIN" set "$RED"
done

"$SCOOTBG_BIN" kill
wait "$bgd" 2>/dev/null || true

"$PYTHON3" - "$scratch" "$OUT" << 'PYEOF'
import struct, sys, zlib

scratch, out = sys.argv[1], sys.argv[2]

def decode(path):
    data = open(path, 'rb').read()
    assert data[:8] == b'\x89PNG\r\n\x1a\n', path
    pos, width, height, ctype, idat = 8, None, None, None, b''
    while pos < len(data):
        (length,) = struct.unpack('>I', data[pos:pos + 4])
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if kind == b'IHDR':
            width, height, depth, ctype, _, _, _ = struct.unpack('>IIBBBBB', body)
            assert depth == 8 and ctype in (2, 6), (path, depth, ctype)
        elif kind == b'IDAT':
            idat += body
        pos += 12 + length
    channels = 3 if ctype == 2 else 4
    raw = zlib.decompress(idat)
    stride = width * channels
    pixels, prev, p = bytearray(width * height * 3), bytearray(stride), 0
    for y in range(height):
        f = raw[p]; p += 1
        line = bytearray(raw[p:p + stride]); p += stride
        if f == 1:
            for i in range(channels, stride): line[i] = (line[i] + line[i - channels]) & 255
        elif f == 2:
            for i in range(stride): line[i] = (line[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = prev[i]
                c = prev[i - channels] if i >= channels else 0
                q = a + b - c
                pa, pb, pc = abs(q - a), abs(q - b), abs(q - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 255
        elif f != 0:
            raise AssertionError((path, 'filter', f))
        for x in range(width):
            o, q = (y * width + x) * 3, x * channels
            pixels[o:o + 3] = line[q:q + 3]
        prev = line
    return width, height, pixels

def encode(width, height, pixels):
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw += pixels[y * width * 3:(y + 1) * width * 3]
    ihdr = struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)
    idat = zlib.compress(bytes(raw), 9)
    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr) + chunk(b'IDAT', idat) + chunk(b'IEND', b'')

# Twelve frames: four per kind (three mid-flight, one final), in capture
# order fade, wipe, grow.
frames = [decode(f'{scratch}/f{i}.png') for i in range(12)]
w, h = frames[0][0], frames[0][1]
assert all((fw, fh) == (w, h) for fw, fh, _ in frames), 'one output, one size'
# Half size: the page shows a strip, not full screenshots.
sw, sh = w // 2, h // 2
small = []
for _, _, pixels in frames:
    down = bytearray(sw * sh * 3)
    for y in range(sh):
        for x in range(sw):
            s = ((2 * y) * w + 2 * x) * 3
            o = (y * sw + x) * 3
            down[o] = pixels[s]
            down[o + 1] = pixels[s + 1]
            down[o + 2] = pixels[s + 2]
    small.append(down)
strip = bytearray(sw * 4 * sh * 3 * 3)
for row in range(3):
    for col in range(4):
        src = small[row * 4 + col]
        for y in range(sh):
            o = ((row * sh + y) * sw * 4 + col * sw) * 3
            strip[o:o + sw * 3] = src[y * sw * 3:(y + 1) * sw * 3]
open(out, 'wb').write(encode(sw * 4, sh * 3, strip))
print(f'wrote {out}: {sw * 4}x{sh * 3}')
PYEOF
