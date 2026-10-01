---
title: "`exec`, `push` and `button` modules: extend the bar without Rust"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M4"
resolved: "2026-09-30"
---

# `exec`, `push` and `button` modules — RESOLVED

Resolved 2026-09-30 (draft PR, second of the M4 stack, on the pointer input
PR). What landed, the decisions, and the evidence are below the original
entry, which is kept as filed. **The resource ratchet for it is measured on the
dev VM only (below); the run on the real hardware is still to do.**

Filed 2026-09-29. Serves **daily-drive** (launcher and power buttons) and
**computer use** (anything can put text on the bar).

The escape hatches that keep the built-in module set small.

- **`button`**: an icon or text and the interaction keys. A launcher button is
  `on-click = { exec = ["scootlaunch"] }` (fuzzel or wofi until ours exists);
  log out is `{ scoot = "quit" }` or `exec = ["scoot","msg","action","quit"]`.
- **`exec`**: run a command and treat each stdout line as an update, plain text
  or one JSON object (`text`, `class`, `tooltip`). The child's stdout is a
  polled fd; it restarts with backoff if it exits, and a chatty child cannot
  grow memory (line cap, drop the excess with a warning).
- **`push`**: `scootbar msg set ID JSON` updates a module from anywhere. No fd
  of its own beyond the control socket, so it costs nothing until used.

scootbar's own JSON shape, deliberately not Waybar's; document it and version
it. Both get the standard interaction keys. A module with neither output nor
push yet shows nothing (or a configured placeholder), not an error.

## Streaming, not polling

