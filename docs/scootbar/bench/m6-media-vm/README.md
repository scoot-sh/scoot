# M6 media module: dev VM evidence

The raw runs behind the
[media module's cost table](../../backlog/lightest.md#m6-media-module-level-cost-measured-2026-10-03)
and the PR that landed the [module](../../backlog/resolved/media-module-done.md)
on the [D-Bus client](../../backlog/resolved/dbus-client-done.md). Not a
`scripts/scootbar-bench` run (that harness needs its compositor lane and has
no D-Bus player): a handful of shell scripts and one Python player, written
for the dev VM (`ssh -p 2222 dev@localhost`, aarch64, 6 CPUs, rustc 1.97.1,
shared with another agent's builds and tests: load average 3 to 7 while
these ran), with that machine's paths in them (a nix `python3` with
`jeepney`, the nixpkgs mpv built with its MPRIS script, `playerctl`). Read
them as the method, adapt the paths to rerun.

All against the code commit `4d2207598`, whose `crates/` tree is
`c5a935bdbccedcf7eed359182362b998bbb38aa1` (`git rev-parse HEAD:crates`): the
review round's code, rebased onto `main` with the tooltips PR. Later commits
change tests (`flake.txt`, `fuzz.txt` and `final-extra.txt` are at the last);
the first round's code commit was `ee46b8a47` (tree `958dcf867f66`), and
`logs/mutation.txt` and `logs/live-*.txt` are from it (the code they exercise,
the match rules and the end-to-end flow, is unchanged); the tree is what the
evidence is keyed by. `main` for the "before" binary is `c2fa2df02` (the
tooltips PR); the "feature off" binary is the branch built with the default
features minus `media`. The three were built release (`lto = "fat"`,
stripped), each source `tar`red to the VM, with their own `CARGO_TARGET_DIR`.

| File | What |
|---|---|
| `scripts/player.py` | an MPRIS player on `jeepney` (an independent D-Bus marshaller): owns `org.mpris.MediaPlayer2.NAME`, answers `GetAll` and `Get` of the Player interface, handles `PlayPause`, `Next` and `Previous` (and signals the change), and does what flag files ask: change track, take the long title, die without releasing anything, flood `Position` or titles for 20 s |
| `scripts/measure.sh`, `scripts/runall.sh` | one headless `scoot`, a private `dbus-daemon` (none for `nobus`), one bar, and the players a row names (stubs, or a real mpv with its MPRIS script playing a ten-minute WAV, or mpv's synthetic `lavfi` sine); `/proc/PID` after 14 s and 60 s later. Samples go outside the runtime directory (the bar's inotify watch is on it while there is no bus) |
| `logs/binary-sizes.txt` | file size and `readelf` section sizes of the `main`, media-off and media-on builds (the file size is quantized to 64 KiB steps), and `ldd` of each: libc, libm and libgcc_s only |
| `logs/measure-final.log`, `logs/measure-rerun-row2.txt` | the fourteen rows of the cost table; the second row lost pages to the VM's memory pressure between t0 and t60 (RSS 3048 kB), so it was rerun twice on the same binary (the table uses the reruns' RSS; PSS read 2323 and 1464 kB, which moves with what else maps the binary) |
| `scripts/flood.sh`, `logs/flood-final.txt` | one playing stub signalling `Position` (or a new title, or flipping the status) as fast as it can for 20 s, and a quiet baseline: signals sent, bar wakeups, CPU ticks, RSS |
| `scripts/live.sh`, `logs/live-private.txt`, `logs/live-broker.txt`, `screenshots/` | end to end with `scoot msg screenshot`: a player found by listing the bus, a second one appearing, `playerctl` (an independent client) reading the state the bar's controls changed, the skip refusal, the most recently playing player shown, a long title cut with an ellipsis, a player killed with `kill -9`, the empty module and its refusals, then two real mpv instances. Once on a private `dbus-daemon` and once on the VM's own session bus, which is `dbus-broker` 37 (read-only apart from the names the script's players own for the run: none left after) |
| `scripts/gen_modules_arms.py` (removed by `cli-help-matrix`) | how the 128 `modules!` arms of `src/cli.rs` were regenerated (a build with no clock, workspaces or window title: one per subset of the seven optional modules of that day). A record, not a tool: it hardcoded those seven, needed a python3 the dev shell lacks, and did not apply to `popup` (a feature, not a module). The matrix is gone — the `Modules:` line is built at run time from the registry — so there is nothing left to regenerate. `cli::tests::the_modules_line_is_the_registry_in_order` checks every build's line against the registry |
| `logs/ci.txt` | what CI's `scootbar` job runs, on the VM, for the shipped tree: `cargo nextest run -p scootbar` and `cargo test -p scootbar --no-fail-fast` with `SCOOTBAR_REQUIRE_DBUS_DAEMON=1`, `SCOOTBAR_REQUIRE_SCOOT=1` and `SCOOTBAR_REQUIRE_SWAY=1` (`SCOOTBAR_TEST_SCOOT` the VM's `scoot` of 1 October, see below), `--bin` unit tests with every feature, none, and each alone, and `cargo clippy -p scootbar --all-targets -- -D warnings` for the default, none, each of the 15 features alone, `media` with each of the other 14, `popup` with each of the other 14, `icon-image` with the default, and two mixed sets: 67 runs, 65 `OK` and the two below |
| `logs/flake.txt` | the media, link and mpris tests and the two help tests, 100 runs under `cargo nextest` (a process each) and 100 under `cargo test` (one process, concurrent), with the VM loaded by other work |
| `logs/fuzz.txt` | `cargo fuzz run -s none dbus` at CI's budget (1,000,000 runs, seed 1) over the corpus (which now holds two messages marshalled by sd-bus) and a 300 s run |
| `logs/mutation.txt` | the zero-wakeup claim is not vacuous: with `arg0namespace` removed from the owner-changes rule, `unrelated_bus_traffic_wakes_nothing` and `the_bus_is_asked_for_the_mpris_namespace_and_the_player_interface_only` fail |

## What ran against what

- **The two `FAIL`s in `logs/ci.txt`** (nextest and `cargo test`) are the
  same five tests: `popup::a_bar_with_no_popup_binding_binds_nothing_for_popups_until_invoke_asks`
  and four of `tooltip::`. They fail the same way on `main` with the same
  binary (`logs/popup-on-main.txt`): the VM's only `scoot` is the build of 1
  October (`/var/cargo-target/debug/scoot`), older than what those tests
  expect. CI builds `scoot` from the tree under test, so these are not CI
  results. (`agent::layout_rectangles_...`, another known failure of that
  binary, passed here.)
- **Not run here**: the Asahi M2 and real hardware, the full
  `scripts/scootbar-bench`, a real Spotify, Firefox or Chromium as the
  player, `nix build` of the package and the Nix module check (the VM has
  neither the pinned inputs nor the room), and a Waybar comparison (its
  nixpkgs build has no `mpris` module).
