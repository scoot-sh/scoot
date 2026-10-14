//! A GPU library loader probe that cannot hang the session.
//!
//! Since the default renderer request became `auto`, a `gpu-scanout` build's
//! `--tty` startup `dlopen`s libEGL (and, with `runtime-gbm`, libgbm) once
//! per candidate DRM device to decide the tier. A present-but-pathological
//! library -- one whose ELF constructor never returns -- would hang that
//! `dlopen` forever, turning a startup that previously resolved without
//! probing into a hang with no log line and nothing for a supervisor to act
//! on. `dlopen` has no timeout flag, and a helper thread cannot fix it: a
//! stuck thread cannot be killed, and a stuck `dlopen` may hold the loader
//! lock, wedging every later `dlopen` in the process (including the real EGL
//! load the GPU tier would then attempt). So the probe runs in a forked
//! helper process with a deadline instead: the parent kills the helper on
//! expiry and treats the library as unloadable, and the session falls back
//! to the CPU renderer with a warning.
//!
//! ## Why `fork` is sound here
//!
//! Both callers run before the session serves anything: `tty::open_device`
//! at startup (the event loop does not exist yet -- see the `set_var`
//! safety note in `compositor::run`, which relies on the same fact) and on
//! the seat-reconnect path (the event loop's own thread; the compositor
//! spawns no worker threads -- the only `std::thread::spawn` under
//! `compositor/` is `--nested`'s startup handshake helper and the test
//! harnesses -- and every other child is a *process* via `State::spawn`).
//! Forking a single-threaded process copies no lock state that another
//! thread could hold, so the helper's `dlopen` cannot inherit a wedged
//! loader lock. The one process-global `SIGCHLD` consumer,
//! `child_reaper`, only ever `waitpid`s pids `State::spawn` tracked, never
//! `-1`, so it cannot steal this helper's exit status; this module reaps
//! the helper itself, so no zombie is ever left behind.
//!
//! ## What the helper may do
//!
//! Only `dlopen` and `_exit` (deliberately no `dlclose`: running a
//! pathological library's destructors in the helper would add a second hang
//! shape for nothing -- the helper exits and the kernel reclaims it).
//! Everything else -- the `CString`, the deadline, the kill, the reap, the
//! `dlerror` text for the failure message -- happens in the parent.
//!
//! Startup-only, never per frame: one fork per candidate device that reaches
//! the probe (a `fork` plus a healthy `dlopen`, single-digit ms -- see the
//! cost test), and devices whose KMS driver class already decides the CPU
//! tier never probe at all (see `tty::open_device`).

use std::ffi::CString;
use std::time::{Duration, Instant};

/// How long a loader probe may take before the library counts as
/// present-but-unresponsive: ~200x a healthy `dlopen` (single-digit ms),
/// so one bad library costs seconds, never forever.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// What a loader probe found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    /// The library loaded (the helper exited 0).
    Loadable,
    /// The library failed to load, with the loader's own reason.
    Missing(String),
    /// The helper did not answer in time and was killed: the library is
    /// present but unresponsive.
    TimedOut,
}

/// Probes whether `soname` loads, in a forked helper with a `timeout`
/// deadline: [`Probe::Loadable`] when it loads, [`Probe::Missing`] with the
/// loader's reason when it fails fast, [`Probe::TimedOut`] when the helper
/// had to be killed.
///
/// The fork is only sound while this process is single-threaded (see the
/// module doc). Both production callers hold that precondition. Tests run
/// in a multithreaded harness, where forking around a `dlopen` could
/// inherit a loader lock held by a sibling test thread -- so the
/// hanging-library test accepts that shape (a wedged helper still answers
/// `TimedOut`, which is the assertion) and the healthy-library assertions
/// retry rather than failing a flake.
pub(crate) fn probe_library(soname: &str, timeout: Duration) -> Probe {
    let name = match CString::new(soname) {
        Ok(name) => name,
        Err(_) => return Probe::Missing("invalid library name".to_owned()),
    };
    let Some(pid) = spawn_helper() else {
        // No helper (pid/fd pressure): today's behavior -- probe inline.
        // Strictly no worse than before this module existed, and a debug
        // line rather than a warning: the fallback below already says what
        // the session is doing.
        tracing::debug!(
            error = %std::io::Error::last_os_error(),
            "loader probe could not fork; probing inline"
        );
        return probe_inline(&name);
    };
    if pid == 0 {
        // The helper. Only `dlopen` and `_exit`, per the module doc -- and
        // no `dlclose`: the helper exits either way, so closing would only
        // run a pathological library's destructors for no reason.
        //
        // SAFETY: the single-threaded precondition (module doc) means no
        // other thread can hold loader state this `dlopen` needs; nothing
        // else runs in the helper past this point.
        unsafe {
            let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            libc::_exit(i32::from(handle.is_null()));
        }
    }
    match wait_for_helper(pid, timeout) {
        Waited::Exited(0) => Probe::Loadable,
        Waited::Exited(_) => Probe::Missing(load_failure_detail(&name)),
        Waited::Gone => Probe::TimedOut,
    }
}