Interval-polled scripts are a leading source of leaks and CPU burn in other bars
(Waybar #5303, #4987). The design answer is the stdout stream above: a script that
can wait for an event prints when it has one. Many scripts cannot, so decide whether
to offer `interval = N` (the bar re-runs the command every N seconds) as a
convenience: if yes, **off by default, a documented floor (no sub-second), one
shared timer, one run in flight per module, and the cost said plainly in the docs**;
if no, say so and show the recipe (`while sleep N; do ...; done`), which moves the
choice, and the cost, into the user's script.

(*As filed. The recipe the docs show is `while :; do ...; sleep N; done`: the
filed form sleeps before its first print, so the module sits on its
placeholder for N seconds after every start and reload.*)

## Limits and safety

Command lines are never passed through a shell unless the user writes `sh -c`
explicitly. The control socket is `0600`, same-user, and `set` only changes
display text; it cannot run anything. Cap line length, module count and
update rate (coalesce to the next frame).

## Done when

A shell one-liner becomes a module with a click action, and `scootbar msg set`
updates it with no polling on either side.

## What landed

[`docs/scootbar/cli.md#button-push-and-exec-modules`](../../cli.md#button-push-and-exec-modules)
is the reference.

- **Modules defined by name.** `[button.NAME]`, `[push.NAME]` and
  `[exec.NAME]` tables (`crates/scootbar/src/config/custom.rs`), placed by the
  lists like built-in ones (`modules/custom.rs`). A name is 1 to 32 letters,
  digits, `-` or `_`, not a built-in's id, unique across the kinds, at most 32
  defined. Ids are `&'static str` in the registry, so a name is **interned**
  (leaked once, reused by every later reload, at most 256 distinct over the
  daemon's life, a few hundred bytes), and only a *placed* module starts. Each
  of `button`, `push` and `exec` is a Cargo feature, on by default; a build
  without one refuses its table as an unknown key.
- **`button`** (`modules/button.rs`): text and the clock's three icon keys
  (`config/icon.rs` generalized to take a table's prefix), a margin, the
  interaction keys. No fd, no wakeup, nothing allocated warm. A launcher is
  `on-click = { exec = ["scootlaunch"] }`.
- **`push`** (`modules/push.rs`): `scootbar msg set ID VALUE`, VALUE a JSON
  string (the text), an object of the payload, or `null` (clears it). No fd,
  no timer, no thread; a `set` that changes nothing is not a redraw, several
  in one turn are one. `msg set` now takes any well-formed id (the daemon says
  what is placed), and the control socket reads `"value": null` as a value
  (serde would read it as absent).
- **`exec`** (`modules/exec/`): streams a command's stdout, one update per line.
  The pipe is polled; **at most one 4 KiB read per 16 ms** while data keeps
  coming (the pipe is not even polled in between, so the child fills it and
  blocks: back-pressure, no buffering, and the last line of a read is what is
  shown); a line past 4096 bytes is dropped whole, never truncated; restarts
  after 1 s, 2, 4 ... 60 s, a run of 30 s starting over; reaped through a
  pidfd; stdin `/dev/null`, stderr the bar's, no inherited fd; the **whole
  process group is killed** when the module goes (a reload) and when the
  command exits (a worker it backgrounded does not pile up), and **the
  command ends with the bar however the bar ends** (decision 7). At most 8
  placed.
- **The payload** (`modules/payload.rs`, `class.rs`): scootbar's own, version
  1, `text`, `class`, `tooltip`, `version` (a newer one is refused by name),
  unknown keys ignored; lines and values past 4096 bytes and JSON nested past 8
  deep refused; text cut at 256 bytes on a character boundary with every
  control character a space. A **`payload` fuzz target** (`fuzz/`, in CI with
  a fixed budget) and a stable replay of its corpus.

## Decisions

1. **No `interval` key.** The entry asked for the decision. Streaming is the
   point and the lightest way to run a script: the bar waits on a pipe and does
   nothing in between. `interval` would add a shared timer, a run-once state per
   module and a floor to defend, for a convenience `while :; do ...; sleep N; done`
   gives in a script the user can see, with the cost where they can see it. The
   recipe is in the docs (print first: the filed `while sleep N; do ...; done`
   shows the placeholder for N seconds after every start and reload). (`interval = N` can still be added later, off by
   default; nothing here makes it harder.)
2. **`format` is explicit (`text` or `json`, default `text`).** Detecting JSON
   by a leading `{` would show a script's `{brace}` text as a parse error.
3. **`button`, `push` and `exec` are Cargo features and in `default`**, as the
   other modules. They cost 91 KB of `.text` in a build that never configures
   one (below); **a maintainer call** whether they belong in `default`, or whether
   a user wanting only a clock and workspaces should opt in.
4. **Modules defined by name are named in the config file's lists only**:
   `--left` and `--center` take the built-in ids. The flags are for the
   built-in ones.
5. **A reload restarts every `exec` command**, whether or not its table
   changed: the old module is dropped (its process group killed) and the new one
   started. Simple, and a reload is rare; keeping a child across a reload that
   did not touch it is a refinement, not a gap
   ([exec-keep-across-reload](../exec-keep-across-reload.md): estimated, not
   small, so a ticket).
6. **Killing a group after the leader was reaped** has one theoretical
   hazard, a pid reused between the reap and the `kill`; the group number stays
   the group's while any member lives, and a free pid is not reused until the pid
   space wraps (millions of forks), so it cannot happen in that interval. It is
   written in the code beside the call.

7. **An `exec` command ends with the bar, however the bar ends: the guard**
   (review finding, measured). The first draft killed a command's process
   group in `Drop for Running`, which runs on a clean exit only. Run with
   `~/m4/orphan.sh` on the dev VM (a headless scoot, the bar, an `exec` module
   running `sh -c "while :; do date +%S; sleep 1; done"`, a signal to the bar,
   the loop shells counted 0.3 s and 3 s later; `env --default-signal=INT,QUIT`
   because a background job of a non-interactive shell hands its child
   `SIGINT` ignored):

   ```text
   before (8e34e3fd3's bar)        loop shells alive   after (d201a88ed's bar)   loop shells alive
                                   +0.3 s   +3 s                                 +0.3 s   +3 s
   SIGTERM  (exit 143)               1        1        SIGTERM  (exit 143)         0        0
   SIGINT   (exit 130)               1        1        SIGINT   (exit 130)         0        0
   SIGHUP   (exit 129)               1        1        SIGHUP   (exit 129)         0        0
   SIGKILL  (exit 137)               1        1        SIGKILL  (exit 137)         0        0
   SIGABRT  (exit 134; a panic)      1        1        SIGABRT  (exit 134)         0        0
   ```

   (`kill -SEGV` is no crash test: Rust's handler resumes a signal sent from
   outside, and the bar went on running.) The survivors were reparented to pid
   1 and ran for ever: a loop left by an earlier session's testing of this
   very PR, `sh -c "while :; do date +%H:%M; sleep 60; done"`, was found still
   running on the VM (parent pid 1) long after the bar that started it was gone,
   and killed by hand. The docs claimed otherwise (`SIGPIPE` ends only the
   *writer*, not the shell that loops, and `SIGTERM` and `SIGINT` were called
   "harmless"), which was wrong for this module.

   **Options, and why this one.** The harm is real, and `CLAUDE.md` says not to
   defer one, so it is fixed, within the crate's rules (`#![forbid(unsafe_code)]`,
   no thread, no signal handler, nothing new in the dependency tree):
   - *Catch `SIGTERM`/`SIGINT`/`SIGHUP` and exit through the normal path, whose
     `Drop` kills the groups.* Needs a signal handler or a blocked mask read
     through a `signalfd` (rustix has neither a safe `sigprocmask` nor
     `signalfd`; the loop has no equivalent), so `unsafe`; and it cannot cover
     `SIGKILL`, a crash or the out-of-memory killer, which the finding
     includes. Rejected.
   - *`PR_SET_PDEATHSIG` from `Command::pre_exec`.* The one mechanism that
     covers every death, but `pre_exec` is `unsafe`. Rejected as written.
   - *A safe crate offering it.* None in the tree (`nix` and `libc` are not
     dependencies, and one wrapping `prctl` would still have to run it in the
     child, which is the `pre_exec` problem again); `rustix` already wraps the
     call safely.
   - *A pipe whose write end the child holds, so an orphan sees end of file.*
     Does nothing for an arbitrary command. Rejected.
   - **Shipped: the bar starts the command through itself** (`modules/exec/
     guard.rs`). It runs `/proc/self/exe __exec-guard BAR-PID PROGRAM ARGS`;
     that process arms `PR_SET_PDEATHSIG(SIGKILL)` on itself with `rustix`'s
     safe `set_parent_process_death_signal`, checks `getppid()` is still the bar
     (the bar may have died before the signal was armed), and replaces itself
     with the command by `CommandExt::exec` (safe). The command is then the child
     the bar spawned: same pid, same group, same pipe, same pidfd, nothing extra
     running and no thread. Commands a pointer binding starts are not guarded (a
     launched application outliving a bar restart is the point of `KillMode=process`).

   Cost, measured on the Asahi M2 (release, `lto = "fat"`, stripped): `.text`
   +3,424 bytes (1,076,392 to 1,079,816: +0.3%), `.rodata` +192, `.eh_frame`
   +400, the file the same size in 64 KiB-quantized segments (1,446,624 either
   way); a spawn through the guard takes about 1.55 ms against 1.16 ms for a
   plain `fork` and `exec` of `/bin/true` (three runs of 1,000: 1,567, 1,538
   and 1,575 ms against 1,149, 1,168 and 1,165), **+0.4 ms once per start**,
   and a command restarts at most once a second. Idle cost: none (the guard
   is gone once it has exec'd).

   **What remains, and it is the maintainer's to weigh:** the signal is not
   inherited by what the command starts. A shell loop dies, but the `sleep 60`
   it was in the middle of runs out its sleep (the next write of a writer such
   as `date` or `curl` to the closed pipe ends it with `SIGPIPE`), and a worker
   the command backgrounded and left is not touched by a killed bar (a clean
   exit and a reload still kill the whole group). A set-user-id command is not
   covered (the kernel clears the signal across such an `exec`). A command
   that is itself a shell loop with a long `sleep` therefore leaves that one
   sleeping process behind, for at most its sleep.

## The ratchet

The same bench, dev VM and settings as the pointer entry's
([the ratchet there](pointer-and-interactions-done.md#the-ratchet-not-passed)):
`bench.py run --scope clock-workspaces --compositors scoot --rounds 3
--settle-secs 20 --idle-secs 120 --switches 40`, two runs of each alternating,
**PR 1** (pointer input, `c9361fc4c`'s tree) against **this PR**, so the rows are
the cost of this entry alone. **Dev VM numbers, not the ratchet's machine; the
run that counts is the maintainer's and is still to do.**

| Row | PR 1 (2 runs) | this PR (2 runs) | `compare` |
| --- | --- | --- | --- |
| Size, stripped binary + non-glibc closure | 1,514,152 | 1,579,720 | same (+4.3%) |
| Bare executable (not gated) | 1,381,056 | 1,446,624 | +4.7% |
| `.text` / `.rodata` / `.eh_frame` | 984,488 / 101,095 / 96,996 | 1,076,328 / 104,191 / 101,252 | +9.3% of `.text` |
| Idle RSS, PSS | 3.5, 3.5 / 1.9, 1.9 MiB | 3.5, 3.5 / 1.9, 1.9 | same |
| Idle wakeups per minute | 2, 2 | 2, 2 | same |
| Idle CPU, 120 s window | 1.1, 0.7 ms | 1.1, 1.0 | same, regressed (run 2) |
| CPU while switching workspaces | 4.4, 4.3 ms | 4.4, 4.2 | same |
| Startup to first frame | 32.3, 37.5 ms | 46.8, 33.5 | regressed (run 1), same |
| Lines of Rust / direct dependencies | 28,732 / 10 | see `bench.py report` / 10 | no new dependency |

- **A build that configures none of the three costs nothing at run time**
  (RSS, PSS, wakeups and the switching CPU are unchanged) and 66 KB of binary,
  91 KB of `.text`, mostly the config's `toml` deserialization of three more
  table shapes and `Command`.
- **What using each costs** (this PR's binary, a clock plus one module, one
  120 s window each, bar process only, the commands' own CPU not counted):

  | Config | RSS | Idle wakeups / 120 s | CPU |
  | --- | --- | --- | --- |
  | clock only | 3,672 kB | 4 | 1.09 ms |
  | + a `button` | 3,688 kB | 4 | 1.08 ms |
  | + a `push` (never written to) | 3,688 kB | 4 | 0.85 ms |
  | + an `exec` printing a line a minute | 3,936 kB | 8 | 1.32 ms |
  | + an `exec` printing a line a second | 3,948 kB | 242 | 27.8 ms |

  A line is two wakeups (the pipe, and the compositor's release for the frame it
  draws) and about 0.22 ms of the bar's CPU. A `button` or a `push` costs
  16 kB and nothing else; an `exec` costs about 260 kB (the module, its pipe's
  line buffer, a second `wl_shm` buffer when it first redraws while the first is
  held) and one extra process.
- **A flooding child** (`yes`, as fast as the pipe takes it): over 3 seconds
  the bar's RSS grew by at most 1 MiB (asserted), its descriptor count did not
  move, and it spent under a quarter of a core (asserted; in practice a few
  percent), all in `tests/exec.rs`.

## Evidence

On the dev VM (aarch64, 6 vCPUs), each tree in its own directory with its own
`CARGO_TARGET_DIR` and `CARGO_INCREMENTAL=0` (the disk is small), shipped by
`tar` over `ssh`. The integration tests ran against the VM's `scoot` of
2026-09-28 (ipc protocol 4). The fuzz target was **compiled** (`cargo check
--locked` of the fuzz crate) and its corpus replayed on the stable toolchain;
**`cargo fuzz` itself was not run** (no cargo-fuzz here; CI runs it for 2,000,000
iterations).

The checks ran at `18d954692` (the code commit; the commit after it is
backlog-only). Raw results:

```text
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar [FLAGS] --all-targets -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | --no-default-features --features clock
  | ... --features workspaces | ... --features icon-image | ... --features button
  | ... --features push | ... --features exec | --features icon-image | --all-features
SCOOTBAR_TEST_SCOOT=/var/cargo-target/debug/scoot SCOOTBAR_REQUIRE_SCOOT=1 \
  cargo nextest run -p scootbar --no-fail-fast        Summary 623 tests run: 623 passed, 0 skipped
cargo nextest run -p scootbar --no-fail-fast  (target dir with no scoot)   623 passed (the integration tests skip)
SCOOTBAR_TEST_SCOOT=... SCOOTBAR_REQUIRE_SCOOT=1 cargo test -p scootbar    545 + 3 + 11 + 8 + 11 + 7 + 4 + 2 + 1 + 4 + 11 + 7 + 9 passed
cargo nextest run -p scootbar --bin scootbar --all-features                564 passed
  ... --no-default-features 349 | clock 422 | workspaces 408 | icon-image 367
  | button 371 | push 374 | exec 391
cargo check --locked --bins in crates/scootbar/fuzz                        ok (payload target compiles)
nix build path:.#packages.aarch64-linux.scootbar                          ok
nix build path:.#checks.aarch64-linux.scootbar-modules                     ok
```

A first run at the code commit before it failed one test under `cargo test`:
`dropping_the_module_kills_the_command_and_everything_it_started` asserted
that this *process* had no children, which another test's child breaks when
tests share one process (`cargo test`); it and the zombie test now check the
command's own pid (`echo $$`), and both runners pass.

Not verified: real hardware, sway, a `button` whose command opens a window on a
real desktop (the click was checked to run the command), and `cargo fuzz`.

### After review (2026-10-01)

The review found a false claim and a flaky test; both are fixed in
`d201a88ed` (this entry is the commit after it and changes no code). The
checks above ran at an earlier tree, so they are stale for what it touched
and were re-run at it. The stack was rebased onto `main` at `7a1f9030a` and
the pointer PR's fixes in between (no file under `crates/` changed by the
rebase itself).

- **The false claim: a killed bar left shell-loop commands running for ever**
  (decision 7: the guard, the five-signal measurement before and after, the
  options, the cost, and what remains).
- **Two flaky tests**: `dropping_the_module_kills_the_command_and_everything_it_started`
  asserted `running("sleep 311")` once, right after the script printed
  `started`, when the forked worker might not have exec'd `sleep` yet (the
  same in `tests/exec.rs`'s reload test). Each now waits for the worker against
  a deadline. The unit test, alone, in a loop with four spinning CPUs on the dev
  VM (`~/m4/stress.sh`, one `--exact` run per iteration, 300 iterations):

  ```text
  before (8e34e3fd3's test):  7 failures / 300   (iterations 37, 40, 88, 184, 191, 206, 255)
  after  (d201a88ed):         0 failures / 300   load average 1.75
  ```
- **Docs**: the recipe is print-first (`while :; do ...; sleep N; done`), a
  one-shot command is a poll backing off to a minute, and "one line per
  restart" says what it is (throttled to one warning a second); the signals
  text in `daemon/mod.rs`, `--help` and `cli.md` no longer calls a kill
  "harmless" for the bar's commands.
- **Not done, and a ticket**: keeping an unchanged `exec` child across a reload
  ([exec-keep-across-reload](../exec-keep-across-reload.md)): it touches
  `modules::start`, the trait and every other module kind, which is not small.
- **A test that is not here, and why**: the integration tests kill the bar
  with `SIGKILL` and `SIGTERM`; `SIGINT`, `SIGHUP` and a crash are measured by
  hand (the table in decision 7). A harness under `nohup` hands the bar
  `SIGHUP` ignored, and a background job of a shell hands it `SIGINT`
  ignored, so a test for either would hang or lie depending on how it was
  started (an earlier draft hung a `cargo test` run under `nohup` for 18
  minutes, which is why the wait is now bounded).

```text
checks at d201a88ed (git archive of it, dev VM, own targets in /dev/shm, scoot built from
the pointer tree: no crates/scoot change anywhere in the stack, /dev/shm/m4t/debug/scoot):
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar [FLAGS] --all-targets -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | ... --features clock | ... --features workspaces
  | ... --features button | ... --features push | ... --features exec
  | --features icon-image | --all-features
SCOOTBAR_TEST_SCOOT=/dev/shm/m4t/debug/scoot SCOOTBAR_REQUIRE_SCOOT=1 \
  cargo nextest run -p scootbar --no-fail-fast        Summary 633 tests run: 633 passed, 0 skipped
cargo nextest run -p scootbar --no-fail-fast  (target dir with no scoot)   633 passed (the integration tests skip)
SCOOTBAR_TEST_SCOOT=... SCOOTBAR_REQUIRE_SCOOT=1 cargo test -p scootbar    553 + 3 + 11 + 8 + 13 + 7 + 4 + 2 + 1 + 4 + 11 + 7 + 9 passed
cargo nextest run -p scootbar --bin scootbar --all-features                572 passed
  ... --no-default-features 350 | clock 423 | workspaces 409 | icon-image 368
  | button 372 | push 375 | exec 399
cargo check --locked --bins in crates/scootbar/fuzz                        ok
```

Not re-run: the two `nix build` checks of the original run (no `nix/` file
and no dependency changed), and real hardware beyond the Asahi release builds
used for the guard's cost.
