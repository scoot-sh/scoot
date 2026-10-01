#!/usr/bin/env python3
"""CPU per redraw of a text-heavy bar: the row the resource ratchet's
harness (`bench.py`) does not have, because its bars idle and its switching
row redraws two workspace pills.

    scripts/scootbar-bench/redraw.py --scoot S --scootbar B --font F.ttf [--sets 3000] [--rounds 5]

A headless scoot and a scootbar with the workspaces, a `push` module and the
clock. Over the control socket the bar is sent `--sets` `set` requests, each
a different 32-character text, one at a time (a reply is waited for), so each
is one turn of the bar's loop and one full redraw: the request parsed, the
text laid out and drawn through the glyph cache, the damage computed, the
buffer committed. The bar's CPU time (`/proc/PID/schedstat`, nanoseconds on
the CPU) over the sets, divided by their number, is printed per round with
the median; the first 200 sets only fill the glyph cache and the pools, and
a second of quiet before and half a second after each round keeps the
timer's wakeup out of it.

It measures one binary: run it for each build, alternating (A B A B), as
the numbers move by a few percent with the machine's frequency. A build
older than the `push` module (before M4) cannot be given the config.
"""

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

WARM = 200
ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789 WXYZ"
CONFIG = """left = ["workspaces"]
right = ["status", "clock"]

[push.status]
placeholder = "idle"
"""


def texts(n):
    """`n` different 32-character strings of letters, digits and spaces. The
    first four characters are `i` in base 41 (so no two are alike), the rest
    are shifted copies of them, so the texts do not read as a pattern the
    glyph cache could make cheap."""
    base = len(ALPHABET)
    assert n <= base**4, "more texts than the first four characters can tell apart"
    return [
        "".join(ALPHABET[(i // base ** (j % 4) % base + 3 * j) % base] for j in range(32))
        for i in range(n)
    ]


def request(text):
    body = {"protocol": 1, "type": "set", "id": "status", "value": {"text": text}}
    return (json.dumps(body) + "\n").encode()


def median(xs):
    return sorted(xs)[len(xs) // 2]


def wait_for(predicate, timeout):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    return None


def run(args):
    d = tempfile.mkdtemp(prefix="sbr", dir="/tmp")
    os.chmod(d, 0o700)
    for sub in ("h", "c", "ca", "s"):
        os.mkdir(os.path.join(d, sub))
    env = dict(os.environ, XDG_RUNTIME_DIR=d, HOME=f"{d}/h", XDG_CONFIG_HOME=f"{d}/c",
               XDG_CACHE_HOME=f"{d}/ca", XDG_STATE_HOME=f"{d}/s")
    for var in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "SWAYSOCK", "DISPLAY", "SCOOT_SOCKET", "WAYLAND_DEBUG"):
        env.pop(var, None)
    open(f"{d}/scoot.toml", "w").close()
    with open(f"{d}/bar.toml", "w") as f:
        f.write(CONFIG)
    scoot = subprocess.Popen(
        [args.scoot, "--headless", "--width", "1920", "--height", "1080", "--outputs", "1",
         "--socket", f"{d}/s.sock", "--config", f"{d}/scoot.toml"],
        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    bar = None
    try:
        display = wait_for(lambda: next((n for n in os.listdir(d) if n.startswith("wayland-")
                                         and not n.endswith(".lock") and os.path.exists(f"{d}/s.sock")), None), 10)
        if not display:
            raise RuntimeError("scoot did not come up")
        env["WAYLAND_DISPLAY"] = display
        bar = subprocess.Popen(
            [args.scootbar, "daemon", "--config", f"{d}/bar.toml", "--font", args.font, "--font-size", "14",
             "--height", "26", "--clock-format", "%a %d %b %H:%M"],
            env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        control = wait_for(lambda: next((f"{d}/{n}" for n in os.listdir(d)
                                         if "scootbar" in n and n.endswith(".sock")), None), 10)
        if not control:
            bar.poll()
            raise RuntimeError(f"the bar has no control socket: {bar.stderr.read(400)!r}")
        time.sleep(3)
        conn = socket.socket(socket.AF_UNIX)
        conn.connect(control)
        stream = conn.makefile("rwb", buffering=0)
        requests = [request(t) for t in texts(args.sets)]

        def send(message):
            stream.write(message)
            if not stream.readline():
                raise RuntimeError("the bar closed the control socket")

        def cpu_ns():
            with open(f"/proc/{bar.pid}/schedstat") as f:
                return int(f.read().split()[0])

        for message in requests[:WARM]:
            send(message)
        per_set = []
        for _ in range(args.rounds):
            time.sleep(1)
            before = cpu_ns()
            for message in requests:
                send(message)
            time.sleep(0.5)
            per_set.append((cpu_ns() - before) / args.sets / 1000.0)
        with open(f"/proc/{bar.pid}/status") as f:
            rss = next(int(line.split()[1]) for line in f if line.startswith("VmRSS"))
        return {"bar": args.scootbar, "sets": args.sets, "us_per_set": [round(x, 1) for x in per_set],
                "median_us": round(median(per_set), 1), "rss_kb": rss}
    finally:
        if bar is not None:
            bar.terminate()
            bar.wait()
        scoot.terminate()
        shutil.rmtree(d, ignore_errors=True)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--scoot", required=True, help="a headless-capable scoot")
    p.add_argument("--scootbar", required=True)
    p.add_argument("--font", required=True, help="a TTF file")
    p.add_argument("--sets", type=int, default=3000)
    p.add_argument("--rounds", type=int, default=5)
    args = p.parse_args()
    if args.sets <= WARM or args.rounds < 1:
        p.error(f"--sets must exceed {WARM} and --rounds be at least 1")
    print(json.dumps(run(args)))


if __name__ == "__main__":
    sys.exit(main())
