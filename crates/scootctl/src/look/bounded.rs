//! Bounded child runs: a timeout that really bounds, with capped output.
//!
//! Two runners shared one shape: spawn, poll `try_wait`, then collect
//! with `wait_with_output`. A reload that forks and holds the pipes open
//! (`sleep 8 & exec sleep 0`) blocked the target reloads for the
//! grandchild's whole life, and a `yes`-style reload captured output
//! with no bound — or wedged entirely once the pipe buffer filled, since
//! nobody drained until exit.
//!
//! One helper fixes every caller (target reloads and `run_with_timeout`),
//! so there is exactly one place that gets this right:
//! - the child leads its own process group (`process_group(0)`), so the
//!   deadline kill reaches forked grandchildren, not just the child (a
//!   grandchild that calls `setsid` leaves the group and is not reached);
//! - both pipes drain on nonblocking reads while the child runs, so a
//!   full pipe never wedges a child that would otherwise exit;
//! - each stream keeps at most [`MAX_OUTPUT_BYTES`], then the group is
//!   killed and the run reports the truncation;
//! - past the deadline nothing waits for pipe EOF: the group is killed,
//!   the direct child is reaped (`wait`, never `wait_with_output`), and
//!   whatever drained so far is dropped with the timeout;
//! - the group kill is `rustix`'s safe `kill_process_group` wrapper: no
//!   `unsafe` anywhere in this module.
//!
//! [`MAX_OUTPUT_BYTES`] bounds memory: a `yes`-style reload at the 300 s
//! maximum timeout holds at most two caps (stdout plus stderr), never
//! gigabytes.

use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How many output bytes are kept per piped stream (stdout, stderr).
/// 64 KiB is the usual pipe capacity: a run that fills it is already a
/// runaway, and every caller here answers in lines (a reload note, a
/// `fc-match` path, a one-word verdict), so nothing legitimate is cut.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// A finished run: the exit status plus the drained (capped) outputs.
#[derive(Debug)]
pub struct BoundedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// How a bounded run failed. Shaped so each caller reports precisely: a
/// missing binary is a note naming the app, a timeout or a truncation is
/// a note saying the swap stands.
#[derive(Debug)]
pub enum BoundedError {
    /// The spawn itself failed.
    Spawn(std::io::Error),
    /// The deadline passed: the whole process group was killed.
    Timeout { program: String, secs: u64 },
    /// A stream passed [`MAX_OUTPUT_BYTES`]: the group was killed.
    OutputCap { program: String },
    /// Polling, reading or reaping the child failed.
    Wait(String),
}

impl std::fmt::Display for BoundedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundedError::Spawn(error) => write!(f, "{error}"),
            BoundedError::Timeout { program, secs } => {
                write!(f, "`{program}` timed out after {secs}s and was killed")
            }
            BoundedError::OutputCap { program } => write!(
                f,
                "`{program}` wrote past {MAX_OUTPUT_BYTES} bytes of output: truncated and killed"
            ),
            BoundedError::Wait(detail) => write!(f, "{detail}"),
        }
    }
}

