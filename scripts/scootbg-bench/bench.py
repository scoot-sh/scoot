#!/usr/bin/env python3
"""scootbg against every other wallpaper daemon: the ticket-11 benchmark.

    scripts/scootbg-bench/bench.py run --out DIR [--compositor scoot|sway]
        [--rounds 5] [--only ROWS] [--daemons NAMES] [--idle-secs 60]
        [--window-secs 60]
    scripts/scootbg-bench/bench.py report DIR
    scripts/scootbg-bench/bench.py compare DIR BASELINE_DIR

Run it from the repository root inside the dev shell (``devenv shell --
python3 scripts/scootbg-bench/bench.py run ...``: it needs ``nix``,
``strip``, ``ldd`` and ImageMagick), as root or with rights to make a
cgroup (CPU is accounted per daemon run by one; see ``procs.py``), after
``cargo build --release -p scoot -p scootbg``. The competitors come from
the flake's pinned nixpkgs (built in the sandbox, normally fetched from the
cache); Mesa's software EGL from the same nixpkgs for the daemons that
render with OpenGL.

``run`` writes ``DIR/meta.json`` (machine, versions, store paths, the
image recipe, what each daemon supports, the static rows) and
``DIR/runs.jsonl`` (every raw run, one JSON object per line, appended as it
finishes), then ``DIR/table.md``. ``report`` re-renders the table and the
gate from them; ``compare`` checks scootbg now against an earlier
results directory by the same noise rule, for a change that must not
regress. The method is in ``docs/scootbg/README.md`` (the comparison
section) and in each module's docstring.
"""

import argparse
import hashlib
import json
import os
import platform
import re
import signal
import shutil
import struct
import subprocess
import sys
import time
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import daemons as D  # noqa: E402
import report  # noqa: E402
import scenarios  # noqa: E402
import size  # noqa: E402

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

# The 6000×4000 JPEG of tickets 6, 8 and 9: fractal plasma with Gaussian
# noise, so it has a photo's entropy. ImageMagick 7.1.2-29 (the dev shell's)
# makes it byte for byte: 8,851,735 B.
JPEG_RECIPE = [
    "-seed", "1", "-size", "6000x4000", "plasma:fractal", "-attenuate", "0.5",
    "+noise", "Gaussian", "-quality", "92", "-sampling-factor", "4:2:0",
]
JPEG_SHA256 = "301279ffb3d4c8e39aecac039e596eb4795829293e47670468247bc747f027a0"

ROWS = ("startup", "set", "restore", "idle")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def write_png(path, width, height, rgb):
    """A solid-color RGB PNG, in the standard library only."""
    row = b"\x00" + bytes(rgb) * width
    raw = row * height

    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw)))
        f.write(chunk(b"IEND", b""))


def check_image(path, expected=JPEG_SHA256):
    """Refuses a test JPEG that is not the recorded one: every published
    figure is for that file, so another would compare nothing."""
    got = sha256(path)
    if got != expected:
        raise ValueError(
            f"{path} has sha256 {got}, not the recorded {expected}: it was not made by "
            f"`magick {' '.join(JPEG_RECIPE)}` with ImageMagick 7.1.2-29 (another "
            "ImageMagick makes other noise). Remove it and run in the dev shell, whose "
            "ImageMagick is that one."
        )
    return got


def images(directory, magick):
    os.makedirs(directory, exist_ok=True)
    jpeg = os.path.join(directory, "big.jpg")
    if not os.path.exists(jpeg):
        if not magick:
            sys.exit("no ImageMagick (`magick`) to make the test JPEG: run in the dev shell")
        subprocess.run([magick, *JPEG_RECIPE, jpeg], check=True)
    try:
        check_image(jpeg)
    except ValueError as e:
        sys.exit(f"bench: {e}")
    first = os.path.join(directory, "first.png")
    if not os.path.exists(first):
        write_png(first, 64, 64, (200, 50, 50))
    return {"jpeg": jpeg, "first": first}


def machine():
    info = {"kernel": platform.release(), "cpus": os.cpu_count()}
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    info["cpu"] = line.split(":", 1)[1].strip()
                    break
        with open("/proc/meminfo") as f:
            info["memory_kb"] = int(f.readline().split()[1])
    except OSError:
        pass
    info["loadavg_start"] = os.getloadavg()
    return info


