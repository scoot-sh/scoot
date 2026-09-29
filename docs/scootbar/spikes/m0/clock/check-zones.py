#!/usr/bin/env python3
"""Check the spike's TZif reader against zdump (tzcode's own localtime).

For every zone under ZONEINFO (skipping posix/ and right/), `zdump -v -c
LO,HI` lists each transition as two lines (the second before it and the
second of it) with the local offset, isdst and abbreviation tzcode computes.
The spike's `dump` mode is fed the same instants from ZONEINFO and from
SLIMDIR (the same data compiled `zic -b slim`, which leans on the footer),
and every disagreement is printed.

Usage: check-zones.py SPIKE ZONEINFO SLIMDIR ZDUMP [LO HI]
"""
import calendar, os, subprocess, sys, time

spike, zoneinfo, slim, zdump = sys.argv[1:5]
lo, hi = (sys.argv[5], sys.argv[6]) if len(sys.argv) > 6 else ("1800", "2200")

zones = []
for root, dirs, files in os.walk(zoneinfo):
    rel = os.path.relpath(root, zoneinfo)
    if rel.split(os.sep)[0] in ("posix", "right"):
        continue
    for f in files:
        p = os.path.join(root, f)
        with open(p, "rb") as fh:
            if fh.read(4) == b"TZif":
                zones.append(os.path.relpath(p, zoneinfo))
zones.sort()

checked = mismatches = footer_region = 0
for z in zones:
    out = subprocess.run([zdump, "-v", "-c", f"{lo},{hi}", z], capture_output=True, text=True,
                         env={"TZDIR": zoneinfo}).stdout
    want = []
    for line in out.splitlines():
        # "<zone>  Sun Mar  8 06:59:59 2026 UT = Sun Mar  8 01:59:59 2026 EST isdst=0 gmtoff=-18000"
        if " UT = " not in line:
            continue
        left, right = line.split(" UT = ", 1)
        ut = " ".join(left.split()[1:])
        try:
            t = calendar.timegm(time.strptime(ut, "%a %b %d %H:%M:%S %Y"))
        except ValueError:
            continue
        r = right.split()
        abbr = r[5]
        isdst = int(r[6].split("=")[1])
        gmtoff = int(r[7].split("=")[1])
        want.append((t, gmtoff, isdst, abbr))
    if not want:
        continue
    stdin = "".join(f"{w[0]}\n" for w in want)
    for tree in (zoneinfo, slim):
        path = os.path.join(tree, z)
        if not os.path.exists(path):
            print(f"MISSING {tree} {z}")
            continue
        got = subprocess.run([spike, "dump", path], input=stdin, capture_output=True, text=True)
        if got.returncode != 0:
            print(f"FAIL {tree} {z}: {got.stderr.strip()}")
            mismatches += 1
            continue
        for w, g in zip(want, got.stdout.splitlines()):
            t, off, dst, ab = g.split()
            checked += 1
            if (int(off), int(dst), ab) != (w[1], w[2], w[3]):
                mismatches += 1
                if mismatches <= 40:
                    print(f"MISMATCH {tree} {z} t={w[0]} want={w[1:]} got={(off, dst, ab)}")
print(f"zones={len(zones)} instants_checked={checked} mismatches={mismatches}")