/// Forks a probe helper: `Some(pid)` in the parent (0 in the helper, which
/// continues past the return), `None` when the fork itself failed.
///
/// The helper must only call async-signal-safe functions plus whatever load
/// it is probing, then `_exit` -- never return into Rust code. See the
/// module doc for why forking is sound at the call sites.
///
/// SAFETY: the caller upholds the module's single-threaded precondition.
pub(crate) fn spawn_helper() -> Option<libc::pid_t> {
    // SAFETY: see above.
    let pid = unsafe { libc::fork() };
    if pid < 0 { None } else { Some(pid) }
}

/// The synchronous probe [`probe_library`] degrades to when `fork` itself
/// fails: exactly what every caller did before this module existed.
fn probe_inline(name: &CString) -> Probe {
    // SAFETY: `dlerror` cleared first so the text below belongs to this
    // call; the `CString` outlives it; the handle is closed before
    // returning.
    unsafe {
        libc::dlerror();
        let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            return Probe::Missing(load_error_text());
        }
        libc::dlclose(handle);
    }
    Probe::Loadable
}

/// The loader's reason the helper's library failed to load, read in the
/// parent after the helper reported failure.
///
/// A second `dlopen`, not the helper's own text: the helper can only exit
/// with a status (see the module doc). Sound because the helper proved the
/// load *returns* (it exited rather than hanging) -- the only new risk is a
/// library swapped between the two calls, which at worst mistates an error
/// string on a startup that is already falling back.
fn load_failure_detail(name: &CString) -> String {
    // SAFETY: as in `probe_inline`.
    unsafe {
        libc::dlerror();
        let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            return load_error_text();
        }
        libc::dlclose(handle);
    }
    "loaded on retry after the probe failed".to_owned()
}

/// The pending `dlerror` text, or a stand-in when there is none.
fn load_error_text() -> String {
    // SAFETY: `dlerror` returns either NULL or a pointer to a
    // NUL-terminated string it owns; copying it out before any further dl*
    // call is the documented use.
    unsafe {
        let text = libc::dlerror();
        if text.is_null() {
            "unknown loader error".to_owned()
        } else {
            std::ffi::CStr::from_ptr(text)
                .to_string_lossy()
                .into_owned()
        }
    }
}

/// What [`wait_for_helper`] found.
pub(crate) enum Waited {
    /// The helper exited with this status.
    Exited(i32),
    /// The helper was killed after the deadline (or is otherwise gone):
    /// callers treat this as the library being unresponsive.
    Gone,
}

