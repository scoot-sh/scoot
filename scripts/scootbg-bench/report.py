"""The table and the gate, from ``runs.jsonl`` and ``meta.json``.

Every cell is the median of the rounds with its range, ``median
[min–max]``. The gate is the ticket's rule, per row, for lower-is-better
figures: a competitor **beats** scootbg when its median is lower by more
than the larger of 5% of scootbg's median and the two sides' combined
spread (each side's max − min); scootbg **wins** by the same margin the
other way; anything inside is a tie, and a tie does not block. A row a
daemon cannot do is "n/a", never a win; a daemon that did not run is
"did not run".
"""

import json
import statistics

REFERENCE = "scootbg"

# (label, row, variant, geometry, metric, unit, gated). Memory is kept in
# kB in the runs and shown in MiB. The idle rows with the floor as one
# process sees it (raw RSS and PSS) are shown but not gated: a daemon that
# unmaps its buffer once committed (swaybg) keeps the pixels alive in the
# compositor instead, so its own RSS drops with no memory saved. The gated
# figure with the floor is the total (see ``scenarios.idle``).
ROWS = [
    ('Idle RSS, 1× 1080p, image', 'idle', 'image', '1x1920x1080', 'rss_kb', 'MiB', False),
    ('Idle PSS, 1× 1080p, image', 'idle', 'image', '1x1920x1080', 'pss_kb', 'MiB', False),
    ('Idle floor (the buffers the compositor maps), 1× 1080p, image', 'idle', 'image', '1x1920x1080', 'floor_kb', 'MiB', False),
    ('Idle RSS above the floor, 1× 1080p, image', 'idle', 'image', '1x1920x1080', 'rss_above_floor_kb', 'MiB', True),
    ('Idle PSS above the floor, 1× 1080p, image', 'idle', 'image', '1x1920x1080', 'pss_above_floor_kb', 'MiB', True),
    ("Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image", 'idle', 'image', '1x1920x1080', 'total_pss_kb', 'MiB', True),
    ('Idle RSS, 2× 4K, image', 'idle', 'image', '2x3840x2160', 'rss_kb', 'MiB', False),
    ('Idle PSS, 2× 4K, image', 'idle', 'image', '2x3840x2160', 'pss_kb', 'MiB', False),
    ('Idle floor (the buffers the compositor maps), 2× 4K, image', 'idle', 'image', '2x3840x2160', 'floor_kb', 'MiB', False),
    ('Idle RSS above the floor, 2× 4K, image', 'idle', 'image', '2x3840x2160', 'rss_above_floor_kb', 'MiB', True),
    ('Idle PSS above the floor, 2× 4K, image', 'idle', 'image', '2x3840x2160', 'pss_above_floor_kb', 'MiB', True),
    ("Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image", 'idle', 'image', '2x3840x2160', 'total_pss_kb', 'MiB', True),
    ('Idle RSS, 1× 1080p, color', 'idle', 'color', '1x1920x1080', 'rss_kb', 'MiB', False),
    ('Idle PSS, 1× 1080p, color', 'idle', 'color', '1x1920x1080', 'pss_kb', 'MiB', False),
    ('Idle floor (the buffers the compositor maps), 1× 1080p, color', 'idle', 'color', '1x1920x1080', 'floor_kb', 'MiB', False),
    ('Idle RSS above the floor, 1× 1080p, color', 'idle', 'color', '1x1920x1080', 'rss_above_floor_kb', 'MiB', True),
    ('Idle PSS above the floor, 1× 1080p, color', 'idle', 'color', '1x1920x1080', 'pss_above_floor_kb', 'MiB', True),
    ("Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color", 'idle', 'color', '1x1920x1080', 'total_pss_kb', 'MiB', True),
    ('Idle RSS, 2× 4K, color', 'idle', 'color', '2x3840x2160', 'rss_kb', 'MiB', False),
    ('Idle PSS, 2× 4K, color', 'idle', 'color', '2x3840x2160', 'pss_kb', 'MiB', False),
    ('Idle floor (the buffers the compositor maps), 2× 4K, color', 'idle', 'color', '2x3840x2160', 'floor_kb', 'MiB', False),
    ('Idle RSS above the floor, 2× 4K, color', 'idle', 'color', '2x3840x2160', 'rss_above_floor_kb', 'MiB', True),
    ('Idle PSS above the floor, 2× 4K, color', 'idle', 'color', '2x3840x2160', 'pss_above_floor_kb', 'MiB', True),
    ("Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color", 'idle', 'color', '2x3840x2160', 'total_pss_kb', 'MiB', True),
    ('Idle wakeups in 60 s, 1× 1080p, image', 'idle', 'image', '1x1920x1080', 'wakeups', '', True),
    ('Idle wakeups in 60 s, 2× 4K, image', 'idle', 'image', '2x3840x2160', 'wakeups', '', True),
    ('Idle wakeups in 60 s, 1× 1080p, color', 'idle', 'color', '1x1920x1080', 'wakeups', '', True),
    ('Idle wakeups in 60 s, 2× 4K, color', 'idle', 'color', '2x3840x2160', 'wakeups', '', True),
    ('Idle CPU in 60 s, 1× 1080p, image', 'idle', 'image', '1x1920x1080', 'window_cpu_ms', 'ms', True),
    ('Idle CPU in 60 s, 2× 4K, image', 'idle', 'image', '2x3840x2160', 'window_cpu_ms', 'ms', True),
    ('Idle CPU in 60 s, 1× 1080p, color', 'idle', 'color', '1x1920x1080', 'window_cpu_ms', 'ms', True),
    ('Idle CPU in 60 s, 2× 4K, color', 'idle', 'color', '2x3840x2160', 'window_cpu_ms', 'ms', True),
    ('Peak memory, JPEG at start-up, 1× 4K', 'startup', 'image', '1x3840x2160', 'peak_kb', 'MiB', True),
    ('Peak memory, live change to the JPEG, 1× 4K', 'set', 'image', '1x3840x2160', 'peak_kb', 'MiB', True),
    ('Set: latency to the JPEG', 'set', 'image', '1x3840x2160', 'latency_ms', 'ms', True),
    ('Set: CPU for the JPEG', 'set', 'image', '1x3840x2160', 'cpu_ms', 'ms', True),
    ('Set: latency to a color', 'set', 'color', '1x3840x2160', 'latency_ms', 'ms', True),
    ('Set: CPU for a color', 'set', 'color', '1x3840x2160', 'cpu_ms', 'ms', True),
    ('Startup: to a color on screen', 'startup', 'color', '1x3840x2160', 'latency_ms', 'ms', True),
    ('Startup: CPU, color', 'startup', 'color', '1x3840x2160', 'cpu_ms', 'ms', True),
    ('Startup: to the JPEG on screen', 'startup', 'image', '1x3840x2160', 'latency_ms', 'ms', True),
    ('Startup: CPU, JPEG', 'startup', 'image', '1x3840x2160', 'cpu_ms', 'ms', True),
    ('Restore: to the JPEG on screen', 'restore', 'image', '1x3840x2160', 'latency_ms', 'ms', True),
    ('Restore: CPU, JPEG', 'restore', 'image', '1x3840x2160', 'cpu_ms', 'ms', True),
    ('Restore: to a color on screen', 'restore', 'color', '1x3840x2160', 'latency_ms', 'ms', True),
    ('Restore: CPU, color', 'restore', 'color', '1x3840x2160', 'cpu_ms', 'ms', True),
]


