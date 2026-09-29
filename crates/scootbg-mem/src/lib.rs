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
//!   it lies on a read-only mount (scootbar's font; scootbg does not use
//!   it).
//!
//! Every `unsafe` block carries a `// SAFETY:` argument and holds exactly
//! one unsafe operation (clippy's `undocumented_unsafe_blocks` and
//! `multiple_unsafe_ops_per_block`, both denied in `Cargo.toml`). The design
//! and its review are in
//! `docs/scootbg/backlog/resolved/dependencies-done.md` §11.
//!
//! Pure Rust: `rustix` on its `linux_raw` backend, no `libc` crate. Linux
//! only; on any other target this crate is empty.

#![cfg(target_os = "linux")]

pub mod alloc;
pub mod file;
pub mod shm;
pub mod zeroed;

pub use alloc::LargeAlloc;
pub use shm::{ShmBuffer, ShmError};
pub use zeroed::zeroed_bytes;
