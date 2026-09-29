#!/usr/bin/env bash
# Regenerates the zone fixtures the TZif reader's tests check against:
# for each zone, the fat file as tzdata ships it, the same zone compiled
# `zic -b slim` (which leaves every transition past the present rules to
# the POSIX footer), and what `zdump` (tzcode's own localtime) says at the
# second before and the second of every transition from 1900 to 2100, as
# `UNIX_TIME OFFSET ISDST ABBREVIATION` lines. The tests then need no
# tzdata on the machine (M0's check-zones.py, made a test; the record's
# §3b ran it over every zone).
#
# Usage, from the repository root, with tzdata from the pinned nixpkgs:
#   crates/scootbar/src/modules/clock/fixtures/generate.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tzdata=$(nix build --inputs-from . 'nixpkgs#tzdata.out' --no-link --print-out-paths)
bin=$(nix build --inputs-from . 'nixpkgs#tzdata.bin' --no-link --print-out-paths)
zoneinfo=$tzdata/share/zoneinfo
slim=$(mktemp -d)
trap 'rm -rf "$slim"' EXIT
"$bin/bin/zic" -b slim -d "$slim" "$zoneinfo/tzdata.zi"
zones=(
  America/New_York    # northern DST, the common case
  Australia/Sydney    # southern DST: summer spans the new year
  Australia/Lord_Howe # a 30-minute DST shift
  Europe/Dublin       # negative DST in the source data (winter is "DST")
  Europe/London
  Asia/Kolkata        # +05:30, no DST
  America/St_Johns    # -03:30 with DST
  Pacific/Chatham     # +12:45 with DST
  Africa/Casablanca   # many irregular transitions (Ramadan)
  America/Sao_Paulo   # DST abolished: the footer has no rule
  Asia/Tehran         # DST abolished in 2022
  Pacific/Apia        # skipped a whole day (2011)
)
cd "$here"
rm -f -- *.tzif *.zdump
for zone in "${zones[@]}"; do
  name=${zone//\//_}
  cp "$zoneinfo/$zone" "$name.fat.tzif"
  cp "$slim/$zone" "$name.slim.tzif"
  chmod 644 "$name.fat.tzif" "$name.slim.tzif"
  TZDIR=$zoneinfo "$bin/bin/zdump" -v -c 1900,2100 "$zone" |
    python3 -c '
import calendar, sys, time
for line in sys.stdin:
    if " UT = " not in line:
        continue
    left, right = line.split(" UT = ", 1)
    try:
        t = calendar.timegm(time.strptime(" ".join(left.split()[1:]), "%a %b %d %H:%M:%S %Y"))
    except ValueError:
        continue
    r = right.split()
    print(t, r[7].split("=")[1], r[6].split("=")[1], r[5])
' >"$name.zdump"
done
wc -c -- *.tzif *.zdump | tail -1
