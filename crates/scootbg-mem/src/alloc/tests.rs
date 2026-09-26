//! `LargeAlloc` called directly (the test harness keeps the system
//! allocator; `tests/global.rs` runs with this one installed).

use std::alloc::{GlobalAlloc, Layout};

use super::{LargeAlloc, MAP_ALIGN, THRESHOLD, is_mapped};

const A: LargeAlloc = LargeAlloc;

/// The `/proc/self/maps` ranges, as `(start, end)`. `/proc` is always
/// mounted where these tests run.
fn vmas() -> Vec<(usize, usize)> {
    let maps = std::fs::read_to_string("/proc/self/maps").expect("/proc/self/maps");
    maps.lines()
        .filter_map(|line| {
            let (lo, hi) = line.split_whitespace().next()?.split_once('-')?;
            Some((
                usize::from_str_radix(lo, 16).ok()?,
                usize::from_str_radix(hi, 16).ok()?,
            ))
        })
        .collect()
}

/// Whether `ptr..ptr + len` is page-aligned memory inside one mapping,
/// which is what a block of ours looks like. (The kernel may merge it with
/// a neighbouring anonymous mapping, so "starts a mapping" would be too
/// strict; glibc's own mmapped chunks start 16 bytes into their mapping,
/// so the alignment tells them apart.)
fn is_mapped_block(ptr: *mut u8, len: usize) -> bool {
    let start = ptr as usize;
    start.is_multiple_of(MAP_ALIGN)
        && vmas()
            .iter()
            .any(|&(lo, hi)| lo <= start && start + len <= hi)
}

/// Whether some mapping starts exactly at `ptr`.
fn starts_a_mapping(ptr: *mut u8) -> bool {
    vmas().iter().any(|&(lo, _)| lo == ptr as usize)
}

fn layout(size: usize, align: usize) -> Layout {
    Layout::from_size_align(size, align).unwrap()
}

/// Fills `len` bytes at `ptr` with a pattern derived from the index.
///
/// # Safety
/// `ptr` must be valid for writes of `len` bytes.
unsafe fn fill(ptr: *mut u8, len: usize) {
    // SAFETY: the caller guarantees `len` writable bytes.
    let bytes = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
}

/// Whether the first `len` bytes at `ptr` still hold `fill`'s pattern.
///
/// # Safety
/// `ptr` must be valid for reads of `len` bytes.
unsafe fn holds_pattern(ptr: *const u8, len: usize) -> bool {
    // SAFETY: the caller guarantees `len` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    bytes.iter().enumerate().all(|(i, &b)| b == (i % 251) as u8)
}

#[test]
fn routing_is_by_size_and_alignment() {
    assert!(!is_mapped(layout(THRESHOLD - 1, 8)));
    assert!(is_mapped(layout(THRESHOLD, 8)));
    assert!(is_mapped(layout(THRESHOLD, MAP_ALIGN)));
    assert!(!is_mapped(layout(THRESHOLD, MAP_ALIGN * 2)));
    assert!(!is_mapped(layout(1, 1)));
}

#[test]
fn a_large_block_is_its_own_page_aligned_mapping() {
    let l = layout(THRESHOLD, 16);
    // SAFETY: non-zero size.
    let ptr = unsafe { A.alloc(l) };
    assert!(!ptr.is_null());
    assert_eq!(ptr as usize % MAP_ALIGN, 0);
    assert!(is_mapped_block(ptr, THRESHOLD), "not a mapping of its own");
    // SAFETY: `ptr` holds `l.size()` bytes.
    unsafe { fill(ptr, l.size()) };
    // SAFETY: allocated above with `l`.
    unsafe { A.dealloc(ptr, l) };
}

#[test]
fn a_small_block_is_not_a_mapping_of_its_own() {
    let l = layout(THRESHOLD - 1, 8);
    // SAFETY: non-zero size.
    let ptr = unsafe { A.alloc(l) };
    assert!(!ptr.is_null());
    // SAFETY: `ptr` holds `l.size()` bytes.
    unsafe { fill(ptr, l.size()) };
    // glibc's own threshold is 128 KiB too, but it hands out its chunk's
    // user pointer 16 bytes past the mapping start, never the start itself.
    assert!(!starts_a_mapping(ptr));
    // SAFETY: allocated above with `l`.
    unsafe { A.dealloc(ptr, l) };
}

