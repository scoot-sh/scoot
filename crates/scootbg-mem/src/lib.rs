//! The only `unsafe` code in scootbg, and in scootbar.
//!
//! `scootbg` and `scootbar` are `#![forbid(unsafe_code)]`; everything that
//! has to be `unsafe` lives here, in small modules a reviewer can read end
//! to end:
//!
//! - [`alloc`]: the global allocator that gives every block of 128 KiB or
//!   more its own mapping, so freed heap goes back to the kernel;
//! - [`shm`]: the sealed-memfd buffer behind a `wl_shm` pool, with the
//!   fd closed once the compositor has its own copy;
//! - [`zeroed`]: a zeroed byte buffer that is both fallible and committed
//!   only as it is written, for decoders' output;
//! - [`file`]: a file mapped read-only for the process's life, only where
//!   it is a root-owned, unwritable file on a read-only mount (scootbar's font;
//!   scootbg does not use it).
//! - [`count`]: a counting global allocator for tests that pin an
//!   allocation-free path (scootbar's warm tick), forwarding everything to
//!   the system allocator.
//!
//! Every `unsafe` block carries a `// SAFETY:` argument and holds exactly
//! one unsafe operation (clippy's `undocumented_unsafe_blocks` and
//! `multiple_unsafe_ops_per_block`, both denied in `Cargo.toml`). The design
//! and its review are in
//! `docs/scootbg/backlog/resolved/dependencies-done.md` §11.
//!
//! Pure Rust: `rustix` on its `linux_raw` backend, no `libc` crate. Linux
//! only.

// Off Linux, say so plainly: this is the first error a build of scootbg or
// scootbar (or of the whole workspace on a Mac) reports. rustc goes on to
// report `rustix`'s missing Linux-only items after it, so read the first.
#[cfg(not(target_os = "linux"))]
compile_error!(
    "scootbg-mem, scootbg and scootbar run on Linux only; on a Mac, \
     cargo check --workspace --exclude scootbar --exclude scootbg --exclude scootbg-mem \
     (dev/README.md)"
);

pub mod alloc;
pub mod count;
pub mod file;
pub mod shm;
pub mod zeroed;

pub use alloc::LargeAlloc;
pub use count::{CountingAlloc, count_allocations};
pub use shm::{ShmBuffer, ShmError};
pub use zeroed::zeroed_bytes;