def load(results_dir):
    with open(f"{results_dir}/meta.json") as f:
        meta = json.load(f)
    runs = []
    with open(f"{results_dir}/runs.jsonl") as f:
        for line in f:
            if line.strip():
                runs.append(json.loads(line))
    return meta, runs


def verdict(ref, other):
    """``"beaten"`` when ``other`` (lower is better) beats ``ref`` beyond
    the margin, ``"win"`` the other way, else ``"tie"``; with the margin."""
    mr, mo = statistics.median(ref), statistics.median(other)
    spread = (max(ref) - min(ref)) + (max(other) - min(other))
    margin = max(0.05 * abs(mr), spread)
    if mr - mo > margin:
        return "beaten", margin
    if mo - mr > margin:
        return "win", margin
    return "tie", margin


def _fmt(v, unit):
    if unit == "MiB":
        return f"{v:.1f}"
    if unit == "ms":
        return f"{v:.0f}" if v >= 100 else f"{v:.1f}"
    if unit == "B":
        return f"{v:,.0f}"
    return f"{v:.0f}"


def cell(values, unit):
    m = statistics.median(values)
    if len(values) == 1 or min(values) == max(values):
        return _fmt(m, unit)
    return f"{_fmt(m, unit)} [{_fmt(min(values), unit)}–{_fmt(max(values), unit)}]"


def series(runs, daemon, row, variant, geometry, metric, scale):
    return [
        r[metric] * scale
        for r in runs
        if r.get("ok")
        and r["daemon"] == daemon
        and r["row"] == row
        and r["variant"] == variant
        and r["geometry"] == geometry
        and r.get(metric) is not None
    ]


def static_rows(meta):
    """Size and Disk: one measurement each (no spread), in bytes."""
    rows = []
    for label, key in (
        ("Size: stripped binaries + non-glibc `ldd` closure", "size_bytes"),
        ("Disk: installed with its non-glibc closure, plus what it writes", "disk_bytes"),
    ):
        vals = {}
        for name, d in meta["daemons"].items():
            v = d.get("static", {}).get(key)
            if v is not None:
                vals[name] = [v]
        rows.append((label, vals, "B"))
    return rows


