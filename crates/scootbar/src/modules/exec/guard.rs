//! The step between the bar and an `exec` command that makes the command
//! die with the bar, however the bar dies.
//!
//! A bar that is killed (`SIGTERM`, `SIGKILL`, a crash) runs no destructor,
//! so [`super::Running`]'s group kill cannot be what ends its commands, and
//! a command that is a shell loop (`sh -c "while :; do date; sleep 1;
//! done"`) would run on for ever, reparented to init. The kernel can end
//! it: `prctl(PR_SET_PDEATHSIG, SIGKILL)` delivers a signal to a process
//! when its parent dies. But it is set by the child, on itself, and the one
//! place Rust offers between `fork` and `exec` to run code is
//! `Command::pre_exec`, which is `unsafe` (and the crate forbids `unsafe`).
//!
//! So the bar starts the command through itself: it runs `/proc/self/exe`
//! (always this binary, even one replaced on disk since it started) with
//! [`MARKER`], its own pid and the command line. That process, the guard,
//! is a whole new program in which `rustix` sets the parent-death signal
//! (safe), checks the parent is still the bar (the bar may have died before
//! the signal was armed, and the signal is only for deaths after it), and
//! replaces itself with the command with `CommandExt::exec` (safe). The
//! command is then the child the bar spawned (same pid, same process group,
//! same pipe and pidfd), with the signal armed. Nothing is left running in
//! between: no extra process, no thread, nothing allocated after the exec.
//!
//! What this covers, and what it does not:
//!
//! - **Covers**: the command itself, when the bar dies for any reason a
//!   process can: `kill -TERM`, `-INT`, `-HUP`, `-KILL`, a crash, the
//!   out-of-memory killer. A shell loop is the command, so it is gone.
//! - **Does not cover** what the command started in turn (the kernel clears
//!   the signal across `fork`): the `sleep 1` a loop was in the middle of
//!   runs out its sleep (the writer of a pipe whose reader is gone ends by
//!   `SIGPIPE`/`EPIPE` on its next write), and a worker the command
//!   backgrounded and left is not touched. That is why a clean exit and a
//!   reload still kill the whole group ([`super::kill_group`]).
//! - The signal is cleared by a set-user-id or file-capability `exec`, so a
//!   command that is one such program is not covered.
//!
//! The signal is tied to the *thread* that spawned the command: the bar
//! spawns from its one thread and has no other, so that is the bar's life.
//!
//! In the unit tests `/proc/self/exe` is the test harness, not the bar, so
//! they start the command directly (the guard is checked on a running bar,
//! `tests/exec.rs`, and its pieces below).

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

use rustix::process::{Pid, Signal, getppid, set_parent_process_death_signal};

use crate::print::warn;

/// The first argument that makes `scootbar` a guard instead of the CLI. Not
/// in `--help`: it is the bar's own, and harmless to run by hand (it runs
/// the command it is given, as the user could).
pub const MARKER: &str = "__exec-guard";

/// The statuses of a guard that could not become the command, which are a
/// shell's: 127 (no such program) and 126 (found, not executable) are said
/// by the bar's one warning for the restart ([`meaning`]), and the guard
/// stays silent so a failed start is one line, not two. Any other failure
/// has nothing to say it but the guard, which does, and exits 125.
const NOT_FOUND: u8 = 127;
const NOT_EXECUTABLE: u8 = 126;
const NOT_RUN: u8 = 125;

/// What a command's exit `code` says when it is one of the guard's: the
/// words for the bar's warning (a command that exits with one of them by
/// itself, as `sh -c missing` does with 127, means the same).
pub fn meaning(code: i32) -> Option<&'static str> {
    match u8::try_from(code).ok()? {
        NOT_FOUND => Some(", command not found"),
        NOT_EXECUTABLE => Some(", not executable"),
        NOT_RUN => Some(", could not be run: see the line above"),
        _ => None,
    }
}

