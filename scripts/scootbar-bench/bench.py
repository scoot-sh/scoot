#!/usr/bin/env python3
"""scootbar against the bars it is measured with: the resource ratchet's
benchmark (docs/scootbar/backlog/lightest.md).

    scripts/scootbar-bench/bench.py run --out DIR [--compositors scoot,sway]
        [--bars scootbar,yambar,waybar,ironbar,ashell] [--scope clock|clock-workspaces] [--rounds 5]
        [--settle-secs 30] [--idle-secs 300] [--switches 240] [--switch-hz 4]
    scripts/scootbar-bench/bench.py report DIR
    scripts/scootbar-bench/bench.py compare DIR BASELINE_DIR

Run it from the repository root inside the dev shell (``devenv shell --
python3 scripts/scootbar-bench/bench.py run ...``: it needs ``nix``,
``strip`` and ImageMagick), as root or with rights to make a cgroup (CPU is
accounted per bar run by one), after ``cargo build --release -p scootbar``
and a build of ``scoot`` (any profile: the compositor's
own cost is not measured; driven via ``scoot msg``, the only client). The competitors, sway, foot, the session bus
and DejaVu come from the flake's pinned nixpkgs.

It reuses ``scripts/scootbg-bench``'s runner: the headless compositor with
its protocol trace (``session.py``, ``commits.py``), the per-run cgroup and
``/proc`` readings (``procs.py``), the Size row (``size.py``) and the noise
rule (``report.py``). What is scootbar's own: the bars and their configs
(``bars.py``), the stage with two windows on two workspaces
(``stage.py``), the rows (``measure.py``) and the tables and gates
(``tables.py``).

``run`` writes ``DIR/meta.json`` (machine, commit, versions, store paths,
the settings, the static rows) and ``DIR/runs.jsonl`` (every raw run, one
JSON object per line, appended as it finishes), then ``DIR/table.md``.
``report`` re-renders the tables and the competitor gate (the ratchet's
rule 2), and exits 1 when a competitor beats scootbar on a gated row or a
gated row has no scootbar value to judge; ``compare`` checks scootbar
against an earlier results directory (the last milestone's), the
ratchet's rule 1, and exits 1 on a regression. The method is in
docs/scootbar/testing.md.
"""

import argparse
import hashlib
import json
import os
import platform
import shutil
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
# scootbg's runner, after this directory, so a name here is never shadowed.
sys.path.append(os.path.join(REPO, "scripts", "scootbg-bench"))

import bars as B  # noqa: E402
import machine as hw  # noqa: E402
import measure  # noqa: E402
import size  # noqa: E402
import tables  # noqa: E402
from daemons import nix_resolve  # noqa: E402
from stage import Stage  # noqa: E402


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def git(*args):
    return subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True).stdout.strip()


def version_line(argv, env=None):
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=30,
                           env=dict(os.environ, **(env or {})))
    except (OSError, subprocess.TimeoutExpired) as e:
        return f"unknown ({e})"
    text = (r.stdout + "\n" + r.stderr).strip()
    return next((line.strip() for line in text.splitlines() if "version" in line.lower()
                 or line.startswith(("scoot", "sway"))), text[:200])


def machine():
    info = {"kernel": platform.release(), "arch": platform.machine(), "cpus": os.cpu_count(),
            "page_size": os.sysconf("SC_PAGE_SIZE"), "loadavg_start": os.getloadavg(),
            "start": hw.state()}
    try:
        with open("/proc/cpuinfo") as f:
            info["cpu"] = next(line.split(":", 1)[1].strip() for line in f
                               if line.startswith("model name"))
    except (OSError, StopIteration):
        # aarch64's /proc/cpuinfo has no model name: lscpu has.
        try:
            lscpu = subprocess.run(["lscpu"], capture_output=True, text=True, timeout=10).stdout
            names = [line.split(":", 1)[1].strip() for line in lscpu.splitlines()
                     if line.startswith("Model name")]
            vendor = next((line.split(":", 1)[1].strip() for line in lscpu.splitlines()
                           if line.startswith("Vendor ID")), "")
            info["cpu"] = f"{vendor} {' + '.join(names)}".strip()
        except (OSError, subprocess.SubprocessError):
            pass
    return info