def build(meta, runs):
    daemons = list(meta["daemons"].keys())
    ran = {n for n, d in meta["daemons"].items() if d.get("ran")}
    header = "| Row | " + " | ".join(daemons) + " |"
    sep = "|---|" + "---|" * len(daemons)
    lines = [header, sep]
    losses, wins, ties = [], [], []

    def emit(label, vals, unit, applies, gated=True):
        cells = []
        ref = vals.get(REFERENCE)
        for n in daemons:
            if n not in ran:
                cells.append("did not run")
                continue
            if not applies(n) or not vals.get(n):
                cells.append("n/a")
                continue
            text = cell(vals[n], unit)
            informational = meta["daemons"][n].get("informational")
            if n != REFERENCE and ref and gated and not informational:
                v, margin = verdict(ref, vals[n])
                if v == "beaten":
                    text = f"**{text}** (beats scootbg)"
                    losses.append((label, n, statistics.median(ref), statistics.median(vals[n]), margin))
                elif v == "win":
                    wins.append((label, n))
                else:
                    ties.append((label, n))
            cells.append(text)
        suffix = f" ({unit})" if unit and unit != "B" else (" (bytes)" if unit == "B" else "")
        info = "" if gated else " *(not gated)*"
        lines.append(f"| {label}{suffix}{info} | " + " | ".join(cells) + " |")

    for label, vals, unit in static_rows(meta):
        emit(label, vals, unit, lambda n, vals=vals: n in vals)
    for label, row, variant, geometry, metric, unit, gated in ROWS:
        scale = 1 / 1024 if unit == "MiB" else 1
        vals = {
            n: series(runs, n, row, variant, geometry, metric, scale) for n in daemons
        }
        supported = meta["supports"]

        def applies(n, row=row, variant=variant):
            return supported.get(n, {}).get(f"{row}:{variant}", False)

        emit(label, vals, unit, applies, gated)
    return "\n".join(lines), losses, wins, ties


def failures(runs):
    return [r for r in runs if not r.get("ok") and r["row"] != "check"]


def render(results_dir):
    meta, runs = load(results_dir)
    table, losses, wins, ties = build(meta, runs)
    out = [table, ""]
    out.append(f"Gate: {len(losses)} loss(es), {len(wins)} win(s), {len(ties)} tie(s) for scootbg.")
    for label, n, ref, other, margin in losses:
        out.append(f"- LOSS: {label}: {n} {other:.2f} against scootbg {ref:.2f} (margin {margin:.2f})")
    for label, n in ties:
        out.append(f"- tie: {label}: {n}")
    bad = failures(runs)
    if bad:
        out.append("")
        out.append(f"{len(bad)} failed run(s):")
        for r in bad:
            first = (r.get("error") or "").splitlines()[:1]
            out.append(f"- {r['daemon']} {r['row']} {r['variant']} round {r['round']}: {first}")
    return "\n".join(out)


def compare(results_dir, baseline_dir):
    """scootbg now against scootbg in a baseline run, row by row, by the
    same rule: the regression check for later changes."""
    meta, runs = load(results_dir)
    bmeta, bruns = load(baseline_dir)
    out = ["| Row | baseline | now | verdict |", "|---|---|---|---|"]
    regressions = 0
    for label, row, variant, geometry, metric, unit, _gated in ROWS:
        scale = 1 / 1024 if unit == "MiB" else 1
        now = series(runs, REFERENCE, row, variant, geometry, metric, scale)
        base = series(bruns, REFERENCE, row, variant, geometry, metric, scale)
        if not now or not base:
            continue
        v, _ = verdict(base, now)
        word = {"beaten": "better", "win": "**REGRESSED**", "tie": "same"}[v]
        regressions += v == "win"
        out.append(f"| {label} | {cell(base, unit)} | {cell(now, unit)} | {word} |")
    for label, vals, unit in static_rows(meta):
        bvals = {lbl: v for lbl, v, _unit in static_rows(bmeta)}.get(label, {})
        if REFERENCE in vals and REFERENCE in bvals:
            v, _ = verdict(bvals[REFERENCE], vals[REFERENCE])
            word = {"beaten": "better", "win": "**REGRESSED**", "tie": "same"}[v]
            regressions += v == "win"
            out.append(f"| {label} | {cell(bvals[REFERENCE], unit)} | {cell(vals[REFERENCE], unit)} | {word} |")
    out.append("")
    out.append(f"{regressions} regression(s) beyond the margin.")
    return "\n".join(out), regressions
