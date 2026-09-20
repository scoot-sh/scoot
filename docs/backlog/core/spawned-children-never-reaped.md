---
title: "Every spawned child becomes a zombie: nothing in scoot ever reaps one"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Every spawned child becomes a zombie: nothing in scoot ever reaps one

Found 2026-09-19 while answering "how do we start waybar/fuzzel/a browser at
session start". **Confirmed live, not code-traced** — this entry is not
reasoning about what the code should do, it is a `ps` listing.

`State::spawn` (`crates/scoot/src/compositor/state.rs:966`) ends with:

```rust
match child.spawn() {
    Ok(_) => tracing::info!(?command, "spawned"),
```

The `Ok` arm binds `_`, so the `std::process::Child` is dropped on the spot.
Rust's `Child` does **not** reap on drop (its docs: *"There is no
implementation of Drop for child processes, so if you do not ensure the
Child has exited then it will continue to run…"*), and nothing installs a
`SIGCHLD` handler to reap it later:

```sh
$ git grep -nE 'SIGCHLD|signal_hook|Signals::new|calloop::signals|libc::signal|sigaction|waitpid|try_wait' -- crates/
crates/scoot/src/compositor/ipc/accept/tests.rs:338:        unsafe { libc::waitpid(child, &mut status, 0) },
crates/scoot/src/compositor/wayland_accept/tests.rs:295:        unsafe { libc::waitpid(child, &mut status, 0) },
```

