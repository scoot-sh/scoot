"""Buffer commits, as the compositor received them.

Every timing row ends at "the wallpaper is on screen", and every daemon
uses a different Wayland library (libwayland in C, wayland-rs, Mesa's EGL
on top of either). One observer serves them all: the compositor, run with
``WAYLAND_DEBUG=server``, prints every request it receives with a
wall-clock timestamp. A buffer commit is a ``wl_surface.commit`` on a
surface whose pending state has a non-null ``wl_surface.attach`` since its
previous commit.

Two formats, one per server library:

- wayland-rs (scoot, through Smithay):
  ``[ 204695.751][rs] <- wl_surface@10.commit, ()``: the time is
  CLOCK_REALTIME in microseconds, truncated to 32 bits, printed as
  milliseconds.
- libwayland 1.24+ (sway): ``[21:58:30.696327] wl_surface#11.commit()``:
  local time of day with microseconds. Requests carry no arrow; events it
  sends are printed with `` -> ``.
"""

import re
import time

_RS = re.compile(
    r"^\[\s*(\d+)\.(\d{3})\]\[rs\] <- wl_surface@(\d+)\.(attach|commit|destroy), \((.*)\)\s*$"
)
_LIBWAYLAND = re.compile(
    r"^\[(\d\d):(\d\d):(\d\d)\.(\d{6})\]\s+(?:\{[^}]*\}\s+)?"
    r"wl_surface#(\d+)\.(attach|commit|destroy)\((.*)\)\s*$"
)
_NULL_BUFFERS = {"0", "null", "nil", "NULL"}


def _unwrap_u32(truncated_us, reference_us):
    """The full microsecond time that truncates to ``truncated_us`` and is
    nearest ``reference_us``."""
    span = 1 << 32
    k = round((reference_us - truncated_us) / span)
    return truncated_us + k * span


def _time_of_day_to_us(hh, mm, ss, us, reference_us):
    """Local ``hh:mm:ss.us`` as epoch microseconds nearest the reference
    (so a trace across midnight still resolves)."""
    ref_s = reference_us / 1e6
    lt = time.localtime(ref_s)
    midnight = time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday, 0, 0, 0, 0, 0, -1))
    of_day = (hh * 3600 + mm * 60 + ss) * 1_000_000 + us
    best = None
    for day in (-1, 0, 1):
        candidate = int((midnight + day * 86400) * 1_000_000) + of_day
        if best is None or abs(candidate - reference_us) < abs(best - reference_us):
            best = candidate
    return best


def parse_line(line, reference_us):
    """``(time_us, surface_id, request, args)`` for a ``wl_surface`` attach,
    commit or destroy request, else ``None``."""
    m = _RS.match(line)
    if m:
        truncated = int(m.group(1)) * 1000 + int(m.group(2))
        return (_unwrap_u32(truncated, reference_us), int(m.group(3)), m.group(4), m.group(5))
    m = _LIBWAYLAND.match(line)
    if m:
        hh, mm, ss, us = (int(m.group(i)) for i in range(1, 5))
        return (
            _time_of_day_to_us(hh, mm, ss, us, reference_us),
            int(m.group(5)),
            m.group(6),
            m.group(7),
        )
    return None


def attach_is_null(args):
    first = args.split(",", 1)[0].strip()
    return first in _NULL_BUFFERS


class Trace:
    """Reads a compositor's ``WAYLAND_DEBUG=server`` stderr as it grows and
    keeps every buffer commit as ``(time_us, surface_id)``.

    Surface ids are the client's; the harness runs one Wayland client per
    compositor at a time, and a ``destroy`` forgets a surface's pending
    state, so an id reused by a later daemon starts clean.
    """

    def __init__(self, path):
        self.path = path
        self._offset = 0
        self._partial = b""
        self._pending = {}  # surface id -> a non-null attach since the last commit
        self.commits = []  # (time_us, surface id)

    def poll(self):
        try:
            with open(self.path, "rb") as f:
                f.seek(self._offset)
                chunk = f.read()
        except FileNotFoundError:
            return
        if not chunk:
            return
        self._offset += len(chunk)
        data = self._partial + chunk
        lines = data.split(b"\n")
        self._partial = lines.pop()
        reference = time.time_ns() // 1000
        for raw in lines:
            parsed = parse_line(raw.decode("utf-8", "replace"), reference)
            if parsed is None:
                continue
            t, surface, request, args = parsed
            if request == "attach":
                self._pending[surface] = not attach_is_null(args)
            elif request == "commit":
                if self._pending.pop(surface, False):
                    self.commits.append((t, surface))
            else:  # destroy
                self._pending.pop(surface, None)

    def since(self, t0_us):
        return [(t, s) for (t, s) in self.commits if t >= t0_us]
