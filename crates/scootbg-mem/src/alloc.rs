//! The large-allocation global allocator.
//!
//! Blocks of [`THRESHOLD`] (128 KiB) or more, with an alignment of at most
//! [`MAP_ALIGN`] (4096), become their own private anonymous mapping and go
//! back to the kernel on free. Everything else goes to [`System`].
//!
//! Why: glibc raises its mmap threshold after the first large free, so a
//! long-lived daemon ends up keeping a decode's buffers in its heap for
//! good (61.6 MB measured after eight wallpaper changes). With the large
//! blocks routed here, glibc never sees one, its threshold never moves, and
//! dependencies' internal buffers are returned too. Evidence:
//! `docs/scootbg/backlog/resolved/dependencies-done.md` §6.
//!
//! Nothing in this module allocates or panics: a failure returns null,
//! which is `GlobalAlloc`'s out-of-memory signal.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::c_void;
use std::ptr;

use rustix::mm::{MapFlags, MremapFlags, ProtFlags, mmap_anonymous, mremap, munmap};

#[cfg(test)]
mod tests;

/// The size from which a block gets its own mapping: glibc's own default
/// mmap threshold, pinned so it can no longer rise.
pub const THRESHOLD: usize = 128 * 1024;

/// The largest alignment a mapping satisfies, as a constant.
///
/// Every Linux page size is at least 4096 and `mmap` returns page-aligned
/// memory, so 4096 is correct on every system as a *bound*. It is never
/// used as a size. `rustix::param::page_size()` must not be called here: in
/// rustix 1.1.4 it lazily reads the auxv, which `unwrap()`s on an old
/// kernel without `/proc` and allocates from inside the allocator on its
/// `/proc` fallback.
pub const MAP_ALIGN: usize = 4096;

/// The global allocator: `#[global_allocator] static A: LargeAlloc = LargeAlloc;`
#[derive(Debug, Default, Clone, Copy)]
pub struct LargeAlloc;

/// Whether `layout` is served by its own mapping.
///
/// A pure function of the layout, which is the whole routing argument:
/// `GlobalAlloc` hands `dealloc` and `realloc` the layout the block was
/// allocated with, so a block always returns to the path that made it.
///
/// A large block aligned above [`MAP_ALIGN`] stays with `System`. glibc
/// serves a large `memalign` from its own mmap, and freeing that raises its
/// dynamic threshold again, but nothing in scootbg or its dependencies is
/// known to ask for one, so it is left there rather than routed through an
/// over-allocate-and-trim mapping here.
#[inline]
pub fn is_mapped(layout: Layout) -> bool {
    layout.size() >= THRESHOLD && layout.align() <= MAP_ALIGN
}

/// A fresh private anonymous mapping of `size` bytes, or null.
fn map(size: usize) -> *mut u8 {
    // SAFETY: a null hint with no `MAP_FIXED` lets the kernel choose an
    // address, so the new mapping cannot replace or alias any existing
    // memory; `size` is non-zero (at least `THRESHOLD`).
    let mapped = unsafe {
        mmap_anonymous(
            ptr::null_mut(),
            size,
            ProtFlags::READ | ProtFlags::WRITE,
            MapFlags::PRIVATE,
        )
    };
    match mapped {
        Ok(ptr) => ptr.cast(),
        Err(_) => ptr::null_mut(),
    }
}

// SAFETY: every method upholds `GlobalAlloc`'s contract:
// - mapped blocks come from `map`, are page-aligned (which covers every
//   alignment `is_mapped` routes to them) and are exactly `layout.size()`
//   bytes of fresh memory that aliases nothing;
// - `dealloc` and `realloc` route by the same pure `is_mapped(layout)`
//   test as `alloc`, and the caller passes the allocation's own layout, so
//   `(ptr, layout.size())` on the mapped path is always one whole mapping
//   of ours and on the `System` path always a `System` block;
// - failure is a null return, never a panic or an unwind.
unsafe impl GlobalAlloc for LargeAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if is_mapped(layout) {
            map(layout.size())
        } else {
            // SAFETY: the caller's contract (a non-zero-size layout) is
            // passed through unchanged.
            unsafe { System.alloc(layout) }
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if is_mapped(layout) {
            // Anonymous mappings are zero-filled by the kernel.
            map(layout.size())
        } else {
            // SAFETY: as in `alloc`.
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if is_mapped(layout) {
            // SAFETY: `ptr` came from `map(layout.size())` (routing is a
            // pure function of the layout the caller must pass back), so
            // this unmaps exactly one whole mapping of ours, which the
            // caller promises is no longer used. A failure cannot be
            // reported from here and leaves the mapping in place (a leak,
            // not unsoundness), so it is ignored.
            let _ = unsafe { munmap(ptr.cast::<c_void>(), layout.size()) };
        } else {
            // SAFETY: `ptr` is a `System` block with this layout.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // `GlobalAlloc` promises `new_size` rounded up to `layout.align()`
        // does not overflow `isize`, so this cannot fail for a conforming
        // caller; null keeps a broken one from reaching anything below.
        let Ok(new_layout) = Layout::from_size_align(new_size, layout.align()) else {
            return ptr::null_mut();
        };
        match (is_mapped(layout), is_mapped(new_layout)) {
            (false, false) => {
                // SAFETY: `ptr` is a `System` block with `layout`, and the
                // caller's `new_size` contract is passed through unchanged.
                unsafe { System.realloc(ptr, layout, new_size) }
            }
            (true, true) => {
                // SAFETY: `(ptr, layout.size())` is one whole mapping of
                // ours (see `dealloc`). `MAYMOVE` without `FIXED` lets the
                // kernel grow in place or move to an address it picks, so
                // nothing else is replaced, and the contents up to the
                // smaller size are kept. On failure the old mapping is
                // untouched, which is what a null return requires.
                let moved = unsafe {
                    mremap(
                        ptr.cast::<c_void>(),
                        layout.size(),
                        new_size,
                        MremapFlags::MAYMOVE,
                    )
                };
                match moved {
                    Ok(new) => new.cast(),
                    Err(_) => ptr::null_mut(),
                }
            }
            // Crossing the threshold, in either direction: allocate on the
            // new path, copy, free on the old one.
            _ => {
                // SAFETY: `new_layout` has a non-zero size: it is at least
                // `THRESHOLD` when mapped, and when not, `layout` was
                // mapped, so this is a shrink to a `new_size` the caller
                // guarantees is non-zero.
                let new = unsafe { self.alloc(new_layout) };
                if new.is_null() {
                    return new;
                }
                // SAFETY: both blocks are valid for `min(old, new)` bytes,
                // and `new` is a fresh allocation, so they do not overlap.
                unsafe { ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size)) };
                // SAFETY: `ptr` is the caller's block with `layout`, and it
                // is not used again: the caller receives `new` instead.
                unsafe { self.dealloc(ptr, layout) };
                new
            }
        }
    }
}
