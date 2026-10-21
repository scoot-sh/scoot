#!/usr/bin/env awk
# stats.awk: median/p95/min/max/mean over one integer per line.
# Lines that are not non-negative integers (e.g. TIMEOUT markers the
# bench writes when a frame never arrived) are counted as timeouts and
# excluded from the distribution, never silently dropped: the first
# output field is the timeout count. Usage: stats.awk < column.txt
# Output: timeouts=<t> n=<n> min=<a> median=<m> p95=<p> max=<x> mean=<v>
# Portable awk only (no gawk asort): insertion sort, n is in the tens.
BEGIN { n = 0; t = 0; sum = 0 }
{
    if ($1 ~ /^[0-9]+$/) { vals[n++] = $1 + 0; sum += $1 }
    else { t++ }
}
END {
    if (n == 0) { printf("timeouts=%d n=0 min=- median=- p95=- max=- mean=-\n", t); exit 0 }
    for (i = 1; i < n; i++) {
        v = vals[i]; j = i - 1
        while (j >= 0 && vals[j] > v) { vals[j + 1] = vals[j]; j-- }
        vals[j + 1] = v
    }
    min = vals[0]; max = vals[n - 1]
    if (n % 2 == 1) { med = vals[(n - 1) / 2] } else { med = int((vals[n / 2 - 1] + vals[n / 2]) / 2) }
    rank = int(n * 95 / 100)
    if (rank >= n) { rank = n - 1 }
    printf("timeouts=%d n=%d min=%d median=%d p95=%d max=%d mean=%d\n", t, n, min, med, vals[rank], max, int(sum / n))
}