def code_size(source):
    """scootbar's own size, the ratchet's maintainability rows: lines of
    Rust under ``crates/scootbar/src`` of the tree ``source`` (the one the
    binary was built from) and outside its ``tests.rs`` files, and its
    direct dependencies on Linux (normal ones, from ``cargo metadata``)."""
    root = os.path.join(source, "crates", "scootbar", "src")
    lines = outside = 0
    for dirpath, _dirs, names in os.walk(root):
        for name in names:
            if name.endswith(".rs"):
                with open(os.path.join(dirpath, name), encoding="utf-8") as f:
                    n = sum(1 for _ in f)
                lines += n
                outside += 0 if name == "tests.rs" else n
    r = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"],
                       cwd=source, capture_output=True, text=True, check=True)
    package = next(p for p in json.loads(r.stdout)["packages"] if p["name"] == "scootbar")
    names = sorted({
        d["name"] for d in package["dependencies"]
        if d["kind"] is None and (d["target"] is None or "linux" in d["target"])
    })
    return {"lines": lines, "lines_outside_tests": outside,
            "direct_dependencies": len(names), "dependency_names": names}


def cmd_run(a):
    os.makedirs(a.out, exist_ok=True)
    strip = shutil.which("strip")
    if not strip:
        sys.exit("no `strip` on PATH: run in the dev shell")
    magick = shutil.which("magick")
    compositors = a.compositors.split(",")
    wanted = a.bars.split(",")
    unknown = set(wanted) - {c.name for c in B.ALL}
    if unknown:
        sys.exit(f"unknown bars: {', '.join(sorted(unknown))}")
    if a.scope not in B.SCOPES:
        sys.exit(f"unknown scope {a.scope}: one of {', '.join(B.SCOPES)}")
    for path in (a.scootbar, a.scoot):
        if not os.path.isfile(path):
            sys.exit(f"no {path}: cargo build --release -p scootbar, and build scoot")

    tools = {}
    for attr in ("foot", "dbus", "dejavu_fonts"):
        tools[attr], _ = nix_resolve(attr, REPO)
    font_file = os.path.join(nix_resolve("dejavu_fonts.minimal", REPO)[0],
                             "share", "fonts", "truetype", "DejaVuSans.ttf")
    stage_tools = {"foot": tools["foot"], "dbus": tools["dbus"],
                   "fonts": os.path.join(tools["dejavu_fonts"], "share", "fonts")}
    sway_bin = None
    bins = {"scoot": a.scoot}
    if "sway" in compositors:
        sway_store, _ = nix_resolve("sway", REPO)
        sway_bin = os.path.join(sway_store, "bin", "sway")
        bins.update(sway=sway_bin, swaymsg=os.path.join(sway_store, "bin", "swaymsg"))
        grim, _ = nix_resolve("grim", REPO)
        bins["grim"] = os.path.join(grim, "bin", "grim")

    roster = []
    for cls in B.ALL:
        if cls.name not in wanted:
            continue
        if cls is B.Scootbar:
            bar = cls(binary=a.scootbar, version=version_line([a.scootbar, "--version"]))
        else:
            store, version = nix_resolve(cls.nix_attr, REPO)
            bar = cls(store=store, version=version)
        roster.append(bar)

    meta = {
        "date": time.strftime("%Y-%m-%d %H:%M:%S %Z"),
        "machine": machine(),
        "commit": git("rev-parse", "HEAD"),
        "tree_dirty": bool(git("status", "--porcelain", "--untracked-files=no")),
        "harness": {n: sha256(os.path.join(HERE, n)) for n in sorted(os.listdir(HERE))
                    if n.endswith(".py")},
        "scope": a.scope,
        "compositors": compositors,
        "compositor_versions": {
            "scoot": version_line([a.scoot, "--version"]),
            **({"sway": version_line([sway_bin, "--version"],
                                     {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent"})}
               if sway_bin else {}),
        },
        "compositor_binaries": {k: {"path": v, "sha256": sha256(os.path.realpath(v))}
                                for k, v in bins.items()},
        "tools": {**tools, "font_file": font_file},
        "settings": {k: getattr(a, k) for k in ("rounds", "settle_secs", "idle_secs",
                                                "switches", "switch_hz")},
        "bars": {},
    }
    for bar in roster:
        entry = {"version": bar.version, "ran": {}}
        if bar.informational:
            entry["informational"] = True
        if bar.store:
            entry["store"] = bar.store
        entry["binaries"] = [{"path": p, "sha256": sha256(os.path.realpath(p))}
                             for p in bar.elf_binaries()]
        entry["static"] = {k: v for k, v in size.weigh(bar.elf_binaries(), strip).items()
                           if k != "files"}
        meta["bars"][bar.name] = entry
    if B.Scootbar in (type(b) for b in roster):
        meta["bars"]["scootbar"]["code"] = code_size(a.scootbar_source)
        meta["bars"]["scootbar"]["source"] = {
            "tree": a.scootbar_source,
            "commit": subprocess.run(["git", "-C", a.scootbar_source, "rev-parse", "HEAD"],
                                     capture_output=True, text=True).stdout.strip(),
        }

    runs_path = os.path.join(a.out, "runs.jsonl")
    meta_path = os.path.join(a.out, "meta.json")

    def save_meta():
        with open(meta_path, "w") as f:
            json.dump(meta, f, indent=1)

    def record(recs):
        with open(runs_path, "a") as f:
            for r in recs:
                f.write(json.dumps(r) + "\n")
                keys = ("latency_ms", "rss_kb", "pss_kb", "wakeups_per_min", "idle_cpu_ms",
                        "switch_cpu_ms")
                brief = {k: round(r[k], 2) for k in keys if k in r}
                status = "ok" if r.get("ok") else "FAILED " + (r.get("error") or "").splitlines()[0]
                print(f"[{time.strftime('%H:%M:%S')}] {r['compositor']:5} {r['bar']:8} "
                      f"{r['row']:7} r{r['round']} {status} {brief}", flush=True)

    save_meta()
    for compositor in compositors:
        for bar in roster:
            why = bar.cannot_show(a.scope, compositor)
            if why:
                # Not run, and no number invented: the report names it.
                meta["bars"][bar.name].setdefault("cannot_show", {})[compositor] = why
                meta["bars"][bar.name]["ran"][compositor] = False
                print(f"[{time.strftime('%H:%M:%S')}] {compositor:5} {bar.name:8} "
                      f"NOT RUN at scope {a.scope}: {why}", flush=True)
                save_meta()
                continue
            with Stage(compositor, bins, stage_tools, keep=a.keep) as stage:
                config_dir = stage.sess.path("config", bar.name)
                os.makedirs(config_dir, exist_ok=True)
                argv = bar.write_config(config_dir, font_file, a.scope, compositor)
                recs = measure.startup(stage, bar, argv, a.rounds)
                recs += measure.idle(stage, bar, argv, a.settle_secs, a.idle_secs, a.switches,
                                     a.switch_hz, magick=magick,
                                     parts=tuple(B.SCOPES[a.scope]))
                record(recs)
                meta["bars"][bar.name]["ran"][compositor] = any(r.get("ok") for r in recs)
                save_meta()
    meta["machine"]["loadavg_end"] = os.getloadavg()
    meta["machine"]["end"] = hw.state()
    save_meta()
    text = tables.render(a.out)
    with open(os.path.join(a.out, "table.md"), "w") as f:
        f.write(text + "\n")
    print(text)


def _terminate(signum, _frame):
    # Unwinds like Ctrl-C, so every stage's `finally` stops its compositor,
    # bus, windows and bar and removes the cgroup; later signals are
    # ignored so they cannot cut that cleanup short.
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(sig, signal.SIG_IGN)
    raise KeyboardInterrupt(f"signal {signum}")


def main():
    signal.signal(signal.SIGTERM, _terminate)
    signal.signal(signal.SIGHUP, _terminate)
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--compositors", default="scoot,sway")
    r.add_argument("--bars", default=",".join(c.name for c in B.ALL if not c.informational),
                   help="comma-separated; default is scootbar and the ratified competitors "
                        "(yambar, waybar); ironbar and ashell are informational and opt-in")
    r.add_argument("--scope", default="clock", help=f"one of {', '.join(B.SCOPES)}")
    r.add_argument("--rounds", type=int, default=5, help="startups timed per bar")
    r.add_argument("--settle-secs", type=float, default=30.0)
    r.add_argument("--idle-secs", type=float, default=300.0)
    r.add_argument("--switches", type=int, default=240)
    r.add_argument("--switch-hz", type=float, default=4.0)
    r.add_argument("--scootbar", default=os.path.join(target, "release", "scootbar"))
    r.add_argument("--scoot", default=os.path.join(target, "release", "scoot"))
    r.add_argument("--scootbar-source", default=REPO,
                   help="the source tree --scootbar was built from, for the code rows "
                        "(default: this one)")
    r.add_argument("--keep", action="store_true", help="keep each stage's scratch directory")
    rp = sub.add_parser("report")
    rp.add_argument("dir")
    c = sub.add_parser("compare")
    c.add_argument("dir")
    c.add_argument("baseline")
    a = p.parse_args()
    if a.cmd == "run":
        if a.rounds < 1 or a.idle_secs <= 0 or a.switch_hz <= 0 or a.switches < 0:
            p.error("--rounds, --idle-secs and --switch-hz must be positive, --switches not negative")
        cmd_run(a)
    elif a.cmd == "report":
        text, failed = tables.gate(a.dir)
        print(text)
        sys.exit(1 if failed else 0)
    else:
        text, regressions = tables.compare(a.dir, a.baseline)
        print(text)
        sys.exit(1 if regressions else 0)


if __name__ == "__main__":
    main()
