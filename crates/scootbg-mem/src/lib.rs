//! The only `unsafe` code in scootbg.
//!
//! `scootbg` itself is `#![forbid(unsafe_code)]`; everything that has to be
//! `unsafe` lives here, in two small modules a reviewer can read end to
//! end:
//!
//! - [`alloc`]: the global allocator that gives every block of 128 KiB or
//!   more its own mapping, so freed heap goes back to the kernel;
//! - [`shm`]: the sealed-memfd buffer behind a `wl_shm` pool, with the
//!   "never write after attach" rule enforced by type state.
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
pub mod shm;

pub use alloc::LargeAlloc;
pub use shm::{Attached, ShmBuffer, ShmError};
