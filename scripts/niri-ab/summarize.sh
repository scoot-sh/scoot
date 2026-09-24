#!/usr/bin/env bash
# Reduces one scripts/niri-ab-bench.sh run to markdown tables: the median of
# the per-round values, with the range across rounds in brackets. Raw rows
# stay in the run directory; this only reads them.
#
#   scripts/niri-ab/summarize.sh /path/to/run-main [/path/to/run-diag]
#
# A DIAG run (second argument) contributes only what it exists for: frames
# per scene and time to first frame. Its CPU numbers are not results (the
# host's protocol log perturbs pacing) and are not read.
set -euo pipefail
MAIN=${1:?usage: summarize.sh RUN_DIR [DIAG_RUN_DIR]}
DIAG=${2:-}
command -v gawk >/dev/null || { echo "summarize.sh needs gawk (asort)" >&2; exit 1; }

# med(a, n): median of a[1..n] after sorting; rng: "lo-hi".
AWKLIB='
function med(a, n,   b, i) { n = asort(a, b); if (n == 0) return "-";
    return (n % 2) ? b[(n + 1) / 2] : (b[n / 2] + b[n / 2 + 1]) / 2 }
function lo(a,   b, n) { n = asort(a, b); return n ? b[1] : "-" }
function hi(a,   b, n) { n = asort(a, b); return n ? b[n] : "-" }
function cell(a, fmt) { return sprintf(fmt " [" fmt "-" fmt "]", med(a), lo(a), hi(a)) }
'
ORDER="scoot-pixman scoot-gles niri-off niri-on"

echo "### CPU per scene (compositor process, all threads)"
echo
echo "Median of rounds [min-max]. \`cpu ms\` sums the threads alive at both ends of the"
echo "scene; \`process ms\` is the process total from /proc/PID/stat (10 ms ticks),"
echo "which also counts threads that exited during the scene. Where they differ,"
echo "trust \`process ms\`. \`cpu%\` and \`us per event\` use \`cpu ms\`."
echo
gawk -F'\t' -v order="$ORDER" "$AWKLIB"'
NR == 1 { next }
{
    k = $2 SUBSEP $3; nk[k]++; i = nk[k]
    ms[k][i] = $6 / 1e6
    pm[k][i] = $7 * 10
    pct[k][i] = 100 * $6 / 1e9 / $4
    wps[k][i] = $8 / $4
    if ($5 > 0) per[k][i] = $6 / 1e3 / $5
    scenes[$3] = 1
}
END {
    split("idle pointer relayout shot-ipc shot-grim animate", sc, " ")
    nv = split(order, vs, " ")
    print "| scene | variant | cpu ms | process ms | cpu% | wakeups/s | us per event |"
    print "|---|---|---|---|---|---|---|"
    for (s = 1; s <= 6; s++) for (v = 1; v <= nv; v++) {
        k = vs[v] SUBSEP sc[s]; if (!(k in nk)) continue
        pe = (sc[s] == "idle" || sc[s] == "animate") ? "-" : cell(per[k], "%.0f")
        printf "| %s | %s | %s | %s | %s | %s | %s |\n", sc[s], vs[v],
            cell(ms[k], "%.1f"), cell(pm[k], "%d"), cell(pct[k], "%.1f"), cell(wps[k], "%.1f"), pe
    }
}' "$MAIN/results.tsv"

echo
echo "### Memory (kB, smaps_rollup)"
echo
gawk -F'\t' -v order="$ORDER" "$AWKLIB"'
NR == 1 { next }
{ k = $2 SUBSEP $3; nk[k]++; i = nk[k]; rss[k][i] = $4; pss[k][i] = $5; an[k][i] = $6; fi_[k][i] = $7; th[k][i] = $8 }
END {
    split("empty 3foot shots end", pt, " "); nv = split(order, vs, " ")
    print "| point | variant | Rss | Pss | Pss_Anon | Pss_File | threads |"
    print "|---|---|---|---|---|---|---|"
    for (p = 1; p <= 4; p++) for (v = 1; v <= nv; v++) {
        k = vs[v] SUBSEP pt[p]; if (!(k in nk)) continue
        printf "| %s | %s | %s | %s | %s | %s | %s |\n", pt[p], vs[v], cell(rss[k], "%d"),
            cell(pss[k], "%d"), cell(an[k], "%d"), cell(fi_[k], "%d"), med(th[k])
    }
}' "$MAIN/memory.tsv"

