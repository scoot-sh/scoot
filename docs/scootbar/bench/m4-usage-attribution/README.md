# Where M4's idle memory went: the raw evidence (2026-10-01)

For [m4-usage-optimization](../../backlog/m4-usage-optimization.md) and the
[README's "M4 usage optimization"](../../README.md#m4-usage-optimization).
Everything here was read from a running scootbar on the Asahi M2 (Apple M2,
16 KiB pages, Linux 7.1.13), headless scoot (the pinned release binary,
sha256 `33aa667cef85...`), `--right clock` with the harness's look
(`tools/quick.sh`), 20 s after the first frame, with nothing else on the box.

| Name | What | Source tree | Binary sha256 |
|---|---|---|---|
| `base1`-`base3` | `main` before M4, release (`lto = "fat"`, stripped) | `98c4b7a32` | `411ab1bd59ed...` |
| `tip1`-`tip3` | the M4 stack as it is on `main`, same profile | `c9d2cd361` (`af296989f`'s `crates/`) | `28eaea4a4271...` |
| `cand1`-`cand3` | this PR: the stack plus the hot-text order file | the PR's tree | `860185c2d678...` |

Per run, the files of `/proc/PID/` as they were read (`smaps`, `rollup` =
`smaps_rollup`, `stat`, `status`, `maps`), `stat.early` (0.5 s after the
start), and `text.pagemap` (the executable's `r-xp` range of `pagemap`, 8 bytes
a page, bit 63 = present) with `text.range`. `summary.txt` is one line a run:

```text
$ tools/evidence.sh      # quick.sh three times for each binary, then the lines below
base1 Rss: 3616 kB Pss: 2200 kB ... text-Rss Rss: 960 kB minflt 167 early-minflt 154
```

- **`pagemap.txt`**: the residency of the `r-xp` mapping, a character per 16 KiB
  page, for `base1`, `tip1` and `cand1`.
- **`functions/`**: which functions the bar executes. `executed-main-callgrind.txt`
  and `executed-tip-callgrind.txt` are callgrind's (`valgrind --tool=callgrind
  --demangle=no`, the bar built with `--cfg rustix_use_libc`, the one backend
  valgrind can run: rustix's own `AT_SYSINFO_EHDR` check faults under it), the
  functions of the bar's own object, 231 for `main` and 232 for the tip;
  `executed-tip-qemu.txt` is `qemu-aarch64 -d in_asm`'s translation log on the
  shipped (raw-syscall) tip build, 235 functions; `only-in-tip.txt` the 11 names
  (crate hashes wildcarded) that the tip's list has and `main`'s has not. Their
  sizes: 244,200 B for `main`'s 231 and 256,904 B for the tip's 232.
- **`levers.txt`** and **`lever-runs/`**: the levers that were tried, one block
  each, with the Cargo or `RUSTFLAGS` change, the three runs' `r-xp` `Rss`,
  `Pss_File` and `Pss_Anon`, and the raw `smaps_rollup`.
- **`redraw.txt`**: `scripts/scootbar-bench/redraw.py` (3000 `set`s a round,
  five rounds), twice each in the order tip, `opt-level = "s"`, `opt-level = 2`,
  this PR's, tip, ...: one JSON line per run.
- **`tools/`**: `quick.sh` (the reading), `pagemap-sections.py` (residency by
  ELF section, given `readelf -SW`), `callgrind-functions.py` (the function set
  of a callgrind file).

What the files say, in short: the `r-xp` mapping is 1184 kB with 960 resident on
`main` and 1392 with 1152 at the tip; `Pss_Anon` is 448 to 464 kB in both;
the minor faults at the first frame are 154 and 157; the executed functions
are 231 and 232, 244 KB and 257 KB; and with the order file the `r-xp`
`Rss` is 640 kB.
