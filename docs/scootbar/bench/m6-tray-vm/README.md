# M6 tray and D-Bus client: dev VM evidence

The raw runs behind the
[tray's cost table](../../backlog/lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02)
and the PR that landed the [tray](../../backlog/tray.md) on the
[D-Bus client](../../backlog/resolved/dbus-client-done.md). Not a
`scripts/scootbar-bench` run (that harness needs its compositor lane, and
has no D-Bus item): a handful of shell scripts, written for the dev VM
(`ssh -p 2222 dev@localhost`, aarch64, 6 CPUs, rustc 1.97.1, shared with
other agents' builds: load average 1 to 5 while these ran), with that
machine's paths in them (`/tmp/tray-takeover-run`, a nix `python3` with
`jeepney`). Read them as the method, adapt the paths to rerun.

All against the code commit `6a2662cd7`, whose `crates/` tree is
`7bc1a04525c8` (`git rev-parse HEAD:crates`; the PR's later commits are docs
and this directory), except where a row says it was kept from the first
round (`9e67c9fbd`, tree `6960f3fd6564`: the Waybar run, the fuzz runs, and
the clippy and test matrices, rerun on the new tree where the log says so).
`main` for the "before" binary is `a60852c2e`; the "feature off" binary is
the branch built with the default features minus `tray`. Both were
shipped to the VM with `git archive`, built release (`lto = "fat"`,
stripped) with their own `CARGO_TARGET_DIR`.

| File | What |
|---|---|
| `scripts/item.py` | a StatusNotifierItem on `jeepney` (an independent D-Bus marshaller): owns an item name, registers with the watcher, answers `GetAll` with two pixmaps, logs `Activate`, `SecondaryActivate` and `Scroll`, changes its icon and emits `NewIcon` when a flag file appears; `flood` re-announces without pause |
| `scripts/measure.sh`, `scripts/runall.sh` | one headless `scoot`, a private `dbus-daemon`, one bar, N items; `/proc/PID` after 14 s and 60 s later. Samples go outside the runtime directory (the bar's inotify watch is on it while there is no bus) |
| `logs/binary-sizes.txt` | file size and `readelf` section sizes of the `main`, tray-off and tray-on builds (the file size is quantized to 64 KiB steps) |
| `logs/measure-final.log` | the ten rows: `main` and the branch with no module, the clock on `main`, with the feature off, with the tray built and not placed; the tray alone with no bus, a bus and no items, one item, eight; the tray with the clock and one item |
| `scripts/flood.sh`, `logs/flood-final.txt` | one item re-announcing as fast as it is read, 20 s: 434 reads, 0.14 CPU-seconds. The same item before the 50 ms floor existed (not kept as a file): 20,354 reads, 1.93 CPU-seconds (utime 74 + stime 119 ticks), 1,850 wakeups a second |
| `scripts/live.sh`, `logs/live-final.txt`, `screenshots/` | end to end with `scoot msg screenshot`: an item found by listing the bus and one registered, `busctl` (sd-bus) reading the bar's watcher object, every action (`Scroll(-1)` and `Scroll(1)` arrive with opposite signs), a changed icon, a crash. In the screenshots the icons are two 14 by 14 squares (196 pixels each) of `(200,40,60)` and `(200,130,60)`; after the change the second is `(140,130,120)`; after item 1 is `kill -9`'d only `(140,130,120)` remains |
| `scripts/twobar.sh`, `logs/twobar-final.txt` | two bars on one bus: one owns the watcher and one hosts; an item registered after both appears in both; the owner `kill -9`'d, the host takes the name and keeps the item; a bar restarted hosts against it |
| `scripts/waybar-measure.sh`, `logs/waybar-1item.txt` | Waybar 0.15.0 (nixpkgs), tray only, one item, the same harness |
| `logs/clippy-feature-matrix.txt` | `cargo clippy -p scootbar --all-targets -- -D warnings` for no features, each of the 14 alone (`popup` included), the tray with each of the other 13, the default, the default with `icon-image`, `tray,network,battery` and `tray,popup,volume,microphone`: 32 builds, all `OK`, at `6a2662cd7` |
| `logs/test-matrix.txt` | `cargo nextest run -p scootbar` (1046 tests, 2 skipped), `cargo test -p scootbar`, and `--bin scootbar` with no features, `tray`, `tray,clock`, `tray,network,volume`, `clock` and `tray,popup,volume,microphone`, with `SCOOTBAR_REQUIRE_DBUS_DAEMON=1`, at `6a2662cd7` |
| `logs/fuzz-ci-1M.txt`, `logs/fuzz-long-601s.txt` | `cargo fuzz run -s none dbus` at the CI budget (1,000,000 runs, 8 s on a warm corpus) and for 600 s (34,432,616 runs): no finding, at `6a2662cd7` |
| `scripts/noreply*.{py,sh}`, `logs/call-timeouts.txt` | a peer that owns a name and never replies, called with `dbus-send --reply-timeout`: a daemon configured with `reply_timeout` 5000 answered at 5.0 s ("timeout by message bus"); a stock `session.conf` daemon (400 s) and the VM's `dbus-broker` (130 s) never did, the client's own timeout fired (libdbus's wording) |
| `scripts/reload.sh`, `logs/reload-final.txt` | `scootbar msg reload` three times and a config move of the tray, with two items: the same items shown, `watcher` still `owner`, one process owning both watcher names, 9 fds throughout |
| `scripts/gen_modules_arms.py` (removed by `cli-help-matrix`) | how the 64 `modules!` arms of `src/cli.rs` were regenerated (a build with no clock, workspaces or window title: one per subset of the six optional modules of that day). A record, not a tool: it hardcoded those six, needed a python3 the dev shell lacks, and did not apply to `popup` (a feature, not a module). The matrix is gone — the `Modules:` line is built at run time from the registry — so there is nothing left to regenerate |
