#!/usr/bin/env python3
"""Measure one status bar on a running compositor. Throwaway M0 harness;
method and numbers: docs/scootbar/backlog/resolved/dependencies-done.md.

  bar-bench.py [--runs N] [--settle S] [--window W] [--switch-cmd CMD0 CMD1
               --switch-secs S --switch-hz HZ] -- COMMAND...

The environment (WAYLAND_DISPLAY, XDG_CONFIG_HOME, fonts, the bus) is the
caller's. Phases:

  A. Startup to first frame, N runs: the bar starts under WAYLAND_DEBUG=1,
     and each stderr line is stamped on arrival. First frame is the first
     `commit` of a surface that was given a non-nil `attach` after a
     layer-surface `ack_configure`. Time is from just before exec.
  B. Idle: a fresh start without debug output, a fixed settle, then a
     window. Wakeups are voluntary context switches, summed over every
     thread of the bar and its descendants (/proc/PID/task/*/status); CPU is
     summed on-CPU time (/proc/PID/task/*/schedstat, ns) plus utime+stime
     ticks (/proc/PID/stat). Memory at the window's end: VmRSS, Pss
     (smaps_rollup), RssAnon, and VmHWM (peak) for the whole run.
  C. Optional: the same counters while CMD0/CMD1 alternate (a workspace
     switch each) at HZ for S seconds.
"""
import argparse, os, re, signal, subprocess, sys, time

ap = argparse.ArgumentParser()
ap.add_argument("--runs", type=int, default=5)
ap.add_argument("--settle", type=float, default=30)
ap.add_argument("--window", type=float, default=300)
ap.add_argument("--switch-cmd", nargs=2)
ap.add_argument("--switch-secs", type=float, default=60)
ap.add_argument("--switch-hz", type=float, default=4)
ap.add_argument("cmd", nargs=argparse.REMAINDER)
a = ap.parse_args()
cmd = a.cmd[1:] if a.cmd and a.cmd[0] == "--" else a.cmd


def tree(root):
    kids = {}
    for p in os.listdir("/proc"):
        if not p.isdigit():
            continue
        try:
            st = open(f"/proc/{p}/stat").read()
        except OSError:
            continue
        ppid = int(st.rsplit(")", 1)[1].split()[1])
        kids.setdefault(ppid, []).append(int(p))
    out, todo = [], [root]
    while todo:
        p = todo.pop()
        out.append(p)
        todo.extend(kids.get(p, []))
    return out


def field(text, key):
    for line in text.splitlines():
        if line.startswith(key):
            return int(line.split()[1])
    return 0


def sample(root):
    s = dict(vol=0, nonvol=0, run_ns=0, ticks=0, rss=0, pss=0, anon=0, hwm=0, threads=0, procs=0, names=[])
    for p in tree(root):
        try:
            status = open(f"/proc/{p}/status").read()
            stat = open(f"/proc/{p}/stat").read()
            rollup = open(f"/proc/{p}/smaps_rollup").read()
            tasks = os.listdir(f"/proc/{p}/task")
        except OSError:
            continue
        s["procs"] += 1
        s["names"].append(stat.split("(", 1)[1].rsplit(")", 1)[0])
        f = stat.rsplit(")", 1)[1].split()
        s["ticks"] += int(f[11]) + int(f[12])
        s["rss"] += field(status, "VmRSS:")
        s["anon"] += field(status, "RssAnon:")
        s["hwm"] += field(status, "VmHWM:")
        s["pss"] += field(rollup, "Pss:")
        for t in tasks:
            try:
                ts = open(f"/proc/{p}/task/{t}/status").read()
                s["run_ns"] += int(open(f"/proc/{p}/task/{t}/schedstat").read().split()[0])
            except OSError:
                continue
            s["threads"] += 1
            s["vol"] += field(ts, "voluntary_ctxt_switches:")
            s["nonvol"] += field(ts, "nonvoluntary_ctxt_switches:")
    return s


def stop(pr):
    pr.send_signal(signal.SIGTERM)
    try:
        pr.wait(5)
    except subprocess.TimeoutExpired:
        pr.kill()
        pr.wait()


# A. startup to first frame
attach = re.compile(r"-> wl_surface[@#](\d+)\.attach\((?!nil)")
commit = re.compile(r"-> wl_surface[@#](\d+)\.commit\(")
acked = re.compile(r"-> zwlr_layer_surface_v1[@#]\d+\.ack_configure\(")
starts = []
for r in range(a.runs):
    env = dict(os.environ, WAYLAND_DEBUG="1")
    t0 = time.monotonic()
    pr = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, errors="replace")
    got, seen_ack, pending = None, False, set()
    deadline = t0 + 30
    for line in pr.stderr:
        if acked.search(line):
            seen_ack = True
        m = attach.search(line)
        if m and seen_ack:
            pending.add(m.group(1))
        m = commit.search(line)
        if m and m.group(1) in pending:
            got = (time.monotonic() - t0) * 1000
            break
        if time.monotonic() > deadline:
            break
    stop(pr)
    starts.append(got)
    time.sleep(1)
print(f"startup_ms={[round(x, 1) if x is not None else None for x in starts]}")

# B. idle
pr = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=open("/tmp/sb-bench/last-bar.log", "w"))
time.sleep(a.settle)
s0, t0 = sample(pr.pid), time.monotonic()
time.sleep(a.window)
s1, t1 = sample(pr.pid), time.monotonic()
w = t1 - t0
print(f"procs={s1['procs']} names={s1['names']} threads={s1['threads']}")
print(
    f"idle window_s={w:.1f} vol={s1['vol'] - s0['vol']} nonvol={s1['nonvol'] - s0['nonvol']} "
    f"wakeups_per_min={(s1['vol'] - s0['vol']) / w * 60:.2f} cpu_ms={(s1['run_ns'] - s0['run_ns']) / 1e6:.2f} "
    f"ticks={s1['ticks'] - s0['ticks']}"
)
print(f"mem rss_kb={s1['rss']} pss_kb={s1['pss']} anon_kb={s1['anon']} hwm_kb={s1['hwm']}")

# C. workspace switching
if a.switch_cmd:
    n = int(a.switch_secs * a.switch_hz)
    s0, t0 = sample(pr.pid), time.monotonic()
    for i in range(n):
        subprocess.run(a.switch_cmd[i % 2], shell=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        nxt = t0 + (i + 1) / a.switch_hz
        time.sleep(max(0, nxt - time.monotonic()))
    s1, t1 = sample(pr.pid), time.monotonic()
    print(
        f"switch n={n} window_s={t1 - t0:.1f} vol={s1['vol'] - s0['vol']} cpu_ms={(s1['run_ns'] - s0['run_ns']) / 1e6:.2f} "
        f"ticks={s1['ticks'] - s0['ticks']} rss_kb={s1['rss']} hwm_kb={s1['hwm']}"
    )
stop(pr)
