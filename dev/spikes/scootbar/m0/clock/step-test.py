#!/usr/bin/env python3
"""Step the system clock under running spike clocks, then put it back.

WARNING: this sets CLOCK_REALTIME (needs CAP_SYS_TIME). Run it only on a
disposable machine with nothing else depending on wall time, never on a
workstation. The clock is restored from CLOCK_MONOTONIC at the end, and on
any exception, to within the few microseconds the two reads take.

  step-test.py SPIKE OUTDIR ZONE=PATH...

Each ZONE=PATH starts `SPIKE run PATH` with its output in OUTDIR/ZONE.log
(a '/' in ZONE becomes '_'). Then, with a pause after each so a boundary
passes: +1 h, -1 h (back to now), 10 s before the Lord Howe DST start
(2026-10-03T15:30Z, +10:30 -> +11), 10 s before the Sydney one
(2026-10-03T16:00Z), 10 s before the New York DST end (2026-11-01T06:00Z).
The last step is the restore.
"""
import calendar, os, subprocess, sys, time

spike, outdir = sys.argv[1], sys.argv[2]
zones = [z.split("=", 1) for z in sys.argv[3:]]
RT, MONO = time.CLOCK_REALTIME, time.CLOCK_MONOTONIC
offset = time.clock_gettime(RT) - time.clock_gettime(MONO)
os.makedirs(outdir, exist_ok=True)
steps = open(os.path.join(outdir, "steps.log"), "w")


def step(label, target):
    before = time.clock_gettime(MONO)
    time.clock_settime(RT, target)
    steps.write(f"{label} mono={before:.6f} set_to={target:.3f}\n")
    steps.flush()


def iso(s):
    return calendar.timegm(time.strptime(s, "%Y-%m-%dT%H:%M:%SZ"))


procs = []
try:
    for name, path in zones:
        log = open(os.path.join(outdir, name.replace("/", "_") + ".log"), "w")
        procs.append(subprocess.Popen([spike, "run", path], stdout=log, stderr=subprocess.STDOUT))
    time.sleep(3)
    now = time.clock_gettime(MONO) + offset
    step("forward_1h", now + 3600)
    time.sleep(3)
    step("back_to_now", time.clock_gettime(MONO) + offset)
    time.sleep(3)
    for label, t in (("lord_howe_dst_start", "2026-10-03T15:29:50Z"),
                     ("sydney_dst_start", "2026-10-03T15:59:50Z"),
                     ("new_york_dst_end", "2026-11-01T05:59:50Z")):
        step(label, iso(t))
        time.sleep(15)
finally:
    step("restore", time.clock_gettime(MONO) + offset)
    time.sleep(2)
    for p in procs:
        p.terminate()
        p.wait()
