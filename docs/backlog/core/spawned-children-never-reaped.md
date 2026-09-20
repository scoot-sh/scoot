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
Rust's `Child` does **not** reap on drop — its docs say so in as many words —
and there is no signal handling anywhere in the workspace to reap it later:

```sh
git grep -nE 'SIGCHLD|signal_hook|Signals::new|calloop::signals|libc::signal|sigaction|waitpid|try_wait' -- crates/
# (three unrelated hits, all `session_lock.cancel_blank_wait`)
```

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

## Why this is high, stated honestly

The per-zombie cost is small — one pid and one `task_struct`, no memory the
child held, no fds. This is **not** a plausible route to pid exhaustion:
`pid_max` is in the millions on a systemd box, and a user would have to spawn
for weeks.

The case for high is different, and rests on three things:

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
very hard to trace back here. (A `SIG_DFL` disposition and a blocked/handled
mask are reset across `exec`; an ignored one is not. That asymmetry is the
whole hazard.)

The right shape is a signal source on the existing calloop loop, with a
`waitpid(-1, WNOHANG)` drain:

```rust
loop {
    match waitpid(-1, WNOHANG) {
        Ok(Some(pid, status)) => tracing::debug!(?pid, ?status, "child exited"),
        Ok(None) | Err(ECHILD) => break,
    }
}
```

The drain loop matters: signals coalesce, so one `SIGCHLD` can stand for
several exits.

`waitpid(-1, …)` — reap *anything* — is safe here specifically because
**`Command::new` appears exactly once in the whole crate**
(`state.rs:970`). scoot has no other child it might be waiting on, so there
is no risk of stealing a status some other part of the code needs. If a
second spawn site ever appears, that reasoning has to be revisited, and the
comment on the handler should say so.

Mechanism options, with the dependency cost of each:

- **calloop's own `signals` feature.** Verified against the pinned
  `calloop 0.14.4`: `signals = ["nix"]`, and `Cargo.lock` shows calloop's
  dependency set today as `bitflags, polling, rustix, slab, tracing` — so
  the feature is **off**, and enabling it means a direct `calloop` dependency
  and pulls `nix`, which is not in the tree.
- **`signalfd` as a `calloop::generic::Generic` source.** No new crate if
  `rustix` (already in the tree, 1.1.4) exposes what is needed; fits the loop
  the way every other fd source already does, and keeps reaping on the loop
  thread where `State` lives.
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
`SIGCHLD` disposition is `SIG_DFL`, which is the regression that the
`SIG_IGN` shortcut would introduce and that nothing else would catch.
