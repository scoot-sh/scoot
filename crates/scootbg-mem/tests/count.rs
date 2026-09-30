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

/// Two windows armed on two threads at once each count only their own
/// thread's allocations: one thread allocates in a tight loop while the
/// other measures empty windows, and a neighbour's allocations must never
/// show up in an empty window. (`cargo test` runs the tests of one binary
/// on concurrent threads, so this is the situation the other tests are in.)
#[test]
fn concurrent_windows_do_not_count_each_others_allocations() {
    const ROUNDS: usize = 20_000;
    let noisy = std::thread::spawn(|| {
        for _ in 0..ROUNDS {
            let ((), n) = count_allocations(|| {
                let v = std::hint::black_box(vec![0u8; 64]);
                drop(v);
            });
            assert_eq!(n, 1, "one vec, counted {n}");
        }
    });
    for _ in 0..ROUNDS {
        let ((), n) = count_allocations(|| {});
        assert_eq!(n, 0, "an empty window counted a neighbour's allocation");
    }
    noisy
        .join()
        .expect("the allocating thread saw exact counts");
}
