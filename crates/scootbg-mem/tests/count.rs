//! `CountingAlloc` installed as this test binary's global allocator: an
//! empty closure counts nothing, a `vec!` counts something, and each
//! window starts over.

use scootbg_mem::{CountingAlloc, count_allocations};

#[global_allocator]
static COUNTING: CountingAlloc = CountingAlloc;

#[test]
fn an_empty_closure_allocates_nothing() {
    let ((), count) = count_allocations(|| {});
    assert_eq!(count, 0);
}

#[test]
fn a_vec_allocates_and_a_second_window_starts_over() {
    // The `vec!` is the point (a heap allocation), not a fixed array.
    #![allow(clippy::useless_vec)]
    let (_, first) = count_allocations(|| {
        let _v = vec![0u8; 64];
    });
    assert!(first >= 1, "a vec is an allocation, counted {first}");
    let (_, second) = count_allocations(|| {});
    assert_eq!(second, 0);
}