/// Why `command` (from [`command`]) could not be started, for the module's
/// warning about `program`. Through the guard, the process the bar failed to
/// start is the bar itself at `/proc/self/exe`, not the user's program (the
/// guard runs that, and says when it cannot), and what is missing when it
/// is not found is `/proc`: naming the user's program there sent a reader
/// looking at the wrong thing.
pub fn spawn_error(command: &Command, program: &str, error: &io::Error) -> String {
    let started = command.get_program();
    if started == OsStr::new(program) {
        return format!("cannot run `{program}`: {error}");
    }
    let hint = if error.kind() == io::ErrorKind::NotFound {
        " (the bar runs each command through itself, which needs /proc mounted)"
    } else {
        ""
    };
    format!(
        "cannot start `{}` to run `{program}`: {error}{hint}",
        started.to_string_lossy()
    )
}

/// The command that starts `program args` the way the module wants it: its
/// stdio and group are set by the caller. Through the guard, except in the
/// unit tests.
pub fn command(program: &str, args: &[String]) -> Command {
    if cfg!(test) {
        direct(program, args)
    } else {
        guarded("/proc/self/exe", std::process::id(), program, args)
    }
}

fn direct(program: &str, args: &[String]) -> Command {
    let mut command = Command::new(program);
    command.args(args);
    command
}

fn guarded(exe: &str, bar: u32, program: &str, args: &[String]) -> Command {
    let mut command = Command::new(exe);
    command
        .arg(MARKER)
        .arg(bar.to_string())
        .arg(program)
        .args(args);
    command
}

/// What a guard was asked to run.
#[derive(Debug, PartialEq, Eq)]
struct Plan<'a> {
    /// The bar, which the guard's parent must still be.
    bar: Pid,
    program: &'a OsStr,
    args: &'a [OsString],
}

/// Reads `[bar pid, program, args...]` (what follows the marker); `None`
/// when a pid or the program is missing or unreadable.
fn parse(args: &[OsString]) -> Option<Plan<'_>> {
    let [bar, program, rest @ ..] = args else {
        return None;
    };
    Some(Plan {
        // Through `u32` and `i32`: `Pid::from_raw` asserts (in a debug build)
        // that it is not negative, and a request is not trusted to be.
        bar: Pid::from_raw(i32::try_from(bar.to_str()?.parse::<u32>().ok()?).ok()?)?,
        program,
        args: rest,
    })
}

/// Whether this process was started as a guard (`args` is what follows the
/// program name), and if so becomes the command. Returns only when that
/// failed (the status to exit with) or when this is not a guard at all
/// (`None`: carry on as the CLI).
pub fn run(args: impl IntoIterator<Item = OsString>) -> Option<ExitCode> {
    let mut args = args.into_iter();
    if !matches!(args.next(), Some(first) if first == MARKER) {
        return None;
    }
    let args: Vec<OsString> = args.collect();
    let Some(plan) = parse(&args) else {
        warn(format_args!("scootbar: {MARKER}: a pid and a program"));
        return Some(ExitCode::from(NOT_RUN));
    };
    // Armed first, and the parent checked after: a bar that dies in between
    // is seen by the check, one that dies after is seen by the signal.
    if let Err(error) = set_parent_process_death_signal(Some(Signal::KILL)) {
        warn(format_args!(
            "scootbar: cannot arm the command to end with the bar: {error}"
        ));
    }
    if getppid() != Some(plan.bar) {
        // The bar is gone (this process was reparented): the command is
        // not wanted.
        return Some(ExitCode::FAILURE);
    }
    let error = Command::new(plan.program).args(plan.args).exec();
    Some(ExitCode::from(match error.kind() {
        io::ErrorKind::NotFound => NOT_FOUND,
        io::ErrorKind::PermissionDenied => NOT_EXECUTABLE,
        _ => {
            warn(format_args!(
                "scootbar: cannot run `{}`: {error}",
                plan.program.to_string_lossy()
            ));
            NOT_RUN
        }
    }))
}

#[cfg(test)]
mod tests;
