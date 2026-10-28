//! A test binary that never installs [`CountingAlloc`] must not read a
//! silent zero: every zero-allocation pin would pass with nothing behind it
//! (the vacuous pins of PRs #670 and #671). Deliberately no
//! `#[global_allocator]` here.

use scootbg_mem::count_allocations;

#[test]
#[should_panic(expected = "CountingAlloc is not the global allocator")]
fn counting_without_the_allocator_installed_is_loud() {
    let _ = count_allocations(|| std::hint::black_box(vec![1u8; 64]));
}
