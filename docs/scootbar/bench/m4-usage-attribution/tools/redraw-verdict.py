#!/usr/bin/env python3
"""redraw-verdict.py redraw.txt : the repo's verdict() (scripts/scootbg-bench/report.py) on the redraw rows,
the build's runs as the samples (its median per run), the tip as the baseline; run from the repository root."""
import json, sys, statistics
sys.path.insert(0, "scripts/scootbg-bench")
import report

runs, rounds = {}, {}
for line in open(sys.argv[1]):
    if "{" in line:
        name, body = line.split(" ", 1)
        data = json.loads(body)
        runs.setdefault(name, []).append(data["median_us"])
        rounds.setdefault(name, []).extend(data["us_per_set"])
word = {"tie": "same", "win": "REGRESSED", "beaten": "better"}
for name in ("cand", "opts", "opt2"):
    v, margin = report.verdict(runs["tip"], runs[name])
    v2, margin2 = report.verdict(rounds["tip"], rounds[name])
    change = 100 * (statistics.median(rounds[name]) / statistics.median(rounds["tip"]) - 1)
    print(f"{name} against tip: runs' medians {runs[name]} against {runs['tip']}: {word[v]} (margin {margin:.1f} us); "
          f"all {len(rounds[name])} rounds: {word[v2]} (margin {margin2:.1f} us); median of rounds {change:+.1f}%")