/// Runs `program` with `args` and waits at most `timeout`. A child that
/// outlives the wait — or floods past [`MAX_OUTPUT_BYTES`] on either
/// stream — has its whole process group killed and is reported, never
/// waited on past the deadline.
pub fn run_bounded(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<BoundedOutput, BoundedError> {
    use std::os::unix::io::AsFd as _;
    use std::os::unix::process::CommandExt as _;

    let name = program.to_string_lossy().into_owned();
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The child leads its own group, so its id is the group id: the
        // deadline kill below reaches everything it forked. Safe (no
        // `unsafe`): the `process_group` setter and the `rustix` kill.
        .process_group(0)
        .spawn()
        .map_err(BoundedError::Spawn)?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    for fd in [stdout.as_fd(), stderr.as_fd()] {
        let flags = rustix::fs::fcntl_getfl(fd).map_err(|error| {
            BoundedError::Wait(format!("cannot poll `{}`: {error}", program.display()))
        })?;
        rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(|error| {
            BoundedError::Wait(format!("cannot poll `{}`: {error}", program.display()))
        })?;
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut chunk = [0u8; 8192];
    let deadline = Instant::now() + timeout;
    loop {
        let mut made_progress = false;
        if !stdout_eof {
            match pump(&mut stdout, &mut chunk) {
                Ok(Pump::Data(n)) => {
                    made_progress = true;
                    out.extend_from_slice(&chunk[..n]);
                    if out.len() > MAX_OUTPUT_BYTES {
                        kill_group(&mut child);
                        child.wait().ok();
                        return Err(BoundedError::OutputCap {
                            program: name.clone(),
                        });
                    }
                }
                Ok(Pump::Eof) => stdout_eof = true,
                Ok(Pump::Blocked) => {}
                Err(error) => {
                    kill_group(&mut child);
                    child.wait().ok();
                    return Err(BoundedError::Wait(format!(
                        "cannot read the output of `{}`: {error}",
                        program.display()
                    )));
                }
            }
        }
        if !stderr_eof {
            match pump(&mut stderr, &mut chunk) {
                Ok(Pump::Data(n)) => {
                    made_progress = true;
                    err.extend_from_slice(&chunk[..n]);
                    if err.len() > MAX_OUTPUT_BYTES {
                        kill_group(&mut child);
                        child.wait().ok();
                        return Err(BoundedError::OutputCap {
                            program: name.clone(),
                        });
                    }
                }
                Ok(Pump::Eof) => stderr_eof = true,
                Ok(Pump::Blocked) => {}
                Err(error) => {
                    kill_group(&mut child);
                    child.wait().ok();
                    return Err(BoundedError::Wait(format!(
                        "cannot read the output of `{}`: {error}",
                        program.display()
                    )));
                }
            }
        }
        let exited = child.try_wait().map_err(|error| {
            BoundedError::Wait(format!("cannot wait on `{}`: {error}", program.display()))
        })?;
        // Both pipes at EOF means every writer is gone, so the child is
        // gone too — but only `try_wait` says so for certain (a child
        // that closed its pipes without exiting still runs).
        if stdout_eof
            && stderr_eof
            && let Some(status) = exited
        {
            return Ok(BoundedOutput {
                status,
                stdout: out,
                stderr: err,
            });
        }
        if Instant::now() >= deadline {
            // Never wait for pipe EOF past the deadline: kill the group
            // (a forked grandchild holding the pipes dies with it) and
            // reap the direct child, so no zombie is left either.
            kill_group(&mut child);
            child.wait().ok();
            return Err(BoundedError::Timeout {
                program: name,
                secs: timeout.as_secs(),
            });
        }
        if !made_progress {
            // Nothing drained: poll slowly. A child that exited with its
            // pipes held open elsewhere wakes faster than one still
            // running, but neither spins.
            std::thread::sleep(Duration::from_millis(if exited.is_some() { 5 } else { 50 }));
        }
    }
}

/// One nonblocking read: data, end of stream, or nothing yet.
enum Pump {
    Data(usize),
    Eof,
    Blocked,
}

fn pump(pipe: &mut impl std::io::Read, chunk: &mut [u8]) -> std::io::Result<Pump> {
    match pipe.read(chunk) {
        Ok(0) => Ok(Pump::Eof),
        Ok(n) => Ok(Pump::Data(n)),
        Err(error)
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::Interrupted =>
        {
            Ok(Pump::Blocked)
        }
        Err(error) => Err(error),
    }
}

/// Kills the child's whole process group, then the child itself. The
/// child is its group leader (`process_group(0)` at spawn), so its id
/// names the group. Failures are ignored on purpose: the group may
/// already be gone.
fn kill_group(child: &mut Child) {
    if let Some(pgid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pgid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests;
