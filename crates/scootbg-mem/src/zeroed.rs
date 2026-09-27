//! A zeroed byte buffer that is fallible and committed lazily.
//!
//! `Vec::try_reserve_exact` followed by `resize(len, 0)` is fallible but
//! writes every byte at once, so a file that only *claims* a large size
//! (a PNG header saying 16384×16384, a few hundred bytes in all) commits
//! the whole buffer before a pixel is read. `vec![0; len]` asks the
//! allocator for zeroed memory, which it commits only as it is written,
//! but aborts the process if the allocation fails. This is both: it asks
//! for zeroed memory (`alloc_zeroed`, which for a large block is a fresh
//! anonymous mapping, [`crate::alloc`], and for a small one `calloc`) and
//! returns `None` if the allocator refuses.
//!
//! The pages are committed as the decoder writes them, so a truncated or
//! lying file costs what it actually decodes, not what it claims.

use std::alloc::{Layout, alloc_zeroed};

#[cfg(test)]
mod tests;

/// `len` zero bytes, or `None` if they cannot be allocated (or `len` is
/// more than any allocation may hold).
pub fn zeroed_bytes(len: usize) -> Option<Vec<u8>> {
    if len == 0 {
        return Some(Vec::new());
    }
    // `Err` past `isize::MAX`, the most any allocation may hold.
    let layout = Layout::array::<u8>(len).ok()?;
    // SAFETY: `layout` has a non-zero size (`len > 0`), which is all
    // `alloc_zeroed` asks of its caller.
    let ptr = unsafe { alloc_zeroed(layout) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` was allocated by the global allocator (what `Vec<u8>`
    // uses) with exactly the layout a `Vec<u8>` of capacity `len` has
    // (size `len`, alignment 1); all `len` bytes are initialised (zeroed);
    // and the length does not exceed the capacity. The `Vec` becomes the
    // only owner of the block and frees it with that same layout.
    Some(unsafe { Vec::from_raw_parts(ptr, len, len) })
}
