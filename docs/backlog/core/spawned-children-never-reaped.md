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
a user would have to spawn for weeks.

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

Worth getting exactly right, because the recommended mechanism leans on it.
Measured on the dev VM (two small `rustc` probes, not reasoned from memory):

| disposition/state in the parent | after `execve` |
| --- | --- |
| **ignored** (`SIG_IGN`) | **survives** — child shows `SigIgn: …10000` |
| **handled** (a function) | reset to `SIG_DFL` — child's `SigCgt` loses the bit |
| **blocked** (in the signal mask) | **preserved** — child shows `SigBlk: …10000` |

The blocked row is the one that is easy to get backwards: `execve`
**preserves the signal mask**. scoot's children come out with an empty mask
today only because `std::process::Command` does its own `sigprocmask` in the
child before `exec` — a libstd behavior, not an exec guarantee. (Proven by
blocking SIGCHLD from a `pre_exec` closure, i.e. *after* libstd's reset, and
exec'ing `grep` directly: `SigBlk: 0000000000010000`. An earlier probe
exec'ing `sh` read `0` and was misleading — bash unblocks SIGCHLD at
startup.)

This matters because **signalfd requires blocking SIGCHLD process-wide.**
Anyone who believes the mask is cleared by `exec` will not check whether
that block leaks into children — and it would, the moment any spawn path
stops going through `Command` (a `pre_exec` closure, a raw fork/exec). A
child with SIGCHLD blocked misses its own children's exits: same class of
silent breakage as the `SIG_IGN` trap, different mechanism.

Related constraint on the same design: `sigprocmask` is **per-thread**, so a
signalfd only sees SIGCHLD if every thread that could receive it has it
blocked. A live `--headless` scoot reports `Threads: 1`, so this is fine
today — but it is an assumption the design rests on, not a given, and it
should be written next to the code.

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
reproducing on nobody's machine. Two ways out, and one should be chosen
deliberately rather than discovered:

- **Track spawned pids** and `waitpid(pid, WNOHANG)` each, so scoot only
  ever reaps children it started. Costs a small set to maintain.
- **Register the reaper only on the real `run` path**, never in a unit test
  that shares a process with the fork/waitpid tests.

(`Command::new` itself appears at `state.rs:970` and three times in
`tests/msg_broken_pipe.rs` — a separate test binary, so not part of this
hazard. The unit-test `fork` sites are.)

### Mechanism options, with the dependency cost of each

- **`libc` directly.** `libc = "0.2"` is **already a direct Linux dependency**
  of `crates/scoot` (`Cargo.toml:99`, added for `SO_PEERCRED`), and the tree
  already calls `libc::waitpid` in two tests. This is the zero-new-dependency
  route, and probably the one to beat.
- **`signalfd` as a `calloop::generic::Generic` source.** Fits the loop the
  way every other fd source already does and keeps reaping on the thread
  where `State` lives — but `rustix 1.1.4` lists `signalfd` under
  `not_implemented!` (`src/not_implemented.rs:300`, the only occurrence of
  the string in the crate), so this is `libc`'s `signalfd` or nothing.
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

Mind the constraint above when writing it: whatever the test installs must
not be a process-wide `waitpid(-1)` drain, or it will reap the children
`ipc/accept/tests.rs` and `wayland_accept/tests.rs` fork for themselves —
green under `nextest`, red under `cargo test`.