def run_text(argv):
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=30)
        return (r.stdout + r.stderr).strip().splitlines()[0] if (r.stdout + r.stderr).strip() else ""
    except (OSError, subprocess.TimeoutExpired):
        return ""


def version_of(argv, pattern, env=None):
    """The version line from ``argv --version``, found by ``pattern``, not
    just its first line. nixpkgs' sway wrapper starts `dbus-run-session`
    when no bus address is set, which may print first or fail outright, so
    sway is asked with an address set (``env``), as the session starts
    it."""
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=30,
                           env=dict(os.environ, **(env or {})))
    except (OSError, subprocess.TimeoutExpired) as e:
        return f"unknown ({e})"
    for line in (r.stdout + "\n" + r.stderr).splitlines():
        if re.search(pattern, line):
            return line.strip()
    return "unknown: " + (r.stdout + r.stderr).strip()[:200]


def provenance(path):
    """Where a binary really is, and what it is."""
    real = os.path.realpath(path)
    return {"path": path, "realpath": real, "sha256": sha256(real)}


HARNESS_DIR = os.path.dirname(os.path.abspath(__file__))


def harness_state():
    """The harness's own state: its commit, whether its directory differs
    from it (untracked files included), and every file's hash."""
    files = sorted(n for n in os.listdir(HARNESS_DIR) if n.endswith(".py"))
    return {
        "commit": git("rev-parse", "HEAD"),
        "status": git("status", "--porcelain", "--untracked-files=all", "--", HARNESS_DIR),
        "files": {n: sha256(os.path.join(HARNESS_DIR, n)) for n in files},
    }


def git(*args):
    return subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True).stdout.strip()


