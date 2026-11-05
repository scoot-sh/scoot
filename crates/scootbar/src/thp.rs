//! Transparent huge pages, off for the bar and on for what it launches.
//!
//! glibc 2.44's allocator `madvise(MADV_HUGEPAGE)`s its heaps, and on a
//! kernel whose transparent huge pages are `madvise` (most distributions'
//! default) a heap that grows across a 2 MiB boundary can end up one
//! resident, dirty 2 MiB page. A bar with several modules started showed
//! it on the main `[heap]` in 4 of 8 settled runs: 2.9 to 3.3 MB of
//! anonymous memory against 1.3 MB with the allocator's `hugetlb=0`. The
//! bar's heap is a megabyte or two and gains nothing from a huge page, so
//! the daemon opts its whole process out ([`disable`]).
//!
//! The setting is inherited by a child and survives `execve`, and the bar
//! starts real programs (a click's command, a network or Bluetooth menu, an
//! `exec` module's command). Those must not run with huge pages off because
//! the bar wanted a small floor. The crate forbids `unsafe`, so there is no
//! `pre_exec` to clear it in the child; instead [`spawn`] turns the
//! setting off for the one `fork` and back on straight after, and the
//! `exec` guard (`modules::exec::guard`) clears it in its own process
//! before it becomes the command. The bar's own heap is exposed only to
//! what `spawn` itself allocates inside that window, and to a worker
//! thread's allocation that lands in it: a rare, user-triggered, brief
//! gap, which at worst leaves one huge page.
//!
//! A kernel without the call refuses it; that is not an error, the bar just
//! keeps the allocator's default. `tests/exec.rs` pins both halves.

use std::io;
use std::process::{Child, Command};

/// Keeps the kernel from backing this process's heap with 2 MiB huge
/// pages. `main` calls it before the command line is parsed: a page the
/// heap has already faulted in stays huge, and called at the top of
/// `daemon::run` instead, 2 of 10 settled runs still held one.
pub fn disable() {
    let _ = rustix::thread::disable_transparent_huge_pages(true);
}

/// Clears the opt-out, unconditionally: huge pages are enabled afterwards
/// even if whoever started the bar had already disabled them, so a bar
/// started that way hands its commands huge pages anyway. For the one
/// process that is about to `exec` a user's command and has nothing left
/// to protect.
pub fn restore() {
    let _ = rustix::thread::disable_transparent_huge_pages(false);
}

/// Starts `command` with huge pages enabled (see [`restore`]: enabled, not
/// the bar's starting setting), and leaves the bar's own heap opted out
/// again after.
pub fn spawn(command: &mut Command) -> io::Result<Child> {
    restore();
    let child = command.spawn();
    disable();
    child
}
