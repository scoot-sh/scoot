# scootbar M0 spikes (throwaway)

The code behind [the M0 record](../../../../docs/scootbar/backlog/resolved/dependencies-done.md),
kept so its numbers can be re-derived. **Not product code**: none of it is a
workspace member (each crate has its own empty `[workspace]`), CI does not
build it (the classify job ignores `docs/*`), and nothing links to it but the
record. The real clock and text path are written fresh in the M1 entries.

| Dir | What |
|---|---|
| `fonts/` | `sb-font-spike`: loads one font file (read or mmapped), fills a glyph cache with printable ASCII at 1x and 1.5x, draws one clock line. One engine per build: `--features fontdue`, `ab_glyph` or `swash` (none is the size base), plus `mmap`. |
| `clock/` | `sb-clock-spike`: the minute clock on an absolute `CLOCK_REALTIME` timerfd with cancel-on-set, and the hand-rolled TZif reader (`src/tzif.rs`). Other zone backends for comparison: `--features tzrs`, `jiff`, `chrono`, `libc`; `utc` is the size base, `bare` drops the test modes. `check-zones.py` compares a backend with `zdump` over every zone; `idle-clock.sh` counts wakeups over a window; `step-test.py` **sets the system clock** (a disposable machine only) to test clock steps and DST. |
| `bench/` | `bar-bench.py`: startup to first frame, idle wakeups, CPU and memory of one bar on a running compositor. |
| `results/` | The raw output behind every number in the record, the bar and compositor configs (`bar-configs/`, with `run-all.sh`, the driver), and screenshots (`shots/`). |

Build each with a private target directory, from its own directory, inside
the dev shell, e.g.
`CARGO_TARGET_DIR=/tmp/sb-font-ab_glyph devenv shell -- cargo build --release --features ab_glyph,mmap`.
The commands and inputs for every number are in the record.