/// Waits up to `timeout` for the forked helper `pid`, then kills and reaps
/// it. Always reaps: success, failure and timeout alike leave no zombie.
///
/// Only ever passed pids [`spawn_helper`] returned, never `-1`, so the
/// `child_reaper` (which only reaps pids `State::spawn` tracked) cannot
/// interfere, whichever of the two runs first.
pub(crate) fn wait_for_helper(pid: libc::pid_t, timeout: Duration) -> Waited {
    let start = Instant::now();
    loop {
        let mut status = 0;
        // SAFETY: `waitpid` on a positive pid this process forked, with
        // `WNOHANG`, never blocks; `status` is a valid out-pointer.
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if reaped == pid {
            if libc::WIFEXITED(status) {
                return Waited::Exited(libc::WEXITSTATUS(status));
            }
            // Signalled (by anything but our own kill below, which returns
            // directly): the helper is gone without answering.
            return Waited::Gone;
        }
        if reaped < 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno == libc::EINTR {
                continue;
            }
            // `ECHILD` (reaped elsewhere -- impossible per above, but
            // handled rather than spun on) and anything else: the helper is
            // gone without an answer we can read.
            return Waited::Gone;
        }
        if start.elapsed() >= timeout {
            // SAFETY: `kill` on our own helper's pid; failure (already
            // exited) is fine -- the blocking reap below settles it.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            return reap_after_kill(pid);
        }
        // 1 ms polls: the healthy path answers on an early poll (a fork plus
        // a healthy `dlopen` is single-digit ms), while a full 2 s timeout
        // that only a pathological library ever reaches costs ~2000
        // `waitpid` syscalls -- microseconds apiece, nothing against the
        // seconds the timeout itself spends.
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Reaps a helper [`wait_for_helper`] just killed: blocking, since the
/// `SIGKILL` guarantees it exits. Returns `Gone` (the timeout answer)
/// however the reap lands.
fn reap_after_kill(pid: libc::pid_t) -> Waited {
    loop {
        let mut status = 0;
        // SAFETY: as above, but blocking: `SIGKILL` was just sent, so this
        // returns once the kernel releases the helper.
        let reaped = unsafe { libc::waitpid(pid, &mut status, 0) };
        if reaped == pid {
            return Waited::Gone;
        }
        if reaped >= 0 {
            continue;
        }
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EINTR {
            continue;
        }
        // `ECHILD` (reaped between the kill and here) or anything else:
        // either way the helper is gone.
        return Waited::Gone;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `gh #177` shape, now answered through the helper: a soname no
    /// machine provides fails fast with its name on the error, never a
    /// panic and never the timeout.
    #[test]
    fn a_missing_library_fails_fast_with_its_name_on_it() {
        let missing = "libscoot-probe-no-such-library.so.1";
        let start = Instant::now();
        let probe = probe_library(missing, Duration::from_secs(10));
        let elapsed = start.elapsed();
        let detail = match probe {
            Probe::Missing(detail) => detail,
            other => panic!("a missing library must fail the probe, not {other:?}"),
        };
        assert!(
            elapsed < Duration::from_secs(5),
            "a missing library must fail the probe fast, not at the deadline: {elapsed:?}"
        );
        assert!(
            !detail.is_empty(),
            "the failure should carry the loader's reason"
        );
    }

    /// Builds a library whose ELF constructor never returns, then proves the
    /// probe fires its timeout and the process survives to probe again: the
    /// regression pin for a `--tty` startup hanging where it previously
    /// resolved without probing.
    ///
    /// The follow-up probe of a healthy library is the load-bearing half:
    /// a thread-based timeout would leave the stuck `dlopen` holding the
    /// loader lock behind, wedging every later `dlopen` -- including the
    /// real EGL load the GPU tier would then attempt. The forked helper
    /// dies with its address space instead, so the very next probe loads
    /// cleanly.
    #[test]
    fn a_hanging_library_times_out_and_leaves_the_loader_usable() {
        let Some(path) = hanging_library() else {
            eprintln!("a_hanging_library_times_out: no C compiler on PATH; skipped");
            return;
        };
        let start = Instant::now();
        let probe = probe_library(&path.to_string_lossy(), Duration::from_millis(500));
        let elapsed = start.elapsed();
        assert_eq!(
            probe,
            Probe::TimedOut,
            "a library whose constructor never returns must time out, not hang"
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "the probe must return near its deadline, not hang: {elapsed:?}"
        );
        // The loader is not wedged: a healthy library loads immediately
        // afterwards in this same process.
        let start = Instant::now();
        let mut healthy = None;
        for _ in 0..3 {
            let probe = probe_library(super::super::gles::LIB_EGL_SONAME, Duration::from_secs(5));
            if probe == Probe::Loadable {
                healthy = Some(start.elapsed());
                break;
            }
        }
        assert!(
            healthy.is_some(),
            "after killing a hung helper, a healthy library must still probe loadable \
             (a stuck thread holding the loader lock would wedge this)"
        );
        eprintln!(
            "healthy EGL probe after a kill: {:?}",
            healthy.expect("probed")
        );
    }

    /// The normal path's price, for the record CLAUDE.md asks for whenever a
    /// change lands on a startup path: a fork plus one healthy `dlopen`.
    /// Asserts nothing about time (a wall-clock threshold in CI is a flake,
    /// not a guarantee) -- run with `-- --nocapture` to read the numbers.
    #[test]
    fn loader_probe_cost() {
        const ROUNDS: u32 = 5;
        let mut best = Duration::MAX;
        for _ in 0..ROUNDS {
            let start = Instant::now();
            let probe = probe_library(super::super::gles::LIB_EGL_SONAME, Duration::from_secs(10));
            let elapsed = start.elapsed();
            assert_eq!(probe, Probe::Loadable, "the suite needs a loadable libEGL");
            best = best.min(elapsed);
        }
        eprintln!("loader probe (fork + healthy libEGL dlopen): best of {ROUNDS} = {best:?}");
    }

    /// Compiles a tiny shared library whose constructor sleeps forever, in a
    /// temp dir. `None` when no C compiler is on `PATH` (the caller skips --
    /// the same shape as the udmabuf-gated suites).
    fn hanging_library() -> Option<std::path::PathBuf> {
        if std::process::Command::new("cc")
            .arg("--version")
            .output()
            .is_err()
        {
            return None;
        }
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = dir.path().join("scoot_hang.c");
        std::fs::write(
            &source,
            "#include <unistd.h>\n\
             __attribute__((constructor)) static void scoot_hang(void) {\n\
             for (;;) { sleep(60); }\n\
             }\n",
        )
        .expect("the hang source");
        let lib = dir.path().join("libscoot_hang.so");
        let status = std::process::Command::new("cc")
            .args([
                "-shared",
                "-fPIC",
                "-o",
                &lib.to_string_lossy(),
                &source.to_string_lossy(),
            ])
            .status()
            .expect("a C compiler run");
        assert!(status.success(), "the hanging library must compile");
        // The temp dir must outlive the probe: leak it (the OS reclaims it
        // with the test process, like every other test temp dir that a
        // child process outlives).
        let path = lib.clone();
        std::mem::forget(dir);
        Some(path)
    }
}
