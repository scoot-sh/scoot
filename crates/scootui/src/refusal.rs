//! Test support: run a test body in a child process whose allocator
//! refuses large blocks, to see what the code under test does when memory
//! is refused (an address-space limit such as `LimitAS=` or `ulimit -v`,
//! or strict overcommit). The same class
//! `crates/scootbg/tests/scaler_probe.rs` pins for the wallpaper daemon.
//!
//! The refusal is the test binary's counting allocator answering null
//! ([`scootbg_mem::refuse_from`]), which is exactly what the system
//! allocator answers in those situations, with none of the variance a real
//! `RLIMIT_AS` has: glibc serves a request from address space it already
//! holds (a thread arena's 64 MiB reserve, the top of the `brk` heap, which
//! the first `brk` makes about 2 MiB) before it asks the kernel, so a limit
//! lowered to "now plus a little" refuses a 40 MiB block sometimes and a
//! 1 MiB one never. Measured here, with `strace`, before this was written.
//!
//! The refusal is global to the process and a block the code under test
//! should have asked for fallibly aborts it, so the test re-runs itself
//! (`--exact`, one test) as a child: the parent only checks the child
//! exited cleanly and really ran the test (`1 passed`, so a name that
//! matches nothing cannot pass). The child's body builds its fixtures
//! first, then calls [`Refusing::from`], and whatever it does next that
//! asks for that many bytes or more in one block is refused.

use std::process::Command;

const CHILD: &str = "SCOOTUI_REFUSAL_CHILD";

/// Handed to the body in the child; see the module docs.
pub struct Refusing(());

impl Refusing {
    /// From now on, a block of `bytes` or more is refused (`usize::MAX`
    /// lifts the refusal).
    pub fn from(&self, bytes: usize) {
        scootbg_mem::refuse_from(bytes);
    }
}

/// Runs `body` in a child process of this test binary, passing it the
/// [`Refusing`] to start refusing with, and fails the calling test unless
/// the child's run of it passed. `path` is the calling test's own path,
/// `concat!(module_path!(), "::name")`. The test binary's global allocator
/// must be [`scootbg_mem::CountingAlloc`].
pub fn in_refusing_child(path: &str, body: impl FnOnce(&Refusing)) {
    if std::env::var_os(CHILD).is_some() {
        body(&Refusing(()));
        return;
    }
    // libtest's names have no crate segment.
    let name = path.split_once("::").map_or(path, |(_, rest)| rest);
    let exe = std::env::current_exe().expect("this test binary's path");
    let out = Command::new(exe)
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .output()
        .expect("re-run the test as a child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the refusing child did not pass ({:?}):\n--- stdout\n{stdout}\n--- stderr\n{stderr}",
        out.status
    );
    assert!(
        stdout.contains("1 passed"),
        "the refusing child did not run exactly {name}:\n{stdout}"
    );
}