echo
echo "### Startup (ms from exec)"
echo
gawk -F'\t' -v order="$ORDER" -v diag="$DIAG" "$AWKLIB"'
FNR == 1 { next }
FILENAME == ARGV[1] { i = ++n1[$2]; ipc[$2][i] = $3; next }
$4 != "-" { i = ++n2[$2]; ff[$2][i] = $4 }
END {
    nv = split(order, vs, " ")
    print "| variant | IPC answering | first frame on the host (DIAG pass) |"
    print "|---|---|---|"
    for (v = 1; v <= nv; v++) printf "| %s | %s | %s |\n", vs[v], cell(ipc[vs[v]], "%.1f"),
        (vs[v] in n2) ? cell(ff[vs[v]], "%.1f") : "-"
}' "$MAIN/startup.tsv" ${DIAG:+"$DIAG/startup.tsv"}

echo
echo "### Screenshot latency (ms wall, every capture of every round)"
echo
gawk -F'\t' -v order="$ORDER" "$AWKLIB"'
NR == 1 { next }
{ k = $2 SUBSEP $3; i = ++nk[k]; w[k][i] = $5; b[k][i] = $6 }
END {
    nv = split(order, vs, " ")
    print "| variant | method | n | wall ms | PNG bytes |"
    print "|---|---|---|---|---|"
    for (v = 1; v <= nv; v++) for (m = 1; m <= 2; m++) {
        k = vs[v] SUBSEP (m == 1 ? "ipc" : "grim"); if (!(k in nk)) continue
        printf "| %s | %s | %d | %s | %s |\n", vs[v], (m == 1 ? "own IPC" : "grim"), nk[k],
            cell(w[k], "%.1f"), med(b[k])
    }
}' "$MAIN/shots.tsv"

if [ -n "$DIAG" ]; then
    echo
    echo "### Frames the nested compositor presented to the host (DIAG pass)"
    echo
    gawk -F'\t' -v order="$ORDER" "$AWKLIB"'
    NR == 1 { next }
    $12 != "-" { k = $2 SUBSEP $3; i = ++nk[k]; f[k][i] = $12; fps[k][i] = $12 / $4
        if ($5 > 0) fpe[k][i] = $12 / $5 }
    END {
        split("idle pointer relayout shot-ipc shot-grim animate", sc, " "); nv = split(order, vs, " ")
        print "| scene | variant | frames | frames/s | frames per event |"
        print "|---|---|---|---|---|"
        for (s = 1; s <= 6; s++) for (v = 1; v <= nv; v++) {
            k = vs[v] SUBSEP sc[s]; if (!(k in nk)) continue
            pe = (sc[s] == "idle" || sc[s] == "animate") ? "-" : cell(fpe[k], "%.2f")
            printf "| %s | %s | %s | %s | %s |\n", sc[s], vs[v], cell(f[k], "%d"), cell(fps[k], "%.1f"), pe
        }
    }' "$DIAG/results.tsv"
fi

if [ -n "$DIAG" ]; then
    echo
    echo "### CPU per presented frame (main-pass CPU median / DIAG-pass frame median)"
    echo
    echo "Two passes, so a ratio of medians rather than a per-round figure."
    echo
    gawk -F'\t' -v order="$ORDER" "$AWKLIB"'
    FNR == 1 { next }
    FILENAME == ARGV[1] { k = $2 SUBSEP $3; i = ++nc[k]; c[k][i] = $6 / 1e6; next }
    $12 != "-" { k = $2 SUBSEP $3; i = ++nf[k]; f[k][i] = $12 }
    END {
        split("pointer relayout animate", sc, " "); nv = split(order, vs, " ")
        print "| scene | variant | cpu ms | frames | ms per frame |"
        print "|---|---|---|---|---|"
        for (s = 1; s <= 3; s++) for (v = 1; v <= nv; v++) {
            k = vs[v] SUBSEP sc[s]; if (!(k in nc) || !(k in nf)) continue
            cm = med(c[k]); fm = med(f[k])
            printf "| %s | %s | %.1f | %d | %s |\n", sc[s], vs[v], cm, fm, (fm > 0 ? sprintf("%.2f", cm / fm) : "no frames")
        }
    }' "$MAIN/results.tsv" "$DIAG/results.tsv"
fi
