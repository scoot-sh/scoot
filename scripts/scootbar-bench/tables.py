"""The tables and the two ratchet gates, from ``runs.jsonl`` and
``meta.json``.

The noise rule is scootbg's (``scripts/scootbg-bench/report.py``,
``verdict``): lower is better; one side beats the other when its median
is lower by more than the largest of 5% of the reference's median, the two
sides' combined spread (max − min), and the unit's absolute floor; else a
tie, and a tie does not block. The two gates are the ratchet's
(docs/scootbar/backlog/lightest.md):

1. ``report``: **no competitor beats scootbar** at the milestone's scope,
   on any gated row both have.
2. ``compare``: **no row of scootbar regresses** against an earlier run
   (the last milestone's).

Rows the ratchet does not gate are shown, marked so: the bare executable
(size is judged with what it links, as ruled), threads and the switching
wakeups (context, not a cost the ratchet names).
"""

import json
import statistics

import report as scootbg_report

REFERENCE = "scootbar"

# (label, row, metric, unit, gated). Memory is kept in kB and shown in MiB.
ROWS = [
    ("Startup to first frame", "startup", "latency_ms", "ms", True),
    ("Idle RSS", "idle", "rss_kb", "MiB", True),
    ("Idle PSS", "idle", "pss_kb", "MiB", True),
    ("Idle heap (`RssAnon`)", "idle", "rss_anon_kb", "MiB", True),
    ("Peak memory (`VmHWM`)", "idle", "hwm_kb", "MiB", True),
    ("Idle wakeups per minute", "idle", "wakeups_per_min", "", True),
    ("Idle CPU in the window", "idle", "idle_cpu_ms", "ms", True),
    ("CPU while switching workspaces", "idle", "switch_cpu_ms", "ms", True),
    ("Wakeups while switching workspaces", "idle", "switch_wakeups", "", False),
    ("Threads", "idle", "threads", "", False),
]

STATIC = [
    ("Size: stripped binary + non-glibc `ldd` closure", "size_bytes", "B", True),
    ("Bare executable, stripped", "binaries_bytes", "B", False),
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


def series(runs, compositor, bar, row, metric, scale):
    return [
        r[metric] * scale
        for r in runs
        if r.get("ok")
        and r["compositor"] == compositor
        and r["bar"] == bar
        and r["row"] == row
        and r.get(metric) is not None
    ]


def _scale(unit):
    return 1 / 1024 if unit == "MiB" else 1


def table(meta, runs, compositor):
    """One compositor's table, and the gate's findings:
    ``(text, losses, ties)``."""
    bars = list(meta["bars"])
    lines = ["| Row | " + " | ".join(bars) + " |", "|---|" + "---|" * len(bars)]
    losses, ties = [], []

    def emit(label, vals, unit, gated):
        ref = vals.get(REFERENCE)
        cells = []
        for name in bars:
            if not vals.get(name):
                cells.append("did not run" if not meta["bars"][name].get("ran", {}).get(compositor)
                             else "n/a")
                continue
            text = scootbg_report.cell(vals[name], unit)
            if name != REFERENCE and ref and gated:
                verdict, margin = scootbg_report.verdict(ref, vals[name], unit)
                if verdict == "beaten":
                    text = f"**{text}** (beats scootbar)"
                    losses.append((compositor, label, name, statistics.median(ref),
                                   statistics.median(vals[name]), margin))
                elif verdict == "tie":
                    ties.append((compositor, label, name))
            cells.append(text)
        suffix = f" ({unit})" if unit and unit != "B" else (" (bytes)" if unit == "B" else "")
        note = "" if gated else " *(not gated)*"
        lines.append(f"| {label}{suffix}{note} | " + " | ".join(cells) + " |")

    for label, key, unit, gated in STATIC:
        vals = {n: [b["static"][key]] for n, b in meta["bars"].items()
                if b.get("static", {}).get(key) is not None}
        emit(label, vals, unit, gated)
    for label, row, metric, unit, gated in ROWS:
        vals = {n: series(runs, compositor, n, row, metric, _scale(unit)) for n in bars}
        if any(vals.values()):
            emit(label, vals, unit, gated)
    return "\n".join(lines), losses, ties


def failures(runs):
    return [r for r in runs if not r.get("ok")]


def render(results_dir):
    meta, runs = load(results_dir)
    out = []
    all_losses = []
    for compositor in meta["compositors"]:
        text, losses, ties = table(meta, runs, compositor)
        all_losses += losses
        out += [f"**On {compositor}** ({meta['compositor_versions'].get(compositor, '')})", "", text, ""]
        for _, label, name in ties:
            out.append(f"- tie: {label}: {name}")
        if ties:
            out.append("")
    info = meta["bars"].get(REFERENCE, {}).get("code")
    if info:
        out.append(
            f"scootbar's code: {info['lines']:,} lines of Rust in `crates/scootbar/src`, "
            f"{info['lines_outside_tests']:,} outside `tests.rs` files; "
            f"{info['direct_dependencies']} direct dependencies on Linux "
            f"({', '.join(info['dependency_names'])})."
        )
        out.append("")
    out.append(f"Gate (no competitor beats scootbar): {len(all_losses)} loss(es).")
    for compositor, label, name, ref, other, margin in all_losses:
        out.append(f"- LOSS on {compositor}: {label}: {name} {other:.2f} against "
                   f"scootbar {ref:.2f} (margin {margin:.2f})")
    bad = failures(runs)
    if bad:
        out += ["", f"{len(bad)} failed run(s):"]
        for r in bad:
            first = (r.get("error") or "").splitlines()[:1]
            out.append(f"- {r['compositor']} {r['bar']} {r['row']} round {r['round']}: {first}")
    return "\n".join(out)


def compare(results_dir, baseline_dir):
    """scootbar now against scootbar in an earlier run, row by row, by the
    same rule: the ratchet's no-regression check. Returns ``(text,
    regressions)``."""
    meta, runs = load(results_dir)
    bmeta, bruns = load(baseline_dir)
    out = ["| Compositor | Row | baseline | now | verdict |", "|---|---|---|---|---|"]
    regressions = 0

    def judge(compositor, label, base, now, unit, gated):
        nonlocal regressions
        verdict, _ = scootbg_report.verdict(base, now, unit)
        word = {"beaten": "better", "win": "**REGRESSED**", "tie": "same"}[verdict]
        if not gated:
            word += " (not gated)"
        elif verdict == "win":
            regressions += 1
        out.append(f"| {compositor} | {label} | {scootbg_report.cell(base, unit)} | "
                   f"{scootbg_report.cell(now, unit)} | {word} |")

    for compositor in meta["compositors"]:
        if compositor not in bmeta["compositors"]:
            continue
        for label, row, metric, unit, gated in ROWS:
            now = series(runs, compositor, REFERENCE, row, metric, _scale(unit))
            base = series(bruns, compositor, REFERENCE, row, metric, _scale(unit))
            if now and base:
                judge(compositor, label, base, now, unit, gated)
    for label, key, unit, gated in STATIC:
        now = meta["bars"].get(REFERENCE, {}).get("static", {}).get(key)
        base = bmeta["bars"].get(REFERENCE, {}).get("static", {}).get(key)
        if now is not None and base is not None:
            judge("any", label, [base], [now], unit, gated)
    out += ["", f"{regressions} regression(s) beyond the margin."]
    return "\n".join(out), regressions
