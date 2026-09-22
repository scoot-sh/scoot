#!/usr/bin/env bash
# Asahi.md Test 4, end to end, as one command: run the alternating tier
# benchmark and print the analysis.
#
#   Ctrl+Alt+F2, log in, cd to this checkout
#   scripts/asahi-test4.sh
#
# **Stay on that VT until it prints the analysis.** A `--tty` compositor on an
# inactive VT is paused by logind -- no DRM master, nothing rendered, nothing
# flipped -- so leaving mid-run measures a compositor doing no work. Each
# round detects that and marks itself `paused` rather than reporting numbers.
#
# This wraps `tty-tier-bench.sh` (which does the measuring) so the parameters
# and the analysis live in one place instead of in a shell history. Everything
# here is bash + coreutils; the optional image comparison uses ImageMagick's
# `compare` if it happens to be around and prints the command to run later if
# not, so nothing on the benchmark VT needs installing.
#
# To re-read a finished run instead of measuring a new one:
#
#   ANALYSE_ONLY=1 OUT=/tmp/scoot-asahi-test4 scripts/asahi-test4.sh
#
# Overrides: ROUNDS, OUT, ANALYSE_ONLY, and anything tty-tier-bench.sh takes.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

ROUNDS=${ROUNDS:-2}
OUT=${OUT:-/tmp/scoot-asahi-test4}
ANALYSE_ONLY=${ANALYSE_ONLY:-0}
export ROUNDS OUT

# Never measure into a directory that already holds a run. This is the
# evidence, and a benchmark run rewrites `summary.tsv`, `environment.txt` and
# the per-round logs in place -- so pointing this script at a finished run's
# directory *destroys that run*, which is exactly what happened on
# 2026-09-21: `OUT=/tmp/scoot-tier-bench scripts/asahi-test4.sh`, intended as
# "re-read the old numbers", instead re-ran the benchmark, failed every round
# on a busy seat, and left four `came_up=no` rows where four rounds of real
# measurements had been. The PNGs and power samples survived only because a
# failed round returns before writing them.
#
# CLAUDE.md treats the implementer's recorded evidence as a cache entry keyed
# to a tree state. A tool that silently invalidates that cache when asked to
# read it is the wrong shape, so reading is now a mode and overwriting takes
# an explicit flag.
if [ -e "$OUT/summary.tsv" ] && [ "$ANALYSE_ONLY" != 1 ]; then
    echo "$OUT already holds a run ($OUT/summary.tsv exists)." >&2
    echo >&2
    echo "  to re-read it:        ANALYSE_ONLY=1 OUT=$OUT $0" >&2
    echo "  to measure afresh:    OUT=<a new directory> $0" >&2
    echo "  to overwrite it:      OVERWRITE=1 OUT=$OUT $0   (destroys the numbers in it)" >&2
    [ "${OVERWRITE:-0}" = 1 ] || exit 1
    echo "OVERWRITE=1 given; replacing the run in $OUT" >&2
fi

# Both tiers must come from one tree or the A/B says nothing, so this checks
# for the builds rather than making them: a `nix build` here would silently
# rebuild against whatever the working tree says *now*, which is not
# necessarily what the other tier was built from.
missing=
for p in result-scoot/bin/scoot result-scoot-gpu/bin/scoot result-scootctl/bin/scootctl; do
    [ -x "$p" ] || missing="$missing $p"
done
if [ -n "$missing" ]; then
    echo "missing:$missing" >&2
    echo >&2
    echo "build all three from the same tree first:" >&2
    echo "  nix build .#scoot     -o result-scoot" >&2
    echo "  nix build .#scoot-gpu -o result-scoot-gpu" >&2
    echo "  nix build .#scootctl  -o result-scootctl" >&2
    exit 1
fi

# Everything this script prints also lands in one file. The run happens on a
# VT the operator switches away from as soon as it finishes, and scrollback on
# a text console is not evidence -- so the report has to be readable
# afterwards, from anywhere, without having watched it.
mkdir -p "$OUT" || exit 1
# The report is suffixed in analyse-only mode so that re-reading a run cannot
# overwrite the report the run itself wrote.
REPORT="$OUT/test4-report.txt"
[ "$ANALYSE_ONLY" = 1 ] && REPORT="$OUT/test4-reanalysis.txt"
{
    echo "Asahi.md Test 4 -- $(date -Is)"
    echo "script: $0  rounds: $ROUNDS  out: $OUT"
    [ "$ANALYSE_ONLY" = 1 ] && echo "ANALYSE_ONLY: re-reading an existing run, measuring nothing"
} > "$REPORT"

if [ "$ANALYSE_ONLY" != 1 ]; then
    scripts/tty-tier-bench.sh 2>&1 | tee -a "$REPORT"
    rc=${PIPESTATUS[0]}
    [ "$rc" = 0 ] || exit "$rc"
