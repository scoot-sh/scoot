//! Arbitrary bytes through scootbg's whole image path: decode, fit, crop,
//! scale, pack. The input layout and the guards are in
//! `crates/scootbg/src/image/fuzz.rs`, which this compiles as its own
//! module; running it is in `../README.md`.
//!
//! scootbg is a binary with no library, so its image modules are compiled
//! here by `#[path]`, unchanged: `crate::color` and `crate::image` resolve
//! in this crate exactly as they do in scootbg's. They are formatted with
//! scootbg (`cargo fmt -p scootbg`), so `rustfmt::skip` keeps this crate's
//! `cargo fmt` from following them to the `cfg(test)` modules it cannot
//! resolve from here.

#![no_main]

/// The daemon's allocator, so a claimed-but-unwritten buffer costs here
/// what it costs there (committed lazily, `scootbg_mem::zeroed_bytes`).
#[global_allocator]
static ALLOCATOR: scootbg_mem::LargeAlloc = scootbg_mem::LargeAlloc;

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/color.rs"]
mod color;

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/image/mod.rs"]
mod image;

#[rustfmt::skip]
#[path = "../../src/image/fuzz.rs"]
mod entry;

libfuzzer_sys::fuzz_target!(|data: &[u8]| entry::whole_path(data));
