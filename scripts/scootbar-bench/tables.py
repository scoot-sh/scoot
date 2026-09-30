"""The tables and the two ratchet gates, from ``runs.jsonl`` and
``meta.json``.

The noise rule is scootbg's (``scripts/scootbg-bench/report.py``,
``verdict``): lower is better; one side beats the other when its median
is lower by more than the largest of 5% of the reference's median, the two
sides' combined spread (max − min), and the unit's absolute floor; else a
tie, and a tie does not block. The two gates are the ratchet's rules
(docs/scootbar/backlog/lightest.md, numbered as there), and ``bench.py``
exits 1 when either fails:

1. ``compare``: **no row of scootbar regresses** against an earlier run
   (the last milestone's).
2. ``report``: **no competitor beats scootbar** at the milestone's scope,
   on any gated row both have. A gated row a competitor has and scootbar
   lacks (its runs failed, or it was not run) fails it too: a gate that
   cannot be judged has not been passed. So does a competitor that cannot
   show the scope on a compositor at all (yambar has no ext-workspace
   module, so not on scoot): it is named, never given a number.

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
    ``(text, losses, ties, unjudged)``."""
    bars = list(meta["bars"])
    lines = ["| Row | " + " | ".join(bars) + " |", "|---|" + "---|" * len(bars)]
    losses, ties, unjudged = [], [], []

    def emit(label, vals, unit, gated):
        ref = vals.get(REFERENCE)
        cells = []
        for name in bars:
            if not vals.get(name):
                if meta["bars"][name].get("cannot_show", {}).get(compositor):
                    cells.append("cannot show this scope")
                elif not meta["bars"][name].get("ran", {}).get(compositor):
                    cells.append("did not run")
                else:
                    cells.append("n/a")
                continue
            text = scootbg_report.cell(vals[name], unit)
            if name != REFERENCE and not ref and gated:
                unjudged.append((compositor, label, name))
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
    return "\n".join(lines), losses, ties, unjudged


def failures(runs):
    return [r for r in runs if not r.get("ok")]


def render(results_dir):
    """The tables and rule 2's verdict, as ``report`` prints them."""
    return gate(results_dir)[0]


def gate(results_dir):
    """The tables, and how many findings fail rule 2 (a competitor ahead,
    or a gated row scootbar has no value for): ``(text, failed)``."""
    meta, runs = load(results_dir)
    out = []
    all_losses = []
    all_unjudged = []
    for compositor in meta["compositors"]:
        text, losses, ties, unjudged = table(meta, runs, compositor)
        all_losses += losses
        all_unjudged += unjudged
        out += [f"**On {compositor}** ({meta['compositor_versions'].get(compositor, '')})", "", text, ""]
        for _, label, name in ties:
            out.append(f"- tie: {label}: {name}")
        if ties:
            out.append("")
    out += machine_lines(meta, runs)
    info = meta["bars"].get(REFERENCE, {}).get("code")
    if info:
        out.append(
            f"scootbar's code: {info['lines']:,} lines of Rust in `crates/scootbar/src`, "
            f"{info['lines_outside_tests']:,} outside `tests.rs` files; "
            f"{info['direct_dependencies']} direct dependencies on Linux "
            f"({', '.join(info['dependency_names'])})."
        )
        out.append("")
    not_compared = [(c, name, why) for name, b in meta["bars"].items()
                    for c, why in b.get("cannot_show", {}).items()]
    out.append(f"Gate (no competitor beats scootbar): {len(all_losses)} loss(es).")
    for compositor, label, name, ref, other, margin in all_losses:
        out.append(f"- LOSS on {compositor}: {label}: {name} {other:.2f} against "
                   f"scootbar {ref:.2f} (margin {margin:.2f})")
    if all_unjudged:
        out.append(f"Not judged, so not passed: {len(all_unjudged)} gated row(s) with no "
                   "scootbar value.")
    for compositor, label, name in all_unjudged:
        out.append(f"- NO SCOOTBAR VALUE on {compositor}: {label}: {name} has one")
    if not_compared:
        out.append(f"Not compared, so not passed: {len(not_compared)} bar and compositor "
                   "pair(s) cannot show the milestone's scope, so no row was measured or "
                   "invented for them (the rule does not say what to do then: a maintainer "
                   "call).")
    for compositor, name, why in not_compared:
        out.append(f"- NOT COMPARED on {compositor}: {name}: {why}")
    bad = failures(runs)
    if bad:
        out += ["", f"{len(bad)} failed run(s):"]
        for r in bad:
            first = (r.get("error") or "").splitlines()[:1]
            out.append(f"- {r['compositor']} {r['bar']} {r['row']} round {r['round']}: {first}")
    return "\n".join(out), len(all_losses) + len(all_unjudged) + len(not_compared)


def machine_lines(meta, runs):
    """What the machine did during the runs (``machine.py``): whether the
    clocks were capped or mains power lost in any record, and the range of
    the current frequencies seen, so a throttled run says so beside its
    numbers. Nothing when the runs carry no readings (M1's do not)."""
    states = [s for r in runs for s in (r.get("hw_start"), r.get("hw_end")) if s]
    if not states:
        return []
    capped = sum(1 for s in states
                 if any(c["policy_max_khz"] and c["hw_max_khz"]
                        and c["policy_max_khz"] < c["hw_max_khz"] for c in s["cpus"].values()))
    curs = [c["cur_khz"] for s in states for c in s["cpus"].values() if c["cur_khz"]]
    governors = sorted({c["governor"] for s in states for c in s["cpus"].values()
                        if c["governor"]})
    mains = [p.get("online") for s in states for p in s["power"].values()
             if p.get("type") == "Mains"]
    temps = [v for s in states for v in s["temps_c"].values()]
    line = (f"Machine, {len(states)} readings around the runs: governor "
            f"{'/'.join(governors) or '?'}; cpufreq policy cap below the hardware maximum in "
            f"{capped}; current frequency seen {min(curs) / 1000:.0f} to {max(curs) / 1000:.0f} MHz"
            if curs else f"Machine, {len(states)} readings around the runs")
    if mains:
        line += f"; a mains supply offline in {sum(1 for m in mains if m == '0')} of them"
    if temps:
        line += f"; hwmon temperatures {min(temps):.0f} to {max(temps):.0f} C"
    return [line + ".", ""]


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
