//! Counting heap allocations on one thread, for tests that pin an
//! allocation-free path (scootbar's warm tick).
//!
//! [`CountingAlloc`] forwards every call to [`System`]; while armed (this
//! thread only) it counts `alloc`, `alloc_zeroed` and `realloc` calls.
//! [`count_allocations`] arms it around a closure and reports the count.
//! Only the calling thread is counted, so neighbours running concurrently
//! in the same test process (as under `cargo test`) cannot pollute the
//! measurement; nesting is not supported (an inner call would reset the
//! outer's count).
//!
//! Nothing here allocates: the counter is an [`AtomicUsize`], the armed
//! flag a thread-local, so counting cannot recurse into the allocator.
//! Unarmed allocations cost one relaxed load each.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Allocations made on the armed thread since it was armed.
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// Whether any thread is armed: the fast path keeping unarmed allocations
/// to one relaxed load.
static ANY_ARMED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread's allocations are counted now.
fn armed() -> bool {
    if ANY_ARMED.load(Ordering::Relaxed) == 0 {
        return false;
    }
    ARMED.try_with(|armed| armed.get()).unwrap_or(false)
}

/// The global allocator that counts: `#[global_allocator] static A:
/// CountingAlloc = CountingAlloc;` in a test target, with
/// [`count_allocations`] measuring through it.
#[derive(Debug, Default, Clone, Copy)]
pub struct CountingAlloc;

// SAFETY: every call is forwarded to `System` unchanged (same pointer,
// same layout, same contract), so the allocator upholds `GlobalAlloc`'s
// contract exactly as `System` does; the counting touches only an atomic
// counter and a thread-local flag, neither of which allocates or can fail.
unsafe impl GlobalAlloc for CountingAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout is passed through unchanged.
        let block = unsafe { System.alloc(layout) };
        if armed() {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        block
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout is passed through unchanged.
        let block = unsafe { System.alloc_zeroed(layout) };
        if armed() {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        block
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` is a `System` block with this layout, as the caller
        // promises every global allocator.
        unsafe { System.dealloc(ptr, layout) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` is a `System` block with `layout`, and the caller's
        // `new_size` contract is passed through unchanged.
        let block = unsafe { System.realloc(ptr, layout, new_size) };
        if armed() {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        block
    }
}

/// Disarms on drop, so a panicking `f` cannot leave a later test in the
/// same process counting.
struct Disarm;

impl Drop for Disarm {
    fn drop(&mut self) {
        ARMED.try_with(|armed| armed.set(false)).ok();
        ANY_ARMED.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Runs `f`, returning what it returned and how many heap allocations it
/// made on this thread. Do not nest: an inner call resets the outer's
/// count.
pub fn count_allocations<R>(f: impl FnOnce() -> R) -> (R, usize) {
    COUNT.store(0, Ordering::Relaxed);
    ANY_ARMED.fetch_add(1, Ordering::Relaxed);
    ARMED.with(|armed| armed.set(true));
    let _disarm = Disarm;
    let returned = f();
    let count = COUNT.load(Ordering::Relaxed);
    (returned, count)
}