Two hits, both in **tests**, both a blocking `waitpid` on a child that test
forked itself. Neither is signal handling, so the conclusion holds — but
note those two sites now, because they constrain the fix (see "the shape of
the drain" below). Confirmed independently on a live process: the SIGCHLD
bit (`0x10000`) is set in none of `SigBlk`, `SigIgn` or `SigCgt` in
`/proc/<pid>/status`.

So every child scoot starts — the `--` startup command, every `spawn` bind,
every `scoot msg action spawn …` — stays in the process table as a zombie
from the moment it exits until scoot itself exits.

## The evidence

Dev VM, `/var/cargo-target/debug/scoot` (mtime 2026-09-19 23:53; the tree's
last change to `state.rs` was `40a04f3`, 17:32 the same day, and the
checked-out branch has no commits beyond `main`):

```sh
$ scoot --headless --socket /tmp/zombie-probe.sock --width 800 --height 600 &
$ SCOOT_SOCKET=/tmp/zombie-probe.sock scoot msg action spawn true   # x4
$ ps -o pid,ppid,stat,comm --ppid 351921
    PID    PPID STAT COMMAND
 351924  351921 Z    true <defunct>
 351928  351921 Z    true <defunct>
 351930  351921 Z    true <defunct>
 351932  351921 Z    true <defunct>
```

Four spawns, four zombies, none of which ever goes away. `true` exits
instantly; a real client just moves the moment further out.

The live run also closes a gap the grep alone leaves open: the grep proves
*scoot* installs no handler, not that no dependency installs one. The `Z`
entries prove nothing anywhere in the process reaps.

**Reproduced independently** by review on the same day, different pids
(`352564`/`352566`/`352568`/`352571` under `352559`), still `Z` twelve
seconds later — so this is not one run's artifact.

## Why this is high, stated honestly

The per-zombie cost is small — one pid and one `task_struct`, no memory the
child held, no fds. On an ordinary desktop this is **not** a plausible route
to pid exhaustion: the dev VM's `/proc/sys/kernel/pid_max` is `4194304`, and
a user would have to spawn for weeks. Not quite unconditional even there —
the kernel uncharges both the pids cgroup and the per-user `RLIMIT_NPROC` in
`release_task()`, i.e. at *reap* time, so a zombie holds its `RLIMIT_NPROC`
slot too, and that ceiling is far lower than `pid_max`.

**On the named deployment target it is a different story, and that is worth
stating rather than waving off.** A linuxserver webtop is a container, where
`pids.max` (Docker's `--pids-limit`, a k8s pod pid limit) counts zombies as
live tasks — they occupy the cgroup's budget exactly like running processes.
s6-overlay as pid 1 reaps *orphans*, which these are not: they stay scoot's
children for as long as scoot lives. A long agent-driven session against a
pid-limited container eventually meets `fork: Resource temporarily
unavailable` and can then spawn nothing at all, including whatever the user
would reach for to recover. That is a wedged session, not an untidy `ps`.

The case for high rests on three things, of which the container one above is
the sharpest:

1. **It is unbounded within a session, and scoot is a session-lifetime
   process.** A `--tty` daily driver runs for days. Every launcher hit, every
   terminal opened and closed, every `spawn` bind leaves one behind.
2. **It is visible.** `htop`, `ps`, `btop` all show `<defunct>` rows parented
   to the compositor. On a compositor people are asked to daily-drive, a
   growing column of zombies under `scoot` reads as "this thing leaks",
   whatever the actual byte cost.
3. **Agent-driven use is the fastest accumulator.** Computer use means many
   short-lived spawns — the exact pattern that produces one zombie per
   action, and `docs/ipc.md` invites it.

Against that: the fix is small, self-contained, and testable in-harness. A
confirmed bug with a bounded fix is a good next pick regardless of the size
of the harm.

## Shape

**Do not reach for `signal(SIGCHLD, SIG_IGN)`.** It is the one-liner that
looks right and is a trap: an *ignored* disposition survives `execve`, so
every child scoot spawns inherits it, and any of them that calls `wait()` on
its own children gets `ECHILD` instead of an exit status. Shells, `waybar`'s
script modules and anything supervising a subprocess break in ways that are
very hard to trace back here.

### What `execve` actually does to signal state

Worth getting exactly right, because it decides which mechanism wins.
Measured on the dev VM with `rustc` probes that exec `grep` directly — **no
shell anywhere in the path**, which matters: a probe that exec'd `sh` read
the mask as empty and was wrong, because bash unblocks SIGCHLD at startup.

| state in the parent | in the child after `execve` |
| --- | --- |
| **ignored** (`SIG_IGN`) | **survives** — child shows `SigIgn: …10000` |
| **handled** (a function) | reset to `SIG_DFL` — child's `SigCgt` loses the bit |
| **blocked** (in the signal mask) | **preserved** — child shows `SigBlk: …10000` |

The blocked row is the one that is easy to get backwards, and it is not
rescued by libstd. One probe shows both halves at once — parent blocks
SIGCHLD, then a plain `Command::spawn` (no `pre_exec`, no raw fork):

```text
parent SigBlk: 0000000000010000   child SigBlk: 0000000000010000   <- mask inherited
parent SigIgn: 0000000000001000   child SigIgn: 0000000000000000   <- SIGPIPE reset
```

Read it as: `std::process::Command` resets **SIGPIPE only** — bit `0x1000`,
signal 13, which Rust ignores at its own startup and hands back to children
as `SIG_DFL` — and **inherits the mask untouched**. (Behavioral evidence,
measured here; review reports that libstd's `sys/process/unix/unix.rs` says
the same in a comment, which nobody on this ticket has read directly. The
measurement is the load-bearing part and does not depend on it.)

So **nothing clears the mask. There is no safety net.**

This decides the mechanism, because **signalfd requires blocking SIGCHLD
process-wide**:

- Choose **signalfd** and every child spawned through the ordinary `Command`
  path — the user's terminal, the shell in it, waybar with script modules,
  anything from a `spawn` bind or `scoot msg action spawn` — starts with
  SIGCHLD blocked and misses its own children's exits. Not a latent hazard
  waiting on some future raw fork: the ordinary path *is* the leak. Taking
  this route therefore **obliges** a `pre_exec` closure that
  `sigprocmask(SIG_SETMASK, <empty>)`s in every child. Not optional.
- Choose a **`sigaction` handler** that writes to a `calloop::ping` and the
  problem does not exist: row 2 says a caught handler is reset to `SIG_DFL`
  by `exec`, so it cannot leak into any child, and nothing needs blocking.

That asymmetry is the single most useful thing on this page, and it is the
reverse of the intuition that a signalfd is "the clean modern way".

Related constraint if signalfd is chosen anyway: `sigprocmask` is
**per-thread**, so a signalfd only sees SIGCHLD if every thread that could
receive it has it blocked. A live `--headless` scoot reports `Threads: 1`,
so this is fine today — but it is an assumption the design rests on, not a
given, and it should be written next to the code.

### The shape of the drain

A signal source on the existing calloop loop, draining in a loop —
pseudo-code, since signals coalesce and one `SIGCHLD` can stand for several
exits:

```text
while let Some((pid, status)) = waitpid(<target>, WNOHANG)? {
    debug!(?pid, ?status, "child exited");
}
// stop on "no more children" / "nothing to report"
```

**`<target>` is the real decision, and `-1` (reap anything) is not obviously
safe.** The two `libc::waitpid` sites the grep above turned up —
`compositor/ipc/accept/tests.rs:338` and `compositor/wayland_accept/tests.rs:295`
— `libc::fork()` a child and then block on `waitpid(child, …, 0)` for its
status. They live in the **`scoot` unit-test binary**, which is the same
process the in-harness reaper test proposed below would run in. A
process-wide `waitpid(-1)` drain installed there would reap those children
first, and their `waitpid(child, …)` would return `ECHILD` and fail the
assertion.

Worse, that failure is **runner-dependent**: `nextest` gives each test its
own process, so it cannot see it; `cargo test` shares one, so it can. Per
`CLAUDE.md`'s own note on the asymmetry, that lands as a CI-only failure
reproducing on nobody's machine. Three ways out, not equal:

- **Track spawned pids** and `waitpid(pid, WNOHANG)` each, so scoot only
  ever reaps children it started. Costs a small set to maintain, and
  **dominates the next option** — it removes the hazard *and* leaves the
  in-harness test able to install the reaper and assert on it.
- **Register the reaper only on the real `run` path**, never in a unit test
  sharing a process with the fork/waitpid tests. Safe, but self-defeating on
  its own: the in-harness test below presumes the test installs the reaper,
  so this option deletes the test it is protecting.
- **…unless it is paired with an integration test** in `crates/scoot/tests/`
  (its own binary, exactly like `msg_broken_pipe.rs`) that launches a real
  headless scoot and reads `ps --ppid`. That exercises the real `run` path
  and shares no process with the unit tests, so it composes with the option
  above instead of contradicting it.

(`Command::new` itself appears at `state.rs:970` and three times in
`tests/msg_broken_pipe.rs` — a separate test binary, so not part of this
hazard. The unit-test `fork` sites are.)

### Mechanism options, with the dependency cost of each

Ordered by the mask finding above, not just by dependency cost.

- **`libc::sigaction` + a `calloop::ping`** — the one to beat. `libc = "0.2"`
  is **already a direct Linux dependency** of `crates/scoot`
  (`Cargo.toml:99`, added for `SO_PEERCRED`) and calloop is already in the
  tree, so this adds nothing. It blocks no signal, so it cannot leak into a
  child, and `exec` resets the handler for free. The handler itself must be
  async-signal-safe — writing one byte to a ping fd is, which is the whole
  reason for the ping.
- **`signalfd` as a `calloop::generic::Generic` source.** Fits the loop the
  way every other fd source already does and keeps reaping on the thread
  where `State` lives — but it **requires** the process-wide block, and
  therefore the `pre_exec` mask reset in every child described above. Also
  `rustix 1.1.4` lists `signalfd` under `not_implemented!`
  (`src/not_implemented.rs:300`, the only occurrence of the string in the
  crate), so it would be `libc`'s `signalfd` regardless.
- **calloop's own `signals` feature.** Verified against the pinned
  `calloop 0.14.4`: `signals = ["nix"]`, and `Cargo.lock` shows calloop's
  dependency set today as `bitflags, polling, rustix, slab, tracing` — so
  the feature is **off**, and enabling it means a direct `calloop` dependency
  and pulls `nix`, which is not in the tree.
- **`signal-hook` plus a `calloop::ping`.** Also a new crate.

Whichever wins, the handler belongs next to `spawn`, and the decision should
be written down — a future reader will otherwise re-ask why the obvious
`SIG_IGN` was not used.

## What this is *not*

This is reaping, not supervision. Restarting a bar that died, backing off a
crash loop and deciding whether an exit was the user's intent are a service
manager's job, not a compositor's — no peer compositor supervises. See
[`startup-programs-and-autostart.md`](../config/startup-programs-and-autostart.md)
for where that boundary is drawn and why. The two are related only in that a
`SIGCHLD` source is the hook anything else would need, and that reaping is
worth doing whether or not supervision ever lands.

## Tests

In-harness and cheap: spawn a fast-exiting child through `State::spawn`, run
the loop, and assert the process table under scoot's pid holds no `Z` entry
(read `/proc/<pid>/stat`'s state field, or `ps --ppid`). Fail-first is easy
to prove by neutering the handler. Worth a second test that the *child's*
`SIGCHLD` disposition is `SIG_DFL` **and its signal mask is empty** — the
two regressions the `SIG_IGN` shortcut and the signalfd block would
introduce respectively, neither of which anything else would catch. Both
read straight out of the child's `/proc/self/status` (`SigIgn`, `SigBlk`).

The `SigBlk` half is not hypothetical: since libstd inherits the mask, a
signalfd implementation that forgets its `pre_exec` reset fails this test
and nothing else in the suite would notice.

Mind the constraint above when writing it: whatever the test installs must
not be a process-wide `waitpid(-1)` drain, or it will reap the children
`ipc/accept/tests.rs` and `wayland_accept/tests.rs` fork for themselves —
green under `nextest`, red under `cargo test`.
