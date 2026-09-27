//! `LargeAlloc` installed as this test binary's global allocator, the way
//! the daemon runs it: the harness, `Vec`, `String` and threads all
//! allocate through it.
#![cfg(target_os = "linux")]

use std::thread;

use scootbg_mem::LargeAlloc;
use scootbg_mem::alloc::THRESHOLD;

#[global_allocator]
static GLOBAL: LargeAlloc = LargeAlloc;

/// A deterministic byte for index `i` of buffer `seed`.
fn byte(seed: usize, i: usize) -> u8 {
    (seed.wrapping_mul(31).wrapping_add(i) % 253) as u8
}

/// Several threads grow, shrink, clone and drop buffers across the
/// threshold at once, each checking its own contents as it goes. Any
/// routing mistake (a mapped block freed to glibc, or the reverse), a lost
/// copy on a threshold crossing or a mremap that failed to keep contents
/// shows as a crash or a wrong byte.
#[test]
fn threads_grow_and_shrink_across_the_threshold() {
    let threads: Vec<_> = (0..8)
        .map(|t| {
            thread::spawn(move || {
                for round in 0..40 {
                    let seed = t * 1000 + round;
                    let mut v: Vec<u8> = Vec::new();
                    // Grows one push at a time: the capacity doubles through
                    // the threshold and on through mremap growth.
                    let target = THRESHOLD * (1 + (round % 5)) + round;
                    for i in 0..target {
                        v.push(byte(seed, i));
                    }
                    let copy = v.clone();
                    assert_eq!(copy, v);
                    // Shrink back down across the threshold.
                    v.truncate(THRESHOLD / 2 + round);
                    v.shrink_to_fit();
                    assert!(v.iter().enumerate().all(|(i, &b)| b == byte(seed, i)));
                    // And a zeroed large one.
                    let zeros = vec![0u8; THRESHOLD * 3 + round];
                    assert!(zeros.iter().all(|&b| b == 0));
                }
            })
        })
        .collect();
    for handle in threads {
        handle.join().expect("a worker thread panicked");
    }
}

/// A large block's pages go back to the kernel on free, which is the whole
/// point: RSS rises by the block and falls again.
#[test]
fn freed_large_blocks_leave_rss() {
    fn rss_anon_kb() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
        status
            .lines()
            .find_map(|l| l.strip_prefix("RssAnon:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .expect("RssAnon in /proc/self/status")
    }
    // Large against what the stress test above holds at once (~24 MiB
    // across its threads, under `cargo test`'s shared process), so its
    // churn cannot flip either assertion.
    const SIZE: usize = 256 << 20;
    let before = rss_anon_kb();
    let mut block = vec![0u8; SIZE];
    // Touch every page so it is resident.
    for page in block.chunks_mut(4096) {
        page[0] = 1;
    }
    let during = rss_anon_kb();
    drop(block);
    let after = rss_anon_kb();
    assert!(
        during >= before + 200 * 1024,
        "256 MiB touched, RssAnon {before} -> {during} kB"
    );
    assert!(
        after + 200 * 1024 <= during,
        "freed 256 MiB, RssAnon {during} -> {after} kB"
    );
}

/// The failure this allocator exists for. Under plain glibc, freeing a
/// 4 MiB block raises the mmap threshold to 4 MiB, so the next 512 KiB
/// block comes from the heap (not page-aligned) and stays there after its
/// free. Here it is still a mapping of its own, page-aligned, however
/// large the last free was.
#[test]
fn a_large_free_does_not_move_later_blocks_into_the_heap() {
    let big = vec![1u8; 4 << 20];
    drop(big);
    let next = vec![2u8; 512 << 10];
    assert_eq!(
        next.as_ptr() as usize % scootbg_mem::alloc::MAP_ALIGN,
        0,
        "a 512 KiB block after a 4 MiB free was not a mapping of its own"
    );
}