fi

SUMMARY="$OUT/summary.tsv"
[ -s "$SUMMARY" ] || { echo "no summary at $SUMMARY" | tee -a "$REPORT" >&2; exit 1; }

# Refuse a summary whose shape this analysis was not written for, rather than
# reading the wrong columns. The format has already changed once -- the
# 2026-09-21 run 1 wrote 16 columns, before the two power columns existed --
# and every field index below would silently shift by one against such a file,
# turning `width_jiffies/width_events` into `width_events/width_ms`. That
# produced a plausible-looking and completely wrong relayout figure when a
# later analysis pass pooled the two runs by hand.
cols=$(head -1 "$SUMMARY" | awk -F'\t' '{print NF}')
if [ "$cols" != 18 ]; then
    {
        echo "refusing to analyse $SUMMARY: $cols columns, expected 18."
        echo "A 16-column file is the pre-2026-09-21 format (no power columns);"
        echo "its width_* fields sit one place to the left and would be misread."
        echo "The measurements themselves are fine -- read them by hand, or re-run."
    } | tee -a "$REPORT" >&2
    exit 1
fi

# One block, teed once at the end of it, so the report file and the console
# get the same bytes in the same order.
{
echo
echo "================ ANALYSIS ================"
echo

# Medians and spreads per tier, normalised per damage event. Medians, never a
# best-of: this project has read a 70% regression off a single pair that was
# pure noise, and the per-event normalisation matters because the two tiers
# get through different event counts in the same fixed window.
awk -F'\t' '
function med(s,   a,n,i,j,t) { n=split(s,a," ")
  for(i=1;i<=n;i++) for(j=i+1;j<=n;j++) if(a[j]+0<a[i]+0){t=a[i];a[i]=a[j];a[j]=t}
  return (n%2) ? a[(n+1)/2]+0 : (a[n/2]+a[n/2+1])/2 }
function lo(s,   a,n,i,v) { n=split(s,a," "); v=a[1]+0; for(i=2;i<=n;i++) if(a[i]+0<v) v=a[i]+0; return v }
function watts(s, cnt) { return cnt ? sprintf("%.2f", med(s)/1e6) : "n/a" }
# "too small to measure" rather than a fatal division: a median of 0 means the
# scene cost under one jiffy, which is a real (and good) outcome, not an error.
function ratio(a, b) { return (b > 0) ? sprintf("%.2fx", a/b) : "n/a (gpu median is 0 -- scene too cheap to measure)" }
function hi(s,   a,n,i,v) { n=split(s,a," "); v=a[1]+0; for(i=2;i<=n;i++) if(a[i]+0>v) v=a[i]+0; return v }
NR==1 { next }
$3=="died" { bad[$2] = bad[$2] " r" $1 "(compositor died mid-round)"; next }
$3!="yes" { bad[$2] = bad[$2] " r" $1 "(did not start)"; next }
$4=="yes" { bad[$2] = bad[$2] " r" $1 "(VT paused)"; next }
{ isecs = $8 }
# A scene with no events cannot be normalised per event, and dividing by it
# would abort the whole analysis (awk makes division by zero fatal) *after*
# the tables have printed -- leaving the operator, who has already switched
# VTs, a report that silently stops at the ratios. Reachable with a short
# MOVE_SECS/WIDTH_SECS.
$11+0 == 0 || $15+0 == 0 { bad[$2] = bad[$2] " r" $1 "(a scene recorded no events)"; next }
{
  t=$2; rounds[t]++
  scan[t]=$6; conn[t]=$5
  idle[t]=idle[t] $7 " "
  mv[t]=mv[t] ($10/$11) " "; mvev[t]=mvev[t] $11 " "
  wd[t]=wd[t] ($14/$15) " "; wdev[t]=wdev[t] $15 " "
  rss[t]=rss[t] $18 " "
  # A power column can be "-" when a scene was too short for the sampler to
  # fire. Counted, not coerced: `"-"+0` is 0 in awk, and printing a missing
  # measurement as 0.00 W would be a fabricated number -- the rehearsal did
  # exactly that before this guard, on a 1-second relayout scene.
  if ($9  ~ /^[0-9]+$/) { pidle[t]=pidle[t] $9  " "; nidle[t]++ }
  if ($13 ~ /^[0-9]+$/) { pmv[t]=pmv[t]     $13 " "; nmv[t]++ }
  if ($17 ~ /^[0-9]+$/) { pwd[t]=pwd[t]     $17 " "; nwd[t]++ }
}
END {
  printf "tier   n  connector  scanout  idle j/%ss%s  motion j/ev (min-max)      relayout j/ev (min-max)     RSS MB\n", isecs, (isecs<10 ? " " : "")
  for (t in rounds)
    printf "%-6s %-2d %-10s %-8s %-11s %-26s %-27s %.1f\n", t, rounds[t], conn[t], scan[t],
      med(idle[t]),
      sprintf("%.4f (%.4f-%.4f)", med(mv[t]), lo(mv[t]), hi(mv[t])),
      sprintf("%.4f (%.4f-%.4f)", med(wd[t]), lo(wd[t]), hi(wd[t])),
      med(rss[t])/1024
  print ""
  print "power, whole system, mean W (this is battery draw, not the GPU alone):"
  printf "%-6s %-9s %-9s %-9s\n", "tier", "idle", "motion", "relayout"
  for (t in rounds)
    printf "%-6s %-9s %-9s %-9s\n", t, watts(pidle[t], nidle[t]), watts(pmv[t], nmv[t]), watts(pwd[t], nwd[t])
  print ""
  if ("dumb" in rounds && "gpu" in rounds) {
    print "ratios, dumb / gpu (>1 means the GPU tier is cheaper):"
    # Every denominator is guarded. A tier whose median scene cost rounds to
    # 0 jiffies is not far-fetched -- a headless rehearsal recorded 1 jiffy
    # across 143 motion events -- and an unguarded ratio would make awk abort
    # here, taking the rest of the report with it.
    printf "  motion CPU     %s\n", ratio(med(mv["dumb"]), med(mv["gpu"]))
    printf "  relayout CPU   %s\n", ratio(med(wd["dumb"]), med(wd["gpu"]))
    if (med(rss["dumb"]) > 0)
      printf "  RSS            %+.1f MB on gpu (%+.0f%%)\n",
        (med(rss["gpu"])-med(rss["dumb"]))/1024, 100*(med(rss["gpu"])/med(rss["dumb"])-1)
    if (nmv["gpu"] && nmv["dumb"])
      printf "  power, motion  %+.2f W on gpu\n", (med(pmv["gpu"])-med(pmv["dumb"]))/1e6
    if (nwd["gpu"] && nwd["dumb"])
      printf "  power, relay.  %+.2f W on gpu\n", (med(pwd["gpu"])-med(pwd["dumb"]))/1e6
    print ""
    if (med(idle["dumb"])==0 && med(idle["gpu"])==0)
      print "  idle: both tiers used no measurable CPU -- neither wakes when nothing moves."
  }
  for (t in bad) print "EXCLUDED from the medians above -- " t ":" bad[t]
}' "$SUMMARY"

# The correctness comparison, on the pinned scene: two freshly mapped windows
# with the pointer parked, which both tiers reach identically. The number to
# expect if the ONLY difference is renderer rounding is one least-significant
# bit per pixel. Two yardsticks, because they differ by 3x and the wrong one
# invites a false investigation: ONE channel of four differing everywhere is
# w*h/255/4, about 4016 at 2560x1600; all three colour channels differing
# everywhere measures about 12044 (measured, ImageMagick 7.1.2 Q16-HDRI, by
# adding 1/255 to R,G,B and comparing). Much
# larger means the tiers really drew different things and wants investigating;
# `compare -metric AE` is an absolute-error sum, not a count of differing
# pixels, which this project has misread before.
echo "---- correctness: pinned-scene captures ----"
CMP=$(command -v compare 2>/dev/null)
shopt -s nullglob
pairs=0
for d in "$OUT"/r*-dumb-pinned.png; do
    r=${d##*/}; r=${r%%-dumb-pinned.png}
    g="$OUT/$r-gpu-pinned.png"
    [ -f "$g" ] || continue
    pairs=$((pairs + 1))
    if [ -n "$CMP" ]; then
        printf "%s  dumb vs gpu AE=" "$r"
        "$CMP" -metric AE "$d" "$g" null: 2>&1
        echo
    fi
done
if [ "$pairs" = 0 ]; then
    echo "no pinned pairs found in $OUT"
elif [ -z "$CMP" ]; then
    echo "$pairs pair(s) captured; ImageMagick is not installed here, so compare them later with:"
    echo "  nix shell nixpkgs#imagemagick -c compare -metric AE \\"
    echo "    $OUT/r1-dumb-pinned.png $OUT/r1-gpu-pinned.png null:"
    echo "yardsticks at 2560x1600 if only renderer rounding differed: ~4016 for one"
    echo "channel of four, ~12044 for all three colour channels. Far below either means"
    echo "the tiers drew the same frame; far above means go and look at the crop."
fi

echo
echo "raw evidence: $OUT (per-round logs, jiffy and microwatt samples, screenshots, summary.tsv)"
} 2>&1 | tee -a "$REPORT"

echo
echo "full report written to $REPORT"