#[test]
fn alloc_zeroed_is_zero_on_both_paths() {
    for size in [64, THRESHOLD - 1, THRESHOLD, 3 * THRESHOLD + 7] {
        let l = layout(size, 8);
        // SAFETY: non-zero size.
        let ptr = unsafe { A.alloc_zeroed(l) };
        assert!(!ptr.is_null());
        // SAFETY: `ptr` holds `size` zeroed bytes.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, size) };
        assert!(bytes.iter().all(|&b| b == 0), "size {size} not zeroed");
        // SAFETY: allocated above with `l`.
        unsafe { A.dealloc(ptr, l) };
    }
}

#[test]
fn an_over_aligned_large_block_goes_to_system_and_is_aligned() {
    let l = layout(THRESHOLD * 2, MAP_ALIGN * 4);
    assert!(!is_mapped(l));
    // SAFETY: non-zero size.
    let ptr = unsafe { A.alloc(l) };
    assert!(!ptr.is_null());
    assert_eq!(ptr as usize % (MAP_ALIGN * 4), 0);
    // SAFETY: `ptr` holds `l.size()` bytes.
    unsafe { fill(ptr, l.size()) };
    // SAFETY: allocated above with `l`.
    unsafe { A.dealloc(ptr, l) };
}

/// Grows and shrinks through every route: small→small, small→large
/// (crossing up), large→large (mremap, growing then shrinking), and
/// large→small (crossing down), checking the contents survive each step.
#[test]
fn realloc_keeps_contents_across_every_route() {
    let sizes = [
        100,
        THRESHOLD - 1,
        THRESHOLD,
        8 * THRESHOLD + 3,
        THRESHOLD + 1,
        THRESHOLD - 1,
        10,
    ];
    let mut l = layout(sizes[0], 8);
    // SAFETY: non-zero size.
    let mut ptr = unsafe { A.alloc(l) };
    assert!(!ptr.is_null());
    // SAFETY: `ptr` holds `l.size()` bytes.
    unsafe { fill(ptr, l.size()) };
    for &new_size in &sizes[1..] {
        let kept = l.size().min(new_size);
        // SAFETY: `ptr` was allocated with `l`, and `new_size` is non-zero.
        let new = unsafe { A.realloc(ptr, l, new_size) };
        assert!(!new.is_null(), "realloc {} -> {new_size} failed", l.size());
        // SAFETY: the new block holds at least `kept` bytes.
        let kept_contents = unsafe { holds_pattern(new, kept) };
        assert!(kept_contents, "{} -> {new_size} lost contents", l.size());
        l = layout(new_size, 8);
        if is_mapped(l) {
            assert!(is_mapped_block(new, new_size));
        }
        ptr = new;
        // SAFETY: `ptr` holds `l.size()` bytes.
        unsafe { fill(ptr, l.size()) };
    }
    // SAFETY: allocated (last) with `l`.
    unsafe { A.dealloc(ptr, l) };
}

/// An allocation the kernel cannot satisfy returns null, on every entry
/// point, and nothing panics. `isize::MAX` rounded down to the page is the
/// largest size a `Layout` allows with this alignment; no address space
/// holds it.
#[test]
fn an_impossible_size_is_null_not_a_panic() {
    let huge = (isize::MAX as usize) & !(MAP_ALIGN - 1);
    let l = layout(huge, MAP_ALIGN);
    assert!(is_mapped(l));
    // SAFETY: non-zero size.
    assert!(unsafe { A.alloc(l) }.is_null());
    // SAFETY: non-zero size.
    assert!(unsafe { A.alloc_zeroed(l) }.is_null());

    // mremap growth to the impossible size fails and leaves the block.
    let small = layout(THRESHOLD, 8);
    // SAFETY: non-zero size.
    let ptr = unsafe { A.alloc(small) };
    assert!(!ptr.is_null());
    // SAFETY: `ptr` holds `small.size()` bytes.
    unsafe { fill(ptr, small.size()) };
    // SAFETY: allocated with `small`; `huge` is non-zero and does not
    // overflow `isize` when rounded to 8.
    assert!(unsafe { A.realloc(ptr, small, huge) }.is_null());
    // SAFETY: a failed realloc leaves the old block intact.
    assert!(unsafe { holds_pattern(ptr, small.size()) });

    // Crossing up to it fails the same way.
    let tiny = layout(64, 8);
    // SAFETY: non-zero size.
    let p2 = unsafe { A.alloc(tiny) };
    assert!(!p2.is_null());
    // SAFETY: allocated with `tiny`; `huge` as above.
    assert!(unsafe { A.realloc(p2, tiny, huge) }.is_null());

    // SAFETY: both blocks are still ours after the failed reallocs.
    unsafe { A.dealloc(ptr, small) };
    // SAFETY: as above.
    unsafe { A.dealloc(p2, tiny) };
}
