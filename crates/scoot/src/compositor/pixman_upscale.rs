//! Pixel pins for upscaled `wl_shm` surfaces under the pixman renderer.
//!
//! Smithay's pixman backend pairs `Filter::Bilinear` with `Repeat::None`
//! (`pixman/mod.rs` at the pinned fork rev), so a bilinear tap at a surface
//! edge reads past the texture, where pixman returns transparent black, and
//! the faded tap is written straight into the framebuffer. Any buffer scaled
//! *up* to its on-screen size -- a 1x1 buffer through `wp_viewporter`, a
//! scale-1 buffer on a scale-2 output -- used to get a semi-transparent
//! 1-px edge (and faded corners) instead of keeping its edge pixels. The fix
//! is a commit on the `scoot-sh/smithay` fork (`Repeat::Pad`; see
//! `docs/forks.md`), and these tests pin the framebuffer contract it
//! restores: corners, edge midpoints and centre exactly the fill color.
//!
//! There is deliberately no scoot-side implementation here: the defect and
//! the fix both live in the fork. What lives here is the fail-first net --
//! each upscale test fails on the pre-fix fork under pixman and passes after
//! -- plus a downscale test that passes on both revs, pinning that the fix
//! changed nothing where nothing was broken.
//!
//! ## Trust model
//!
//! No new protocol surface: the tests drive the same `xdg_toplevel` +
//! `wl_shm` + `wp_viewporter` path every client already uses.

#[cfg(test)]
mod tests;