def cmd_run(a):
    os.makedirs(a.out, exist_ok=True)
    magick = shutil.which("magick")
    strip = shutil.which("strip")
    if not strip:
        sys.exit("no `strip` on PATH: run in the dev shell")
    bins = {"scoot": a.scoot, "scootbg": a.scootbg}
    if a.compositor == "sway":
        bins["sway"] = a.sway or os.environ.get("SCOOTBG_TEST_SWAY") or shutil.which("sway")
        if not bins["sway"]:
            sys.exit("no sway: pass --sway or set SCOOTBG_TEST_SWAY")
        grim, _ = D.nix_resolve("grim", REPO)
        bins["grim"] = os.path.join(grim, "bin", "grim")
    for key in ("scoot", "scootbg"):
        if not os.path.isfile(bins[key]):
            sys.exit(f"no {key} at {bins[key]}: cargo build --release -p scoot -p scootbg")

    mesa, mesa_version = D.nix_resolve("mesa", REPO)
    egl_env = {
        "__EGL_VENDOR_LIBRARY_DIRS": os.path.join(mesa, "share", "glvnd", "egl_vendor.d"),
        "LD_LIBRARY_PATH": os.path.join(mesa, "lib"),
    }

    wanted = set(a.daemons.split(",")) if a.daemons else None
    roster = []
    meta = {
        "date": time.strftime("%Y-%m-%d %H:%M:%S %Z"),
        "machine": machine(),
        "compositor": a.compositor,
        "compositor_version": version_of([bins["sway"], "--version"], r"^sway version ",
                     {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent/no-bus"})
        if a.compositor == "sway" else version_of([bins["scoot"], "--version"], r"^scoot "),
        "scoot_binary": provenance(bins["scoot"]),
        "compositor_binaries": {k: provenance(v) for k, v in bins.items() if k != "scootbg"},
        "harness": harness_state(),
        "commit": git("rev-parse", "HEAD"),
        "tree_dirty": bool(git("status", "--porcelain", "--untracked-files=no")),
        "mesa": {"store": mesa, "version": mesa_version},
        "rounds": a.rounds,
        "idle_secs": a.idle_secs,
        "window_secs": a.window_secs,
        "daemons": {},
        "supports": {},
    }
    for cls in D.ALL:
        if wanted and cls.name not in wanted:
            continue
        if issubclass(cls, D.Scootbg):
            d = cls(bins={"scootbg": bins["scootbg"]})
            entry = {"version": version_of([bins["scootbg"], "--version"], r"^scootbg "),
                     "binary": bins["scootbg"], "sha256": sha256(bins["scootbg"]),
                     "store": a.scootbg_store}
            if a.scootbg_store:
                entry["store_binary"] = provenance(os.path.join(a.scootbg_store, "bin", "scootbg"))
        else:
            store, version = D.nix_resolve(cls.nix_attr, REPO)
            d = cls(store=store)
            entry = {"version": version, "store": store, "nix_attr": cls.nix_attr}
        if not issubclass(cls, D.Scootbg):
            entry["binaries"] = [provenance(b) for b in d.elf_binaries()]
        entry["needs_egl"] = d.needs_egl
        entry["informational"] = d.informational
        meta["daemons"][d.name] = entry
        meta["supports"][d.name] = {
            f"{row}:{kind}": d.supports(row, kind)
            for row in ROWS for kind in ("image", "color")
        }
        roster.append(d)

    imgs = images(a.images, magick)
    meta["images"] = {
        "jpeg": {"path": imgs["jpeg"], "bytes": os.path.getsize(imgs["jpeg"]),
                 "sha256": sha256(imgs["jpeg"]), "recipe": "magick " + " ".join(JPEG_RECIPE),
                 "expected_sha256": JPEG_SHA256},
        "first": {"path": imgs["first"], "what": "64x64 solid #c83232 PNG, the wallpaper a live change replaces"},
    }
    ctx = scenarios.Context(a.compositor, bins, imgs, egl_env, magick=magick, keep=a.keep)
    runs_path = os.path.join(a.out, "runs.jsonl")
    meta_path = os.path.join(a.out, "meta.json")

    def save_meta():
        with open(meta_path, "w") as f:
            json.dump(meta, f, indent=1)

    def record(recs):
        with open(runs_path, "a") as f:
            for r in recs:
                f.write(json.dumps(r) + "\n")
                status = "ok" if r.get("ok") else "FAILED"
                brief = {k: r[k] for k in ("latency_ms", "cpu_ms", "peak_kb", "rss_kb", "pss_kb", "wakeups") if k in r}
                print(f"[{time.strftime('%H:%M:%S')}] {r['daemon']:9} {r['row']:12} {r['variant']:9} "
                      f"{r['geometry']:12} r{r['round']} {status} {brief}", flush=True)
                if not r.get("ok") and r.get("error"):
                    print("    " + r["error"].replace("\n", "\n    "), flush=True)

    def guarded(scenario, ctx, *args, **kwargs):
        """A scenario that fails outside its own handling (a compositor that
        will not start, a cgroup that cannot be made) is a failed row, not
        the end of the run."""
        try:
            return scenario(ctx, *args, **kwargs)
        except (SystemExit, KeyboardInterrupt):
            raise
        except Exception as e:  # noqa: BLE001 -- recorded, not swallowed
            target = args[0]
            names = [d.name for d in target] if isinstance(target, list) else [target.name]
            return [{
                "row": scenario.__name__, "variant": " ".join(str(x) for x in args[1:3]),
                "geometry": "", "daemon": n, "compositor": ctx.compositor, "round": -1,
                "time": time.strftime("%Y-%m-%dT%H:%M:%S"), "ok": False,
                "error": f"{type(e).__name__}: {e}",
            } for n in names]

    save_meta()
    only = set(a.only.split(",")) if a.only else set(ROWS)

    # 1. Does each daemon run here at all?
    ran = []
    for d in roster:
        recs = guarded(scenarios.check, ctx, d)
        record(recs)
        meta["daemons"][d.name]["ran"] = recs[0]["ok"]
        if recs[0].get("daemon_exe"):
            meta["daemons"][d.name]["exe"] = provenance(recs[0]["daemon_exe"])
        if recs[0].get("compositor_exe") and "compositor_exe" not in meta:
            meta["compositor_exe"] = provenance(recs[0]["compositor_exe"])
        if not recs[0]["ok"]:
            meta["daemons"][d.name]["did_not_run"] = recs[0].get("error", "")
        else:
            ran.append(d)
    save_meta()

    # 2. The timing rows, one daemon at a time, the order rotating per round.
    for rnd in range(1, a.rounds + 1):
        k = (rnd - 1) % max(len(ran), 1)
        order = ran[k:] + ran[:k]
        for d in order:
            if "startup" in only:
                record(guarded(scenarios.startup, ctx, d, "color", rnd))
                record(guarded(scenarios.startup, ctx, d, "image", rnd))
            if "set" in only:
                record(guarded(scenarios.live_set, ctx, d, rnd))
            if "restore" in only:
                record(guarded(scenarios.restore, ctx, d, "image", rnd))
                record(guarded(scenarios.restore, ctx, d, "color", rnd))

    # 3. The idle rows: a batch of every daemon at once per setup.
    if "idle" in only:
        for rnd in range(1, a.rounds + 1):
            for geometry in ((1920, 1080, 1), (3840, 2160, 2)):
                for kind in ("image", "color"):
                    record(guarded(scenarios.idle, ctx, ran, geometry, kind, rnd,
                                   idle_s=a.idle_secs, window_s=a.window_secs))

    # 4. The static rows.
    with open(runs_path) as f:
        all_runs = [json.loads(line) for line in f if line.strip()]
    for d in roster:
        if d.informational:
            continue
        entry = meta["daemons"][d.name]
        mapped = {lib for r in all_runs if r["daemon"] == d.name and r["row"] == "idle"
                  for lib in r.get("libraries", [])}
        static = size.weigh(d.elf_binaries(), strip, mapped)
        store = d.store or a.scootbg_store
        if store:
            static["package_bytes"] = size.installed_bytes(store)
            static.update(size.closure_bytes(store, d.elf_binaries()))
        else:  # no package given: the binary alone, and the closure of what it links
            static["package_bytes"] = os.path.getsize(bins["scootbg"])
            static["closure_bytes"] = static["size_bytes"]
            static["closure_paths"] = []
        # What it writes: the median per scenario (after a start-up, after
        # live changes), the largest of those counted.
        by_variant = {}
        for r in all_runs:
            if r["daemon"] == d.name and r["row"] == "disk-written":
                by_variant.setdefault(r["variant"], []).append(r)
        if by_variant:
            medians = []
            for variant, recs in by_variant.items():
                recs.sort(key=lambda r: r["bytes"])
                medians.append(recs[len(recs) // 2])
            worst = max(medians, key=lambda r: (r["bytes"], r["mesa_shader_cache_bytes"]))
            static["written_bytes"] = worst["bytes"]
            static["written_files"] = worst["files"]
            static["written_after"] = worst["variant"]
            static["mesa_shader_cache_bytes"] = max(r["mesa_shader_cache_bytes"] for r in medians)
        static["disk_bytes"] = static["closure_bytes"] + static.get("written_bytes", 0)
        entry["static"] = static
    meta["machine"]["loadavg_end"] = os.getloadavg()
    save_meta()
    text = report.render(a.out)
    with open(os.path.join(a.out, "table.md"), "w") as f:
        f.write(text + "\n")
    print(text)


def _terminate(signum, _frame):
    # Unwinds like Ctrl-C, so every session's and run's `finally` stops its
    # compositor and daemon and removes its cgroup. Later signals are
    # ignored: a second one (Ctrl-C twice, or a supervisor signalling the
    # whole group) would otherwise land inside that `finally` and skip the
    # rest of the cleanup.
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(sig, signal.SIG_IGN)
    raise KeyboardInterrupt(f"signal {signum}")


def main():
    signal.signal(signal.SIGTERM, _terminate)
    signal.signal(signal.SIGHUP, _terminate)
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--compositor", choices=("scoot", "sway"), default="scoot")
    r.add_argument("--rounds", type=int, default=5)
    r.add_argument("--only", help=f"comma-separated subset of {','.join(ROWS)}")
    r.add_argument("--daemons", help="comma-separated subset of " + ",".join(c.name for c in D.ALL))
    r.add_argument("--idle-secs", type=float, default=60.0)
    r.add_argument("--window-secs", type=float, default=60.0)
    r.add_argument("--scoot", default=os.path.join(REPO, "target", "release", "scoot"))
    r.add_argument("--scootbg", default=os.path.join(REPO, "target", "release", "scootbg"))
    r.add_argument("--scootbg-store", help="scootbg's nix package, for the installed size")
    r.add_argument("--sway")
    r.add_argument("--images", default="/tmp/sbb-images")
    r.add_argument("--keep", action="store_true", help="keep each session's scratch directory")
    rp = sub.add_parser("report")
    rp.add_argument("dir")
    c = sub.add_parser("compare")
    c.add_argument("dir")
    c.add_argument("baseline")
    a = p.parse_args()
    if a.cmd == "run":
        cmd_run(a)
    elif a.cmd == "report":
        print(report.render(a.dir))
    else:
        text, regressions = report.compare(a.dir, a.baseline)
        print(text)
        sys.exit(1 if regressions else 0)


if __name__ == "__main__":
    main()
